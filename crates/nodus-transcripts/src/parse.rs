//! What a transcript line says: the prompt a user typed, Claude's reply and
//! tool calls, and when (discovery.ts's line helpers).

use std::sync::LazyLock;

use nodus_domain::ToolCall;
use nodus_domain::js::{self, slice_utf16, utf16_len};
use regex::{NoExpand, Regex};
use serde_json::Value;

use crate::date;
use crate::lines::Json;
use crate::paths::basename;
use crate::value::{as_str, field, truthy};

/// The tool a background agent calls to deliver its report; its message is the agent's last word.
pub const HANDBACK_TOOL: &str = "SubagentHandback";

/// User-message text that is tooling noise rather than something the user typed (SESS-019).
const NOISE_PREFIXES: [&str; 11] = [
    "<local-command-",
    "<system-reminder>",
    "Caveat:",
    "[Request interrupted",
    "<task-notification",
    "<bash-input>",
    "<bash-stdout>",
    "<bash-stderr>",
    "<user-prompt-submit-hook>",
    "Base directory for this skill",
    "This session is being continued from a previous conversation",
];

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static COMMAND_NAME: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<command-name>(.*?)</command-name>"));
static COMMAND_ARGS: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<command-args>(.*?)</command-args>"));
static REMINDER: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<system-reminder>.*?</system-reminder>"));
static PASTED_BLOCK: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<pasted_content[^>]*>.*?</pasted_content[^>]*>"));
static PASTED_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"</?pasted_content[^>]*>"));
static SUMMARY: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<summary>(.*?)</summary>"));

/// A prompt the user typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserText {
    pub text: String,
    /// A slash command, as `/name args`.
    pub is_command: bool,
}

/// What an assistant line says and does.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantParts {
    pub text: String,
    pub tools: Vec<String>,
    /// The tool_use blocks behind `tools`.
    pub uses: Vec<Value>,
    /// A background agent handed in its report: `text` ends with it.
    pub handback: bool,
}

/// `promptSummary`: prompt text for titles and card previews, pasted blocks
/// collapsed to a placeholder (SESS-021).
pub fn prompt_summary(text: &str) -> String {
    let collapsed = PASTED_BLOCK.replace_all(text, NoExpand("[Pasted text]"));
    PASTED_TAG.replace_all(&collapsed, "").into_owned()
}

/// `promptFull`: prompt text for the transcript preview and search, pasted
/// content kept and its tags dropped (SESS-022).
pub fn prompt_full(text: &str) -> String {
    js::trim(&PASTED_TAG.replace_all(text, "")).to_owned()
}

/// Spaces collapsed, and cut to `max` UTF-16 units with an ellipsis (SESS-023).
pub fn truncate(text: &str, max: usize) -> String {
    let flat = js::collapse_spaces(text);
    let flat = js::trim(&flat);
    if utf16_len(flat) > max {
        let cut = slice_utf16(flat, 0, max.saturating_sub(1));
        format!("{}…", js::trim_end(&cut))
    } else {
        flat.to_owned()
    }
}

/// The prompt the user typed, or none for tool results, meta and noise. In an
/// agent's transcript the prompts come from the session rather than a person,
/// so `any_origin` keeps them (SESS-014…020).
pub fn user_text(line: &Json, any_origin: bool) -> Option<UserText> {
    // SESS-014
    if ["isMeta", "isCompactSummary", "isVisibleInTranscriptOnly"]
        .iter()
        .any(|key| truthy(line.get(*key)))
    {
        return None;
    }
    // SESS-015
    let origin = field(line.get("origin"), "kind");
    if truthy(origin) && origin.and_then(Value::as_str) != Some("human") && !any_origin {
        return None;
    }

    // SESS-016
    let content = field(line.get("message"), "content");
    let text = match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => {
            if blocks.iter().any(|b| block_type(b) == Some("tool_result")) {
                return None;
            }
            block_texts(blocks).collect::<Vec<_>>().join("\n")
        }
        _ => String::new(),
    };
    // SESS-017
    let text = js::trim(&text);
    if text.is_empty() {
        return None;
    }

    // SESS-018: slash commands, before the noise check.
    if text.starts_with("<command-name>") || text.starts_with("<command-message>") {
        let capture = |re: &Regex| re.captures(text).map(|c| js::trim(&c[1]).to_owned());
        let name = capture(&COMMAND_NAME).filter(|name| !name.is_empty())?;
        let args = capture(&COMMAND_ARGS).filter(|args| !args.is_empty());
        let name = if name.starts_with('/') {
            name
        } else {
            format!("/{name}")
        };
        let text = match args {
            Some(args) => format!("{name} {args}"),
            None => name,
        };
        return Some(UserText { text, is_command: true });
    }
    // SESS-019
    if NOISE_PREFIXES.iter().any(|p| text.starts_with(p)) {
        return None;
    }
    // SESS-020
    let cleaned = REMINDER.replace_all(text, "");
    let cleaned = js::trim(&cleaned);
    (!cleaned.is_empty()).then(|| UserText {
        text: cleaned.to_owned(),
        is_command: false,
    })
}

