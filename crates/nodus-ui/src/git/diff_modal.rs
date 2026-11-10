//! The Changes window (`DiffModal.tsx`, `git.css`; GIT-032…GIT-047): the
//! project's uncommitted changes, a file list beside the selected file's
//! diff, like GitHub Desktop.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, HighlightStyle, InteractiveElement, IntoElement,
    KeyDownEvent, ListAlignment, ListState, MouseButton, ParentElement, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, list, px,
};
use nodus_domain::js::utf16_len;
use nodus_domain::{GitFileChange, GitFileDiff, GitFileStatus, GitStatus};
use nodus_git::labels::{self, DiffSelection, ListKey};
use nodus_git::{DiffLine, DiffLineKind, ParsedDiff, parse_diff};

use crate::actions::icons::{Glyph, glyph};
use crate::components::button;
use crate::components::modal::Modal;
use crate::components::native_tooltip;
use crate::components::selectable;
use crate::git::{GIT_POLL, GitEvent, GitModel, close_diff};
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// A file's diff as read, or why it couldn't be.
#[derive(Clone)]
enum Loaded {
    Diff {
        file: GitFileChange,
        diff: GitFileDiff,
        parsed: Rc<ParsedDiff>,
    },
    Error {
        file: GitFileChange,
        error: String,
    },
}

impl Loaded {
    fn file(&self) -> &GitFileChange {
        match self {
            Self::Diff { file, .. } | Self::Error { file, .. } => file,
        }
    }
}

pub struct DiffModal {
    git: Entity<GitModel>,
    pub project_id: String,
    /// GIT-033: whether the read made on opening has settled.
    checked: bool,
    settled_at_open: u64,
    selection: DiffSelection,
    /// GIT-041: each file's last read, by `labels::file_key`.
    cache: HashMap<String, Loaded>,
    /// What the pane shows (GIT-042: the last file's until the new one loads).
    loaded: Option<Loaded>,
    /// The file the read loop is for, and Refresh's restarts.
    reading: Option<String>,
    reloads: u64,
    read_loop: Option<Task<()>>,
    /// The lines on screen: which file they're for, and how many.
    lines: ListState,
    drawn: Option<(String, Rc<ParsedDiff>)>,
    list_scroll: ScrollHandle,
    scrolled_to: Option<usize>,
    panel_focus: FocusHandle,
    list_focus: FocusHandle,
    /// What had focus before it opened, given back when it closes.
    previous_focus: Option<FocusHandle>,
    focused_once: bool,
    _subscriptions: Vec<Subscription>,
}

