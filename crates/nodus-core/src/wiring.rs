//! The traits domains declare for what they need from each other, as the
//! core implements them (docs/specs/README.md, Layers). Each holds the core weakly, so the
//! domains it's handed to never keep it alive, and does nothing once it's gone.
//!
//! The board and the views call [`CoreSessions`]' queries while they hold the
//! state, so none of them touches the `Store` (integration debt).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use nodus_board::{BoardSessions, BoxFuture, Launcher, Resume, StartNew};
use nodus_domain::{Event, EventSink, SessionCard, SessionMeta, SessionStatus, SessionUsage};
use nodus_platform::process::SpawnError;
use nodus_platform::runner::{Program, RunOutput, RunRequest, Runner};
use nodus_platform::{ClaudeNotFound, Env};
use nodus_sessions::launch::{self, ActionRun, SpawnOptions};
use nodus_sessions::notify::SessionMetas;
use nodus_tools::{ProjectAdder, ViewTools};
use nodus_transcripts::Transcripts;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::Nodus;
use crate::nodus::Inner;

/// Where every event goes: each subscriber (`window.api.on.*`). Dropped when
/// nobody listens, as 0.9.4 drops events while there's no window (PLAT-123,
/// SESS-203).
#[derive(Clone)]
pub(crate) struct Broadcast(pub(crate) broadcast::Sender<Event>);

impl EventSink for Broadcast {
    fn emit(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

/// A status change, for the side effects that follow it (HOST-220).
pub(crate) struct StatusChange {
    pub(crate) session_id: String,
    pub(crate) status: SessionStatus,
    pub(crate) live: bool,
}

/// The status machine's events (`configureStatus`'s `onChange`,
/// `onAgentChange`, `onUsage`). Each goes to the window at once; a status
/// change is then queued for the notifier, the board and the views, which
/// take it in order on a task of their own (HOST-220). The machine may report
/// while the board or the session host client holds its state, so nothing
/// here waits on either.
pub(crate) struct StatusFanout {
    pub(crate) events: Broadcast,
    pub(crate) changes: mpsc::UnboundedSender<StatusChange>,
}

impl EventSink for StatusFanout {
    fn emit(&self, event: Event) {
        let change = match &event {
            Event::Status {
                session_id,
                status,
                live,
            } => Some(StatusChange {
                session_id: session_id.clone(),
                status: *status,
                live: *live,
            }),
            _ => None,
        };
        // HOST-220: `ev:status` to the window first.
        self.events.emit(event);
        if let Some(change) = change {
            let _ = self.changes.send(change);
        }
    }
}

/// The session processes and their status, for the board and the views
/// (`pty.ts` and `hooks.ts` as board.ts and views.ts call them).
pub(crate) struct CoreSessions(pub(crate) Weak<Inner>);

impl BoardSessions for CoreSessions {
    fn is_live(&self, session_id: &str) -> bool {
        self.0.upgrade().is_some_and(|core| core.host.is_live(session_id))
    }

    fn status(&self, session_id: &str) -> SessionStatus {
        self.0
            .upgrade()
            .map_or(SessionStatus::Done, |core| core.status.status(session_id))
    }

    fn usage(&self, session_id: &str) -> Option<SessionUsage> {
        self.0.upgrade()?.status.usage(session_id)
    }

    fn attach_agents(&self, card: &mut SessionCard, repo_path: &str, last_prompt_at: Option<f64>) {
        let Some(core) = self.0.upgrade() else { return };
        let files = core
            .board
            .transcripts()
            .list_agent_files(repo_path, &card.node.session_id);
        core.agents.attach_agents(&core.status, card, &files, last_prompt_at);
    }

    fn stop(&self, session_id: &str) {
        if let Some(core) = self.0.upgrade() {
            drop(core.host.kill(session_id));
        }
    }

    fn stop_and_forget(&self, session_id: &str) {
        let Some(core) = self.0.upgrade() else { return };
        // `void ptys.kill(id).then(() => forgetStatus(id))`: asked now, forgotten once gone.
        let killed = core.host.kill(session_id);
        let (status, session_id) = (core.status.clone(), session_id.to_owned());
        core.runtime.spawn(async move {
            killed.await;
            status.forget(&session_id);
        });
    }

    fn kill(&self, session_id: &str) -> BoxFuture<'_, ()> {
        let killed = self.0.upgrade().map(|core| core.host.kill(session_id));
        Box::pin(async move {
            if let Some(killed) = killed {
                killed.await;
            }
        })
    }

    fn forget_status(&self, session_id: &str) {
        if let Some(core) = self.0.upgrade() {
            core.status.forget(session_id);
        }
    }
}

/// How the board starts and types into sessions (`createSession`,
/// `sendToSession`, SESS-119, SESS-144, SESS-145).
pub(crate) struct CoreLauncher(pub(crate) Weak<Inner>);

impl Launcher for CoreLauncher {
    fn start_new(&self, start: StartNew) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let core = self.0.upgrade().ok_or_else(gone)?;
            let args = start.action.as_ref().map(nodus_actions::run_args);
            let action = start.action.as_ref().zip(args).map(|(action, args)| ActionRun {
                prompt: &action.prompt,
                args,
            });
            let opts = launch::for_create(
                &start.project.id,
                &start.project.repo_path,
                &start.node,
                action,
                start.initial_prompt.as_deref(),
            );
            core.spawn_session(opts).await
        })
    }

    fn resume_with_prompt(&self, resume: Resume) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let core = self.0.upgrade().ok_or_else(gone)?;
            // A view's brain is told about its view every time it starts (SESS-133).
            let brain_args = core.brain_args(&resume.project.id, &resume.node);
            let opts = launch::for_send(
                &resume.project.id,
                &resume.project.repo_path,
                &resume.node,
                resume.has_transcript,
                &resume.prompt,
                brain_args,
            );
            core.spawn_session(opts).await
        })
    }

    fn type_prompt(&self, session_id: &str, text: &str) {
        if let Some(core) = self.0.upgrade() {
            core.host.send_prompt(session_id, text);
        }
    }
}

