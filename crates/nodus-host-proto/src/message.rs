//! The messages between the app (pty.ts) and the session host (host.ts).
//!
//! Fields are in the order the TypeScript writes them, since JSON.stringify
//! keeps insertion order and the wire is compared byte for byte; optional
//! fields are left out when unset, as JSON.stringify leaves out `undefined`.
//! Numbers are `f64`, as JavaScript's are, and are written through
//! `nodus_domain::json` so whole ones come out without a fraction. Payloads
//! the host only forwards (hooks, MCP requests and replies, status lines)
//! stay `serde_json::Value`, in their original key order.

use nodus_domain::SessionStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hooks and the keystrokes status depends on, in the order they happened,
/// with when. A hook entry is told apart by its `hook` key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HistoryEntry {
    Hook {
        hook: String,
        /// The hook's payload, slimmed to its short scalar fields (HOST-067).
        #[serde(default)]
        payload: Value,
        /// Milliseconds since the epoch; `None` when an entry has none, which
        /// the status machine reads as now (`at = Date.now()`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
    },
    Input {
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
    },
}

/// Claude's latest status line report, and when it came.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusLine {
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
}

/// A live session as the host hands it to an app in its welcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSession {
    pub session_id: String,
    pub project_id: String,
    /// Status when the process started.
    pub initial: SessionStatus,
    /// Recent output, for replaying into a terminal.
    #[serde(default)]
    pub buffer: String,
    /// Everything that has moved its status since the process started, oldest first.
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// Claude's latest status line report. Optional, as an older host doesn't
    /// keep one; that needs no new protocol, and running sessions carry on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_line: Option<StatusLine>,
}

/// `spawn`: start a session's process in a new pseudo-terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnRequest {
    /// The request id the `spawned` reply carries back.
    pub id: f64,
    pub session_id: String,
    pub project_id: String,
    pub initial: SessionStatus,
    pub file: String,
    pub args: Vec<String>,
    /// The process's whole environment, in the order it is given.
    pub env: EnvMap,
    pub cwd: String,
    /// Not rounded: the app sends `Math.max(20, cols ?? 120)`.
    pub cols: f64,
    pub rows: f64,
}

/// An environment: names to values, in the order they were given, as the
/// app sends a JavaScript object. Values are strings from any app; the host
/// turns anything else into one as JavaScript would.
pub type EnvMap = serde_json::Map<String, Value>;

/// App → host (`ClientMessage`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "hello")]
    Hello { protocol: f64 },
    #[serde(rename = "spawn")]
    Spawn(SpawnRequest),
    #[serde(rename = "write", rename_all = "camelCase")]
    Write { session_id: String, data: String },
    #[serde(rename = "resize", rename_all = "camelCase")]
    Resize { session_id: String, cols: f64, rows: f64 },
    #[serde(rename = "kill", rename_all = "camelCase")]
    Kill { session_id: String },
    /// Stops every session and the host itself.
    #[serde(rename = "shutdown")]
    Shutdown,
    /// The app's answer to a relayed MCP request; no `reply` when it had none.
    #[serde(rename = "mcp-reply")]
    McpReply {
        id: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<Value>,
    },
}

/// The host's `welcome`, the first thing it says to a connection that said hello.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub protocol: f64,
    pub hook_port: f64,
    pub hook_token: String,
    #[serde(default)]
    pub sessions: Vec<HostSession>,
    /// The host relays the board tools' MCP requests to the app. An older
    /// host doesn't, which needs no new protocol: its sessions go without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<bool>,
    /// The hook server takes the token in an Authorization header, so hook
    /// commands can keep it out of their command line, which any user on the
    /// Mac can list. An older host only reads it from the URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_auth_header: Option<bool>,
    /// On Windows, the key the host wrote to the host folder, which only the
    /// user can read. It shows the app this pipe is its own host's
    /// ([`crate::trusts_host`]), as anyone on the PC can make a pipe with that name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// Host → app (`HostMessage`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostMessage {
    #[serde(rename = "welcome")]
    Welcome(Welcome),
    #[serde(rename = "spawned")]
    Spawned {
        /// The request's id, echoed; absent only when the request had none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    #[serde(rename = "data", rename_all = "camelCase")]
    Data { session_id: String, data: String },
    #[serde(rename = "exit", rename_all = "camelCase")]
    Exit {
        session_id: String,
        /// Always a number from a POSIX host; a 0.9.4 Windows host can leave
        /// it out when the pseudo-console closed before the process was reaped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<f64>,
    },
    #[serde(rename = "hook", rename_all = "camelCase")]
    Hook {
        session_id: String,
        event: String,
        payload: Value,
    },
    /// A JSON-RPC request a session's claude process sent to the board tools,
    /// for the app to answer.
    #[serde(rename = "mcp", rename_all = "camelCase")]
    Mcp {
        id: f64,
        session_id: String,
        message: Value,
    },
}

