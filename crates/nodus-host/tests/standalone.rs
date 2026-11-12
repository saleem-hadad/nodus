//! `host --socket <path>`: the host on its own, as `nodus-host.service`
//! runs it on a server (SRV-004).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_host::Logger;
use nodus_host::transport::{ClientStream, connect};
use nodus_host_proto::{MessageReader, PROTOCOL};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const TIMEOUT: Duration = Duration::from_secs(15);

struct Standalone {
    dir: tempfile::TempDir,
    socket: PathBuf,
    lines: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<i32>,
}

impl Standalone {
    /// A host at `<scratch>/run/nodus/host.sock`, whose folders aren't there yet.
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-alone-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("run").join("nodus").join("host.sock");
        let (log, lines) = Logger::memory();
        let args = ["--socket".to_owned(), socket.to_string_lossy().into_owned()];
        let task = tokio::spawn(async move { nodus_host::run(&args, log).await });
        for _ in 0..100 {
            if connect(&socket).await.is_ok() {
                return Self {
                    dir,
                    socket,
                    lines,
                    task,
                };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the host didn't start: {:?}", lines.lock().unwrap());
    }

    fn logged(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .map(|line| line[25..].to_owned())
            .collect()
    }
}

struct Conn {
    stream: ClientStream,
    reader: MessageReader,
    queue: std::collections::VecDeque<Value>,
}

impl Conn {
    async fn hello(socket: &Path, protocol: u32) -> (Self, Value) {
        let mut conn = Self {
            stream: connect(socket).await.unwrap(),
            reader: MessageReader::new(),
            queue: Default::default(),
        };
        conn.send(json!({"type": "hello", "protocol": protocol})).await;
        let welcome = conn.recv().await.expect("a welcome");
        (conn, welcome)
    }

    async fn send(&mut self, msg: Value) {
        let line = nodus_domain::json::to_string(&msg).unwrap() + "\n";
        self.stream.write_all(line.as_bytes()).await.unwrap();
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
}

/// Whether `pid` has gone, or is a zombie nothing will reap (in a container
/// without an init, an orphan's parent never waits for it).
fn dead(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return true;
    }
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')))
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Its folder is made, its socket is the group's too, and it outlives every
/// app that leaves, however long nobody comes (HOST-032 and HOST-047 don't
/// apply).
#[tokio::test]
async fn listens_for_its_group_and_waits_for_its_app() {
    let host = Standalone::start().await;
    assert_eq!(mode(host.socket.parent().unwrap()), 0o750);
    assert_eq!(mode(&host.socket), 0o660);
    let (app, welcome) = Conn::hello(&host.socket, PROTOCOL).await;
    assert_eq!(
        (&welcome["type"], &welcome["protocol"]),
        (&json!("welcome"), &json!(PROTOCOL))
    );
    drop(app);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!host.task.is_finished(), "{:?}", host.logged());
    // It still answers.
    let (_app, welcome) = Conn::hello(&host.socket, PROTOCOL).await;
    assert_eq!(welcome["type"], "welcome");
    let logged = host.logged();
    assert!(logged[0].starts_with(&format!("listening {} hooks on ", host.socket.display())));
    assert!(!logged.iter().any(|line| line == "idle, exiting"), "{logged:?}");
}

/// An app on another protocol hears the welcome, and the host stops its
/// sessions and exits, for systemd to start the new binary's.
#[tokio::test]
async fn exits_when_another_protocol_says_hello() {
    let host = Standalone::start().await;
    let (mut app, _) = Conn::hello(&host.socket, PROTOCOL).await;
    app.send(json!({
        "type": "spawn", "id": 1, "sessionId": "s1", "projectId": "p1", "initial": "done",
        "file": "/bin/sh", "args": ["-c", "echo $$; exec sleep 30"],
        "env": {"PATH": "/usr/bin:/bin"}, "cwd": host.dir.path(), "cols": 80, "rows": 24,
    }))
    .await;
    assert_eq!(app.recv().await.unwrap(), json!({"type": "spawned", "id": 1}));
    let pid = loop {
        let msg = app.recv().await.unwrap();
        if let Some(pid) = msg["data"].as_str().and_then(|data| data.trim().parse::<i32>().ok()) {
            break pid;
        }
    };

    let (_newer, welcome) = Conn::hello(&host.socket, PROTOCOL + 1).await;
    assert_eq!(welcome["protocol"], PROTOCOL);
    let code = tokio::time::timeout(TIMEOUT, host.task)
        .await
        .expect("it exits")
        .unwrap();
    assert_eq!(code, 0);
    assert!(dead(pid), "the session outlived the host");
    let logged: Vec<String> = host.lines.lock().unwrap().iter().map(|l| l[25..].to_owned()).collect();
    let other = format!("app connected (protocol {})", PROTOCOL + 1);
    let from = logged.iter().position(|line| *line == other).unwrap();
    assert_eq!(
        logged[from..from + 3],
        [
            other.clone(),
            "another protocol, exiting".to_owned(),
            "stopping every session".to_owned()
        ]
    );
}

#[tokio::test]
async fn needs_a_socket_path_that_fits() {
    let (log, lines) = Logger::memory();
    assert_eq!(nodus_host::run(&["--socket".to_owned()], log).await, 1);
    assert!(
        lines.lock().unwrap()[0].ends_with(" failed to start Error: usage: host --socket <socket path>"),
        "{lines:?}"
    );

    // Too long for a socket: it fails rather than listen somewhere else.
    let long = format!("/tmp/{}/host.sock", "n".repeat(120));
    let (log, lines) = Logger::memory();
    assert_eq!(nodus_host::run(&["--socket".to_owned(), long.clone()], log).await, 1);
    let logged = lines.lock().unwrap();
    assert!(
        logged[0].contains(&format!(
            " failed to start Error: the socket path {long} is longer than "
        )),
        "{logged:?}"
    );
    assert!(!Path::new(&long).parent().unwrap().exists());
}
