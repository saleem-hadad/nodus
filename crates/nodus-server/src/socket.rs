//! One app's WebSocket (SRV-023, SRV-026): its calls answered by the core,
//! and the core's events it asked for, in the order the core sent them.
//!
//! Three tasks share a connection. The reader takes frames and starts each
//! call in the order they came, as 0.9.4's handlers started: up to its first
//! wait at once, the rest on a task of its own, so keys typed reach the
//! session in order. The forwarder takes the core's events and passes on
//! what this connection gets; `watch`, `session:open` and `session:close`
//! go through it, so what they change applies from exactly their place in
//! the stream. The writer sends frames out, one queue for all.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use nodus_domain::Event;
use nodus_wire::{Answer, CALLS, CallError, CallFrame, Frame, NOT_AVAILABLE, Notice, Request, SETUP_CALLS};
use serde_json::Value;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use crate::auth::Who;
use crate::serve::Server;
use crate::{calls, dispatch};

pub(crate) type Socket = WebSocketStream<TokioIo<Upgraded>>;

/// How many frames may wait to go out. Past that the forwarder waits for
/// the app, and once the core's events run ahead of it the app is let go.
const OUTBOX: usize = 256;
/// How long the last frames and the close get once the connection ends.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

type Answering = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

enum Outgoing {
    Text(String),
    /// The last frame.
    Close(Option<CloseFrame>),
}

/// What the reader asks of the forwarder.
enum Control {
    Watch {
        id: u64,
        projects: Vec<String>,
    },
    Open {
        id: u64,
        project_id: String,
        session_id: String,
        cols: f64,
        rows: f64,
    },
    Close {
        id: u64,
        session_id: String,
    },
    /// A session that didn't start after all: none of its output.
    Forget {
        session_id: String,
    },
}