/// `b?.type`.
fn block_type(block: &Value) -> Option<&str> {
    as_str(field(Some(block), "type"))
}

/// The `text` of each text block whose text is a string, in order.
fn block_texts(blocks: &[Value]) -> impl Iterator<Item = &str> {
    blocks
        .iter()
        .filter(|b| block_type(b) == Some("text"))
        .filter_map(|b| as_str(field(Some(b), "text")))
}

/// What an assistant line says and which tools it calls (SESS-024, SESS-025).
pub fn assistant_parts(line: &Json) -> AssistantParts {
    let empty = || AssistantParts {
        text: String::new(),
        tools: Vec::new(),
        uses: Vec::new(),
        handback: false,
    };
    let blocks = match field(line.get("message"), "content") {
        Some(Value::String(text)) => {
            return AssistantParts {
                text: js::trim(text).to_owned(),
                ..empty()
            };
        }
        Some(Value::Array(blocks)) => blocks,
        _ => return empty(),
    };
    let mut texts: Vec<&str> = block_texts(blocks).collect();
    let mut uses = Vec::new();
    let mut handback = false;
    for block in blocks {
        if block_type(block) != Some("tool_use") {
            continue;
        }
        let Some(name) = as_str(field(Some(block), "name")) else {
            continue;
        };
        // The handback is the agent's reply, not a step of its work.
        if name == HANDBACK_TOOL {
            handback = true;
            if let Some(message) = as_str(field(field(Some(block), "input"), "message")) {
                texts.push(message);
            }
        } else {
            uses.push(block.clone());
        }
    }
    let tools = uses
        .iter()
        .filter_map(|b| as_str(field(Some(b), "name")).map(str::to_owned))
        .collect();
    AssistantParts {
        text: js::trim(&texts.join("\n")).to_owned(),
        tools,
        uses,
        handback,
    }
}

/// MCP tools are named mcp__<server>__<tool>; the tool's own name reads best (SESS-026).
pub fn tool_name(name: &str) -> &str {
    if !name.starts_with("mcp__") {
        return name;
    }
    let own = name.rfind("__").map_or(name, |at| &name[at + 2..]);
    if own.is_empty() { name } else { own }
}

/// `String(use.name)` for a tool_use block whose name is a string.
fn use_name(use_block: &Value) -> &str {
    as_str(field(Some(use_block), "name")).unwrap_or("undefined")
}

/// A tool use's input when it's an object; otherwise nothing is read from it.
pub(crate) fn use_input(use_block: &Value) -> Option<&Json> {
    field(Some(use_block), "input")?.as_object()
}

/// A string field of a tool's input that has something besides spaces in it, untrimmed.
pub(crate) fn input_str<'a>(input: Option<&'a Json>, key: &str) -> Option<&'a str> {
    input?.get(key)?.as_str().filter(|s| !js::trim(s).is_empty())
}

/// A tool_use block as a name and a one-line summary of what it acted on (SESS-027).
pub fn describe_tool_use(use_block: &Value) -> ToolCall {
    let input = use_input(use_block);
    let s = |key: &str| input_str(input, key);
    let file = s("file_path").or_else(|| s("notebook_path"));
    // A file's name is taken even when it's empty, which then leaves no detail.
    let detail = s("command")
        .or_else(|| file.map(basename))
        .or_else(|| s("pattern"))
        .or_else(|| s("url"))
        .or_else(|| s("query"))
        .or_else(|| s("description"))
        .or_else(|| s("skill"))
        .or_else(|| s("path"))
        .or_else(|| s("prompt"));
    ToolCall {
        name: tool_name(use_name(use_block)).to_owned(),
        detail: detail.filter(|d| !d.is_empty()).map(|d| truncate(d, 160)),
    }
}

/// When a line was written: `Date.parse(line.timestamp)` (SESS-028).
pub fn time_of(line: &Json) -> Option<f64> {
    date::parse(line.get("timestamp")?.as_str()?)
}

/// All the text in a line's message, whether its content is a string or blocks.
pub fn message_text(line: &Json) -> String {
    match field(line.get("message"), "content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|b| as_str(field(Some(b), "text")).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Whether a user line is a task notification: a background command (or
/// task) reporting that it ended (SESS-057).
pub fn is_task_notification(line: &Json) -> bool {
    as_str(field(line.get("origin"), "kind")) == Some("task-notification")
        || message_text(line).contains("<task-notification>")
}

/// The summary of a task notification, if the line is one (SESS-057).
pub fn task_notification(line: &Json) -> Option<String> {
    if !is_task_notification(line) {
        return None;
    }
    let text = message_text(line);
    let summary = SUMMARY.captures(&text).map(|c| js::trim(&c[1]).to_owned());
    Some(
        summary
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "A background task finished".into()),
    )
}

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)] // Helpers that build JSON lines read clearer taking values.
mod tests {
    use super::*;
    use serde_json::json;

