//! Running git the one way git.ts does (GIT-001, GIT-002).

use std::path::Path;

use nodus_platform::Env;
use nodus_platform::env::EnvMap;
use nodus_platform::process::{Command, ExecError, Exit, Output};

/// The default cap on each of git's output streams, past which the run fails
/// as Node's `maxBuffer` makes it (GIT-001).
pub const DEFAULT_MAX_BUFFER: usize = 32 * 1024 * 1024;
/// Diffs larger than this are not shown (GIT-018).
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

/// What git answered: its output and exit code. A non-zero exit is an answer
/// (not a repository, files that differ), not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitResult {
    pub stdout: String,
    pub code: i32,
}

/// Why git couldn't answer, with Node's wording: `spawn git ENOENT`,
/// `stdout maxBuffer length exceeded`, or `Command failed: git …` when a
/// signal stopped it.
pub type RunError = ExecError;

/// Runs git with the user's environment: `git --no-optional-locks -c
/// core.quotepath=false <args>`, looked up on the login shell's PATH, with
/// `GIT_TERMINAL_PROMPT=0` so it never waits on a password.
#[derive(Debug, Clone)]
pub struct Git {
    env: EnvMap,
}

impl Git {
    /// `spawnEnv({ GIT_TERMINAL_PROMPT: '0' })` from the login-shell environment.
    pub fn from_env(env: &Env) -> Self {
        Self::new(env.spawn_env())
    }

    /// Runs git in `spawn_env` (`spawnEnv()`), with `GIT_TERMINAL_PROMPT=0` set over it.
    pub fn new(mut spawn_env: EnvMap) -> Self {
        spawn_env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        Self { env: spawn_env }
    }

    /// The argv after `git` for these arguments (GIT-001).
    pub fn argv<'a>(args: &[&'a str]) -> Vec<&'a str> {
        // --no-optional-locks: polling status must never take index.lock from
        // under a git command Claude is running in the same repo.
        let mut argv = vec!["--no-optional-locks", "-c", "core.quotepath=false"];
        argv.extend_from_slice(args);
        argv
    }

    /// Runs git in `cwd` with a cap on each stream (GIT-001, GIT-002).
    pub async fn run(&self, cwd: &Path, args: &[&str], max_buffer: usize) -> Result<GitResult, RunError> {
        let output = Command::new("git")
            .args(Self::argv(args))
            .cwd(cwd)
            .env(self.env.clone())
            .max_buffer(max_buffer)
            .output()
            .await?;
        answer(output)
    }
}

/// GIT-002: exit 0 or any other exit code is an answer; going past the cap or
/// a signal is a failure, as `execFile`'s callback tells them apart.
fn answer(output: Output) -> Result<GitResult, RunError> {
    if output.overflow.is_some() {
        return Err(output.check().expect_err("past maxBuffer fails"));
    }
    match output.exit {
        Exit::Code(code) => Ok(GitResult {
            stdout: output.stdout_text(),
            code,
        }),
        Exit::Signal(_) => Err(output.check().expect_err("a signal fails")),
    }
}
