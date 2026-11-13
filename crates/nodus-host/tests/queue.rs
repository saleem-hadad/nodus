//! The prompt queue (QUEUE-001–007): the app sets a session's queued
//! prompts, and the host hands the first to Claude by answering the Stop
//! hook of a process whose hook command prints the answer.
#![cfg(unix)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use nodus_domain::{QUEUED_PROMPT_LEAD, queued_reason};
use nodus_host::Logger;
use nodus_host::transport::{ClientStream, connect};
use nodus_host_proto::{MessageReader, PROTOCOL};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(15);

struct TestHost {
    _dir: tempfile::TempDir,
    socket: PathBuf,
}

impl TestHost {
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-queue-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("host.sock");
        let args = [
            socket.to_string_lossy().into_owned(),
            socket.with_extension("key").to_string_lossy().into_owned(),
        ];
        let (log, _lines) = Logger::memory();
        tokio::spawn(async move { nodus_host::run(&args, log).await });
        for _ in 0..100 {
            if connect(&socket).await.is_ok() {
                return Self { _dir: dir, socket };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the host didn't start");
    }
}

struct Conn {
    stream: ClientStream,
    reader: MessageReader,
    queue: VecDeque<Value>,
}

impl Conn {
    /// An app: says hello and takes the welcome.
    async fn app(host: &TestHost) -> (Self, Value) {
        let mut conn = Self {
            stream: connect(&host.socket).await.unwrap(),
            reader: MessageReader::new(),
            queue: VecDeque::new(),
        };
        conn.send(json!({"type": "hello", "protocol": PROTOCOL})).await;
        let welcome = conn.recv().await;
        (conn, welcome)
    }

    async fn send(&mut self, msg: Value) {
        let line = nodus_domain::json::to_string(&msg).unwrap() + "\n";
        self.stream.write_all(line.as_bytes()).await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            if let Some(msg) = self.queue.pop_front() {
                return msg;
            }
            let n = tokio::time::timeout(TIMEOUT, self.stream.read(&mut buffer))
                .await
                .expect("a message")
                .unwrap();
            assert!(n > 0, "the host closed the connection");
            self.queue.extend(self.reader.feed(&buffer[..n]));
        }
    }

    /// The next message that isn't output.
    async fn recv_event(&mut self) -> Value {
        loop {
            let msg = self.recv().await;
            if msg["type"] != "data" {
                return msg;
            }
        }
    }

    /// Nothing more comes for a while.
    async fn quiet(&mut self) {
        let mut buffer = [0u8; 256];
        assert!(self.queue.is_empty(), "{:?}", self.queue);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), self.stream.read(&mut buffer))
                .await
                .is_err(),
            "the app heard something"
        );
    }
}

/// An app with one live session, `s1`, and the hook port and token.
async fn app_with_a_session(host: &TestHost) -> (Conn, u16, String) {
    let (mut app, welcome) = Conn::app(host).await;
    app.send(
        json!({"type": "spawn", "id": 1, "sessionId": "s1", "projectId": "p", "initial": "working",
                    "file": "/bin/sleep", "args": ["30"], "env": {"PATH": "/usr/bin:/bin"}, "cwd": "/tmp",
                    "cols": 80, "rows": 24}),
    )
    .await;
    assert_eq!(app.recv_event().await, json!({"type": "spawned", "id": 1}));
    let port = u16::try_from(welcome["hookPort"].as_u64().unwrap()).unwrap();
    (app, port, welcome["hookToken"].as_str().unwrap().to_owned())
}

/// What the hook server answered: status, content type and body.
#[derive(Debug, PartialEq)]
struct Answer {
    status: u16,
    content_type: Option<String>,
    body: String,
}

async fn post_hook(port: u16, token: &str, query: &str, body: &str) -> Answer {
    let request = format!(
        "POST /hook?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(TIMEOUT, stream.read_to_string(&mut raw))
        .await
        .expect("an answer")
        .unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let content_type = lines
        .filter_map(|line| line.split_once(": "))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.to_owned());
    Answer {
        status,
        content_type,
        body: body.to_owned(),
    }
}

