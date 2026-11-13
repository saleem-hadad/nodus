//! The messages between the app (pty.ts) and the session host (host.ts).
//!
//! Fields are in the order the TypeScript writes them, since JSON.stringify
//! keeps insertion order and the wire is compared byte for byte; optional
//! fields are left out when unset, as JSON.stringify leaves out `undefined`.
//! Numbers are `f64`, as JavaScript's are, and are written through
//! `nodus_domain::json` so whole ones come out without a fraction. Payloads
//! the host only forwards (hooks, MCP requests and replies, status lines)
//! stay `serde_json::Value`, in their original key order.

use std::collections::BTreeMap;

use nodus_domain::{QueuedPrompt, SessionStatus};
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
    /// The prompts waiting for its turn to end, first to go first, so a
    /// relaunched app shows them (QUEUE-004). Left out when there are
    /// none, and by a host from before the queue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<QueuedPrompt>,
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
    /// The session's own token, on a server (SRV-143): its hooks, status
    /// line, board tools and credential helper must send this one, never
    /// the host's shared token. Its hook commands find it in a curl config
    /// of their own. Left out by the app, whose sessions share the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
}

/// An environment: names to values, in the order they were given, as the
/// app sends a JavaScript object. Values are strings from any app; the host
/// turns anything else into one as JavaScript would.
pub type EnvMap = serde_json::Map<String, Value>;

/// `exec`: run a process to its end, outside any terminal, and answer with
/// what it printed (SRV-005). `nodus serve` runs every process this way, so
/// it runs as the host's user. Not in 0.9.4; a host that doesn't know it
/// ignores it (HOST-040), so it needs no new protocol, and only a runner
/// whose host comes from its own binary sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecRequest {
    /// The request id the `exec-result` carries back.
    pub id: f64,
    /// The program, a path or a name looked up on `env`'s PATH, then its
    /// arguments.
    pub argv: Vec<String>,
    /// The folder it runs in; the host's own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Exactly this environment; the host's own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    /// Written to stdin, which is then closed; closed from the start when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    /// The whole process tree is stopped after this long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<f64>,
    /// `argv[0]` stands for Claude Code's own program, which the host finds
    /// on `env`'s PATH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<bool>,
    /// What's kept of stdout: everything when absent, up to the host's cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<ExecCapture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<ExecCapture>,
    /// The signal a timeout or a `maxBuffer` sends, by number; SIGTERM when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_signal: Option<f64>,
    /// Unix: in a session of its own (`setsid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_session: Option<bool>,
}

/// What's kept of an `exec`'s output stream: `"all"`, `"ignore"`,
/// `{"first": bytes}` (the rest read and dropped) or `{"maxBuffer": bytes}`
/// (past it the process is stopped and the run fails, as Node's `maxBuffer`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecCapture {
    All,
    Ignore,
    First(f64),
    MaxBuffer(f64),
}

/// `exec-result`: how an `exec` ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub id: f64,
    /// Its exit code; absent when a signal stopped it or it never started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<f64>,
    /// The signal that stopped it, by number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<f64>,
    /// What it printed, decoded as UTF-8 with bad bytes replaced.
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    /// The timeout stopped it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    /// `"stdout"` or `"stderr"`: the stream that passed its `maxBuffer` first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<String>,
    /// Why it never started, as the host's side words it (`spawn git ENOENT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The OS's error number for that, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errno: Option<f64>,
}

