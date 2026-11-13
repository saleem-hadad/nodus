//! The app's side of the session host (spec 02, HOST-091–128), against a
//! scripted fake host where the wire matters, the Rust host where a real one
//! does, and the 0.9.4 Node host when it's been built (see `node_host`).
#![cfg(unix)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nodus_domain::{Event, EventSink, QueuedPrompt, SessionStatus};
use nodus_host_proto::{HostPaths, MessageReader, PROTOCOL, PROTOCOL_0_9_4};
use nodus_sessions::client::{
    BoardTools, BoxFuture, ClientError, HostClient, HostInfo, HostLauncher, HostPrompt, ProcessLauncher, ReplaceChoice,
    Wiring,
};
use nodus_sessions::hooks::{HookEndpoint, SessionInput};
use nodus_sessions::launch::{Command, SpawnOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// What the client tells the rest of the app, in one ordered record.

#[derive(Default)]
struct Record {
    lines: Mutex<Vec<String>>,
    /// The sessions the status machine says are waiting on the user.
    waiting: Mutex<Vec<String>>,
}

impl Record {
    fn push(&self, line: String) {
        self.lines.lock().unwrap().push(line);
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    fn set_waiting(&self, session_id: &str, waiting: bool) {
        let mut sessions = self.waiting.lock().unwrap();
        sessions.retain(|id| id != session_id);
        if waiting {
            sessions.push(session_id.to_owned());
        }
    }

    async fn wait_for(&self, line: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.lines().iter().any(|l| l == line) {
            assert!(Instant::now() < deadline, "no {line:?} in {:#?}", self.lines());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

struct Status(Arc<Record>);

impl SessionInput for Status {
    fn process_started(&self, session_id: &str, initial: SessionStatus) {
        self.0.push(format!("started {session_id} {}", initial.as_str()));
    }
    fn process_exited(&self, session_id: &str) {
        self.0.push(format!("ended {session_id}"));
    }
    fn hook(&self, event: &str, session_id: &str, payload: &Value, at: Option<f64>) {
        self.0.push(format!("hook {session_id} {event} {payload} {at:?}"));
    }
    fn user_input(&self, session_id: &str, data: &str, at: Option<f64>) {
        self.0.push(format!("input {session_id} {data:?} {at:?}"));
    }
    fn is_waiting(&self, session_id: &str) -> bool {
        self.0.waiting.lock().unwrap().iter().any(|id| id == session_id)
    }
}

struct Sink(Arc<Record>);

impl EventSink for Sink {
    fn emit(&self, event: Event) {
        match event {
            Event::PtyData { session_id, data } => self.0.push(format!("data {session_id} {data:?}")),
            Event::PtyExit { session_id, exit_code } => self.0.push(format!("exit {session_id} {exit_code}")),
            Event::Queue { session_id, items } => {
                let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
                self.0.push(format!("queue {session_id} [{}]", ids.join(",")));
            }
            other => self.0.push(format!("{other:?}")),
        }
    }
}

struct Tools;

impl BoardTools for Tools {
    fn answer(&self, _session_id: &str, message: Value) -> BoxFuture<Result<Option<Value>, String>> {
        Box::pin(async move {
            match message["method"].as_str() {
                Some("fail") => Err("Error: no board".into()),
                Some("none") => Ok(None),
                _ => Ok(Some(
                    json!({"jsonrpc": "2.0", "id": message["id"], "result": {"ok": true}}),
                )),
            }
        })
    }
}

struct Prompt {
    choice: ReplaceChoice,
    asked: AtomicUsize,
}

impl HostPrompt for Prompt {
    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let choice = self.choice;
        Box::pin(async move { choice })
    }
}

/// Starts the Rust host in this process, as `Nodus host` would.
#[derive(Default)]
struct InProcess {
    started: AtomicUsize,
    logs: Mutex<Vec<Arc<Mutex<Vec<String>>>>>,
}

impl HostLauncher for InProcess {
    fn start(&self, paths: &HostPaths) -> std::io::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);
        nodus_host_proto::ensure_private_dir(&paths.dir)?;
        let (log, lines) = nodus_host::Logger::memory();
        self.logs.lock().unwrap().push(lines);
        let args = [
            paths.socket.to_string_lossy().into_owned(),
            paths.key.to_string_lossy().into_owned(),
        ];
        tokio::spawn(async move { nodus_host::run(&args, log).await });
        Ok(())
    }
}

impl InProcess {
    fn logged(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .flat_map(|lines| lines.lock().unwrap().clone())
            .map(|l| l[25..].to_owned())
            .collect()
    }
}

/// A launcher that starts nothing.
struct Nothing;

impl HostLauncher for Nothing {
    fn start(&self, _paths: &HostPaths) -> std::io::Result<()> {
        Ok(())
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    user_data: PathBuf,
    record: Arc<Record>,
    endpoint: Arc<HookEndpoint>,
    prompt: Arc<Prompt>,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-client-")
            .tempdir_in("/tmp")
            .unwrap();
        Self {
            user_data: dir.path().join("ud"),
            _dir: dir,
            record: Arc::default(),
            endpoint: Arc::new(HookEndpoint::new()),
            prompt: Arc::new(Prompt {
                choice: ReplaceChoice::StopSessions,
                asked: AtomicUsize::new(0),
            }),
        }
    }

    fn with_choice(mut self, choice: ReplaceChoice) -> Self {
        self.prompt = Arc::new(Prompt {
            choice,
            asked: AtomicUsize::new(0),
        });
        self
    }

    fn client(&self, launcher: Arc<dyn HostLauncher>) -> HostClient {
        HostClient::new(
            &self.user_data,
            Wiring {
                events: Arc::new(Sink(Arc::clone(&self.record))),
                status: Arc::new(Status(Arc::clone(&self.record))),
                endpoint: Arc::clone(&self.endpoint),
                board_tools: Arc::new(Tools),
                prompt: Arc::clone(&self.prompt) as Arc<dyn HostPrompt>,
                launcher,
                server: None,
            },
        )
    }

    fn paths(&self) -> HostPaths {
        nodus_host_proto::host_paths(&self.user_data, nodus_host_proto::Platform::current())
    }

    /// A fake host listening where the client looks.
    fn fake_host(&self) -> FakeHost {
        let paths = self.paths();
        nodus_host_proto::ensure_private_dir(&paths.dir).unwrap();
        FakeHost(UnixListener::bind(&paths.socket).unwrap())
    }
}

fn shell(script: &str) -> impl FnOnce(&SpawnOptions, HostInfo) -> Result<Command, String> + Send + 'static {
    let script = script.to_owned();
    move |_, _| {
        Ok(Command {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), script],
            env: [("PATH".to_owned(), "/usr/bin:/bin".to_owned())].into_iter().collect(),
        })
    }
}

