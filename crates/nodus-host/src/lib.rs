//! The session host: a background process that owns the sessions'
//! pseudo-terminals, so they keep running after the app quits.
//!
//! It keeps each session's recent output and everything that moved its
//! status, so a relaunched app gets them all back. It also serves the hook
//! endpoint every claude process posts to, and relays the board tools' MCP
//! requests to the app, since both have to outlive the app as the sessions
//! do. It exits once it has no sessions and no app connected.
//!
//! Ports `src/main/host.ts`.
