//! Every session's terminal for the window (registry.ts's module state): the
//! `nodus-terminal` registry, fed by one subscription to the core's output
//! events for all sessions (TERM-008), and the core's `sessions.*` commands
//! behind the registry's `Sessions` side.
//!
//! Ordering (registry.ts's header): `sessions.open` replies with the output
//! the host buffered up to that moment, and anything said before the reply
//! is in it. Electron's in-order IPC gave that for free; here the reply and
//! the events take different roads, so the pump puts them back in order:
//!
//! - an open of a live session is answered at once by the core, so the pump
//!   calls it itself, lets the events already queued before that moment go
//!   through first (dropped as the open is in flight, TERM-009), then hands
//!   the reply over;
//! - from then until the attach that asked has applied the reply, the
//!   session's later events are held, then written in order (TERM-010);
//! - an open that has to start the process holds the session's events from
//!   the moment it's asked (there are none before the process starts).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context as TaskContext, Poll, Waker};

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Task};
use nodus_core::Nodus;
use nodus_domain::Event;
use nodus_terminal::registry::{OpenFuture, Sessions};
use nodus_terminal::{ContainerId, Platform, Registry};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::shell::window::Core;

/// What the store tells the views showing a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalsEvent {
    /// Output was written to the session's terminal (registry.ts's `onOutput`).
    Output(SharedString),
    /// Something a drawing shows changed (output dropped or exit line written too).
    Changed(SharedString),
}

/// The window's terminals.
pub struct Terminals {
    pub registry: Arc<Registry>,
    pub nodus: Nodus,
    next_container: u64,
    releases: mpsc::UnboundedSender<String>,
    _pump: Task<()>,
}

impl EventEmitter<TerminalsEvent> for Terminals {}

struct TerminalsGlobal(Entity<Terminals>);

impl Global for TerminalsGlobal {}

/// The core's tokio runtime, set at start-up: opening a session starts
/// work on it (the host connection, the spawn), and the registry's timers
/// (the 30 s limit for starting sessions) run there.
pub struct CoreRuntime(pub tokio::runtime::Handle);

impl Global for CoreRuntime {}

/// The core's runtime, or (in tests, with none set) one of our own.
pub fn core_runtime(cx: &App) -> tokio::runtime::Handle {
    cx.try_global::<CoreRuntime>()
        .map_or_else(fallback_runtime, |r| r.0.clone())
}

fn fallback_runtime() -> tokio::runtime::Handle {
    static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("the terminal timers' runtime");
            let handle = runtime.handle().clone();
            std::thread::Builder::new()
                .name("nodus-terminal-timers".into())
                .spawn(move || runtime.block_on(std::future::pending::<()>()))
                .expect("the terminal timers' thread");
            handle
        })
        .clone()
}

/// An open asked of the core, and where its answer goes.
struct OpenRequest {
    project_id: String,
    session_id: String,
    cols: usize,
    rows: usize,
    reply: oneshot::Sender<Result<String, String>>,
}

/// The registry's session side: the core's `sessions.*`.
struct CoreSessions {
    nodus: Nodus,
    opens: mpsc::UnboundedSender<OpenRequest>,
}

impl Sessions for CoreSessions {
    fn open(&self, project_id: &str, session_id: &str, cols: usize, rows: usize) -> OpenFuture {
        let (reply, answer) = oneshot::channel();
        let _ = self.opens.send(OpenRequest {
            project_id: project_id.to_owned(),
            session_id: session_id.to_owned(),
            cols,
            rows,
            reply,
        });
        Box::pin(async move { answer.await.unwrap_or_else(|_| Err("The window closed".to_owned())) })
    }

    fn write(&self, session_id: &str, data: &str) {
        self.nodus.sessions().write(session_id, data);
    }

    fn resize(&self, session_id: &str, cols: usize, rows: usize) {
        self.nodus.sessions().resize(session_id, cols as f64, rows as f64);
    }
}

type OpenResultFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// The core's `session:open`, as the message a rejected IPC call carried.
fn core_open(nodus: Nodus, request: &OpenRequest) -> OpenResultFuture {
    let (project, session, cols, rows) = (
        request.project_id.clone(),
        request.session_id.clone(),
        request.cols as f64,
        request.rows as f64,
    );
    Box::pin(async move {
        nodus
            .sessions()
            .open(&project, &session, cols, rows)
            .await
            .map(|r| r.replay)
            .map_err(|e| e.ipc_message())
    })
}

/// Polls a future once, without waiting.
fn poll_once<T>(future: &mut Pin<Box<dyn Future<Output = T> + Send>>) -> Option<T> {
    let mut cx = TaskContext::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

impl Terminals {
    /// The window's terminals, made on first use.
    pub fn global(cx: &mut App) -> Entity<Terminals> {
        if let Some(global) = cx.try_global::<TerminalsGlobal>() {
            return global.0.clone();
        }
        let nodus = cx.global::<Core>().0.clone();
        let entity = cx.new(|cx| Terminals::new(nodus, cx));
        cx.set_global(TerminalsGlobal(entity.clone()));
        entity
    }

    /// The window's terminals, if any session has one yet.
    pub fn try_global(cx: &App) -> Option<Entity<Terminals>> {
        cx.try_global::<TerminalsGlobal>().map(|g| g.0.clone())
    }

    fn new(nodus: Nodus, cx: &mut Context<'_, Self>) -> Self {
        let (opens_tx, opens) = mpsc::unbounded_channel();
        let (releases_tx, releases) = mpsc::unbounded_channel();
        let runtime = core_runtime(cx);
        let sessions = Arc::new(CoreSessions {
            nodus: nodus.clone(),
            opens: opens_tx,
        });
        let registry = Registry::new(sessions, Platform::current(), runtime.clone());
        let events = nodus.subscribe();
        let pump = Pump {
            registry: registry.clone(),
            nodus: nodus.clone(),
            held: HashMap::new(),
            runtime,
        };
        let task = cx.spawn(async move |this, cx| pump.run(events, opens, releases, this, cx).await);
        Self {
            registry,
            nodus,
            next_container: 1,
            releases: releases_tx,
            _pump: task,
        }
    }

    /// A container id no other host uses.
    pub fn new_container(&mut self) -> ContainerId {
        let id = ContainerId(self.next_container);
        self.next_container += 1;
        id
    }

    /// The attach that asked for an open has applied its reply: the
    /// session's held events can go in.
    pub fn release(&self, session_id: &str) {
        let _ = self.releases.send(session_id.to_owned());
    }

    /// Destroys a session's terminal (TERM-027).
    pub fn dispose(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        self.registry.dispose(session_id);
        cx.emit(TerminalsEvent::Changed(SharedString::from(session_id.to_owned())));
    }
}

/// `TERM-027` from anywhere: a card removed, a run finished, a view gone.
pub fn dispose(session_id: &str, cx: &mut App) {
    if let Some(terminals) = Terminals::try_global(cx) {
        terminals.update(cx, |t, cx| t.dispose(session_id, cx));
    }
}

/// The event pump.
struct Pump {
    registry: Arc<Registry>,
    nodus: Nodus,
    /// Sessions whose events wait for their open's reply to be applied.
    held: HashMap<String, Vec<Event>>,
    runtime: tokio::runtime::Handle,
}

/// What finished while the pump waited.
enum Wake {
    Event(Event),
    Lagged,
    Closed,
    Open(OpenRequest),
    Opened(oneshot::Sender<Result<String, String>>, Result<String, String>),
    Release(String),
}

impl Pump {
    async fn run(
        mut self,
        mut events: broadcast::Receiver<Event>,
        mut opens: mpsc::UnboundedReceiver<OpenRequest>,
        mut releases: mpsc::UnboundedReceiver<String>,
        this: gpui::WeakEntity<Terminals>,
        cx: &mut gpui::AsyncApp,
    ) {
        let (done_tx, mut done) = mpsc::unbounded_channel();
        loop {
            let wake = tokio::select! {
                biased;
                Some(id) = releases.recv() => Wake::Release(id),
                Some((reply, result)) = done.recv() => Wake::Opened(reply, result),
                Some(request) = opens.recv() => Wake::Open(request),
                event = events.recv() => match event {
                    Ok(event) => Wake::Event(event),
                    Err(broadcast::error::RecvError::Lagged(_)) => Wake::Lagged,
                    Err(broadcast::error::RecvError::Closed) => Wake::Closed,
                },
            };
            let mut touched = Touched::default();
            match wake {
                Wake::Closed => break,
                Wake::Lagged => tracing::warn!("terminal output fell behind; some was lost"),
                Wake::Event(event) => {
                    self.event(event, &mut touched);
                    // Everything else already here, in one go.
                    while let Ok(event) = events.try_recv() {
                        self.event(event, &mut touched);
                    }
                }
                Wake::Open(request) => {
                    let mut open = core_open(self.nodus.clone(), &request);
                    let entered = self.runtime.enter();
                    let session = request.session_id.clone();
                    let polled = poll_once(&mut open);
                    drop(entered);
                    match polled {
                        Some(result) => {
                            // A live session: the replay covers what's queued.
                            for _ in 0..events.len() {
                                match events.try_recv() {
                                    Ok(event) => self.event(event, &mut touched),
                                    Err(_) => break,
                                }
                            }
                            self.held.entry(session).or_default();
                            let _ = request.reply.send(result);
                        }
                        None => {
                            self.held.entry(session).or_default();
                            let done = done_tx.clone();
                            let reply = request.reply;
                            let running = self.runtime.spawn(open);
                            cx.spawn(async move |_| {
                                let result = running
                                    .await
                                    .unwrap_or_else(|_| Err("The session couldn't be opened".to_owned()));
                                let _ = done.send((reply, result));
                            })
                            .detach();
                        }
                    }
                }
                Wake::Opened(reply, result) => {
                    let _ = reply.send(result);
                }
                Wake::Release(session) => {
                    if let Some(events) = self.held.remove(&session) {
                        for event in events {
                            self.apply(event, &mut touched);
                        }
                    }
                }
            }
            if touched.is_empty() {
                continue;
            }
            let ok = this.update(cx, |_, cx| {
                for id in touched.output {
                    cx.emit(TerminalsEvent::Output(id.into()));
                }
                for id in touched.changed {
                    cx.emit(TerminalsEvent::Changed(id.into()));
                }
            });
            if ok.is_err() {
                break;
            }
        }
    }

    fn event(&mut self, event: Event, touched: &mut Touched) {
        let session = match &event {
            Event::PtyData { session_id, .. } | Event::PtyExit { session_id, .. } => session_id.clone(),
            _ => return,
        };
        if let Some(held) = self.held.get_mut(&session) {
            held.push(event);
            return;
        }
        self.apply(event, touched);
    }

    fn apply(&mut self, event: Event, touched: &mut Touched) {
        let before = |id: &str| self.registry.with_terminal(id, |t| t.generation());
        match event {
            Event::PtyData { session_id, data } => {
                let was = before(&session_id);
                self.registry.pty_data(&session_id, &data);
                // Written only when the terminal was ready (TERM-009…TERM-011).
                if was.is_some() && was != before(&session_id) && !data.is_empty() {
                    touched.output(session_id);
                }
            }
            Event::PtyExit { session_id, .. } => {
                let was = before(&session_id);
                self.registry.pty_exit(&session_id);
                if was != before(&session_id) {
                    touched.changed(session_id);
                }
            }
            _ => {}
        }
    }
}

/// The sessions a batch of events changed.
#[derive(Default)]
struct Touched {
    output: Vec<String>,
    changed: Vec<String>,
}

impl Touched {
    fn is_empty(&self) -> bool {
        self.output.is_empty() && self.changed.is_empty()
    }

    fn output(&mut self, id: String) {
        if !self.output.contains(&id) {
            self.output.push(id);
        }
    }

    fn changed(&mut self, id: String) {
        if !self.changed.contains(&id) {
            self.changed.push(id);
        }
    }
}
