//! Wire compatibility with the 0.9.4 session host (plan rule 4).
//!
//! One scripted conversation — hello and welcome, spawning, output, input,
//! resizing, hooks and the status line, the board tools' relay, a protocol-0
//! probe, kill and shutdown — runs against a host, and everything the host
//! says (on the socket, over HTTP and in its log) is written down with the
//! parts that change between runs (port, token, times) replaced. The Rust
//! host has to produce exactly `fixtures/wire-transcript.txt`, but for the
//! protocol number in its welcomes: 2 where 0.9.4 says 1 (decision U1). The
//! apps in the conversation say the number of the host they talk to.
//!
//! The fixture was recorded from the real 0.9.4 Node host. The Electron app
//! is no longer in this repo; to check the fixture against Node again, build
//! it from a checkout of tag `v0.9.4` beside this one (Node 22+, pnpm) and
//! point the test at it (relative paths are from this repo's root):
//!
//! ```sh
//! git worktree add ../nodus-0.9.4 v0.9.4
//! (cd ../nodus-0.9.4 && pnpm install && pnpm build)
//! NODUS_NODE_HOST_JS=../nodus-0.9.4/out/main/host.js \
//! NODUS_NODE_BIN=../nodus-0.9.4/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron \
//!   cargo test -p nodus-host --test wire -- --nocapture
//! ```
//!
//! `NODUS_UPDATE_GOLDEN=1` rewrites the fixture from the Node host's run.
#![cfg(unix)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nodus_host::Logger;
use nodus_host::transport::{ClientStream, connect};
use nodus_host_proto::{MessageReader, PROTOCOL, PROTOCOL_0_9_4};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(15);
const FIXTURE: &str = "tests/fixtures/wire-transcript.txt";

/// A connection to the host, as an app or a probe.
struct Conn {
    name: &'static str,
    stream: ClientStream,
    reader: MessageReader,
    queue: VecDeque<Value>,
}

impl Conn {
    async fn open(name: &'static str, socket: &Path) -> Self {
        let stream = connect(socket).await.expect("the host listens");
        Self {
            name,
            stream,
            reader: MessageReader::new(),
            queue: VecDeque::new(),
        }
    }

    async fn send(&mut self, msg: Value) {
        let line = nodus_domain::json::to_string(&msg).unwrap() + "\n";
        self.stream.write_all(line.as_bytes()).await.unwrap();
    }

    /// The next message, or None once the host closed the connection.
    async fn recv(&mut self) -> Option<Value> {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            if let Some(msg) = self.queue.pop_front() {
                return Some(msg);
            }
            let n = tokio::time::timeout(TIMEOUT, self.stream.read(&mut buffer))
                .await
                .unwrap_or_else(|_| panic!("{} heard nothing", self.name))
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.queue.extend(self.reader.feed(&buffer[..n]));
        }
    }

    /// Nothing arrives for a while.
    async fn quiet(&mut self) {
        let mut buffer = vec![0u8; 1024];
        if let Some(msg) = self.queue.pop_front() {
            panic!("{} got {msg}", self.name);
        }
        if let Ok(read) = tokio::time::timeout(Duration::from_millis(300), self.stream.read(&mut buffer)).await {
            let n = read.unwrap_or(0);
            panic!("{} got {:?}", self.name, String::from_utf8_lossy(&buffer[..n]));
        }
    }
}

/// What the host said, in order, with what changes between runs replaced.
#[derive(Default)]
struct Transcript {
    lines: Vec<String>,
    token: String,
    port: u16,
    /// The protocol the host speaks: 0.9.4's, or the Rust host's.
    protocol: u32,
}

impl Transcript {
    fn new(protocol: u32) -> Self {
        Self {
            protocol,
            ..Self::default()
        }
    }

    fn note(&mut self, line: String) {
        self.lines.push(line);
    }

    /// Written down as 0.9.4 would say it: a welcome's protocol must be the
    /// host's, and is written as 0.9.4's number.
    fn got(&mut self, conn: &Conn, msg: &Value) {
        let mut msg = normalize(msg);
        if msg["type"] == "welcome" {
            assert_eq!(msg["protocol"], json!(self.protocol), "{} got {msg}", conn.name);
            msg["protocol"] = json!(PROTOCOL_0_9_4);
            // QUEUE-001: the Rust host also says it keeps queues, which
            // 0.9.4's doesn't; nothing else changes until one is set.
            let queue = msg.as_object_mut().and_then(|fields| fields.shift_remove("queue"));
            let expected = (self.protocol == PROTOCOL).then(|| json!(true));
            assert_eq!(queue, expected, "{} got {msg}", conn.name);
        }
        self.note(format!(
            "{}← {}",
            conn.name,
            nodus_domain::json::to_string(&msg).unwrap()
        ));
    }