    fn line(v: Value) -> Json {
        v.as_object().unwrap().clone()
    }

    fn user(content: Value) -> Json {
        line(json!({ "type": "user", "message": { "role": "user", "content": content } }))
    }

    fn text_of(l: &Json) -> Option<(String, bool)> {
        user_text(l, false).map(|u| (u.text, u.is_command))
    }

    #[test]
    fn reads_what_the_user_typed() {
        assert_eq!(
            text_of(&user(json!("  Fix the login  "))),
            Some(("Fix the login".into(), false))
        );
        assert_eq!(
            text_of(&user(
                json!([{ "type": "text", "text": "a" }, { "type": "image" }, { "type": "text", "text": "b" }])
            )),
            Some(("a\nb".into(), false))
        );
        assert_eq!(text_of(&user(json!([{ "type": "tool_result", "content": "x" }]))), None);
        assert_eq!(text_of(&user(json!({ "x": 1 }))), None);
        assert_eq!(text_of(&user(json!("   "))), None);
    }

    #[test]
    fn leaves_out_meta_and_other_origins() {
        let mut l = user(json!("hi"));
        l.insert("isMeta".into(), json!(true));
        assert_eq!(text_of(&l), None);
        let mut l = user(json!("hi"));
        l.insert("isCompactSummary".into(), json!(1));
        assert_eq!(text_of(&l), None);
        let mut l = user(json!("hi"));
        l.insert("isVisibleInTranscriptOnly".into(), json!(true));
        assert_eq!(text_of(&l), None);
        let mut l = user(json!("hi"));
        l.insert("origin".into(), json!({ "kind": "task-notification" }));
        assert_eq!(text_of(&l), None);
        assert!(user_text(&l, true).is_some());
        l.insert("origin".into(), json!({ "kind": "human" }));
        assert!(text_of(&l).is_some());
        l.insert("isMeta".into(), json!(0));
        assert!(text_of(&l).is_some());
    }

    #[test]
    fn reads_slash_commands() {
        let cmd = "<command-message>review</command-message>\n<command-name>/review</command-name>\n<command-args> 12 </command-args>";
        assert_eq!(text_of(&user(json!(cmd))), Some(("/review 12".into(), true)));
        let bare = "<command-name>clear</command-name><command-args></command-args>";
        assert_eq!(text_of(&user(json!(bare))), Some(("/clear".into(), true)));
        assert_eq!(text_of(&user(json!("<command-message>x</command-message>"))), None);
        // Before the noise check.
        assert_eq!(
            text_of(&user(json!("<command-name>/x</command-name><system-reminder>"))),
            Some(("/x".into(), true))
        );
    }

    #[test]
    fn drops_noise_and_reminders() {
        for noise in NOISE_PREFIXES {
            assert_eq!(text_of(&user(json!(format!("{noise} more")))), None, "{noise}");
        }
        assert_eq!(
            text_of(&user(json!(
                "Do it <system-reminder>secret\nstuff</system-reminder> now"
            ))),
            Some(("Do it  now".into(), false))
        );
        assert_eq!(
            text_of(&user(json!("x<system-reminder>a</system-reminder>"))),
            Some(("x".into(), false))
        );
    }

    #[test]
    fn summarizes_and_keeps_pasted_text() {
        let text = "See <pasted_content id=1>long\ntext</pasted_content> and <pasted_content x>";
        assert_eq!(prompt_summary(text), "See [Pasted text] and ");
        assert_eq!(prompt_full(text), "See long\ntext and");
    }

    #[test]
    fn truncates_in_utf16_units() {
        assert_eq!(truncate("  a \n b  ", 10), "a b");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab cdef", 4), "ab…");
        assert_eq!(truncate("😀😀😀", 4), "😀\u{fffd}…");
        assert_eq!(truncate("😀😀", 4), "😀😀");
        assert_eq!(truncate("\u{FEFF}x\u{FEFF}", 4), "x");
    }

