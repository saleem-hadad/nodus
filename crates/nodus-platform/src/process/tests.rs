use std::path::Path;
use std::time::Instant;

use rustix::process::{Pid, test_kill_process};
use tempfile::TempDir;

use super::*;

fn sh(script: &str) -> Command {
    Command::new("/bin/sh").arg("-c").arg(script)
}

fn alive(pid: i32) -> bool {
    Pid::from_raw(pid).is_some_and(|pid| test_kill_process(pid).is_ok())
}

/// Waits up to `limit` for `done`, checking every 20 ms.
async fn eventually(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    done()
}

/// The pid a script wrote to `file`, once it has.
async fn pid_in(file: &Path) -> i32 {
    let mut pid = None;
    eventually(Duration::from_secs(5), || {
        pid = std::fs::read_to_string(file)
            .ok()
            .and_then(|text| text.trim().parse().ok());
        pid.is_some()
    })
    .await;
    pid.expect("the script wrote its pid")
}

#[tokio::test]
async fn captures_what_it_prints_and_how_it_exits() {
    let output = sh("printf out; printf err >&2; exit 3").output().await.unwrap();
    assert_eq!(output.stdout, b"out");
    assert_eq!(output.stderr, b"err");
    assert_eq!(output.exit, Exit::Code(3));
    assert_eq!(output.code(), Some(3));
    assert_eq!(output.signal(), None);
    assert!(!output.succeeded());
    assert!(!output.timed_out);
    assert_eq!(output.overflow, None);
    assert_eq!(output.command, "/bin/sh -c printf out; printf err >&2; exit 3");
}

#[tokio::test]
async fn fails_as_exec_file_does() {
    let err = sh("echo oops >&2; exit 2").exec().await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "Command failed: /bin/sh -c echo oops >&2; exit 2\noops\n"
    );
    assert_eq!(err.output().unwrap().code(), Some(2));
    let ok = sh("echo fine").exec().await.unwrap();
    assert_eq!(ok.stdout_text(), "fine\n");
    assert!(ok.succeeded());
}

#[tokio::test]
async fn reports_the_signal_that_stopped_it() {
    let output = sh("kill -TERM $$").output().await.unwrap();
    assert_eq!(output.exit, Exit::Signal(Signal::TERM));
    assert_eq!(output.code(), None);
    assert_eq!(output.signal().map(Signal::name).as_deref(), Some("SIGTERM"));
    assert!(matches!(output.check(), Err(ExecError::Failed { .. })));
}

#[test]
fn names_signals_as_node_does() {
    assert_eq!(Signal::KILL.name(), "SIGKILL");
    assert_eq!(Signal::TERM.to_string(), "SIGTERM");
    assert_eq!(Signal::HUP.name(), "SIGHUP");
    assert_eq!(Signal::INT.name(), "SIGINT");
    assert_eq!(Signal::from_raw(11).name(), "SIGSEGV");
    assert_eq!(Signal::from_raw(13).name(), "SIGPIPE");
    if cfg!(target_os = "macos") {
        assert_eq!(Signal::from_raw(30).name(), "SIGUSR1");
        assert_eq!(Signal::from_raw(10).name(), "SIGBUS");
    }
    assert_eq!(Signal::from_raw(99).name(), "SIG99");
}

#[tokio::test]
async fn closes_stdin_unless_given_bytes() {
    let started = Instant::now();
    let closed = Command::new("/bin/cat").output().await.unwrap();
    assert_eq!(closed.stdout, b"");
    assert!(started.elapsed() < Duration::from_secs(5));
    let fed = Command::new("/bin/cat").stdin("héllo 🎉\n").output().await.unwrap();
    assert_eq!(fed.stdout_text(), "héllo 🎉\n");
    assert_eq!(fed.exit, Exit::Code(0));
}

#[tokio::test]
async fn feeds_stdin_to_a_program_that_never_reads_it() {
    let output = sh("exit 0").stdin(vec![b'x'; 1 << 20]).output().await.unwrap();
    assert_eq!(output.exit, Exit::Code(0));
}

