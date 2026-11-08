//! The Windows pseudo-console layer behaves as node-pty 1.1.0's ConPTY does
//! in the 0.9.4 host (spec 02, HOST-049–063). Runs on the Windows CI job.
#![cfg(windows)]

use std::time::Duration;

use nodus_host::pty::{Pty, PtyEvent, PtyOptions};
use tokio::sync::mpsc::UnboundedReceiver;

fn options(file: &str, args: &[&str]) -> PtyOptions {
    PtyOptions {
        file: file.into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: std::env::vars().collect(),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80.0,
        rows: 24.0,
    }
}

async fn run_to_exit(mut events: UnboundedReceiver<PtyEvent>) -> (String, i64) {
    let mut output = String::new();
    let collect = async {
        while let Some(event) = events.recv().await {
            match event {
                PtyEvent::Data(data) => output.push_str(&data),
                PtyEvent::Exit(code) => return code,
            }
        }
        panic!("the console went without an exit");
    };
    let code = tokio::time::timeout(Duration::from_secs(30), collect)
        .await
        .expect("it exits");
    (output, code)
}

#[tokio::test]
async fn host_049_runs_the_program_and_reports_its_exit_code() {
    let (_pty, events) = Pty::spawn(
        &options("cmd.exe", &["/c", "echo nodus-pty& exit /b 3"]),
        "xterm-256color",
    )
    .unwrap();
    let (output, code) = run_to_exit(events).await;
    assert!(output.contains("nodus-pty"), "{output:?}");
    assert_eq!(code, 3);
}

#[tokio::test]
async fn host_052_a_missing_program_is_file_not_found() {
    let err = Pty::spawn(&options("C:\\nonexistent\\claude.exe", &[]), "xterm-256color").unwrap_err();
    assert_eq!(err, "File not found: C:\\nonexistent\\claude.exe");
    let err = Pty::spawn(&options("nodus-no-such-program.exe", &[]), "xterm-256color").unwrap_err();
    assert_eq!(err, "File not found: ");
}

#[tokio::test]
async fn host_051_passes_the_command_line_as_quoted() {
    let (_pty, events) = Pty::spawn(
        &options("cmd.exe", &["/c", "echo", "two words", "\"q\""]),
        "xterm-256color",
    )
    .unwrap();
    let (output, code) = run_to_exit(events).await;
    assert_eq!(code, 0);
    assert!(output.contains("\"two words\" \\\"q\\\""), "{output:?}");
}

#[tokio::test]
async fn host_063_kill_ends_the_console_and_what_runs_in_it() {
    let (pty, events) = Pty::spawn(&options("cmd.exe", &[]), "xterm-256color").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    pty.kill();
    let (_, code) = run_to_exit(events).await;
    assert_eq!(code, 1);
}

#[tokio::test]
async fn host_059_writes_reach_the_program() {
    let (pty, events) = Pty::spawn(&options("cmd.exe", &["/q", "/k"]), "xterm-256color").unwrap();
    pty.write("echo typed-in\r\nexit 5\r\n");
    let (output, code) = run_to_exit(events).await;
    assert!(output.contains("typed-in"), "{output:?}");
    assert_eq!(code, 5);
}
