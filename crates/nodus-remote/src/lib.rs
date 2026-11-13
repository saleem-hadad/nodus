//! The app's client for one Nodus server (SRV-027, SRV-028, SRV-160): the
//! connection `nodus-core` sends a server project's calls through, and
//! whose events it passes on as its own.
//!
//! A [`Remote`] keeps itself connected. It checks the server's hello first,
//! and talks only to one with this app's [`PROTOCOL`]; then it opens the
//! WebSocket with the device's token, sends `watch` for the projects it was
//! asked to watch, and hands out the server's events. When the connection
//! drops it tries again after 1, 2, 4… seconds, at most every 30, and on
//! reconnecting it sends `watch` again before anything else. Its
//! [`Status`] says how that's going, and every change of it arrives in
//! [`RemoteEvents`] among the events.
//!
//! Calls wait at most [`timeout_for`] their kind, and fail at once while
//! it isn't connected, with [`OFFLINE`]. Frames, calls and the client under
//! all this are `nodus-wire`'s.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nodus_domain::{Event, ServerState};
use nodus_wire::client::{Client, ConnectError, Events, hello};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::AbortHandle;

pub use nodus_wire::{CallError, Hello, PROTOCOL, Request};

/// What a call fails with while the server isn't connected.
pub const OFFLINE: &str = "Not connected to the server";

/// What a call fails with when the server doesn't answer in time.
pub const TIMED_OUT: &str = "The server didn't answer in time";

/// What a connection the server turned the token down for says.
pub const TOKEN_REFUSED: &str = "The server turned down this device's token";

/// How long reaching the server may take: its hello, then the WebSocket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest wait between two attempts (SRV-028).
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// How long a call waits for its answer. The calls that ask Claude
/// something in the background (a section's name, summaries, an action's
/// draft) take minutes; views may run commands; the rest are quick.
pub fn timeout_for(request: &Request) -> Duration {
    match request {
        Request::BoardNameSection { .. } | Request::SessionSummarize { .. } | Request::ActionsDraft { .. } => {
            Duration::from_secs(600)
        }
        Request::ViewsCreate { .. }
        | Request::ViewsCall { .. }
        | Request::ViewsRefresh { .. }
        | Request::ViewsAsk { .. }
        | Request::ViewsBrain { .. }
        | Request::SessionCreate { .. }
        | Request::SessionOpen { .. }
        | Request::SessionSend { .. } => Duration::from_secs(120),
        _ => Duration::from_secs(30),
    }
}

/// The wait before attempt `attempt + 1`, counting from 0: 1, 2, 4… seconds,
/// at most 30 (SRV-028).
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(5)).min(MAX_BACKOFF)
}

/// How the connection is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub state: ServerState,
    /// The server's last hello: its name, version and protocol.
    pub hello: Option<Hello>,
    /// Why it's offline, when it is.
    pub error: Option<String>,
}

/// What a [`Remote`] hands out, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteEvent {
    /// The connection's [`Status`] changed.
    Status(Status),
    /// One of the server's events.
    Event(Event),
}

/// A [`Remote`]'s events and status changes, across reconnects. None once
/// the remote has been closed.
pub struct RemoteEvents(mpsc::UnboundedReceiver<RemoteEvent>);

impl RemoteEvents {
    pub async fn next(&mut self) -> Option<RemoteEvent> {
        self.0.recv().await
    }

    /// The next one that has arrived, without waiting.
    pub fn try_next(&mut self) -> Option<RemoteEvent> {
        self.0.try_recv().ok()
    }
}

/// Why [`check`] turned a server down.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    /// Nothing answered, or not as a Nodus server does.
    #[error("Can't reach a Nodus server at {url}: {reason}")]
    Unreachable { url: String, reason: String },
    /// SRV-027: it speaks another protocol.
    #[error("This server runs Nodus {}. Update it in Settings → Servers.", .0.version)]
    ProtocolMismatch(Hello),
    #[error("{TOKEN_REFUSED}")]
    TokenRefused,
}