#[tokio::test]
async fn runs_with_the_environment_and_folder_given() {
    let dir = TempDir::new().unwrap();
    let env = EnvMap::from([("FOO".into(), "bar".into()), ("PATH".into(), "/usr/bin:/bin".into())]);
    let output = sh(r#"printf '%s|%s|%s|' "$FOO" "$HOME" "$EXTRA"; pwd -P"#)
        .env(env)
        .env_var("EXTRA", "x")
        .cwd(dir.path())
        .output()
        .await
        .unwrap();
    let real = dir.path().canonicalize().unwrap();
    assert_eq!(output.stdout_text(), format!("bar||x|{}\n", real.display()));
}

#[tokio::test]
async fn inherits_the_apps_environment_without_one() {
    let output = sh(r#"printf %s "$PATH""#).output().await.unwrap();
    assert_eq!(output.stdout_text(), std::env::var("PATH").unwrap_or_default());
}

#[tokio::test]
async fn finds_a_program_on_the_path_it_is_given() {
    let output = Command::new("echo")
        .arg("found")
        .env(EnvMap::from([("PATH".into(), "/bin".into())]))
        .output()
        .await
        .unwrap();
    assert_eq!(output.stdout_text(), "found\n");
    let err = Command::new("nodus-no-such-program")
        .env(EnvMap::from([("PATH".into(), "/usr/bin:/bin".into())]))
        .output()
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "spawn nodus-no-such-program ENOENT");
}

#[tokio::test]
async fn says_why_it_could_not_start() {
    let missing = Command::new("/nonexistent/nodus-test").output().await.unwrap_err();
    assert_eq!(missing.to_string(), "spawn /nonexistent/nodus-test ENOENT");
    assert_eq!(missing.code, "ENOENT");
    let no_folder = sh("true").cwd("/nonexistent/folder").output().await.unwrap_err();
    assert_eq!(no_folder.to_string(), "spawn /bin/sh ENOENT");

    let dir = TempDir::new().unwrap();
    let script = dir.path().join("not-executable");
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    let denied = Command::new(&script).exec().await.unwrap_err();
    assert_eq!(denied.to_string(), format!("spawn {} EACCES", script.display()));
    assert!(denied.output().is_none());
}

#[tokio::test]
async fn stops_a_process_past_its_max_buffer() {
    let output = Command::new("/usr/bin/yes").max_buffer(1000).output().await.unwrap();
    assert_eq!(output.stdout.len(), 1000);
    assert_eq!(output.overflow, Some(Stream::Stdout));
    assert_eq!(output.exit, Exit::Signal(Signal::TERM));
    assert!(!output.succeeded());
    let err = output.check().unwrap_err();
    assert_eq!(err.to_string(), "stdout maxBuffer length exceeded");
    assert_eq!(err.output().unwrap().stdout.len(), 1000);

    let stderr = sh("yes >&2")
        .stdout(Capture::All)
        .stderr(Capture::MaxBuffer(10))
        .output()
        .await
        .unwrap();
    assert_eq!(stderr.overflow, Some(Stream::Stderr));
    assert_eq!(stderr.stderr, b"y\ny\ny\ny\ny\n");
}

#[tokio::test]
async fn within_its_max_buffer_it_runs_to_the_end() {
    let output = sh("printf 1234567890").max_buffer(10).output().await.unwrap();
    assert_eq!(output.stdout, b"1234567890");
    assert_eq!(output.overflow, None);
    assert!(output.succeeded());
}

#[tokio::test]
async fn keeps_the_first_bytes_and_lets_it_finish() {
    let output = Command::new("/usr/bin/head")
        .args(["-c", "200000", "/dev/zero"])
        .stdout(Capture::First(1000))
        .stderr(Capture::Ignore)
        .output()
        .await
        .unwrap();
    assert_eq!(output.stdout.len(), 1000);
    assert_eq!(output.exit, Exit::Code(0));
    assert_eq!(output.overflow, None);
    let ignored = sh("echo out; echo err >&2")
        .stdout(Capture::Ignore)
        .output()
        .await
        .unwrap();
    assert_eq!(ignored.stdout, b"");
    assert_eq!(ignored.stderr, b"err\n");
}

#[tokio::test]
async fn a_timeout_stops_the_whole_tree() {
    let started = Instant::now();
    let output = sh("sleep 30 & echo $!; wait")
        .timeout(Duration::from_millis(300))
        .kill_signal(Signal::KILL)
        .output()
        .await
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    assert!(output.timed_out);
    assert_eq!(output.exit, Exit::Signal(Signal::KILL));
    let grandchild: i32 = output.stdout_text().trim().parse().unwrap();
    assert!(eventually(Duration::from_secs(3), || !alive(grandchild)).await);
    let err = output.check().unwrap_err();
    assert!(err.to_string().starts_with("Command failed: /bin/sh -c sleep 30"));
}

#[tokio::test]
async fn a_timeout_sends_the_kill_signal() {
    let output = Command::new("/bin/sleep")
        .arg("30")
        .timeout(Duration::from_millis(100))
        .output()
        .await
        .unwrap();
    assert!(output.timed_out);
    assert_eq!(output.exit, Exit::Signal(Signal::TERM));
}

#[tokio::test]
async fn a_timeout_gives_up_on_output_held_by_a_process_that_left() {
    if !Path::new("/usr/bin/perl").exists() {
        return;
    }
    // The perl process leaves the tree with a session of its own, keeping stdout open.
    let started = Instant::now();
    let output = sh("perl -MPOSIX -e 'POSIX::setsid(); sleep 4' & sleep 30")
        .timeout(Duration::from_millis(200))
        .kill_signal(Signal::KILL)
        .output()
        .await
        .unwrap();
    assert!(output.timed_out);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_killer_stops_it_from_elsewhere() {
    let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    assert!(child.pid() > 0);
    let killer = child.killer();
    let wait = tokio::spawn(child.wait());
    killer.kill(Signal::TERM);
    let output = wait.await.unwrap();
    assert_eq!(output.exit, Exit::Signal(Signal::TERM));
    assert!(!output.timed_out);
    // Once waited on, it signals nothing.
    killer.kill(Signal::KILL);
}

#[tokio::test]
async fn dropping_a_child_stops_the_tree() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("pid");
    let child = sh(&format!("sleep 30 & echo $! > '{}'; wait", file.display()))
        .spawn()
        .unwrap();
    let grandchild = pid_in(&file).await;
    assert!(alive(grandchild));
    drop(child);
    assert!(eventually(Duration::from_secs(3), || !alive(grandchild)).await);
}

#[tokio::test]
async fn giving_up_on_the_wait_stops_the_tree() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("pid");
    let child = sh(&format!("sleep 30 & echo $! > '{}'; wait", file.display()))
        .spawn()
        .unwrap();
    let waiting = tokio::time::timeout(Duration::from_millis(300), child.wait());
    let grandchild = pid_in(&file).await;
    assert!(waiting.await.is_err());
    assert!(eventually(Duration::from_secs(3), || !alive(grandchild)).await);
}