/// One message as it goes on the wire: `JSON.stringify(msg) + '\n'`. (HOST-014)
pub fn encode<T: Serialize + ?Sized>(msg: &T) -> String {
    let mut line = nodus_domain::json::to_string(msg).expect("a message is always JSON");
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode<T: serde::de::DeserializeOwned>(line: &str) -> T {
        nodus_domain::json::from_str(line.trim_end_matches('\n')).unwrap()
    }

    /// Every app → host message, as 0.9.4 writes it (Data & formats §5).
    #[test]
    fn writes_client_messages_as_the_app_does() {
        let mut env = EnvMap::new();
        env.insert("PATH".into(), json!("/usr/bin:/bin"));
        env.insert("NODUS_SESSION".into(), json!("s1"));
        let cases = [
            (
                ClientMessage::Hello { protocol: 1.0 },
                r#"{"type":"hello","protocol":1}"#,
            ),
            (
                ClientMessage::Spawn(SpawnRequest {
                    id: 1.0,
                    session_id: "s1".into(),
                    project_id: "p1".into(),
                    initial: SessionStatus::Done,
                    file: "/usr/local/bin/claude".into(),
                    args: vec!["--session-id".into(), "s1".into()],
                    env,
                    cwd: "/tmp/repo".into(),
                    cols: 120.0,
                    rows: 36.5,
                }),
                r#"{"type":"spawn","id":1,"sessionId":"s1","projectId":"p1","initial":"done","file":"/usr/local/bin/claude","args":["--session-id","s1"],"env":{"PATH":"/usr/bin:/bin","NODUS_SESSION":"s1"},"cwd":"/tmp/repo","cols":120,"rows":36.5}"#,
            ),
            (
                ClientMessage::Write {
                    session_id: "s1".into(),
                    data: "\x1b".into(),
                },
                r#"{"type":"write","sessionId":"s1","data":"\u001b"}"#,
            ),
            (
                ClientMessage::Resize {
                    session_id: "s1".into(),
                    cols: 132.0,
                    rows: 40.0,
                },
                r#"{"type":"resize","sessionId":"s1","cols":132,"rows":40}"#,
            ),
            (
                ClientMessage::Kill {
                    session_id: "s1".into(),
                },
                r#"{"type":"kill","sessionId":"s1"}"#,
            ),
            (ClientMessage::Shutdown, r#"{"type":"shutdown"}"#),
            (
                ClientMessage::McpReply {
                    id: 3.0,
                    reply: Some(json!({"jsonrpc":"2.0","id":7,"result":{}})),
                },
                r#"{"type":"mcp-reply","id":3,"reply":{"jsonrpc":"2.0","id":7,"result":{}}}"#,
            ),
            (
                ClientMessage::McpReply { id: 4.0, reply: None },
                r#"{"type":"mcp-reply","id":4}"#,
            ),
        ];
        for (msg, wire) in cases {
            assert_eq!(encode(&msg), format!("{wire}\n"));
            assert_eq!(decode::<ClientMessage>(wire), msg, "{wire}");
        }
    }

    /// Every host → app message, as 0.9.4 writes it (Data & formats §6).
    #[test]
    fn writes_host_messages_as_the_host_does() {
        let welcome = HostMessage::Welcome(Welcome {
            protocol: 1.0,
            hook_port: 52341.0,
            hook_token: "tok".into(),
            sessions: vec![HostSession {
                session_id: "s1".into(),
                project_id: "p1".into(),
                initial: SessionStatus::Working,
                buffer: "recent output\r\n".into(),
                history: vec![
                    HistoryEntry::Hook {
                        hook: "UserPromptSubmit".into(),
                        payload: json!({"session_id":"s1","hook_event_name":"UserPromptSubmit"}),
                        at: Some(1_762_600_000_000.0),
                    },
                    HistoryEntry::Input {
                        input: "\x1b".into(),
                        at: Some(1_762_600_001_000.0),
                    },
                ],
                status_line: Some(StatusLine {
                    payload: json!({"model":{"display_name":"Model"}}),
                    at: Some(1_762_600_002_000.0),
                }),
            }],
            mcp: Some(true),
            hook_auth_header: Some(true),
            key: None,
        });
        let cases = [
            (
                welcome,
                r#"{"type":"welcome","protocol":1,"hookPort":52341,"hookToken":"tok","sessions":[{"sessionId":"s1","projectId":"p1","initial":"working","buffer":"recent output\r\n","history":[{"hook":"UserPromptSubmit","payload":{"session_id":"s1","hook_event_name":"UserPromptSubmit"},"at":1762600000000},{"input":"\u001b","at":1762600001000}],"statusLine":{"payload":{"model":{"display_name":"Model"}},"at":1762600002000}}],"mcp":true,"hookAuthHeader":true}"#,
            ),
            (
                HostMessage::Spawned {
                    id: Some(json!(1)),
                    error: None,
                },
                r#"{"type":"spawned","id":1}"#,
            ),
            (
                HostMessage::Spawned {
                    id: Some(json!(2)),
                    error: Some("posix_spawnp failed.".into()),
                },
                r#"{"type":"spawned","id":2,"error":"posix_spawnp failed."}"#,
            ),
            (
                HostMessage::Data {
                    session_id: "s1".into(),
                    data: "héllo 🎉\n".into(),
                },
                "{\"type\":\"data\",\"sessionId\":\"s1\",\"data\":\"héllo 🎉\\n\"}",
            ),
            (
                HostMessage::Exit {
                    session_id: "s1".into(),
                    exit_code: Some(0.0),
                },
                r#"{"type":"exit","sessionId":"s1","exitCode":0}"#,
            ),
            (
                HostMessage::Hook {
                    session_id: "s1".into(),
                    event: "PreToolUse".into(),
                    payload: json!({"tool_name":"Bash","tool_input":{"command":"ls"}}),
                },
                r#"{"type":"hook","sessionId":"s1","event":"PreToolUse","payload":{"tool_name":"Bash","tool_input":{"command":"ls"}}}"#,
            ),
            (
                HostMessage::Mcp {
                    id: 1.0,
                    session_id: "s1".into(),
                    message: json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_notes","arguments":{}}}),
                },
                r#"{"type":"mcp","id":1,"sessionId":"s1","message":{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_notes","arguments":{}}}}"#,
            ),
        ];
        for (msg, wire) in cases {
            assert_eq!(encode(&msg), format!("{wire}\n"));
            assert_eq!(decode::<HostMessage>(wire), msg, "{wire}");
        }
    }

    #[test]
    fn a_windows_welcome_carries_the_key_last() {
        let welcome = Welcome {
            protocol: 1.0,
            hook_port: 1.0,
            hook_token: "t".into(),
            sessions: vec![],
            mcp: Some(true),
            hook_auth_header: Some(true),
            key: Some("k".repeat(64)),
        };
        assert_eq!(
            encode(&HostMessage::Welcome(welcome)),
            format!(
                "{{\"type\":\"welcome\",\"protocol\":1,\"hookPort\":1,\"hookToken\":\"t\",\"sessions\":[],\"mcp\":true,\"hookAuthHeader\":true,\"key\":\"{}\"}}\n",
                "k".repeat(64)
            )
        );
    }

    #[test]
    fn reads_an_older_hosts_welcome() {
        // No mcp, hookAuthHeader, key or statusLine: each reads as absent.
        let welcome: HostMessage = decode(
            r#"{"type":"welcome","protocol":1,"hookPort":4000,"hookToken":"abc","sessions":[{"sessionId":"s","projectId":"p","initial":"done","buffer":"","history":[]}]}"#,
        );
        let HostMessage::Welcome(welcome) = welcome else {
            panic!("not a welcome")
        };
        assert_eq!(welcome.mcp, None);
        assert_eq!(welcome.hook_auth_header, None);
        assert_eq!(welcome.sessions[0].status_line, None);
    }

    #[test]
    fn forwards_payload_numbers_as_javascript_writes_them() {
        // JavaScript's own spellings, such as 1e+21, read and come back unchanged.
        let wire =
            r#"{"type":"hook","sessionId":"s","event":"Stop","payload":{"big":1e+21,"small":1e-7,"x":0.1,"n":-3}}"#;
        let msg: HostMessage = decode(wire);
        assert_eq!(encode(&msg), format!("{wire}\n"));
    }

    #[test]
    fn escapes_strings_as_json_stringify_does() {
        let msg = ClientMessage::Write {
            session_id: "s".into(),
            data: "\"\\\u{8}\u{c}\n\r\t\u{1}\u{1f}/\u{2028}\u{2029}\u{7f}".into(),
        };
        assert_eq!(
            encode(&msg),
            "{\"type\":\"write\",\"sessionId\":\"s\",\"data\":\"\\\"\\\\\\b\\f\\n\\r\\t\\u0001\\u001f/\u{2028}\u{2029}\u{7f}\"}\n"
        );
    }
}