/// Checks that `url` is a Nodus server with this app's protocol (SRV-027)
/// that takes `token`, by connecting once. Gives its hello.
pub async fn check(url: &str, token: &str) -> Result<Hello, CheckError> {
    match attempt(url, token).await {
        Ok((hello, client, _events)) => {
            client.close();
            Ok(hello)
        }
        Err(Failed::Mismatch(hello)) => Err(CheckError::ProtocolMismatch(hello)),
        Err(Failed::Refused(_)) => Err(CheckError::TokenRefused),
        Err(Failed::Unreachable(reason, _)) => Err(CheckError::Unreachable {
            url: url.to_owned(),
            reason,
        }),
    }
}

/// One server, kept connected until it's closed or dropped.
pub struct Remote {
    shared: Arc<Shared>,
    task: AbortHandle,
}

struct Shared {
    url: String,
    token: String,
    runtime: Handle,
    /// The connection, while there is one.
    client: Mutex<Option<Arc<Client>>>,
    status: watch::Sender<Status>,
    /// The projects whose events it gets, sent again on every reconnect.
    /// Locked before `client` wherever both are.
    watched: Mutex<Vec<String>>,
    events: mpsc::UnboundedSender<RemoteEvent>,
}

impl Remote {
    /// Starts connecting to the server at `url` (`http://host:port`) with
    /// `token`, on `runtime`, and keeps at it until closed. Its status starts
    /// as [`ServerState::Connecting`].
    pub fn start(runtime: &Handle, url: &str, token: &str) -> (Self, RemoteEvents) {
        let (events, received) = mpsc::unbounded_channel();
        let (status, _) = watch::channel(Status {
            state: ServerState::Connecting,
            hello: None,
            error: None,
        });
        let shared = Arc::new(Shared {
            url: url.to_owned(),
            token: token.to_owned(),
            runtime: runtime.clone(),
            client: Mutex::new(None),
            status,
            watched: Mutex::new(Vec::new()),
            events,
        });
        let task = runtime.spawn(keep_connected(Arc::clone(&shared))).abort_handle();
        (Self { shared, task }, RemoteEvents(received))
    }

    /// The server's address, as it was given.
    pub fn url(&self) -> &str {
        &self.shared.url
    }

    pub fn status(&self) -> Status {
        self.shared.status.borrow().clone()
    }

    /// Each change of status from now on.
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.shared.status.subscribe()
    }

    pub fn is_connected(&self) -> bool {
        self.shared.client().is_some()
    }

    /// Makes a call and waits for its answer, at most [`timeout_for`] it.
    pub async fn call(&self, request: &Request) -> Result<Value, CallError> {
        self.call_within(request, timeout_for(request)).await
    }

    /// Makes a call and waits at most `within` for its answer.
    pub async fn call_within(&self, request: &Request, within: Duration) -> Result<Value, CallError> {
        let Some(client) = self.shared.client() else {
            return Err(CallError::new(request.name(), OFFLINE));
        };
        let answer = client.begin(request);
        tokio::time::timeout(within, answer)
            .await
            .unwrap_or_else(|_| Err(CallError::new(request.name(), TIMED_OUT)))
    }

    /// Makes a call and reads its answer as `T`.
    pub async fn call_as<T: DeserializeOwned>(&self, request: &Request) -> Result<T, CallError> {
        let answer = self.call(request).await?;
        serde_json::from_value(answer)
            .map_err(|err| CallError::new(request.name(), format!("The server's answer didn't fit: {err}")))
    }

    /// Sends a call without waiting for its answer, in order with every
    /// call before and after it: keys typed, resizes, a view's reports. A
    /// failure is only logged. False while it isn't connected.
    pub fn send(&self, request: &Request) -> bool {
        let Some(client) = self.shared.client() else {
            return false;
        };
        let answer = client.begin(request);
        let within = timeout_for(request);
        self.shared.runtime.spawn(async move {
            if let Ok(Err(err)) = tokio::time::timeout(within, answer).await {
                tracing::warn!("[remote] {} failed: {}", err.channel, err.message);
            }
        });
        true
    }

    /// Gets these projects' events from now on, and no others' (SRV-026),
    /// on this connection and every one after it (SRV-028).
    pub fn watch(&self, projects: Vec<String>) {
        let mut watched = lock(&self.shared.watched);
        if *watched == projects {
            return;
        }
        watched.clone_from(&projects);
        if let Some(client) = self.shared.client() {
            drop(client.begin(&Request::Watch { projects }));
        }
    }

    /// The projects it watches.
    pub fn watched(&self) -> Vec<String> {
        lock(&self.shared.watched).clone()
    }

    /// Stops: the connection closes, calls waiting fail, no more attempts.
    pub fn close(&self) {
        self.task.abort();
        if let Some(client) = lock(&self.shared.client).take() {
            client.close();
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.close();
    }
}