    /// The next message, which must be of this type.
    async fn expect(&mut self, conn: &mut Conn, kind: &str) -> Value {
        let msg = conn
            .recv()
            .await
            .unwrap_or_else(|| panic!("{} was closed waiting for {kind}", conn.name));
        assert_eq!(msg["type"], kind, "{} got {msg}", conn.name);
        self.got(conn, &msg);
        msg
    }

    /// Output for a session until it ends with `tail`, written down as one message.
    async fn expect_output(&mut self, conn: &mut Conn, session: &str, tail: &str) {
        let mut output = String::new();
        while !output.ends_with(tail) {
            let msg = conn.recv().await.expect("output");
            assert_eq!(
                (msg["type"].as_str(), msg["sessionId"].as_str()),
                (Some("data"), Some(session)),
                "{msg}"
            );
            output.push_str(msg["data"].as_str().unwrap());
        }
        self.got(conn, &json!({"type": "data", "sessionId": session, "data": output}));
    }

    async fn closed(&mut self, conn: &mut Conn) {
        assert_eq!(conn.recv().await, None, "{} wasn't closed", conn.name);
        self.note(format!("{}← (closed)", conn.name));
    }

    /// An HTTP request to the hook server; the token stands in for `<token>`.
    async fn http(&mut self, method: &str, target: &str, headers: &[(&str, &str)], body: &str) -> String {
        let fill = |text: &str| text.replace("<token>", &self.token);
        let mut request = format!(
            "{method} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
            fill(target),
            self.port
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {}\r\n", fill(value)));
        }
        request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        let _ = tokio::time::timeout(TIMEOUT, stream.read_to_end(&mut raw))
            .await
            .expect("an answer");
        let raw = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let mut lines = head.lines();
        let status = lines.next().unwrap_or("").split(' ').nth(1).unwrap_or("").to_owned();
        let mut shown = Vec::new();
        let mut chunked = false;
        for line in lines {
            let (name, value) = line.split_once(": ").unwrap_or((line, ""));
            match name.to_ascii_lowercase().as_str() {
                "content-type" | "allow" => shown.push(format!("{}={value}", name.to_ascii_lowercase())),
                "transfer-encoding" => chunked = value == "chunked",
                _ => {}
            }
        }
        let body = if chunked { dechunk(body) } else { body.to_owned() };
        self.note(
            format!("HTTP {method} {target} → {status} {} {body}", shown.join(" "))
                .trim_end()
                .to_owned(),
        );
        body
    }
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.push_str(&rest[..size]);
        body = &rest[size + 2..];
    }
    out
}

/// The message with its port, token, key and times replaced.
fn normalize(msg: &Value) -> Value {
    let mut msg = msg.clone();
    if msg["type"] == "welcome" {
        msg["hookPort"] = json!("<port>");
        msg["hookToken"] = json!("<token>");
        if msg.get("key").is_some() {
            msg["key"] = json!("<key>");
        }
        for session in msg["sessions"].as_array_mut().into_iter().flatten() {
            for entry in session["history"].as_array_mut().into_iter().flatten() {
                entry["at"] = json!("<at>");
            }
            if session.get("statusLine").is_some() {
                session["statusLine"]["at"] = json!("<at>");
            }
        }
    }
    msg
}

/// The host's log lines, without times, with the socket and port replaced;
/// only the first line of a multi-line entry (an error's stack).
fn normalize_log(lines: &[String], socket: &Path, port: u16) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.len() > 25 && line.as_bytes()[4] == b'-' && line.as_bytes()[23] == b'Z')
        .map(|line| {
            let line = &line[25..];
            line.replace(&*socket.to_string_lossy(), "<socket>")
                .replace(&format!(" {port}"), " <port>")
        })
        .map(|line| format!("LOG {line}"))
        .collect()
}

