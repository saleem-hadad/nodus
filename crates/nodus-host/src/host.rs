//! The host's state: its sessions, the apps connected to it, and the board
//! tools' requests waiting on an app. Everything that changes it happens
//! under one lock, so messages go out in the order things happened, as they
//! do from the single-threaded Node host.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use nodus_domain::SessionStatus;
use nodus_domain::js::utf16_len;
use nodus_host_proto::{
    HistoryEntry, HostMessage, HostSession, MessageReader, PROTOCOL, ReplayBuffer, StatusLine, Welcome, encode,
    is_status_key,
};
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch};

use crate::log::Logger;
use crate::pty::{Pty, PtyEvent, PtyOptions};
use crate::transport::Duplex;

pub(crate) const HISTORY_LIMIT: usize = 1000;
pub(crate) const KILL_GRACE: Duration = Duration::from_millis(3000);
/// A host nobody connects to, started by an app that then crashed, gives up after this.
pub(crate) const UNUSED: Duration = Duration::from_millis(30_000);
/// The name every session's terminal goes by (its `TERM`).
const TERMINAL_NAME: &str = "xterm-256color";
/// Hook payload strings longer than this (UTF-16 units) stay out of history.
const SLIM_STRING_LIMIT: usize = 1024;

/// `Date.now()`.
pub(crate) fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_millis() as f64)
}

/// A line on its way to a connection; shared between every app it's broadcast to.
type Line = Arc<str>;

struct Session {
    /// Tells this run of a session id apart from a later one.
    generation: u64,
    project_id: String,
    initial: SessionStatus,
    pty: Pty,
    /// Recent output, for replay. (HOST-056)
    output: ReplayBuffer,
    history: VecDeque<HistoryEntry>,
    status_line: Option<StatusLine>,
    kill_timer: Option<tokio::task::JoinHandle<()>>,
}

struct Relay {
    client: u64,
    settle: oneshot::Sender<Option<Value>>,
}

/// A request relayed to an app, waiting for its answer.
pub(crate) struct PendingRelay {
    relay: u64,
    answer: oneshot::Receiver<Option<Value>>,
    /// 20 s from when it was sent.
    deadline: tokio::time::Instant,
}

#[derive(Default)]
struct State {
    sessions: IndexMap<String, Session>,
    /// Connected apps that speak our protocol, oldest first.
    clients: IndexMap<u64, mpsc::UnboundedSender<Line>>,
    relays: HashMap<u64, Relay>,
    next_relay: u64,
    stopping: bool,
}

/// The session host, shared by every task that serves it.
pub(crate) struct Host {
    me: std::sync::Weak<Host>,
    state: Mutex<State>,
    pub(crate) log: Logger,
    pub(crate) hook_token: String,
    /// Shows the app this pipe is ours, on Windows (trustsHost).
    pub(crate) host_key: Option<String>,
    pub(crate) hook_port: AtomicU64,
    next_id: AtomicU64,
    /// Lines handed to a connection and not yet written, so an exiting host
    /// can let them out first, as Node's synchronous socket writes do.
    unwritten: Arc<AtomicUsize>,
    exit: watch::Sender<Option<i32>>,
}