impl DiffModal {
    /// Made by the model as `diffOpen` goes on, which reads the status again
    /// (GIT-033); `settled_at_open` is its count of settled reads before that.
    pub fn new(git: Entity<GitModel>, project_id: String, settled_at_open: u64, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&git, |m, git, event, cx| match event {
                GitEvent::Settled => {
                    if !m.checked && git.read(cx).settled > m.settled_at_open {
                        m.checked = true;
                        cx.notify();
                    }
                }
                // GIT-041: the window gaining focus reads the shown file again.
                GitEvent::Focused => {
                    m.reloads += 1;
                    m.reading = None;
                    cx.notify();
                }
            }),
            cx.observe(&git, |_, _, cx| cx.notify()),
        ];
        Self {
            git,
            project_id,
            checked: false,
            settled_at_open,
            selection: DiffSelection::default(),
            cache: HashMap::new(),
            loaded: None,
            reading: None,
            reloads: 0,
            read_loop: None,
            lines: ListState::new(0, ListAlignment::Top, px(400.)),
            drawn: None,
            list_scroll: ScrollHandle::new(),
            scrolled_to: None,
            panel_focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            previous_focus: None,
            focused_once: false,
            _subscriptions: subscriptions,
        }
    }

    /// The focus to give back when it closes.
    pub fn previous_focus(&self) -> Option<FocusHandle> {
        self.previous_focus.clone()
    }

    fn files(&self, cx: &App) -> Vec<GitFileChange> {
        self.git
            .read(cx)
            .status
            .as_ref()
            .map(|g| g.files.clone())
            .unwrap_or_default()
    }

    /// GIT-041: the selected file is read now, then every 4 s while the
    /// window has focus; a newly selected one shows its cached diff at once.
    fn sync_reads(&mut self, file: Option<&GitFileChange>, cx: &mut Context<'_, Self>) {
        let Some(file) = file else {
            self.reading = None;
            self.read_loop = None;
            self.loaded = None;
            return;
        };
        let key = labels::file_key(file);
        if self.reading.as_deref() == Some(key.as_str()) {
            return;
        }
        if let Some(cached) = self.cache.get(&key) {
            self.loaded = Some(cached.clone());
        }
        self.reading = Some(key.clone());
        let file = file.clone();
        let project = self.project_id.clone();
        let nodus = self.git.read(cx).state.read(cx).nodus.clone();
        let git = self.git.clone();
        self.read_loop = Some(cx.spawn(async move |this, cx| {
            let mut first = true;
            loop {
                if !first {
                    cx.background_executor().timer(GIT_POLL).await;
                    let active = cx.update(|cx| git.read(cx).active);
                    if !active {
                        continue;
                    }
                }
                first = false;
                let result = nodus.git().diff(&project, &file).await;
                let alive = this.update(cx, |m, cx| {
                    m.store(&key, &file, result.map_err(|e| e.ipc_message()), cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// GIT-041, GIT-043: a re-read replaces the cached diff only when it
    /// changed (or the last read failed); a failure is kept and shown.
    fn store(
        &mut self,
        key: &str,
        file: &GitFileChange,
        result: Result<GitFileDiff, String>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.reading.as_deref() != Some(key) {
            return;
        }
        let next = match result {
            Ok(diff) => {
                if let Some(Loaded::Diff { diff: prev, .. }) = self.cache.get(key)
                    && labels::same_diff(prev, &diff)
                {
                    return;
                }
                let parsed = Rc::new(parse_diff(&diff.patch));
                Loaded::Diff {
                    file: file.clone(),
                    diff,
                    parsed,
                }
            }
            Err(error) => Loaded::Error {
                file: file.clone(),
                error,
            },
        };
        self.cache.insert(key.to_owned(), next.clone());
        self.loaded = Some(next);
        cx.notify();
    }

    /// `Refresh` (GIT-041): the status again, and the selected file's reads
    /// start over.
    fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        self.git.update(cx, GitModel::refresh);
        self.reloads += 1;
        self.reading = None;
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(previous) = self.previous_focus.clone() {
            window.focus(&previous, cx);
        }
        let state = self.git.read(cx).state.clone();
        close_diff(&state, cx);
    }

    /// GIT-039: with the list focused, ↓/↑ move by one, Home/End jump.
    fn on_list_key(&mut self, e: &KeyDownEvent, cx: &mut Context<'_, Self>) {
        let key = match e.keystroke.key.as_str() {
            "down" => ListKey::Down,
            "up" => ListKey::Up,
            "home" => ListKey::Home,
            "end" => ListKey::End,
            _ => return,
        };
        cx.stop_propagation();
        let files = self.files(cx);
        self.selection.key(&files, key);
        cx.notify();
    }
}

impl Render for DiffModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let git: Option<GitStatus> = self.git.read(cx).status.clone();
        let files = git.as_ref().map(|g| g.files.clone()).unwrap_or_default();
        let index = self.selection.index(&files);
        let file = index.map(|i| files[i].clone());
        self.sync_reads(file.as_ref(), cx);

        // GIT-039: the list takes focus as the window opens, if it's there.
        if !self.focused_once {
            self.focused_once = true;
            self.previous_focus = window.focused(cx);
            if files.is_empty() {
                window.focus(&self.panel_focus, cx);
            } else {
                window.focus(&self.list_focus, cx);
            }
        }
        // GIT-039: the selected row is scrolled into view when it changes.
        if index != self.scrolled_to {
            self.scrolled_to = index;
            if let Some(i) = index {
                self.list_scroll.scroll_to_item(i);
            }
        }

        let viewport = window.viewport_size();
        let panel_w = px(1180.).min(viewport.width - px(48.));
        let panel_h = (viewport.height * 0.88).min(px(900.));
        let count = files.len();
        let this = cx.entity();

        let header = {
            let refresh = this.clone();
            let close = this.clone();
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(16.))
                .pt(px(16.))
                .pr(px(12.))
                .pb(px(14.))
                .pl(px(22.))
                .border_b(px(0.5))
                .border_color(t.separator)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_family(fonts::DISPLAY)
                                .text_size(TEXT_LG)
                                .line_height(px(21.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Changes"),
                        )
                        // GIT-034
                        .when_some(git.as_ref(), |d, git| {
                            d.child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_x(px(14.))
                                    .gap_y(px(4.))
                                    .mt(px(3.))
                                    .text_color(t.text_secondary)
                                    .text_size(TEXT_SM)
                                    .line_height(px(16.8))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .min_w_0()
                                            .child(glyph(Glyph::GitBranch, 12., 2.2, t.text_secondary))
                                            .child(labels::branch_label(git)),
                                    )
                                    .child(labels::changed_files(count)),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            button("diff-refresh")
                                .ghost()
                                .square()
                                .tooltip("Refresh")
                                .child(glyph(Glyph::RefreshCw, 14., 2., t.text_primary))
                                .on_click(move |_, _, cx| refresh.update(cx, DiffModal::refresh)),
                        )
                        .child(
                            button("diff-close")
                                .ghost()
                                .square()
                                .tooltip("Close (Esc)")
                                .child(glyph(Glyph::X, 16., 2., t.text_primary))
                                .on_click(move |_, window, cx| close.update(cx, |m, cx| m.close(window, cx))),
                        ),
                )
        };

        let body = match &git {
            // GIT-033, GIT-035
            None => empty(
                if self.checked {
                    vec![
                        empty_title("Not a Git repository"),
                        div()
                            .child("This project’s folder isn’t tracked by Git, so there are no changes to show.")
                            .into_any_element(),
                    ]
                } else {
                    vec![div().child("Reading changes…").into_any_element()]
                },
                &t,
            ),
            Some(git) if count == 0 => empty(
                vec![
                    empty_title("No local changes"),
                    div()
                        .child("There are no uncommitted changes on ")
                        .child(
                            div()
                                .font_family(fonts::mono())
                                .text_size(TEXT_SM)
                                .child(labels::branch_label(git)),
                        )
                        .child(".")
                        .flex()
                        .into_any_element(),
                ],
                &t,
            ),
            Some(_) => {
                let focused = self.list_focus.is_focused(window);
                let rows: Vec<AnyElement> = files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| file_row(&this, i, f, index == Some(i), focused, &t))
                    .collect();
                let files_w = (panel_w * 0.26).clamp(px(220.), px(320.));
                let list_key = this.clone();
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("diff-files")
                            .track_focus(&self.list_focus)
                            .track_scroll(&self.list_scroll)
                            .flex_none()
                            .w(files_w)
                            .p(px(6.))
                            .overflow_y_scroll()
                            .border_r(px(0.5))
                            .border_color(t.separator)
                            .bg(t.bg_sunken)
                            .on_key_down(move |e, _, cx| list_key.update(cx, |m, cx| m.on_list_key(e, cx)))
                            .children(rows),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .children(self.loaded.clone().map(|l| self.pane(&l, window, cx))),
                    )
                    .into_any_element()
            }
        };

        let on_close = this.clone();
        let esc = this.clone();
        Modal::new("diff-modal", panel_w, move |window, cx| {
            on_close.update(cx, |m, cx| m.close(window, cx));
        })
        .render(
            div()
                .id("diff-panel")
                .track_focus(&self.panel_focus)
                // GIT-032: Escape closes it.
                .on_key_down(move |e: &KeyDownEvent, window, cx| {
                    if e.keystroke.key == "escape" {
                        cx.stop_propagation();
                        esc.update(cx, |m, cx| m.close(window, cx));
                    }
                })
                .flex()
                .flex_col()
                .h(panel_h)
                .text_color(t.text_primary)
                .child(header)
                .child(body),
            window,
            cx,
        )
    }
}

impl DiffModal {
    /// GIT-044…GIT-047: the selected file's header and diff.
    fn pane(&mut self, loaded: &Loaded, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let file = loaded.file();
        let (dir, name) = labels::split_path(&file.path);
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(38.))
            .px(px(16.))
            .border_b(px(0.5))
            .border_color(t.separator)
            .text_size(TEXT_MD)
            .whitespace_nowrap()
            .child(status_icon(file.status, status_color(file.status, &t)))
            .child({
                // `.diff-pane-path.selectable`: the folder and name copy as one path.
                let path = selectable::inline_selection(
                    SharedString::from(format!("diff-pane-path-{}", labels::file_key(file))),
                    window,
                    cx,
                );
                let dir_part = (!dir.is_empty()).then(|| {
                    div().text_color(t.text_secondary).child(selectable::selectable_part(
                        &path,
                        0,
                        dir.to_owned(),
                        Vec::new(),
                        cx,
                    ))
                });
                let name_part = selectable::selectable_part(&path, 1, name.to_owned(), Vec::new(), cx);
                selectable::selection_scope(&path, cx)
                    .flex()
                    .min_w_0()
                    .overflow_hidden()
                    .children(dir_part)
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(name_part))
            })
            .when_some(file.old_path.clone().filter(|p| !p.is_empty()), |d, old| {
                d.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(t.text_tertiary)
                        .text_size(TEXT_SM)
                        .child(format!("from {old}")),
                )
            });

        // GIT-045: what the pane says, in this order.
        let body: AnyElement = match loaded {
            Loaded::Error { error, .. } => notice("Couldn’t read this diff", Some(error.clone()), &t, window, cx),
            Loaded::Diff { diff, .. } if diff.binary == Some(true) => {
                notice("Binary file not shown", None, &t, window, cx)
            }
            Loaded::Diff { diff, .. } if diff.too_large == Some(true) => {
                notice("This diff is too large to show", None, &t, window, cx)
            }
            Loaded::Diff { parsed, .. } if parsed.summary.is_some() => {
                notice(parsed.summary.as_deref().unwrap_or_default(), None, &t, window, cx)
            }
            Loaded::Diff { parsed, .. } => self.lines(parsed, &labels::file_key(file), window, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(head)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// GIT-046, GIT-047: the diff's lines, drawn as they scroll into view.
    fn lines(&mut self, parsed: &Rc<ParsedDiff>, key: &str, window: &mut Window, cx: &mut App) -> AnyElement {
        let t = theme::tokens(cx);
        let count = parsed.lines.len();
        match &self.drawn {
            Some((drawn, shown)) if drawn == key && Rc::ptr_eq(shown, parsed) => {}
            // GIT-046: a re-read of the same file keeps its place…
            Some((drawn, shown)) if drawn == key => {
                let before = shown.lines.len();
                self.lines.splice(0..before, count);
            }
            // …another file starts at the top.
            _ => self.lines.reset(count),
        }
        self.drawn = Some((key.to_owned(), parsed.clone()));

        let mono = gpui::Font {
            family: fonts::mono().into(),
            ..gpui::Font::default()
        };
        let font_id = window.text_system().resolve_font(&mono);
        let ch = window.text_system().ch_advance(font_id, TEXT_SM).unwrap_or(px(7.2));
        let num_w = ch * parsed.number_width() as f32;
        let parsed = parsed.clone();
        // `.diff-text`: a drag selects across the lines' text, not their numbers.
        let selection = selectable::text_selection(SharedString::from(format!("diff-lines-{key}")), window, cx);
        let lines = selection.clone();
        selectable::selection_scope(&selection, cx)
            .size_full()
            .child(
                list(self.lines.clone(), move |ix, _, cx| {
                    let line = DrawnLine {
                        ix,
                        count: parsed.lines.len(),
                        num_w,
                        ch,
                    };
                    diff_line(&parsed.lines[ix], line, &t, &lines, cx)
                })
                .size_full(),
            )
            .into_any_element()
    }
}

/// The Changes list's empty and placeholder states (GIT-033, GIT-035).
fn empty(children: Vec<AnyElement>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(24.))
        .py(px(48.))
        .text_color(t.text_secondary)
        .text_center()
        .children(children)
        .into_any_element()
}

