//! The server's pool of Claude accounts, as the core uses it (SRV-100…104).
//! The pool itself is the server's ([`crate::Hosting::claude_account`]);
//! here every Claude Code process the core starts on a server takes an
//! account's token, and what the pool needs to move a session is offered.
//!
//! - **Sessions** (SRV-101): each start of a Claude card takes an account
//!   and gets its token as `CLAUDE_CODE_OAUTH_TOKEN`, only through the spawn
//!   sent to the host; the card records the account. With none to take,
//!   the start fails in the pool's words.
//! - **Background `claude -p`** (SRV-101): names, summaries, action drafts
//!   and forks go through [`PoolRunner`], which does the same for every run
//!   of Claude Code without a token of its own.
//! - **Usage and failed turns** (SRV-102, SRV-103): what a session reports
//!   with its own token goes to the pool with the account it runs on
//!   ([`crate::hosting::CoreServerSessions`]).
//! - **Moving** (SRV-104): stopping a session, and resuming it, with
//!   `Continue.` when it was mid-turn; the start takes another account.

use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use nodus_domain::{Event, SessionKind, SessionStatus};
use nodus_platform::process::SpawnError;
use nodus_platform::runner::{BoxFuture, Program, RunOutput, RunRequest, Runner};
use nodus_platform::{ClaudeNotFound, Env};
use nodus_sessions::launch;
use serde_json::Value;

use crate::error::OnChannel;
use crate::nodus::Inner;
use crate::{CommandError, CommandResult, Nodus};

/// The variable Claude Code takes a `claude setup-token` token from.
pub const TOKEN_VAR: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// What a moved session is sent once it's back mid-turn (SRV-104).
pub const CONTINUE: &str = "Continue.";
/// How long checking a new token may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(90);
/// What a check run may print.
const CHECK_OUTPUT: usize = 1024 * 1024;

/// A Claude account from the server's pool, taken for one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolAccount {
    pub id: String,
    pub label: String,
    /// Its `claude setup-token` token: the process's `CLAUDE_CODE_OAUTH_TOKEN`.
    pub token: String,
}

/// A session running on an account, for moving it (SRV-104, SRV-105).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSession {
    pub project_id: String,
    pub session_id: String,
    /// Mid-turn: working or waiting on someone.
    pub busy: bool,
}

/// Whether a run starts Claude Code: [`Program::Claude`], or the path the
/// app's own runner found it at.
fn is_claude(program: &Program) -> bool {
    match program {
        Program::Claude => true,
        Program::Path(path) => path.file_stem().is_some_and(|stem| stem.eq_ignore_ascii_case("claude")),
    }
}

/// The core's runner on a server (SRV-101): each run of Claude Code that
/// has no token of its own takes an account from the pool first. Every
/// other run goes through as it is.
pub(crate) struct PoolRunner {
    pub(crate) inner: Arc<dyn Runner>,
    pub(crate) core: Weak<Inner>,
}

impl Runner for PoolRunner {
    fn run(&self, mut request: RunRequest) -> BoxFuture<Result<RunOutput, SpawnError>> {
        if is_claude(&request.program)
            && let Some(core) = self.core.upgrade()
            && let Some(hosted) = core.hosted()
        {
            let mut env = request.env.take().unwrap_or_else(|| {
                core.env
                    .borrow()
                    .as_ref()
                    .map(|env| env.spawn_env())
                    .unwrap_or_default()
            });
            if !env.get(TOKEN_VAR).is_some_and(|token| !token.is_empty()) {
                match hosted.hosting.claude_account() {
                    Ok(Some(account)) => {
                        env.insert(TOKEN_VAR.into(), account.token);
                    }
                    Ok(None) => {}
                    Err(why) => return Box::pin(async move { Err(SpawnError::reported("claude", why, None)) }),
                }
            }
            request.env = Some(env);
        }
        self.inner.run(request)
    }

    fn claude(&self, env: &Env) -> Result<Program, ClaudeNotFound> {
        self.inner.claude(env)
    }
}

impl Inner {
    /// SRV-101: a Claude card's process on a server takes an account: its
    /// token for the environment, and the card records which. None for a
    /// terminal, in the app, or on a server with no pool. Fails in the
    /// pool's words when no account can take it.
    pub(crate) fn take_account(
        &self,
        project_id: &str,
        session_id: &str,
        kind: Option<SessionKind>,
    ) -> Result<Option<(String, String)>, String> {
        let Some(hosted) = self.hosted() else {
            return Ok(None);
        };
        if kind == Some(SessionKind::Terminal) {
            return Ok(None);
        }
        let Some(account) = hosted.hosting.claude_account()? else {
            return Ok(None);
        };
        let recorded = self.store.update(|state| {
            let node = state.board_mut(project_id).nodes.get_mut(session_id)?;
            let same = node.account().as_deref() == Some(account.id.as_str())
                && node.account_label().as_deref() == Some(account.label.as_str());
            node.set_account(Some((&account.id, &account.label)));
            Some(!same)
        });
        if recorded == Some(true) {
            let _ = self.events.send(Event::BoardChanged {
                project_id: project_id.to_owned(),
            });
        }
        Ok(Some((TOKEN_VAR.to_owned(), account.token)))
    }

    /// The account a session's card records (SRV-101).
    pub(crate) fn session_account(&self, session_id: &str) -> Option<String> {
        self.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            state.board(project_id)?.nodes.get(session_id)?.account()
        })
    }
}

