//! The app's side of the session host (`src/main/pty.ts`): connecting to a
//! running host or starting one, adopting its sessions, and passing each
//! session's input, size and output between the window and the host.
//!
//! The pseudo-terminals live in the session host (`nodus-host`), a
//! background process that outlives the app, so sessions keep running after
//! it quits. This keeps each live session's recent output for replaying into
//! terminals, coalesces output into `Event::PtyData` every 16 ms, reports
//! exits as `Event::PtyExit`, and tells the status machine (`hooks`) through
//! [`SessionInput`] when a session starts and stops, each hook and status
//! line, and each keystroke. Board tools requests the host relays are
//! answered through [`BoardTools`]; a host from another version of Nodus is
//! put to the user through [`HostPrompt`].
//!
//! What starts a session is [`launch::SpawnOptions`](crate::launch::SpawnOptions);
//! the command line is built by the caller (`launch::command_for`) once the
//! host is connected, since its hook endpoint and board tools come from it.

mod connection;
mod launcher;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;
use nodus_domain::{Event, EventSink, SessionStatus};
use nodus_host_proto::{
    ClientMessage, HistoryEntry, HostMessage, HostPaths, PROTOCOL, Platform, ReplayBuffer, STATUS_LINE_EVENT,
    SpawnRequest, Welcome, encode, host_paths, trusts_host, write_hook_config,
};
use serde_json::Value;
use tokio::sync::{oneshot, watch};

pub use launcher::{HostLauncher, ProcessLauncher};

use self::connection::{Conn, Greeting, Reader};
use crate::hooks::{HookEndpoint, SessionInput};
use crate::launch::{self, Command, SpawnOptions};

/// How often output reaches the window, at most, per session.
const FLUSH: Duration = Duration::from_millis(16);
/// The host sends SIGKILL after its own 3 s grace period.
const KILL_WAIT: Duration = Duration::from_millis(4000);
/// How long a host just started gets to listen, tried every 100 ms.
const HOST_START_TRIES: u32 = 100;
const HOST_START_POLL: Duration = Duration::from_millis(100);
/// A host that accepts a connection but never answers is treated as gone.
const WELCOME: Duration = Duration::from_millis(5000);

const DIDNT_ANSWER: &str = "The session host didn’t answer";
const UNTRUSTED: &str = "Another program answered as the session host, so Nodus didn’t trust it with your sessions.";
const STOPPED: &str = "The session host stopped";
const OTHER_PROTOCOL: &str = "The session host speaks another protocol";

/// Why connecting or starting a session failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// Shown to the user as it is (0.9.4's text).
    #[error("{0}")]
    Failed(String),
    /// The user chose Quit when sessions from another version were running:
    /// the app exits at once with code 0, without its quit handlers (HOST-103).
    #[error("quit")]
    Quit,
}

/// The user's answer to sessions from another version of Nodus (HOST-102).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceChoice {
    /// `Stop Sessions`, the default.
    StopSessions,
    /// `Quit`, the cancel button.
    Quit,
}

/// A boxed future, for the traits the app implements.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Asks the user what to do about a host on another protocol: a warning
/// box with message `Sessions from another version of Nodus are still
/// running`, detail `This version can’t connect to them. Stop them to carry
/// on, or quit and let them finish.`, and buttons `Stop Sessions` (default)
/// and `Quit` (cancel). It can come up at launch. (HOST-102)
pub trait HostPrompt: Send + Sync {
    fn other_version(&self) -> BoxFuture<ReplaceChoice>;
}

/// Answers the board tools' requests the host relays (04's `answerMcp`):
/// the reply, `None` for none, or an error, logged as `[board-tools]`.
pub trait BoardTools: Send + Sync {
    fn answer(&self, session_id: &str, message: Value) -> BoxFuture<Result<Option<Value>, String>>;
}