fn empty_title(text: &'static str) -> AnyElement {
    EmptyTitle(text).into_any_element()
}

/// An empty state's title, in the theme's primary text.
#[derive(IntoElement)]
struct EmptyTitle(&'static str);

impl gpui::RenderOnce for EmptyTitle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::tokens(cx);
        div()
            .font_family(fonts::DISPLAY)
            .text_size(TEXT_LG)
            .line_height(px(21.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.text_primary)
            .child(self.0)
    }
}

/// GIT-045: a message in the pane, with what went wrong under it
/// (`.diff-notice-detail.selectable`).
fn notice(title: &str, detail: Option<String>, t: &Tokens, window: &mut Window, cx: &mut App) -> AnyElement {
    let detail = detail.map(|detail| selectable::selectable("diff-notice-detail", detail, window, cx));
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(24.))
        .py(px(48.))
        .text_color(t.text_secondary)
        .text_center()
        .child(div().child(title.to_owned()))
        .when_some(detail, |d, detail| {
            d.child(
                div()
                    .max_w(px(520.))
                    .font_family(fonts::mono())
                    .text_size(TEXT_SM)
                    .text_color(t.text_tertiary)
                    .child(detail),
            )
        })
        .into_any_element()
}

/// GIT-037: the icon a status is drawn with.
fn status_icon(status: GitFileStatus, color: gpui::Rgba) -> impl IntoElement {
    let g = Glyph::from_name(labels::status_icon(status)).unwrap_or(Glyph::SquareDot);
    glyph(g, 15., 2., color)
}