impl Host {
    pub(crate) fn new(log: Logger, hook_token: String, host_key: Option<String>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            state: Mutex::new(State {
                next_relay: 1,
                ..State::default()
            }),
            log,
            hook_token,
            host_key,
            hook_port: AtomicU64::new(0),
            next_id: AtomicU64::new(1),
            unwritten: Arc::new(AtomicUsize::new(0)),
            exit: watch::channel(None).0,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------
    // Exiting

    /// Asks the host to exit with `code`; the first ask wins.
    pub(crate) fn exit(&self, code: i32) {
        self.exit.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(code);
            true
        });
    }

    /// Resolves with the exit code once the host has decided to exit, after
    /// what it had to say has been written (up to half a second).
    pub(crate) async fn exited(&self) -> i32 {
        let mut exit = self.exit.subscribe();
        let code = loop {
            if let Some(code) = *exit.borrow_and_update() {
                break code;
            }
            if exit.changed().await.is_err() {
                break 0;
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        while self.unwritten.load(Ordering::SeqCst) > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        code
    }

    /// Lets go of every session's terminal and every app, once the host has
    /// exited (as the process going would).
    pub(crate) fn close_sessions(&self) {
        let mut state = self.lock();
        for (_, mut s) in state.sessions.drain(..) {
            if let Some(timer) = s.kill_timer.take() {
                timer.abort();
            }
        }
        state.clients.clear();
        state.relays.clear();
    }

    /// The idle rule: no sessions, and no app or stopping. (HOST-047)
    fn exit_if_idle(&self, state: &State) {
        if state.sessions.is_empty() && (state.clients.is_empty() || state.stopping) {
            self.log.log(&["idle, exiting"]);
            self.exit(0);
        }
    }

    /// After [`crate::host::UNUSED`], a host no app has connected to applies
    /// the idle rule. (HOST-032)
    pub(crate) fn unused_check(&self) {
        let state = self.lock();
        if state.clients.is_empty() {
            self.exit_if_idle(&state);
        }
    }

    // -----------------------------------------------------------------------
    // Connections

    fn broadcast(&self, state: &State, msg: &HostMessage) {
        let line: Line = encode(msg).into();
        for client in state.clients.values() {
            self.unwritten.fetch_add(1, Ordering::SeqCst);
            if client.send(Arc::clone(&line)).is_err() {
                self.unwritten.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    fn send_to(&self, writer: &mpsc::UnboundedSender<Line>, msg: &HostMessage) {
        self.unwritten.fetch_add(1, Ordering::SeqCst);
        if writer.send(encode(msg).into()).is_err() {
            self.unwritten.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Serves one connection until it closes.
    pub(crate) async fn serve(self: Arc<Self>, stream: impl Duplex) {
        let id = self.next_id();
        let (mut read, mut write) = tokio::io::split(stream);
        let (writer, mut lines) = mpsc::unbounded_channel::<Line>();
        let unwritten = Arc::clone(&self.unwritten);
        let writing = tokio::spawn(async move {
            while let Some(line) = lines.recv().await {
                let failed = write.write_all(line.as_bytes()).await.is_err();
                unwritten.fetch_sub(1, Ordering::SeqCst);
                if failed {
                    // Socket errors are ignored (HOST-043); what's left is dropped.
                    while lines.try_recv().is_ok() {
                        unwritten.fetch_sub(1, Ordering::SeqCst);
                    }
                    lines.close();
                    while lines.recv().await.is_some() {
                        unwritten.fetch_sub(1, Ordering::SeqCst);
                    }
                    return;
                }
            }
            let _ = write.shutdown().await;
        });

        let mut reader = MessageReader::new();
        let mut greeted = false;
        let mut buffer = vec![0u8; 64 * 1024];
        'reading: loop {
            let n = match read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for msg in reader.feed(&buffer[..n]) {
                if !self.on_message(id, &writer, &mut greeted, &msg) {
                    // socket.destroy(): nothing more is read or written.
                    writing.abort();
                    break 'reading;
                }
            }
        }
        drop(writer);
        self.closed(id, greeted);
    }

    /// One message from a connection; false when the connection is to be
    /// closed at once.
    fn on_message(&self, id: u64, writer: &mpsc::UnboundedSender<Line>, greeted: &mut bool, msg: &Value) -> bool {
        let mut state = self.lock();
        // msg.type, which throws for null as in JavaScript.
        let Some(kind) = message_type(msg) else {
            self.log.log(&[
                "message failed",
                "TypeError: Cannot read properties of null (reading 'type')",
            ]);
            return true;
        };
        if !*greeted {
            // HOST-036: the first message must be hello.
            if kind != Some("hello") {
                return false;
            }
            *greeted = true;
            let welcome = self.welcome(&state);
            self.send_to(writer, &welcome);
            let protocol = msg.get("protocol");
            // HOST-038: an app on another protocol may only ask us to stop.
            if protocol.and_then(Value::as_f64) == Some(f64::from(PROTOCOL)) {
                state.clients.insert(id, writer.clone());
                self.log.log(&["app connected", ""]);
            } else {
                self.log
                    .log(&["app connected", &format!("(protocol {})", js_string(protocol))]);
            }
            return true;
        }
        // HOST-039: a greeted connection that isn't an app may only stop us.
        if state.clients.contains_key(&id) || kind == Some("shutdown") {
            self.handle(&mut state, writer, kind, msg);
        }
        true
    }

    /// HOST-037: every live session, in spawn order, with its output,
    /// history and status line.
    fn welcome(&self, state: &State) -> HostMessage {
        let sessions = state
            .sessions
            .iter()
            .map(|(session_id, s)| HostSession {
                session_id: session_id.clone(),
                project_id: s.project_id.clone(),
                initial: s.initial,
                buffer: s.output.joined(),
                history: s.history.iter().cloned().collect(),
                status_line: s.status_line.clone(),
            })
            .collect();
        HostMessage::Welcome(Welcome {
            protocol: f64::from(PROTOCOL),
            hook_port: self.hook_port.load(Ordering::SeqCst) as f64,
            hook_token: self.hook_token.clone(),
            sessions,
            mcp: Some(true),
            hook_auth_header: Some(true),
            key: self.host_key.clone(),
        })
    }

    /// A connection closed. (HOST-044, HOST-045)
    fn closed(&self, id: u64, greeted: bool) {
        let mut state = self.lock();
        if state.clients.shift_remove(&id).is_some() {
            self.log.log(&["app disconnected"]);
        }
        let waiting: Vec<u64> = state
            .relays
            .iter()
            .filter(|(_, r)| r.client == id)
            .map(|(relay, _)| *relay)
            .collect();
        for relay in waiting {
            if let Some(relay) = state.relays.remove(&relay) {
                let _ = relay.settle.send(None);
            }
        }
        // A probe from another host never says hello, and must not stop this one.
        if greeted {
            self.exit_if_idle(&state);
        }
    }

    fn handle(&self, state: &mut State, writer: &mpsc::UnboundedSender<Line>, kind: Option<&str>, msg: &Value) {
        let session = || msg.get("sessionId").and_then(Value::as_str);
        match kind {
            Some("spawn") => {
                let error = self.spawn(state, msg);
                // HOST-041: only to the connection that asked.
                self.send_to(
                    writer,
                    &HostMessage::Spawned {
                        id: msg.get("id").cloned(),
                        error,
                    },
                );
            }
            Some("write") => {
                // HOST-059, HOST-060
                let (Some(s), Some(data)) = (
                    session().and_then(|id| state.sessions.get_mut(id)),
                    msg.get("data").and_then(Value::as_str),
                ) else {
                    return;
                };
                s.pty.write(data);
                if is_status_key(data) {
                    record(
                        s,
                        HistoryEntry::Input {
                            input: data.to_owned(),
                            at: Some(now_ms()),
                        },
                    );
                }
            }
            Some("resize") => {
                // HOST-061: errors (an exited process, a bad size) are swallowed.
                if let Some(s) = session().and_then(|id| state.sessions.get(id))
                    && let (Some(cols), Some(rows)) = (
                        msg.get("cols").and_then(Value::as_f64),
                        msg.get("rows").and_then(Value::as_f64),
                    )
                {
                    let _ = s.pty.resize(cols, rows);
                }
            }
            Some("kill") => {
                if let Some(id) = session().map(str::to_owned)
                    && state.sessions.contains_key(&id)
                {
                    self.kill(state, &id, &[Signal::Term]);
                }
            }
            Some("shutdown") => self.shutdown(state),
            Some("mcp-reply") => {
                let relay = msg.get("id").and_then(Value::as_f64).and_then(relay_id);
                if let Some(relay) = relay.and_then(|relay| state.relays.remove(&relay)) {
                    let _ = relay.settle.send(msg.get("reply").cloned());
                }
            }
            // HOST-040: a second hello or an unknown type is ignored.
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Sessions

    /// Starts a session; the error's message when it couldn't. (HOST-048–054)
    fn spawn(&self, state: &mut State, msg: &Value) -> Option<String> {
        let session_id = msg.get("sessionId").and_then(Value::as_str);
        if session_id.is_some_and(|id| state.sessions.contains_key(id)) {
            return None;
        }
        let request = match SpawnFields::read(msg) {
            Ok(request) => request,
            Err(err) => return Some(err),
        };
        let (pty, events) = match Pty::spawn(&request.options, TERMINAL_NAME) {
            Ok(spawned) => spawned,
            Err(err) => return Some(err),
        };
        let session_id = request.session_id;
        let generation = self.next_id();
        state.sessions.insert(
            session_id.clone(),
            Session {
                generation,
                project_id: request.project_id,
                initial: request.initial,
                pty,
                output: ReplayBuffer::new(),
                history: VecDeque::new(),
                status_line: None,
                kill_timer: None,
            },
        );
        self.log.log(&["spawned", &session_id, &request.file_as_given]);
        let host = self.arc();
        tokio::spawn(async move { host.follow(session_id, generation, events).await });
        None
    }

    /// The host, for a task to hold.
    pub(crate) fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("the host is alive while it serves")
    }

    /// Passes a session's output on, then its exit. (HOST-055–058)
    async fn follow(
        self: Arc<Self>,
        session_id: String,
        generation: u64,
        mut events: mpsc::UnboundedReceiver<PtyEvent>,
    ) {
        while let Some(event) = events.recv().await {
            let mut state = self.lock();
            let Some(s) = state
                .sessions
                .get_mut(&session_id)
                .filter(|s| s.generation == generation)
            else {
                return;
            };
            match event {
                PtyEvent::Data(data) => {
                    s.output.push(data.clone());
                    self.broadcast(
                        &state,
                        &HostMessage::Data {
                            session_id: session_id.clone(),
                            data,
                        },
                    );
                }
                PtyEvent::Exit(code) => {
                    if let Some(timer) = s.kill_timer.take() {
                        timer.abort();
                    }
                    state.sessions.shift_remove(&session_id);
                    self.log.log(&["exited", &session_id, &code.to_string()]);
                    self.broadcast(
                        &state,
                        &HostMessage::Exit {
                            session_id,
                            exit_code: Some(code as f64),
                        },
                    );
                    self.exit_if_idle(&state);
                    return;
                }
            }
        }
    }

    /// HOST-062, HOST-063: the signals now, and SIGKILL after the grace
    /// period unless one is already on its way. Windows has no signals.
    fn kill(&self, state: &mut State, session_id: &str, signals: &[Signal]) {
        let Some(s) = state.sessions.get_mut(session_id) else {
            return;
        };
        #[cfg(windows)]
        {
            let _ = signals;
            s.pty.kill();
        }
        #[cfg(unix)]
        {
            for signal in signals {
                s.pty.signal(signal.number());
            }
            if s.kill_timer.is_none() {
                let host = self.arc();
                let (session_id, generation) = (session_id.to_owned(), s.generation);
                s.kill_timer = Some(tokio::spawn(async move {
                    tokio::time::sleep(KILL_GRACE).await;
                    let state = host.lock();
                    if let Some(s) = state.sessions.get(&session_id).filter(|s| s.generation == generation) {
                        s.pty.signal(Signal::Kill.number());
                    }
                }));
            }
        }
    }

    /// HOST-064: stops every session and then the host.
    fn shutdown(&self, state: &mut State) {
        self.log.log(&["stopping every session"]);
        state.stopping = true;
        // SIGHUP too, as when a terminal window closes: interactive shells ignore SIGTERM.
        let ids: Vec<String> = state.sessions.keys().cloned().collect();
        for id in ids {
            self.kill(state, &id, &[Signal::Term, Signal::Hup]);
        }
        self.exit_if_idle(state);
        let host = self.arc();
        tokio::spawn(async move {
            tokio::time::sleep(KILL_GRACE + Duration::from_millis(1000)).await;
            host.exit(0);
        });
    }

    // -----------------------------------------------------------------------
    // Hooks and the board tools (http.rs)

    /// Whether a session is live.
    pub(crate) fn has_session(&self, session_id: &str) -> bool {
        self.lock().sessions.contains_key(session_id)
    }

    /// A hook or status line report for a session. (HOST-073–078)
    pub(crate) fn hook(&self, event: &str, session_id: &str, payload: Value) {
        let mut state = self.lock();
        let Some(s) = state.sessions.get_mut(session_id) else {
            return;
        };
        if event == nodus_host_proto::STATUS_LINE_EVENT {
            s.status_line = Some(StatusLine {
                payload: payload.clone(),
                at: Some(now_ms()),
            });
        } else {
            let Some(slimmed) = slim(&payload) else {
                // Object.entries(null) throws in the request's end handler.
                self.log
                    .log(&["uncaught", "TypeError: Cannot convert undefined or null to object"]);
                return;
            };
            record(
                s,
                HistoryEntry::Hook {
                    hook: event.to_owned(),
                    payload: Value::Object(slimmed),
                    at: Some(now_ms()),
                },
            );
        }
        self.broadcast(
            &state,
            &HostMessage::Hook {
                session_id: session_id.to_owned(),
                event: event.to_owned(),
                payload,
            },
        );
    }

    /// Passes a board tools request to the newest app, at once, so a batch's
    /// requests reach it in order; None when no app is connected. (HOST-085, HOST-087)
    pub(crate) fn start_relay(&self, session_id: &str, message: Value) -> Option<PendingRelay> {
        let mut state = self.lock();
        // There's only ever one app; the newest connection is the live one.
        let (&client, writer) = state.clients.last()?;
        let writer = writer.clone();
        let relay = state.next_relay;
        state.next_relay += 1;
        let (settle, answer) = oneshot::channel();
        state.relays.insert(relay, Relay { client, settle });
        self.send_to(
            &writer,
            &HostMessage::Mcp {
                id: relay as f64,
                session_id: session_id.to_owned(),
                message,
            },
        );
        Some(PendingRelay {
            relay,
            answer,
            deadline: tokio::time::Instant::now() + crate::http::RELAY_TIMEOUT,
        })
    }

    /// The app's answer to a relayed request: its reply, or None when it had
    /// none, went away, or took longer than 20 s. (HOST-086)
    pub(crate) async fn finish_relay(&self, pending: PendingRelay) -> Option<Value> {
        let PendingRelay {
            relay,
            answer,
            deadline,
        } = pending;
        match tokio::time::timeout_at(deadline, answer).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => None,
            Err(_) => {
                self.lock().relays.remove(&relay);
                None
            }
        }
    }
}

/// The signals the host sends on a Unix.
#[derive(Debug, Clone, Copy)]
enum Signal {
    Term,
    Hup,
    #[cfg_attr(windows, expect(dead_code, reason = "Windows has no signals"))]
    Kill,
}

#[cfg(unix)]
impl Signal {
    fn number(self) -> libc::c_int {
        match self {
            Self::Term => libc::SIGTERM,
            Self::Hup => libc::SIGHUP,
            Self::Kill => libc::SIGKILL,
        }
    }
}

/// A relay id from a JSON number: only whole, positive ones can match.
fn relay_id(id: f64) -> Option<u64> {
    (id.fract() == 0.0 && (1.0..9_007_199_254_740_992.0).contains(&id)).then_some(id as u64)
}

/// `msg.type`: None when reading it would throw (`null`); Some(None) when
/// it isn't a string or there's none.
fn message_type(msg: &Value) -> Option<Option<&str>> {
    match msg {
        Value::Null => None,
        Value::Object(fields) => Some(fields.get("type").and_then(Value::as_str)),
        _ => Some(None),
    }
}

/// `String(value)` for what a hello's `protocol` can be.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n
            .as_f64()
            .map_or_else(|| n.to_string(), nodus_domain::json::number_to_string),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

fn record(s: &mut Session, entry: HistoryEntry) {
    push_capped(&mut s.history, entry);
}

/// HOST-066: at most 1000 entries, the oldest dropped first.
fn push_capped(history: &mut VecDeque<HistoryEntry>, entry: HistoryEntry) {
    history.push_back(entry);
    while history.len() > HISTORY_LIMIT {
        history.pop_front();
    }
}

/// A hook payload as kept in history: its short scalar fields, without tool
/// inputs and outputs, in their order (HOST-067). None for `null`, which
/// `Object.entries` refuses.
fn slim(payload: &Value) -> Option<Map<String, Value>> {
    let keep = |value: &Value| match value {
        Value::Number(_) | Value::Bool(_) => true,
        Value::String(s) => utf16_len(s) <= SLIM_STRING_LIMIT,
        _ => false,
    };
    match payload {
        Value::Null => None,
        Value::Object(fields) => Some(
            fields
                .iter()
                .filter(|(_, v)| keep(v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        // Object.entries of an array or a string lists it by index.
        Value::Array(items) => Some(
            items
                .iter()
                .enumerate()
                .filter(|(_, v)| keep(v))
                .map(|(i, v)| (i.to_string(), v.clone()))
                .collect(),
        ),
        Value::String(s) => Some(
            s.chars()
                .enumerate()
                .map(|(i, c)| (i.to_string(), Value::String(c.into())))
                .collect(),
        ),
        Value::Number(_) | Value::Bool(_) => Some(Map::new()),
    }
}

/// A `spawn` message's fields, read as node-pty would take them.
struct SpawnFields {
    session_id: String,
    project_id: String,
    initial: SessionStatus,
    /// `req.file` as the log line shows it.
    file_as_given: String,
    options: PtyOptions,
}

impl SpawnFields {
    /// What node-pty throws for arguments of the wrong type.
    #[cfg(unix)]
    const USAGE: &str = "Usage: pty.fork(file, args, env, cwd, cols, rows, uid, gid, utf8, helperPath, onexit)";
    #[cfg(windows)]
    const USAGE: &str = "Usage: pty.startProcess(file, cols, rows, debug, pipeName, inheritCursor, useConptyDll)";

    fn read(msg: &Value) -> Result<Self, String> {
        let text = |name: &str| msg.get(name).and_then(Value::as_str).map(str::to_owned);
        let falsy = |value: Option<&Value>| match value {
            None | Some(Value::Null) => true,
            Some(Value::Bool(b)) => !b,
            Some(Value::String(s)) => s.is_empty(),
            Some(Value::Number(n)) => n.as_f64().is_none_or(|n| n == 0.0 || n.is_nan()),
            _ => false,
        };
        let file = match msg.get("file") {
            v if falsy(v) => String::new(),
            Some(Value::String(file)) => file.clone(),
            _ => return Err(Self::USAGE.into()),
        };
        let args = match msg.get("args") {
            v if falsy(v) => Vec::new(),
            Some(Value::String(_)) if cfg!(unix) => return Err("args as a string is not supported on unix.".into()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| Self::USAGE.to_owned())?,
            _ => return Err(Self::USAGE.into()),
        };
        let env = match msg.get("env") {
            Some(Value::Object(vars)) => vars.iter().map(|(k, v)| (k.clone(), js_string(Some(v)))).collect(),
            v if falsy(v) => host_env(),
            _ => Vec::new(),
        };
        let cwd = match msg.get("cwd") {
            v if falsy(v) => String::new(),
            Some(Value::String(cwd)) => cwd.clone(),
            _ => return Err(Self::USAGE.into()),
        };
        let size = |name: &str| match msg.get(name) {
            v if falsy(v) => Ok(0.0),
            Some(Value::Number(n)) => Ok(n.as_f64().unwrap_or(0.0)),
            _ => Err(Self::USAGE.to_owned()),
        };
        let (cols, rows) = (size("cols")?, size("rows")?);
        let initial = msg
            .get("initial")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(SessionStatus::Done);
        Ok(Self {
            session_id: text("sessionId").unwrap_or_default(),
            project_id: text("projectId").unwrap_or_default(),
            initial,
            file_as_given: file.clone(),
            options: PtyOptions {
                file,
                args,
                env,
                cwd,
                cols,
                rows,
            },
        })
    }
}

/// `process.env` with node-pty's `_sanitizeEnv`, for a spawn without one.
fn host_env() -> Vec<(String, String)> {
    const DROPPED: [&str; 8] = [
        "TMUX",
        "TMUX_PANE",
        "STY",
        "WINDOW",
        "WINDOWID",
        "TERMCAP",
        "COLUMNS",
        "LINES",
    ];
    std::env::vars()
        .filter(|(name, _)| !DROPPED.contains(&name.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slims_payloads_to_short_scalars_in_order() {
        let long = "x".repeat(1025);
        let exact = "é".repeat(1024);
        let payload = json!({
            "session_id": "s1", "n": 3, "ok": true, "nothing": null, "tool_input": {"command": "ls"},
            "list": [1], "long": long, "exact": exact, "last": 1.5
        });
        assert_eq!(
            Value::Object(slim(&payload).unwrap()),
            json!({"session_id": "s1", "n": 3, "ok": true, "exact": exact, "last": 1.5})
        );
        assert_eq!(slim(&Value::Null), None);
        assert_eq!(Value::Object(slim(&json!(5)).unwrap()), json!({}));
        assert_eq!(
            Value::Object(slim(&json!([1, {"a": 1}, "x"])).unwrap()),
            json!({"0": 1, "2": "x"})
        );
        assert_eq!(Value::Object(slim(&json!("ab")).unwrap()), json!({"0": "a", "1": "b"}));
    }

    #[test]
    fn writes_a_protocol_as_string_does() {
        assert_eq!(js_string(None), "undefined");
        assert_eq!(js_string(Some(&json!(0))), "0");
        assert_eq!(js_string(Some(&json!(2.5))), "2.5");
        assert_eq!(js_string(Some(&json!("1"))), "1");
        assert_eq!(js_string(Some(&json!(null))), "null");
        assert_eq!(js_string(Some(&json!([1, null, "a"]))), "1,,a");
        assert_eq!(js_string(Some(&json!({}))), "[object Object]");
    }

    #[test]
    fn host_066_keeps_the_newest_1000_history_entries() {
        let mut history = VecDeque::new();
        for i in 0..1005 {
            push_capped(
                &mut history,
                HistoryEntry::Input {
                    input: "\r".into(),
                    at: Some(f64::from(i)),
                },
            );
        }
        assert_eq!(history.len(), 1000);
        assert_eq!(
            history.front(),
            Some(&HistoryEntry::Input {
                input: "\r".into(),
                at: Some(5.0)
            })
        );
        assert_eq!(
            history.back(),
            Some(&HistoryEntry::Input {
                input: "\r".into(),
                at: Some(1004.0)
            })
        );
    }

    #[test]
    fn reads_relay_ids() {
        assert_eq!(relay_id(1.0), Some(1));
        assert_eq!(relay_id(1.5), None);
        assert_eq!(relay_id(0.0), None);
        assert_eq!(relay_id(-1.0), None);
    }

    /// Lets tasks the paused clock has made due run.
    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn host_086_each_relay_gives_up_20_seconds_after_it_was_sent() {
        let (log, _lines) = Logger::memory();
        let host = Host::new(log, "token".into(), None);
        // An app that never answers.
        let (app, mut heard) = mpsc::unbounded_channel::<Line>();
        host.lock().clients.insert(1, app);

        let first = host
            .start_relay("s1", json!({"id": 1, "method": "tools/call"}))
            .unwrap();
        tokio::time::advance(Duration::from_millis(10_000)).await;
        let second = host
            .start_relay("s1", json!({"id": 2, "method": "tools/call"}))
            .unwrap();
        assert!(heard.recv().await.unwrap().contains(r#""type":"mcp""#));
        assert!(heard.recv().await.unwrap().contains(r#""sessionId":"s1""#));
        let (first, second) = {
            let host = Arc::clone(&host);
            let other = Arc::clone(&host);
            (
                tokio::spawn(async move { host.finish_relay(first).await }),
                tokio::spawn(async move { other.finish_relay(second).await }),
            )
        };
        tokio::time::advance(Duration::from_millis(9_999)).await;
        settle().await;
        assert!(!first.is_finished());
        // 20 s after the first was sent: it settles with nothing, and is forgotten.
        tokio::time::advance(Duration::from_millis(1)).await;
        settle().await;
        assert!(first.is_finished());
        assert_eq!(first.await.unwrap(), None);
        assert!(!second.is_finished());
        assert_eq!(host.lock().relays.len(), 1);
        tokio::time::advance(Duration::from_millis(10_000)).await;
        assert_eq!(second.await.unwrap(), None);
        assert!(host.lock().relays.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn host_064_exits_4_seconds_after_shutdown_whatever_is_left() {
        let (log, lines) = Logger::memory();
        let host = Host::new(log, "token".into(), None);
        // A session that ignores SIGTERM and SIGHUP, whose exit the host never
        // hears of (nothing follows its events), so it's still there when the
        // time is up: as a process that outlived SIGKILL would be.
        let (pty, mut events) = Pty::spawn(
            &PtyOptions {
                file: "/bin/sh".into(),
                args: vec!["-c".into(), "trap '' TERM HUP; echo ready; exec sleep 30".into()],
                env: vec![("PATH".into(), "/usr/bin:/bin".into())],
                cwd: "/tmp".into(),
                cols: 80.0,
                rows: 24.0,
            },
            TERMINAL_NAME,
        )
        .unwrap();
        // The trap is set before any signal goes.
        let mut said = String::new();
        while !said.contains("ready") {
            match events.recv().await {
                Some(PtyEvent::Data(data)) => said.push_str(&data),
                other => panic!("{other:?}"),
            }
        }
        host.lock().sessions.insert(
            "s1".into(),
            Session {
                generation: 1,
                project_id: "p".into(),
                initial: SessionStatus::Done,
                pty,
                output: ReplayBuffer::new(),
                history: VecDeque::new(),
                status_line: None,
                kill_timer: None,
            },
        );
        host.shutdown(&mut host.lock());
        let exited = {
            let host = Arc::clone(&host);
            tokio::spawn(async move { host.exited().await })
        };
        tokio::time::advance(Duration::from_millis(3_999)).await;
        settle().await;
        assert!(!exited.is_finished(), "{:?}", lines.lock().unwrap());
        assert_eq!(host.lock().sessions.len(), 1);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(exited.await.unwrap(), 0);
        let logged: Vec<String> = lines.lock().unwrap().iter().map(|l| l[25..].to_owned()).collect();
        assert_eq!(logged, ["stopping every session"]);
    }
}
