//! Starting a session from the board or floating capture, and sending one a
//! prompt (ipc.ts's `createSession` and `sendToSession`, SESS-146). The
//! process itself is started by a [`Launcher`] (nodus-core, with
//! `nodus_sessions::launch` and the session host).

use nodus_domain::js;
use nodus_domain::{
    BoardNode, CreateSessionOptions, Project, ProjectAction, SessionCard, SessionKind, SessionModel, SessionStatus,
};
use nodus_store::BoardState;

use crate::archive::bare_node;
use crate::board::Board;
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
    // SESS-117
    if let Some(action) = &action {
        node.title = Some(action.name.clone());
    }
    Ok(NewSession { node, action })
}

impl Board {
    /// `session:create`, once the login-shell environment is ready: a new card
    /// on the board, its process started. The card is on the board before the
    /// process starts, and taken off again if it can't (SESS-114…120).
    pub async fn create_session(
        &self,
        project_id: &str,
        opts: &CreateSessionOptions,
        launcher: &dyn Launcher,
    ) -> Result<SessionCard, BoardError> {
        let project = self.project(project_id)?;
        // SESS-114: a lowercase v4 UUID.
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = self.now();
        let NewSession { node, action } = self.store.update(|state| {
            let globals = state.actions.clone();
            let board = state.board_mut(project_id);
            let new = new_session(board, &project, globals.as_deref(), opts, &session_id, now)?;
            // SESS-118
            board.nodes.insert(session_id.clone(), new.node.clone());
            Ok::<_, BoardError>(new)
        })?;
        let start = StartNew {
            project,
            node: node.clone(),
            action,
            initial_prompt: opts.initial_prompt.clone(),
        };
        if let Err(err) = launcher.start_new(start).await {
            self.store
                .update(|state| state.board_mut(project_id).nodes.shift_remove(&session_id));
            return Err(BoardError::Start(err));
        }
        // SESS-120
        self.store.save();
        Ok(self.card(&node))
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
