//! A real server a "session" started is found by its environment, and stops
//! when asked (DRAWER-020…024).

#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nodus_platform::listeners::{Scanner, stop};

#[test]
fn finds_a_server_by_its_session_and_stops_it() {
    let session = format!("test-{}", std::process::id());
    let script = "import socket,sys,time\ns=socket.socket()\ns.bind(('127.0.0.1',0))\ns.listen()\nprint(s.getsockname()[1],flush=True)\ntime.sleep(60)";
    let Ok(mut child) = Command::new("python3")
        .args(["-c", script])
        // A session's own id, not the one this test may run inside of.
        .env_remove("NODUS_SESSION")
        .env("CLAUDE_CODE_SESSION_ID", &session)
        .env("AI_AGENT", "claude-code_test_agent")
        .stdout(Stdio::piped())
        .spawn()
    else {
        // No python3 here to be the server.
        return;
    };
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line.trim().parse().unwrap();

    let mut scanner = Scanner::new();
    let found = scanner.scan();
    let mine: Vec<_> = found.iter().filter(|l| l.session == session).collect();
    assert_eq!(mine.len(), 1, "{found:?}");
    assert_eq!(mine[0].port, port);
    assert_eq!(mine[0].pid, child.id());
    assert!(mine[0].started_at > 0);
    assert_eq!(mine[0].label, "python");
    // Seen once, read from the cache the next time.
    assert_eq!(scanner.scan().iter().filter(|l| l.session == session).count(), 1);

    stop(child.id()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the server didn't stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!scanner.scan().iter().any(|l| l.session == session));
}