/// Serves one connection, made with `who`'s token, until either side
/// closes it, or the server does: when its device is revoked or its member
/// removed (SRV-042), or a setup token's setup is over (SRV-060).
pub(crate) async fn serve(server: Arc<Server>, socket: Socket, who: Who) {
    let (sink, mut frames) = socket.split();
    let (outbox, outgoing) = mpsc::channel(OUTBOX);
    let writer = tokio::spawn(write(sink, outgoing));
    let (control, controls) = mpsc::unbounded_channel();
    // The forwarder, and the server, end the connection by sending why here.
    let (stop, mut stopped) = watch::channel(None);
    let connection = server.auth.connected(who.clone(), stop.clone());
    // SRV-042: members hear the server's notices; a setup token doesn't.
    let notices = (!matches!(who, Who::Setup { .. })).then(|| server.auth.notices());
    let forwarder = Forwarder {
        server: Arc::clone(&server),
        outbox: outbox.clone(),
        stop,
        control: control.downgrade(),
        watched: HashSet::new(),
        open: HashSet::new(),
    };
    let events = tokio::spawn(forwarder.run(server.nodus.subscribe(), controls, notices));

    let mut close = None;
    loop {
        let message = tokio::select! {
            message = frames.next() => message,
            _ = stopped.changed() => {
                close = stopped.borrow().clone();
                break;
            }
        };
        match message {
            Some(Ok(Message::Text(text))) => {
                if let Some(why) = receive(&server, &who, text.as_str(), &outbox, &control).await {
                    close = Some(why);
                    break;
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            // SRV-023: a frame over 4 MB closes the connection.
            Some(Err(WsError::Capacity(_))) => {
                close = Some(CloseFrame {
                    code: CloseCode::Size,
                    reason: "Frames are at most 4 MB".into(),
                });
                break;
            }
            Some(Err(_)) => break,
            // Text frames only; pings are answered as they're read.
            Some(Ok(_)) => {}
        }
    }

    // SRV-026: what it opened and watched goes with it.
    server.auth.disconnected(connection);
    events.abort();
    let _ = outbox.try_send(Outgoing::Close(close));
    drop((outbox, control));
    let writing = writer.abort_handle();
    if tokio::time::timeout(CLOSE_GRACE, writer).await.is_err() {
        writing.abort();
    }
}

/// One frame from the app. Anything but a call is for the server to ignore:
/// presence until it's relayed (SRV-180), and frames only an app reads.
/// Why to close the connection, when it's to close.
async fn receive(
    server: &Arc<Server>,
    who: &Who,
    text: &str,
    outbox: &mpsc::Sender<Outgoing>,
    control: &mpsc::UnboundedSender<Control>,
) -> Option<CloseFrame> {
    let Ok(Frame::Call(CallFrame { id, call, args })) = Frame::parse(text) else {
        return None;
    };
    // SRV-025: what stays on the app isn't answered here.
    if !CALLS.contains(&call.as_str()) {
        answer(outbox, id, Err(CallError::new(call, NOT_AVAILABLE))).await;
        return None;
    }
    // SRV-060: a setup token is for the wizard's steps 1 and 2, and only
    // while it holds.
    if matches!(who, Who::Setup { .. }) {
        if !SETUP_CALLS.contains(&call.as_str()) {
            answer(outbox, id, Err(CallError::new(call, SIGN_IN_FIRST))).await;
            return None;
        }
        if !server.auth.still_good(who) {
            answer(outbox, id, Err(CallError::new(call, SETUP_OVER))).await;
            return Some(CloseFrame {
                code: CloseCode::Policy,
                reason: SETUP_OVER.into(),
            });
        }
    }
    let request = match Request::from_call(&call, args) {
        Ok(request) => request,
        Err(err) => {
            answer(outbox, id, Err(CallError::new(call, format!("Bad arguments: {err}")))).await;
            return None;
        }
    };
    let _ = match request {
        Request::Watch { projects } => control.send(Control::Watch { id, projects }),
        Request::SessionOpen {
            project_id,
            session_id,
            cols,
            rows,
        } => control.send(Control::Open {
            id,
            project_id,
            session_id,
            cols,
            rows,
        }),
        Request::SessionClose { session_id } => control.send(Control::Close { id, session_id }),
        request => {
            let mut answering: Answering = if request.is_server_call() {
                Box::pin(calls::answer(Arc::clone(server), who.clone(), request))
            } else {
                let member = server.auth.login_of(who);
                Box::pin(dispatch::answer(server.nodus.clone(), member, request))
            };
            match poll_once(&mut answering) {
                Some(result) => answer(outbox, id, result).await,
                None => {
                    let outbox = outbox.clone();
                    tokio::spawn(async move { answer(&outbox, id, answering.await).await });
                }
            }
            Ok(())
        }
    };
    None
}

/// What a setup token's call that isn't the wizard's first steps gets.
const SIGN_IN_FIRST: &str = "Sign in to do that: a setup token only names the server and sets up GitHub";
/// What a setup token gets once it has expired or setup is over.
const SETUP_OVER: &str =
    "The setup token has expired, or setup is done: sign in, or run nodus setup-code for a new code";

/// Passes on the core's events this connection gets (SRV-026).
struct Forwarder {
    server: Arc<Server>,
    outbox: mpsc::Sender<Outgoing>,
    stop: watch::Sender<Option<CloseFrame>>,
    control: mpsc::WeakUnboundedSender<Control>,
    /// The projects whose events it gets, from its last `watch`.
    watched: HashSet<String>,
    /// The sessions whose output it gets: opened, and not closed since.
    open: HashSet<String>,
}

impl Forwarder {
    async fn run(
        mut self,
        mut events: broadcast::Receiver<Event>,
        mut controls: mpsc::UnboundedReceiver<Control>,
        mut notices: Option<broadcast::Receiver<Notice>>,
    ) {
        loop {
            let going = tokio::select! {
                biased;
                control = controls.recv() => match control {
                    Some(control) => self.control(control, &mut events).await,
                    None => false,
                },
                event = events.recv() => match event {
                    Ok(event) => self.forward(&event).await,
                    Err(RecvError::Lagged(_)) => self.fell_behind(),
                    Err(RecvError::Closed) => false,
                },
                notice = next_notice(&mut notices) => {
                    let text = Frame::Notice(notice).to_text();
                    self.outbox.send(Outgoing::Text(text)).await.is_ok()
                }
            };
            if !going {
                return;
            }
        }
    }

    /// Sends the event if this connection gets it. False once it has gone.
    async fn forward(&self, event: &Event) -> bool {
        if !self.wants(event) {
            return true;
        }
        let text = Frame::Event(event.clone()).to_text();
        self.outbox.send(Outgoing::Text(text)).await.is_ok()
    }

    /// SRV-026: a session's output only while it's open here; its status,
    /// agents, usage and exit, a board's changes, a view's events and a
    /// run's end only while its project is watched; the projects and the
    /// Claude accounts changing always. The rest is the app's own (SRV-025).
    fn wants(&self, event: &Event) -> bool {
        match event {
            Event::PtyData { session_id, .. } => self.open.contains(session_id),
            Event::PtyExit { session_id, .. } => self.open.contains(session_id) || self.watches_session(session_id),
            Event::Status { session_id, .. }
            | Event::AgentStatus { session_id, .. }
            | Event::Usage { session_id, .. } => self.watches_session(session_id),
            Event::SummarizeProgress(progress) => self.watches_session(&progress.session_id),
            Event::BoardChanged { project_id } | Event::View { project_id, .. } => self.watched.contains(project_id),
            Event::RunFinished(run) => self.watched.contains(&run.project_id),
            // SRV-105: the pool is the whole server's, so every member's.
            Event::ProjectsChanged | Event::AccountsChanged { .. } => true,
            Event::Menu(_)
            | Event::UpdateStatus(_)
            | Event::OpenCard(_)
            | Event::CaptureShown(_)
            | Event::CaptureContext(_)
            | Event::ServerStatus { .. }
            | Event::ServerNotice { .. } => false,
        }
    }

    fn watches_session(&self, session_id: &str) -> bool {
        !self.watched.is_empty()
            && self
                .server
                .session_project(session_id)
                .is_some_and(|project_id| self.watched.contains(&project_id))
    }

    /// False once the connection has gone.
    async fn control(&mut self, control: Control, events: &mut broadcast::Receiver<Event>) -> bool {
        match control {
            Control::Watch { id, projects } => {
                self.watched = projects.into_iter().collect();
                self.answer(id, Ok(Value::Null)).await
            }
            Control::Open {
                id,
                project_id,
                session_id,
                cols,
                rows,
            } => self.open(id, project_id, session_id, (cols, rows), events).await,
            Control::Close { id, session_id } => {
                self.open.remove(&session_id);
                self.answer(id, Ok(Value::Null)).await
            }
            Control::Forget { session_id } => {
                self.open.remove(&session_id);
                true
            }
        }
    }

    /// `session:open`, with its output from exactly after what it replays.
    async fn open(
        &mut self,
        id: u64,
        project_id: String,
        session_id: String,
        (cols, rows): (f64, f64),
        events: &mut broadcast::Receiver<Event>,
    ) -> bool {
        let request = Request::SessionOpen {
            project_id,
            session_id: session_id.clone(),
            cols,
            rows,
        };
        let mut opening: Answering = Box::pin(dispatch::answer(self.server.nodus.clone(), None, request));
        match poll_once(&mut opening) {
            // A live session answers at once, and its replay holds all it
            // sent before (HOST-119): its output already waiting here was
            // in it, so it isn't sent again.
            Some(result) => {
                for _ in 0..events.len() {
                    let event = match events.try_recv() {
                        Ok(event) => event,
                        Err(TryRecvError::Lagged(_)) => return self.fell_behind(),
                        Err(_) => break,
                    };
                    let replayed = matches!(&event, Event::PtyData { session_id: of, .. } if *of == session_id);
                    if !replayed && !self.forward(&event).await {
                        return false;
                    }
                }
                if result.is_ok() {
                    self.open.insert(session_id);
                }
                self.answer(id, result).await
            }
            // One that starts now has nothing to replay: all its output is new.
            None => {
                self.open.insert(session_id.clone());
                let outbox = self.outbox.clone();
                let control = self.control.clone();
                tokio::spawn(async move {
                    let result = opening.await;
                    if result.is_err()
                        && let Some(control) = control.upgrade()
                    {
                        let _ = control.send(Control::Forget { session_id });
                    }
                    answer(&outbox, id, result).await;
                });
                true
            }
        }
    }

    /// The app fell so far behind that the core's events ran past it. A
    /// fresh start beats a terminal with a hole in it: the app reconnects,
    /// reloads its boards and replays its terminals (SRV-028).
    fn fell_behind(&self) -> bool {
        self.stop.send_replace(Some(CloseFrame {
            code: CloseCode::Again,
            reason: "Fell behind; reconnect".into(),
        }));
        false
    }

    async fn answer(&self, id: u64, result: Result<Value, CallError>) -> bool {
        answer(&self.outbox, id, result).await;
        !self.outbox.is_closed()
    }
}

async fn answer(outbox: &mpsc::Sender<Outgoing>, id: u64, result: Result<Value, CallError>) {
    let text = Frame::Answer(Answer { id, result }).to_text();
    let _ = outbox.send(Outgoing::Text(text)).await;
}

/// The server's next notice for members; never, for a socket that doesn't
/// get them. One that fell behind skips what it missed: notices aren't
/// state.
async fn next_notice(notices: &mut Option<broadcast::Receiver<Notice>>) -> Notice {
    if let Some(notices) = notices {
        loop {
            match notices.recv().await {
                Ok(notice) => return notice,
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    }
    std::future::pending().await
}

/// Sends frames until the last one, flushing whenever none are waiting.
async fn write(mut sink: SplitSink<Socket, Message>, mut outgoing: mpsc::Receiver<Outgoing>) {
    while let Some(next) = outgoing.recv().await {
        let sent = match next {
            Outgoing::Text(text) => sink.feed(Message::text(text)).await,
            Outgoing::Close(frame) => {
                let _ = sink.send(Message::Close(frame)).await;
                break;
            }
        };
        if sent.is_err() || (outgoing.is_empty() && sink.flush().await.is_err()) {
            break;
        }
    }
    let _ = sink.close().await;
}

/// Polls a future once, without a waker: its result if it's ready at once.
/// The future carries on from where it stopped when it's polled again.
fn poll_once<T>(future: &mut Pin<Box<dyn Future<Output = T> + Send>>) -> Option<T> {
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}