/// The conversation with a host on this protocol. Returns the transcript.
async fn converse(socket: &Path, workdir: &Path, protocol: u32) -> Transcript {
    let mut t = Transcript::new(protocol);
    let cwd = workdir.to_string_lossy().into_owned();
    let env = json!({"PATH": "/usr/bin:/bin", "A": "1"});
    let spawn = |id: u32, session: &str, args: Value| {
        json!({"type": "spawn", "id": id, "sessionId": session, "projectId": "p1", "initial": "done",
               "file": "/bin/sh", "args": args, "env": env, "cwd": cwd, "cols": 100, "rows": 30})
    };

    // HOST-036–038: hello, welcome.
    let mut a = Conn::open("A", socket).await;
    a.send(json!({"type": "hello", "protocol": protocol})).await;
    let welcome = t.expect(&mut a, "welcome").await;
    t.token = welcome["hookToken"].as_str().unwrap().to_owned();
    t.port = u16::try_from(welcome["hookPort"].as_u64().unwrap()).unwrap();

    // HOST-049–058: a session that prints, reads a line, and exits 4.
    a.send(spawn(
        1,
        "s1",
        json!([
            "-c",
            "printf 'ready\\n'; read line; printf 'got %s\\n' \"$line\"; exit 4"
        ]),
    ))
    .await;
    t.expect(&mut a, "spawned").await;
    t.expect_output(&mut a, "s1", "ready\r\n").await;
    a.send(json!({"type": "write", "sessionId": "s1", "data": "hello\r"}))
        .await;
    t.expect_output(&mut a, "s1", "got hello\r\n").await;
    t.expect(&mut a, "exit").await;

    // A session that stays, a duplicate spawn, and a spawn node-pty refuses.
    a.send(spawn(
        2,
        "s2",
        json!(["-c", "printf 'live\\n'; read a; read b; exit 0"]),
    ))
    .await;
    t.expect(&mut a, "spawned").await;
    t.expect_output(&mut a, "s2", "live\r\n").await;
    a.send(spawn(3, "s2", json!(["-c", "exit 9"]))).await;
    t.expect(&mut a, "spawned").await;
    a.send(spawn(4, "s3", json!("not an array"))).await;
    t.expect(&mut a, "spawned").await;

    // HOST-059–061: status keys go into history; the rest is ignored.
    a.send(json!({"type": "write", "sessionId": "s2", "data": "1"})).await;
    t.expect_output(&mut a, "s2", "1").await;
    a.send(json!({"type": "write", "sessionId": "s2", "data": "\r"})).await;
    t.expect_output(&mut a, "s2", "\r\n").await;
    a.send(json!({"type": "write", "sessionId": "nobody", "data": "x"}))
        .await;
    a.send(json!({"type": "write", "sessionId": "s2", "data": 5})).await;
    a.send(json!({"type": "resize", "sessionId": "s2", "cols": 120, "rows": 40}))
        .await;
    a.send(json!({"type": "resize", "sessionId": "s2", "cols": 0, "rows": 40}))
        .await;
    a.send(json!({"type": "something-new"})).await;
    a.send(json!({"type": "hello", "protocol": protocol})).await;
    a.quiet().await;

    // HOST-068–078: hooks.
    let bearer = [
        ("Authorization", "Bearer <token>"),
        ("Content-Type", "application/json"),
    ];
    let prompt = "p".repeat(1025);
    let body = format!(
        r#"{{"session_id":"s2","prompt":"{prompt}","n":1,"ok":true,"tool_input":{{"command":"ls"}},"z":null}}"#
    );
    t.http("POST", "/hook?evt=UserPromptSubmit&sid=s2", &bearer, &body)
        .await;
    t.expect(&mut a, "hook").await;
    t.http(
        "POST",
        "/hook?evt=StatusLine&sid=s2&token=<token>",
        &[],
        r#"{"model":{"display_name":"Model"},"cost":{"total_cost_usd":0.5}}"#,
    )
    .await;
    t.expect(&mut a, "hook").await;
    t.http(
        "POST",
        "/hook?evt=Stop&sid=s2",
        &[("Authorization", "Bearer wrong")],
        "{}",
    )
    .await;
    t.http("POST", "/hook?evt=Stop&sid=s2&token=wrong", &[], "{}").await;
    t.http("GET", "/hook?evt=Stop&sid=s2", &bearer, "").await;
    t.http("POST", "/hooks?evt=Stop&sid=s2", &bearer, "{}").await;
    t.http("POST", "/hook?evt=Stop&sid=nobody", &bearer, "{}").await;
    t.http("POST", "/hook?sid=s2", &bearer, "{}").await;
    t.http("POST", "/hook?evt=&sid=s2", &bearer, "{}").await;
    a.quiet().await;
    t.http("POST", "/hook?evt=Notification&sid=s2", &bearer, "not json")
        .await;
    t.expect(&mut a, "hook").await;
    t.http("POST", "/hook?evt=Odd%20One&sid=s2", &bearer, "").await;
    t.expect(&mut a, "hook").await;
    t.http("POST", "/hook?evt=Stop&sid=s2", &bearer, "null").await;
    t.http("POST", "/hook?evt=StatusLine&sid=s2", &bearer, "[1,\"two\"]")
        .await;
    t.expect(&mut a, "hook").await;
    t.http("POST", "/hook?evt=Odd&sid=s2", &bearer, "\"ab\"").await;
    t.expect(&mut a, "hook").await;
    a.quiet().await;

    // HOST-037, HOST-046: a second app gets the session with its history.
    let mut b = Conn::open("B", socket).await;
    b.send(json!({"type": "hello", "protocol": protocol})).await;
    t.expect(&mut b, "welcome").await;

    // HOST-079–089: the board tools' relay.
    let mcp = [("Authorization", "Bearer <token>"), ("X-Nodus-Session", "s2")];
    t.http("POST", "/mcp", &[("X-Nodus-Session", "s2")], "{}").await;
    t.http("POST", "/mcp?token=<token>", &[("X-Nodus-Session", "s2")], "{}")
        .await;
    t.http("POST", "/mcp", &[("Authorization", "Bearer <token>")], "{}")
        .await;
    t.http(
        "POST",
        "/mcp",
        &[("Authorization", "Bearer <token>"), ("X-Nodus-Session", "nobody")],
        "{}",
    )
    .await;
    t.http("DELETE", "/mcp", &mcp, "").await;
    t.http("GET", "/mcp", &mcp, "").await;
    t.http("POST", "/mcp", &mcp, "not json").await;
    t.http(
        "POST",
        "/mcp",
        &mcp,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
    t.http(
        "POST",
        "/mcp",
        &mcp,
        r#"[{"jsonrpc":"2.0","method":"a"},{"jsonrpc":"2.0","id":1}]"#,
    )
    .await;

    let request = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#;
    let answer = tokio::spawn(http_alone(t.port, t.token.clone(), mcp_headers("s2"), request.into()));
    let relayed = t.expect(&mut b, "mcp").await;
    b.send(json!({"type": "mcp-reply", "id": relayed["id"], "reply": {"jsonrpc": "2.0", "id": 7, "result": {"tools": []}}})).await;
    t.note(answer.await.unwrap());

    let batch = r#"[{"jsonrpc":"2.0","method":"n"},{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{}},{"jsonrpc":"2.0","id":"nine","method":"ping"}]"#;
    let answer = tokio::spawn(http_alone(t.port, t.token.clone(), mcp_headers("s2"), batch.into()));
    let first = t.expect(&mut b, "mcp").await;
    let second = t.expect(&mut b, "mcp").await;
    b.send(json!({"type": "mcp-reply", "id": second["id"], "reply": {"jsonrpc": "2.0", "id": "nine", "result": {}}}))
        .await;
    b.send(json!({"type": "mcp-reply", "id": first["id"]})).await;
    t.note(answer.await.unwrap());

    // HOST-044: the newest app goes; what was waiting on it settles unanswered.
    let request = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call"}"#;
    let answer = tokio::spawn(http_alone(t.port, t.token.clone(), mcp_headers("s2"), request.into()));
    t.expect(&mut b, "mcp").await;
    drop(b);
    t.note(answer.await.unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Now the older app gets them; a null reply counts as none.
    let request = r#"{"jsonrpc":"2.0","id":11,"method":"resources/list"}"#;
    let answer = tokio::spawn(http_alone(t.port, t.token.clone(), mcp_headers("s2"), request.into()));
    let relayed = t.expect(&mut a, "mcp").await;
    a.send(json!({"type": "mcp-reply", "id": relayed["id"], "reply": null}))
        .await;
    t.note(answer.await.unwrap());

    // HOST-038, HOST-039: a probe on another protocol gets a welcome, and may only stop the host.
    let mut c = Conn::open("C", socket).await;
    c.send(json!({"type": "hello", "protocol": 0})).await;
    t.expect(&mut c, "welcome").await;
    c.send(json!({"type": "write", "sessionId": "s2", "data": "x\r"})).await;
    c.send(json!({"type": "kill", "sessionId": "s2"})).await;
    c.quiet().await;
    a.quiet().await;

    // HOST-062: kill.
    a.send(json!({"type": "kill", "sessionId": "s2"})).await;
    t.expect(&mut a, "exit").await;
    a.send(json!({"type": "kill", "sessionId": "s2"})).await;
    a.quiet().await;

    // HOST-064, HOST-065: shutdown, from the probe, with an app connected.
    a.send(
        json!({"type": "spawn", "id": 5, "sessionId": "s4", "projectId": "p1", "initial": "working",
                  "file": "/bin/sleep", "args": ["30"], "env": env, "cwd": cwd, "cols": 80, "rows": 24}),
    )
    .await;
    t.expect(&mut a, "spawned").await;
    c.send(json!({"type": "shutdown"})).await;
    t.expect(&mut a, "exit").await;
    t.closed(&mut a).await;
    t.closed(&mut c).await;
    t
}

fn mcp_headers(session: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), "Bearer <token>".into()),
        ("X-Nodus-Session".into(), session.into()),
    ]
}

