//! Starting a session from the board or floating capture, and sending one a
//! prompt (ipc.ts's `createSession` and `sendToSession`, SESS-146). The
//! process itself is started by a [`Launcher`] (nodus-core, with
//! `nodus_sessions::launch` and the session host).

use std::path::PathBuf;

use nodus_domain::js;
use nodus_domain::{
    BoardNode, CreateSessionOptions, CreatedBy, ForkFrom, Project, ProjectAction, SessionCard, SessionEffort,
    SessionKind, SessionModel, SessionStatus,
};
use nodus_store::BoardState;

use crate::archive::bare_node;
use crate::board::Board;
use crate::geometry::NEW_CARD_H;
use crate::runs::find_action;
use crate::{BoardError, BoxFuture};

/// A new card, and the action it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession {
    pub node: BoardNode,
    pub action: Option<ProjectAction>,
}

/// A new card's process to start (SESS-119).
#[derive(Debug, Clone, PartialEq)]
pub struct StartNew {
    pub project: Project,
    pub node: BoardNode,
    /// The action it runs: its prompt is the first prompt, and it runs in
    /// auto mode with `runArgs(action)` (05).
    pub action: Option<ProjectAction>,
    /// `opts.initialPrompt` as the window sent it.
    pub initial_prompt: Option<String>,
    /// A fork, whose conversation was written for it: it resumes that
    /// rather than starting afresh, in the folder it was copied in (SESS-222).
    pub resume: bool,
}

/// A stopped session to resume with a prompt (SESS-145).
#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    pub project: Project,
    pub node: BoardNode,
    /// A card never used has no transcript to resume.
    pub has_transcript: bool,
    /// The prompt, trimmed.
    pub prompt: String,
}

/// How the board starts and types into session processes. nodus-core
/// implements it with `nodus_sessions::launch` and the session host, after
/// the login-shell environment is ready.
pub trait Launcher: Send + Sync {
    /// Starts a new card's process: `claude`, or a shell for a terminal. The
    /// error is the host's, shown as it is.
    fn start_new(&self, start: StartNew) -> BoxFuture<'_, Result<(), String>>;
    /// Resumes a stopped Claude session with a prompt.
    fn resume_with_prompt(&self, resume: Resume) -> BoxFuture<'_, Result<(), String>>;
    /// Types a prompt into a live session as a bracketed paste, then Return
    /// 200 ms later (SESS-144).
    fn type_prompt(&self, session_id: &str, text: &str);
}

