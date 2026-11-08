//! Sessions as the app sees them: its side of the session host, how claude
//! and shells are started, and what each session is doing.
//!
//! That is the connection to the host, with each live session's recent output
//! for replaying into terminals; the status machine the Claude Code hooks
//! drive (working, waiting, done); the subagents a session started; and the
//! model, context and cost its status line reports.
//!
//! Ports `src/main/pty.ts`, `src/main/hooks.ts` and `src/main/agents.ts`.