fn opts(session: &str) -> SpawnOptions {
    SpawnOptions {
        session_id: session.into(),
        project_id: "p1".into(),
        cwd: "/tmp".into(),
        ..SpawnOptions::default()
    }
}

// ---------------------------------------------------------------------------
// A host whose every word the test writes.

struct FakeHost(UnixListener);

impl FakeHost {
    async fn accept(&self) -> FakeConn {
        let (stream, _) = tokio::time::timeout(TIMEOUT, self.0.accept())
            .await
            .expect("a connection")
            .unwrap();
        FakeConn {
            stream,
            reader: MessageReader::new(),
            queue: VecDeque::new(),
        }
    }
}

struct FakeConn {
    stream: UnixStream,
    reader: MessageReader,
    queue: VecDeque<Value>,
}

impl FakeConn {
    async fn send(&mut self, msgs: &[Value]) {
        let text: String = msgs
            .iter()
            .map(|m| nodus_domain::json::to_string(m).unwrap() + "\n")
            .collect();
        self.stream.write_all(text.as_bytes()).await.unwrap();
    }

    async fn recv(&mut self) -> Option<Value> {
        let mut buffer = vec![0u8; 65536];
        loop {
            if let Some(msg) = self.queue.pop_front() {
                return Some(msg);
            }
            let n = tokio::time::timeout(TIMEOUT, self.stream.read(&mut buffer))
                .await
                .expect("a message")
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.queue.extend(self.reader.feed(&buffer[..n]));
        }
    }

    /// Answers the hello with this welcome.
    async fn greet(&mut self, welcome: Value) {
        assert_eq!(self.recv().await, Some(json!({"type": "hello", "protocol": PROTOCOL})));
        self.send(&[welcome]).await;
    }
}

fn welcome(sessions: Value) -> Value {
    let mut welcome = json!({"type": "welcome", "protocol": PROTOCOL, "hookPort": 4000, "hookToken": "tok_en-1",
                             "sessions": [], "mcp": true, "hookAuthHeader": true});
    welcome["sessions"] = sessions;
    welcome
}

fn session(id: &str, project: &str, buffer: &str) -> Value {
    json!({"sessionId": id, "projectId": project, "initial": "done", "buffer": buffer, "history": []})
}

/// A client adopted by a fake host with these sessions, and the host's end.
async fn adopted(setup: &Setup, sessions: Value) -> (HostClient, FakeConn) {
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut conn = host.accept().await;
    conn.greet(welcome(sessions)).await;
    connecting.await.unwrap().unwrap();
    (client, conn)
}

// ---------------------------------------------------------------------------
// Connecting

#[tokio::test]
async fn host_095_launch_adopts_and_never_starts_a_host() {
    let setup = Setup::new();
    let launcher = Arc::new(InProcess::default());
    let client = setup.client(Arc::clone(&launcher) as Arc<dyn HostLauncher>);
    client.connect_host(false).await.unwrap();
    assert_eq!(launcher.started.load(Ordering::SeqCst), 0);
    assert!(client.live_session_ids().is_empty());
}

#[tokio::test]
async fn host_096_100_108_adopts_every_session_with_its_history_in_order() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut conn = host.accept().await;
    assert_eq!(conn.recv().await, Some(json!({"type": "hello", "protocol": PROTOCOL})));
    let s1 = json!({"sessionId": "s1", "projectId": "p1", "initial": "working", "buffer": "old output",
        "history": [{"hook": "UserPromptSubmit", "payload": {"n": 1}, "at": 1000}, {"input": "\u{1b}", "at": 2000}],
        "statusLine": {"payload": {"model": {"display_name": "M"}}, "at": 3000}});
    // A message in the same read as the welcome is handled after it.
    conn.send(&[
        welcome(json!([s1, session("s2", "p2", "")])),
        json!({"type": "data", "sessionId": "s1", "data": "new"}),
    ])
    .await;
    connecting.await.unwrap().unwrap();
    setup.record.wait_for("data s1 \"new\"").await;
    assert_eq!(
        setup.record.lines(),
        vec![
            "started s1 working",
            "hook s1 UserPromptSubmit {\"n\":1} Some(1000.0)",
            "input s1 \"\\u{1b}\" Some(2000.0)",
            "hook s1 StatusLine {\"model\":{\"display_name\":\"M\"}} Some(3000.0)",
            "started s2 done",
            "data s1 \"new\"",
        ]
    );
    assert_eq!(client.live_session_ids(), vec!["s1", "s2"]);
    assert_eq!(client.snapshot("s1"), "old outputnew");
    assert_eq!(client.snapshot("s2"), "");
    // HOST-105–107: the endpoint and the curl config, and the board tools.
    let env = setup.endpoint.env("s1");
    assert_eq!(env["NODUS_PORT"], "4000");
    assert_eq!(env["NODUS_TOKEN"], "tok_en-1");
    let config = setup.paths().hook_config;
    assert_eq!(env["NODUS_HOOK_CONFIG"], config.to_string_lossy());
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "header = \"Authorization: Bearer tok_en-1\"\n"
    );
    assert!(client.relays_mcp());
    // HOST-093: connected, so it returns at once.
    client.connect_host(true).await.unwrap();
}