/// GIT-037: its colour.
fn status_color(status: GitFileStatus, t: &Tokens) -> gpui::Rgba {
    match status {
        GitFileStatus::Modified => t.status_working,
        GitFileStatus::Added | GitFileStatus::Untracked => t.diff_add,
        GitFileStatus::Deleted | GitFileStatus::Conflicted => t.diff_del,
        GitFileStatus::Renamed => t.accent,
    }
}

/// GIT-036, GIT-038, GIT-040: a row of the list.
fn file_row(
    modal: &Entity<DiffModal>,
    i: usize,
    file: &GitFileChange,
    selected: bool,
    focused: bool,
    t: &Tokens,
) -> AnyElement {
    let (dir, name) = labels::split_path(&file.path);
    let lit = selected && focused;
    let text = if lit { t.text_on_accent } else { t.text_primary };
    let faint = |c: gpui::Rgba| if lit { alpha(t.text_on_accent, 0.85).into() } else { c };
    let modal = modal.clone();
    div()
        .id(("diff-file", i))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .pl(px(10.))
        .pr(px(8.))
        .rounded(RADIUS_SM)
        .text_size(TEXT_MD)
        .text_color(text)
        .map(|d| {
            if lit {
                d.bg(t.accent)
            } else if selected {
                d.bg(t.bg_pressed)
            } else {
                d.hover(|s| s.bg(t.bg_hover))
            }
        })
        .tooltip(native_tooltip(SharedString::from(labels::file_title(file))))
        // GIT-039: a press selects.
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            modal.update(cx, |m, cx| {
                let files = m.files(cx);
                m.selection.select(&files, i as isize);
                cx.notify();
            });
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .when(!dir.is_empty(), |d| {
                    d.child(
                        div()
                            .min_w_0()
                            .flex_shrink(1.)
                            .truncate()
                            .text_color(faint(t.text_secondary))
                            .child(dir.to_owned()),
                    )
                })
                .child(div().flex_none().max_w_full().truncate().child(name.to_owned())),
        )
        .child(status_icon(file.status, faint(status_color(file.status, t))))
        .into_any_element()
}

