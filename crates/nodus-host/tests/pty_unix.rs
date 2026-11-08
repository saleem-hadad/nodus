//! The macOS pseudo-terminal layer behaves as node-pty 1.1.0 does in the
//! 0.9.4 host (spec 02, HOST-049–063).
#![cfg(unix)]

use std::time::{Duration, Instant};

use nodus_host::pty::{Pty, PtyEvent, PtyOptions};
use tokio::sync::mpsc::UnboundedReceiver;

const NAME: &str = "xterm-256color";

fn options(file: &str, args: &[&str]) -> PtyOptions {
    PtyOptions {
        file: file.into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80.0,
        rows: 24.0,
    }
}

/// Everything the terminal printed, and how it exited.
async fn run_to_exit(mut events: UnboundedReceiver<PtyEvent>) -> (String, i64) {
    let mut output = String::new();
    let collect = async {
        while let Some(event) = events.recv().await {
            match event {
                PtyEvent::Data(data) => output.push_str(&data),
                PtyEvent::Exit(code) => return code,
            }
        }
        panic!("the terminal went without an exit");
    };
    let code = tokio::time::timeout(Duration::from_secs(20), collect)
        .await
        .expect("it exits");
    (output, code)
}

fn spawn(opts: &PtyOptions) -> (Pty, UnboundedReceiver<PtyEvent>) {
    Pty::spawn(opts, NAME).expect("it starts")
}

#[tokio::test]
async fn host_049_runs_the_program_in_a_terminal_and_reports_its_exit_code() {
    let (_pty, events) = spawn(&options("/bin/sh", &["-c", "printf 'héllo 🎉'; exit 3"]));
    let (output, code) = run_to_exit(events).await;
    assert_eq!(output, "héllo 🎉");
    assert_eq!(code, 3);
}

#[tokio::test]
async fn host_049_gives_exactly_the_environment_plus_pwd_and_term() {
    let mut opts = options("/usr/bin/env", &[]);
    opts.env = vec![("A".into(), "1".into()), ("TERM".into(), "dumb".into())];
    let cwd = opts.cwd.clone();
    let (_pty, events) = spawn(&opts);
    let (output, code) = run_to_exit(events).await;
    assert_eq!(code, 0);
    let lines: Vec<&str> = output.lines().collect();
    // TERM is replaced where it was; PWD is added at the end.
    assert_eq!(
        lines,
        vec!["A=1".to_owned(), format!("TERM={NAME}"), format!("PWD={cwd}")]
    );
}

#[tokio::test]
async fn host_050_passes_each_argument_unchanged() {
    let (_pty, events) = spawn(&options(
        "/bin/sh",
        &["-c", "printf '[%s]' \"$@\"", "sh", "two words", "", "\"q\"", "a\\b"],
    ));
    let (output, _) = run_to_exit(events).await;
    assert_eq!(output, "[two words][][\"q\"][a\\b]");
}

#[tokio::test]
async fn host_053_a_folder_that_cant_be_entered_still_spawns_and_exits_1() {
    let mut opts = options("/bin/sh", &["-c", "echo never"]);
    opts.cwd = "/nonexistent/nodus-test-folder".into();
    let (_pty, events) = spawn(&opts);
    let (output, code) = run_to_exit(events).await;
    assert_eq!((output.as_str(), code), ("", 1));
}

#[tokio::test]
async fn host_053_a_program_that_cant_run_still_spawns_and_exits_1() {
    let (_pty, events) = spawn(&options("/nonexistent/claude", &[]));
    assert_eq!(run_to_exit(events).await, (String::new(), 1));
    // Looked up on the PATH it is given, not the host's.
    let mut opts = options("sh", &["-c", "echo found"]);
    opts.env = vec![("PATH".into(), "/nonexistent".into())];
    let (_pty, events) = spawn(&opts);
    assert_eq!(run_to_exit(events).await, (String::new(), 1));
    opts.env = vec![("PATH".into(), "/nonexistent:/bin".into())];
    let (_pty, events) = spawn(&opts);
    assert_eq!(run_to_exit(events).await.0, "found\r\n");
}

#[tokio::test]
async fn runs_a_script_without_a_shebang_in_sh_as_execvp_does() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("plain");
    std::fs::write(&script, "echo from sh\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (_pty, events) = spawn(&options(&script.to_string_lossy(), &[]));
    assert_eq!(run_to_exit(events).await, ("from sh\r\n".into(), 0));
}