impl Shared {
    fn client(&self) -> Option<Arc<Client>> {
        lock(&self.client).clone()
    }

    /// Changes the status, telling the events when it changed.
    fn set(&self, status: Status) {
        let changed = self.status.send_if_modified(|current| {
            let changed = *current != status;
            if changed {
                current.clone_from(&status);
            }
            changed
        });
        if changed {
            let _ = self.events.send(RemoteEvent::Status(status));
        }
    }

    fn hello(&self) -> Option<Hello> {
        self.status.borrow().hello.clone()
    }
}

/// Why one attempt to connect failed.
enum Failed {
    /// Unreachable, with the hello when it answered one.
    Unreachable(String, Option<Hello>),
    Mismatch(Hello),
    /// The token was turned down.
    Refused(Hello),
}

/// The hello, checked, then the WebSocket.
async fn attempt(url: &str, token: &str) -> Result<(Hello, Client, Events), Failed> {
    let said = match tokio::time::timeout(CONNECT_TIMEOUT, hello(url)).await {
        Ok(Ok(said)) => said,
        Ok(Err(err)) => return Err(Failed::Unreachable(err.to_string(), None)),
        Err(_) => return Err(Failed::Unreachable(TIMED_OUT.to_owned(), None)),
    };
    if said.protocol != PROTOCOL {
        return Err(Failed::Mismatch(said));
    }
    match tokio::time::timeout(CONNECT_TIMEOUT, Client::connect(url, token)).await {
        Ok(Ok((client, events))) => Ok((said, client, events)),
        Ok(Err(ConnectError::Refused(401))) => Err(Failed::Refused(said)),
        Ok(Err(err)) => Err(Failed::Unreachable(err.to_string(), Some(said))),
        Err(_) => Err(Failed::Unreachable(TIMED_OUT.to_owned(), Some(said))),
    }
}

/// Connects, passes the events on until the connection drops, and does it
/// again after a wait, for as long as the remote is open (SRV-028).
async fn keep_connected(shared: Arc<Shared>) {
    let mut failures = 0u32;
    loop {
        let status = match attempt(&shared.url, &shared.token).await {
            Ok((said, client, events)) => {
                if connected(&shared, said.clone(), client, events).await {
                    failures = 0;
                }
                Status {
                    state: ServerState::Offline,
                    hello: Some(said),
                    error: Some(nodus_wire::client::DISCONNECTED.to_owned()),
                }
            }
            Err(Failed::Mismatch(said)) => Status {
                state: ServerState::ProtocolMismatch,
                hello: Some(said),
                error: None,
            },
            Err(Failed::Refused(said)) => Status {
                state: ServerState::Offline,
                hello: Some(said),
                error: Some(TOKEN_REFUSED.to_owned()),
            },
            Err(Failed::Unreachable(reason, said)) => Status {
                state: ServerState::Offline,
                hello: said.or_else(|| shared.hello()),
                error: Some(reason),
            },
        };
        shared.set(status);
        tokio::time::sleep(backoff(failures)).await;
        failures = failures.saturating_add(1);
    }
}

