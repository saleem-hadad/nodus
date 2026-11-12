//! `exec`: the host runs a process for the app outside any terminal, as its
//! own user, and answers with what it printed (SRV-005).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nodus_host::Logger;
use nodus_host::transport::{ClientStream, connect};
use nodus_host_proto::{MessageReader, PROTOCOL};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(15);

struct TestHost {
    dir: tempfile::TempDir,
    socket: PathBuf,
}

impl TestHost {
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-exec-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("host.sock");
        let (log, _lines) = Logger::memory();
        let args = [
            socket.to_string_lossy().into_owned(),
            socket.with_extension("key").to_string_lossy().into_owned(),
        ];
        tokio::spawn(async move { nodus_host::run(&args, log).await });
        for _ in 0..100 {
            if connect(&socket).await.is_ok() {
                return Self { dir, socket };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the host didn't start");
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }
}

struct Conn {
    stream: ClientStream,
    reader: MessageReader,
    queue: std::collections::VecDeque<Value>,
}

impl Conn {
    async fn app(socket: &Path, protocol: u32) -> Self {
        let mut conn = Self {
            stream: connect(socket).await.unwrap(),
            reader: MessageReader::new(),
            queue: Default::default(),
        };
        conn.send(json!({"type": "hello", "protocol": protocol})).await;
        let welcome = conn.recv().await.expect("a welcome");
        assert_eq!(welcome["type"], "welcome");
        conn
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

    /// Runs one exec and gives its result.
    async fn exec(&mut self, exec: Value) -> Value {
        let id = exec["id"].clone();
        let mut msg = json!({"type": "exec"});
        msg.as_object_mut().unwrap().extend(exec.as_object().unwrap().clone());
        self.send(msg).await;
        let result = self.recv().await.expect("an exec-result");
        assert_eq!(
            (&result["type"], &result["id"]),
            (&json!("exec-result"), &id),
            "{result}"
        );
        result
    }
}

fn path_env(extra: &[(&str, &str)]) -> Value {
    let mut env = json!({"PATH": "/usr/bin:/bin"});
    for (name, value) in extra {
        env[*name] = json!(value);
    }
    env
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

async fn gone(pid: i32) -> bool {
    for _ in 0..150 {
        // SAFETY: signal 0 only asks whether the process is there.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn pid_in(file: &Path) -> i32 {
    for _ in 0..250 {
        if let Ok(text) = std::fs::read_to_string(file)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no pid in {}", file.display());
}

/// It runs with exactly the environment, folder and stdin it's given, and a
/// non-zero exit is an answer.
#[tokio::test]
async fn runs_in_the_folder_and_environment_given() {
    let host = TestHost::start().await;
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let result = app
        .exec(json!({
            "id": 1,
            "argv": ["sh", "-c", "pwd; echo \"$WHO|$HOME\"; cat; echo oops >&2; exit 3"],
            "cwd": host.root(),
            "env": path_env(&[("WHO", "nodus-run")]),
            "stdin": "fed héllo\n",
        }))
        .await;
    assert_eq!(
        result,
        json!({
            "type": "exec-result", "id": 1, "code": 3,
            "stdout": format!("{}\nnodus-run|\nfed héllo\n", host.root().display()),
            "stderr": "oops\n",
        })
    );

    // Without an environment, the host's own.
    let own = app
        .exec(json!({"id": 2, "argv": ["/bin/sh", "-c", "echo \"$PATH\""]}))
        .await;
    assert_eq!(own["stdout"], format!("{}\n", std::env::var("PATH").unwrap()));
}

#[tokio::test]
async fn says_why_it_never_started() {
    let host = TestHost::start().await;
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let missing = app
        .exec(json!({"id": 1, "argv": ["nodus-no-such-program"], "env": path_env(&[])}))
        .await;
    assert_eq!(
        missing,
        json!({"type": "exec-result", "id": 1, "stdout": "", "stderr": "", "error": "spawn nodus-no-such-program ENOENT", "errno": 2})
    );
    let nowhere = app
        .exec(json!({"id": 2, "argv": ["/bin/true"], "cwd": host.root().join("nowhere")}))
        .await;
    assert_eq!(nowhere["error"], "spawn /bin/true ENOENT");
    let empty = app.exec(json!({"id": 3, "argv": []})).await;
    assert_eq!(empty["error"], "An exec needs a program to run");
}

/// `claude: true` is Claude Code's program, found on the request's PATH by
/// the host, which is where it runs.
#[tokio::test]
async fn finds_claude_where_it_runs() {
    let host = TestHost::start().await;
    let bin = host.root().join("bin");
    std::fs::create_dir(&bin).unwrap();
    script(&bin, "claude", "echo \"stand-in $*\"");
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let found = app
        .exec(json!({"id": 1, "argv": ["claude", "-p", "hi"], "claude": true, "env": {"PATH": path}}))
        .await;
    assert_eq!(
        (&found["code"], &found["stdout"]),
        (&json!(0), &json!("stand-in -p hi\n"))
    );

    let none = app
        .exec(json!({"id": 2, "argv": ["claude", "-p"], "claude": true, "env": {"PATH": "/nowhere"}}))
        .await;
    assert!(
        none["error"]
            .as_str()
            .unwrap()
            .starts_with("Claude Code CLI not found."),
        "{none}"
    );
    assert_eq!(none["errno"], 2);
}

/// Past its timeout the whole tree is stopped with the signal asked for.
#[tokio::test]
async fn stops_the_tree_at_the_timeout() {
    let host = TestHost::start().await;
    let pidfile = host.root().join("grandchild");
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let started = Instant::now();
    let result = app
        .exec(json!({
            "id": 1,
            "argv": ["/bin/sh", "-c", format!("sleep 30 & echo $! > '{}'; echo started; wait", pidfile.display())],
            "timeoutMs": 300,
            "killSignal": 9,
        }))
        .await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        result,
        json!({"type": "exec-result", "id": 1, "signal": 9, "stdout": "started\n", "stderr": "", "timedOut": true})
    );
    assert!(
        gone(pid_in(&pidfile).await).await,
        "the grandchild outlived the timeout"
    );
}

/// What's kept of each stream: up to `first` and on, or up to `maxBuffer`
/// and stopped. (A stream with no cap has the host's, `exec::OUTPUT_LIMIT`,
/// which is too big to fill quickly here.)
#[tokio::test]
async fn caps_what_it_keeps() {
    let host = TestHost::start().await;
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let flood = app
        .exec(json!({"id": 1, "argv": ["/usr/bin/yes"], "stdout": {"maxBuffer": 1000}}))
        .await;
    assert_eq!(flood["stdout"].as_str().unwrap().len(), 1000);
    assert_eq!((&flood["overflow"], &flood["signal"]), (&json!("stdout"), &json!(15)));

    let first = app
        .exec(json!({
            "id": 2,
            "argv": ["/bin/sh", "-c", "printf 0123456789abcdef; printf err >&2"],
            "stdout": {"first": 10},
            "stderr": "ignore",
        }))
        .await;
    assert_eq!(
        first,
        json!({"type": "exec-result", "id": 2, "code": 0, "stdout": "0123456789", "stderr": ""})
    );
}

/// The app giving up on a run, or going, stops it.
#[tokio::test]
async fn stops_what_the_app_gives_up_on() {
    let host = TestHost::start().await;
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let pidfile = host.root().join("cancelled");
    app.send(json!({
        "type": "exec", "id": 1,
        "argv": ["/bin/sh", "-c", format!("echo $$ > '{}'; exec sleep 30", pidfile.display())],
    }))
    .await;
    let pid = pid_in(&pidfile).await;
    app.send(json!({"type": "exec-cancel", "id": 1})).await;
    let result = app.recv().await.unwrap();
    assert_eq!((&result["type"], &result["signal"]), (&json!("exec-result"), &json!(9)));
    assert!(gone(pid).await);

    let pidfile = host.root().join("left");
    app.send(json!({
        "type": "exec", "id": 2,
        "argv": ["/bin/sh", "-c", format!("echo $$ > '{}'; exec sleep 30", pidfile.display())],
    }))
    .await;
    let pid = pid_in(&pidfile).await;
    // A second app keeps the host up; the first one leaves.
    let _other = Conn::app(&host.socket, PROTOCOL).await;
    drop(app);
    assert!(gone(pid).await, "the run outlived the app that asked for it");
}

/// HOST-039: a connection on another protocol may only stop the host.
#[tokio::test]
async fn runs_nothing_for_another_protocol() {
    let host = TestHost::start().await;
    let marker = host.root().join("ran");
    let mut other = Conn::app(&host.socket, PROTOCOL + 1).await;
    other
        .send(json!({"type": "exec", "id": 1, "argv": ["/usr/bin/touch", marker]}))
        .await;
    let mut app = Conn::app(&host.socket, PROTOCOL).await;
    let ours = host.root().join("ours");
    let ran = app.exec(json!({"id": 9, "argv": ["/usr/bin/touch", &ours]})).await;
    assert_eq!(ran["code"], 0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(ours.exists());
    assert!(!marker.exists());
}
