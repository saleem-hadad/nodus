//! A client over the frames: connects to a server's WebSocket with a token,
//! makes calls, and hands out the events the server sends it. The server's
//! command line and its tests use it; the app's client builds timeouts and
//! reconnecting on top (SRV-028).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use futures_util::{SinkExt, StreamExt};
use nodus_domain::Event;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, ClientRequestBuilder, Message};

use crate::{
    CallError, CallFrame, Frame, GITHUB_NONCE_PATH, HELLO_PATH, Hello, HttpError, MAX_FRAME, Nonce, NonceRequest,
    Notice, PORT, Request, SETUP_CLAIM_PATH, SIGN_IN_PATH, SIGN_IN_POLL_PATH, SOCKET_PATH, SetupClaim, SetupClaimed,
    SignInPoll, SignInPollRequest, SignInStart, SignInStarted,
};

/// What a call waiting when the connection goes fails with.
pub const DISCONNECTED: &str = "The connection to the server closed";

/// Why connecting failed.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("Not a server address: {0}")]
    Address(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The server answered with this HTTP status: 401 for a wrong token,
    /// 403 for a name it doesn't go by.
    #[error("The server refused the connection ({0})")]
    Refused(u16),
    #[error("{0}")]
    Protocol(String),
    /// A route answered this HTTP status, saying why.
    #[error("{message}")]
    Answered { status: u16, message: String },
}

/// How the connection ended: the close frame's code and reason, when the
/// server sent one (1009 for a frame over the cap, 1013 for a client that
/// fell behind).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closed {
    pub code: Option<u16>,
    pub reason: String,
}

/// The server's events, in the order it sent them, and its notices.
pub struct Events {
    events: mpsc::UnboundedReceiver<Event>,
    notices: mpsc::UnboundedReceiver<Notice>,
}

