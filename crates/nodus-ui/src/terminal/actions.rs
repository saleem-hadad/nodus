//! The store actions the terminal's windows call (`store.ts`): opening and
//! closing the three windows, archiving, removing and stopping cards. The
//! board's own store (spec 09) owns `patchCards` and `removeCard`; until it
//! routes them here, these apply the same change to the loaded board and
//! save it through the core.

use gpui::{App, Entity, Global};
use nodus_domain::{NodePatch, SearchSnippet, SessionCard, SessionStatus};

use super::store;
use crate::state::AppState;

/// `PreviewFind`: the search a preview was opened from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PreviewFind {
    pub query: String,
    /// The best match, whose time picks the turn to scroll to.
    pub snippet: Option<SearchSnippet>,
}

/// `previewFind`, beside `previewSessionId` in the app state.
#[derive(Debug, Clone, Default)]
pub struct PreviewFindState(pub Option<PreviewFind>);

impl Global for PreviewFindState {}

/// `openTerminal` (MODAL-002): the session window, closing the preview.
pub fn open_terminal(state: &Entity<AppState>, session_id: &str, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.terminal_session_id = Some(session_id.to_owned());
        s.preview_session_id = None;
        cx.notify();
    });
}

/// `closeTerminal` (MODAL-006: the process keeps running).
pub fn close_terminal(state: &Entity<AppState>, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.terminal_session_id = None;
        cx.notify();
    });
}

/// `openPreview` (TRANS-002): records the search, closes the session window.
pub fn open_preview(state: &Entity<AppState>, session_id: &str, find: Option<PreviewFind>, cx: &mut App) {
    cx.set_global(PreviewFindState(find));
    state.update(cx, |s, cx| {
        s.preview_session_id = Some(session_id.to_owned());
        s.terminal_session_id = None;
        cx.notify();
    });
}

pub fn close_preview(state: &Entity<AppState>, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.preview_session_id = None;
        cx.notify();
    });
}

/// `openAgents`.
pub fn open_agents(state: &Entity<AppState>, session_id: &str, agent_id: Option<&str>, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.agent_view = Some((session_id.to_owned(), agent_id.map(str::to_owned)));
        cx.notify();
    });
}

pub fn close_agents(state: &Entity<AppState>, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.agent_view = None;
        cx.notify();
    });
}

/// The search the open preview came from.
pub fn preview_find(cx: &App) -> Option<PreviewFind> {
    cx.try_global::<PreviewFindState>().and_then(|f| f.0.clone())
}

/// The card on the loaded board (`useCard`).
pub fn card(state: &AppState, session_id: &str) -> Option<SessionCard> {
    state
        .board
        .as_ref()?
        .cards
        .iter()
        .find(|c| c.node.session_id == session_id)
        .cloned()
}

/// `patchCards`: the change on the loaded board at once, then saved.
pub fn patch_cards(state: &Entity<AppState>, patches: Vec<NodePatch>, cx: &mut App) {
    let project = state.update(cx, |s, cx| {
        let board = s.board.as_mut()?;
        for patch in &patches {
            if let Some(card) = board.cards.iter_mut().find(|c| c.node.session_id == patch.session_id) {
                if let Some(archived) = patch.archived {
                    card.node.archived = archived;
                    if archived {
                        // Archived sessions always read as done, and fold back to a card.
                        card.status = SessionStatus::Done;
                        card.node.expanded = None;
                    }
                }
                if let Some(x) = patch.x {
                    card.node.x = x;
                }
                if let Some(y) = patch.y {
                    card.node.y = y;
                }
            }
        }
        cx.notify();
        Some(board.project.id.clone())
    });
    let Some(project) = project else { return };
    let nodus = state.read(cx).nodus.clone();
    cx.spawn(async move |_| {
        let _ = nodus.board().save_nodes(&project, &patches).await;
    })
    .detach();
}

/// `removeCard`: off the board, its terminal disposed (TERM-027), its node removed.
pub fn remove_card(state: &Entity<AppState>, session_id: &str, cx: &mut App) {
    let project = state.update(cx, |s, cx| {
        let board = s.board.as_mut()?;
        board.cards.retain(|c| c.node.session_id != session_id);
        s.selection.retain(|id| id != session_id);
        cx.notify();
        Some(board.project.id.clone())
    });
    store::dispose(session_id, cx);
    let Some(project) = project else { return };
    let nodus = state.read(cx).nodus.clone();
    let id = session_id.to_owned();
    cx.spawn(async move |_| nodus.board().remove_node(&project, &id).await)
        .detach();
}

/// `stopSessions`: every one stopped at once (SESS-153).
pub fn stop_sessions(state: &Entity<AppState>, ids: Vec<String>, cx: &mut App) {
    let nodus = state.read(cx).nodus.clone();
    cx.spawn(async move |_| nodus.sessions().stop(&ids).await).detach();
}