#[tokio::test]
async fn host_105_an_older_host_gets_no_curl_config_and_no_board_tools() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut conn = host.accept().await;
    conn.greet(json!({"type": "welcome", "protocol": PROTOCOL, "hookPort": 4001, "hookToken": "a\"b", "sessions": []}))
        .await;
    connecting.await.unwrap().unwrap();
    let env = setup.endpoint.env("s1");
    assert_eq!(
        (env["NODUS_PORT"].as_str(), env.get("NODUS_HOOK_CONFIG")),
        ("4001", None)
    );
    assert!(!client.relays_mcp());
    assert!(!setup.paths().hook_config.exists());
}

#[tokio::test]
async fn host_105_a_token_that_cant_go_in_a_config_leaves_hooks_on_the_url() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut conn = host.accept().await;
    conn.greet(json!({"type": "welcome", "protocol": PROTOCOL, "hookPort": 1, "hookToken": "x\"y", "sessions": [], "hookAuthHeader": true}))
        .await;
    connecting.await.unwrap().unwrap();
    assert_eq!(setup.endpoint.env("s").get("NODUS_HOOK_CONFIG"), None);
}

#[tokio::test(start_paused = true)]
async fn host_097_a_host_that_never_answers_didnt_answer() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let _conn = host.accept().await;
    assert_eq!(
        connecting.await.unwrap(),
        Err(ClientError::Failed("The session host didn’t answer".into()))
    );
}

#[tokio::test]
async fn host_097_a_host_that_closes_before_answering_didnt_answer() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    drop(host.accept().await);
    assert_eq!(
        connecting.await.unwrap(),
        Err(ClientError::Failed("The session host didn’t answer".into()))
    );
}

#[tokio::test]
async fn host_099_102_104_stops_a_host_on_another_protocol_then_starts_one() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let launcher = Arc::new(InProcess::default());
    let client = setup.client(Arc::clone(&launcher) as Arc<dyn HostLauncher>);
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(true).await }
    });
    let mut old = host.accept().await;
    old.greet(json!({"type": "welcome", "protocol": PROTOCOL + 1, "hookPort": 1, "hookToken": "t", "sessions": []}))
        .await;
    assert_eq!(old.recv().await, Some(json!({"type": "shutdown"})));
    // The old host goes, and its socket with it.
    drop(old);
    drop(host);
    std::fs::remove_file(setup.paths().socket).unwrap();
    connecting.await.unwrap().unwrap();
    assert_eq!(setup.prompt.asked.load(Ordering::SeqCst), 1);
    assert_eq!(launcher.started.load(Ordering::SeqCst), 1);
    client.stop_all().await;
}

#[tokio::test]
async fn host_103_quit_closes_the_connection_and_says_so() {
    let setup = Setup::new().with_choice(ReplaceChoice::Quit);
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut old = host.accept().await;
    old.greet(json!({"type": "hello-back"})).await;
    assert_eq!(connecting.await.unwrap(), Err(ClientError::Quit));
    assert_eq!(old.recv().await, None);
}

/// The welcome a 0.9.4 Node host sends with one session running, as
/// `nodus-host/tests/fixtures/wire-transcript.txt` recorded it.
fn welcome_0_9_4() -> Value {
    json!({"type": "welcome", "protocol": PROTOCOL_0_9_4, "hookPort": 52341, "hookToken": "tok",
           "sessions": [{"sessionId": "s1", "projectId": "p1", "initial": "working", "buffer": "recent output\r\n",
                         "history": [{"hook": "UserPromptSubmit", "payload": {"session_id": "s1"}, "at": 1000}]}],
           "mcp": true, "hookAuthHeader": true})
}

/// U1: the first launch after the update finds 0.9.4's Node host still
/// running. It's another version (HOST-099), so the user is asked (HOST-102),
/// and Stop Sessions shuts it down (HOST-104) without adopting a session.
#[tokio::test]
async fn host_099_102_104_a_0_9_4_host_is_another_version_and_its_sessions_are_stopped() {
    let setup = Setup::new();
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut old = host.accept().await;
    old.greet(welcome_0_9_4()).await;
    assert_eq!(old.recv().await, Some(json!({"type": "shutdown"})));
    drop(old);
    connecting.await.unwrap().unwrap();
    assert_eq!(setup.prompt.asked.load(Ordering::SeqCst), 1);
    assert!(client.live_session_ids().is_empty());
    assert!(setup.record.lines().is_empty(), "{:?}", setup.record.lines());
    // Its hook port and token weren't taken on.
    assert_eq!(setup.endpoint.env("s1")["NODUS_PORT"], "0");
    assert!(!setup.paths().hook_config.exists());
}

/// U1, HOST-103: Quit leaves 0.9.4's host and its sessions running.
#[tokio::test]
async fn host_103_quit_leaves_a_0_9_4_host_running() {
    let setup = Setup::new().with_choice(ReplaceChoice::Quit);
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(true).await }
    });
    let mut old = host.accept().await;
    old.greet(welcome_0_9_4()).await;
    assert_eq!(connecting.await.unwrap(), Err(ClientError::Quit));
    assert_eq!(old.recv().await, None);
    assert!(client.live_session_ids().is_empty());
    assert!(setup.record.lines().is_empty());
}

#[tokio::test(start_paused = true)]
async fn host_092_a_host_that_doesnt_start_is_reported_with_its_log() {
    let setup = Setup::new();
    let client = setup.client(Arc::new(Nothing));
    let log = setup.paths().log;
    assert_eq!(
        client.connect_host(true).await,
        Err(ClientError::Failed(format!(
            "The session host didn't start. See {}",
            log.display()
        )))
    );
}

