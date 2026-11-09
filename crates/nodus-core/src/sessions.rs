//! `window.api.sessions`: starting, opening, typing into, resizing and
//! stopping sessions, and reading what was said in them (`session:*` in
//! `src/main/ipc.ts`).

use nodus_domain::{
    AgentInfo, CreateSessionOptions, OpenSessionResult, SessionCard, SessionSearchHit, SummaryResult, TranscriptMessage,
};
use nodus_sessions::launch;

use crate::error::OnChannel;
use crate::wiring::CoreLauncher;
use crate::{CommandResult, Nodus};

/// `window.api.sessions`.
pub struct SessionsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl SessionsApi<'_> {
    /// `session:create` (SESS-114…120, SESS-146): a new card on the board and
    /// its process started, once the login shell's environment is in. Fails
    /// with `That action no longer exists`, the `claude` not found message,
    /// or the session host's error; the card is taken off again then.
    pub async fn create(&self, project_id: &str, opts: &CreateSessionOptions) -> CommandResult<SessionCard> {
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "session:create";
                self.nodus.env().await;
                let launcher = CoreLauncher(std::sync::Arc::downgrade(&self.nodus.inner));
                self.nodus
                    .inner
                    .board
                    .create_session(project_id, opts, &launcher)
                    .await
                    .on(CHANNEL)
            })
            .await
    }

    /// `session:open` (HOST-222, SESS-135, SESS-136): a live session is
    /// resized and gives its buffered output to replay; any other is started
    /// (resumed when it has a transcript) at this size, once the login
    /// shell's environment is in, with nothing to replay.
    pub async fn open(
        &self,
        project_id: &str,
        session_id: &str,
        cols: f64,
        rows: f64,
    ) -> CommandResult<OpenSessionResult> {
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "session:open";
                let inner = &self.nodus.inner;
                // SESS-135: no project lookup for a live one.
                if inner.host.is_live(session_id) {
                    inner.host.resize(session_id, cols, rows);
                    return Ok(OpenSessionResult {
                        replay: inner.host.snapshot(session_id),
                        live: true,
                    });
                }
                self.nodus.env().await;
                let project = inner
                    .store
                    .read(|state| state.project(project_id).cloned())
                    .on(CHANNEL)?;
                let node = inner
                    .store
                    .read(|state| state.board(project_id).and_then(|b| b.nodes.get(session_id).cloned()));
                let brain_args = node.as_ref().and_then(|node| inner.brain_args(project_id, node));
                let opts = launch::for_open(
                    project_id,
                    &project.repo_path,
                    session_id,
                    node.as_ref(),
                    inner.board.has_transcript(&project, session_id),
                    brain_args,
                    (cols, rows),
                );
                inner.spawn_session(opts).await.on(CHANNEL)?;
                Ok(OpenSessionResult {
                    replay: String::new(),
                    live: true,
                })
            })
            .await
    }

    /// `session:write` (HOST-223, SESS-138), fire and forget: an action run
    /// the user steps into becomes theirs first (SESS-139), a view's brain
    /// they type into isn't idle (VIEW-098), then the keys go to the process
    /// (nothing when it isn't live).
    pub fn write(&self, session_id: &str, data: &str) {
        let inner = &self.nodus.inner;
        // Before the write: an interrupt would otherwise end the run's turn and remove it.
        if launch::is_takeover(data, inner.status.status(session_id)) {
            inner.board.claim_run(session_id);
        }
        inner.views.brain_input(session_id);
        inner.host.write(session_id, data);
    }

    /// `session:send` (SESS-141…145, SESS-146): a prompt sent as if typed at
    /// the session's prompt, resuming it first when it isn't running.
    /// Rejects `That session is no longer on the board`, and `It’s waiting
    /// for your answer. Open it to reply.` while it waits on a question.
    pub async fn send(&self, project_id: &str, session_id: &str, text: &str) -> CommandResult<()> {
        self.nodus
            .on_core(async {
                let launcher = CoreLauncher(std::sync::Arc::downgrade(&self.nodus.inner));
                self.nodus
                    .inner
                    .board
                    .send_to_session(project_id, session_id, text, &launcher)
                    .await
                    .on("session:send")
            })
            .await
    }

    /// `session:resize` (HOST-224, SESS-147), fire and forget: ignored unless
    /// live and both are finite; floored, at least 20 × 5.
    pub fn resize(&self, session_id: &str, cols: f64, rows: f64) {
        self.nodus.inner.host.resize(session_id, cols, rows);
    }

    /// `session:kill` (HOST-224, SESS-148): asks the host to stop the
    /// process; done when it has exited or after 4 s, at once when it isn't
    /// live. The card stays, and opening it resumes the session.
    pub async fn kill(&self, session_id: &str) {
        self.nodus
            .on_core(async {
                self.nodus.inner.host.kill(session_id).await;
            })
            .await;
    }

    /// Stopping sessions from the board or the terminal modal (SESS-153):
    /// every one killed at once, done once all have exited or timed out. The
    /// window picks them with `nodus_board::stoppable` and asks first with
    /// `nodus_board::stop_confirmation`.
    pub async fn stop(&self, session_ids: &[String]) {
        self.nodus
            .on_core(async {
                let host = &self.nodus.inner.host;
                let kills: Vec<_> = session_ids.iter().map(|id| host.kill(id)).collect();
                for killed in kills {
                    killed.await;
                }
            })
            .await;
    }

    /// `session:transcript` (SESS-162): the conversation, for the preview.
    pub async fn transcript(&self, project_id: &str, session_id: &str) -> CommandResult<Vec<TranscriptMessage>> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .board
                    .transcript(project_id, session_id)
                    .on("session:transcript")
            })
            .await
    }

    /// `session:summarize` (SESS-165…174): handoff summaries, one per id
    /// asked for in that order, once the login shell's environment is in.
    /// Progress arrives as `Event::SummarizeProgress`.
    pub async fn summarize(
        &self,
        project_id: &str,
        session_ids: &[String],
        request_id: &str,
        focus: Option<&str>,
    ) -> CommandResult<Vec<SummaryResult>> {
        self.nodus
            .on_core(async {
                let env = self.nodus.env().await;
                self.nodus
                    .inner
                    .board
                    .summarize(project_id, session_ids, request_id, focus, &env)
                    .await
                    .on("session:summarize")
            })
            .await
    }

    /// `session:search` (SESS-175): the board's sessions whose transcripts
    /// hold every word of the query, best first.
    pub async fn search(&self, project_id: &str, query: &str) -> CommandResult<Vec<SessionSearchHit>> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .board
                    .search(project_id, query)
                    .await
                    .on("session:search")
            })
            .await
    }

    /// `session:agents` (SESS-164, HOST-198): every subagent the session has
    /// started, oldest first.
    pub async fn agents(&self, project_id: &str, session_id: &str) -> CommandResult<Vec<AgentInfo>> {
        self.nodus
            .on_core(async {
                let inner = &self.nodus.inner;
                let project = inner
                    .store
                    .read(|state| state.project(project_id).cloned())
                    .on("session:agents")?;
                let files = inner
                    .board
                    .transcripts()
                    .list_agent_files(&project.repo_path, session_id);
                Ok(inner.agents.read_agents(&inner.status, session_id, &files))
            })
            .await
    }

    /// `session:agent-transcript` (SESS-163, HOST-200): a subagent's steps.
    /// `Not an agent of this session` for ids that aren't safe in a path.
    pub async fn agent_transcript(
        &self,
        project_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> CommandResult<Vec<TranscriptMessage>> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .board
                    .agent_transcript(project_id, session_id, agent_id)
                    .on("session:agent-transcript")
            })
            .await
    }
}
