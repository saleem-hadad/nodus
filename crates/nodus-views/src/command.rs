//! Running one of a view's command lines in the user's shell (VIEW-076 to VIEW-080).

use std::time::Duration;

use nodus_domain::{CommandResult, js};
use nodus_platform::Env;
use nodus_platform::process::{Capture, Command, Signal};

/// How long a command may run before its whole process tree is stopped (VIEW-078).
pub const COMMAND_TIMEOUT: Duration = Duration::from_millis(2 * 60 * 1000);
const MAX_STDOUT_CHARS: usize = 1024 * 1024;
const MAX_STDERR_CHARS: usize = 64 * 1024;
const TIMED_OUT: &str = "It took longer than 2 minutes, so it was stopped";

/// The first `max` UTF-16 units of what a stream printed, decoded as UTF-8
/// with bad bytes replaced (VIEW-077). A UTF-16 unit takes at most 4 bytes,
/// so `4 × max` bytes are read; 0.9.4 decodes chunk by chunk, which differs
/// only where a character is split between two chunks.
fn text(bytes: &[u8], max: usize) -> String {
    js::slice_utf16(&String::from_utf8_lossy(bytes), 0, max)
}

/// Runs `run` in the user's shell (`shellCommand`, 01) in `cwd`, with their
/// environment and `NO_COLOR=1`, stdin closed, in a session of its own so a
/// timeout stops everything it started. A non-zero exit is still a result.
pub(crate) async fn execute(env: &Env, cwd: &str, name: &str, run: &str, clock: impl Fn() -> f64) -> CommandResult {
    let started = clock();
    let shell = env.shell_command(run);
    let mut vars = env.spawn_env();
    vars.insert("NO_COLOR".into(), "1".into());
    let output = Command::new(&shell.file)
        .args(&shell.args)
        .cwd(cwd)
        .env(vars)
        .stdout(Capture::First(MAX_STDOUT_CHARS * 4))
        .stderr(Capture::First(MAX_STDERR_CHARS * 4))
        .timeout(COMMAND_TIMEOUT)
        .kill_signal(Signal::KILL)
        .new_session()
        .output()
        .await;
    let at = clock();
    match output {
        Ok(out) => CommandResult {
            name: name.to_owned(),
            // A signal, a timeout among them, leaves no code.
            code: if out.timed_out { None } else { out.code().map(i64::from) },
            stdout: text(&out.stdout, MAX_STDOUT_CHARS),
            stderr: text(&out.stderr, MAX_STDERR_CHARS),
            at,
            duration_ms: at - started,
            error: out.timed_out.then(|| TIMED_OUT.to_owned()),
        },
        // VIEW-079: `spawn /bin/zsh ENOENT`.
        Err(err) => CommandResult {
            name: name.to_owned(),
            code: None,
            stdout: String::new(),
            stderr: String::new(),
            at,
            duration_ms: at - started,
            error: Some(err.to_string()),
        },
    }
}