#[tokio::test]
async fn runs_in_a_group_or_a_session_of_its_own() {
    let script = "echo $$; ps -o pgid= -p $$; ps -o stat= -p $$";
    let grouped = sh(script).output().await.unwrap().stdout_text();
    let lines: Vec<&str> = grouped.lines().map(str::trim).collect();
    assert_eq!(lines[0], lines[1], "its own process group");
    assert!(!lines[2].contains('s'), "not a session leader: {}", lines[2]);

    let session = sh(script).new_session().output().await.unwrap().stdout_text();
    let lines: Vec<&str> = session.lines().map(str::trim).collect();
    assert_eq!(lines[0], lines[1]);
    assert!(lines[2].contains('s'), "a session leader: {}", lines[2]);
}

#[tokio::test]
async fn a_detached_process_outlives_the_wait_and_logs() {
    let dir = TempDir::new().unwrap();
    let log = dir.path().join("host.log");
    let file = File::create(&log).unwrap();
    let pid = sh("echo out; echo err >&2; ps -o stat= -p $$; echo done")
        .env_var("NODUS_TEST", "1")
        .cwd(dir.path())
        .spawn_detached(Some(file))
        .unwrap();
    assert!(pid > 0);
    let read = || std::fs::read_to_string(&log).unwrap_or_default();
    assert!(eventually(Duration::from_secs(5), || read().ends_with("done\n")).await);
    let text = read();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    assert_eq!(lines[..2], ["out", "err"]);
    assert!(lines[2].contains('s'), "a session leader: {}", lines[2]);

    let quiet = sh("echo nowhere").spawn_detached(None).unwrap();
    assert!(quiet > 0);
    let missing = Command::new("/nonexistent/nodus-test")
        .spawn_detached(None)
        .unwrap_err();
    assert_eq!(missing.to_string(), "spawn /nonexistent/nodus-test ENOENT");
}
