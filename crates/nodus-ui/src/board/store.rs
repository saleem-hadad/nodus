//! The store's board actions (`store.ts`: `loadBoard`, `patchCards`,
//! `upsertCard`, `removeCard`, `setArchive`, `saveSection`,
//! `removeSection`, `saveNote`, `removeNotes`, `upsertView`, `patchView`,
//! `removeView`, `setStatus`, `setAgentStatus`, `setUsage`, and the modal
//! openers): each changes the window's copy of the board at once, then
//! saves through the core.

use gpui::{Context, Task};
use nodus_domain::{BoardNote, BoardSection, NodePatch, Rect, SessionCard, SessionStatus, SessionUsage, ViewPatch};

use crate::state::AppState;
use crate::terminal::actions::{PreviewFind, PreviewFindState};

/// `unstickNotes`: stickers on cards leaving the board fall off, onto the
/// board where an active card was, or back where they were from an archived one.
fn unstick_notes(notes: &mut [BoardNote], leaving: &[SessionCard]) {
    for note in notes {
        let Some(card) = note
            .session_id
            .as_deref()
            .and_then(|sid| leaving.iter().find(|c| c.node.session_id == sid))
        else {
            continue;
        };
        note.session_id = None;
        if !card.node.archived {
            note.x = card.node.x;
            note.y = card.node.y;
        }
    }
}

/// MODAL-028: the session window's card came back archived in a reload
/// (archived from outside the window, e.g. `archive_self`).
pub(crate) fn came_back_archived(open: Option<&str>, cards: &[SessionCard]) -> bool {
    open.is_some_and(|open| cards.iter().any(|c| c.node.session_id == open && c.node.archived))
}

impl AppState {
    /// `loadBoard` (UI-050, UI-051, BOARD-006): shows the spinner only when
    /// another project's board is up, ignores a stale answer, and closes the
    /// terminal modal when its session came back archived (MODAL-028).
    pub fn fetch_board(&mut self, project_id: &str, cx: &mut Context<'_, Self>) {
        self.board_loading = self.board.as_ref().is_none_or(|b| b.project.id != project_id);
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        let project_id = project_id.to_owned();
        cx.spawn(async move |this, cx| {
            let loaded = crate::board::on_core(&rt, {
                let (nodus, project_id) = (nodus.clone(), project_id.clone());
                async move { nodus.board().load(&project_id).await }
            })
            .await;
            let _ = this.update(cx, |state, cx| {
                if let Ok(board) = loaded
                    && state.route.project_id() == Some(project_id.as_str())
                {
                    if came_back_archived(state.terminal_session_id.as_deref(), &board.cards) {
                        state.terminal_session_id = None;
                    }
                    // SRV-028: a board read-only while its server is away
                    // has no session window, preview or agents open over it.
                    if board.offline {
                        state.terminal_session_id = None;
                        state.preview_session_id = None;
                        state.agent_view = None;
                        state.selection.clear();
                    }
                    state.board = Some(board);
                }
                state.board_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// `patchCards`: a session nothing was said in leaves the board rather
    /// than being archived (BOARD-111); archived cards read as done, are
    /// stamped, leave their section and fold back.
    pub fn patch_cards(&mut self, patches: Vec<NodePatch>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if patches.is_empty() {
            return;
        }
        let patch_of = |id: &str| patches.iter().rev().find(|p| p.session_id == id);
        let dropped: Vec<SessionCard> = board
            .cards
            .iter()
            .filter(|c| {
                patch_of(&c.node.session_id).is_some_and(|p| p.archived == Some(true))
                    && !c.node.archived
                    && !crate::board::layout::is_terminal(c)
                    && nodus_board::is_empty_session(c.meta.as_ref())
            })
            .cloned()
            .collect();
        let gone: Vec<&str> = dropped.iter().map(|c| c.node.session_id.as_str()).collect();
        unstick_notes(&mut board.notes, &dropped);
        let now = crate::parity::now_ms(cx);
        board.cards.retain(|c| !gone.contains(&c.node.session_id.as_str()));
        for card in &mut board.cards {
            let Some(p) = patch_of(&card.node.session_id) else {
                continue;
            };
            let was_archived = card.node.archived;
            if let Some(x) = p.x {
                card.node.x = x;
            }
            if let Some(y) = p.y {
                card.node.y = y;
            }
            if let Some(archived) = p.archived {
                card.node.archived = archived;
            }
            if let Some(title) = &p.title {
                card.node.title = Some(title.clone());
            }
            if let Some(section) = &p.section_id {
                card.node.section_id.clone_from(section);
            }
            if let Some(expanded) = p.expanded {
                card.node.expanded = Some(expanded);
            }
            if let Some(w) = p.w {
                card.node.w = Some(w);
            }
            if let Some(h) = p.h {
                card.node.h = Some(h);
            }
            if p.archived == Some(true) {
                card.status = SessionStatus::Done;
                if !was_archived {
                    card.node.archived_at = Some(now);
                }
            }
            if card.node.archived {
                card.node.section_id = None;
                card.node.expanded = None;
            }
        }
        let gone: Vec<String> = gone.into_iter().map(str::to_owned).collect();
        self.selection.retain(|id| !gone.contains(id));
        let project_id = board.project.id.clone();
        cx.notify();
        // UI-054: the sessions that left take their terminals with them.
        for id in &gone {
            crate::terminal::store::dispose(id, cx);
        }
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            let _ = crate::board::on_core(&rt, {
                let (nodus, project_id, patches) = (nodus.clone(), project_id.clone(), patches.clone());
                async move { nodus.board().save_nodes(&project_id, &patches).await }
            })
            .await;
        })
        .detach();
    }

    /// `upsertCard`.
    pub fn upsert_card(&mut self, card: SessionCard, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        match board
            .cards
            .iter_mut()
            .find(|c| c.node.session_id == card.node.session_id)
        {
            Some(existing) => *existing = card,
            None => board.cards.push(card),
        }
        cx.notify();
    }

    /// `removeCard`: off the board at once, then removed in the core (which
    /// waits for its process to stop).
    pub fn remove_card(&mut self, session_id: &str, cx: &mut Context<'_, Self>) -> Task<()> {
        let Some(board) = self.board.as_mut() else {
            return Task::ready(());
        };
        let leaving: Vec<SessionCard> = board
            .cards
            .iter()
            .filter(|c| c.node.session_id == session_id)
            .cloned()
            .collect();
        board.cards.retain(|c| c.node.session_id != session_id);
        unstick_notes(&mut board.notes, &leaving);
        self.selection.retain(|id| id != session_id);
        let project_id = board.project.id.clone();
        cx.notify();
        // UI-059: its terminal goes before the core removes it.
        crate::terminal::store::dispose(session_id, cx);
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        let session_id = session_id.to_owned();
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, {
                let (nodus, project_id, session_id) = (nodus.clone(), project_id.clone(), session_id.clone());
                async move { nodus.board().remove_node(&project_id, &session_id).await }
            })
            .await;
        })
    }

