//! The session host's wire protocol: one JSON object per line over a Unix
//! socket, or a named pipe on Windows, between the app and the session host.
//!
//! It must stay byte-compatible with the 0.9.4 host (same `PROTOCOL` number,
//! same messages, same socket and pipe names), so the Rust app adopts the
//! sessions a running Node host holds, and a Node app adopts a Rust host's.
//!
//! Ports `src/main/hostProtocol.ts`: the messages ([`message`]), reading them
//! off a socket ([`reader`]), where the host's files live and who may use
//! them ([`paths`]), and how a Windows command line is quoted ([`cmdline`]).

pub mod buffer;
pub mod cmdline;
pub mod message;
pub mod paths;
pub mod reader;

pub use buffer::{BUFFER_LIMIT, ReplayBuffer};
pub use cmdline::{windows_command_line, windows_spawn_command_line};
pub use message::{
    ClientMessage, EnvMap, HistoryEntry, HostMessage, HostSession, SpawnRequest, StatusLine, Welcome, encode,
};
pub use paths::{
    HookConfigError, HostPaths, Platform, ensure_private_dir, host_paths, trusts_host, write_hook_config,
    write_private_file,
};
pub use reader::{MessageReader, Utf8Decoder};

/// Bumped whenever a message changes shape. The app only talks to a host that
/// speaks its version. `hello`, `welcome.protocol` and `shutdown` never change,
/// so any app can tell an older host apart and stop it. (HOST-013)
///
/// Whether the first Rust release bumps it is decision U1, settled at
/// packaging; until then it stays 0.9.4's.
pub const PROTOCOL: u32 = 1;

/// The event name Claude's status line reports arrive under, beside the hooks.
/// They come after every reply, so only the latest is kept, apart from history.
pub const STATUS_LINE_EVENT: &str = "StatusLine";

/// Where a claude process reaches the board tools: on the hook server, with the
/// hook token as its bearer token and its session id in this header.
pub const MCP_PATH: &str = "/mcp";
pub const MCP_SESSION_HEADER: &str = "X-Nodus-Session";

/// The keystrokes the status heuristics act on (`noteUserInput` in hooks.ts):
/// exactly Return, Esc, Ctrl+C or a digit 1–9, as one write.
/// `STATUS_KEY = /^(?:\r|\x1b|\x03|[1-9])$/`.
pub fn is_status_key(data: &str) -> bool {
    matches!(
        data,
        "\r" | "\x1b" | "\x03" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_keys_are_single_keystrokes() {
        for key in ["\r", "\x1b", "\x03", "1", "5", "9"] {
            assert!(is_status_key(key), "{key:?}");
        }
        for other in ["0", "a", "\r\n", "\n", "12", "", "\x1b[A", "\x1b\x1b"] {
            assert!(!is_status_key(other), "{other:?}");
        }
    }
}