/// An HTTP request on its own task, while the test answers the relay.
async fn http_alone(port: u16, token: String, headers: Vec<(String, String)>, body: String) -> String {
    let mut t = Transcript {
        token,
        port,
        ..Transcript::default()
    };
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    t.http("POST", "/mcp", &headers, &body).await;
    t.lines.pop().unwrap()
}

fn compare(transcript: &[String]) {
    let golden = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE)).expect("the fixture");
    let golden: Vec<&str> = golden.lines().collect();
    for (i, (got, want)) in transcript.iter().zip(&golden).enumerate() {
        assert_eq!(got, want, "line {}", i + 1);
    }
    assert_eq!(transcript.len(), golden.len(), "{transcript:#?}");
}

/// A scratch folder for the host's socket and the sessions' cwd, short
/// enough that the socket stays in it.
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("nodus-wire-")
        .tempdir_in("/tmp")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rust_host_says_what_the_node_host_says() {
    let dir = scratch();
    let socket = dir.path().join("host.sock");
    let (log, lines) = Logger::memory();
    let args = [
        socket.to_string_lossy().into_owned(),
        dir.path().join("host.key").to_string_lossy().into_owned(),
    ];
    let host = tokio::spawn(async move { nodus_host::run(&args, log).await });
    wait_for_socket(&socket).await;

    let mut transcript = converse(&socket, dir.path(), PROTOCOL).await;
    let code = tokio::time::timeout(TIMEOUT, host)
        .await
        .expect("the host exits")
        .unwrap();
    transcript.note(format!("EXIT {code}"));
    let logged = lines.lock().unwrap().clone();
    let port = transcript.port;
    transcript.lines.extend(normalize_log(&logged, &socket, port));
    compare(&transcript.lines);
}