fn no_content() -> Answer {
    Answer {
        status: 204,
        content_type: None,
        body: String::new(),
    }
}

fn item(id: &str, text: &str) -> Value {
    json!({"id": id, "text": text, "at": 1_762_600_000_000u64})
}

fn queue_set(items: &[Value]) -> Value {
    json!({"type": "queue-set", "sessionId": "s1", "items": items})
}

fn queue(items: &[Value]) -> Value {
    json!({"type": "queue", "sessionId": "s1", "items": items})
}

#[tokio::test]
async fn queue_001_a_welcome_says_the_host_keeps_queues() {
    let host = TestHost::start().await;
    let (_app, welcome) = Conn::app(&host).await;
    assert_eq!(welcome["queue"], json!(true));
    // Between hookAuthHeader and a Windows key, as Welcome's fields go.
    let keys: Vec<&String> = welcome.as_object().unwrap().keys().collect();
    assert_eq!(keys[keys.len() - 2..], ["hookAuthHeader", "queue"]);
}

#[tokio::test]
async fn queue_002_005_006_a_stop_that_asks_for_a_reply_gets_the_first_queued_prompt() {
    let host = TestHost::start().await;
    let (mut app, port, token) = app_with_a_session(&host).await;
    let (mut other, _) = Conn::app(&host).await;

    // QUEUE-002: set, and every app hears it.
    let (first, second) = (item("q1", "run the tests, then commit"), item("q2", "héllo 🎉"));
    app.send(queue_set(&[first.clone(), second.clone()])).await;
    assert_eq!(app.recv_event().await, queue(&[first.clone(), second.clone()]));
    assert_eq!(other.recv_event().await, queue(&[first.clone(), second.clone()]));

    // QUEUE-005: the first goes to Claude as the Stop hook's answer.
    let answer = post_hook(
        port,
        &token,
        "evt=Stop&sid=s1&reply=1",
        r#"{"session_id":"s1","hook_event_name":"Stop","stop_hook_active":false}"#,
    )
    .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.content_type.as_deref(), Some("application/json"));
    let decision: Value = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(
        decision,
        json!({"decision": "block", "reason": queued_reason("run the tests, then commit")})
    );
    assert!(decision["reason"].as_str().unwrap().starts_with(QUEUED_PROMPT_LEAD));

    // QUEUE-006: every app hears a prompt, not a stop, then the queue without it.
    for conn in [&mut app, &mut other] {
        assert_eq!(
            conn.recv_event().await,
            json!({"type": "hook", "sessionId": "s1", "event": "UserPromptSubmit",
                   "payload": {"prompt": "run the tests, then commit", "nodusQueued": true}})
        );
        assert_eq!(conn.recv_event().await, queue(std::slice::from_ref(&second)));
    }

    // The next one, with characters beyond ASCII, which come escaped.
    let answer = post_hook(port, &token, "evt=Stop&sid=s1&reply=1", "{}").await;
    assert!(answer.body.is_ascii(), "{}", answer.body);
    assert!(answer.body.contains(r"h\u00e9llo \ud83c\udf89"), "{}", answer.body);
    let decision: Value = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(decision["reason"], json!(queued_reason("héllo 🎉")));
    assert_eq!(app.recv_event().await["event"], "UserPromptSubmit");
    assert_eq!(app.recv_event().await, queue(&[]));

    // QUEUE-007: with the queue empty, the Stop is a Stop.
    let answer = post_hook(port, &token, "evt=Stop&sid=s1&reply=1", "{}").await;
    assert_eq!(answer, no_content());
    assert_eq!(
        app.recv_event().await,
        json!({"type": "hook", "sessionId": "s1", "event": "Stop", "payload": {}})
    );
    app.quiet().await;

    // QUEUE-004, QUEUE-006: a relaunched app replays prompts, not stops.
    let (_relaunched, welcome) = Conn::app(&host).await;
    let history: Vec<(&str, &Value)> = welcome["sessions"][0]["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (entry["hook"].as_str().unwrap(), &entry["payload"]))
        .collect();
    assert_eq!(
        history,
        vec![
            (
                "UserPromptSubmit",
                &json!({"prompt": "run the tests, then commit", "nodusQueued": true})
            ),
            ("UserPromptSubmit", &json!({"prompt": "héllo 🎉", "nodusQueued": true})),
            ("Stop", &json!({})),
        ]
    );
    assert!(welcome["sessions"][0].get("queue").is_none());
    app.send(json!({"type": "shutdown"})).await;
}

