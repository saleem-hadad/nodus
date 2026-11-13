//! A transcript's conversation, for the archived card's preview and the agent
//! viewer (`readTranscript`).

use std::fs::{self, File};
use std::path::Path;

use nodus_domain::{MessageRole, TranscriptMessage};
use serde_json::Value;

use crate::lines::read_lines;
use crate::nodeerr::fs_message;
use crate::parse::{assistant_parts, describe_tool_use, prompt_full, task_notification, time_of, user_text};
use crate::value::{field, strict_eq, truthy};

/// Only this much of the end of a transcript is read (SESS-054).
pub const TRANSCRIPT_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// And only this many of its last messages are kept (SESS-062).
pub const TRANSCRIPT_MAX_MESSAGES: usize = 500;

/// A transcript that's there but can't be opened, worded as Node's error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReadError(pub String);

/// The conversation in a transcript. `agent` reads a subagent's transcript:
/// its lines are all sidechain lines, its prompts come from the session, and
/// each tool call keeps what it acted on (SESS-053…062).
pub fn read_transcript(file: &Path, agent: bool) -> Result<Vec<TranscriptMessage>, ReadError> {
    // SESS-053
    let Ok(stat) = fs::metadata(file) else {
        return Ok(Vec::new());
    };
    let size = stat.len();
    let fd = File::open(file).map_err(|err| ReadError(fs_message(&err, "open", file)))?;

    let mut messages: Vec<TranscriptMessage> = Vec::new();
    // The assistant turn being added to, as an index into `messages`.
    let mut turn: Option<usize> = None;
    let mut last_assistant_id: Option<Value> = None;

    // SESS-054
    let start = size.saturating_sub(TRANSCRIPT_MAX_BYTES);
    read_lines(
        &fd,
        start,
        size,
        start > 0,
        |line| {
            // SESS-055
            if truthy(line.get("isSidechain")) && !agent {
                return;
            }
            let timestamp = time_of(line).unwrap_or(0.0); // SESS-061
            match line.get("type").and_then(Value::as_str) {
                Some("user") => {
                    // SESS-056/057: an agent waits on a background command, and its notification wakes it.
                    if agent && let Some(note) = task_notification(line) {
                        turn = None;
                        messages.push(TranscriptMessage {
                            role: MessageRole::User,
                            text: note,
                            timestamp,
                            tools: None,
                            calls: None,
                            notice: Some(true),
                            by: None,
                        });
                        return;
                    }
                    let Some(u) = user_text(line, agent) else {
                        return;
                    };
                    turn = None;
                    messages.push(TranscriptMessage {
                        role: MessageRole::User,
                        text: prompt_full(&u.text),
                        timestamp,
                        tools: None,
                        calls: None,
                        notice: None,
                        by: None,
                    });
                }
                Some("assistant") => {
                    // SESS-058
                    let parts = assistant_parts(line);
                    if parts.text.is_empty() && parts.tools.is_empty() {
                        return;
                    }
                    // Consecutive assistant lines (tool loops) form one turn in the preview.
                    // SESS-059: an agent works in one long turn, so there each thing it says
                    // after some calls starts a new step, keeping its notes beside its calls.
                    let has_calls = turn.is_some_and(|t| messages[t].calls.as_ref().is_some_and(|c| !c.is_empty()));
                    if agent && !parts.text.is_empty() && has_calls {
                        turn = None;
                    }
                    let t = *turn.get_or_insert_with(|| {
                        messages.push(TranscriptMessage {
                            role: MessageRole::Assistant,
                            text: String::new(),
                            timestamp,
                            tools: Some(Vec::new()),
                            calls: agent.then(Vec::new),
                            notice: None,
                            by: None,
                        });
                        messages.len() - 1
                    });
                    let id = field(line.get("message"), "id");
                    let message = &mut messages[t];
                    if !parts.text.is_empty() {
                        let same_message = truthy(id) && strict_eq(id, last_assistant_id.as_ref());
                        message.text = if message.text.is_empty() {
                            parts.text
                        } else {
                            let sep = if same_message { "\n" } else { "\n\n" };
                            format!("{}{sep}{}", message.text, parts.text)
                        };
                    }
                    last_assistant_id = id.cloned();
                    // SESS-060
                    if let Some(tools) = &mut message.tools {
                        tools.extend(parts.tools);
                    }
                    if let Some(calls) = &mut message.calls {
                        calls.extend(parts.uses.iter().map(describe_tool_use));
                    }
                }
                _ => {}
            }
        },
        None,
    );

    // SESS-060
    for m in &mut messages {
        if m.tools.as_ref().is_some_and(Vec::is_empty) {
            m.tools = None;
        }
        if m.calls.as_ref().is_some_and(Vec::is_empty) {
            m.calls = None;
        }
    }
    // SESS-062
    let keep_from = messages.len().saturating_sub(TRANSCRIPT_MAX_MESSAGES);
    Ok(messages.split_off(keep_from))
}

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)] // Helpers that build JSON lines read clearer taking values.
mod tests {
    use super::*;
    use nodus_domain::ToolCall;
    use serde_json::json;