    /// `setArchive`.
    pub fn set_archive(&mut self, rect: Rect, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        board.archive = rect;
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, {
                let (nodus, project_id, rect) = (nodus.clone(), project_id.clone(), rect);
                async move { nodus.board().save_archive(&project_id, rect).await }
            })
            .await;
        })
        .detach();
    }

    /// `saveSection`.
    pub fn save_section(&mut self, section: BoardSection, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        match board.sections.iter_mut().find(|s| s.id == section.id) {
            Some(existing) => *existing = section.clone(),
            None => board.sections.push(section.clone()),
        }
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, {
                let (nodus, project_id, section) = (nodus.clone(), project_id.clone(), section.clone());
                async move { nodus.board().save_section(&project_id, &section).await }
            })
            .await;
        })
        .detach();
    }

    /// `removeSection`: its cards stay where they are, ungrouped.
    pub fn remove_section(&mut self, section_id: &str, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        board.sections.retain(|s| s.id != section_id);
        for card in &mut board.cards {
            if card.node.section_id.as_deref() == Some(section_id) {
                card.node.section_id = None;
            }
        }
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        let section_id = section_id.to_owned();
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, {
                let (nodus, project_id, section_id) = (nodus.clone(), project_id.clone(), section_id.clone());
                async move { nodus.board().remove_section(&project_id, &section_id).await }
            })
            .await;
        })
        .detach();
    }

    /// `saveNote`.
    pub fn save_note(&mut self, note: BoardNote, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        match board.notes.iter_mut().find(|n| n.id == note.id) {
            Some(existing) => *existing = note.clone(),
            None => board.notes.push(note.clone()),
        }
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, {
                let (nodus, project_id, note) = (nodus.clone(), project_id.clone(), note.clone());
                async move { nodus.board().save_note(&project_id, &note).await }
            })
            .await;
        })
        .detach();
    }

    /// `removeNotes`.
    pub fn remove_notes(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if ids.is_empty() {
            return;
        }
        board.notes.retain(|n| !ids.contains(&n.id));
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, {
                let (nodus, project_id, ids) = (nodus.clone(), project_id.clone(), ids.clone());
                async move { nodus.board().remove_notes(&project_id, &ids).await }
            })
            .await;
        })
        .detach();
    }

    /// `upsertView`.
    pub fn upsert_view(&mut self, view: nodus_domain::BoardView, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        match board.views.iter_mut().find(|v| v.id == view.id) {
            Some(existing) => *existing = view,
            None => board.views.push(view),
        }
        cx.notify();
    }

    /// `patchView`.
    pub fn patch_view(&mut self, patch: ViewPatch, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if let Some(view) = board.views.iter_mut().find(|v| v.id == patch.id) {
            if let Some(x) = patch.x {
                view.x = x;
            }
            if let Some(y) = patch.y {
                view.y = y;
            }
            if let Some(w) = patch.w {
                view.w = Some(w);
            }
            if let Some(h) = patch.h {
                view.h = Some(h);
            }
            if let Some(title) = &patch.title {
                view.title.clone_from(title);
            }
            if let Some(starts) = patch.starts_sessions {
                view.starts_sessions = Some(starts);
            }
        }
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            let _ = crate::board::on_core(&rt, {
                let (nodus, project_id, patch) = (nodus.clone(), project_id.clone(), patch.clone());
                async move { nodus.views().save(&project_id, &patch).await }
            })
            .await;
        })
        .detach();
    }

    /// `removeView`: the view, and its brain's card, go at once; the brain's
    /// terminal is disposed and its window closes if open (MODAL-030).
    pub fn remove_view(&mut self, view_id: &str, cx: &mut Context<'_, Self>) -> Task<()> {
        let Some(board) = self.board.as_mut() else {
            return Task::ready(());
        };
        let brain = board
            .views
            .iter()
            .find(|v| v.id == view_id)
            .and_then(|v| v.session_id.clone());
        board.views.retain(|v| v.id != view_id);
        if let Some(brain) = &brain {
            board.cards.retain(|c| &c.node.session_id != brain);
            if self.terminal_session_id.as_ref() == Some(brain) {
                self.terminal_session_id = None;
            }
        }
        let project_id = board.project.id.clone();
        cx.notify();
        // UI-069: the brain's terminal goes with its card.
        if let Some(brain) = &brain {
            crate::terminal::store::dispose(brain, cx);
        }
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        let view_id = view_id.to_owned();
        cx.spawn(async move |_, _| {
            let _ = crate::board::on_core(&rt, {
                let (nodus, project_id, view_id) = (nodus.clone(), project_id.clone(), view_id.clone());
                async move { nodus.views().remove(&project_id, &view_id).await }
            })
            .await;
        })
    }

    /// `setStatus`: archived cards always read as done; a process that's gone
    /// takes its agents with it.
    pub fn set_status(&mut self, session_id: &str, status: SessionStatus, live: bool, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        let Some(card) = board.cards.iter_mut().find(|c| c.node.session_id == session_id) else {
            return;
        };
        card.status = if card.node.archived {
            SessionStatus::Done
        } else {
            status
        };
        card.live = live;
        if !live && let Some(agents) = card.agents.as_mut() {
            for agent in agents {
                agent.status = SessionStatus::Done;
            }
        }
        cx.notify();
    }

    /// `setAgentStatus`.
    pub fn set_agent_status(
        &mut self,
        session_id: &str,
        agent_id: &str,
        status: SessionStatus,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(board) = self.board.as_mut() else { return };
        let Some(agent) = board
            .cards
            .iter_mut()
            .find(|c| c.node.session_id == session_id)
            .and_then(|c| c.agents.as_mut())
            .and_then(|a| a.iter_mut().find(|a| a.agent_id == agent_id))
        else {
            return;
        };
        if agent.status != status {
            agent.status = status;
            cx.notify();
        }
    }

    /// `setUsage`.
    pub fn set_usage(&mut self, session_id: &str, usage: SessionUsage, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if let Some(card) = board.cards.iter_mut().find(|c| c.node.session_id == session_id) {
            card.usage = Some(usage);
            cx.notify();
        }
    }

    /// `openTerminal`: the terminal modal, over the preview.
    pub fn open_terminal(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        self.terminal_session_id = Some(session_id.to_owned());
        self.preview_session_id = None;
        cx.notify();
    }

    /// `openPreview(sessionId)`: the transcript preview, closing the terminal
    /// modal, opened from no search (TRANS-002).
    pub fn open_preview(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        self.open_preview_at(session_id, None, cx);
    }

    /// `openPreview(sessionId, find)` (TRANS-002, BOARD-037): records the
    /// search it was opened from (none clears the last one), so the preview
    /// opens at the match.
    pub fn open_preview_at(&mut self, session_id: &str, find: Option<PreviewFind>, cx: &mut Context<'_, Self>) {
        cx.set_global(PreviewFindState(find));
        self.preview_session_id = Some(session_id.to_owned());
        self.terminal_session_id = None;
        cx.notify();
    }

    /// `openAgents`.
    pub fn open_agents(&mut self, session_id: &str, agent_id: Option<&str>, cx: &mut Context<'_, Self>) {
        self.agent_view = Some((session_id.to_owned(), agent_id.map(str::to_owned)));
        cx.notify();
    }

    /// `saveInk` (INKUI-024): the board's ink at once, then saved (spec 11).
    pub fn save_ink(&mut self, strokes: Vec<nodus_domain::InkStroke>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        board.ink = strokes.clone();
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, async move { nodus.board().save_ink(&project_id, &strokes).await }).await;
        })
        .detach();
    }

    /// Shapes, new or changed: on the board at once, then saved (SHAPE-030).
    pub fn save_shapes(&mut self, shapes: Vec<nodus_domain::BoardShape>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if shapes.is_empty() {
            return;
        }
        for shape in &shapes {
            match board.shapes.iter_mut().find(|s| s.id == shape.id) {
                Some(existing) => *existing = shape.clone(),
                None => board.shapes.push(shape.clone()),
            }
        }
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(
                &rt,
                async move { nodus.board().save_shapes(&project_id, &shapes).await },
            )
            .await;
        })
        .detach();
    }

    /// Shapes off the board, and the links they held.
    pub fn remove_shapes(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if ids.is_empty() {
            return;
        }
        board.shapes.retain(|s| !ids.contains(&s.id));
        board.links.retain(|l| !ids.iter().any(|id| l.touches(id)));
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, async move { nodus.board().remove_shapes(&project_id, &ids).await }).await;
        })
        .detach();
    }

    /// Links, new or changed.
    pub fn save_links(&mut self, links: Vec<nodus_domain::BoardLink>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if links.is_empty() {
            return;
        }
        for link in &links {
            match board.links.iter_mut().find(|l| l.id == link.id) {
                Some(existing) => *existing = link.clone(),
                None => board.links.push(link.clone()),
            }
        }
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, async move { nodus.board().save_links(&project_id, &links).await }).await;
        })
        .detach();
    }

    pub fn remove_links(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_mut() else { return };
        if ids.is_empty() {
            return;
        }
        board.links.retain(|l| !ids.contains(&l.id));
        let project_id = board.project.id.clone();
        cx.notify();
        let nodus = self.nodus.clone();
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, async move { nodus.board().remove_links(&project_id, &ids).await }).await;
        })
        .detach();
    }

    /// `setSelection`: the selected session cards (BOARD-…, store sync).
    pub fn set_selection(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        if self.selection != ids {
            self.selection = ids;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::BoardNode;
    use serde_json::Map;

    fn card(id: &str, archived: bool) -> SessionCard {
        SessionCard {
            node: BoardNode {
                session_id: id.into(),
                kind: None,
                x: 0.0,
                y: 0.0,
                archived,
                title: None,
                parents: None,
                created_by_app: None,
                run_of: None,
                expanded: None,
                view_of: None,
                auto_mode: None,
                from_view: None,
                created_at: 0.0,
                w: None,
                h: None,
                archived_at: None,
                section_id: None,
                extra: Map::new(),
            },
            meta: None,
            status: SessionStatus::Done,
            live: false,
            agents: None,
            agent_count: None,
            usage: None,
        }
    }

    #[test]
    fn a_reload_closes_the_window_whose_card_came_back_archived() {
        // MODAL-028: archived from outside (archive_self) while its window is open.
        let cards = [card("live", false), card("gone", true)];
        assert!(came_back_archived(Some("gone"), &cards));
        assert!(!came_back_archived(Some("live"), &cards));
        // A card no longer on the board, or no window, leaves things be.
        assert!(!came_back_archived(Some("elsewhere"), &cards));
        assert!(!came_back_archived(None, &cards));
    }
}