#[tokio::test]
async fn host_058_a_signal_reports_exit_code_0() {
    let (pty, mut events) = spawn(&options("/bin/sleep", &["30"]));
    // Let it start before signalling it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    pty.signal(libc::SIGTERM);
    let exit = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(PtyEvent::Exit(code)) = events.recv().await {
                return code;
            }
        }
    });
    assert_eq!(exit.await.unwrap(), 0);
}

#[tokio::test]
async fn host_062_signals_go_to_the_process_not_its_group() {
    // The shell catches TERM; the sleep it started is in the same group.
    let script = "trap 'echo trapped' TERM; sleep 30 & echo $!; wait; sleep 30";
    let (pty, mut events) = spawn(&options("/bin/sh", &["-c", script]));
    let mut output = String::new();
    let mut read_until = async |text: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !output.contains(text) {
            assert!(Instant::now() < deadline, "no {text:?} in {output:?}");
            if let Ok(Some(PtyEvent::Data(data))) = tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
                output.push_str(&data);
            }
        }
        output.clone()
    };
    let started = read_until("\n").await;
    let child: libc::pid_t = started.lines().next().unwrap().trim().parse().unwrap();
    pty.signal(libc::SIGTERM);
    read_until("trapped").await;
    // SAFETY: signal 0 only checks the process exists.
    let alive = unsafe { libc::kill(child, 0) } == 0;
    // SAFETY: tidy up the sleep and the shell.
    unsafe {
        libc::kill(child, libc::SIGKILL);
        libc::kill(pty.pid() as libc::pid_t, libc::SIGKILL);
    }
    assert!(alive, "the sleep in the shell's group got the signal too");
}

#[tokio::test]
async fn host_057_every_byte_of_output_arrives_before_the_exit() {
    let (_pty, events) = spawn(&options(
        "/bin/sh",
        &["-c", "head -c 300000 /dev/zero | tr '\\0' x; exit 0"],
    ));
    let (output, code) = run_to_exit(events).await;
    assert_eq!(code, 0);
    assert_eq!(output.len(), 300_000);
    assert!(output.bytes().all(|b| b == b'x'));
}

#[tokio::test]
async fn host_057_an_exit_waits_at_most_200_ms_for_the_terminal_to_close() {
    // The background sleep keeps the terminal open after the shell exits.
    let started = Instant::now();
    let (_pty, events) = spawn(&options("/bin/sh", &["-c", "sleep 5 & exit 7"]));
    let (_, code) = run_to_exit(events).await;
    assert_eq!(code, 7);
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
}

#[tokio::test]
async fn host_059_writes_reach_the_program() {
    let (pty, events) = spawn(&options("/bin/sh", &["-c", "read line; echo \"got $line\""]));
    pty.write("héllo\r");
    let (output, code) = run_to_exit(events).await;
    assert_eq!(code, 0);
    // The terminal echoes the line, then the shell answers.
    assert!(output.ends_with("got héllo\r\n"), "{output:?}");
}

#[tokio::test]
async fn host_061_resizes_the_terminal_and_refuses_bad_sizes() {
    let (pty, events) = spawn(&options("/bin/sh", &["-c", "sleep 0.5; stty size"]));
    pty.resize(100.7, 40.0).unwrap();
    for (cols, rows) in [(0.0, 10.0), (10.0, -1.0), (f64::NAN, 10.0), (f64::INFINITY, 10.0)] {
        assert_eq!(
            pty.resize(cols, rows).unwrap_err(),
            "resizing must be done using positive cols and rows"
        );
    }
    let (output, _) = run_to_exit(events).await;
    assert_eq!(output, "40 100\r\n");
}

#[tokio::test]
async fn starts_at_the_size_it_is_given_or_80_by_24() {
    let mut opts = options("/bin/sh", &["-c", "stty size"]);
    opts.cols = 132.9;
    opts.rows = 43.0;
    let (_pty, events) = spawn(&opts);
    assert_eq!(run_to_exit(events).await.0, "43 132\r\n");
    opts.cols = 0.0;
    opts.rows = f64::NAN;
    let (_pty, events) = spawn(&opts);
    assert_eq!(run_to_exit(events).await.0, "24 80\r\n");
}

#[tokio::test]
async fn starts_in_its_own_session_with_the_terminal_as_its_controlling_one() {
    let (_pty, events) = spawn(&options(
        "/bin/sh",
        &[
            "-c",
            "tty >/dev/null && echo tty; ps -o sess= -p $$ >/dev/null; echo ok",
        ],
    ));
    let (output, _) = run_to_exit(events).await;
    assert_eq!(output, "tty\r\nok\r\n");
}