    #[test]
    fn reads_replies_and_tool_calls() {
        let l = line(json!({ "message": { "content": [
            { "type": "text", "text": " First " },
            { "type": "tool_use", "name": "Edit", "input": { "file_path": "/r/a.ts" } },
            { "type": "tool_use", "name": HANDBACK_TOOL, "input": { "message": "Report" } },
            { "type": "tool_use", "name": 5 },
            { "type": "text", "text": "Second" },
        ] } }));
        let parts = assistant_parts(&l);
        assert_eq!(parts.text, "First \nSecond\nReport");
        assert_eq!(parts.tools, ["Edit"]);
        assert_eq!(parts.uses.len(), 1);
        assert!(parts.handback);
        let s = assistant_parts(&line(json!({ "message": { "content": "  hi " } })));
        assert_eq!((s.text.as_str(), s.tools.len(), s.handback), ("hi", 0, false));
        assert_eq!(assistant_parts(&line(json!({ "message": { "content": 5 } }))).text, "");
    }

    #[test]
    fn names_tools_and_what_they_acted_on() {
        assert_eq!(tool_name("mcp__nodus__create_note"), "create_note");
        assert_eq!(tool_name("mcp__x__"), "mcp__x__");
        assert_eq!(tool_name("Bash"), "Bash");
        let call = |input: Value| describe_tool_use(&json!({ "name": "Bash", "input": input }));
        assert_eq!(
            call(json!({ "command": "ls -la", "description": "d" }))
                .detail
                .as_deref(),
            Some("ls -la")
        );
        assert_eq!(
            call(json!({ "command": "  ", "pattern": "p" })).detail.as_deref(),
            Some("p")
        );
        assert_eq!(
            call(json!({ "file_path": "/r/src/a.ts" })).detail.as_deref(),
            Some("a.ts")
        );
        assert_eq!(
            call(json!({ "notebook_path": "/r/n.ipynb" })).detail.as_deref(),
            Some("n.ipynb")
        );
        // A file whose name is empty stops the search there.
        assert_eq!(call(json!({ "file_path": "/", "pattern": "p" })).detail, None);
        assert_eq!(call(json!("not an object")).detail, None);
        assert_eq!(
            call(json!({ "prompt": "x".repeat(200) }))
                .detail
                .unwrap()
                .chars()
                .count(),
            160
        );
        let mcp = describe_tool_use(&json!({ "name": "mcp__nodus__list_notes", "input": {} }));
        assert_eq!(
            mcp,
            ToolCall {
                name: "list_notes".into(),
                detail: None
            }
        );
    }

    #[test]
    fn reads_an_agents_steps_as_agents_ts_does() {
        // The subagents' reader (02) shares these; its cases, checked against agents.ts.
        assert_eq!(truncate("ab   cdef", 4), "ab…");
        assert_eq!(truncate("a🎉bc", 4), "a🎉…");
        // JavaScript cuts the emoji in half; the half left reads as U+FFFD.
        assert_eq!(truncate("a🎉bc", 3), "a\u{FFFD}…");
        assert_eq!(truncate("ab🎉c", 3), "ab…");
        let call = |block: Value| describe_tool_use(&block);
        assert_eq!(
            call(json!({"name": "Read", "input": {"file_path": "/a/b/c.ts/"}}))
                .detail
                .as_deref(),
            Some("c.ts")
        );
        // A file whose name is empty stops the search.
        assert_eq!(
            call(json!({"name": "Read", "input": {"file_path": "/", "pattern": "x"}})).detail,
            None
        );
        assert_eq!(
            call(json!({"name": "Grep", "input": {"command": " ", "pattern": "TODO"}}))
                .detail
                .as_deref(),
            Some("TODO")
        );
        assert_eq!(
            call(json!({"name": "mcp__nodus__create_note", "input": "x"})).name,
            "create_note"
        );
        assert_eq!(call(json!({"name": "mcp__nodus__", "input": {}})).name, "mcp__nodus__");
        assert_eq!(call(json!({"name": "mcp__a___b"})).name, "b");
        assert_eq!(call(json!({"name": "Task", "input": ["prompt"]})).detail, None);
    }

    #[test]
    fn reads_times_and_task_notifications() {
        assert_eq!(
            time_of(&line(json!({ "timestamp": "1970-01-01T00:00:01.000Z" }))),
            Some(1000.0)
        );
        assert_eq!(time_of(&line(json!({ "timestamp": 5 }))), None);
        assert_eq!(time_of(&line(json!({ "timestamp": "soon" }))), None);
        let note = user(json!(
            "<task-notification><summary> Build done </summary></task-notification>"
        ));
        assert!(is_task_notification(&note));
        assert_eq!(task_notification(&note).as_deref(), Some("Build done"));
        let mut bare = user(json!([{ "type": "text", "text": "x" }]));
        bare.insert("origin".into(), json!({ "kind": "task-notification" }));
        assert_eq!(task_notification(&bare).as_deref(), Some("A background task finished"));
        assert_eq!(task_notification(&user(json!("hi"))), None);
        assert_eq!(
            message_text(&user(json!([{ "text": "a" }, { "type": "x" }, { "text": "b" }]))),
            "a\n\nb"
        );
    }
}
