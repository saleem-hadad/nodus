//! Where the child processes the core starts run, other than sessions'
//! terminals: git, the background `claude -p` calls (names, summaries,
//! drafted actions, capture's guess) and views' commands.
//!
//! A [`RunRequest`] says what to run the way a [`Command`] does, and a
//! [`Runner`] runs it. On a Mac or a PC that's [`LocalRunner`], which starts
//! it here exactly as each caller always has. `nodus serve` runs as a user
//! that holds the server's secrets and must never run repo content, so its
//! runner sends each request to the session host, which runs it as the
//! sessions' user (SRV-005, `HostRunner` in nodus-sessions).
//!
//! Which program to start is decided on the side that starts it: a bare name
//! like `git` is looked up on the request's PATH, and [`Program::Claude`] is
//! Claude Code's own program, found there as [`Env`] finds it. A runner says
//! which `claude` its callers ask for ([`Runner::claude`]).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use crate::env::{EnvMap, find_claude};
use crate::process::{Capture, Command, Output, Signal, SpawnError};
use crate::{ClaudeNotFound, Env};

/// A runner's answer, on its way.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// What a run printed and how it ended.
pub type RunOutput = Output;

/// The program a request starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// A path, or a name looked up on the request's PATH (`git`).
    Path(PathBuf),
    /// Claude Code's own program, found on the request's PATH (or the
    /// runner's own, when the request has no environment) by whichever side
    /// starts it. Not found, the run fails with [`ClaudeNotFound`]'s message.
    Claude,
}

/// The path as given, or `claude`.
impl std::fmt::Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => write!(f, "{}", path.display()),
            Self::Claude => f.write_str("claude"),
        }
    }
}

impl From<&str> for Program {
    fn from(path: &str) -> Self {
        Self::Path(path.into())
    }
}

impl From<&Path> for Program {
    fn from(path: &Path) -> Self {
        Self::Path(path.into())
    }
}

impl From<PathBuf> for Program {
    fn from(path: PathBuf) -> Self {
        Self::Path(path)
    }
}

/// A process to run, and how: a [`Command`] that can be sent elsewhere.
/// Build it as a `Command` is built, and give it to a [`Runner`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct RunRequest {
    pub program: Program,
    pub args: Vec<String>,
    /// The folder it runs in; the runner's own when none.
    pub cwd: Option<PathBuf>,
    /// Exactly this environment, or the runner's own when none.
    pub env: Option<EnvMap>,
    /// Written to stdin, which is then closed; closed from the start when none.
    pub stdin: Option<String>,
    pub stdout: Capture,
    pub stderr: Capture,
    /// The whole process tree is stopped once it has run this long.
    pub timeout: Option<Duration>,
    /// What a timeout or a `maxBuffer` sends.
    pub kill_signal: Signal,
    /// Unix: a session of its own (`setsid`), as [`Command::new_session`].
    pub new_session: bool,
}

impl RunRequest {
    /// Runs `program` with nothing else set, as [`Command::new`] would.
    pub fn new(program: impl Into<Program>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: None,
            stdin: None,
            stdout: Capture::All,
            stderr: Capture::All,
            timeout: None,
            kill_signal: Signal::TERM,
            new_session: false,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, env: EnvMap) -> Self {
        self.env = Some(env);
        self
    }

    pub fn stdin(mut self, text: impl Into<String>) -> Self {
        self.stdin = Some(text.into());
        self
    }

    pub fn stdout(mut self, capture: Capture) -> Self {
        self.stdout = capture;
        self
    }

    pub fn stderr(mut self, capture: Capture) -> Self {
        self.stderr = capture;
        self
    }

    /// Node's `maxBuffer`, on both streams.
    pub fn max_buffer(self, bytes: usize) -> Self {
        self.stdout(Capture::MaxBuffer(bytes)).stderr(Capture::MaxBuffer(bytes))
    }

    pub fn timeout(mut self, limit: Duration) -> Self {
        self.timeout = Some(limit);
        self
    }

    pub fn kill_signal(mut self, signal: Signal) -> Self {
        self.kill_signal = signal;
        self
    }

    pub fn new_session(mut self) -> Self {
        self.new_session = true;
        self
    }

    /// The [`Command`] that runs it here, with `claude` found on this side.
    /// Fails, without starting anything, when it's Claude Code and there's
    /// none on the PATH.
    pub fn command(&self) -> Result<Command, SpawnError> {
        let program = match &self.program {
            Program::Path(path) => path.clone(),
            Program::Claude => {
                let own = || std::env::var("PATH").unwrap_or_default();
                let path_var = match &self.env {
                    Some(env) => env.get("PATH").cloned().unwrap_or_default(),
                    None => own(),
                };
                // ENOENT, which Windows numbers ERROR_FILE_NOT_FOUND, both 2.
                find_claude(&path_var)
                    .ok_or_else(|| SpawnError::reported("claude", ClaudeNotFound.to_string(), Some(2)))?
            }
        };
        let mut command = Command::new(program)
            .args(&self.args)
            .stdout(self.stdout)
            .stderr(self.stderr)
            .kill_signal(self.kill_signal);
        if let Some(dir) = &self.cwd {
            command = command.cwd(dir);
        }
        if let Some(env) = &self.env {
            command = command.env(env.clone());
        }
        if let Some(text) = &self.stdin {
            command = command.stdin(text.as_bytes());
        }
        if let Some(limit) = self.timeout {
            command = command.timeout(limit);
        }
        if self.new_session {
            command = command.new_session();
        }
        Ok(command)
    }
}

