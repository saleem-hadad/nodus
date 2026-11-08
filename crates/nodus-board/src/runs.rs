//! Action runs and archive_self on the board side (ipc.ts's `finishRun`,
//! `claimRun`, `archiveIfAsked`, `findAction`).

use nodus_domain::{ActionRunResult, Event, Project, ProjectAction};

use crate::archive::archive_node;
use crate::board::Board;
use crate::cards::set;
use crate::{Effects, PoisonFree};

/// One of the project's own actions, or a global one (SESS-115, SESS-155).
pub fn find_action<'a>(
    project: &'a Project,
    global: Option<&'a [ProjectAction]>,
    action_id: &str,
) -> Option<&'a ProjectAction> {
    project
        .actions
        .iter()
        .flatten()
        .chain(global.into_iter().flatten())
        .find(|a| a.id == action_id)
}

impl Board {
    /// The project whose board has a node for the session, if that node is an
    /// action run.
    fn run_project(&self, session_id: &str) -> Option<String> {
        self.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            let node = state.board(project_id)?.nodes.get(session_id)?;
            set(node.run_of.as_deref()).then(|| project_id.to_owned())
        })
    }

    /// An action run whose turn has ended is removed, and its transcript hidden
    /// like a removed card's. The window is told what Claude said last, which
    /// the prompt asks to be a one-line summary. Nothing happens for a session
    /// that isn't a run (SESS-155).
    pub fn finish_run(&self, session_id: &str) {
        let Some(project_id) = self.run_project(session_id) else {
            return;
        };
        let Ok(project) = self.project(&project_id) else {
            return;
        };
        let meta = self.transcripts.session(&project.repo_path, session_id).map(|f| f.meta);
        let mut effects = Effects::default();
        self.store.update(|state| {
            let Some(node) = state.board(&project_id).and_then(|b| b.nodes.get(session_id)) else {
                return;
            };
            let Some(run_of) = node.run_of.clone().filter(|r| !r.is_empty()) else {
                return;
            };
            let action_name = state
                .project(&project_id)
                .ok()
                .and_then(|p| find_action(p, state.actions.as_deref(), &run_of))
                .map(|a| a.name.clone());
            let action_name = action_name
                .or_else(|| node.title.clone())
                .unwrap_or_else(|| "Action".into());
            let board = state.board_mut(&project_id);
            board.nodes.shift_remove(session_id);
            if !board.hidden.iter().any(|h| h == session_id) {
                board.hidden.push(session_id.to_owned());
            }
            effects.stop_and_forget.push(session_id.to_owned());
            effects.events.push(Event::RunFinished(ActionRunResult {
                project_id: project_id.clone(),
                session_id: session_id.to_owned(),
                action_name,
                message: meta.and_then(|m| m.last_assistant_text),
            }));
            effects.events.push(Event::BoardChanged {
                project_id: project_id.clone(),
            });
        });
        self.apply(effects);
    }

    /// A run the user typed into or interrupted is theirs: it joins the board
    /// as an ordinary session. Nothing happens for one that isn't a run
    /// (SESS-140).
    pub fn claim_run(&self, session_id: &str) {
        let Some(project_id) = self.run_project(session_id) else {
            return;
        };
        let claimed = self.store.update(|state| {
            let node = state.board_mut(&project_id).nodes.get_mut(session_id);
            node.and_then(|n| n.run_of.take()).is_some()
        });
        if claimed {
            self.events.emit(Event::BoardChanged { project_id });
        }
    }

    /// Remembers that a session asked to be archived (archive_self, 04), which
    /// happens at its next `done` (SESS-157).
    pub fn archive_when_done(&self, session_id: &str) {
        self.archive_when_done.lock_free().insert(session_id.to_owned());
    }

    /// Archives a session that asked to be, now its turn has ended (SESS-157).
    pub fn archive_if_asked(&self, session_id: &str) {
        if !self.archive_when_done.lock_free().remove(session_id) {
            return;
        }
        let found = self.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            let node = state.board(project_id)?.nodes.get(session_id)?;
            (!node.archived).then(|| project_id.to_owned())
        });
        let Some(project_id) = found else {
            return;
        };
        let now = self.now();
        let mut effects = Effects::default();
        self.store.update(|state| {
            if archive_node(state.board_mut(&project_id), session_id, now, &mut effects) {
                effects.events.push(Event::BoardChanged {
                    project_id: project_id.clone(),
                });
            }
        });
        self.apply(effects);
    }

    /// What the board does when a session's status turns `done`, after the
    /// window and notifications are told and before views' brains are: a run
    /// is finished, then a session that asked to be is archived (SESS-154).
    pub fn on_done(&self, session_id: &str) {
        self.finish_run(session_id);
        self.archive_if_asked(session_id);
    }
}
