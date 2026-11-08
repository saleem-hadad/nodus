//! When the host starts, stays and exits, and who hears what (spec 02,
//! HOST-022–047, HOST-062, HOST-081, HOST-087).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nodus_host::Logger;
use nodus_host::transport::{ClientStream, connect};
use nodus_host_proto::MessageReader;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const TIMEOUT: Duration = Duration::from_secs(15);

struct TestHost {
    dir: tempfile::TempDir,
    socket: PathBuf,
    lines: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<i32>,
}

impl TestHost {
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-life-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("host.sock");
        let task_and_lines = spawn_host(&socket);
        wait_for(&socket).await;
        Self {
            dir,
            socket,
            lines: task_and_lines.1,
            task: task_and_lines.0,
        }
    }

    fn logged(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .map(|line| line[25..].to_owned())
            .collect()
    }

    async fn exit_code(self) -> i32 {
        tokio::time::timeout(TIMEOUT, self.task)
            .await
            .expect("the host exits")
            .unwrap()
    }

    async fn still_running(&self) -> bool {
        tokio::time::sleep(Duration::from_millis(300)).await;
        !self.task.is_finished()
    }
}

fn spawn_host(socket: &Path) -> (JoinHandle<i32>, Arc<Mutex<Vec<String>>>) {
    let (log, lines) = Logger::memory();
    let args = [
        socket.to_string_lossy().into_owned(),
        socket.with_extension("key").to_string_lossy().into_owned(),
    ];
    (tokio::spawn(async move { nodus_host::run(&args, log).await }), lines)
}

async fn wait_for(socket: &Path) {
    for _ in 0..100 {
        if connect(socket).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the host didn't start");
}

struct Conn {
    stream: ClientStream,
    reader: MessageReader,
    queue: std::collections::VecDeque<Value>,
}

impl Conn {
    async fn open(socket: &Path) -> Self {
        Self {
            stream: connect(socket).await.unwrap(),
            reader: MessageReader::new(),
            queue: Default::default(),
        }
    }

    async fn app(socket: &Path) -> (Self, Value) {
        let mut conn = Self::open(socket).await;
        conn.send(json!({"type": "hello", "protocol": 1})).await;
        let welcome = conn.recv().await.expect("a welcome");
        (conn, welcome)
    }

    async fn send(&mut self, msg: Value) {
        self.send_raw(&(nodus_domain::json::to_string(&msg).unwrap() + "\n"))
            .await;
    }

    async fn send_raw(&mut self, text: &str) {
        self.stream.write_all(text.as_bytes()).await.unwrap();
    }

    async fn recv(&mut self) -> Option<Value> {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            if let Some(msg) = self.queue.pop_front() {
                return Some(msg);
            }
            let n = tokio::time::timeout(TIMEOUT, self.stream.read(&mut buffer))
                .await
                .expect("an answer")
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.queue.extend(self.reader.feed(&buffer[..n]));
        }
    }

    /// The next message that isn't output.
    async fn recv_event(&mut self) -> Option<Value> {
        loop {
            let msg = self.recv().await?;
            if msg["type"] != "data" {
                return Some(msg);
            }
        }
    }
}

fn spawn_msg(id: u32, session: &str, file: &str, args: &[&str]) -> Value {
    json!({"type": "spawn", "id": id, "sessionId": session, "projectId": "p", "initial": "done", "file": file,
           "args": args, "env": {"PATH": "/usr/bin:/bin"}, "cwd": "/tmp", "cols": 80, "rows": 24})
}

