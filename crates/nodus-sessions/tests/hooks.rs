//! The status machine, usage and hook commands: hooks.test.ts ported, and
//! replays of what the real hooks.ts did (fixtures/generate.mjs).

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::{advance, fixture};
use nodus_domain::{Event, RecordingSink, SessionStatus, SessionUsage, json};
use nodus_sessions::hooks::{HookEndpoint, StatusMachine, TokioClock};
use nodus_sessions::usage::to_usage;
use serde_json::{Value, json};

/// What hooks.test.ts's beforeEach sets up: a machine whose listeners record
/// what they're told, and whose `hasPendingWork` says what the test wants.
struct Harness {
    machine: StatusMachine,
    sink: Arc<RecordingSink>,
    pending: Arc<AtomicBool>,
    pending_asked: Arc<Mutex<Vec<String>>>,
}

impl Harness {
    fn new() -> Self {
        let sink = Arc::new(RecordingSink::default());
        let pending = Arc::new(AtomicBool::new(false));
        let pending_asked = Arc::new(Mutex::new(Vec::new()));
        let machine = StatusMachine::new(sink.clone(), Arc::new(TokioClock::starting_at(1_700_000_000_000.0)), {
            let pending = pending.clone();
            let asked = pending_asked.clone();
            move |file: &str| {
                asked.lock().unwrap().push(file.to_owned());
                pending.load(Ordering::SeqCst)
            }
        });
        Self {
            machine,
            sink,
            pending,
            pending_asked,
        }
    }

    /// A new live session, as hooks.test.ts's newSession.
    fn new_session(&self, id: &str) -> String {
        self.machine.process_started(id, Done);
        id.to_owned()
    }

    fn hook(&self, event: &str, id: &str, payload: &Value) {
        self.machine.receive_hook(event, id, payload, None);
    }

    fn statuses(&self, id: &str) -> Vec<SessionStatus> {
        self.changes(id).into_iter().map(|(status, _)| status).collect()
    }

    fn changes(&self, id: &str) -> Vec<(SessionStatus, bool)> {
        self.sink
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Status {
                    session_id,
                    status,
                    live,
                } if session_id == id => Some((status, live)),
                _ => None,
            })
            .collect()
    }

    fn agent_changes(&self) -> Vec<(String, String, SessionStatus)> {
        self.sink
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::AgentStatus {
                    session_id,
                    agent_id,
                    status,
                } => Some((session_id, agent_id, status)),
                _ => None,
            })
            .collect()
    }

    fn usages(&self) -> Vec<(String, SessionUsage)> {
        self.sink
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Usage { session_id, usage } => Some((session_id, usage)),
                _ => None,
            })
            .collect()
    }
}

use SessionStatus::{Done, Waiting, Working};

// --- session status --------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn session_status_is_working_through_a_turn_and_done_when_it_stops() {
    // HOST-141, HOST-142, HOST-146, HOST-134
    let h = Harness::new();
    let id = h.new_session("s1");
    assert_eq!(h.machine.status(&id), Done);
    h.hook("UserPromptSubmit", &id, &json!({}));
    assert_eq!(h.machine.status(&id), Working);
    h.hook("PreToolUse", &id, &json!({"tool_name": "Bash"}));
    h.hook("PostToolUse", &id, &json!({}));
    assert_eq!(h.machine.status(&id), Working);
    h.hook("Stop", &id, &json!({}));
    assert_eq!(h.machine.status(&id), Done);
    // Only changes are reported.
    assert_eq!(h.statuses(&id), vec![Done, Working, Done]);
}

#[tokio::test(start_paused = true)]
async fn session_status_ends_a_turn_that_failed_on_an_api_error() {
    // HOST-146: a server's sessions report `StopFailure` (SRV-103) in place of Stop.
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("UserPromptSubmit", &id, &json!({}));
    h.hook("StopFailure", &id, &json!({ "error": "rate_limit" }));
    assert_eq!(h.statuses(&id), vec![Done, Working, Done]);
}

