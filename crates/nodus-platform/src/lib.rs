//! The operating system seams the domains share, behind one API on every
//! platform with the macOS and Windows specifics inside.
//!
//! - [`env`]: the environment sessions start in: the user's login-shell
//!   environment, where the `claude` binary is, and the shell a terminal or a
//!   command runs in. Ports `src/main/env.ts`.
//! - [`process`]: starting child processes, the one way every crate does it.
//! - [`secrets`]: Electron's `safeStorage`, so the xAI key 0.9.4 saved still
//!   opens (the Keychain on macOS, DPAPI and `Local State` on Windows).

pub mod env;
pub mod process;
pub mod secrets;

pub use env::{ClaudeNotFound, Env, ShellCommand};