    fn write(lines: &[Value]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(file.path(), text).unwrap();
        file
    }

    fn user(content: Value, s: u32) -> Value {
        json!({ "type": "user", "timestamp": format!("1970-01-01T00:00:{s:02}Z"), "message": { "content": content } })
    }

    fn assistant(content: Value, id: Option<&str>, s: u32) -> Value {
        json!({ "type": "assistant", "timestamp": format!("1970-01-01T00:00:{s:02}Z"), "message": { "id": id, "content": content } })
    }

    fn text(t: &str) -> Value {
        json!({ "type": "text", "text": t })
    }

    fn tool(name: &str, input: Value) -> Value {
        json!({ "type": "tool_use", "name": name, "input": input })
    }

    #[test]
    fn reads_a_missing_transcript_as_empty() {
        assert_eq!(read_transcript(Path::new("/nowhere/t.jsonl"), false), Ok(vec![]));
    }

    #[test]
    fn merges_assistant_lines_into_turns() {
        let file = write(&[
            user(json!("Fix <pasted_content>it</pasted_content>"), 1),
            assistant(json!([text("One")]), Some("m1"), 2),
            assistant(
                json!([text("Two"), tool("Bash", json!({ "command": "ls" }))]),
                Some("m1"),
                3,
            ),
            assistant(json!([text("Three")]), Some("m2"), 4),
            // A tool-only line, then one with nothing, which leaves the last id alone.
            assistant(json!([tool("mcp__nodus__list_notes", json!({}))]), None, 5),
            assistant(json!([]), Some("m9"), 6),
            assistant(json!([text("Four")]), None, 7),
            user(json!([{ "type": "tool_result" }]), 8),
            json!({ "type": "user", "isSidechain": true, "message": { "content": "side" } }),
            assistant(json!([text("Five")]), None, 9),
            user(json!("Thanks"), 10),
        ]);
        let messages = read_transcript(file.path(), false).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].text, "Fix it");
        assert_eq!(messages[0].timestamp, 1000.0);
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[1].text, "One\nTwo\n\nThree\n\nFour\n\nFive");
        assert_eq!(messages[1].timestamp, 2000.0);
        assert_eq!(
            messages[1].tools.as_deref(),
            Some(&["Bash".to_owned(), "mcp__nodus__list_notes".to_owned()][..])
        );
        assert_eq!(messages[1].calls, None);
        assert_eq!(messages[2].text, "Thanks");
        assert_eq!(messages[2].tools, None);
    }

    #[test]
    fn reads_an_agents_steps_and_notices() {
        let side = |mut v: Value| {
            v["isSidechain"] = json!(true);
            v
        };
        let mut task = user(json!("Look for dead code"), 1);
        task["origin"] = json!({ "kind": "session" });
        let file = write(&[
            side(task),
            side(assistant(
                json!([text("Looking"), tool("Grep", json!({ "pattern": "dead" }))]),
                Some("a"),
                2,
            )),
            side(assistant(
                json!([tool("Read", json!({ "file_path": "/r/a.ts" }))]),
                Some("b"),
                3,
            )),
            side(assistant(json!([text("Found it")]), Some("c"), 4)),
            side(user(
                json!("<task-notification><summary>Tests passed</summary></task-notification>"),
                5,
            )),
            side(assistant(
                json!([tool("SubagentHandback", json!({ "message": "Report" }))]),
                Some("d"),
                6,
            )),
            json!({ "type": "user", "timestamp": "nope", "isSidechain": true, "message": { "content": "Bare" } }),
        ]);
        let messages = read_transcript(file.path(), true).unwrap();
        let summary: Vec<(&str, Option<usize>, Option<bool>)> = messages
            .iter()
            .map(|m| (m.text.as_str(), m.calls.as_ref().map(Vec::len), m.notice))
            .collect();
        assert_eq!(
            summary,
            [
                ("Look for dead code", None, None),
                ("Looking", Some(2), None),
                ("Found it", None, None),
                ("Tests passed", None, Some(true)),
                ("Report", None, None),
                ("Bare", None, None),
            ]
        );
        assert_eq!(
            messages[1].calls.as_ref().unwrap()[1],
            ToolCall {
                name: "Read".into(),
                detail: Some("a.ts".into())
            }
        );
        assert_eq!(messages[5].timestamp, 0.0);
    }

    #[test]
    fn keeps_the_last_500_messages() {
        let lines: Vec<Value> = (0..600).map(|i| user(json!(format!("p{i}")), 1)).collect();
        let file = write(&lines);
        let messages = read_transcript(file.path(), false).unwrap();
        assert_eq!(messages.len(), 500);
        assert_eq!(messages[0].text, "p100");
    }

    #[cfg(unix)]
    #[test]
    fn fails_with_the_os_error_when_it_cant_be_opened() {
        use std::os::unix::fs::PermissionsExt;
        let file = write(&[user(json!("p"), 1)]);
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything.
        if File::open(file.path()).is_ok() {
            return;
        }
        let err = read_transcript(file.path(), false).unwrap_err();
        assert_eq!(
            err.0,
            format!("EACCES: permission denied, open '{}'", file.path().display())
        );
    }
}
