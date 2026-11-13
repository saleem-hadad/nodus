//! What a Nodus server and the apps connected to it say to each other
//! (SRV-022…027): the routes, the frames on the WebSocket, the calls a server
//! answers and their arguments, and the server's hello. Events are
//! `nodus_domain::Event`, whose serde form is the wire's.
//!
//! Plain data with no I/O, so the server and the app's client share it. The
//! `client` feature adds a client over the frames, on tokio ([`client`]).
//!
//! # The WebSocket
//!
//! JSON text frames, written as JavaScript writes them
//! (`nodus_domain::json`), none over [`MAX_FRAME`] bytes:
//!
//! ```text
//! client → server   { "id": 7, "call": "board:load", "args": { "projectId": "…" } }
//! server → client   { "id": 7, "ok": … }
//!                   { "id": 7, "err": { "channel": "board:load", "message": "Unknown project …" } }
//! server → client   { "ev": "pty-data", "p": { "sessionId": "…", "data": "…" } }
//! client → server   { "presence": { … } }   (SRV-180, from slice 6)
//! ```

mod call;
#[cfg(feature = "client")]
pub mod client;
pub mod files;
mod frame;

use serde::{Deserialize, Serialize};

pub use call::{CALLS, NOT_AVAILABLE, Request};
pub use files::{FILES_PATH, MAX_UPLOAD, UPLOADS_PATH, Upload};
pub use frame::{Answer, CallError, CallFrame, Frame, FrameError};

/// What the wire speaks. A client and a server with different protocols
/// don't talk (SRV-027).
pub const PROTOCOL: u32 = 1;

/// The port a server listens on (SRV-020).
pub const PORT: u16 = 7420;

/// The largest frame, in bytes: a bigger one closes the connection (SRV-023).
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// `GET`: the server's [`Hello`], with no token.
pub const HELLO_PATH: &str = "/api/hello";

/// `GET`, upgraded: the WebSocket, with the token as `Authorization: Bearer`.
pub const SOCKET_PATH: &str = "/api/ws";

/// What `GET /api/hello` answers (SRV-022).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    /// The server's name, shown in the app.
    pub name: String,
    /// The Nodus version it runs.
    pub version: String,
    /// Its [`PROTOCOL`].
    pub protocol: u32,
    /// Whether it waits for its setup code to be claimed (SRV-060).
    pub needs_setup: bool,
}