/// GIT-047: one line of the diff: old and new numbers, the sign, the text
/// (tabs 4 wide, wrapping anywhere), the changed part highlighted.
/// Where a diff line is drawn: its index of `count`, and the number and
/// sign columns' widths.
#[derive(Clone, Copy)]
struct DrawnLine {
    ix: usize,
    count: usize,
    num_w: Pixels,
    ch: Pixels,
}

fn diff_line(
    line: &DiffLine,
    drawn: DrawnLine,
    t: &Tokens,
    selection: &Entity<selectable::TextSelection>,
    cx: &mut App,
) -> AnyElement {
    let DrawnLine { ix, count, num_w, ch } = drawn;
    let (first, last) = (ix == 0, ix + 1 == count);
    let (bg, sign_color, word_bg) = match line.kind {
        DiffLineKind::Add => (Some(alpha(t.diff_add, 0.13)), t.diff_add, Some(alpha(t.diff_add, 0.34))),
        DiffLineKind::Del => (Some(alpha(t.diff_del, 0.11)), t.diff_del, Some(alpha(t.diff_del, 0.30))),
        DiffLineKind::Hunk => (Some(alpha(t.accent, 0.08)), t.text_tertiary, None),
        _ => (None, t.text_tertiary, None),
    };
    let text_color = match line.kind {
        DiffLineKind::Hunk => t.text_secondary,
        DiffLineKind::Note => t.text_tertiary,
        _ => t.text_primary,
    };
    let (text, changed) = expand_tabs(&line.text, line.changed);
    let mut highlights = Vec::new();
    if let (Some((start, end)), Some(word_bg)) = (changed, word_bg)
        && end > start
    {
        highlights.push((
            start..end,
            HighlightStyle {
                background_color: Some(word_bg),
                ..HighlightStyle::default()
            },
        ));
    }
    let styled = selectable::selectable_part(selection, ix, text, highlights, cx);
    let num = |n: Option<u64>| {
        div()
            .flex_none()
            .w(num_w)
            .flex()
            .justify_end()
            .text_color(t.text_tertiary)
            .children(n.map(|n| n.to_string()))
    };
    let row = div()
        .flex()
        .gap(px(10.))
        .min_h(px(20.))
        .pl(px(12.))
        .font_family(fonts::mono())
        .text_size(TEXT_SM)
        .line_height(px(20.))
        .font_features(fonts::tabular())
        .when_some(bg, Styled::bg)
        .child(num(line.old_no))
        .child(num(line.new_no))
        .child(
            div()
                .flex_none()
                .w(ch)
                .flex()
                .justify_end()
                .text_color(sign_color)
                .child(line.kind.sign()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pr(px(16.))
                .text_color(text_color)
                .when(line.kind == DiffLineKind::Note, Styled::italic)
                .child(styled),
        );
    div()
        .when(line.kind == DiffLineKind::Hunk && !first, |d| d.pt(px(8.)))
        .when(last, |d| d.pb(px(12.)))
        .child(row)
        .into_any_element()
}

/// Tabs to the next multiple of 4 columns (`tab-size: 4`), and the changed
/// range (UTF-16 units of the original) as byte offsets of the result.
fn expand_tabs(text: &str, changed: Option<(usize, usize)>) -> (String, Option<(usize, usize)>) {
    let mut out = String::with_capacity(text.len());
    let mut col = 0usize;
    let mut units = 0usize;
    let (mut start, mut end) = (None, None);
    let mark = |units: usize, len: usize, start: &mut Option<usize>, end: &mut Option<usize>| {
        if let Some((s, e)) = changed {
            if start.is_none() && units >= s {
                *start = Some(len);
            }
            if end.is_none() && units >= e {
                *end = Some(len);
            }
        }
    };
    for c in text.chars() {
        mark(units, out.len(), &mut start, &mut end);
        if c == '\t' {
            let n = 4 - col % 4;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
        units += c.len_utf16();
    }
    mark(units, out.len(), &mut start, &mut end);
    let changed = changed.map(|_| (start.unwrap_or(out.len()), end.unwrap_or(out.len())));
    debug_assert!(utf16_len(text) == units);
    (out, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tabs_and_keeps_the_changed_part() {
        assert_eq!(expand_tabs("a\tb", None), ("a   b".into(), None));
        assert_eq!(expand_tabs("\tx", Some((1, 2))), ("    x".into(), Some((4, 5))));
        // UTF-16 units in, bytes out.
        assert_eq!(expand_tabs("é = 1", Some((4, 5))), ("é = 1".into(), Some((5, 6))));
        assert_eq!(expand_tabs("ab", Some((2, 2))), ("ab".into(), Some((2, 2))));
    }
}
