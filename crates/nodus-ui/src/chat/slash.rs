//! The composer's `/` menu (CHAT-042…CHAT-046): the commands and skills
//! Claude Code offers in the session, narrowed as the name is typed, and
//! picked with the keys or the pointer. What's listed comes from
//! `nodus_transcripts::slash`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, CursorStyle, EntityInputHandler as _, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Point, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Window, div, point, px,
};
use nodus_transcripts::slash::{self, SlashCommand};

use super::look;
use super::view::ChatView;
use crate::components::tween::enter;
use crate::theme::tokens::{DUR_FAST, Tokens};
use crate::theme::{fonts, motion};

/// The rows the menu shows before it scrolls.
const ROWS: f32 = 8.;
/// Each row's height.
const ROW_H: f32 = 30.;
/// The menu's inset round its rows.
const PAD: f32 = 5.;
/// What was read from disk is read again when the menu opens after this long.
const FRESH: Duration = Duration::from_secs(10);
/// The name column fits names up to this many characters; longer ones are cut.
const NAME_MAX_CHARS: usize = 26;

/// The menu's state, per chat.
#[derive(Default)]
pub(crate) struct SlashMenu {
    /// The commands and skills on disk for the session, as last read.
    on_disk: Arc<[SlashCommand]>,
    read_at: Option<Instant>,
    reading: bool,
    /// The skills Claude Code listed in the transcript.
    listed: Arc<[SlashCommand]>,
    /// Everything the menu offers, made again when either of those changes.
    all: Option<Arc<[SlashCommand]>>,
    /// The highlighted row.
    active: usize,
    /// Esc closed the menu; typing opens it again.
    dismissed: bool,
    /// The pointer as it last moved over a row, so rows scrolling under a
    /// still pointer don't take the highlight from the keys.
    pointer: Option<Point<Pixels>>,
    scroll: ScrollHandle,
}

impl SlashMenu {
    fn all(&mut self) -> Arc<[SlashCommand]> {
        let Self {
            all, on_disk, listed, ..
        } = self;
        Arc::clone(all.get_or_insert_with(|| slash::merge(on_disk, listed).into()))
    }

    /// The skills the transcript listed, as the chat last read them.
    pub(crate) fn set_listed(&mut self, listed: Arc<[SlashCommand]>) {
        if !Arc::ptr_eq(&self.listed, &listed) {
            self.listed = listed;
            self.all = None;
        }
    }
}

impl ChatView {
    /// CHAT-042: what follows the slash while the composer holds a single
    /// word that starts with one.
    fn slash_typed(&self, cx: &App) -> Option<String> {
        let value = self.composer.read(cx).value();
        let word = value.strip_prefix('/')?;
        (!word.chars().any(char::is_whitespace)).then(|| word.to_owned())
    }

    /// CHAT-042, CHAT-044: the commands the menu shows now, best first; none
    /// while it's closed.
    pub(crate) fn slash_items(&mut self, window: &Window, cx: &App) -> Vec<SlashCommand> {
        if self.slash.dismissed || !self.composer_handle(cx).is_focused(window) {
            return Vec::new();
        }
        let Some(typed) = self.slash_typed(cx) else {
            return Vec::new();
        };
        let all = self.slash.all();
        slash::matching(&all, &typed).into_iter().cloned().collect()
    }

    /// The menu's commands while the keys are the menu's: none while an
    /// input method composes.
    fn slash_keyed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<SlashCommand> {
        let composing = self
            .composer
            .update(cx, |s, cx| s.marked_text_range(window, cx))
            .is_some();
        if composing {
            return Vec::new();
        }
        self.slash_items(window, cx)
    }

    /// CHAT-042: the composer's words changed: the menu opens again at its
    /// top, and what's on disk is read when it's old.
    pub(crate) fn slash_changed(&mut self, cx: &mut Context<'_, Self>) {
        self.slash.active = 0;
        self.slash.dismissed = false;
        self.slash.scroll.set_offset(point(px(0.), px(0.)));
        if self.slash_typed(cx).is_some() {
            self.slash_read(cx);
        }
    }

