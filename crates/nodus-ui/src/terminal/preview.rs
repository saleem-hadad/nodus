//! The transcript preview (`TranscriptPreview.tsx`; TRANS-001…TRANS-024): an
//! archived (or not live) session's conversation, read-only, with the search
//! it was opened from marked, and `Restore & Open`.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Bounds, Context, Entity, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Pixels, Render, ScrollHandle, Styled, Subscription, Task, Window, div, point, px, relative,
};
use nodus_domain::{NodePatch, TranscriptMessage};

use super::actions::{self, PreviewFind, card};
use super::messages::{MessageProps, empty_note, message, skeleton};
use super::text::{format_range, found_index, messages_label};
use crate::components::button;
use crate::components::icon::Icon;
use crate::components::modal::Modal;
use crate::components::scroll::scroll_area;
use crate::format::card_title;
use crate::parity;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// What reading the transcript has given so far.
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loading,
    Error(String),
    Ready(Vec<TranscriptMessage>),
}

pub struct TranscriptPreview {
    state: Entity<AppState>,
    pub session_id: String,
    project_id: String,
    find: Option<PreviewFind>,
    terms: Vec<String>,
    load: Load,
    found: Option<usize>,
    expanded: HashSet<usize>,
    all_calls: HashSet<usize>,
    scroll: ScrollHandle,
    /// The found message's box, once drawn, and whether the first scroll happened.
    found_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scrolled: bool,
    focus: gpui::FocusHandle,
    _fetch: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TranscriptPreview {
    pub fn new(
        state: Entity<AppState>,
        session_id: String,
        project_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let find = actions::preview_find(cx);
        let terms = find
            .as_ref()
            .map(|f| nodus_transcripts::search::query_terms(&f.query))
            .unwrap_or_default();
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        let mut this = Self {
            state,
            session_id,
            project_id,
            find,
            terms,
            load: Load::Loading,
            found: None,
            expanded: HashSet::new(),
            all_calls: HashSet::new(),
            scroll: ScrollHandle::new(),
            found_bounds: Rc::new(Cell::new(None)),
            scrolled: false,
            focus,
            _fetch: None,
            _subscriptions: subscriptions,
        };
        this.fetch(cx);
        this
    }

    /// TRANS-004, TRANS-005: read once, again on Try again.
    fn fetch(&mut self, cx: &mut Context<'_, Self>) {
        self.load = Load::Loading;
        self.scrolled = false;
        let nodus = self.state.read(cx).nodus.clone();
        let (project, session) = (self.project_id.clone(), self.session_id.clone());
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let result = nodus.sessions().transcript(&project, &session).await;
            let _ = this.update(cx, |this, cx| {
                this.load = match result {
                    Ok(messages) => Load::Ready(messages),
                    Err(e) => Load::Error(e.ipc_message()),
                };
                this.found = match (&this.load, &this.find) {
                    (Load::Ready(messages), Some(find)) => found_index(messages, find, &this.terms),
                    _ => None,
                };
                // TRANS-011: the found message starts expanded.
                if let Some(i) = this.found {
                    this.expanded.insert(i);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// TRANS-021: back on the board beside the archive, then opened.
    fn restore(&mut self, cx: &mut Context<'_, Self>) {
        let Some((x, y)) = self
            .state
            .read(cx)
            .board
            .as_ref()
            .map(|b| super::text::free_slot_beside_archive(&b.archive, &b.cards, &self.session_id))
        else {
            return;
        };
        actions::patch_cards(
            &self.state,
            vec![NodePatch {
                session_id: self.session_id.clone(),
                archived: Some(false),
                x: Some(x),
                y: Some(y),
                ..Default::default()
            }],
            cx,
        );
        actions::close_preview(&self.state, cx);
        actions::open_terminal(&self.state, &self.session_id, cx);
    }

    /// TRANS-008: the found message centred, else the latest messages.
    fn first_scroll(&mut self, cx: &mut Context<'_, Self>) {
        if self.scrolled || !matches!(self.load, Load::Ready(_)) {
            return;
        }
        match self.found {
            Some(_) => {
                let Some(found) = self.found_bounds.get() else {
                    cx.notify();
                    return;
                };
                let view = self.scroll.bounds();
                let offset = self.scroll.offset();
                let top_in_content = found.top() - view.top() - offset.y;
                let target = top_in_content - (view.size.height - found.size.height) / 2.;
                let max = self.scroll.max_offset().y;
                self.scroll
                    .set_offset(point(px(0.), -target.clamp(px(0.), max.max(px(0.)))));
            }
            None => self.scroll.scroll_to_bottom(),
        }
        self.scrolled = true;
        cx.notify();
    }

    fn header(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let card = card(self.state.read(cx), &self.session_id);
        let meta = card.as_ref().and_then(|c| c.meta.clone());
        let count = match &self.load {
            Load::Ready(messages) => Some(messages.len() as u32),
            _ => meta.as_ref().map(|m| m.message_count),
        };
        let now = parity::now_ms(cx);
        let this = cx.entity();
        let state = self.state.clone();
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .flex_none()
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
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(card_title(card.as_ref())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(14.))
                            .gap_y(px(4.))
                            .mt(px(3.))
                            .text_size(TEXT_SM)
                            .text_color(t.text_secondary)
                            .when_some(meta.as_ref(), |d, m| {
                                d.child(format_range(m.created_at, m.updated_at, now))
                            })
                            .when_some(count, |d, n| d.child(messages_label(n)))
                            .when_some(meta.as_ref().and_then(|m| m.git_branch.clone()), |d, branch| {
                                d.child(
                                    div()
                                        .font_family(super::MONO)
                                        .text_size(TEXT_XS)
                                        .line_height(px(18.))
                                        .text_color(t.text_tertiary)
                                        .child(branch),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .flex_none()
                    .child(
                        button("tp-restore")
                            .primary()
                            .icon(Icon::ArchiveRestore, 14., 2.)
                            .label("Restore & Open")
                            .on_click(move |_, _, cx| this.update(cx, |p, cx| p.restore(cx))),
                    )
                    .child(
                        button("tp-close")
                            .ghost()
                            .square()
                            .icon(Icon::X, 16., 2.)
                            .tooltip("Close (Esc)")
                            .on_click(move |_, _, cx| actions::close_preview(&state, cx)),
                    ),
            )
            .into_any_element()
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let this = cx.entity();
        let content: AnyElement = match &self.load {
            Load::Loading => skeleton(cx),
            Load::Error(message) => {
                let retry = this.clone();
                empty_note(
                    "Couldn’t read this transcript.",
                    Some(message),
                    Some(
                        button("tp-retry")
                            .label("Try again")
                            .on_click(move |_, _, cx| retry.update(cx, |p, cx| p.fetch(cx)))
                            .into_any_element(),
                    ),
                    cx,
                )
            }
            Load::Ready(messages) if messages.is_empty() => empty_note("No messages in this session.", None, None, cx),
            Load::Ready(messages) => {
                let snippet = self
                    .find
                    .as_ref()
                    .and_then(|f| f.snippet.as_ref())
                    .filter(|s| s.kind == nodus_domain::SearchSnippetKind::Tool);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .pt(px(12.))
                    .children(messages.iter().enumerate().map(|(i, m)| {
                        let found = self.found == Some(i);
                        let (toggle, all) = (this.clone(), this.clone());
                        let bounds = self.found_bounds.clone();
                        message(
                            MessageProps {
                                id: ("tp-msg", i).into(),
                                message: m,
                                author: None,
                                highlight: &self.terms,
                                found,
                                found_call: if found { snippet } else { None },
                                expanded: self.expanded.contains(&i),
                                all_calls: self.all_calls.contains(&i),
                                on_toggle: Rc::new(move |_, cx| {
                                    toggle.update(cx, |p, cx| {
                                        if !p.expanded.remove(&i) {
                                            p.expanded.insert(i);
                                        }
                                        cx.notify();
                                    });
                                }),
                                on_all_calls: Rc::new(move |_, cx| {
                                    all.update(cx, |p, cx| {
                                        p.all_calls.insert(i);
                                        cx.notify();
                                    });
                                }),
                                on_bounds: found.then(|| {
                                    Rc::new(move |b, _: &mut gpui::App| bounds.set(Some(b))) as Rc<dyn Fn(_, &mut _)>
                                }),
                            },
                            cx,
                        )
                    }))
                    .into_any_element()
            }
        };
        scroll_area("tp-body", &self.scroll, [8., 22., 24., 22.], content, window, cx)
    }
}

impl Render for TranscriptPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        // min(760px, 92vw) × 80vh.
        let width = px(760.).min(viewport.width * 0.92);
        let height = viewport.height * 0.8;
        if !self.scrolled && matches!(self.load, Load::Ready(_)) {
            let this = cx.entity();
            window.on_next_frame(move |_, cx| this.update(cx, |p, cx| p.first_scroll(cx)));
        }
        let header = self.header(cx);
        let body = self.body(window, cx);
        let state = self.state.clone();
        let close = self.state.clone();
        let content = div()
            .id("tp-panel")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .w_full()
            .h(height)
            .line_height(relative(1.4))
            // TRANS-003: Escape closes it.
            .on_key_down(move |e: &KeyDownEvent, _, cx| {
                let m = &e.keystroke.modifiers;
                let command = if cfg!(target_os = "macos") {
                    m.platform
                } else {
                    m.control
                };
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    actions::close_preview(&close, cx);
                } else if e.keystroke.key == "k" && command && !m.shift && !m.alt {
                    // TRANS-024: back to the search it came from.
                    cx.stop_propagation();
                    actions::close_preview(&close, cx);
                    let nodus = close.read(cx).nodus.clone();
                    nodus.emit(nodus_domain::Event::Menu(nodus_domain::MenuCommand::Search));
                }
            })
            .child(header)
            .child(body);
        Modal::new("tp-modal", width, move |_, cx| actions::close_preview(&state, cx)).render(content, window, cx)
    }
}
