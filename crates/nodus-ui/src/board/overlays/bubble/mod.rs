//! Asking Claude on the spot (spec 11 BUBBLE): two of the dock's tools.
//! With the bubble out, a click on the board opens a small field there;
//! with the mic out, a click starts speaking there and the next click sends
//! it. Either way, what's sent goes at once: a marker waits on the spot
//! while Claude (Haiku) decides in the background whether it's a session
//! (and on which model), a sticky note or a view, which then lands with its
//! top left on the spot. This file holds the state and what it does;
//! `view.rs` draws the bubble, the markers, the mic's pill and the cursors.
//! The board's side (`submit_from_bubble`) is in `board/bar.rs`.

mod view;

use gpui::component::input::{InputEvent, TextareaState};
use gpui::{AppContext as _, Context, Entity, EntityInputHandler as _, Focusable as _, Subscription, Window};
use nodus_board::geometry::Point;
use nodus_domain::js;
use nodus_voice::controller::Phase;

pub(crate) use view::{cursor, mic_cursor};

use crate::board::canvas::BoardCanvas;
use crate::voice::Mic;

/// The open bubble.
pub(crate) struct Bubble {
    /// Where it was opened, in board units: it points there, and what it
    /// makes lands there.
    pub at: Point,
    pub field: Entity<TextareaState>,
    /// The field's text, as typed.
    pub text: String,
    /// The field has had the keyboard, as a frame saw it.
    pub had_focus: bool,
    /// Tells one bubble's elements from the last one's.
    pub key: usize,
    _subscriptions: Vec<Subscription>,
}

/// What a bubble shows, kept for its exit once it's closed or sent
/// (MOTION-011).
#[derive(Clone)]
pub(crate) struct BubbleLook {
    pub at: Point,
    pub key: usize,
    pub text: String,
}

impl Bubble {
    pub fn look(&self) -> BubbleLook {
        BubbleLook {
            at: self.at,
            key: self.key,
            text: self.text.clone(),
        }
    }
}

/// Something sent from the board, while Claude decides where it goes
/// (BUBBLE-030).
pub(crate) struct Asking {
    pub id: usize,
    pub at: Point,
    pub text: String,
}

