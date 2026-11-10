//! The board's prompt bar (`Sidekick.tsx`, spec 11 BAR): what's typed or
//! said starts a session; `/view` and `/note` make those instead; with one
//! session selected it's sent into that session, and with more selected
//! they go to the new session as context. This file holds its state and
//! what it does; `view.rs` draws it, `popovers.rs` the slash menu and the
//! colour picker. The board's side (`submit_from_bar`, `bar_into`,
//! `bar_context_count`) is in `board/bar.rs`.

mod keys;
mod popovers;
mod text;
mod view;
mod voice;

use gpui::component::input::{InputEvent, TextareaState};
use gpui::{AppContext as _, Context, Entity, Focusable as _, Subscription, Window};
use nodus_board::compose::{SlashCommand, Target, match_commands, take_command};
use nodus_domain::{FolderColor, js};

use crate::board::bar::Submission;
use crate::board::canvas::BoardCanvas;
use crate::board::layout::card_title;

/// The bar's state, per board mount (BAR-041).
pub(crate) struct Bar {
    pub field: Option<Entity<TextareaState>>,
    /// BAR-005: spelling (the textarea's `spellCheck`).
    pub spell: Option<Entity<crate::components::spellcheck::Spellcheck>>,
    /// The field's text, as last seen.
    pub text: String,
    pub target: Option<Target>,
    /// BAR-022: yellow (`NOTE_COLOR`) each time the board mounts.
    pub color: FolderColor,
    pub focused: bool,
    /// The highlighted command while the slash menu is open.
    pub active: usize,
    /// Esc closed the slash menu; typing opens it again.
    pub menu_dismissed: bool,
    /// The selected session the user chose not to send into.
    pub dropped: Option<String>,
    /// The `into` session last seen, to forget `dropped` when it changes.
    pub last_into: Option<String>,
    pub picking: bool,
    /// The colour picker's and the note swatch's boxes, as last drawn.
    pub picker_area: [Option<gpui::Bounds<gpui::Pixels>>; 2],
    pub sending: bool,
    /// The board's `/` key asked for the keyboard, with `/` typed.
    pub focus_request: Option<Option<String>>,
    /// The bar's mic (spec 05's `voice::Mic`).
    pub mic: Option<Entity<crate::voice::Mic>>,
    /// What was said, waiting to land in the field.
    pub heard: Vec<String>,
    /// A submission went through: the field empties on the next frame.
    pub clear_pending: bool,
    /// The field's value is being set by the bar, not typed.
    setting: bool,
    pub placeholder: String,
    /// The mic's phase isn't idle (BAR-004), as of this frame.
    pub voice_busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            field: None,
            spell: None,
            text: String::new(),
            target: None,
            color: FolderColor::Yellow,
            focused: false,
            active: 0,
            menu_dismissed: false,
            dropped: None,
            last_into: None,
            picking: false,
            picker_area: [None, None],
            sending: false,
            focus_request: None,
            mic: None,
            heard: Vec::new(),
            clear_pending: false,
            setting: false,
            placeholder: String::new(),
            voice_busy: false,
            _subscriptions: Vec::new(),
        }
    }
}

/// What the bar shows for the session it sends into.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SendsInto {
    pub session_id: String,
    pub title: String,
}