#[tokio::test(start_paused = true)]
async fn session_status_waits_for_the_user_on_a_permission_prompt_or_a_question() {
    // HOST-142, HOST-143, HOST-144, HOST-145
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("PermissionRequest", &id, &json!({}));
    assert_eq!(h.machine.status(&id), Waiting);
    h.hook("PostToolUse", &id, &json!({}));
    h.hook("PreToolUse", &id, &json!({"tool_name": "AskUserQuestion"}));
    assert_eq!(h.machine.status(&id), Waiting);
    h.hook("Notification", &id, &json!({"notification_type": "permission_prompt"}));
    assert_eq!(h.machine.status(&id), Waiting);
}

#[tokio::test(start_paused = true)]
async fn session_status_ignores_notifications_that_are_not_about_permission() {
    // HOST-144
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook(
        "Notification",
        &id,
        &json!({"notification_type": "idle_prompt", "message": "Claude is waiting"}),
    );
    assert_eq!(h.machine.status(&id), Done);
}

#[tokio::test(start_paused = true)]
async fn session_status_goes_back_to_working_when_the_user_answers_a_prompt() {
    // HOST-164
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("PermissionRequest", &id, &json!({}));
    h.machine.note_user_input(&id, "1", None);
    assert_eq!(h.machine.status(&id), Working);
}

#[tokio::test(start_paused = true)]
async fn session_status_ends_the_turn_on_an_interrupt_that_no_hook_follows() {
    // HOST-163
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("UserPromptSubmit", &id, &json!({}));
    h.machine.note_user_input(&id, "\x1b", None);
    assert_eq!(h.machine.status(&id), Working);
    advance(1200).await;
    assert_eq!(h.machine.status(&id), Done);
}

#[tokio::test(start_paused = true)]
async fn session_status_keeps_working_when_a_hook_follows_the_interrupt() {
    // HOST-163
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("UserPromptSubmit", &id, &json!({}));
    h.machine.note_user_input(&id, "\x03", None);
    advance(100).await;
    h.hook("PostToolUse", &id, &json!({}));
    advance(1200).await;
    assert_eq!(h.machine.status(&id), Working);
}

#[tokio::test(start_paused = true)]
async fn session_status_reads_as_done_once_the_process_is_gone() {
    // HOST-136
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("UserPromptSubmit", &id, &json!({}));
    h.machine.process_exited(&id);
    assert_eq!(h.machine.status(&id), Done);
    assert_eq!(h.changes(&id).last(), Some(&(Done, false)));
}

#[tokio::test(start_paused = true)]
async fn session_status_reports_the_process_ending_on_session_end_but_not_on_clear() {
    // HOST-148
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("UserPromptSubmit", &id, &json!({}));
    h.hook("SessionEnd", &id, &json!({"reason": "clear"}));
    assert_eq!(h.changes(&id).last(), Some(&(Done, true)));
    h.hook("SessionEnd", &id, &json!({"reason": "logout"}));
    assert_eq!(h.changes(&id).last(), Some(&(Done, false)));
}

#[tokio::test(start_paused = true)]
async fn session_status_ignores_events_it_does_not_know() {
    // HOST-138
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("SomethingNew", &id, &json!({}));
    assert_eq!(h.machine.status(&id), Done);
}

// --- agents ----------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn agents_keep_the_card_working_after_the_main_thread_stops() {
    // HOST-133, HOST-150, HOST-152, HOST-154
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("UserPromptSubmit", &id, &json!({}));
    h.hook("SubagentStart", &id, &json!({"agent_id": "a1"}));
    h.hook("Stop", &id, &json!({}));
    assert_eq!(h.machine.status(&id), Working);
    h.hook("SubagentStop", &id, &json!({"agent_id": "a1"}));
    assert_eq!(h.machine.status(&id), Done);
    assert_eq!(
        h.agent_changes(),
        vec![(id.clone(), "a1".into(), Working), (id.clone(), "a1".into(), Done)]
    );
}

