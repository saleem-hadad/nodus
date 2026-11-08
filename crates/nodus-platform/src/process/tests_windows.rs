use std::time::Instant;

use super::*;

fn cmd() -> Command {
    Command::new(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()))
}

#[tokio::test]
async fn captures_what_it_prints_and_how_it_exits() {
    let output = cmd().args(["/c", "echo hi"]).output().await.unwrap();
    assert_eq!(output.stdout_text(), "hi\r\n");
    assert_eq!(output.exit, Exit::Code(0));
    let failed = cmd().args(["/c", "exit 3"]).output().await.unwrap();
    assert_eq!(failed.code(), Some(3));
    assert!(matches!(failed.check(), Err(ExecError::Failed { .. })));
}

#[tokio::test]
async fn passes_a_raw_command_line_as_it_is() {
    let output = cmd().raw_arg(r#"/c echo "x  y""#).output().await.unwrap();
    assert_eq!(output.stdout_text(), "\"x  y\"\r\n");
}

#[tokio::test]
async fn a_timeout_stops_it_and_reports_the_signal_sent() {
    let started = Instant::now();
    let output = cmd()
        .args(["/c", "ping -n 30 127.0.0.1 > nul"])
        .timeout(Duration::from_millis(500))
        .output()
        .await
        .unwrap();
    assert!(output.timed_out);
    assert_eq!(output.exit, Exit::Signal(Signal::TERM));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_killer_stops_it_from_elsewhere() {
    let child = cmd().args(["/c", "ping -n 30 127.0.0.1 > nul"]).spawn().unwrap();
    let killer = child.killer();
    let wait = tokio::spawn(child.wait());
    killer.kill(Signal::KILL);
    let output = wait.await.unwrap();
    assert_eq!(output.exit, Exit::Signal(Signal::KILL));
}

#[tokio::test]
async fn says_why_it_could_not_start() {
    let err = Command::new(r"C:\nonexistent\nodus-test.exe")
        .output()
        .await
        .unwrap_err();
    assert_eq!(err.code, "ENOENT");
    assert_eq!(err.to_string(), r"spawn C:\nonexistent\nodus-test.exe ENOENT");
}