impl BoardCanvas {
    /// The field, made the first time the bar is drawn: 1 line growing to
    /// 5, then scrolling (BAR-005).
    pub(crate) fn bar_field(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Entity<TextareaState> {
        if let Some(field) = &self.overlays.bar.field {
            return field.clone();
        }
        // BAR-009: the field's own Return binding runs before `bar_key_down`
        // sees the key; submit_on_enter makes it pass plain Return on (to
        // send, or pick the menu's row) and keep ⇧Return as the newline.
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 5)
                .context_menu(false)
                .submit_on_enter(true)
        });
        let sub = cx.subscribe_in(&field, window, |this, field, e: &InputEvent, window, cx| match e {
            InputEvent::Change => {
                let value = field.read(cx).value().to_string();
                this.bar_changed(value, window, cx);
            }
            InputEvent::Focus => {
                this.overlays.bar.focused = true;
                cx.notify();
            }
            InputEvent::Blur => {
                // BAR-016: blurring resets the menu's dismissal.
                this.overlays.bar.focused = false;
                this.overlays.bar.menu_dismissed = false;
                cx.notify();
            }
            _ => {}
        });
        self.overlays.bar.field = Some(field.clone());
        self.overlays.bar.spell = Some(crate::components::spellcheck::spellcheck(&field, false, window, cx));
        self.overlays.bar._subscriptions.push(sub);
        field
    }

    /// BAR-018: an edit; `/view ` or `/note ` typed out becomes a token.
    fn bar_changed(&mut self, value: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let bar = &mut self.overlays.bar;
        if bar.setting {
            bar.text = value;
            return;
        }
        let typed = if bar.target.is_none() {
            take_command(&value)
        } else {
            None
        };
        match typed {
            Some((target, rest)) => {
                bar.target = Some(target);
                self.bar_set_text(rest, window, cx);
            }
            None => self.overlays.bar.text = value,
        }
        // BAR-015, BAR-016: every edit resets the highlight and reopens the menu.
        self.overlays.bar.active = 0;
        self.overlays.bar.menu_dismissed = false;
        cx.notify();
    }

    /// Sets the field's text from the bar, the caret at the end.
    pub(crate) fn bar_set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.overlays.bar.text = text.clone();
        let Some(field) = self.overlays.bar.field.clone() else {
            return;
        };
        self.overlays.bar.setting = true;
        field.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            let end = s.value().len();
            s.set_selected_range(end..end, cx);
        });
        self.overlays.bar.setting = false;
    }

    /// BAR-024, BAR-020: the session the bar sends into, unless dropped or
    /// a target is set.
    pub(crate) fn bar_sends_into(&self) -> Option<SendsInto> {
        let id = self.bar_into()?;
        let bar = &self.overlays.bar;
        if bar.target.is_some() || bar.dropped.as_ref() == Some(&id) {
            return None;
        }
        let card = self.cards().iter().find(|c| c.node.session_id == id)?;
        Some(SendsInto {
            title: card_title(card),
            session_id: id,
        })
    }

    /// BAR-020: a new or re-made selection is sent into again.
    pub(crate) fn bar_follow_into(&mut self) {
        let into = self.bar_into();
        if into != self.overlays.bar.last_into {
            self.overlays.bar.last_into = into;
            self.overlays.bar.dropped = None;
        }
    }

    /// BAR-014: the commands the slash menu offers now.
    pub(crate) fn bar_commands(&self) -> Vec<SlashCommand> {
        let bar = &self.overlays.bar;
        if bar.target.is_some() || bar.menu_dismissed || !bar.focused {
            return Vec::new();
        }
        match_commands(&bar.text)
    }

    /// BAR-004.
    pub(crate) fn bar_open(&self) -> bool {
        // `voice_busy` is the mic's phase not idle, as of this frame.
        let bar = &self.overlays.bar;
        bar.focused
            || !bar.text.is_empty()
            || bar.target.is_some()
            || self.bar_sends_into().is_some()
            || self.overlays.bar.voice_busy
            || self.bar_context_count() > 0
    }

    /// BAR-017: a command picked from the menu.
    pub(crate) fn bar_pick(&mut self, target: Target, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.overlays.bar.target = Some(target);
        self.bar_set_text(String::new(), window, cx);
        self.overlays.bar.active = 0;
        cx.notify();
    }

    /// BAR-019: a token's ×.
    pub(crate) fn bar_clear_target(&mut self, cx: &mut Context<'_, Self>) {
        self.overlays.bar.target = None;
        self.overlays.bar.picking = false;
        cx.notify();
    }

    pub(crate) fn bar_drop_into(&mut self, cx: &mut Context<'_, Self>) {
        self.overlays.bar.dropped = self.bar_sends_into().map(|i| i.session_id);
        cx.notify();
    }

    /// BAR-022: a press anywhere but the picker and its swatch closes it
    /// (seen before anything else handles it).
    pub(crate) fn bar_outside_press(&mut self, at: gpui::Point<gpui::Pixels>, cx: &mut Context<'_, Self>) {
        let bar = &mut self.overlays.bar;
        if bar.picking && !bar.picker_area.iter().flatten().any(|b| b.contains(&at)) {
            bar.picking = false;
            cx.notify();
        }
    }

    /// BAR-033: the board's `/`: the keyboard to the bar, with `/` typed
    /// when it's empty and no target is set.
    pub(crate) fn bar_focus_from_board(&mut self, start: Option<String>, cx: &mut Context<'_, Self>) {
        self.overlays.bar.focus_request = Some(start);
        cx.notify();
    }

    /// Applies a focus the board asked for, once drawn with a window.
    pub(crate) fn bar_take_focus_request(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(start) = self.overlays.bar.focus_request.take() else {
            return;
        };
        let Some(field) = self.overlays.bar.field.clone() else {
            return;
        };
        if let Some(start) = start
            && self.overlays.bar.text.is_empty()
            && self.overlays.bar.target.is_none()
        {
            // Typed, so the slash menu opens.
            self.bar_changed(start.clone(), window, cx);
            self.bar_set_text(start, window, cx);
        }
        field.update(cx, |s, cx| s.focus(window, cx));
        self.overlays.bar.focused = true;
        cx.notify();
    }

    /// Focuses the field (the pill's background, a picked colour).
    pub(crate) fn bar_focus_field(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(field) = &self.overlays.bar.field {
            let handle = field.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    /// BAR-009…BAR-012: Return.
    pub(crate) fn bar_submit(&mut self, cx: &mut Context<'_, Self>) {
        let words = js::trim(&self.overlays.bar.text).to_owned();
        let menu_open = !self.bar_commands().is_empty();
        if words.is_empty()
            || self.bar_phase(cx) != nodus_voice::controller::Phase::Idle
            || self.overlays.bar.sending
            || menu_open
        {
            return;
        }
        let submission = match self.overlays.bar.target {
            Some(Target::View) => Submission::View { text: words },
            Some(Target::Note) => Submission::Note {
                text: words,
                color: self.overlays.bar.color,
            },
            None => match self.bar_sends_into() {
                Some(into) => Submission::Into {
                    session_id: into.session_id,
                    text: words,
                },
                None => Submission::Session { text: words },
            },
        };
        self.overlays.bar.sending = true;
        cx.notify();
        let done = self.submit_from_bar(submission, cx);
        cx.spawn(async move |this, cx| {
            let done = done.await;
            let _ = this.update(cx, |c, cx| {
                c.overlays.bar.sending = false;
                if done {
                    // BAR-012: the voice notice goes too.
                    if let Some(mic) = &c.overlays.bar.mic {
                        mic.read(cx).dismiss();
                    }
                    c.overlays.bar.target = None;
                    c.overlays.bar.picking = false;
                    c.overlays.bar.clear_pending = true;
                }
                cx.notify();
            });
        })
        .detach();
    }
}