/// How views start and type into their brains (SESS-134, VIEW-088…095).
pub(crate) struct CoreBrains(pub(crate) Weak<Inner>);

impl nodus_views::BrainLauncher for CoreBrains {
    fn spawn(&self, opts: SpawnOptions) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move { self.0.upgrade().ok_or_else(gone)?.spawn_session(opts).await })
    }

    fn write(&self, session_id: &str, data: &str) {
        if let Some(core) = self.0.upgrade() {
            core.host.write(session_id, data);
        }
    }
}

/// What a view's page may ask of other domains: the repository's git status,
/// and the browser.
pub(crate) struct CoreViewHost(pub(crate) Weak<Inner>);

impl nodus_views::ViewHost for CoreViewHost {
    /// `nodus.git()` (VIEW-066): `gitStatus(repoPath)` as the window gets it,
    /// `null` outside a repository or when git can't run.
    fn git_status(&self, repo_path: &str) -> BoxFuture<'_, Value> {
        let repo = PathBuf::from(repo_path);
        Box::pin(async move {
            let Some(core) = self.0.upgrade() else {
                return Value::Null;
            };
            let env = core.env().await;
            match core.git(&env).status(&repo).await {
                Ok(Some(status)) => serde_json::to_value(status).unwrap_or(Value::Null),
                _ => Value::Null,
            }
        })
    }
}

/// The core's runner, for the domains made before it (views, capture): it
/// can be the session host's, and the host connection comes after them.
pub(crate) struct CoreRunner(pub(crate) Weak<Inner>);

impl Runner for CoreRunner {
    fn run(&self, request: RunRequest) -> nodus_platform::runner::BoxFuture<Result<RunOutput, SpawnError>> {
        match self.0.upgrade() {
            Some(core) => core.runner.run(request),
            None => {
                let program = request.program.to_string();
                Box::pin(async move { Err(SpawnError::reported(program, "Nodus has stopped", None)) })
            }
        }
    }

    fn claude(&self, env: &Env) -> Result<Program, ClaudeNotFound> {
        match self.0.upgrade() {
            Some(core) => core.runner.claude(env),
            None => Err(ClaudeNotFound),
        }
    }
}

/// `addProject`, for the board tools' `create_project` (TOOL-034).
pub(crate) struct CoreProjects(pub(crate) Weak<Inner>);

impl ProjectAdder for CoreProjects {
    fn add_project(&self, folder: &Path, name: Option<&str>) -> Result<(nodus_domain::Project, bool), String> {
        let inner = self.0.upgrade().ok_or_else(gone)?;
        Nodus { inner }
            .projects()
            .add_project(&folder.to_string_lossy(), name)
            .map_err(|err| err.message)
    }
}

/// The view tools, passed on to the views (VIEW-102…114).
pub(crate) struct CoreViewTools(pub(crate) nodus_views::Views);

impl ViewTools for CoreViewTools {
    fn list_views(&self, project_id: &str, self_id: &str) -> Result<Value, String> {
        Ok(self.0.list_views_for_tool(project_id, self_id))
    }

    fn get_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<Value, String> {
        self.0
            .get_view_for_tool(project_id, self_id, view_id)
            .map_err(|err| err.to_string())
    }

    fn create_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> nodus_tools::BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.0
                .create_view_from_tool(project_id, self_id, args)
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn update_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> nodus_tools::BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.0
                .update_view_from_tool(project_id, self_id, args)
                .await
                .map_err(|err| err.to_string())
        })
    }
}

/// What notifications say about a session, from its transcript (`readSession`).
pub(crate) struct CoreMetas(pub(crate) Arc<Transcripts>);

impl SessionMetas for CoreMetas {
    fn session_meta(&self, repo_path: &str, session_id: &str) -> Option<SessionMeta> {
        self.0.session(repo_path, session_id).map(|found| found.meta)
    }
}

/// The app as the updater needs it (06).
pub(crate) struct CoreUpdaterApp(pub(crate) Weak<Inner>);

impl nodus_updater::App for CoreUpdaterApp {
    fn version(&self) -> String {
        crate::VERSION.into()
    }

    fn user_data(&self) -> PathBuf {
        self.0.upgrade().map(|core| core.user_data.clone()).unwrap_or_default()
    }

    fn auto_update(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|core| core.store.read(|state| state.settings.auto_update))
    }

    fn live_session_count(&self) -> usize {
        self.0.upgrade().map_or(0, |core| core.host.live_session_ids().len())
    }

    fn stop_all(&self) -> nodus_updater::BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if let Some(core) = self.0.upgrade() {
                core.host.stop_all().await;
            }
            Ok(())
        })
    }

    fn quit(&self) {
        if let Some(core) = self.0.upgrade() {
            core.shell.quit();
        }
    }
}

fn gone() -> String {
    "Nodus is quitting".into()
}