impl BoardCanvas {
    /// BUBBLE-001, BUBBLE-002: a press with the bubble out. A bubble with
    /// words in it keeps them and takes the keyboard back; an empty one
    /// moves here.
    pub(crate) fn bubble_press(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(bubble) = &self.overlays.bubble
            && !js::trim(&bubble.text).is_empty()
        {
            let field = bubble.field.clone();
            field.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        self.close_bubble(None, cx);
        let p = self.to_flow(at);
        self.open_bubble(p, window, cx);
    }

    /// Opens the bubble at a board point, its field focused.
    fn open_bubble(&mut self, at: Point, window: &mut Window, cx: &mut Context<'_, Self>) {
        // BUBBLE-016: Return comes to the bubble's keys; ⇧Return is a new line.
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 5)
                .context_menu(false)
                .submit_on_enter(true)
                .placeholder("Ask Claude, jot a note, or describe a view")
        });
        let subscriptions = vec![cx.subscribe(&field, |this, field, e: &InputEvent, cx| {
            if let InputEvent::Change = e
                && let Some(bubble) = this.overlays.bubble.as_mut()
            {
                bubble.text = field.read(cx).value().to_string();
                cx.notify();
            }
        })];
        self.overlays.bubble_key += 1;
        self.overlays.bubble = Some(Bubble {
            at,
            field: field.clone(),
            text: String::new(),
            had_focus: false,
            key: self.overlays.bubble_key,
            _subscriptions: subscriptions,
        });
        // Focused now, and again once drawn, as a label is.
        field.update(cx, |s, cx| s.focus(window, cx));
        window.on_next_frame(move |window, cx| {
            field.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.notify();
    }

    /// BUBBLE-003: closes it and drops its words; the board gets the
    /// keyboard back when the bubble had it.
    pub(crate) fn close_bubble(&mut self, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.take() else {
            return;
        };
        self.overlays.bubble_leaving = crate::components::presence::leave(bubble.look(), cx);
        if let Some(window) = window
            && bubble.field.read(cx).focus_handle(cx).is_focused(window)
        {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// BUBBLE-003: an empty bubble goes when the keyboard or the tool does;
    /// one with words stays until it's sent or Esc.
    pub(crate) fn close_empty_bubble(&mut self, cx: &mut Context<'_, Self>) {
        if self
            .overlays
            .bubble
            .as_ref()
            .is_some_and(|b| js::trim(&b.text).is_empty())
        {
            self.close_bubble(None, cx);
        }
    }

    /// BUBBLE-013: Return sends it, straight away: the bubble closes, and
    /// Claude decides where it goes in the background.
    pub(crate) fn bubble_submit(&mut self, cx: &mut Context<'_, Self>) {
        let Some(bubble) = self.overlays.bubble.as_ref() else {
            return;
        };
        let text = js::trim(&bubble.text).to_owned();
        if text.is_empty() {
            return;
        }
        let Some(bubble) = self.overlays.bubble.take() else {
            return;
        };
        self.overlays.bubble_leaving = crate::components::presence::leave(bubble.look(), cx);
        // The keyboard goes back to the board, if the bubble still had it.
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
        self.ask_claude(text, bubble.at, cx);
        cx.notify();
    }

    /// BUBBLE-030: a marker waits at `at` while Claude decides where `text`
    /// goes, then it's made there; with no answer, it's a session on Claude
    /// Code's own model. Asks run side by side, and each keeps its board
    /// until it's made, should the project change meanwhile.
    pub(crate) fn ask_claude(&mut self, text: String, at: Point, cx: &mut Context<'_, Self>) {
        self.overlays.asking_key += 1;
        let id = self.overlays.asking_key;
        self.overlays.asking.push(Asking {
            id,
            at,
            text: text.clone(),
        });
        let project = self.board.as_ref().map(|b| b.project.name.clone()).unwrap_or_default();
        let nodus = self.state.read(cx).nodus.clone();
        let canvas = cx.entity();
        cx.spawn(async move |_, cx| {
            let route = nodus.capture().route_bubble(&text, &project).await.ok().flatten();
            let made = canvas.update(cx, |c, cx| {
                c.overlays.asking.retain(|a| a.id != id);
                cx.notify();
                c.submit_from_bubble(text, route.map(|g| g.route), at, cx)
            });
            made.await;
            drop(canvas);
        })
        .detach();
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

    // ---- the mic ------------------------------------------------------------------------

    /// The mic tool's mic, made the first time it's pressed: what it hears
    /// is asked at the spot the recording started (BUBBLE-042).
    fn voice_mic(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Entity<Mic> {
        if let Some(mic) = &self.overlays.voice {
            return mic.clone();
        }
        let nodus = self.state.read(cx).nodus.clone();
        let me = cx.entity().downgrade();
        let mic = cx.new(|cx| {
            Mic::new(
                &nodus,
                self.project_id.clone(),
                std::rc::Rc::new(move |said: String, cx: &mut gpui::App| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, |c, cx| {
                            if let Some(at) = c.overlays.voice_at.take()
                                && !js::trim(&said).is_empty()
                            {
                                c.ask_claude(js::trim(&said).to_owned(), at, cx);
                            }
                        });
                    }
                }),
                window,
                cx,
            )
        });
        cx.observe(&mic, |_, _, cx| cx.notify()).detach();
        self.overlays.voice = Some(mic.clone());
        mic
    }

    /// BUBBLE-040, BUBBLE-041: a press with the mic out starts listening
    /// there; the next press, anywhere, stops and sends it.
    pub(crate) fn voice_press(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mic = self.voice_mic(window, cx);
        let m = mic.read(cx);
        match m.phase() {
            Phase::Idle => {
                self.overlays.voice_at = Some(self.to_flow(at));
                m.record();
            }
            Phase::Recording => m.stop(),
            Phase::Opening | Phase::Transcribing => {}
        }
        cx.notify();
    }

    /// BUBBLE-044: leaving the mic tool stops a recording, which sends
    /// nothing; one already being written down still goes.
    pub(crate) fn voice_cancel(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(mic) = &self.overlays.voice {
            let m = mic.read(cx);
            if matches!(m.phase(), Phase::Opening | Phase::Recording) {
                m.cancel();
            }
        }
    }
}
