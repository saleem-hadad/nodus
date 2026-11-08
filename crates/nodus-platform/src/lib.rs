//! The operating system seams the domains share, behind one API on every
//! platform with the macOS and Windows specifics inside.
//!
//! So far that's the environment sessions start in ([`env`]): the user's
//! login-shell environment, where the `claude` binary is, and the shell a
//! terminal or a command runs in. Ports `src/main/env.ts`.

pub mod env;

pub use env::{ClaudeNotFound, Env, ShellCommand};
