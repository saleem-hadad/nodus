//! The session host's wire protocol: one JSON object per line over a Unix
//! socket, or a named pipe on Windows, between the app and the session host.
//!
//! The messages, and the socket and pipe names, are byte-compatible with the
//! 0.9.4 host. Only the [`PROTOCOL`] number differs: 0.9.4 says 1 and this
//! says 2 (decision U1), so an app that finds the other kind of host running
//! asks to stop its sessions instead of adopting them.
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
    ClientMessage, EnvMap, ExecCapture, ExecRequest, ExecResult, HistoryEntry, HostMessage, HostSession, SpawnRequest,
    StatusLine, Welcome, encode,
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
/// 2 since the first Rust release (decision U1, default (a)). 0.9.4's Node
/// host says 1 and sends the same messages, but it starts every session
/// through node-pty's `spawn-helper` inside the installed bundle, which the
/// update replaces, so new sessions on it would fail with `posix_spawnp
/// failed.` A Rust app that finds it running treats it as another version
/// and asks to stop its sessions (HOST-102), which resume from their cards;
/// a 0.9.4 app that finds a Rust host does the same.
pub const PROTOCOL: u32 = 2;

/// What the 0.9.4 Node host and app say in `hello` and `welcome`.
pub const PROTOCOL_0_9_4: u32 = 1;

/// The event name Claude's status line reports arrive under, beside the hooks.
/// They come after every reply, so only the latest is kept, apart from history.
pub const STATUS_LINE_EVENT: &str = "StatusLine";

/// Where a claude process reaches the board tools: on the hook server, with the
/// hook token as its bearer token and its session id in this header.
pub const MCP_PATH: &str = "/mcp";
pub const MCP_SESSION_HEADER: &str = "X-Nodus-Session";
/// Where a session's git credential helper asks for a token, with its own
/// token as Bearer and its id in [`MCP_SESSION_HEADER`] (SRV-084).
pub const CREDENTIAL_PATH: &str = "/credential";

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

    /// U1: the first Rust release is a protocol change, so a 0.9.4 host or
    /// app is another version, whose sessions are stopped, not adopted.
    #[test]
    fn the_rust_app_speaks_another_protocol_than_0_9_4() {
        assert_eq!(PROTOCOL, 2);
        assert_eq!(PROTOCOL_0_9_4, 1);
    }

    /// HOST-013: `hello` and `welcome.protocol` keep 0.9.4's shape across the
    /// bump, so each side reads the other's number and can stop its host.
    #[test]
    fn hello_and_welcome_carry_the_number_as_0_9_4_did() {
        assert_eq!(
            encode(&ClientMessage::Hello {
                protocol: f64::from(PROTOCOL),
                session_tokens: None,
            }),
            "{\"type\":\"hello\",\"protocol\":2}\n"
        );
        let hello: ClientMessage = nodus_domain::json::from_str(r#"{"type":"hello","protocol":1}"#).unwrap();
        assert_eq!(
            hello,
            ClientMessage::Hello {
                protocol: f64::from(PROTOCOL_0_9_4),
                session_tokens: None,
            }
        );
        let welcome: HostMessage = nodus_domain::json::from_str(
            r#"{"type":"welcome","protocol":1,"hookPort":52341,"hookToken":"tok","sessions":[],"mcp":true,"hookAuthHeader":true}"#,
        )
        .unwrap();
        let HostMessage::Welcome(welcome) = welcome else {
            panic!("a welcome");
        };
        assert_eq!(welcome.protocol, f64::from(PROTOCOL_0_9_4));
    }

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
