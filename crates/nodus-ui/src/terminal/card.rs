//! An expanded card's terminal, for the board's cards (`ExpandedCard.tsx`'s
//! use of `useAttachedTerminal`; the card's chrome, start and wake rules are
//! the board-cards track's, XCARD-019…XCARD-028).
//!
//! [`CardTerminals`] has the shape of board-cards' `TerminalHost` trait
//! (`board/cards/mod.rs`), so plugging it in is one forwarding impl:
//!
//! ```ignore
//! impl board::cards::TerminalHost for CardTerminals {
//!     fn needs_open(&self, id: &str) -> bool { CardTerminals::needs_open(self, id) }
//!     fn set_enabled(&self, id: &str, shell: bool, on: bool, cx: &mut App) { CardTerminals::set_enabled(self, id, shell, on, cx) }
//!     fn phase(&self, id: &str, cx: &App) -> Phase {
//!         match CardTerminals::phase(self, id, cx) {
//!             None => Phase::Idle,
//!             Some(p) => match p.phase {
//!                 TermPhase::Loading => Phase::Loading,
//!                 TermPhase::Ready => Phase::Ready,
//!                 TermPhase::Error => Phase::Error(p.error),
//!             },
//!         }
//!     }
//!     fn reattach(&self, id: &str, cx: &mut App) { CardTerminals::reattach(self, id, cx) }
//!     fn focus(&self, id: &str, window: &mut Window, cx: &mut App) { CardTerminals::focus(self, id, window, cx) }
//!     fn type_in(&self, id: &str, text: &str, starting: bool, cx: &mut App) { CardTerminals::type_in(self, id, text, starting, cx) }
//!     fn element(&self, id: &str, window: &mut Window, cx: &mut App) -> Option<AnyElement> { CardTerminals::element(self, id, window, cx) }
//!     fn mic(&self, _: &str, _: f32, _: &mut Window, _: &mut App) -> Option<AnyElement> { None }
//! }
//! // In the board's `BoardEnv`, rebuilt each frame (cheap):
//! terminals: Some(Rc::new(
//!     CardTerminals::new(project_id, zoom, cx).with_wheel_gate(Rc::new(|id, ctrl, cx| {
//!         cx.default_global::<Wheel>().0.wheel(Instant::now(), Some(id), ctrl)
//!     })),
//! )),
//! ```
//!
//! The terminal is laid out and fitted at 1× and drawn as a picture scaled by
//! the board's zoom, as 0.9.4's canvas was under the board's CSS transform:
//! the same columns and rows at any zoom, soft text when zoomed in, and the
//! pointer mapped back to the unscaled cells (TERM-083).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, Entity, Global, IntoElement, SharedString, WeakEntity, Window};
use nodus_domain::SessionKind;
use nodus_terminal::Registry;
use nodus_terminal::host::AttachPhase;

use super::host::{HostConfig, TerminalHost, WheelGate};
use super::store::Terminals;
use crate::chat::{ChatView, ChatViews};

/// Says whether a wheel event over a session's card terminal scrolls it;
/// called with the session id and whether Ctrl is held (TERM-104…TERM-106).
pub type CardWheelGate = Rc<dyn Fn(&str, bool, &mut App) -> bool>;

/// What the cards asked for before (or while) their terminals render.
#[derive(Default)]
struct CardHosts {
    hosts: HashMap<String, WeakEntity<TerminalHost>>,
    /// `useAttachedTerminal`'s `enabled`, by session.
    enabled: HashMap<String, bool>,
    shells: HashSet<String>,
    /// A wake's focus that came before the card's terminal was there.
    focus: HashSet<String>,
}

impl Global for CardHosts {}

/// The expanded cards' terminals on one board.
#[derive(Clone)]
pub struct CardTerminals {
    project_id: SharedString,
    zoom: f32,
    registry: Arc<Registry>,
    wheel_gate: Option<CardWheelGate>,
}

impl CardTerminals {
    /// For the cards of `project_id`'s board, drawn at the board's `zoom`.
    pub fn new(project_id: impl Into<SharedString>, zoom: f32, cx: &mut App) -> Self {
        let registry = Terminals::global(cx).read(cx).registry.clone();
        Self {
            project_id: project_id.into(),
            zoom,
            registry,
            wheel_gate: None,
        }
    }

    /// The card decides who a wheel gesture belongs to (its arbiter).
    pub fn with_wheel_gate(mut self, gate: CardWheelGate) -> Self {
        self.wheel_gate = Some(gate);
        self
    }

    fn host(session_id: &str, cx: &App) -> Option<Entity<TerminalHost>> {
        cx.try_global::<CardHosts>()?.hosts.get(session_id)?.upgrade()
    }

    /// `registry.needsOpen` (TERM-026).
    pub fn needs_open(&self, session_id: &str) -> bool {
        self.registry.needs_open(session_id)
    }

