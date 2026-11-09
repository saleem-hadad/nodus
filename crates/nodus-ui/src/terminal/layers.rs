//! The terminal's three windows over the board, kept in step with the app
//! state (`App.tsx` renders `TerminalModal`, `TranscriptPreview` and
//! `AgentViewer` side by side): each is open while its id is set and a board
//! is loaded, and is built again when its session changes (MODAL-001,
//! TRANS-001, AGV-001).

use gpui::{AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Subscription, Window, div};

use super::session_window::SessionWindow;
use crate::state::AppState;

/// The layers the shell draws above the main area.
pub struct TerminalLayers {
    state: Entity<AppState>,
    session: Option<Entity<SessionWindow>>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalLayers {
    pub fn new(state: Entity<AppState>, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self {
            state,
            session: None,
            _subscriptions: subscriptions,
        }
    }

    /// The open session window, if any.
    pub fn session_window(&self) -> Option<&Entity<SessionWindow>> {
        self.session.as_ref()
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (terminal, project) = {
            let s = self.state.read(cx);
            (
                s.terminal_session_id.clone(),
                s.board.as_ref().map(|b| b.project.id.clone()),
            )
        };
        match (terminal, project) {
            (Some(id), Some(project)) => {
                if self.session.as_ref().is_none_or(|w| w.read(cx).session_id != id) {
                    let state = self.state.clone();
                    self.session = Some(cx.new(|cx| SessionWindow::new(state, id, project, window, cx)));
                }
            }
            _ => self.session = None,
        }
    }
}

impl Render for TerminalLayers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync(window, cx);
        div().children(self.session.clone())
    }
}