#[tokio::test]
async fn queue_007_a_stop_that_doesnt_ask_for_a_reply_never_gets_one() {
    let host = TestHost::start().await;
    let (mut app, port, token) = app_with_a_session(&host).await;
    app.send(queue_set(&[item("q1", "next")])).await;
    app.recv_event().await;

    // From a process an older app started: its hook throws the answer away.
    let answer = post_hook(port, &token, "evt=Stop&sid=s1", "{}").await;
    assert_eq!(answer, no_content());
    assert_eq!(app.recv_event().await["event"], "Stop");
    // Only Stop hands one over.
    let answer = post_hook(port, &token, "evt=Notification&sid=s1&reply=1", "{}").await;
    assert_eq!(answer, no_content());
    assert_eq!(app.recv_event().await["event"], "Notification");
    // A subagent's turn isn't the user's.
    let answer = post_hook(port, &token, "evt=Stop&sid=s1&reply=1", r#"{"agent_id":"a1"}"#).await;
    assert_eq!(answer, no_content());
    assert_eq!(app.recv_event().await["event"], "Stop");
    app.quiet().await;

    // It's all still there.
    let (_relaunched, welcome) = Conn::app(&host).await;
    assert_eq!(welcome["sessions"][0]["queue"], json!([item("q1", "next")]));
    app.send(json!({"type": "shutdown"})).await;
}

#[tokio::test]
async fn queue_003_a_queue_set_that_crossed_a_hand_over_cant_put_it_back() {
    let host = TestHost::start().await;
    let (mut app, port, token) = app_with_a_session(&host).await;
    let (q1, q2, q3) = (item("q1", "one"), item("q2", "two"), item("q3", "three"));
    app.send(queue_set(&[q1.clone(), q2.clone()])).await;
    app.recv_event().await;
    let answer = post_hook(port, &token, "evt=Stop&sid=s1&reply=1", "{}").await;
    assert_eq!(answer.status, 200);
    assert_eq!(app.recv_event().await["event"], "UserPromptSubmit");
    assert_eq!(app.recv_event().await, queue(std::slice::from_ref(&q2)));

    // The app added one before it heard q1 went.
    app.send(queue_set(&[q1.clone(), q2.clone(), q3.clone()])).await;
    assert_eq!(app.recv_event().await, queue(&[q2.clone(), q3.clone()]));
    // QUEUE-004: and a relaunched app sees that.
    let (_relaunched, welcome) = Conn::app(&host).await;
    assert_eq!(welcome["sessions"][0]["queue"], json!([q2, q3]));
    app.send(json!({"type": "shutdown"})).await;
}

#[tokio::test]
async fn queue_002_a_queue_set_for_no_live_session_or_that_doesnt_read_is_ignored() {
    let host = TestHost::start().await;
    let (mut app, _port, _token) = app_with_a_session(&host).await;
    app.send(json!({"type": "queue-set", "sessionId": "nobody", "items": [item("q1", "x")]}))
        .await;
    app.send(json!({"type": "queue-set", "sessionId": "s1", "items": [{"id": "q1"}]}))
        .await;
    app.send(json!({"type": "queue-set", "sessionId": "s1"})).await;
    app.quiet().await;
    // Emptying it is a change like any other.
    app.send(queue_set(&[])).await;
    assert_eq!(app.recv_event().await, queue(&[]));
    app.send(json!({"type": "shutdown"})).await;
}