    /// Attaches the card's terminal while `enabled`, parks it otherwise
    /// (TERM-092, TERM-099).
    pub fn set_enabled(&self, session_id: &str, shell: bool, enabled: bool, cx: &mut App) {
        let hosts = cx.default_global::<CardHosts>();
        hosts.enabled.insert(session_id.to_owned(), enabled);
        if shell {
            hosts.shells.insert(session_id.to_owned());
        } else {
            hosts.shells.remove(session_id);
        }
        if let Some(host) = Self::host(session_id, cx) {
            host.update(cx, |h, cx| h.set_enabled(enabled, cx));
        }
    }

    /// The card's loading/ready/error phase (TERM-086…TERM-089); `None`
    /// before its terminal has rendered.
    pub fn phase(&self, session_id: &str, cx: &App) -> Option<AttachPhase> {
        Self::host(session_id, cx).map(|h| h.read(cx).phase.clone())
    }

    /// `reattach()`: resumes an exited process (TERM-091, TERM-095).
    pub fn reattach(&self, session_id: &str, cx: &mut App) {
        if let Some(host) = Self::host(session_id, cx) {
            host.update(cx, TerminalHost::reattach);
        }
    }

    /// Focuses the card's terminal, at once or as soon as it's attached
    /// (TERM-095, TERM-100).
    pub fn focus(&self, session_id: &str, window: &mut Window, cx: &mut App) {
        // CHAT-030: a chat's composer takes the keyboard instead.
        if let Some(chat) = ChatViews::get(session_id, cx) {
            let handle = chat.read(cx).composer_handle(cx);
            window.focus(&handle, cx);
            return;
        }
        match Self::host(session_id, cx) {
            Some(host) => host.update(cx, |h, cx| h.request_focus(window, cx)),
            None => {
                cx.default_global::<CardHosts>().focus.insert(session_id.to_owned());
            }
        }
    }

    /// Types `text` in at the prompt (TERM-028, TERM-102).
    pub fn type_in(&self, session_id: &str, text: &str, starting: bool, cx: &mut App) {
        // CHAT-030: into the chat's composer, on its next frame.
        if let Some(chat) = ChatViews::get(session_id, cx) {
            chat.update(cx, |c, cx| c.insert_later(text, cx));
            return;
        }
        self.registry.type_in(session_id, text, starting);
    }

    /// The terminal filling the card's body (its padding is the card's).
    /// It lives while the card renders it every frame; the frame it isn't,
    /// the terminal is parked (TERM-003, TERM-092).
    pub fn element(&self, session_id: &str, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let hosts = cx.default_global::<CardHosts>();
        let enabled = hosts.enabled.get(session_id).copied().unwrap_or(true);
        let kind = if hosts.shells.contains(session_id) {
            SessionKind::Terminal
        } else {
            SessionKind::Claude
        };
        let (id, project) = (session_id.to_owned(), self.project_id.clone());
        let host = window.use_keyed_state(gpui::ElementId::Name(format!("card-terminal-{id}").into()), cx, {
            let id = id.clone();
            move |window, cx| {
                TerminalHost::new(
                    id,
                    project,
                    kind,
                    HostConfig {
                        focus_on_attach: false,
                        card: true,
                    },
                    window,
                    cx,
                )
            }
        });
        let focus = {
            let hosts = cx.default_global::<CardHosts>();
            hosts.hosts.insert(id.clone(), host.downgrade());
            hosts.focus.remove(&id)
        };
        let gate: Option<WheelGate> = self.wheel_gate.clone().map(|gate| {
            let id = id.clone();
            Rc::new(move |ctrl: bool, cx: &mut App| gate(&id, ctrl, cx)) as WheelGate
        });
        let zoom = self.zoom;
        let chat_state = (kind == SessionKind::Claude)
            .then(|| crate::shell::window::main_shell(cx).map(|s| s.read(cx).state.clone()))
            .flatten();
        host.update(cx, |h, cx| {
            h.set_scale(zoom, cx);
            h.set_wheel_gate(gate);
            h.set_enabled(enabled, cx);
            if focus && chat_state.is_none() {
                h.request_focus(window, cx);
            }
        });
        // CHAT-029: a Claude card shows its chat, the terminal docked in it.
        if let Some(state) = chat_state {
            let chat = window.use_keyed_state(gpui::ElementId::Name(format!("card-chat-{id}").into()), cx, {
                let (id, project, host) = (id.clone(), self.project_id.to_string(), host.clone());
                move |window, cx| ChatView::new(state, id, project, host, true, window, cx)
            });
            chat.update(cx, |c, _| c.set_frame(zoom, enabled));
            if focus {
                let handle = chat.read(cx).composer_handle(cx);
                window.focus(&handle, cx);
            }
            return Some(chat.into_any_element());
        }
        Some(host.into_any_element())
    }
}
