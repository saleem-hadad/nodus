//! `exec` on the wire: a [`RunRequest`] as the app's runner sends it, and
//! what came of it as the host answers (SRV-005). Both sides are here, so
//! the request the host runs is the one the app built.
//!
//! The host runs a request exactly as [`LocalRunner`](nodus_platform::runner::LocalRunner)
//! would, finding `claude` in its own place, with one limit of its own: no
//! stream keeps more than [`OUTPUT_LIMIT`] bytes.

use std::path::PathBuf;
use std::time::Duration;

use nodus_host_proto::{ExecCapture, ExecRequest, ExecResult};
use nodus_platform::process::{Capture, Exit, Output, Signal, SpawnError, Stream};
use nodus_platform::runner::{Program, RunOutput, RunRequest};

/// The most the host keeps of each stream: the largest cap any caller asks
/// for (git's 32 MiB). Past it the process is stopped and the run fails, as
/// Node's `maxBuffer` does, so a runaway process can't fill the host or the
/// socket. A request without a cap gets this one.
pub const OUTPUT_LIMIT: usize = 32 * 1024 * 1024;

/// The `exec` that asks for `request`.
pub fn request(id: f64, request: &RunRequest) -> ExecRequest {
    let program = match &request.program {
        Program::Path(path) => path.to_string_lossy().into_owned(),
        Program::Claude => "claude".to_owned(),
    };
    let mut argv = vec![program];
    argv.extend(request.args.iter().cloned());
    ExecRequest {
        id,
        argv,
        cwd: request.cwd.as_ref().map(|cwd| cwd.to_string_lossy().into_owned()),
        env: request.env.clone(),
        stdin: request.stdin.clone(),
        timeout_ms: request.timeout.map(|limit| limit.as_millis() as f64),
        claude: (request.program == Program::Claude).then_some(true),
        stdout: capture_to_wire(request.stdout),
        stderr: capture_to_wire(request.stderr),
        kill_signal: (request.kill_signal != Signal::TERM).then(|| f64::from(request.kill_signal.number())),
        new_session: request.new_session.then_some(true),
    }
}

/// What the host runs for an `exec`: the request as the app built it, with
/// each stream kept to [`OUTPUT_LIMIT`]. None without a program.
pub fn run_request(exec: &ExecRequest) -> Option<RunRequest> {
    let (program, args) = exec.argv.split_first()?;
    let program = if exec.claude == Some(true) {
        Program::Claude
    } else {
        Program::Path(PathBuf::from(program))
    };
    Some(RunRequest {
        program,
        args: args.to_vec(),
        cwd: exec.cwd.as_ref().map(PathBuf::from),
        env: exec.env.clone(),
        stdin: exec.stdin.clone(),
        stdout: capped(exec.stdout),
        stderr: capped(exec.stderr),
        timeout: exec
            .timeout_ms
            .filter(|ms| ms.is_finite() && *ms > 0.0)
            .map(|ms| Duration::from_millis(ms as u64)),
        kill_signal: exec
            .kill_signal
            .filter(|n| n.fract() == 0.0 && (1.0..=64.0).contains(n))
            .map_or(Signal::TERM, |n| Signal::from_raw(n as i32)),
        new_session: exec.new_session == Some(true),
    })
}

/// The host's answer to `exec` `id`: how it ended, or why it never started.
pub fn result(id: f64, outcome: Result<&Output, &SpawnError>) -> ExecResult {
    match outcome {
        Ok(output) => ExecResult {
            id,
            code: output.code().map(f64::from),
            signal: output.signal().map(|signal| f64::from(signal.number())),
            stdout: output.stdout_text(),
            stderr: output.stderr_text(),
            timed_out: output.timed_out.then_some(true),
            overflow: output.overflow.map(|stream| stream.to_string()),
            error: None,
            errno: None,
        },
        Err(err) => ExecResult {
            id,
            code: None,
            signal: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: None,
            overflow: None,
            error: Some(err.to_string()),
            errno: err.source.raw_os_error().map(f64::from),
        },
    }
}

/// The app's side of an `exec-result` for `request`: what the run printed
/// and how it ended, as [`Command::output`](nodus_platform::process::Command::output)
/// would have said it, or why it never started.
pub fn outcome(request: &RunRequest, result: ExecResult) -> Result<RunOutput, SpawnError> {
    let program = request.program.to_string();
    if let Some(error) = result.error {
        let errno = result.errno.filter(|n| n.fract() == 0.0).map(|n| n as i32);
        return Err(SpawnError::reported(program, error, errno));
    }
    let exit = match (result.code, result.signal) {
        (Some(code), _) => Exit::Code(code as i32),
        (None, Some(signal)) => Exit::Signal(Signal::from_raw(signal as i32)),
        (None, None) => Exit::Code(-1),
    };
    let overflow = match result.overflow.as_deref() {
        Some("stdout") => Some(Stream::Stdout),
        Some("stderr") => Some(Stream::Stderr),
        _ => None,
    };
    let mut command = program;
    for arg in &request.args {
        command.push(' ');
        command.push_str(arg);
    }
    Ok(Output {
        stdout: result.stdout.into_bytes(),
        stderr: result.stderr.into_bytes(),
        exit,
        timed_out: result.timed_out == Some(true),
        overflow,
        command,
    })
}

