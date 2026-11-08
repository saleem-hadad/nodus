//! One connection to a session host: a task writing lines in order, and a
//! task reading messages and handing them to the client in order.

use std::sync::Arc;

use nodus_host::transport::ClientStream;
use nodus_host_proto::MessageReader;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, mpsc, oneshot, watch};

/// What goes to the writer.
enum Out {
    Line(String),
    /// `socket.end()`: no more writes; the host closes its side in turn.
    End,
}

/// A handle on a connection; clones share it.
#[derive(Clone)]
pub(super) struct Conn {
    pub(super) id: u64,
    out: mpsc::UnboundedSender<Out>,
    closed: watch::Receiver<bool>,
    destroy: Arc<Notify>,
}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn").field("id", &self.id).finish_non_exhaustive()
    }
}

impl Conn {
    pub(super) fn write(&self, line: String) {
        let _ = self.out.send(Out::Line(line));
    }

    /// `socket.end()`.
    pub(super) fn end(&self) {
        let _ = self.out.send(Out::End);
    }

    /// `socket.destroy()`: closed at once, nothing more read or written.
    pub(super) fn destroy(&self) {
        self.destroy.notify_one();
    }

    /// Resolves once the connection has closed, either way.
    pub(super) async fn closed(&self) {
        let mut closed = self.closed.clone();
        let _ = closed.wait_for(|closed| *closed).await;
    }
}

/// What the client does with what a connection reads.
pub(super) trait Reader: Send + Sync + 'static {
    /// The first message; false when the connection is to be destroyed.
    fn first(&self, conn: &Conn, msg: &Value, greeting: oneshot::Sender<Greeting>) -> bool;
    /// Every message after the first.
    fn message(&self, conn: &Conn, msg: &Value);
    /// The connection closed; `greeting` is still there when nothing was read.
    fn closed(&self, conn: &Conn, greeting: Option<oneshot::Sender<Greeting>>);
}

/// How a greeting ended (`greet` in pty.ts): adopted, another protocol, or
/// an error for the user.
pub(super) type Greeting = Result<bool, String>;

/// Starts serving a connection. Nothing is read until `hello` has been
/// written, so the first message is the host's answer to it.
pub(super) fn open(
    id: u64,
    stream: ClientStream,
    reader: Arc<dyn Reader>,
    greeting: oneshot::Sender<Greeting>,
    welcome_within: std::time::Duration,
) -> Conn {
    let (mut incoming, mut write) = tokio::io::split(stream);
    let (out, mut queue) = mpsc::unbounded_channel::<Out>();
    let (closed_tx, closed) = watch::channel(false);
    let destroy = Arc::new(Notify::new());
    let conn = Conn {
        id,
        out,
        closed,
        destroy: Arc::clone(&destroy),
    };

    let writing = tokio::spawn(async move {
        while let Some(item) = queue.recv().await {
            match item {
                Out::Line(line) => {
                    if write.write_all(line.as_bytes()).await.is_err() {
                        return;
                    }
                }
                Out::End => {
                    let _ = write.shutdown().await;
                    // Held open until the reader lets go.
                    std::future::pending::<()>().await;
                }
            }
        }
    });

    let serving = conn.clone();
    tokio::spawn(async move {
        let conn = serving;
        let mut messages = MessageReader::new();
        let mut greeting = Some(greeting);
        let mut buffer = vec![0u8; 64 * 1024];
        // HOST-097: a host that accepts a connection but never answers is treated as gone.
        let deadline = tokio::time::Instant::now() + welcome_within;
        let whenever = tokio::time::Instant::now() + std::time::Duration::from_secs(100 * 365 * 86_400);
        'reading: loop {
            let limit = if greeting.is_some() { deadline } else { whenever };
            let read = tokio::select! {
                () = destroy.notified() => break,
                read = tokio::time::timeout_at(limit, incoming.read(&mut buffer)) => match read {
                    Ok(read) => read,
                    Err(_) => break,
                },
            };
            let n = match read {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for msg in messages.feed(&buffer[..n]) {
                match greeting.take() {
                    Some(greeting) => {
                        if !reader.first(&conn, &msg, greeting) {
                            break 'reading;
                        }
                    }
                    None => reader.message(&conn, &msg),
                }
            }
        }
        writing.abort();
        drop(incoming);
        let _ = closed_tx.send(true);
        reader.closed(&conn, greeting);
    });
    conn
}