impl Nodus {
    /// SRV-100: whether a token from `claude setup-token` works. One
    /// `claude -p "Reply OK" --output-format json --max-turns 1` with it,
    /// through the runner (in the session host, as `nodus-run`, on a
    /// server), which must exit 0 and say `is_error: false`. An API key in
    /// the environment is left out, so it can't pass in the token's place.
    /// Fails with why, never with the token.
    pub async fn check_claude_token(&self, token: &str) -> Result<(), String> {
        let inner = &self.inner;
        let env = inner.env().await;
        let program = inner.runner.claude(&env).map_err(|err| err.to_string())?;
        let mut vars = env.spawn_env();
        for key in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "NODUS_SESSION",
            "NODUS_TOKEN",
        ] {
            vars.remove(key);
        }
        vars.insert(TOKEN_VAR.into(), token.to_owned());
        let home = vars.get("HOME").cloned().unwrap_or_else(|| "/".to_owned());
        let request = RunRequest::new(program)
            .args(["-p", "Reply OK", "--output-format", "json", "--max-turns", "1"])
            .cwd(Path::new(&home))
            .env(vars)
            .max_buffer(CHECK_OUTPUT)
            .timeout(CHECK_TIMEOUT);
        let out = inner.runner.run(request).await.map_err(|err| err.to_string())?;
        if out.timed_out {
            return Err("Claude didn't answer with that token in time".to_owned());
        }
        let reply: Option<Value> = nodus_domain::json::from_str(out.stdout_text().trim()).ok();
        let is_error = reply.as_ref().and_then(|r| r.get("is_error")).and_then(Value::as_bool);
        if out.succeeded() && is_error == Some(false) {
            return Ok(());
        }
        let said = reply
            .as_ref()
            .and_then(|r| r.get("result"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|said| !said.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                let stderr = out.stderr_text();
                stderr
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_owned)
            });
        let said = said.map(|said| said.replace(token, "…"));
        Err(match said {
            Some(said) => format!("Claude turned that token down: {said}"),
            None => "Claude turned that token down".to_owned(),
        })
    }

    /// The live sessions whose card records `account_id`, and whether each
    /// is mid-turn (SRV-104, SRV-105).
    pub fn sessions_on_account(&self, account_id: &str) -> Vec<AccountSession> {
        let inner = &self.inner;
        let cards: Vec<(String, String)> = inner.store.read(|state| {
            state
                .boards
                .iter()
                .flat_map(|(project_id, board)| {
                    board
                        .nodes
                        .values()
                        .filter(|node| node.account().as_deref() == Some(account_id))
                        .map(|node| (project_id.clone(), node.session_id.clone()))
                })
                .collect()
        });
        cards
            .into_iter()
            .filter(|(_, session_id)| inner.host.is_live(session_id))
            .map(|(project_id, session_id)| {
                let busy = inner.status.status(&session_id) != SessionStatus::Done;
                AccountSession {
                    project_id,
                    session_id,
                    busy,
                }
            })
            .collect()
    }

    /// The account a session's card records (SRV-101).
    pub fn account_of(&self, session_id: &str) -> Option<String> {
        self.inner.session_account(session_id)
    }

    /// The label of the account a session's card records.
    pub fn account_label_of(&self, session_id: &str) -> Option<String> {
        self.inner.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            state.board(project_id)?.nodes.get(session_id)?.account_label()
        })
    }

    /// SRV-104: stops a session's process and starts it again, resumed,
    /// which takes an account afresh: with `Continue.` when it was
    /// mid-turn, else as it was. Fails as the start does.
    pub async fn restart_session(&self, project_id: &str, session_id: &str, busy: bool) -> CommandResult<()> {
        const CHANNEL: &str = "session:restart";
        self.on_core(async {
            let inner = &self.inner;
            inner.host.kill(session_id).await;
            if busy {
                return self.sessions().send(project_id, session_id, CONTINUE).await;
            }
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
                (120.0, 36.0),
            );
            inner
                .spawn_session(opts)
                .await
                .map_err(|err| CommandError::new(CHANNEL, err))
        })
        .await
    }

    /// What a card says about its account (SRV-104): `Moved to <label>`,
    /// or that it waits for one; none takes it away.
    pub fn note_account(&self, session_id: &str, note: Option<&str>) {
        let inner = &self.inner;
        let changed = inner.store.update(|state| {
            let project_id = state.project_id_for_session(session_id)?.to_owned();
            let node = state.board_mut(&project_id).nodes.get_mut(session_id)?;
            if node.account_note().as_deref() == note {
                return None;
            }
            node.set_account_note(note);
            Some(project_id)
        });
        if let Some(project_id) = changed {
            let _ = inner.events.send(Event::BoardChanged { project_id });
        }
    }

    /// A renamed account's label on every card that records it.
    pub fn relabel_account(&self, account_id: &str, label: &str) {
        let inner = &self.inner;
        let changed: Vec<String> = inner.store.update(|state| {
            let mut changed = Vec::new();
            for (project_id, board) in &mut state.boards {
                for node in board.nodes.values_mut() {
                    if node.account().as_deref() == Some(account_id) && node.account_label().as_deref() != Some(label) {
                        node.set_account(Some((account_id, label)));
                        if !changed.contains(project_id) {
                            changed.push(project_id.clone());
                        }
                    }
                }
            }
            changed
        });
        for project_id in changed {
            let _ = inner.events.send(Event::BoardChanged { project_id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_claude_code_however_it_was_found() {
        assert!(is_claude(&Program::Claude));
        assert!(is_claude(&Program::Path("/usr/local/bin/claude".into())));
        #[cfg(windows)]
        assert!(is_claude(&Program::Path(r"C:\Users\me\.local\bin\claude.exe".into())));
        assert!(!is_claude(&Program::Path("/usr/bin/git".into())));
        assert!(!is_claude(&Program::Path("/opt/claude-tools/run".into())));
    }
}