fn capture_to_wire(capture: Capture) -> Option<ExecCapture> {
    match capture {
        Capture::All => None,
        Capture::Ignore => Some(ExecCapture::Ignore),
        Capture::First(bytes) => Some(ExecCapture::First(bytes as f64)),
        Capture::MaxBuffer(bytes) => Some(ExecCapture::MaxBuffer(bytes as f64)),
    }
}

/// A stream's capture as the host keeps it: never more than [`OUTPUT_LIMIT`].
fn capped(capture: Option<ExecCapture>) -> Capture {
    let bytes = |n: f64| {
        if n.is_finite() && n > 0.0 {
            (n as usize).min(OUTPUT_LIMIT)
        } else {
            0
        }
    };
    match capture {
        None | Some(ExecCapture::All) => Capture::MaxBuffer(OUTPUT_LIMIT),
        Some(ExecCapture::Ignore) => Capture::Ignore,
        Some(ExecCapture::First(n)) => Capture::First(bytes(n)),
        Some(ExecCapture::MaxBuffer(n)) => Capture::MaxBuffer(bytes(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_platform::env::EnvMap;

    #[test]
    fn sends_a_request_as_it_was_built() {
        let built = RunRequest::new("git")
            .args(["status", "--porcelain"])
            .cwd("/repo")
            .env(EnvMap::from([("PATH".to_owned(), "/usr/bin".to_owned())]))
            .stdin("in")
            .max_buffer(1024)
            .timeout(Duration::from_millis(1500))
            .kill_signal(Signal::KILL)
            .new_session();
        let wire = request(3.0, &built);
        assert_eq!(wire.argv, ["git", "status", "--porcelain"]);
        assert_eq!(wire.timeout_ms, Some(1500.0));
        assert_eq!(wire.kill_signal, Some(9.0));
        assert_eq!(wire.claude, None);
        assert_eq!(run_request(&wire), Some(built));

        let claude = RunRequest::new(Program::Claude).arg("-p").stderr(Capture::First(0));
        let wire = request(4.0, &claude);
        assert_eq!((wire.argv[0].as_str(), wire.claude), ("claude", Some(true)));
        assert_eq!((wire.stdout, wire.kill_signal, wire.new_session), (None, None, None));
        // Nothing asked of stdout is the host's cap.
        let ran = run_request(&wire).unwrap();
        assert_eq!(ran.program, Program::Claude);
        assert_eq!(ran.stdout, Capture::MaxBuffer(OUTPUT_LIMIT));
        assert_eq!(ran.stderr, Capture::First(0));
        let all = ExecRequest {
            stdout: Some(ExecCapture::All),
            stderr: None,
            ..wire
        };
        let ran = run_request(&all).unwrap();
        assert_eq!(
            (ran.stdout, ran.stderr),
            (Capture::MaxBuffer(OUTPUT_LIMIT), Capture::MaxBuffer(OUTPUT_LIMIT))
        );
    }

    #[test]
    fn keeps_every_stream_to_the_limit() {
        assert_eq!(capped(Some(ExecCapture::First(1e12))), Capture::First(OUTPUT_LIMIT));
        assert_eq!(capped(Some(ExecCapture::MaxBuffer(10.0))), Capture::MaxBuffer(10));
        assert_eq!(capped(Some(ExecCapture::MaxBuffer(f64::NAN))), Capture::MaxBuffer(0));
        assert_eq!(capped(Some(ExecCapture::Ignore)), Capture::Ignore);
        let odd = ExecRequest {
            timeout_ms: Some(-1.0),
            kill_signal: Some(1.5),
            ..request(1.0, &RunRequest::new("x"))
        };
        let ran = run_request(&odd).unwrap();
        assert_eq!((ran.timeout, ran.kill_signal), (None, Signal::TERM));
        assert_eq!(run_request(&ExecRequest { argv: vec![], ..odd }), None);
    }

    #[test]
    fn answers_as_the_app_would_have_seen_it() {
        let built = RunRequest::new("git").args(["log", "-1"]);
        let output = Output {
            stdout: "héllo".as_bytes().to_vec(),
            stderr: b"warn".to_vec(),
            exit: Exit::Signal(Signal::KILL),
            timed_out: true,
            overflow: Some(Stream::Stderr),
            command: "git log -1".into(),
        };
        let answer = result(2.0, Ok(&output));
        assert_eq!(outcome(&built, answer).unwrap(), output);

        let exited = Output {
            stdout: vec![],
            stderr: vec![],
            exit: Exit::Code(128),
            timed_out: false,
            overflow: None,
            command: "git log -1".into(),
        };
        assert_eq!(outcome(&built, result(2.0, Ok(&exited))).unwrap(), exited);
    }

    #[cfg(unix)]
    #[test]
    fn says_why_it_never_started_in_the_hosts_words() {
        let built = RunRequest::new("git");
        let missing = SpawnError::reported("/opt/git", "spawn /opt/git ENOENT", Some(2));
        let err = outcome(&built, result(1.0, Err(&missing))).unwrap_err();
        assert_eq!(err.to_string(), "spawn /opt/git ENOENT");
        assert_eq!((err.program.as_str(), err.code), ("git", "ENOENT"));
        assert_eq!(err.source.kind(), std::io::ErrorKind::NotFound);
    }
}