/// One connection, from `watch` to its end. False when it went before it
/// was up.
async fn connected(shared: &Shared, said: Hello, client: Client, mut events: Events) -> bool {
    let client = Arc::new(client);
    // `watch` goes first, before the client is anyone else's, so every call
    // made on this connection comes after it.
    let watching = {
        let watched = lock(&shared.watched);
        let watching = (!watched.is_empty()).then(|| {
            client.begin(&Request::Watch {
                projects: watched.clone(),
            })
        });
        *lock(&shared.client) = Some(Arc::clone(&client));
        watching
    };
    let up = match watching {
        Some(answer) => matches!(
            tokio::time::timeout(timeout_for(&Request::Watch { projects: Vec::new() }), answer).await,
            Ok(Ok(_))
        ),
        None => true,
    };
    if up {
        shared.set(Status {
            state: ServerState::Connected,
            hello: Some(said),
            error: None,
        });
        while let Some(event) = events.next().await {
            let _ = shared.events.send(RemoteEvent::Event(event));
        }
    }
    let mut current = lock(&shared.client);
    if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, &client)) {
        *current = None;
    }
    drop(current);
    client.close();
    up
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn waits_1_2_4_seconds_and_at_most_30() {
        let waits: Vec<u64> = (0..8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
    }

    #[test]
    fn gives_claudes_calls_minutes_and_the_rest_seconds() {
        let summarize = Request::SessionSummarize {
            project_id: "p".into(),
            session_ids: vec![],
            request_id: "r".into(),
            focus: None,
        };
        assert_eq!(timeout_for(&summarize), Duration::from_secs(600));
        let write = Request::SessionWrite {
            session_id: "s".into(),
            data: "x".into(),
        };
        assert_eq!(timeout_for(&write), Duration::from_secs(30));
    }

    /// A server that answers every request with this hello, and nothing else.
    async fn hello_server(protocol: u32) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let body = format!(r#"{{"name":"old","version":"0.1.0","protocol":{protocol},"needsSetup":false}}"#);
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        format!("http://{address}")
    }

    /// SRV-027
    #[tokio::test]
    async fn wont_talk_to_a_server_with_another_protocol() {
        let url = hello_server(PROTOCOL + 1).await;
        let err = check(&url, "t").await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "This server runs Nodus 0.1.0. Update it in Settings → Servers."
        );

        let (remote, mut events) = Remote::start(&Handle::current(), &url, "t");
        assert_eq!(remote.status().state, ServerState::Connecting);
        let next = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap()
            .unwrap();
        let RemoteEvent::Status(status) = next else {
            panic!("a status first: {next:?}");
        };
        assert_eq!(status.state, ServerState::ProtocolMismatch);
        assert_eq!(status.hello.unwrap().version, "0.1.0");
        let err = remote.call(&Request::ProjectsList {}).await.unwrap_err();
        assert_eq!((err.channel.as_str(), err.message.as_str()), ("projects:list", OFFLINE));
        assert!(!remote.send(&Request::ProjectsList {}));
        remote.close();
    }

    #[tokio::test]
    async fn says_why_nothing_answered() {
        // A port nothing listens on.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let err = check(&url, "t").await.unwrap_err();
        assert!(matches!(err, CheckError::Unreachable { .. }), "{err:?}");

        let (remote, mut events) = Remote::start(&Handle::current(), &url, "t");
        let next = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap();
        let Some(RemoteEvent::Status(status)) = next else {
            panic!("a status: {next:?}");
        };
        assert_eq!(status.state, ServerState::Offline);
        assert!(status.error.is_some());
        drop(remote);
        // Closed, it hands out nothing more.
        assert!(
            tokio::time::timeout(Duration::from_secs(5), events.next())
                .await
                .unwrap()
                .is_none()
        );
    }
}