#[tokio::test(start_paused = true)]
async fn agents_stay_working_after_stopping_with_background_work_that_will_wake_them() {
    // HOST-150
    let h = Harness::new();
    let id = h.new_session("s1");
    h.pending.store(true, Ordering::SeqCst);
    h.hook("SubagentStart", &id, &json!({"agent_id": "a1"}));
    h.hook(
        "SubagentStop",
        &id,
        &json!({"agent_id": "a1", "agent_transcript_path": "/tmp/agent.jsonl"}),
    );
    assert_eq!(*h.pending_asked.lock().unwrap(), vec!["/tmp/agent.jsonl".to_owned()]);
    assert_eq!(h.machine.status(&id), Working);
}

#[tokio::test(start_paused = true)]
async fn agents_make_the_card_wait_while_one_waits_and_an_answer_goes_to_that_agent() {
    // HOST-151, HOST-162
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook("SubagentStart", &id, &json!({"agent_id": "a1"}));
    h.hook("PermissionRequest", &id, &json!({"agent_id": "a1"}));
    assert_eq!(h.machine.status(&id), Waiting);
    h.machine.note_user_input(&id, "\r", None);
    assert_eq!(h.machine.status(&id), Working);
    assert_eq!(h.agent_changes().last(), Some(&(id.clone(), "a1".into(), Working)));
}

// --- status line -----------------------------------------------------------

fn report(extra: &Value) -> Value {
    let mut report = json!({
        "model": {"display_name": "Opus 5.5"},
        "context_window": {
            "current_usage": {"input_tokens": 1000, "cache_creation_input_tokens": 500, "cache_read_input_tokens": 1500},
            "context_window_size": 200000
        },
        "cost": {"total_cost_usd": 0.1234, "total_duration_ms": 5000}
    });
    for (key, value) in extra.as_object().unwrap() {
        report[key] = value.clone();
    }
    report
}

#[tokio::test(start_paused = true)]
async fn status_line_turns_a_report_into_usage() {
    // HOST-166 to HOST-170
    let h = Harness::new();
    let id = h.new_session("s1");
    h.machine
        .receive_hook("StatusLine", &id, &report(&json!({})), Some(1000.0));
    assert_eq!(
        h.usages(),
        vec![(
            id.clone(),
            SessionUsage {
                model: Some("Opus 5.5".into()),
                context_tokens: Some(3000.0),
                context_size: Some(200_000.0),
                context_percent: Some(1.5),
                cost_usd: Some(0.1234),
                duration_ms: Some(5000.0),
                at: 1000.0,
            }
        )]
    );
}

#[tokio::test(start_paused = true)]
async fn status_line_prefers_the_percentage_claude_gives() {
    // HOST-169
    let h = Harness::new();
    let id = h.new_session("s1");
    h.hook(
        "StatusLine",
        &id,
        &report(&json!({"context_window": {"used_percentage": 42, "context_window_size": 200000}})),
    );
    let usage = &h.usages()[0].1;
    assert_eq!(usage.context_percent, Some(42.0));
    assert_eq!(usage.context_tokens, None);
}

#[tokio::test(start_paused = true)]
async fn status_line_does_not_resend_a_report_that_only_moved_the_clock() {
    // HOST-171
    let h = Harness::new();
    let id = h.new_session("s1");
    h.machine
        .receive_hook("StatusLine", &id, &report(&json!({})), Some(1000.0));
    h.machine.receive_hook(
        "StatusLine",
        &id,
        &report(&json!({"cost": {"total_cost_usd": 0.1234, "total_duration_ms": 9000}})),
        Some(2000.0),
    );
    assert_eq!(h.usages().len(), 1);
}

// --- hook commands ---------------------------------------------------------

/// Every command in the settings: the hooks' and the status line's.
fn commands(settings: &str) -> Vec<Value> {
    let settings: Value = serde_json::from_str(settings).unwrap();
    let mut hooks: Vec<Value> = settings["hooks"]
        .as_object()
        .unwrap()
        .values()
        .map(|h| h[0]["hooks"][0].clone())
        .collect();
    hooks.push(settings["statusLine"].clone());
    hooks
}