/// The node `session:create` adds, and the action it runs (SESS-115…117).
pub fn new_session(
    board: &BoardState,
    project: &Project,
    global_actions: Option<&[ProjectAction]>,
    opts: &CreateSessionOptions,
    session_id: &str,
    now: f64,
) -> Result<NewSession, BoardError> {
    let terminal = opts.kind == Some(SessionKind::Terminal);
    // SESS-115
    let action_id = opts.action_id.as_deref().filter(|a| !a.is_empty());
    let action = match action_id {
        Some(id) if !terminal => find_action(project, global_actions, id).cloned(),
        _ => None,
    };
    if action_id.is_some() && action.is_none() {
        return Err(BoardError::NoAction);
    }
    // SESS-116
    let mut node = bare_node(session_id, opts.x, opts.y, false, now);
    node.kind = terminal.then_some(SessionKind::Terminal);
    node.title = opts
        .name
        .as_deref()
        .map(js::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    node.parents = opts.parents.clone().filter(|p| !terminal && !p.is_empty());
    node.created_by_app = Some(true);
    node.run_of = action.as_ref().map(|a| a.id.clone());
    node.expanded = (opts.expanded == Some(true)).then_some(true);
    node.h = Some(NEW_CARD_H);
    node.auto_mode = (!terminal && opts.auto_mode == Some(true)).then_some(true);
    node.from_view = opts
        .from_view
        .clone()
        .filter(|v| !terminal && board.views.contains_key(v));
    node.set_model(
        opts.model
            .as_deref()
            .and_then(SessionModel::parse)
            .filter(|_| !terminal),
    );
    node.set_effort(
        opts.effort
            .as_deref()
            .and_then(SessionEffort::parse)
            .filter(|_| !terminal),
    );
    // SESS-117
    if let Some(action) = &action {
        node.title = Some(action.name.clone());
    }
    Ok(NewSession { node, action })
}

/// SESS-222: a fork hangs off its source, and runs as it does: on its model
/// and effort unless asked otherwise, in its mode, and in its worktree, where
/// its conversation was copied.
fn forked_node(node: &mut BoardNode, source: &BoardNode, opts: &CreateSessionOptions) {
    node.parents = Some(vec![source.session_id.clone()]);
    node.set_forked_from(Some(&source.session_id));
    if opts.model.is_none() {
        node.set_model(source.model().and_then(SessionModel::parse));
    }
    if opts.effort.is_none() {
        node.set_effort(source.effort().and_then(SessionEffort::parse));
    }
    if opts.auto_mode.is_none() {
        node.auto_mode = source.auto_mode;
    }
    node.set_worktree(source.worktree().as_ref());
}

impl Board {
    /// `session:create`, once the login-shell environment is ready: a new card
    /// on the board, its process started. The card is on the board before the
    /// process starts, and taken off again if it can't (SESS-114…120). On a
    /// server, `created_by` is the member who started it (SRV-183).
    pub async fn create_session(
        &self,
        project_id: &str,
        opts: &CreateSessionOptions,
        created_by: Option<&str>,
        launcher: &dyn Launcher,
    ) -> Result<SessionCard, BoardError> {
        let project = self.project(project_id)?;
        // SESS-114: a lowercase v4 UUID.
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = self.now();
        // SESS-222: its conversation first, so there's nothing to take back
        // when it can't be copied.
        let fork = match opts.fork.as_ref().filter(|_| opts.kind != Some(SessionKind::Terminal)) {
            Some(from) => Some(self.fork_conversation(&project, from, &session_id)?),
            None => None,
        };
        let created = self.store.update(|state| {
            let globals = state.actions.clone();
            let board = state.board_mut(project_id);
            let mut new = new_session(board, &project, globals.as_deref(), opts, &session_id, now)?;
            new.node.set_created_by(created_by);
            if let Some((source, _)) = &fork {
                forked_node(&mut new.node, source, opts);
            }
            // SCHED-026: a run of a scheduled action comes out beside its tile.
            if let Some(tile) = new.action.as_ref().and_then(|a| board.tiles.get(&a.id)) {
                let at = crate::tiles::run_spot(board, tile, &new.node);
                (new.node.x, new.node.y) = (at.x, at.y);
            }
            // SESS-118
            board.nodes.insert(session_id.clone(), new.node.clone());
            Ok::<_, BoardError>(new)
        });
        let forget_fork = || {
            if let Some((_, file)) = &fork {
                let _ = std::fs::remove_file(file);
            }
        };
        let NewSession { node, action } = created.inspect_err(|_| forget_fork())?;
        let start = StartNew {
            project,
            node: node.clone(),
            action,
            initial_prompt: opts.initial_prompt.clone(),
            resume: fork.is_some(),
        };
        if let Err(err) = launcher.start_new(start).await {
            self.store
                .update(|state| state.board_mut(project_id).nodes.shift_remove(&session_id));
            forget_fork();
            return Err(BoardError::Start(err));
        }
        // SESS-120
        self.store.save();
        // As it is now: on a server, starting it gave it a worktree (SRV-122).
        let node = self
            .read_board(project_id, |_, board| board.nodes.get(&session_id).cloned())
            .unwrap_or(node);
        Ok(self.card(&node))
    }

    /// SESS-222: `from`'s conversation up to its message, copied as
    /// `session_id`'s in the folder `from` runs in. The source card, and the
    /// new transcript.
    fn fork_conversation(
        &self,
        project: &Project,
        from: &ForkFrom,
        session_id: &str,
    ) -> Result<(BoardNode, PathBuf), BoardError> {
        let source = self
            .read_board(&project.id, |_, board| board.nodes.get(&from.session_id).cloned())
            .filter(|n| n.kind != Some(SessionKind::Terminal))
            .ok_or(BoardError::NotOnBoard)?;
        let folder = self.session_folder(project, &from.session_id);
        let file = self.transcripts.transcript_path(&folder, session_id);
        nodus_transcripts::fork_transcript(
            &self.transcripts.transcript_path(&folder, &from.session_id),
            &from.session_id,
            &from.message_id,
            &file,
            session_id,
        )?;
        Ok((source, file))
    }

    /// `session:send`: a prompt sent into a session as if typed at its prompt,
    /// resuming it first if it isn't running (SESS-141…145).
    pub async fn send_to_session(
        &self,
        project_id: &str,
        session_id: &str,
        text: &str,
        launcher: &dyn Launcher,
    ) -> Result<(), BoardError> {
        // SESS-141
        let text = js::trim(text);
        if text.is_empty() {
            return Ok(());
        }
        // SESS-142
        let node = self.read_board(project_id, |_, board| board.nodes.get(session_id).cloned());
        let node = match node {
            Some(node) if !node.archived && node.kind != Some(SessionKind::Terminal) => node,
            _ => return Err(BoardError::NotOnBoard),
        };
        if self.sessions.is_live(session_id) {
            // SESS-143: Return would answer its question instead.
            if self.sessions.status(session_id) == SessionStatus::Waiting {
                return Err(BoardError::Waiting);
            }
            // SESS-144
            self.claim_run(session_id);
            launcher.type_prompt(session_id, text);
            return Ok(());
        }
        // SESS-145: not running, so it resumes with the prompt, the way it does when opened.
        let project = self.project(project_id)?;
        let resume = Resume {
            has_transcript: self.has_transcript(&project, session_id),
            project,
            node,
            prompt: text.to_owned(),
        };
        launcher.resume_with_prompt(resume).await.map_err(BoardError::Start)?;
        self.claim_run(session_id);
        Ok(())
    }
}