#[tokio::test]
async fn host_023_needs_a_socket_path() {
    let (log, lines) = Logger::memory();
    assert_eq!(nodus_host::run(&[], log.clone()).await, 1);
    assert_eq!(nodus_host::run(&[String::new()], log).await, 1);
    let lines = lines.lock().unwrap();
    assert!(
        lines[0].ends_with("Z failed to start Error: usage: host <socket path> <key path>"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn host_024_leaves_when_another_host_serves_the_socket() {
    let host = TestHost::start().await;
    let (second, lines) = spawn_host(&host.socket);
    assert_eq!(tokio::time::timeout(TIMEOUT, second).await.unwrap().unwrap(), 0);
    assert!(lines.lock().unwrap()[0].ends_with("Z another host is running"));
    // Its probe said nothing, so the first host carries on.
    assert!(host.still_running().await);
    let (_app, welcome) = Conn::app(&host.socket).await;
    assert_eq!(welcome["type"], "welcome");
}

#[tokio::test]
async fn host_025_028_replaces_a_stale_socket_and_keeps_it_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("nodus-life-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("host.sock");
    std::fs::write(&socket, "left behind").unwrap();
    let (_task, _lines) = spawn_host(&socket);
    wait_for(&socket).await;
    assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
}

#[tokio::test]
async fn host_031_logs_where_it_listens() {
    let host = TestHost::start().await;
    let (_app, welcome) = Conn::app(&host.socket).await;
    let port = welcome["hookPort"].as_u64().unwrap();
    assert_eq!(
        host.logged()[0],
        format!("listening {} hooks on {port}", host.socket.display())
    );
    // HOST-030: a fresh token, 32 lowercase hex.
    let token = welcome["hookToken"].as_str().unwrap();
    assert!(token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    // No key on a Mac.
    assert!(welcome.get("key").is_none());
    let _ = host.dir.path();
}

#[tokio::test]
async fn host_036_closes_a_connection_that_doesnt_start_with_hello() {
    let host = TestHost::start().await;
    let mut conn = Conn::open(&host.socket).await;
    conn.send(json!({"type": "spawn", "id": 1})).await;
    assert_eq!(conn.recv().await, None);
    // Lines that aren't JSON don't count as a first message.
    let mut conn = Conn::open(&host.socket).await;
    conn.send_raw("not json\n\n").await;
    conn.send(json!({"type": "hello", "protocol": 1})).await;
    assert_eq!(conn.recv().await.unwrap()["type"], "welcome");
}

#[tokio::test]
async fn host_040_logs_a_message_that_fails_and_carries_on() {
    let host = TestHost::start().await;
    let mut conn = Conn::open(&host.socket).await;
    conn.send_raw("null\n").await;
    conn.send(json!({"type": "hello", "protocol": 1})).await;
    assert_eq!(conn.recv().await.unwrap()["type"], "welcome");
    assert!(
        host.logged()
            .contains(&"message failed TypeError: Cannot read properties of null (reading 'type')".to_owned())
    );
}

#[tokio::test]
async fn host_045_a_probe_without_hello_doesnt_stop_an_idle_host() {
    let host = TestHost::start().await;
    drop(Conn::open(&host.socket).await);
    assert!(host.still_running().await);
    // A connection that said hello does: no sessions, no app.
    let (app, _) = Conn::app(&host.socket).await;
    drop(app);
    assert_eq!(host.exit_code().await, 0);
}

#[tokio::test]
async fn host_047_a_host_with_live_sessions_outlives_its_apps() {
    let host = TestHost::start().await;
    let (mut a, _) = Conn::app(&host.socket).await;
    a.send(spawn_msg(1, "s1", "/bin/sleep", &["30"])).await;
    assert_eq!(a.recv_event().await.unwrap(), json!({"type": "spawned", "id": 1}));
    drop(a);
    assert!(host.still_running().await);
    let (mut b, welcome) = Conn::app(&host.socket).await;
    assert_eq!(welcome["sessions"][0]["sessionId"], "s1");
    // HOST-087: with a session but no app answering, the board tools hear at once that Nodus isn't open.
    b.send(json!({"type": "kill", "sessionId": "s1"})).await;
    assert_eq!(
        b.recv_event().await.unwrap(),
        json!({"type": "exit", "sessionId": "s1", "exitCode": 0})
    );
    // A live app keeps it running with no sessions.
    assert!(host.still_running().await);
    drop(b);
    assert_eq!(host.exit_code().await, 0);
}

#[tokio::test]
async fn host_041_042_broadcasts_to_every_app_and_answers_only_the_asker() {
    let host = TestHost::start().await;
    let (mut a, _) = Conn::app(&host.socket).await;
    let (mut b, _) = Conn::app(&host.socket).await;
    // A probe on another protocol hears nothing after its welcome.
    let mut probe = Conn::open(&host.socket).await;
    probe.send(json!({"type": "hello", "protocol": 0})).await;
    assert_eq!(probe.recv().await.unwrap()["type"], "welcome");

    b.send(spawn_msg(1, "s1", "/bin/sh", &["-c", "printf hi; exit 2"]))
        .await;
    assert_eq!(b.recv().await.unwrap(), json!({"type": "spawned", "id": 1}));
    for conn in [&mut a, &mut b] {
        let mut output = String::new();
        let exit = loop {
            let msg = conn.recv().await.unwrap();
            match msg["type"].as_str() {
                Some("data") => output.push_str(msg["data"].as_str().unwrap()),
                _ => break msg,
            }
        };
        assert_eq!(output, "hi");
        assert_eq!(exit, json!({"type": "exit", "sessionId": "s1", "exitCode": 2}));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut buffer = [0u8; 64];
    assert!(
        tokio::time::timeout(Duration::from_millis(200), probe.stream.read(&mut buffer))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn host_062_sigkill_follows_a_sigterm_the_process_ignores() {
    let host = TestHost::start().await;
    let (mut a, _) = Conn::app(&host.socket).await;
    a.send(spawn_msg(
        1,
        "s1",
        "/bin/sh",
        &["-c", "trap '' TERM; echo ready; while :; do sleep 1; done"],
    ))
    .await;
    assert_eq!(a.recv().await.unwrap()["type"], "spawned");
    while a.recv().await.unwrap()["type"] == "data" {
        // Wait for "ready" before signalling, so the trap is set.
        if host.logged().iter().any(|l| l.starts_with("spawned")) {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let killed = Instant::now();
    a.send(json!({"type": "kill", "sessionId": "s1"})).await;
    // A second kill doesn't start another timer.
    a.send(json!({"type": "kill", "sessionId": "s1"})).await;
    let exit = a.recv_event().await.unwrap();
    assert_eq!(exit, json!({"type": "exit", "sessionId": "s1", "exitCode": 0}));
    let waited = killed.elapsed();
    assert!(
        waited >= Duration::from_millis(2900) && waited < Duration::from_secs(6),
        "{waited:?}"
    );
}

#[tokio::test]
async fn host_081_refuses_an_mcp_body_over_a_mebibyte() {
    let host = TestHost::start().await;
    let (mut a, welcome) = Conn::app(&host.socket).await;
    a.send(spawn_msg(1, "s1", "/bin/sleep", &["30"])).await;
    a.recv_event().await;
    let port = u16::try_from(welcome["hookPort"].as_u64().unwrap()).unwrap();
    let token = welcome["hookToken"].as_str().unwrap();
    let body = format!("\"{}\"", "x".repeat(1024 * 1024));
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nX-Nodus-Session: s1\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let _ = stream.write_all(request.as_bytes()).await;
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(TIMEOUT, stream.read_to_end(&mut answer)).await;
    assert!(
        String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 413"),
        "{:?}",
        String::from_utf8_lossy(&answer[..answer.len().min(80)])
    );
    a.send(json!({"type": "shutdown"})).await;
}

#[tokio::test]
async fn host_087_with_no_app_the_board_tools_hear_at_once() {
    let host = TestHost::start().await;
    let (mut a, welcome) = Conn::app(&host.socket).await;
    a.send(spawn_msg(1, "s1", "/bin/sleep", &["30"])).await;
    a.recv_event().await;
    drop(a);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let port = u16::try_from(welcome["hookPort"].as_u64().unwrap()).unwrap();
    let token = welcome["hookToken"].as_str().unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#;
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nX-Nodus-Session: s1\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let started = Instant::now();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(answer.ends_with("\"isError\":true}}"), "{answer}");
    let (mut b, _) = Conn::app(&host.socket).await;
    b.send(json!({"type": "shutdown"})).await;
    assert_eq!(host.exit_code().await, 0);
}

#[tokio::test(start_paused = true)]
async fn host_032_a_host_no_app_connects_to_gives_up_after_30_seconds() {
    let dir = tempfile::Builder::new()
        .prefix("nodus-life-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("host.sock");
    let (task, lines) = spawn_host(&socket);
    let code = tokio::time::timeout(Duration::from_secs(31), task)
        .await
        .expect("gone by 30 s")
        .unwrap();
    assert_eq!(code, 0);
    assert!(lines.lock().unwrap().last().unwrap().ends_with("Z idle, exiting"));
}