/// The same conversation with the 0.9.4 Node host, when it's been built.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_node_host_says_what_the_fixture_says() {
    let (Some(host_js), Some(bin)) = (
        std::env::var_os("NODUS_NODE_HOST_JS"),
        std::env::var_os("NODUS_NODE_BIN"),
    ) else {
        // Skipped: NODUS_NODE_HOST_JS and NODUS_NODE_BIN aren't set (see the module docs).
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let resolve = |path: std::ffi::OsString| {
        let path = PathBuf::from(path);
        if path.is_absolute() { path } else { root.join(path) }
    };
    let dir = scratch();
    let socket = dir.path().join("host.sock");
    let log_path = dir.path().join("host.log");
    let log_file = std::fs::File::create(&log_path).unwrap();
    let mut node = tokio::process::Command::new(resolve(bin))
        .arg(resolve(host_js))
        .arg(&socket)
        .arg(dir.path().join("host.key"))
        .env("ELECTRON_RUN_AS_NODE", "1")
        .current_dir(dir.path())
        .stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone().unwrap())
        .stderr(log_file)
        .kill_on_drop(true)
        .spawn()
        .expect("the Node host starts");
    wait_for_socket(&socket).await;

    let mut transcript = converse(&socket, dir.path(), PROTOCOL_0_9_4).await;
    let status = tokio::time::timeout(TIMEOUT, node.wait())
        .await
        .expect("the host exits")
        .unwrap();
    transcript.note(format!("EXIT {}", status.code().unwrap_or(-1)));
    let logged: Vec<String> = std::fs::read_to_string(&log_path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let port = transcript.port;
    transcript.lines.extend(normalize_log(&logged, &socket, port));

    if std::env::var_os("NODUS_UPDATE_GOLDEN").is_some() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        std::fs::write(fixture, transcript.lines.join("\n") + "\n").unwrap();
    }
    compare(&transcript.lines);
}

async fn wait_for_socket(socket: &Path) {
    for _ in 0..100 {
        if let Ok(stream) = connect(socket).await {
            // A probe that never says hello doesn't count.
            drop(stream);
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the host didn't start");
}