/// Runs the processes the core starts (SRV-005).
pub trait Runner: Send + Sync {
    /// Starts it and collects what it prints until it ends, as
    /// [`Command::output`] does. Dropping the future before it finishes
    /// stops the process tree.
    fn run(&self, request: RunRequest) -> BoxFuture<Result<RunOutput, SpawnError>>;

    /// The `claude` to ask for (`requireClaude`): the one `env` found at
    /// startup, for a runner that starts processes here, or
    /// [`Program::Claude`] for the side that starts them to find.
    fn claude(&self, env: &Env) -> Result<Program, ClaudeNotFound>;
}

/// Starts every process here, in this process, exactly as each caller
/// started its own before there were runners.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalRunner;

impl Runner for LocalRunner {
    fn run(&self, request: RunRequest) -> BoxFuture<Result<RunOutput, SpawnError>> {
        match request.command() {
            Ok(command) => Box::pin(command.output()),
            Err(err) => Box::pin(async { Err(err) }),
        }
    }

    fn claude(&self, env: &Env) -> Result<Program, ClaudeNotFound> {
        env.require_claude().map(Program::from)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::process::Exit;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn runs_what_a_command_would() {
        let dir = tempfile::tempdir().unwrap();
        let env = EnvMap::from([
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("WHO".into(), "me".into()),
        ]);
        let output = LocalRunner
            .run(
                RunRequest::new("sh")
                    .args(["-c", "pwd; echo \"$WHO\"; cat; echo oops >&2; exit 3"])
                    .cwd(dir.path())
                    .env(env)
                    .stdin("fed\n"),
            )
            .await
            .unwrap();
        let cwd = dir.path().canonicalize().unwrap();
        assert_eq!(output.stdout_text(), format!("{}\nme\nfed\n", cwd.display()));
        assert_eq!(output.stderr_text(), "oops\n");
        assert_eq!(output.exit, Exit::Code(3));
        assert_eq!(output.command, "sh -c pwd; echo \"$WHO\"; cat; echo oops >&2; exit 3");

        let err = LocalRunner
            .run(RunRequest::new("nodus-no-such-program"))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "spawn nodus-no-such-program ENOENT");
        assert_eq!(err.reason, None);
    }

    /// The `claude` on the request's PATH, or none.
    #[tokio::test]
    async fn finds_claude_on_the_side_that_runs_it() {
        let dir = tempfile::tempdir().unwrap();
        script(dir.path(), "claude", "echo \"claude $1\"");
        let path = format!("{}:/usr/bin:/bin", dir.path().display());
        let env = EnvMap::from([("PATH".to_owned(), path)]);
        let output = LocalRunner
            .run(RunRequest::new(Program::Claude).arg("-p").env(env))
            .await
            .unwrap();
        assert_eq!(output.stdout_text(), "claude -p\n");

        let bare = EnvMap::from([("PATH".to_owned(), "/nowhere".to_owned())]);
        let err = LocalRunner
            .run(RunRequest::new(Program::Claude).env(bare))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), ClaudeNotFound.to_string());
        assert_eq!((err.program.as_str(), err.code), ("claude", "ENOENT"));
    }

    #[tokio::test]
    async fn asks_for_the_claude_found_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let claude = script(dir.path(), "claude", "true");
        let env = Env::given(
            EnvMap::from([("PATH".to_owned(), dir.path().display().to_string())]),
            None,
        );
        assert_eq!(LocalRunner.claude(&env), Ok(Program::Path(claude)));
        let none = Env::given(EnvMap::from([("PATH".to_owned(), "/nowhere".to_owned())]), None);
        assert_eq!(LocalRunner.claude(&none), Err(ClaudeNotFound));
    }

    /// Giving up on a run stops it; a run never polled starts nothing.
    #[tokio::test]
    async fn dropping_the_run_stops_it() {
        use rustix::process::{Pid, test_kill_process};
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut run = LocalRunner
            .run(RunRequest::new("/bin/sh").args(["-c", &format!("echo $$ > '{}'; exec sleep 30", pidfile.display())]));
        let pid = loop {
            tokio::select! {
                _ = &mut run => panic!("it ended"),
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
            if let Ok(pid) = std::fs::read_to_string(&pidfile)
                && let Ok(pid) = pid.trim().parse::<i32>()
            {
                break Pid::from_raw(pid).unwrap();
            }
        };
        drop(run);
        let mut gone = false;
        for _ in 0..150 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if test_kill_process(pid).is_err() {
                gone = true;
                break;
            }
        }
        assert!(gone, "the process is still running");

        let marker = dir.path().join("marker");
        drop(LocalRunner.run(RunRequest::new("/usr/bin/touch").arg(marker.display().to_string())));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!marker.exists());
    }
}