/// What the client needs from the rest of the app.
#[derive(Clone)]
pub struct Wiring {
    /// Where `Event::PtyData` and `Event::PtyExit` go.
    pub events: Arc<dyn EventSink>,
    /// The status machine.
    pub status: Arc<dyn SessionInput>,
    /// Set from each adopted host's welcome (HOST-106).
    pub endpoint: Arc<HookEndpoint>,
    pub board_tools: Arc<dyn BoardTools>,
    pub prompt: Arc<dyn HostPrompt>,
    /// How a host is started when there's none.
    pub launcher: Arc<dyn HostLauncher>,
}

/// What a command builder learns from the connected host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostInfo {
    /// The host relays the board tools; one started by an older app doesn't (HOST-107).
    pub relays_mcp: bool,
}

/// The app's connection to the session host. Clones share it.
#[derive(Clone)]
pub struct HostClient {
    inner: Arc<Inner>,
}

struct Inner {
    me: std::sync::Weak<Inner>,
    paths: HostPaths,
    platform: Platform,
    wiring: Wiring,
    state: Mutex<State>,
    /// Held while state changes turn into events and status calls, so they
    /// go out in the order they happened.
    order: Mutex<()>,
    next_id: AtomicU64,
}

#[derive(Default)]
struct State {
    running: IndexMap<String, Running>,
    spawning: HashMap<String, Shared>,
    pending_spawns: HashMap<u64, PendingSpawn>,
    exit_waiters: HashMap<String, Vec<oneshot::Sender<()>>>,
    host: Option<Conn>,
    relays_mcp: bool,
    connecting: Option<(u64, Shared)>,
    next_request: u64,
    quitting: bool,
}

struct Running {
    project_id: String,
    /// Output kept for replaying into a freshly created terminal.
    output: ReplayBuffer,
    pending: String,
    /// The flush timer waiting to send `pending`, by its number.
    flush: Option<u64>,
}

struct PendingSpawn {
    session_id: String,
    project_id: String,
    initial: SessionStatus,
    done: oneshot::Sender<Result<(), ClientError>>,
}

/// One attempt, awaited by everyone who asks while it runs (`connecting`,
/// `spawning` in pty.ts). It runs on its own task, as a promise does.
#[derive(Clone)]
struct Shared(watch::Receiver<Option<Result<(), ClientError>>>);

impl Shared {
    fn spawn(work: impl Future<Output = Result<(), ClientError>> + Send + 'static) -> Self {
        let (done, wait) = watch::channel(None);
        tokio::spawn(async move {
            let _ = done.send(Some(work.await));
        });
        Self(wait)
    }

    async fn wait(&self) -> Result<(), ClientError> {
        let mut wait = self.0.clone();
        match wait.wait_for(Option::is_some).await {
            Ok(result) => result.clone().unwrap_or(Ok(())),
            Err(_) => Err(ClientError::Failed(STOPPED.into())),
        }
    }
}