#[test]
fn hook_commands_register_every_hook_and_the_status_line_printing_nothing() {
    // HOST-180, HOST-182
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "secret-token", Some("/tmp/hook.curlrc"));
    let settings: Value = serde_json::from_str(&endpoint.settings(false)).unwrap();
    let names: Vec<&str> = settings["hooks"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for name in ["SessionStart", "UserPromptSubmit", "PreToolUse", "Stop", "SubagentStop"] {
        assert!(names.contains(&name), "{name}");
    }
    for command in commands(&endpoint.settings(false)) {
        assert!(
            command["command"]
                .as_str()
                .unwrap()
                .ends_with(">/dev/null 2>&1; exit 0")
        );
    }
}

#[test]
fn a_servers_sessions_also_report_turns_that_fail() {
    // SRV-103: StopFailure after the others, on a server only.
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "secret-token", Some("/tmp/hook.curlrc"));
    let local: Value = serde_json::from_str(&endpoint.settings(false)).unwrap();
    assert!(local["hooks"].get("StopFailure").is_none());
    let server: Value = serde_json::from_str(&endpoint.server_settings(false)).unwrap();
    let names: Vec<&String> = server["hooks"].as_object().unwrap().keys().collect();
    assert_eq!(names.last().map(|n| n.as_str()), Some("StopFailure"));
    let command = server["hooks"]["StopFailure"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains("evt=StopFailure&sid=$NODUS_SESSION"), "{command}");
    assert!(command.contains(r#"-K "$NODUS_HOOK_CONFIG""#));
    // The rest is the same, byte for byte.
    let mut without = server.clone();
    without["hooks"].as_object_mut().unwrap().shift_remove("StopFailure");
    assert_eq!(without, local);
}

#[test]
fn hook_commands_read_the_token_from_the_config_file_when_the_host_takes_it_in_a_header() {
    // HOST-176, HOST-177
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "secret-token", Some("/tmp/hook.curlrc"));
    assert!(!endpoint.settings(false).contains("secret-token"));
    for command in commands(&endpoint.settings(false)) {
        let command = command["command"].as_str().unwrap();
        assert!(command.contains(r#"-K "$NODUS_HOOK_CONFIG""#));
        assert!(!command.contains("token="));
    }
    let env = endpoint.env("s1");
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        env,
        vec![
            ("NODUS_HOOK_CONFIG", "/tmp/hook.curlrc"),
            ("NODUS_PORT", "4000"),
            ("NODUS_SESSION", "s1"),
            ("NODUS_TOKEN", "secret-token"),
        ]
    );
}

#[test]
fn hook_commands_put_the_token_in_the_url_for_an_older_host() {
    // HOST-178
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "secret-token", None);
    assert!(!endpoint.settings(false).contains("secret-token"));
    for command in commands(&endpoint.settings(false)) {
        assert!(command["command"].as_str().unwrap().contains("token=$NODUS_TOKEN"));
    }
    assert!(!endpoint.env("s1").contains_key("NODUS_HOOK_CONFIG"));
}

