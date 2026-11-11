//! The bubble (spec 11 BUBBLE): with the pointer tool out, a click on the
//! empty board opens a small field right there. What's typed becomes a
//! session, a sticky note or a view at that spot. Claude (Haiku) guesses
//! which as the typing pauses, and for a session, the model the ask needs;
//! Tab or a click on the chip picks for yourself. This file holds its state
//! and what it does; `view.rs` draws it and its cursor. The board's side
//! (`submit_from_bubble`) is in `board/bar.rs`.

mod view;

use std::time::Duration;

use gpui::component::input::{InputEvent, TextareaState};
use gpui::{AppContext as _, Context, Entity, EntityInputHandler as _, Focusable as _, Subscription, Task, Window};
use nodus_board::geometry::Point;
use nodus_domain::{BubbleGuess, BubbleRoute, SessionModel, js};

pub(crate) use view::cursor;

use crate::board::canvas::BoardCanvas;

/// BUBBLE-010: asked this long after the typing last changed, as the capture panel asks.
const GUESS_AFTER_MS: u64 = 700;
/// BUBBLE-010: shorter text isn't worth a guess.
const MIN_GUESS_CHARS: usize = 3;

/// The open bubble.
pub(crate) struct Bubble {
    /// Where it was opened, in board units: it points there, and what it
    /// makes lands there.
    pub at: Point,
    pub field: Entity<TextareaState>,
    /// The field's text, as typed.
    pub text: String,
    /// Claude's latest guess, and the text (trimmed) it was for.
    pub guess: Option<BubbleGuess>,
    pub guessed_for: String,
    pub guessing: bool,
    /// What the user picked, which stands over the guess until the text is cleared.
    pub choice: Option<BubbleRoute>,
    /// Return was pressed and it waits for the guess to go.
    pub sending: bool,
    /// The field has had the keyboard, as a frame saw it.
    pub had_focus: bool,
    /// Tells one bubble's elements from the last one's.
    pub key: usize,
    run: u64,
    timer: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Bubble {
    /// What Return makes now: the user's pick, else the guess for this
    /// text. None until there's either.
    pub fn route(&self) -> Option<BubbleRoute> {
        self.choice.or_else(|| {
            self.guess
                .as_ref()
                .filter(|_| self.guessed_for == js::trim(&self.text))
                .map(|g| g.route)
        })
    }

    /// What the chip shows (BUBBLE-006): the pick, else the latest guess,
    /// which stays on screen while the text changes until the next one.
    pub fn shown(&self) -> Option<BubbleRoute> {
        self.choice.or_else(|| self.guess.as_ref().map(|g| g.route))
    }
}

impl BoardCanvas {
    /// BUBBLE-001: opens the bubble at a window point, its field focused.
    pub(crate) fn open_bubble(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let p = self.to_flow(at);
        // BUBBLE-016: Return comes to the bubble's keys; ⇧Return is a new line.
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 5)
                .context_menu(false)
                .submit_on_enter(true)
                .placeholder("Ask Claude, jot a note, or describe a view")
        });
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, field, e: &InputEvent, window, cx| match e {
                InputEvent::Change => {
                    let value = field.read(cx).value().to_string();
                    this.bubble_changed(value, cx);
                }
                // BUBBLE-003: the keyboard went elsewhere.
                InputEvent::Blur if this.overlays.bubble.as_ref().is_some_and(|b| !b.sending) => {
                    this.close_bubble(Some(window), cx);
                }
                _ => {}
            }),
        ];
        self.overlays.bubble_key += 1;
        self.overlays.bubble = Some(Bubble {
            at: p,
            field: field.clone(),
            text: String::new(),
            guess: None,
            guessed_for: String::new(),
            guessing: false,
            choice: None,
            sending: false,
            had_focus: false,
            key: self.overlays.bubble_key,
            run: 0,
            timer: None,
            _subscriptions: subscriptions,
        });
        // Focused now, and again once drawn, as a label is.
        field.update(cx, |s, cx| s.focus(window, cx));
        window.on_next_frame(move |window, cx| {
            field.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.notify();
    }

    /// BUBBLE-003: closes it, stopping its guess; the board gets the keyboard
    /// back when the bubble had it.
    pub(crate) fn close_bubble(&mut self, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.take() else {
            return;
        };
        if let Some(window) = window
            && bubble.field.read(cx).focus_handle(cx).is_focused(window)
        {
            window.focus(&self.focus, cx);
        }
        if bubble.guessing {
            let nodus = self.state.read(cx).nodus.clone();
            cx.spawn(async move |_, _| nodus.capture().cancel_bubble().await)
                .detach();
        }
        cx.notify();
    }

    /// BUBBLE-010, BUBBLE-011: an edit. Each pause asks again until the user
    /// picks; clearing the text forgets the guess and the pick.
    fn bubble_changed(&mut self, value: String, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.as_mut() else {
            return;
        };
        bubble.text = value;
        bubble.timer = None;
        let trimmed = js::trim(&bubble.text).to_owned();
        if trimmed.is_empty() {
            bubble.run += 1;
            bubble.guess = None;
            bubble.guessed_for.clear();
            bubble.guessing = false;
            bubble.choice = None;
        } else if bubble.choice.is_none() && js::utf16_len(&trimmed) >= MIN_GUESS_CHARS && trimmed != bubble.guessed_for
        {
            bubble.timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(GUESS_AFTER_MS))
                    .await;
                let _ = this.update(cx, BoardCanvas::bubble_ask);
            }));
        }
        cx.notify();
    }

    /// BUBBLE-012: asks Claude about the text as it is; only the latest
    /// answer counts, and none keeps the guess on screen.
    fn bubble_ask(&mut self, cx: &mut Context<'_, Self>) {
        let project = self.board.as_ref().map(|b| b.project.name.clone()).unwrap_or_default();
        let Some(bubble) = self.overlays.bubble.as_mut() else {
            return;
        };
        bubble.timer = None;
        bubble.run += 1;
        bubble.guessing = true;
        let (run, key, text) = (bubble.run, bubble.key, bubble.text.clone());
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let answer = nodus.capture().guess_bubble(&text, &project).await.ok().flatten();
            let _ = this.update(cx, |c, cx| c.bubble_settle(key, run, answer, &text, cx));
        })
        .detach();
        cx.notify();
    }

    /// An answer to guess `run` of bubble `key`, about `text`. A bubble
    /// waiting to go goes now, on the guess, or as a session on Claude
    /// Code's own model when there's none (BUBBLE-014).
    fn bubble_settle(
        &mut self,
        key: usize,
        run: u64,
        answer: Option<BubbleGuess>,
        text: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(bubble) = self.overlays.bubble.as_mut().filter(|b| b.key == key && b.run == run) else {
            return;
        };
        bubble.guessing = false;
        if let Some(answer) = answer {
            bubble.guessed_for = js::trim(text).to_owned();
            bubble.guess = Some(answer);
        }
        if bubble.sending {
            let route = bubble.route();
            self.bubble_go(route, cx);
        }
        cx.notify();
    }

    /// BUBBLE-013, BUBBLE-014: Return. Goes on the pick or the guess for
    /// this text; otherwise waits for Claude's guess, asking now if it isn't
    /// already.
    pub(crate) fn bubble_submit(&mut self, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.as_mut() else {
            return;
        };
        if js::trim(&bubble.text).is_empty() || bubble.sending {
            return;
        }
        if let Some(route) = bubble.route() {
            self.bubble_go(Some(route), cx);
            return;
        }
        bubble.sending = true;
        if !bubble.guessing {
            self.bubble_ask(cx);
        }
        cx.notify();
    }

    /// BUBBLE-030: the bubble closes and what was typed is made where it was
    /// opened; `None` is a session on Claude Code's own model.
    fn bubble_go(&mut self, route: Option<BubbleRoute>, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.take() else {
            return;
        };
        let text = js::trim(&bubble.text).to_owned();
        let at = bubble.at;
        // The keyboard goes back to the board once the bubble's gone, if the
        // bubble still had it (one that waited may have lost it meanwhile).
        let (field, board) = (bubble.field.read(cx).focus_handle(cx), self.focus.clone());
        cx.defer(move |cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| {
                    if field.is_focused(window) {
                        window.focus(&board, cx);
                    }
                });
            }
        });
        self.submit_from_bubble(text, route, at, cx).detach();
        cx.notify();
    }

    /// BUBBLE-015: Tab, or a click on the chip, picks the next route after
    /// the one shown.
    pub(crate) fn bubble_cycle(&mut self, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.as_mut() else {
            return;
        };
        if bubble.sending {
            return;
        }
        bubble.choice = Some(match bubble.shown() {
            Some(shown) => shown.next(),
            None => BubbleRoute::Session(SessionModel::Sonnet),
        });
        bubble.timer = None;
        cx.notify();
    }

    /// The bubble's keys (BUBBLE-016), before the field's own editing; none
    /// while an input method composes. True when the key was the bubble's.
    pub(crate) fn bubble_key_down(
        &mut self,
        e: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(field) = self.overlays.bubble.as_ref().map(|b| b.field.clone()) else {
            return false;
        };
        if field.update(cx, |s, cx| s.marked_text_range(window, cx)).is_some() {
            return false;
        }
        let m = e.keystroke.modifiers;
        // SRCH-001: ⌘K opens search from here too, as from the bar.
        if e.keystroke.key == "k" && (m.platform || m.control) {
            return self.on_key_down(e, false, cx);
        }
        match e.keystroke.key.as_str() {
            "enter" if !m.shift && !m.platform && !m.control && !m.alt => {
                self.bubble_submit(cx);
                true
            }
            "escape" => {
                self.close_bubble(Some(window), cx);
                true
            }
            _ => false,
        }
    }

    /// Whether a click on the empty board would open the bubble, for its
    /// cursor (BUBBLE-005): the pointer tool, nothing held or open. Ink under
    /// the pointer shows its own cursor over this one.
    pub(crate) fn bubble_cursor_shown(&self, cx: &gpui::App) -> bool {
        self.board.is_some()
            && self.overlays.bubble.is_none()
            && self.overlays.ink.tool == crate::board::overlays::ink::InkTool::Select
            && self.gesture == crate::board::canvas::Gesture::Idle
            && !self.space
            && !self.overlay_open(cx)
    }
}
