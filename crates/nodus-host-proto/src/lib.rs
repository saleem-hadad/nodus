//! The session host's wire protocol: one JSON object per line over a Unix
//! socket, or a named pipe on Windows, between the app and the session host.
//!
//! It must stay byte-compatible with the 0.9.4 host (same `PROTOCOL` number,
//! same messages, same socket and pipe names), so the Rust app adopts the
//! sessions a running Node host holds, and a Node app adopts a Rust host's.
//!
//! Ports `src/main/hostProtocol.ts`.