#[tokio::test]
async fn host_094_a_new_host_on_another_protocol_is_an_error() {
    struct Wrong(Arc<Mutex<Option<FakeHost>>>);
    impl HostLauncher for Wrong {
        fn start(&self, paths: &HostPaths) -> std::io::Result<()> {
            nodus_host_proto::ensure_private_dir(&paths.dir)?;
            let host = FakeHost(UnixListener::bind(&paths.socket)?);
            *self.0.lock().unwrap() = Some(host);
            Ok(())
        }
    }
    let setup = Setup::new();
    let slot = Arc::new(Mutex::new(None));
    let client = setup.client(Arc::new(Wrong(Arc::clone(&slot))));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(true).await }
    });
    let host = loop {
        if let Some(host) = slot.lock().unwrap().take() {
            break host;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let mut conn = host.accept().await;
    conn.greet(json!({"type": "welcome", "protocol": 0, "hookPort": 1, "hookToken": "t", "sessions": []}))
        .await;
    assert_eq!(
        connecting.await.unwrap(),
        Err(ClientError::Failed("The session host speaks another protocol".into()))
    );
}

// ---------------------------------------------------------------------------
// Sessions on a fake host

#[tokio::test]
async fn host_115_a_spawn_error_fails_the_start() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([])).await;
    let starting = tokio::spawn({
        let client = client.clone();
        async move { client.spawn_session(&opts("s1"), shell("true")).await }
    });
    let spawn = conn.recv().await.unwrap();
    assert_eq!(spawn["type"], "spawn");
    conn.send(&[json!({"type": "spawned", "id": spawn["id"], "error": "posix_spawnp failed."})])
        .await;
    assert_eq!(
        starting.await.unwrap(),
        Err(ClientError::Failed("posix_spawnp failed.".into()))
    );
    assert!(!client.is_live("s1"));
}

#[tokio::test]
async fn host_114_sends_the_spawn_as_0_9_4_does() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([])).await;
    let mut first = opts("s1");
    first.initial_prompt = Some("go".into());
    first.cols = Some(10.5);
    let starting = tokio::spawn({
        let client = client.clone();
        async move { client.spawn_session(&first, shell("true")).await }
    });
    let spawn = conn.recv().await.unwrap();
    assert_eq!(
        spawn,
        json!({"type": "spawn", "id": 1, "sessionId": "s1", "projectId": "p1", "initial": "working", "file": "/bin/sh",
               "args": ["-c", "true"], "env": {"PATH": "/usr/bin:/bin"}, "cwd": "/tmp", "cols": 20, "rows": 36})
    );
    // HOST-115: tracked before its output, which may come in the same read.
    conn.send(&[
        json!({"type": "spawned", "id": 1}),
        json!({"type": "data", "sessionId": "s1", "data": "x"}),
    ])
    .await;
    starting.await.unwrap().unwrap();
    setup.record.wait_for("data s1 \"x\"").await;
    assert_eq!(setup.record.lines(), vec!["started s1 working", "data s1 \"x\""]);
    // HOST-113: a live session isn't started again.
    client.spawn_session(&opts("s1"), shell("true")).await.unwrap();
    // The next request id counts on.
    let starting = tokio::spawn({
        let client = client.clone();
        async move { client.spawn_session(&opts("s2"), shell("true")).await }
    });
    let spawn = conn.recv().await.unwrap();
    assert_eq!(
        (spawn["id"].clone(), spawn["initial"].clone(), spawn["cols"].clone()),
        (json!(2), json!("done"), json!(120))
    );
    conn.send(&[
        json!({"type": "spawned", "id": 99}),
        json!({"type": "spawned", "id": 2}),
    ])
    .await;
    starting.await.unwrap().unwrap();
}

#[tokio::test]
async fn host_113_concurrent_starts_of_one_session_share_one_spawn() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([])).await;
    let starts: Vec<_> = (0..3)
        .map(|_| {
            let client = client.clone();
            tokio::spawn(async move { client.spawn_session(&opts("s1"), shell("true")).await })
        })
        .collect();
    let spawn = conn.recv().await.unwrap();
    conn.send(&[json!({"type": "spawned", "id": spawn["id"]})]).await;
    for start in starts {
        start.await.unwrap().unwrap();
    }
    // Only the one spawn went out.
    conn.send(&[json!({"type": "exit", "sessionId": "s1", "exitCode": 0})])
        .await;
    setup.record.wait_for("exit s1 0").await;
    assert_eq!(
        setup.record.lines().iter().filter(|l| l.starts_with("started")).count(),
        1
    );
}

#[tokio::test]
async fn host_116_118_coalesces_output_and_drops_unknown_sessions() {
    let setup = Setup::new();
    let (_client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    conn.send(&[
        json!({"type": "data", "sessionId": "s1", "data": "a"}),
        json!({"type": "data", "sessionId": "nobody", "data": "z"}),
        json!({"type": "data", "sessionId": "s1", "data": "b"}),
        json!({"type": "data", "sessionId": "s1", "data": "c"}),
    ])
    .await;
    setup.record.wait_for("data s1 \"abc\"").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    conn.send(&[json!({"type": "data", "sessionId": "s1", "data": "d"})])
        .await;
    setup.record.wait_for("data s1 \"d\"").await;
    assert_eq!(
        setup.record.lines(),
        vec!["started s1 done", "data s1 \"abc\"", "data s1 \"d\""]
    );
}

#[tokio::test]
async fn host_119_a_snapshot_sends_what_is_pending_first() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "old ")])).await;
    conn.send(&[json!({"type": "data", "sessionId": "s1", "data": "new"})])
        .await;
    // Wait for the data to arrive, but not for its 16 ms flush.
    let deadline = Instant::now() + TIMEOUT;
    let snapshot = loop {
        let snapshot = client.snapshot("s1");
        if snapshot.ends_with("new") || Instant::now() > deadline {
            break snapshot;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(snapshot, "old new");
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Sent once, before the snapshot returned.
    assert_eq!(setup.record.lines(), vec!["started s1 done", "data s1 \"new\""]);
    assert_eq!(client.snapshot("nobody"), "");
}

#[tokio::test]
async fn host_120_an_exit_sends_pending_output_then_ends_the_process_then_reports_it() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    let killed = client.kill("s1");
    assert_eq!(conn.recv().await, Some(json!({"type": "kill", "sessionId": "s1"})));
    conn.send(&[
        json!({"type": "data", "sessionId": "s1", "data": "bye"}),
        json!({"type": "exit", "sessionId": "s1", "exitCode": 143}),
    ])
    .await;
    tokio::time::timeout(Duration::from_secs(1), killed)
        .await
        .expect("the kill resolves on the exit");
    assert_eq!(
        setup.record.lines(),
        vec!["started s1 done", "data s1 \"bye\"", "ended s1", "exit s1 143"]
    );
    assert!(!client.is_live("s1"));
}

