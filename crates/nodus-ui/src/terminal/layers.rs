//! The terminal's three windows over the board, kept in step with the app
//! state (`App.tsx` renders `TerminalModal`, `TranscriptPreview` and
//! `AgentViewer` side by side): each is open while its id is set and a board
//! is loaded, and is built again when its session changes (MODAL-001,
//! TRANS-001, AGV-001). The agents viewer sits above the session window,
//! which sits above the preview, as ⌘W closes them (MODAL-005).

use gpui::{AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Subscription, Window, div};

use super::agents::AgentViewer;
use super::preview::TranscriptPreview;
use super::session_window::SessionWindow;
use crate::state::AppState;

/// The layers the shell draws above the main area.
pub struct TerminalLayers {
    state: Entity<AppState>,
    session: Option<Entity<SessionWindow>>,
    preview: Option<Entity<TranscriptPreview>>,
    agents: Option<Entity<AgentViewer>>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalLayers {
    pub fn new(state: Entity<AppState>, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self {
            state,
            session: None,
            preview: None,
            agents: None,
            _subscriptions: subscriptions,
        }
    }

    /// The open session window, if any.
    pub fn session_window(&self) -> Option<&Entity<SessionWindow>> {
        self.session.as_ref()
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (terminal, preview, agents, project) = {
            let s = self.state.read(cx);
            (
                s.terminal_session_id.clone(),
                s.preview_session_id.clone(),
                s.agent_view.as_ref().map(|(id, _)| id.clone()),
                s.board.as_ref().map(|b| b.project.id.clone()),
            )
        };
        let Some(project) = project else {
            self.session = None;
            self.preview = None;
            self.agents = None;
            return;
        };
        let state = self.state.clone();
        match terminal {
            Some(id) if self.session.as_ref().is_none_or(|w| w.read(cx).session_id != id) => {
                let (state, project) = (state.clone(), project.clone());
                self.session = Some(cx.new(|cx| SessionWindow::new(state, id, project, window, cx)));
            }
            Some(_) => {}
            None => self.session = None,
        }
        match preview {
            Some(id) if self.preview.as_ref().is_none_or(|w| w.read(cx).session_id != id) => {
                let (state, project) = (state.clone(), project.clone());
                self.preview = Some(cx.new(|cx| TranscriptPreview::new(state, id, project, window, cx)));
            }
            Some(_) => {}
            None => self.preview = None,
        }
        match agents {
            Some(id) if self.agents.as_ref().is_none_or(|w| w.read(cx).session_id != id) => {
                self.agents = Some(cx.new(|cx| AgentViewer::new(state, id, project, window, cx)));
            }
            Some(_) => {}
            None => self.agents = None,
        }
    }
}

impl Render for TerminalLayers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync(window, cx);
        div()
            .children(self.preview.clone())
            .children(self.session.clone())
            .children(self.agents.clone())
    }
}