#[test]
fn hook_commands_on_windows_without_git_bash_run_in_powershell_with_the_token_from_the_environment() {
    // HOST-179
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "secret-token", Some(r"C:\Users\me\hook.curlrc"));
    let settings = endpoint.settings(true);
    let hooks = commands(&settings);
    assert_eq!(hooks.len(), 11);
    for hook in hooks {
        assert_eq!(hook["shell"], "powershell");
        let command = hook["command"].as_str().unwrap();
        assert!(command.contains(r#""Bearer $env:NODUS_TOKEN""#));
        assert!(command.ends_with("exit 0"));
    }
    assert!(!settings.contains("secret-token"));
}

#[test]
fn settings_match_the_typescript_byte_for_byte() {
    // HOST-177 to HOST-181: the three variants as hooks.ts writes them.
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "synthetic-token", Some("/tmp/example/hook.curlrc"));
    assert_eq!(&*endpoint.settings(false), fixture("hook-settings-config.json"));
    assert_eq!(&*endpoint.settings(true), fixture("hook-settings-powershell.json"));
    endpoint.set(4000, "synthetic-token", None);
    assert_eq!(&*endpoint.settings(false), fixture("hook-settings-url.json"));
    // PowerShell hooks take the token from the environment whether or not there's a config.
    assert_eq!(&*endpoint.settings(true), fixture("hook-settings-powershell.json"));
    // The URL variant is also what spec 02 lists.
    assert!(
        fixture("hook-settings-url.json")
            .starts_with(r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST"#)
    );
}

#[test]
fn settings_are_built_once_per_kind() {
    // HOST-181
    let endpoint = HookEndpoint::new();
    endpoint.set(4000, "one", Some("/tmp/a"));
    let first = endpoint.settings(false);
    endpoint.set(5000, "two", Some("/tmp/b"));
    assert!(Arc::ptr_eq(&first, &endpoint.settings(false)));
    endpoint.set(5000, "two", Some(""));
    assert!(!Arc::ptr_eq(&first, &endpoint.settings(false)));
}

#[test]
fn hook_env_matches_the_typescript() {
    // HOST-174, HOST-176
    let expected: Value = serde_json::from_str(&fixture("hook-env.json")).unwrap();
    let as_json = |endpoint: &HookEndpoint| serde_json::to_value(endpoint.env("session-1")).unwrap();
    let endpoint = HookEndpoint::new();
    assert_eq!(as_json(&endpoint), expected["none"]);
    endpoint.set(4000, "synthetic-token", Some("/tmp/example/hook.curlrc"));
    assert_eq!(as_json(&endpoint), expected["config"]);
    endpoint.set(4000, "synthetic-token", None);
    assert_eq!(as_json(&endpoint), expected["url"]);
}

// --- hook commands, run by a shell -----------------------------------------

#[cfg(unix)]
mod run_by_a_shell {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};

    use super::*;

    /// What the server got: the URL, the Authorization header and the body.
    #[derive(Debug, PartialEq)]
    pub(super) struct Received {
        pub url: String,
        pub authorization: Option<String>,
        pub body: Vec<u8>,
    }

    /// Answers one request with 204, and tells what it was.
    pub(super) fn serve_once() -> (u16, std::thread::JoinHandle<Received>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let url = line.split(' ').nth(1).unwrap().to_owned();
            let mut authorization = None;
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (name, value) = header.split_once(": ").unwrap();
                match name.to_ascii_lowercase().as_str() {
                    "authorization" => authorization = Some(value.to_owned()),
                    "content-length" => length = value.parse().unwrap(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            stream.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
            Received {
                url,
                authorization,
                body,
            }
        });
        (port, server)
    }

    /// Runs the Stop hook's command the way Claude Code does, with the payload on stdin.
    fn run_stop_hook(endpoint: &HookEndpoint, session_id: &str, payload: &str) -> std::process::Output {
        let settings: Value = serde_json::from_str(&endpoint.settings(false)).unwrap();
        let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut child = Command::new("sh")
            .args(["-c", &command])
            .envs(endpoint.env(session_id))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    #[test]
    fn send_the_token_in_a_header_from_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("hook.curlrc");
        // What writeHookConfig writes (HOST-010).
        std::fs::write(&config, "header = \"Authorization: Bearer abc123\"\n").unwrap();
        let (port, server) = serve_once();
        let endpoint = HookEndpoint::new();
        endpoint.set(port, "abc123", Some(config.to_str().unwrap()));
        let output = run_stop_hook(&endpoint, "s1", r#"{"stop":true}"#);
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            server.join().unwrap(),
            Received {
                url: "/hook?evt=Stop&sid=s1".into(),
                authorization: Some("Bearer abc123".into()),
                body: br#"{"stop":true}"#.to_vec(),
            }
        );
    }

    #[test]
    fn send_the_token_in_the_url_without_a_config_file() {
        let (port, server) = serve_once();
        let endpoint = HookEndpoint::new();
        endpoint.set(port, "abc123", None);
        run_stop_hook(&endpoint, "s1", "{}");
        let received = server.join().unwrap();
        assert_eq!(received.url, "/hook?evt=Stop&sid=s1&token=abc123");
        assert_eq!(received.authorization, None);
    }

    #[test]
    fn print_nothing_and_exit_0_with_no_host() {
        // HOST-182: nothing listens on the port.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let endpoint = HookEndpoint::new();
        endpoint.set(port, "abc123", None);
        let output = run_stop_hook(&endpoint, "s1", "{}");
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
    }
}