impl Events {
    /// The next event; none once the connection has closed and every event
    /// before that has been taken.
    pub async fn next(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// The next event that has arrived, without waiting. Once a call has
    /// been answered, every event the server sent before the answer is here.
    pub fn try_next(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The server's next notice for every member (SRV-042); none once the
    /// connection has closed.
    pub async fn next_notice(&mut self) -> Option<Notice> {
        self.notices.recv().await
    }

    /// The next notice that has arrived, without waiting.
    pub fn try_next_notice(&mut self) -> Option<Notice> {
        self.notices.try_recv().ok()
    }
}

/// A call's answer to come, from [`Client::begin`]: await it. Fails with
/// [`DISCONNECTED`] when the connection goes first.
pub struct Pending {
    call: String,
    /// None when the call never left.
    answer: Option<oneshot::Receiver<Result<Value, CallError>>>,
}

impl Pending {
    fn failed(call: &str) -> Self {
        Self {
            call: call.to_owned(),
            answer: None,
        }
    }
}

impl Future for Pending {
    type Output = Result<Value, CallError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let Some(answer) = this.answer.as_mut() else {
            return Poll::Ready(Err(CallError::new(this.call.as_str(), DISCONNECTED)));
        };
        Pin::new(answer)
            .poll(cx)
            .map(|answer| answer.unwrap_or_else(|_| Err(CallError::new(this.call.as_str(), DISCONNECTED))))
    }
}

/// One connection to a server. Calls can be made from many tasks at once.
pub struct Client {
    outbox: mpsc::UnboundedSender<Message>,
    waiting: Arc<Mutex<Waiting>>,
    next_id: AtomicU64,
    closed: watch::Receiver<Option<Closed>>,
}

/// Calls sent and not yet answered, by id, with their names.
struct Waiting {
    calls: HashMap<u64, (String, oneshot::Sender<Result<Value, CallError>>)>,
    open: bool,
}

/// `GET /api/hello` (SRV-022): the server's name, version and protocol.
pub async fn hello(address: &str) -> Result<Hello, ConnectError> {
    let answer = http(address, "GET", HELLO_PATH, None, None).await?;
    if answer.status != 200 {
        return Err(ConnectError::Refused(answer.status));
    }
    nodus_domain::json::from_str(&answer.text()).map_err(|err| ConnectError::Protocol(err.to_string()))
}

/// `POST /api/setup/claim` (SRV-060): the setup code for a setup token.
pub async fn claim_setup(address: &str, code: &str) -> Result<SetupClaimed, ConnectError> {
    let claim = SetupClaim { code: code.to_owned() };
    post(address, SETUP_CLAIM_PATH, None, &claim).await
}

/// `POST /api/github/setup/nonce` (SRV-062), with a setup or device token.
pub async fn github_nonce(address: &str, token: &str, request: &NonceRequest) -> Result<Nonce, ConnectError> {
    post(address, GITHUB_NONCE_PATH, Some(token), request).await
}

/// `POST /api/auth/device` (SRV-041). While the server has no members,
/// `setup_token` is what lets this sign-in make the first one.
pub async fn start_sign_in(
    address: &str,
    setup_token: Option<&str>,
    device_name: &str,
) -> Result<SignInStarted, ConnectError> {
    let start = SignInStart {
        device_name: device_name.to_owned(),
    };
    post(address, SIGN_IN_PATH, setup_token, &start).await
}

/// `POST /api/auth/device/poll` (SRV-041): asks once.
pub async fn poll_sign_in(address: &str, handle: &str) -> Result<SignInPoll, ConnectError> {
    let poll = SignInPollRequest {
        handle: handle.to_owned(),
    };
    let body = nodus_domain::json::to_string(&poll).map_err(|err| ConnectError::Protocol(err.to_string()))?;
    let answer = http(address, "POST", SIGN_IN_POLL_PATH, None, Some((JSON, body.as_bytes()))).await?;
    let body = answer.text();
    // Refused, denied and expired come with their own statuses, and say so.
    nodus_domain::json::from_str(&body).map_err(|_| answered(answer.status, &body))
}

/// `POST path` with a JSON body, read back as `T` when the status is 200.
async fn post<T: DeserializeOwned>(
    address: &str,
    path: &str,
    token: Option<&str>,
    body: &impl serde::Serialize,
) -> Result<T, ConnectError> {
    let body = nodus_domain::json::to_string(body).map_err(|err| ConnectError::Protocol(err.to_string()))?;
    let answer = http(address, "POST", path, token, Some((JSON, body.as_bytes()))).await?;
    let body = answer.text();
    if answer.status != 200 {
        return Err(answered(answer.status, &body));
    }
    nodus_domain::json::from_str(&body).map_err(|err| ConnectError::Protocol(err.to_string()))
}

/// A route's failure, in its own words when it gave some.
fn answered(status: u16, body: &str) -> ConnectError {
    let message = nodus_domain::json::from_str::<HttpError>(body)
        .map_or_else(|_| format!("The server answered {status}"), |err| err.message);
    ConnectError::Answered { status, message }
}

/// What a route answered: its status, headers and body. The files routes'
/// answers are handed out as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBody {
    pub status: u16,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl FileBody {
    /// A header's value, by its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body as text, for the routes that answer JSON.
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// `PUT /api/uploads` (SRV-163): keeps `bytes` on the server under a name
/// it picks; `name` goes along as a label. Fails with the HTTP status the
/// server answered (413 over [`crate::MAX_UPLOAD`], 401 for the token).
pub async fn upload(address: &str, token: &str, name: &str, bytes: &[u8]) -> Result<crate::Upload, ConnectError> {
    let target = format!("{}?name={}", crate::UPLOADS_PATH, crate::files::encode_name(name));
    let answer = http(address, "PUT", &target, Some(token), Some((OCTET_STREAM, bytes))).await?;
    if answer.status != 200 {
        return Err(ConnectError::Refused(answer.status));
    }
    serde_json::from_slice(&answer.body).map_err(|err| ConnectError::Protocol(err.to_string()))
}

/// `GET /api/files/<id>` (SRV-163): an upload, a board picture or a view's
/// page, whatever the status.
pub async fn download(address: &str, token: &str, id: &str) -> Result<FileBody, ConnectError> {
    http(address, "GET", &format!("{}{id}", crate::FILES_PATH), Some(token), None).await
}

/// What a JSON body is sent as, and an upload's bytes.
const JSON: &str = "application/json";
const OCTET_STREAM: &str = "application/octet-stream";

/// One HTTP/1.1 request on its own connection, closed after it: with the
/// token as `Authorization: Bearer` when there is one, and a body of the
/// type it's paired with.
async fn http(
    address: &str,
    method: &str,
    target: &str,
    token: Option<&str>,
    body: Option<(&str, &[u8])>,
) -> Result<FileBody, ConnectError> {
    let authority = authority(address)?;
    let mut stream = TcpStream::connect(&authority).await?;
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n");
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    if let Some((content_type, bytes)) = body {
        head.push_str(&format!(
            "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
            bytes.len()
        ));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    if let Some((_, bytes)) = body {
        stream.write_all(bytes).await?;
    }
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await?;
    let not_http = || ConnectError::Protocol("The server's answer wasn't HTTP".into());
    let split = answer.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(not_http)?;
    let head = String::from_utf8_lossy(&answer[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.get(9..12))
        .and_then(|code| code.parse().ok())
        .ok_or_else(not_http)?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    Ok(FileBody {
        status,
        headers,
        body: answer[split + 4..].to_vec(),
    })
}

impl Client {
    /// Opens the WebSocket at `address` (`http://host:port`, or `host:port`;
    /// port 7420 when it has none), with `token` as `Authorization: Bearer`,
    /// the only place a token goes (SRV-022).
    pub async fn connect(address: &str, token: &str) -> Result<(Self, Events), ConnectError> {
        let authority = authority(address)?;
        let uri: Uri = format!("ws://{authority}{SOCKET_PATH}")
            .parse()
            .map_err(|_| ConnectError::Address(address.to_owned()))?;
        let request = ClientRequestBuilder::new(uri).with_header("Authorization", format!("Bearer {token}"));
        let stream = TcpStream::connect(&authority).await?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let (socket, _) = tokio_tungstenite::client_async_with_config(request, stream, Some(config))
            .await
            .map_err(|err| match err {
                tungstenite::Error::Http(response) => ConnectError::Refused(response.status().as_u16()),
                tungstenite::Error::Io(err) => ConnectError::Io(err),
                err => ConnectError::Protocol(err.to_string()),
            })?;
        let (mut sink, stream) = socket.split();

        let (outbox, mut sending) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(message) = sending.recv().await {
                let closing = matches!(message, Message::Close(_));
                if sink.send(message).await.is_err() || closing {
                    break;
                }
            }
        });
        let waiting = Arc::new(Mutex::new(Waiting {
            calls: HashMap::new(),
            open: true,
        }));
        let (events, received) = mpsc::unbounded_channel();
        let (notices, noticed) = mpsc::unbounded_channel();
        let (ended, closed) = watch::channel(None);
        tokio::spawn(read(stream, Arc::clone(&waiting), (events, notices), ended));
        let client = Self {
            outbox,
            waiting,
            next_id: AtomicU64::new(1),
            closed,
        };
        Ok((
            client,
            Events {
                events: received,
                notices: noticed,
            },
        ))
    }

    /// Makes a call and waits for its answer.
    pub async fn call(&self, request: &Request) -> Result<Value, CallError> {
        self.call_named(request.name(), request.args()).await
    }

    /// Makes a call and reads its answer as `T`.
    pub async fn call_as<T: DeserializeOwned>(&self, request: &Request) -> Result<T, CallError> {
        let answer = self.call(request).await?;
        serde_json::from_value(answer)
            .map_err(|err| CallError::new(request.name(), format!("The server's answer didn't fit: {err}")))
    }

    /// Makes a call by any name, with any arguments, as a client that
    /// doesn't know the server's calls would.
    pub async fn call_named(&self, call: &str, args: Value) -> Result<Value, CallError> {
        self.begin_named(call, args).await
    }

    /// Sends a call now, after every call sent before it and before any
    /// sent after, and gives its answer to come. Keys typed into a session
    /// go this way, so they reach it in order without waiting on each
    /// other. Dropping the answer leaves the call made.
    pub fn begin(&self, request: &Request) -> Pending {
        self.begin_named(request.name(), request.args())
    }

    /// [`Client::begin`] by any name, with any arguments.
    pub fn begin_named(&self, call: &str, args: Value) -> Pending {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (answered, answer) = oneshot::channel();
        {
            let mut waiting = lock(&self.waiting);
            if !waiting.open {
                return Pending::failed(call);
            }
            waiting.calls.insert(id, (call.to_owned(), answered));
        }
        let frame = Frame::Call(CallFrame {
            id,
            call: call.to_owned(),
            args,
        });
        if !self.send_text(frame.to_text()) {
            lock(&self.waiting).calls.remove(&id);
            return Pending::failed(call);
        }
        Pending {
            call: call.to_owned(),
            answer: Some(answer),
        }
    }

    /// Sends a text frame as it is; false once the connection has gone.
    pub fn send_text(&self, text: String) -> bool {
        self.outbox.send(Message::text(text)).is_ok()
    }

    /// Closes the connection. Calls still waiting fail with [`DISCONNECTED`].
    pub fn close(&self) {
        let _ = self.outbox.send(Message::Close(None));
    }

    /// Waits until the connection has closed, from either side.
    pub async fn closed(&self) -> Closed {
        let mut closed = self.closed.clone();
        match closed.wait_for(Option::is_some).await {
            Ok(ended) => ended.clone().unwrap_or_default(),
            Err(_) => Closed::default(),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

/// Reads frames until the connection closes: answers to the calls waiting
/// for them, events to [`Events`]. Then every call still waiting fails.
async fn read(
    mut stream: futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>,
    waiting: Arc<Mutex<Waiting>>,
    (events, notices): (mpsc::UnboundedSender<Event>, mpsc::UnboundedSender<Notice>),
    ended: watch::Sender<Option<Closed>>,
) {
    let mut closed = Closed::default();
    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Text(text)) => match Frame::parse(text.as_str()) {
                Ok(Frame::Answer(answer)) => {
                    let waiter = lock(&waiting).calls.remove(&answer.id);
                    if let Some((_, answered)) = waiter {
                        let _ = answered.send(answer.result);
                    }
                }
                Ok(Frame::Event(event)) => {
                    let _ = events.send(event);
                }
                Ok(Frame::Notice(notice)) => {
                    let _ = notices.send(notice);
                }
                // A newer server's events, and frames only a server reads.
                _ => {}
            },
            Ok(Message::Close(frame)) => {
                if let Some(frame) = frame {
                    closed = Closed {
                        code: Some(frame.code.into()),
                        reason: frame.reason.to_string(),
                    };
                }
                break;
            }
            Ok(_) => {}
            Err(err) => {
                closed.reason = err.to_string();
                break;
            }
        }
    }
    let calls = {
        let mut waiting = lock(&waiting);
        waiting.open = false;
        std::mem::take(&mut waiting.calls)
    };
    for (_, (call, answered)) in calls {
        let _ = answered.send(Err(CallError::new(call, DISCONNECTED)));
    }
    ended.send_replace(Some(closed));
}

/// `host:port` from `http://host:port/`, `host:port` or `host`.
fn authority(address: &str) -> Result<String, ConnectError> {
    let rest = address.strip_prefix("http://").unwrap_or(address);
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.contains("://") || rest.contains('/') {
        return Err(ConnectError::Address(address.to_owned()));
    }
    // A port, unless the colon belongs to an IPv6 address in brackets.
    let has_port = rest.rsplit_once(':').is_some_and(|(_, port)| !port.contains(']'));
    Ok(if has_port {
        rest.to_owned()
    } else {
        format!("{rest}:{PORT}")
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_host_and_port_in_an_address() {
        assert_eq!(authority("http://nodus:7420").unwrap(), "nodus:7420");
        assert_eq!(authority("http://100.64.1.2:8000/").unwrap(), "100.64.1.2:8000");
        assert_eq!(authority("127.0.0.1:1234").unwrap(), "127.0.0.1:1234");
        assert_eq!(authority("nodus").unwrap(), "nodus:7420");
        assert_eq!(authority("[::1]").unwrap(), "[::1]:7420");
        assert_eq!(authority("[::1]:80").unwrap(), "[::1]:80");
        // No TLS in v1 (SRV-021), and only the server's root.
        for wrong in ["", "https://nodus", "http://nodus/api"] {
            assert!(authority(wrong).is_err(), "{wrong:?}");
        }
    }
}