#[tokio::test(start_paused = true)]
async fn host_124_a_kill_gives_up_waiting_after_4_seconds() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    let started = tokio::time::Instant::now();
    let killed = client.kill("s1");
    assert_eq!(conn.recv().await, Some(json!({"type": "kill", "sessionId": "s1"})));
    killed.await;
    assert_eq!(started.elapsed(), Duration::from_millis(4000));
    // At once for a session that isn't live.
    let started = tokio::time::Instant::now();
    client.kill("nobody").await;
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test]
async fn host_121_123_writes_and_resizes_go_to_live_sessions_only() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    client.write("nobody", "x");
    client.resize("nobody", 80.0, 24.0);
    client.write("s1", "\r");
    client.resize("s1", 10.9, 3.2);
    client.resize("s1", f64::NAN, 24.0);
    client.resize("s1", 132.7, 40.2);
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "write", "sessionId": "s1", "data": "\r"}))
    );
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "resize", "sessionId": "s1", "cols": 20, "rows": 5}))
    );
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "resize", "sessionId": "s1", "cols": 132, "rows": 40}))
    );
    // The keystroke rules saw it first.
    assert_eq!(setup.record.lines(), vec!["started s1 done", "input s1 \"\\r\" None"]);
}

#[tokio::test(start_paused = true)]
async fn host_122_a_prompt_is_pasted_then_entered_200_ms_later() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    let started = tokio::time::Instant::now();
    client.send_prompt("s1", "fix it");
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "write", "sessionId": "s1", "data": "\u{1b}[200~fix it\u{1b}[201~"}))
    );
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "write", "sessionId": "s1", "data": "\r"}))
    );
    assert!(started.elapsed() >= Duration::from_millis(200));
}

#[tokio::test]
async fn host_125_kills_every_session_of_a_project() {
    let setup = Setup::new();
    let sessions = json!([session("a1", "a", ""), session("b1", "b", ""), session("a2", "a", "")]);
    let (client, mut conn) = adopted(&setup, sessions).await;
    client.kill_project("a");
    assert_eq!(conn.recv().await, Some(json!({"type": "kill", "sessionId": "a1"})));
    assert_eq!(conn.recv().await, Some(json!({"type": "kill", "sessionId": "a2"})));
}

#[tokio::test]
async fn host_126_answers_the_board_tools_requests() {
    let setup = Setup::new();
    let (_client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    conn.send(&[
        json!({"type": "mcp", "id": 1, "sessionId": "s1", "message": {"jsonrpc": "2.0", "id": 7, "method": "tools/list"}}),
        json!({"type": "mcp", "id": 2, "sessionId": "s1", "message": {"jsonrpc": "2.0", "id": 8, "method": "fail"}}),
        json!({"type": "mcp", "id": 3, "sessionId": "s1", "message": {"jsonrpc": "2.0", "id": 9, "method": "none"}}),
    ])
    .await;
    let mut replies = vec![
        conn.recv().await.unwrap(),
        conn.recv().await.unwrap(),
        conn.recv().await.unwrap(),
    ];
    replies.sort_by_key(|reply| reply["id"].as_u64());
    assert_eq!(
        replies,
        vec![
            json!({"type": "mcp-reply", "id": 1, "reply": {"jsonrpc": "2.0", "id": 7, "result": {"ok": true}}}),
            json!({"type": "mcp-reply", "id": 2}),
            json!({"type": "mcp-reply", "id": 3}),
        ]
    );
}

#[tokio::test]
async fn host_101_hooks_reach_the_status_machine_as_they_come() {
    let setup = Setup::new();
    let (_client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    conn.send(&[json!({"type": "hook", "sessionId": "s1", "event": "Stop", "payload": {"x": 1}})])
        .await;
    setup.record.wait_for("hook s1 Stop {\"x\":1} None").await;
}

#[tokio::test]
async fn host_109_a_lost_host_ends_every_session_and_fails_pending_starts() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    let starting = tokio::spawn({
        let client = client.clone();
        async move { client.spawn_session(&opts("s2"), shell("true")).await }
    });
    assert_eq!(conn.recv().await.unwrap()["type"], "spawn");
    drop(conn);
    assert_eq!(
        starting.await.unwrap(),
        Err(ClientError::Failed("The session host stopped".into()))
    );
    setup.record.wait_for("exit s1 -1").await;
    assert_eq!(setup.record.lines(), vec!["started s1 done", "ended s1", "exit s1 -1"]);
    // HOST-110: no reconnecting by itself; HOST-127: sends go nowhere.
    client.write("s1", "x");
    assert!(client.live_session_ids().is_empty());
}

#[tokio::test]
async fn host_111_quitting_leaves_the_sessions_running() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    client.disconnect();
    assert_eq!(conn.recv().await, None);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(setup.record.lines(), vec!["started s1 done"]);
}