/// hooks.test.ts: post what Claude sends on stdin, byte for byte. Needs
/// PowerShell: Windows has it, and a Mac may have PowerShell 7.
#[cfg(unix)]
#[test]
fn hook_commands_on_windows_without_git_bash_post_what_claude_sends_on_stdin_byte_for_byte() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let Some(powershell) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("pwsh"))
            .find(|candidate| candidate.is_file())
    }) else {
        return;
    };
    let (port, server) = run_by_a_shell::serve_once();
    let endpoint = HookEndpoint::new();
    endpoint.set(port, "abc123", Some("unused.curlrc"));
    let settings: Value = serde_json::from_str(&endpoint.settings(true)).unwrap();
    let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut child = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .envs(endpoint.env("s1"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = r#"{"prompt":"héllo 🎉","last_assistant_message":"done"}"#;
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    let received = server.join().unwrap();
    assert_eq!(received.url, "/hook?evt=Stop&sid=s1");
    assert_eq!(received.authorization.as_deref(), Some("Bearer abc123"));
    assert_eq!(received.body, payload.as_bytes());
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

// --- what the real hooks.ts did ----------------------------------------------

async fn replay(start: f64, steps: &[Value]) -> Vec<Value> {
    let log = Arc::new(common::Log::default());
    let pending = common::flag();
    let machine = StatusMachine::new(log.clone(), Arc::new(TokioClock::starting_at(start)), {
        let log = log.clone();
        let pending = pending.clone();
        move |file: &str| {
            log.push(json!(["pendingWork", file]));
            pending.load(Ordering::SeqCst)
        }
    });
    for step in steps {
        assert!(
            common::status_step(&machine, &log, &pending, step).await,
            "unknown step {step}"
        );
    }
    log.take()
}

#[test]
fn the_status_machine_does_what_hooks_ts_did() {
    // HOST-132 to HOST-173: every scenario in fixtures/status-scenarios.json.
    let fixture: Value = serde_json::from_str(&fixture("status-scenarios.json")).unwrap();
    let start = fixture["start"].as_f64().unwrap();
    let scenarios = fixture["scenarios"].as_array().unwrap();
    assert!(scenarios.len() >= 20, "the fixture lost its scenarios");
    for scenario in scenarios {
        let got = common::paused_runtime().block_on(replay(start, scenario["steps"].as_array().unwrap()));
        let expected = scenario["events"].as_array().unwrap();
        assert_eq!(
            common::lines(&got),
            common::lines(expected),
            "scenario: {}",
            scenario["name"]
        );
    }
}

#[test]
fn usage_reads_reports_as_hooks_ts_did() {
    // HOST-166 to HOST-170
    let fixture: Value = serde_json::from_str(&fixture("usage.json")).unwrap();
    for (i, case) in fixture.as_array().unwrap().iter().enumerate() {
        let usage = to_usage(&case["payload"], 1000.0 + i as f64);
        assert_eq!(
            json::to_string(&usage).unwrap(),
            json::to_string(&case["usage"]).unwrap(),
            "payload: {}",
            case["payload"]
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_host_client_feeds_it_through_session_input() {
    // HOST-108: a welcome session is tracked, then its history replayed with recorded times.
    use nodus_sessions::hooks::SessionInput;
    let h = Harness::new();
    let input: &dyn SessionInput = &h.machine;
    input.process_started("s1", Working);
    input.hook("PermissionRequest", "s1", &json!({}), Some(1.0));
    input.user_input("s1", "\r", Some(2.0));
    assert_eq!(h.machine.status("s1"), Working);
    input.hook("StatusLine", "s1", &json!({"model": {"id": "m"}}), Some(3.0));
    assert_eq!(h.machine.usage("s1").map(|u| u.at), Some(3.0));
    input.process_exited("s1");
    assert_eq!(
        h.changes("s1"),
        vec![(Working, true), (Waiting, true), (Working, true), (Done, false)]
    );
}
