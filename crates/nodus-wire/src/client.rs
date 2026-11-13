//! A client over the frames: connects to a server's WebSocket with a token,
//! makes calls, and hands out the events the server sends it. The server's
//! command line and its tests use it; the app's client builds timeouts and
//! reconnecting on top (SRV-028).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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

use crate::{CallError, CallFrame, Frame, HELLO_PATH, Hello, MAX_FRAME, PORT, Request, SOCKET_PATH};

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
}

/// How the connection ended: the close frame's code and reason, when the
/// server sent one (1009 for a frame over the cap, 1013 for a client that
/// fell behind).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closed {
    pub code: Option<u16>,
    pub reason: String,
}

/// The server's events, in the order it sent them.
pub struct Events(mpsc::UnboundedReceiver<Event>);

impl Events {
    /// The next event; none once the connection has closed and every event
    /// before that has been taken.
    pub async fn next(&mut self) -> Option<Event> {
        self.0.recv().await
    }

    /// The next event that has arrived, without waiting. Once a call has
    /// been answered, every event the server sent before the answer is here.
    pub fn try_next(&mut self) -> Option<Event> {
        self.0.try_recv().ok()
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
    let authority = authority(address)?;
    let mut stream = TcpStream::connect(&authority).await?;
    let request = format!("GET {HELLO_PATH} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await?;
    let answer = String::from_utf8_lossy(&answer);
    let not_http = || ConnectError::Protocol("The server's answer wasn't HTTP".into());
    let (head, body) = answer.split_once("\r\n\r\n").ok_or_else(not_http)?;
    let status: u16 = head.get(9..12).and_then(|s| s.parse().ok()).ok_or_else(not_http)?;
    if status != 200 {
        return Err(ConnectError::Refused(status));
    }
    nodus_domain::json::from_str(body).map_err(|err| ConnectError::Protocol(err.to_string()))
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
        let (ended, closed) = watch::channel(None);
        tokio::spawn(read(stream, Arc::clone(&waiting), events, ended));
        let client = Self {
            outbox,
            waiting,
            next_id: AtomicU64::new(1),
            closed,
        };
        Ok((client, Events(received)))
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
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (answered, answer) = oneshot::channel();
        {
            let mut waiting = lock(&self.waiting);
            if !waiting.open {
                return Err(CallError::new(call, DISCONNECTED));
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
            return Err(CallError::new(call, DISCONNECTED));
        }
        answer.await.unwrap_or_else(|_| Err(CallError::new(call, DISCONNECTED)))
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
    events: mpsc::UnboundedSender<Event>,
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