#[tokio::test]
async fn host_112_stop_all_ends_what_the_host_didnt_report() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", ""), session("s2", "p1", "")])).await;
    let stopping = tokio::spawn({
        let client = client.clone();
        async move { client.stop_all().await }
    });
    assert_eq!(conn.recv().await, Some(json!({"type": "shutdown"})));
    conn.send(&[json!({"type": "exit", "sessionId": "s1", "exitCode": 0})])
        .await;
    setup.record.wait_for("exit s1 0").await;
    drop(conn);
    stopping.await.unwrap();
    assert_eq!(
        setup.record.lines(),
        vec![
            "started s1 done",
            "started s2 done",
            "ended s1",
            "exit s1 0",
            "ended s2",
            "exit s2 -1"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn host_112_stop_all_gives_up_after_5_seconds_and_keeps_the_sessions() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    let started = tokio::time::Instant::now();
    let stopping = tokio::spawn({
        let client = client.clone();
        async move { client.stop_all().await }
    });
    assert_eq!(conn.recv().await, Some(json!({"type": "shutdown"})));
    stopping.await.unwrap();
    assert_eq!(started.elapsed(), Duration::from_millis(5000));
    assert_eq!(client.live_session_ids(), vec!["s1"]);
}

// ---------------------------------------------------------------------------
// The Rust host

fn in_process(setup: &Setup) -> (HostClient, Arc<InProcess>) {
    let launcher = Arc::new(InProcess::default());
    (setup.client(Arc::clone(&launcher) as Arc<dyn HostLauncher>), launcher)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_093_concurrent_connects_start_one_host() {
    let setup = Setup::new();
    let (client, launcher) = in_process(&setup);
    // Polled in order, so the first asks first; the second gets its attempt.
    let (a, b) = tokio::join!(client.connect_host(true), client.connect_host(false));
    a.unwrap();
    b.unwrap();
    assert_eq!(launcher.started.load(Ordering::SeqCst), 1);
    let deadline = Instant::now() + TIMEOUT;
    while !launcher.logged().contains(&"app connected ".to_owned()) {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(launcher.logged().iter().filter(|l| *l == "app connected ").count(), 1);
    client.stop_all().await;
}

// On one thread, so the first attempt can't end before the second caller
// asks: with no host, connecting fails at once on Linux, and on two threads
// the first attempt could be over before the second asked.
#[tokio::test]
async fn host_093_the_first_callers_attempt_is_everyones() {
    // Whatever `start` the second caller passed: one that doesn't start a
    // host, asked first, is the attempt everyone gets (spec 02, Edge cases).
    let setup = Setup::new();
    let (client, launcher) = in_process(&setup);
    let (a, b) = tokio::join!(client.connect_host(false), client.connect_host(true));
    a.unwrap();
    b.unwrap();
    assert_eq!(launcher.started.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_runs_in_the_rust_host_and_a_relaunched_app_adopts_it() {
    let setup = Setup::new();
    let (client, launcher) = in_process(&setup);
    client
        .spawn_session(
            &opts("s1"),
            shell("printf 'ready\\n'; read line; printf 'got %s\\n' \"$line\"; sleep 30"),
        )
        .await
        .unwrap();
    assert_eq!(launcher.started.load(Ordering::SeqCst), 1);
    assert!(launcher.logged().contains(&"spawned s1 /bin/sh".to_owned()));
    let wait_for_output = async |client: &HostClient, text: &str| {
        let deadline = Instant::now() + TIMEOUT;
        while !client.snapshot("s1").contains(text) {
            assert!(Instant::now() < deadline, "no {text:?} in {:?}", client.snapshot("s1"));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    wait_for_output(&client, "ready\r\n").await;
    client.write("s1", "1");
    client.write("s1", "\r");
    wait_for_output(&client, "got 1\r\n").await;

    // The app quits; the session goes on in the host.
    client.disconnect();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let relaunched = Setup {
        record: Arc::default(),
        ..Setup::new()
    };
    let relaunched_client = HostClient::new(
        &setup.user_data,
        Wiring {
            events: Arc::new(Sink(Arc::clone(&relaunched.record))),
            status: Arc::new(Status(Arc::clone(&relaunched.record))),
            endpoint: Arc::new(HookEndpoint::new()),
            board_tools: Arc::new(Tools),
            prompt: Arc::clone(&relaunched.prompt) as Arc<dyn HostPrompt>,
            launcher: Arc::new(Nothing),
            server: None,
        },
    );
    relaunched_client.connect_host(false).await.unwrap();
    assert_eq!(relaunched_client.live_session_ids(), vec!["s1"]);
    assert_eq!(relaunched_client.snapshot("s1"), "ready\r\n1\r\ngot 1\r\n");
    let lines = relaunched.record.lines();
    assert_eq!(lines[0], "started s1 done");
    // The keystrokes the host recorded, replayed with their times.
    assert!(
        lines[1].starts_with("input s1 \"1\" Some(") && lines[2].starts_with("input s1 \"\\r\" Some("),
        "{lines:?}"
    );

    // Stopping everything ends it.
    relaunched_client.stop_all().await;
    relaunched.record.wait_for("exit s1 0").await;
    assert!(launcher.logged().contains(&"idle, exiting".to_owned()));
}

// ---------------------------------------------------------------------------
// The 0.9.4 Node host

/// U1 against the real 0.9.4 Node host, started as 0.9.4 starts it, with a
/// session a 0.9.4 app started on it: the Rust app asks, Quit leaves the host
/// and its session running, Stop Sessions shuts them down, and nothing is
/// adopted. Runs when the Electron app has been built from a checkout of tag
/// `v0.9.4` (how: nodus-host's `tests/wire.rs`):
///
/// ```sh
/// NODUS_NODE_HOST_JS=../nodus-0.9.4/out/main/host.js \
/// NODUS_NODE_BIN=../nodus-0.9.4/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron \
///   cargo test -p nodus-sessions --test client node_host
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn node_host_a_running_0_9_4_host_is_offered_to_be_stopped() {
    let (Some(host_js), Some(bin)) = (
        std::env::var_os("NODUS_NODE_HOST_JS"),
        std::env::var_os("NODUS_NODE_BIN"),
    ) else {
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let resolve = |path: std::ffi::OsString| {
        let path = PathBuf::from(path);
        if path.is_absolute() { path } else { root.join(path) }
    };
    let node = ProcessLauncher::new(resolve(bin), [resolve(host_js)], [("ELECTRON_RUN_AS_NODE", "1")]);
    let setup = Setup::new().with_choice(ReplaceChoice::Quit);
    let paths = setup.paths();
    node.start(&paths).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    let open = || async {
        loop {
            if let Ok(stream) = UnixStream::connect(&paths.socket).await {
                break FakeConn {
                    stream,
                    reader: MessageReader::new(),
                    queue: VecDeque::new(),
                };
            }
            assert!(Instant::now() < deadline, "the Node host didn't start");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };

    // A 0.9.4 app starts a session, then quits for the update.
    let mut app = open().await;
    app.send(&[json!({"type": "hello", "protocol": PROTOCOL_0_9_4})]).await;
    let welcome = app.recv().await.unwrap();
    assert_eq!((&welcome["type"], &welcome["protocol"]), (&json!("welcome"), &json!(1)));
    app.send(&[
        json!({"type": "spawn", "id": 1, "sessionId": "s1", "projectId": "p1", "initial": "done",
                      "file": "/bin/sleep", "args": ["30"], "env": {"PATH": "/usr/bin:/bin"}, "cwd": "/tmp",
                      "cols": 80, "rows": 24}),
    ])
    .await;
    assert_eq!(app.recv().await, Some(json!({"type": "spawned", "id": 1})));
    drop(app);

    // Quit: the Rust app goes, the host and its session stay.
    let quitting = setup.client(Arc::new(Nothing));
    assert_eq!(quitting.connect_host(false).await, Err(ClientError::Quit));
    assert!(quitting.live_session_ids().is_empty());
    let mut probe = open().await;
    probe.send(&[json!({"type": "hello", "protocol": 0})]).await;
    assert_eq!(probe.recv().await.unwrap()["sessions"][0]["sessionId"], "s1");
    drop(probe);

    // Stop Sessions: the host stops the session and leaves; nothing is adopted.
    let again = Setup {
        user_data: setup.user_data.clone(),
        ..Setup::new()
    };
    let client = again.client(Arc::new(Nothing));
    client.connect_host(false).await.unwrap();
    assert_eq!(again.prompt.asked.load(Ordering::SeqCst), 1);
    assert!(client.live_session_ids().is_empty());
    assert!(again.record.lines().is_empty(), "{:?}", again.record.lines());
    while UnixStream::connect(&paths.socket).await.is_ok() {
        assert!(Instant::now() < deadline, "the Node host is still there");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let log = std::fs::read_to_string(&paths.log).unwrap();
    assert!(log.contains("stopping every session"), "{log}");
}

// ---------------------------------------------------------------------------
// The prompt queue (QUEUE-001–012)

fn queued(id: &str, text: &str) -> QueuedPrompt {
    QueuedPrompt {
        id: id.into(),
        text: text.into(),
        at: 1_762_600_000_000.0,
    }
}

/// As the wire reads it.
fn queued_json(id: &str, text: &str) -> Value {
    json!({"id": id, "text": text, "at": 1_762_600_000_000u64})
}

/// A client adopted by a fake host that keeps queues, with these sessions.
async fn adopted_keeping_queues(setup: &Setup, sessions: Value) -> (HostClient, FakeConn) {
    let host = setup.fake_host();
    let client = setup.client(Arc::new(Nothing));
    let connecting = tokio::spawn({
        let client = client.clone();
        async move { client.connect_host(false).await }
    });
    let mut conn = host.accept().await;
    let mut welcome = welcome(sessions);
    welcome["queue"] = json!(true);
    conn.greet(welcome).await;
    connecting.await.unwrap().unwrap();
    (client, conn)
}

fn paste(text: &str) -> Value {
    json!({"type": "write", "sessionId": "s1", "data": format!("\u{1b}[200~{text}\u{1b}[201~")})
}

fn enter() -> Value {
    json!({"type": "write", "sessionId": "s1", "data": "\r"})
}

fn stop(payload: &Value) -> Value {
    json!({"type": "hook", "sessionId": "s1", "event": "Stop", "payload": payload})
}

#[tokio::test]
async fn queue_001_004_009_adopts_each_sessions_queue_from_a_host_that_keeps_them() {
    let setup = Setup::new();
    let mut s1 = session("s1", "p1", "");
    s1["history"] = json!([{"hook": "UserPromptSubmit", "payload": {}, "at": 1000}]);
    s1["queue"] = json!([queued_json("q1", "one"), queued_json("q2", "two")]);
    let (client, _conn) = adopted_keeping_queues(&setup, json!([s1, session("s2", "p1", "")])).await;
    assert!(client.host_keeps_queues());
    assert_eq!(client.queue("s1"), vec![queued("q1", "one"), queued("q2", "two")]);
    assert_eq!(client.queue("s2"), vec![]);
    // After its status is brought up to date; none for a session without one.
    assert_eq!(
        setup.record.lines(),
        vec![
            "started s1 done",
            "hook s1 UserPromptSubmit {} Some(1000.0)",
            "queue s1 [q1,q2]",
            "started s2 done",
        ]
    );
}

#[tokio::test]
async fn queue_001_an_older_host_doesnt_keep_queues() {
    let setup = Setup::new();
    let (client, _conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    assert!(!client.host_keeps_queues());
}

#[tokio::test]
async fn queue_009_010_sets_a_queue_here_and_in_a_host_that_keeps_them() {
    let setup = Setup::new();
    let (client, mut conn) = adopted_keeping_queues(&setup, json!([session("s1", "p1", "")])).await;
    assert!(client.set_queue("s1", vec![queued("q1", "one"), queued("q2", "two")]));
    // At once here, so another can be added straight after.
    assert_eq!(client.queue("s1"), vec![queued("q1", "one"), queued("q2", "two")]);
    assert_eq!(
        conn.recv().await,
        Some(
            json!({"type": "queue-set", "sessionId": "s1", "items": [queued_json("q1", "one"), queued_json("q2", "two")]})
        )
    );
    // The host's word on it: the same is no change; then it handed q1 over.
    conn.send(&[
        json!({"type": "queue", "sessionId": "s1", "items": [queued_json("q1", "one"), queued_json("q2", "two")]}),
        json!({"type": "queue", "sessionId": "s1", "items": [queued_json("q2", "two")]}),
        json!({"type": "queue", "sessionId": "nobody", "items": [queued_json("q9", "x")]}),
    ])
    .await;
    setup.record.wait_for("queue s1 [q2]").await;
    assert_eq!(client.queue("s1"), vec![queued("q2", "two")]);
    assert_eq!(
        setup.record.lines(),
        vec!["started s1 done", "queue s1 [q1,q2]", "queue s1 [q2]"]
    );
    // Not for a session that isn't live; nothing goes to the host.
    assert!(!client.set_queue("nobody", vec![queued("q3", "three")]));
    assert_eq!(client.queue("nobody"), vec![]);
    assert!(client.set_queue("s1", vec![]));
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "queue-set", "sessionId": "s1", "items": []}))
    );
    setup.record.wait_for("queue s1 []").await;
}

#[tokio::test(start_paused = true)]
async fn queue_011_without_a_host_that_keeps_queues_the_first_is_typed_when_the_turn_ends() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    assert!(client.set_queue("s1", vec![queued("q1", "fix it"), queued("q2", "then this")]));
    setup.record.wait_for("queue s1 [q1,q2]").await;
    // A subagent's stop isn't the turn's end.
    conn.send(&[stop(&json!({"agent_id": "a1"})), stop(&json!({}))]).await;
    // Nothing went to the host before the paste: it doesn't know queue-set.
    assert_eq!(conn.recv().await, Some(paste("fix it")));
    assert_eq!(conn.recv().await, Some(enter()));
    assert_eq!(client.queue("s1"), vec![queued("q2", "then this")]);
    assert_eq!(
        setup.record.lines(),
        vec![
            "started s1 done",
            "queue s1 [q1,q2]",
            r#"hook s1 Stop {"agent_id":"a1"} None"#,
            "hook s1 Stop {} None",
            "queue s1 [q2]",
            r#"input s1 "\u{1b}[200~fix it\u{1b}[201~" None"#,
            r#"input s1 "\r" None"#,
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn queue_011_nothing_is_typed_while_the_session_waits_on_the_user() {
    let setup = Setup::new();
    let (client, mut conn) = adopted(&setup, json!([session("s1", "p1", "")])).await;
    client.set_queue("s1", vec![queued("q1", "fix it")]);
    setup.record.set_waiting("s1", true);
    conn.send(&[stop(&json!({}))]).await;
    setup.record.wait_for("hook s1 Stop {} None").await;
    assert_eq!(client.queue("s1"), vec![queued("q1", "fix it")]);
    // Its next turn's end, once answered: the first thing written is the paste.
    setup.record.set_waiting("s1", false);
    conn.send(&[stop(&json!({}))]).await;
    assert_eq!(conn.recv().await, Some(paste("fix it")));
    assert_eq!(conn.recv().await, Some(enter()));
    assert_eq!(client.queue("s1"), vec![]);
}

#[tokio::test(start_paused = true)]
async fn queue_011_with_a_host_that_keeps_queues_what_it_didnt_hand_over_is_typed() {
    let setup = Setup::new();
    let mut s1 = session("s1", "p1", "");
    s1["queue"] = json!([queued_json("q1", "fix it"), queued_json("q2", "then this")]);
    let (client, mut conn) = adopted_keeping_queues(&setup, json!([s1])).await;
    // A Stop reaching the app means the host didn't hand one over (a hook
    // from an older app's process, or the queue changed on the way).
    conn.send(&[stop(&json!({}))]).await;
    assert_eq!(
        conn.recv().await,
        Some(json!({"type": "queue-set", "sessionId": "s1", "items": [queued_json("q2", "then this")]}))
    );
    assert_eq!(conn.recv().await, Some(paste("fix it")));
    assert_eq!(conn.recv().await, Some(enter()));
    assert_eq!(client.queue("s1"), vec![queued("q2", "then this")]);
    setup.record.wait_for("queue s1 [q2]").await;
}

#[tokio::test]
async fn queue_012_a_session_that_ends_takes_its_queue_with_it() {
    let setup = Setup::new();
    let mut s1 = session("s1", "p1", "");
    s1["queue"] = json!([queued_json("q1", "one")]);
    let (client, mut conn) = adopted_keeping_queues(&setup, json!([s1, session("s2", "p1", "")])).await;
    conn.send(&[
        json!({"type": "exit", "sessionId": "s1", "exitCode": 0}),
        json!({"type": "exit", "sessionId": "s2", "exitCode": 0}),
    ])
    .await;
    setup.record.wait_for("exit s2 0").await;
    assert_eq!(
        setup.record.lines(),
        vec![
            "started s1 done",
            "queue s1 [q1]",
            "started s2 done",
            "ended s1",
            "exit s1 0",
            "queue s1 []",
            "ended s2",
            "exit s2 0",
        ]
    );
    assert_eq!(client.queue("s1"), vec![]);
    assert!(!client.set_queue("s1", vec![queued("q2", "two")]));
}

/// QUEUE-005, QUEUE-006, QUEUE-008 end to end: the app queues a prompt in
/// the Rust host, and the session's Stop hook command, run as Claude Code
/// runs it, prints the host's answer handing it over.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_005_008_the_stop_hook_command_gets_the_queued_prompt_from_the_rust_host() {
    let setup = Setup::new();
    let (client, _launcher) = in_process(&setup);
    let go = setup.user_data.with_file_name("go");
    let endpoint = Arc::clone(&setup.endpoint);
    let go_path = go.to_string_lossy().into_owned();
    let command = move |opts: &SpawnOptions, _: HostInfo| {
        let settings: Value = serde_json::from_str(&endpoint.settings(false)).unwrap();
        let stop = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut env = endpoint.env(&opts.session_id);
        env.insert("PATH".into(), "/usr/bin:/bin".into());
        env.insert("GO".into(), go_path);
        env.insert("STOP".into(), stop);
        Ok(Command {
            file: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                r#"while [ ! -e "$GO" ]; do sleep 0.05; done; printf '{"session_id":"s1"}' | sh -c "$STOP"; printf '\n--done--\n'; sleep 30"#.into(),
            ],
            env,
        })
    };
    client.spawn_session(&opts("s1"), command).await.unwrap();
    assert!(client.host_keeps_queues());
    assert!(client.set_queue("s1", vec![queued("q1", "héllo there")]));
    // The queue-set reaches the host before the hook does.
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(&go, "").unwrap();

    let deadline = Instant::now() + TIMEOUT;
    while !client.snapshot("s1").contains("--done--") {
        assert!(Instant::now() < deadline, "{:?}", client.snapshot("s1"));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let printed = client.snapshot("s1");
    let printed = printed.lines().next().unwrap().trim_end();
    assert_eq!(
        serde_json::from_str::<Value>(printed).unwrap(),
        json!({"decision": "block", "reason": nodus_domain::queued_reason("héllo there")})
    );
    setup.record.wait_for("queue s1 []").await;
    let lines = setup.record.lines();
    assert!(
        lines.contains(&r#"hook s1 UserPromptSubmit {"prompt":"héllo there","nodusQueued":true} None"#.to_owned()),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|line| line.starts_with("hook s1 Stop")), "{lines:?}");
    client.stop_all().await;
}