/// App → host (`ClientMessage`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "hello")]
    Hello {
        protocol: f64,
        /// A server asks for sessions' own tokens and credential relays
        /// (SRV-143, SRV-084); the app leaves it out, and its welcome is
        /// as it was.
        #[serde(rename = "sessionTokens", default, skip_serializing_if = "Option::is_none")]
        session_tokens: Option<bool>,
    },
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
    /// Runs a process outside any terminal (SRV-005); answered by `exec-result`.
    #[serde(rename = "exec")]
    Exec(ExecRequest),
    /// The app gave up on an `exec`: its process tree is stopped (SIGKILL).
    #[serde(rename = "exec-cancel")]
    ExecCancel { id: f64 },
    /// Marks `path` as a folder the host's user trusts, in its `.claude.json`,
    /// as accepting Claude Code's trust dialog there does (SRV-126);
    /// answered by `trusted`. Not in 0.9.4, like `exec`.
    #[serde(rename = "trust")]
    Trust { id: f64, path: String },
    /// The answer to a `credential` request (SRV-084): what the helper
    /// prints, or none when it's refused.
    #[serde(rename = "credential-reply")]
    CredentialReply {
        id: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<Value>,
    },
    /// Replaces a live session's queued prompts (QUEUE-002), answered by a
    /// `queue` to every app. Only sent to a host whose welcome says
    /// `queue`; an older one would ignore it (HOST-040).
    #[serde(rename = "queue-set", rename_all = "camelCase")]
    QueueSet {
        session_id: String,
        items: Vec<QueuedPrompt>,
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
    /// The host keeps each session's queued prompts and hands the first to
    /// Claude when its turn ends (QUEUE-001). An older host doesn't, which
    /// needs no new protocol: the app keeps the queue and types it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<bool>,
    /// On Windows, the key the host wrote to the host folder, which only the
    /// user can read. It shows the app this pipe is its own host's
    /// ([`crate::trusts_host`]), as anyone on the PC can make a pipe with that name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The host gives sessions their own tokens and relays credential
    /// requests (SRV-143, SRV-084). A server takes no host without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_tokens: Option<bool>,
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
        /// The token it came with, when the session has one of its own
        /// (SRV-143), for the server to check as well.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// A JSON-RPC request a session's claude process sent to the board tools,
    /// for the app to answer.
    #[serde(rename = "mcp", rename_all = "camelCase")]
    Mcp {
        id: f64,
        session_id: String,
        message: Value,
        /// The token it came with, when the session has one of its own (SRV-143).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// A git credential a session's helper asked for (SRV-084), with the
    /// token it sent and the session it named, if any: the server checks
    /// both, and answers with `credential-reply`.
    #[serde(rename = "credential", rename_all = "camelCase")]
    Credential {
        id: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        token: String,
        request: Value,
    },
    /// How an `exec` ended, to the connection that sent it.
    #[serde(rename = "exec-result")]
    ExecResult(ExecResult),
    /// A `trust` done, or why it wasn't, to the connection that sent it.
    #[serde(rename = "trusted")]
    Trusted {
        id: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// A session's queued prompts changed: the app set them, or the host
    /// handed the first to Claude (QUEUE-002, QUEUE-005). To every app.
    #[serde(rename = "queue", rename_all = "camelCase")]
    Queue {
        session_id: String,
        items: Vec<QueuedPrompt>,
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
                ClientMessage::Hello {
                    protocol: 1.0,
                    session_tokens: None,
                },
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
                    session_token: None,
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
                queue: vec![],
            }],
            mcp: Some(true),
            hook_auth_header: Some(true),
            queue: None,
            key: None,
            session_tokens: None,
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
                    token: None,
                },
                r#"{"type":"hook","sessionId":"s1","event":"PreToolUse","payload":{"tool_name":"Bash","tool_input":{"command":"ls"}}}"#,
            ),
            (
                HostMessage::Mcp {
                    id: 1.0,
                    session_id: "s1".into(),
                    message: json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_notes","arguments":{}}}),
                    token: None,
                },
                r#"{"type":"mcp","id":1,"sessionId":"s1","message":{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_notes","arguments":{}}}}"#,
            ),
            // SRV-143, SRV-084: what a server's host adds.
            (
                HostMessage::Hook {
                    session_id: "s1".into(),
                    event: "Stop".into(),
                    payload: json!({}),
                    token: Some("t1".into()),
                },
                r#"{"type":"hook","sessionId":"s1","event":"Stop","payload":{},"token":"t1"}"#,
            ),
            (
                HostMessage::Credential {
                    id: 2.0,
                    session_id: Some("s1".into()),
                    token: "t1".into(),
                    request: json!({"protocol":"https","host":"github.com","path":"acme/shop.git"}),
                },
                r#"{"type":"credential","id":2,"sessionId":"s1","token":"t1","request":{"protocol":"https","host":"github.com","path":"acme/shop.git"}}"#,
            ),
        ];
        for (msg, wire) in cases {
            assert_eq!(encode(&msg), format!("{wire}\n"));
            assert_eq!(decode::<HostMessage>(wire), msg, "{wire}");
        }
    }

    /// SRV-005, SRV-126: the messages the Rust app added, which 0.9.4 never sends.
    #[test]
    fn writes_exec_and_trust_messages() {
        let exec = ExecRequest {
            id: 4.0,
            argv: vec!["git".into(), "status".into()],
            cwd: Some("/repo".into()),
            env: Some(BTreeMap::from([("PATH".into(), "/usr/bin".into())])),
            stdin: None,
            timeout_ms: Some(30_000.0),
            claude: None,
            stdout: Some(ExecCapture::MaxBuffer(1024.0)),
            stderr: Some(ExecCapture::First(0.0)),
            kill_signal: Some(9.0),
            new_session: Some(true),
        };
        let cases = [
            (
                ClientMessage::Exec(exec),
                r#"{"type":"exec","id":4,"argv":["git","status"],"cwd":"/repo","env":{"PATH":"/usr/bin"},"timeoutMs":30000,"stdout":{"maxBuffer":1024},"stderr":{"first":0},"killSignal":9,"newSession":true}"#,
            ),
            (
                ClientMessage::ExecCancel { id: 4.0 },
                r#"{"type":"exec-cancel","id":4}"#,
            ),
            (
                ClientMessage::Trust {
                    id: 5.0,
                    path: "/srv/repo".into(),
                },
                r#"{"type":"trust","id":5,"path":"/srv/repo"}"#,
            ),
        ];
        for (msg, wire) in cases {
            assert_eq!(encode(&msg), format!("{wire}\n"));
            assert_eq!(decode::<ClientMessage>(wire), msg, "{wire}");
        }
        let bare: ClientMessage =
            decode(r#"{"type":"exec","id":1,"argv":["claude","-p"],"claude":true,"stdout":"all","stderr":"ignore"}"#);
        let ClientMessage::Exec(bare) = bare else {
            panic!("an exec");
        };
        assert_eq!((bare.cwd, bare.env, bare.timeout_ms), (None, None, None));
        assert_eq!(
            (bare.stdout, bare.stderr),
            (Some(ExecCapture::All), Some(ExecCapture::Ignore))
        );

        let cases = [
            (
                HostMessage::ExecResult(ExecResult {
                    id: 4.0,
                    code: Some(0.0),
                    signal: None,
                    stdout: "## main\n".into(),
                    stderr: String::new(),
                    timed_out: None,
                    overflow: None,
                    error: None,
                    errno: None,
                }),
                r###"{"type":"exec-result","id":4,"code":0,"stdout":"## main\n","stderr":""}"###,
            ),
            (
                HostMessage::ExecResult(ExecResult {
                    id: 6.0,
                    code: None,
                    signal: Some(9.0),
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: Some(true),
                    overflow: Some("stdout".into()),
                    error: None,
                    errno: None,
                }),
                r#"{"type":"exec-result","id":6,"signal":9,"stdout":"","stderr":"","timedOut":true,"overflow":"stdout"}"#,
            ),
            (
                HostMessage::ExecResult(ExecResult {
                    id: 7.0,
                    code: None,
                    signal: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: None,
                    overflow: None,
                    error: Some("spawn git ENOENT".into()),
                    errno: Some(2.0),
                }),
                r#"{"type":"exec-result","id":7,"stdout":"","stderr":"","error":"spawn git ENOENT","errno":2}"#,
            ),
            (
                HostMessage::Trusted { id: 5.0, error: None },
                r#"{"type":"trusted","id":5}"#,
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
            queue: None,
            key: Some("k".repeat(64)),
            session_tokens: None,
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
        // QUEUE-001: nor a queue.
        assert_eq!(welcome.queue, None);
        assert!(welcome.sessions[0].queue.is_empty());
    }

    /// QUEUE-001–005: the queue's messages, which neither 0.9.4 nor an
    /// earlier Rust host sends or reads.
    #[test]
    fn writes_queue_messages() {
        let items = vec![
            QueuedPrompt {
                id: "q1".into(),
                text: "run the tests".into(),
                at: 1_762_600_000_000.0,
            },
            QueuedPrompt {
                id: "q2".into(),
                text: "then\nfix \"them\" 🎉".into(),
                at: 1_762_600_000_000.5,
            },
        ];
        let set = ClientMessage::QueueSet {
            session_id: "s1".into(),
            items: items.clone(),
        };
        let wire = r#"{"type":"queue-set","sessionId":"s1","items":[{"id":"q1","text":"run the tests","at":1762600000000},{"id":"q2","text":"then\nfix \"them\" 🎉","at":1762600000000.5}]}"#;
        assert_eq!(encode(&set), format!("{wire}\n"));
        assert_eq!(decode::<ClientMessage>(wire), set);
        let cleared = ClientMessage::QueueSet {
            session_id: "s1".into(),
            items: vec![],
        };
        assert_eq!(
            encode(&cleared),
            "{\"type\":\"queue-set\",\"sessionId\":\"s1\",\"items\":[]}\n"
        );

        let queue = HostMessage::Queue {
            session_id: "s1".into(),
            items: items[1..].to_vec(),
        };
        let wire = r#"{"type":"queue","sessionId":"s1","items":[{"id":"q2","text":"then\nfix \"them\" 🎉","at":1762600000000.5}]}"#;
        assert_eq!(encode(&queue), format!("{wire}\n"));
        assert_eq!(decode::<HostMessage>(wire), queue);

        // QUEUE-001, QUEUE-004: a new host says so, and a session's queue
        // comes last, only when there is one.
        let welcome = HostMessage::Welcome(Welcome {
            protocol: 2.0,
            hook_port: 1.0,
            hook_token: "t".into(),
            sessions: vec![
                HostSession {
                    session_id: "s1".into(),
                    project_id: "p1".into(),
                    initial: SessionStatus::Working,
                    buffer: String::new(),
                    history: vec![],
                    status_line: None,
                    queue: items[..1].to_vec(),
                },
                HostSession {
                    session_id: "s2".into(),
                    project_id: "p1".into(),
                    initial: SessionStatus::Done,
                    buffer: String::new(),
                    history: vec![],
                    status_line: None,
                    queue: vec![],
                },
            ],
            mcp: Some(true),
            hook_auth_header: Some(true),
            queue: Some(true),
            key: None,
            session_tokens: None,
        });
        let wire = r#"{"type":"welcome","protocol":2,"hookPort":1,"hookToken":"t","sessions":[{"sessionId":"s1","projectId":"p1","initial":"working","buffer":"","history":[],"queue":[{"id":"q1","text":"run the tests","at":1762600000000}]},{"sessionId":"s2","projectId":"p1","initial":"done","buffer":"","history":[]}],"mcp":true,"hookAuthHeader":true,"queue":true}"#;
        assert_eq!(encode(&welcome), format!("{wire}\n"));
        assert_eq!(decode::<HostMessage>(wire), welcome);
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