    /// CHAT-043: reads the commands and skills on disk for the session, off
    /// the main thread, unless they were read in the last [`FRESH`].
    fn slash_read(&mut self, cx: &mut Context<'_, Self>) {
        if self.slash.reading || self.slash.read_at.is_some_and(|at| at.elapsed() < FRESH) {
            return;
        }
        let nodus = self.state.read(cx).nodus.clone();
        let Some((home, folder)) = nodus.sessions().command_places(&self.project_id, &self.session_id) else {
            self.slash.read_at = Some(Instant::now());
            return;
        };
        self.slash.reading = true;
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { slash::on_disk(&home, &folder) }).await;
            let _ = this.update(cx, |this, cx| {
                this.slash.reading = false;
                this.slash.read_at = Some(Instant::now());
                if *this.slash.on_disk != *found {
                    this.slash.on_disk = found.into();
                    this.slash.all = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// CHAT-045: ↓ or ↑ moves the highlight, both wrapping. True when the
    /// menu took the key.
    pub(crate) fn slash_move(&mut self, down: bool, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let count = self.slash_keyed(window, cx).len();
        if count == 0 {
            return false;
        }
        let active = self.slash.active.min(count - 1);
        self.slash.active = if down {
            (active + 1) % count
        } else {
            (active + count - 1) % count
        };
        self.slash.scroll.scroll_to_item(self.slash.active);
        cx.notify();
        true
    }

    /// CHAT-045: Tab, or a click on a row, puts the command in the composer
    /// with a space after it, for what it takes. True when the menu took it.
    pub(crate) fn slash_complete(
        &mut self,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let items = self.slash_keyed(window, cx);
        let row = row.unwrap_or(self.slash.active);
        let Some(command) = items.get(row.min(items.len().saturating_sub(1))) else {
            return false;
        };
        let text = format!("/{} ", command.name);
        self.composer.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.insert(text, window, cx);
        });
        self.slash.active = 0;
        cx.notify();
        true
    }

    /// CHAT-045: Return runs the highlighted command: it's sent as if typed
    /// out in full (CHAT-020…CHAT-025). True when the menu took the key.
    pub(crate) fn slash_run(&mut self, now: bool, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let items = self.slash_keyed(window, cx);
        let Some(command) = items.get(self.slash.active.min(items.len().saturating_sub(1))) else {
            return false;
        };
        let text = format!("/{}", command.name);
        self.composer.update(cx, |s, cx| s.set_value("", window, cx));
        self.slash.active = 0;
        self.deliver(text, now, cx);
        true
    }

    /// CHAT-045: Esc closes the menu (and doesn't stop Claude). True when
    /// the menu was open.
    pub(crate) fn slash_dismiss(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.slash_items(window, cx).is_empty() {
            return false;
        }
        self.slash.dismissed = true;
        cx.notify();
        true
    }

    /// CHAT-046: the menu, its bottom 8px over the composer's top, as wide
    /// as the composer; it rises in over 120ms.
    pub(crate) fn render_slash_menu(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let items = self.slash_items(window, cx);
        if items.is_empty() {
            return None;
        }
        let active = self.slash.active.min(items.len() - 1);
        let id = cx.entity_id();
        let rise = enter(window, cx, ("chat-slash", id), DUR_FAST, motion::ease_out);
        // The names line up in a column as wide as the longest (mono, so
        // each character is 0.6em).
        let longest = items
            .iter()
            .map(|c| c.name.chars().count() + 1)
            .max()
            .unwrap_or(0)
            .min(NAME_MAX_CHARS);
        let name_w = longest as f32 * 12.5 * 0.6 + 4.;
        let me = cx.entity().downgrade();
        let rows = items.iter().enumerate().map(|(i, command)| {
            let (hover_me, click_me) = (me.clone(), me.clone());
            let label = command.source.label();
            let description = if i == active && !command.hint.is_empty() {
                div()
                    .flex()
                    .gap(self.px(8.))
                    .child(
                        div()
                            .flex_none()
                            .font_family(fonts::mono())
                            .text_color(t.text_tertiary)
                            .child(SharedString::from(command.hint.clone())),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(SharedString::from(command.description.clone())),
                    )
                    .into_any_element()
            } else {
                SharedString::from(command.description.clone()).into_any_element()
            };
            div()
                .id(("chat-slash-row", i))
                .flex()
                .flex_none()
                .items_center()
                .gap(self.px(12.))
                .h(self.px(ROW_H))
                .px(self.px(10.))
                .rounded(self.px(8.))
                .when(i == active, |d| d.bg(t.bg_selected))
                .on_mouse_move(move |e, _, cx| {
                    let _ = hover_me.update(cx, |this, cx| {
                        if this.slash.pointer != Some(e.position) && this.slash.active != i {
                            this.slash.active = i;
                            cx.notify();
                        }
                        this.slash.pointer = Some(e.position);
                    });
                })
                .on_click(move |_, window, cx| {
                    let _ = click_me.update(cx, |this, cx| this.slash_complete(Some(i), window, cx));
                })
                .child(
                    div()
                        .flex_none()
                        .w(self.px(name_w))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(fonts::mono())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text_primary)
                        .child(SharedString::from(format!("/{}", command.name))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(t.text_secondary)
                        .child(description),
                )
                .when(!label.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_size(self.px(11.))
                            .text_color(t.text_tertiary)
                            .child(SharedString::from(label.to_owned())),
                    )
                })
        });
        let list = div()
            .id("chat-slash-list")
            .flex()
            .flex_col()
            .max_h(self.px(ROWS * ROW_H + 2. * PAD))
            .p(self.px(PAD))
            .overflow_y_scroll()
            .track_scroll(&self.slash.scroll)
            .children(rows);
        let card = div()
            .id("chat-slash")
            .occlude()
            .cursor(CursorStyle::Arrow)
            .rounded(self.px(14.))
            .border_1()
            .border_color(t.border)
            .bg(look::field_bg(t))
            .shadow(t.lg())
            .text_size(self.px(12.5))
            .line_height(self.px(16.))
            .opacity(rise)
            .translate(px(0.), self.px(6. * (1. - rise)))
            // Keeps the keyboard in the composer.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .child(list);
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_full()
                .pb(self.px(8.))
                .child(card)
                .into_any_element(),
        )
    }
}