impl HostClient {
    /// A client for the host of the app whose data is in `user_data`.
    pub fn new(user_data: impl Into<PathBuf>, wiring: Wiring) -> Self {
        let platform = Platform::current();
        Self {
            inner: Arc::new_cyclic(|me| Inner {
                me: me.clone(),
                paths: host_paths(&user_data.into(), platform),
                platform,
                wiring,
                state: Mutex::new(State {
                    next_request: 1,
                    ..State::default()
                }),
                order: Mutex::new(()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// The host's files for this app.
    pub fn paths(&self) -> &HostPaths {
        &self.inner.paths
    }

    /// HOST-128
    pub fn is_live(&self, session_id: &str) -> bool {
        self.inner.lock().running.contains_key(session_id)
    }

    /// HOST-128: in the order they were tracked.
    pub fn live_session_ids(&self) -> Vec<String> {
        self.inner.lock().running.keys().cloned().collect()
    }

    /// Whether the connected host relays the board tools (HOST-107).
    pub fn relays_mcp(&self) -> bool {
        self.inner.lock().relays_mcp
    }

    /// Connects to the session host and takes on its sessions. With `start`,
    /// starts a host if none is running; without, a missing host just means
    /// no sessions (HOST-093–095). Returns at once when connected; while an
    /// attempt is in flight every caller gets that attempt.
    pub async fn connect_host(&self, start: bool) -> Result<(), ClientError> {
        let attempt = {
            let mut state = self.inner.lock();
            if state.host.is_some() {
                return Ok(());
            }
            match &state.connecting {
                Some((_, attempt)) => attempt.clone(),
                None => {
                    let id = self.inner.next_id();
                    let inner = Arc::clone(&self.inner);
                    let attempt = Shared::spawn(async move {
                        let result = Inner::connect(&inner, start).await;
                        let mut state = inner.lock();
                        if state.connecting.as_ref().is_some_and(|(current, _)| *current == id) {
                            state.connecting = None;
                        }
                        result
                    });
                    state.connecting = Some((id, attempt.clone()));
                    attempt
                }
            }
        };
        attempt.wait().await
    }

    /// Leaves the host and its sessions running, for when the app quits (HOST-111).
    pub fn disconnect(&self) {
        let mut state = self.inner.lock();
        state.quitting = true;
        if let Some(host) = &state.host {
            host.end();
        }
    }

    /// Stops every session and the host (HOST-112). The app can carry on
    /// after, as when an update's installer doesn't start: a new session
    /// starts a new host.
    pub async fn stop_all(&self) {
        let host = {
            let mut state = self.inner.lock();
            let Some(host) = state.host.clone() else { return };
            // The host reports each session it stops; losing it after isn't a crash.
            state.quitting = true;
            host
        };
        host.write(encode(&ClientMessage::Shutdown));
        let _ = tokio::time::timeout(KILL_WAIT + Duration::from_millis(1000), host.closed()).await;
        let still = {
            let mut state = self.inner.lock();
            state.quitting = false;
            state.host.as_ref().is_some_and(|current| current.id == host.id)
        };
        // Any it didn't report before it went are over too.
        if !still {
            let _order = self.inner.order();
            for session_id in self.live_session_ids() {
                self.inner.exited(&session_id, -1);
            }
        }
    }

    /// Starts a session's process in the host, connecting to (or starting)
    /// a host first, then building its command with `command` (HOST-113–115).
    /// Does nothing for a live session; concurrent starts of one session
    /// share the first.
    pub async fn spawn_session<F>(&self, opts: &SpawnOptions, command: F) -> Result<(), ClientError>
    where
        F: FnOnce(&SpawnOptions, HostInfo) -> Result<Command, String> + Send + 'static,
    {
        let attempt = {
            let mut state = self.inner.lock();
            if state.running.contains_key(&opts.session_id) {
                return Ok(());
            }
            match state.spawning.get(&opts.session_id) {
                Some(attempt) => attempt.clone(),
                None => {
                    let (client, opts) = (self.clone(), opts.clone());
                    let session_id = opts.session_id.clone();
                    let attempt = Shared::spawn(async move {
                        let result = client.start_session(&opts, command).await;
                        client.inner.lock().spawning.remove(&opts.session_id);
                        result
                    });
                    state.spawning.insert(session_id, attempt.clone());
                    attempt
                }
            }
        };
        attempt.wait().await
    }

    async fn start_session<F>(&self, opts: &SpawnOptions, command: F) -> Result<(), ClientError>
    where
        F: FnOnce(&SpawnOptions, HostInfo) -> Result<Command, String>,
    {
        // Connected first: the hook endpoint in the command comes from the host.
        self.connect_host(true).await?;
        let info = HostInfo {
            relays_mcp: self.relays_mcp(),
        };
        let Command { file, args, env } = command(opts, info).map_err(ClientError::Failed)?;
        // With a first message the turn starts immediately; show it as working right away.
        let initial = launch::initial_status(opts);
        let (cols, rows) = launch::pty_size(opts);
        let (done, result) = oneshot::channel();
        {
            let mut state = self.inner.lock();
            let id = state.next_request;
            state.next_request += 1;
            state.pending_spawns.insert(
                id,
                PendingSpawn {
                    session_id: opts.session_id.clone(),
                    project_id: opts.project_id.clone(),
                    initial,
                    done,
                },
            );
            let request = SpawnRequest {
                id: id as f64,
                session_id: opts.session_id.clone(),
                project_id: opts.project_id.clone(),
                initial,
                file: file.to_string_lossy().into_owned(),
                args,
                env: env
                    .into_iter()
                    .map(|(name, value)| (name, Value::String(value)))
                    .collect(),
                cwd: opts.cwd.clone(),
                cols,
                rows,
            };
            Inner::send(&state, &ClientMessage::Spawn(request));
        }
        // A spawn sent while no host is connected waits for ever, as in 0.9.4
        // (the launch's attempt is awaited before anything can start one).
        result
            .await
            .unwrap_or_else(|_| Err(ClientError::Failed(STOPPED.into())))
    }

    /// Buffered output for replay (HOST-119). Pending output is sent first,
    /// so everything sent before this call is part of the snapshot and
    /// nothing is sent twice; `""` for a session that isn't live.
    pub fn snapshot(&self, session_id: &str) -> String {
        let _order = self.inner.order();
        self.inner.flush(session_id);
        self.inner
            .lock()
            .running
            .get(session_id)
            .map(|r| r.output.joined())
            .unwrap_or_default()
    }

    /// HOST-121: the keystroke rules first, then the write.
    pub fn write(&self, session_id: &str, data: &str) {
        let _order = self.inner.order();
        if !self.is_live(session_id) {
            return;
        }
        self.inner.wiring.status.user_input(session_id, data, None);
        let state = self.inner.lock();
        Inner::send(
            &state,
            &ClientMessage::Write {
                session_id: session_id.into(),
                data: data.into(),
            },
        );
    }

    /// Sends a prompt to a running Claude session: typed in as a paste, so
    /// its lines don't send it early, then Return 200 ms later (HOST-122).
    pub fn send_prompt(&self, session_id: &str, text: &str) {
        self.write(session_id, &launch::bracketed_paste(text));
        let (client, session_id) = (self.clone(), session_id.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(launch::SEND_RETURN_DELAY).await;
            client.write(&session_id, "\r");
        });
    }

    /// HOST-123
    pub fn resize(&self, session_id: &str, cols: f64, rows: f64) {
        let state = self.inner.lock();
        if !state.running.contains_key(session_id) || !cols.is_finite() || !rows.is_finite() {
            return;
        }
        Inner::send(
            &state,
            &ClientMessage::Resize {
                session_id: session_id.into(),
                cols: cols.floor().max(20.0),
                rows: rows.floor().max(5.0),
            },
        );
    }

    /// Asks the host to stop a session; the future resolves when it has
    /// exited or after 4 s, whichever comes first, and at once for a session
    /// that isn't live (HOST-124). The request goes out when this is called,
    /// whether or not the future is awaited.
    pub fn kill(&self, session_id: &str) -> impl Future<Output = ()> + Send + 'static + use<> {
        let exited = {
            let mut state = self.inner.lock();
            if state.running.contains_key(session_id) {
                let (done, exited) = oneshot::channel();
                state.exit_waiters.entry(session_id.to_owned()).or_default().push(done);
                Inner::send(
                    &state,
                    &ClientMessage::Kill {
                        session_id: session_id.into(),
                    },
                );
                Some(exited)
            } else {
                None
            }
        };
        async move {
            if let Some(exited) = exited {
                let _ = tokio::time::timeout(KILL_WAIT, exited).await;
            }
        }
    }

    /// Kills every live session of a project without waiting (HOST-125).
    pub fn kill_project(&self, project_id: &str) {
        let ids: Vec<String> = {
            let state = self.inner.lock();
            state
                .running
                .iter()
                .filter(|(_, r)| r.project_id == project_id)
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in ids {
            drop(self.kill(&id));
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn order(&self) -> MutexGuard<'_, ()> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// HOST-127: a message sent while no host is connected is dropped.
    fn send(state: &State, msg: &ClientMessage) {
        if let Some(host) = &state.host {
            host.write(encode(msg));
        }
    }

    fn emit(&self, event: Event) {
        self.wiring.events.emit(event);
    }

    // -----------------------------------------------------------------------
    // Connecting

    /// HOST-094: a running host first; another version's is offered to be
    /// stopped; then, with `start`, a new one.
    async fn connect(self: &Arc<Self>, start: bool) -> Result<(), ClientError> {
        let paths = self.paths.clone();
        if let Ok(stream) = nodus_host::transport::connect(&paths.socket).await {
            match self.greet(stream).await? {
                (true, _) => return Ok(()),
                (false, other) => self.replace_host(&other).await?,
            }
        }
        if !start {
            return Ok(());
        }
        let stream = self.start_and_connect().await?;
        match self.greet(stream).await? {
            (true, _) => Ok(()),
            (false, _) => Err(ClientError::Failed(OTHER_PROTOCOL.into())),
        }
    }

    /// Says hello (HOST-096–100). True once adopted; false, with the
    /// connection left open, for a host on another protocol; an error when
    /// what answered isn't this user's host or didn't answer at all.
    async fn greet(self: &Arc<Self>, stream: nodus_host::transport::ClientStream) -> Result<(bool, Conn), ClientError> {
        let (greeted, greeting) = oneshot::channel();
        let reader: Arc<dyn Reader> = Arc::clone(self) as Arc<dyn Reader>;
        let conn = connection::open(self.next_id(), stream, reader, greeted, WELCOME);
        conn.write(encode(&ClientMessage::Hello {
            protocol: f64::from(PROTOCOL),
        }));
        match greeting.await {
            Ok(Ok(adopted)) => Ok((adopted, conn)),
            Ok(Err(why)) => Err(ClientError::Failed(why)),
            Err(_) => Err(ClientError::Failed(DIDNT_ANSWER.into())),
        }
    }

    /// HOST-102–104: stops a host from an older or newer version of the
    /// app, once the user agrees.
    async fn replace_host(&self, other: &Conn) -> Result<(), ClientError> {
        match self.wiring.prompt.other_version().await {
            ReplaceChoice::Quit => {
                other.destroy();
                Err(ClientError::Quit)
            }
            ReplaceChoice::StopSessions => {
                other.write(encode(&ClientMessage::Shutdown));
                other.closed().await;
                Ok(())
            }
        }
    }

    /// HOST-091, HOST-092
    async fn start_and_connect(&self) -> Result<nodus_host::transport::ClientStream, ClientError> {
        self.wiring
            .launcher
            .start(&self.paths)
            .map_err(|err| ClientError::Failed(err.to_string()))?;
        for _ in 0..HOST_START_TRIES {
            tokio::time::sleep(HOST_START_POLL).await;
            if let Ok(stream) = nodus_host::transport::connect(&self.paths.socket).await {
                return Ok(stream);
            }
        }
        Err(ClientError::Failed(format!(
            "The session host didn't start. See {}",
            self.paths.log.display()
        )))
    }

    /// Takes on the sessions a host is already running, with what has
    /// happened in them (HOST-105–108). Called with the order lock held.
    fn adopt(&self, welcome: Welcome) {
        let config = if welcome.hook_auth_header == Some(true) {
            match write_hook_config(&self.paths, &welcome.hook_token) {
                Ok(()) => Some(self.paths.hook_config.to_string_lossy().into_owned()),
                Err(err) => {
                    tracing::error!("[host] could not write the hook config {err}");
                    None
                }
            }
        } else {
            None
        };
        let port = welcome.hook_port as u16;
        self.wiring.endpoint.set(port, &welcome.hook_token, config.as_deref());
        self.lock().relays_mcp = welcome.mcp == Some(true);
        let status = &self.wiring.status;
        for session in welcome.sessions {
            self.track(
                &session.session_id,
                &session.project_id,
                session.initial,
                session.buffer,
            );
            // Replaying them through the status machine brings its status up to date.
            for entry in &session.history {
                match entry {
                    HistoryEntry::Hook { hook, payload, at } => status.hook(hook, &session.session_id, payload, *at),
                    HistoryEntry::Input { input, at } => status.user_input(&session.session_id, input, *at),
                }
            }
            if let Some(line) = &session.status_line {
                status.hook(STATUS_LINE_EVENT, &session.session_id, &line.payload, line.at);
            }
        }
    }

    /// Tracks a session as live, before any of its output, and starts its
    /// status. Called with the order lock held.
    fn track(&self, session_id: &str, project_id: &str, initial: SessionStatus, buffer: String) {
        self.lock().running.insert(
            session_id.to_owned(),
            Running {
                project_id: project_id.to_owned(),
                output: ReplayBuffer::from_text(buffer),
                pending: String::new(),
                flush: None,
            },
        );
        self.wiring.status.process_started(session_id, initial);
    }

    // -----------------------------------------------------------------------
    // Messages from the host

    /// HOST-101, HOST-114–120, HOST-126. Called with the order lock held.
    fn handle(&self, msg: &Value) {
        let Ok(msg) = serde_json::from_value::<HostMessage>(msg.clone()) else {
            return;
        };
        match msg {
            HostMessage::Spawned { id, error } => {
                let pending = id
                    .as_ref()
                    .and_then(Value::as_f64)
                    .and_then(|id| self.lock().pending_spawns.remove(&(id as u64)));
                let Some(pending) = pending else { return };
                match error.filter(|error| !error.is_empty()) {
                    Some(error) => {
                        let _ = pending.done.send(Err(ClientError::Failed(error)));
                    }
                    None => {
                        // Before any output, which may follow in the same read.
                        self.track(&pending.session_id, &pending.project_id, pending.initial, String::new());
                        let _ = pending.done.send(Ok(()));
                    }
                }
            }
            HostMessage::Data { session_id, data } => self.append(&session_id, data),
            HostMessage::Exit { session_id, exit_code } => {
                // 0.9.4 passes an absent code on; the window doesn't read it.
                let code = exit_code.map_or(0, |code| code as i32);
                self.exited(&session_id, code);
            }
            HostMessage::Hook {
                session_id,
                event,
                payload,
            } => {
                self.wiring.status.hook(&event, &session_id, &payload, None);
            }
            HostMessage::Mcp {
                id,
                session_id,
                message,
            } => {
                // Some tools answer once they're done. No reply tells the host the app couldn't answer.
                let inner = self.me();
                tokio::spawn(async move {
                    let reply = match inner.wiring.board_tools.answer(&session_id, message).await {
                        Ok(reply) => reply,
                        Err(err) => {
                            tracing::error!("[board-tools] {err}");
                            None
                        }
                    };
                    Inner::send(&inner.lock(), &ClientMessage::McpReply { id, reply });
                });
            }
            HostMessage::Welcome(_) => {}
        }
    }

    fn append(&self, session_id: &str, data: String) {
        let schedule = {
            let mut state = self.lock();
            let Some(running) = state.running.get_mut(session_id) else {
                return;
            };
            running.pending.push_str(&data);
            running.output.push(data);
            if running.flush.is_some() {
                None
            } else {
                let timer = self.next_id();
                running.flush = Some(timer);
                Some(timer)
            }
        };
        if let Some(timer) = schedule {
            let (inner, session_id) = (self.me(), session_id.to_owned());
            tokio::spawn(async move {
                tokio::time::sleep(FLUSH).await;
                let _order = inner.order();
                inner.flush_timer(&session_id, timer);
            });
        }
    }

    /// The client, for a task to hold.
    fn me(&self) -> Arc<Self> {
        self.me.upgrade().expect("the client is alive while it reads")
    }

    /// HOST-118: the timer fired; everything pending goes out as one string.
    fn flush_timer(&self, session_id: &str, timer: u64) {
        let pending = {
            let mut state = self.lock();
            match state.running.get_mut(session_id) {
                Some(running) if running.flush == Some(timer) => {
                    running.flush = None;
                    std::mem::take(&mut running.pending)
                }
                _ => return,
            }
        };
        if !pending.is_empty() {
            self.emit(Event::PtyData {
                session_id: session_id.to_owned(),
                data: pending,
            });
        }
    }

    /// Sends anything pending now. Called with the order lock held.
    fn flush(&self, session_id: &str) {
        let pending = {
            let mut state = self.lock();
            let Some(running) = state.running.get_mut(session_id) else {
                return;
            };
            running.flush = None;
            std::mem::take(&mut running.pending)
        };
        if !pending.is_empty() {
            self.emit(Event::PtyData {
                session_id: session_id.to_owned(),
                data: pending,
            });
        }
    }

    /// HOST-120: pending output, the status machine, the exit, then the
    /// kills waiting on it. Called with the order lock held.
    fn exited(&self, session_id: &str, exit_code: i32) {
        self.flush(session_id);
        let (was_running, waiters) = {
            let mut state = self.lock();
            let was_running = state.running.shift_remove(session_id).is_some();
            (was_running, state.exit_waiters.remove(session_id).unwrap_or_default())
        };
        if was_running {
            self.wiring.status.process_exited(session_id);
            self.emit(Event::PtyExit {
                session_id: session_id.to_owned(),
                exit_code,
            });
        }
        for waiter in waiters {
            let _ = waiter.send(());
        }
    }

    /// HOST-109: the host went away, and its sessions with it. Called with
    /// the order lock held.
    fn lost(&self, conn: &Conn) {
        let ids = {
            let mut state = self.lock();
            if state.host.as_ref().is_none_or(|host| host.id != conn.id) {
                return;
            }
            state.host = None;
            for (_, pending) in state.pending_spawns.drain() {
                let _ = pending.done.send(Err(ClientError::Failed(STOPPED.into())));
            }
            if state.quitting {
                return;
            }
            state.running.keys().cloned().collect::<Vec<_>>()
        };
        for session_id in ids {
            self.exited(&session_id, -1);
        }
    }
}

impl Reader for Inner {
    /// HOST-098–100: trust first, then the protocol, then adoption, before
    /// anything that follows in the same read.
    fn first(&self, conn: &Conn, msg: &Value, greeting: oneshot::Sender<Greeting>) -> bool {
        let welcome = msg.get("type").and_then(Value::as_str) == Some("welcome");
        let key = if welcome { msg.get("key") } else { None };
        if !trusts_host(key, &self.paths, self.platform) {
            let _ = greeting.send(Err(UNTRUSTED.into()));
            return false;
        }
        if !welcome || msg.get("protocol").and_then(Value::as_f64) != Some(f64::from(PROTOCOL)) {
            let _ = greeting.send(Ok(false));
            return true;
        }
        let Ok(HostMessage::Welcome(welcome)) = serde_json::from_value::<HostMessage>(msg.clone()) else {
            let _ = greeting.send(Err(DIDNT_ANSWER.into()));
            return false;
        };
        let _order = self.order();
        self.lock().host = Some(conn.clone());
        self.adopt(welcome);
        let _ = greeting.send(Ok(true));
        true
    }

    fn message(&self, _conn: &Conn, msg: &Value) {
        let _order = self.order();
        self.handle(msg);
    }

    fn closed(&self, conn: &Conn, greeting: Option<oneshot::Sender<Greeting>>) {
        if let Some(greeting) = greeting {
            let _ = greeting.send(Err(DIDNT_ANSWER.into()));
        }
        let _order = self.order();
        self.lost(conn);
    }
}
