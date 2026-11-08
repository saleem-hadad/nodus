//! Quick headless `claude -p` calls (naming, handoff summaries): starting
//! one, and reading its answer.

use std::path::Path;
use std::time::Duration;

use nodus_domain::js;
use nodus_platform::env::EnvMap;
use nodus_platform::process::{Command, Signal};
use serde_json::Value;

use crate::nodeerr::spawn_message;
use crate::value::{as_str, field, truthy};

/// How a `claude -p` call ended.
pub(crate) struct Finished {
    /// Its exit code; none when a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) enum RunError {
    /// It couldn't be started, worded as Node words it.
    Spawn(String),
    TimedOut,
}

/// Runs `claude` with stdin closed (or `claude -p` waits for piped input)
/// and its output collected; past `timeout` it's killed (SIGKILL, or
/// TerminateProcess on Windows), with whatever it started. Started through
/// `nodus_platform::process`, so no console window opens on Windows and it
/// goes when the caller gives up on it.
pub(crate) async fn run(
    claude: &Path,
    args: &[String],
    cwd: &Path,
    env: &EnvMap,
    timeout: Duration,
) -> Result<Finished, RunError> {
    let output = Command::new(claude)
        .args(args)
        .cwd(cwd)
        .env(env.clone())
        .timeout(timeout)
        .kill_signal(Signal::KILL)
        .output()
        .await
        .map_err(|err| RunError::Spawn(spawn_message(&err.source, claude)))?;
    if output.timed_out {
        return Err(RunError::TimedOut);
    }
    Ok(Finished {
        code: output.code(),
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    })
}

/// `JSON.parse(stdout.trim())`, when it parses.
pub(crate) fn parse_reply(stdout: &str) -> Option<Value> {
    nodus_domain::json::from_str(js::trim(stdout)).ok()
}

/// `parsed.result` when the reply isn't an error and it's a string.
pub(crate) fn result_of(parsed: Option<&Value>) -> Option<&str> {
    if !parsed.is_some_and(|p| truthy(Some(p))) || truthy(field(parsed, "is_error")) {
        return None;
    }
    as_str(field(parsed, "result"))
}

/// Why a call gave no answer: the reply's `result` when it's a non-empty
/// string, else the last 3 lines of stderr, else the exit code (SESS-113,
/// SESS-172).
pub(crate) fn failure(parsed: Option<&Value>, finished: &Finished) -> String {
    if let Some(result) = as_str(field(parsed, "result")).filter(|r| !r.is_empty()) {
        return result.to_owned();
    }
    let lines: Vec<&str> = js::trim(&finished.stderr).split('\n').collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" ");
    if !tail.is_empty() {
        return tail;
    }
    match finished.code {
        Some(code) => format!("claude exited with code {code}"),
        None => "claude exited with code null".into(),
    }
}

// A shell script stands in for `claude`, so these tests run on macOS and Linux.
#[cfg(all(test, unix))]
pub(crate) mod stand_in {
    //! A stand-in `claude` for tests: it logs its arguments and prints what
    //! it's told to. The real one is never run.

    use std::path::{Path, PathBuf};

    /// Writes an executable script that logs its arguments to
    /// `<dir>/argv.<pid>.log` (each ended by NUL), so calls running at once
    /// don't mix, and its working folder to `<dir>/cwd.log`, then runs `body`.
    pub fn write(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("claude");
        let text = format!(
            "#!/bin/sh\nlog='{dir}/argv.'$$'.log'\nfor a in \"$@\"; do printf '%s\\000' \"$a\" >> \"$log\"; done\npwd >> '{dir}/cwd.log'\n{body}\n",
            dir = dir.display(),
        );
        std::fs::write(&script, text).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    /// Each call's arguments, in no particular order.
    pub fn calls(dir: &Path) -> Vec<Vec<String>> {
        let mut calls = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !(name.starts_with("argv.") && name.ends_with(".log")) {
                continue;
            }
            let log = std::fs::read_to_string(&path).unwrap();
            let mut args: Vec<String> = log.split('\0').map(str::to_owned).collect();
            // Each argument ends with NUL, so the last piece is empty.
            args.pop();
            calls.push(args);
        }
        calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn finished(stderr: &str, code: Option<i32>) -> Finished {
        Finished {
            code,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn reads_a_result_unless_its_an_error() {
        let ok = parse_reply("  {\"type\":\"result\",\"is_error\":false,\"result\":\"Hi\"}\n");
        assert_eq!(result_of(ok.as_ref()), Some("Hi"));
        let err = Some(json!({ "is_error": true, "result": "Credit balance too low" }));
        assert_eq!(result_of(err.as_ref()), None);
        assert_eq!(failure(err.as_ref(), &finished("", Some(1))), "Credit balance too low");
        assert_eq!(result_of(Some(&json!({ "result": 5 }))), None);
        assert_eq!(parse_reply("not json"), None);
    }

    #[test]
    fn says_why_from_stderr_or_the_exit_code() {
        assert_eq!(failure(None, &finished("a\nb\nc\nd\n", Some(1))), "b c d");
        assert_eq!(failure(None, &finished("  ", Some(2))), "claude exited with code 2");
        assert_eq!(failure(None, &finished("", None)), "claude exited with code null");
        assert_eq!(
            failure(Some(&json!({ "result": "" })), &finished("oops", Some(1))),
            "oops"
        );
    }
}
