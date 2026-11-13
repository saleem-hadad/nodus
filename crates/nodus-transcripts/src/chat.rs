//! A session's transcript read as a chat, for the chat view (CHAT-001…CHAT-020
//! in spec 16): the user's prompts, Claude's words as Markdown, each tool
//! call as a line, and the charts, forms and pages Claude shows, in the order
//! the transcript has them.
//!
//! [`ChatLog`] follows one transcript file as Claude Code appends to it: each
//! [`ChatLog::refresh`] reads only the lines added since the last one, and a
//! tool's result, or the answers to a form, update the entry they belong to
//! in place.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use nodus_domain::{Answers, ChatEntry, js, queued_text};
use nodus_visual::{Segment, split_fences};
use regex::Regex;
use serde_json::Value;

use crate::lines::{Json, read_lines};
use crate::parse::{describe_tool_use, message_text, prompt_full, task_notification, time_of, tool_name, user_text};
use crate::value::{as_str, field, truthy};

/// Only this much of the end of a long transcript is read when the chat
/// opens (CHAT-002).
pub const CHAT_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The board tools' server, as its tools are named: `mcp__nodus__show_visual`.
const OUR_TOOLS: &str = "mcp__nodus__";

/// A notice's text is cut to this many characters (CHAT-015).
const NOTICE_MAX_CHARS: usize = 2000;

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("a valid pattern"));
static LOCAL_OUTPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<local-command-std(?:out|err)>(.*?)</local-command-std(?:out|err)>").expect("a valid pattern")
});

/// One session's transcript as a chat, kept up to date as the file grows.
pub struct ChatLog {
    file: PathBuf,
    /// Where the next read starts.
    offset: u64,
    entries: Vec<ChatEntry>,
    /// Entries a later line can change, by id: tool calls, forms and pages.
    by_id: HashMap<String, usize>,
    /// Page entries by the view they put up, for their answers.
    pages: HashMap<String, usize>,
    version: u64,
    snapshot: Option<(u64, Arc<[ChatEntry]>)>,
}

impl ChatLog {
    /// A chat of the transcript at `file`, read on the first [`Self::refresh`].
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            offset: 0,
            entries: Vec::new(),
            by_id: HashMap::new(),
            pages: HashMap::new(),
            version: 0,
            snapshot: None,
        }
    }

    /// Reads what the transcript gained since the last call. True when the
    /// chat changed. A missing file is an empty chat (CHAT-001); a file that
    /// shrank was replaced, so it's read again from the start (CHAT-003).
    pub fn refresh(&mut self) -> bool {
        let Ok(stat) = fs::metadata(&self.file) else {
            return false;
        };
        let size = stat.len();
        if size < self.offset {
            *self = Self {
                version: self.version + 1,
                ..Self::new(std::mem::take(&mut self.file))
            };
        }
        if size == self.offset {
            return false;
        }
        let Ok(fd) = File::open(&self.file) else {
            return false;
        };
        // CHAT-002: a long transcript opens at its last 16 MiB.
        let first = self.offset == 0 && size > CHAT_MAX_BYTES;
        let start = if first { size - CHAT_MAX_BYTES } else { self.offset };
        let before = self.version;
        let mut lines = Vec::new();
        self.offset = read_lines(&fd, start, size, first, |line| lines.push(line.clone()), None);
        for line in &lines {
            self.apply(line);
        }
        self.version != before
    }

    /// Everything the chat shows, oldest first.
    pub fn entries(&self) -> &[ChatEntry] {
        &self.entries
    }

    /// The entries as one shared list, made again only after a change.
    pub fn snapshot(&mut self) -> Arc<[ChatEntry]> {
        match &self.snapshot {
            Some((version, entries)) if *version == self.version => Arc::clone(entries),
            _ => {
                let entries: Arc<[ChatEntry]> = self.entries.clone().into();
                self.snapshot = Some((self.version, Arc::clone(&entries)));
                entries
            }
        }
    }

    /// Goes up by one with every change.
    pub fn version(&self) -> u64 {
        self.version
    }

    fn push(&mut self, entry: ChatEntry) {
        match &entry {
            ChatEntry::Tool { id, .. }
            | ChatEntry::Visual { id, .. }
            | ChatEntry::Questions { id, .. }
            | ChatEntry::Page { id, .. } => {
                self.by_id.insert(id.clone(), self.entries.len());
            }
            _ => {}
        }
        self.entries.push(entry);
        self.version += 1;
    }

    fn notice(&mut self, id: String, at: f64, text: &str) {
        let text = ANSI.replace_all(text, "");
        let text = js::trim(&text);
        if text.is_empty() {
            return;
        }
        let text = if text.chars().count() > NOTICE_MAX_CHARS {
            let cut: String = text.chars().take(NOTICE_MAX_CHARS).collect();
            format!("{}…", cut.trim_end())
        } else {
            text.to_owned()
        };
        self.push(ChatEntry::Notice { id, at, text });
    }

    fn apply(&mut self, line: &Json) {
        // CHAT-004: a subagent's lines are its own.
        if truthy(line.get("isSidechain")) {
            return;
        }
        let at = time_of(line).unwrap_or(0.0);
        let uuid = as_str(line.get("uuid")).unwrap_or_default().to_owned();
        match as_str(line.get("type")) {
            Some("user") => self.user(line, uuid, at),
            Some("assistant") => self.assistant(line, &uuid, at),
            Some("system") => self.system(line, uuid, at),
            _ => {}
        }
    }

    fn user(&mut self, line: &Json, uuid: String, at: f64) {
        let content = field(line.get("message"), "content");
        // CHAT-008: a tool's result finishes its call's line.
        if let Some(Value::Array(blocks)) = content {
            let mut results = false;
            for block in blocks
                .iter()
                .filter(|b| as_str(field(Some(b), "type")) == Some("tool_result"))
            {
                results = true;
                let Some(id) = as_str(field(Some(block), "tool_use_id")) else {
                    continue;
                };
                let failed = truthy(field(Some(block), "is_error"));
                let text = result_text(field(Some(block), "content"));
                self.finish_tool(id, failed, &text);
            }
            if results {
                return;
            }
        }
        let text = message_text(line);
        let trimmed = js::trim(&text);
        // CHAT-006: a queued prompt, handed to Claude by the Stop hook.
        if let Some(words) = queued_text(trimmed) {
            let words = js::trim(words).to_owned();
            self.user_said(uuid, at, words, true);
            return;
        }
        if truthy(line.get("isMeta")) || truthy(line.get("isCompactSummary")) {
            return;
        }
        // CHAT-012
        if trimmed.starts_with("[Request interrupted") {
            self.notice(uuid, at, "Interrupted");
            return;
        }
        // CHAT-013
        if let Some(note) = task_notification(line) {
            self.notice(uuid, at, &note);
            return;
        }
        // CHAT-014: what a slash command printed.
        if trimmed.contains("<local-command-std") {
            let out: Vec<String> = LOCAL_OUTPUT
                .captures_iter(trimmed)
                .map(|c| js::trim(&c[1]).to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            self.notice(uuid, at, &out.join("\n"));
            return;
        }
        // CHAT-005
        let Some(said) = user_text(line, false) else {
            return;
        };
        let mut words = prompt_full(&said.text);
        let images = match content {
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter(|b| as_str(field(Some(b), "type")) == Some("image"))
                .count(),
            _ => 0,
        };
        if images > 0 {
            let label = if images == 1 {
                "[1 image]".to_owned()
            } else {
                format!("[{images} images]")
            };
            words = if words.is_empty() {
                label
            } else {
                format!("{words}\n\n{label}")
            };
        }
        self.user_said(uuid, at, words, false);
    }

    /// CHAT-007: a message that answers a form or a page marks it answered.
    fn user_said(&mut self, id: String, at: f64, text: String, queued: bool) {
        let answers = Answers::from_message(&text);
        if let Some(answers) = &answers {
            self.mark_answered(&answers.to, &answers.answers);
        }
        self.push(ChatEntry::User {
            id,
            at,
            text,
            queued,
            answers,
        });
    }

    fn mark_answered(&mut self, to: &str, answers: &Value) {
        let index = self.by_id.get(to).or_else(|| self.pages.get(to)).copied();
        let Some(entry) = index.and_then(|i| self.entries.get_mut(i)) else {
            return;
        };
        match entry {
            ChatEntry::Questions { answered, .. } | ChatEntry::Page { answered, .. } => {
                *answered = Some(answers.clone());
                self.version += 1;
            }
            _ => {}
        }
    }

    fn assistant(&mut self, line: &Json, uuid: &str, at: f64) {
        // CHAT-016
        if truthy(line.get("isApiErrorMessage")) {
            self.notice(uuid.to_owned(), at, &message_text(line));
            return;
        }
        let blocks = match field(line.get("message"), "content") {
            Some(Value::String(text)) => vec![serde_json::json!({ "type": "text", "text": text })],
            Some(Value::Array(blocks)) => blocks.clone(),
            _ => return,
        };
        for (i, block) in blocks.iter().enumerate() {
            match as_str(field(Some(block), "type")) {
                Some("text") => {
                    let text = as_str(field(Some(block), "text")).unwrap_or_default();
                    self.text(&format!("{uuid}:{i}"), at, text);
                }
                Some("tool_use") => self.tool_use(block, at),
                // CHAT-011: thinking isn't shown.
                _ => {}
            }
        }
    }

    /// CHAT-009: Claude's words, with fenced `visual` and `questions` blocks
    /// taken out and shown as what they describe.
    fn text(&mut self, id: &str, at: f64, text: &str) {
        for (j, segment) in split_fences(text).into_iter().enumerate() {
            let id = format!("{id}:{j}");
            match segment {
                Segment::Markdown(markdown) => {
                    let markdown = js::trim(&markdown).to_owned();
                    if !markdown.is_empty() {
                        self.push(ChatEntry::Text { id, at, markdown });
                    }
                }
                Segment::Visual(raw) => match serde_json::from_str::<Value>(&raw) {
                    Ok(spec) => self.push(ChatEntry::Visual { id, at, spec }),
                    Err(err) => self.notice(id, at, &format!("A visual block isn’t valid JSON: {err}")),
                },
                Segment::Questions(raw) => match serde_json::from_str::<Value>(&raw) {
                    Ok(spec) => self.push(ChatEntry::Questions {
                        id,
                        at,
                        spec,
                        answered: None,
                    }),
                    Err(err) => self.notice(id, at, &format!("A questions block isn’t valid JSON: {err}")),
                },
            }
        }
    }

    /// CHAT-010: the board tools that show something become what they show;
    /// every other tool is a line.
    fn tool_use(&mut self, block: &Value, at: f64) {
        let Some(id) = as_str(field(Some(block), "id")).map(str::to_owned) else {
            return;
        };
        let name = as_str(field(Some(block), "name")).unwrap_or_default();
        let input = field(Some(block), "input").cloned().unwrap_or(Value::Null);
        let ours = name.starts_with(OUR_TOOLS);
        let entry = match (ours, tool_name(name)) {
            (true, "show_visual") => ChatEntry::Visual {
                id,
                at,
                spec: input.get("visual").cloned().unwrap_or(input),
            },
            (true, "ask") => ChatEntry::Questions {
                id,
                at,
                spec: input,
                answered: None,
            },
            (true, "show_page" | "create_view") => ChatEntry::Page {
                id,
                at,
                title: as_str(input.get("title")).unwrap_or("A page").to_owned(),
                view_id: None,
                answered: None,
            },
            _ => {
                let call = describe_tool_use(block);
                ChatEntry::Tool {
                    id,
                    at,
                    name: call.name,
                    detail: call.detail.unwrap_or_default(),
                    failed: false,
                    done: false,
                }
            }
        };
        self.push(entry);
    }

    fn finish_tool(&mut self, id: &str, failed: bool, result: &str) {
        let Some(&index) = self.by_id.get(id) else {
            return;
        };
        let entry = &mut self.entries[index];
        match entry {
            ChatEntry::Tool { failed: f, done, .. } => {
                *f = failed;
                *done = true;
            }
            // CHAT-010: a show the board turned down is a failed call's line.
            ChatEntry::Visual { .. } | ChatEntry::Questions { .. } | ChatEntry::Page { .. } if failed => {
                let name = match entry {
                    ChatEntry::Visual { .. } => "show_visual",
                    ChatEntry::Questions { .. } => "ask",
                    _ => "show_page",
                };
                *entry = ChatEntry::Tool {
                    id: id.to_owned(),
                    at: entry.at(),
                    name: name.to_owned(),
                    detail: first_line(result),
                    failed: true,
                    done: true,
                };
            }
            ChatEntry::Page { view_id, .. } => {
                let view = serde_json::from_str::<Value>(js::trim(result))
                    .ok()
                    .and_then(|v| v.get("view_id").and_then(Value::as_str).map(str::to_owned));
                if let Some(view) = view {
                    self.pages.insert(view.clone(), index);
                    *view_id = Some(view);
                }
            }
            _ => return,
        }
        self.version += 1;
    }

    fn system(&mut self, line: &Json, uuid: String, at: f64) {
        match as_str(line.get("subtype")) {
            // CHAT-017
            Some("compact_boundary") => self.notice(uuid, at, "Earlier messages were summarized to free up context"),
            // CHAT-014
            Some("local_command") => {
                let content = as_str(line.get("content")).unwrap_or_default();
                let out: Vec<String> = LOCAL_OUTPUT
                    .captures_iter(content)
                    .map(|c| js::trim(&c[1]).to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
                self.notice(uuid, at, &out.join("\n"));
            }
            _ => {}
        }
    }
}

/// A tool result's text: its content when that's a string, else its text
/// blocks joined.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| as_str(field(Some(b), "text")))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn first_line(text: &str) -> String {
    let line = js::trim(text).lines().next().unwrap_or_default();
    crate::parse::truncate(line, 160)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use serde_json::json;

    use super::*;

    struct Transcript {
        _dir: tempfile::TempDir,
        path: PathBuf,
    }

    impl Transcript {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("s.jsonl");
            fs::write(&path, "").unwrap();
            Self { _dir: dir, path }
        }

        fn add(&self, lines: &[Value]) {
            let mut file = fs::OpenOptions::new().append(true).open(&self.path).unwrap();
            for line in lines {
                writeln!(file, "{line}").unwrap();
            }
        }
    }

    fn user(uuid: &str, content: Value) -> Value {
        json!({ "type": "user", "uuid": uuid, "timestamp": "2026-11-13T10:00:00Z", "message": { "role": "user", "content": content } })
    }

    fn assistant(uuid: &str, block: Value) -> Value {
        json!({ "type": "assistant", "uuid": uuid, "timestamp": "2026-11-13T10:00:01Z", "message": { "role": "assistant", "content": [block] } })
    }

    fn result(uuid: &str, tool: &str, text: &str, error: bool) -> Value {
        user(
            uuid,
            json!([{ "type": "tool_result", "tool_use_id": tool, "is_error": error, "content": [{ "type": "text", "text": text }] }]),
        )
    }

    fn kinds(log: &ChatLog) -> Vec<&'static str> {
        log.entries()
            .iter()
            .map(|e| match e {
                ChatEntry::User { .. } => "user",
                ChatEntry::Text { .. } => "text",
                ChatEntry::Tool { .. } => "tool",
                ChatEntry::Visual { .. } => "visual",
                ChatEntry::Questions { .. } => "questions",
                ChatEntry::Page { .. } => "page",
                ChatEntry::Notice { .. } => "notice",
            })
            .collect()
    }

    #[test]
    fn chat_001_a_missing_transcript_is_an_empty_chat() {
        let mut log = ChatLog::new(PathBuf::from("/nonexistent/x.jsonl"));
        assert!(!log.refresh());
        assert!(log.entries().is_empty());
    }

    #[test]
    fn reads_prompts_words_and_tool_lines_as_they_arrive() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        t.add(&[
            user("u1", json!("Why is the build slow?")),
            assistant("a1", json!({ "type": "thinking", "thinking": "hm" })),
            assistant("a2", json!({ "type": "text", "text": "Let me look." })),
            assistant(
                "a3",
                json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "cargo build" } }),
            ),
        ]);
        assert!(log.refresh());
        assert_eq!(kinds(&log), ["user", "text", "tool"]);
        assert!(
            matches!(&log.entries()[2], ChatEntry::Tool { name, detail, done: false, .. } if name == "Bash" && detail == "cargo build")
        );
        let version = log.version();
        assert!(!log.refresh());
        t.add(&[result("u2", "t1", "ok", false)]);
        assert!(log.refresh());
        assert!(log.version() > version);
        assert!(matches!(
            &log.entries()[2],
            ChatEntry::Tool {
                done: true,
                failed: false,
                ..
            }
        ));
    }

    #[test]
    fn a_partly_written_line_waits_for_its_newline() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        let line = user("u1", json!("hello")).to_string();
        fs::write(&t.path, &line[..10]).unwrap();
        assert!(!log.refresh());
        fs::write(&t.path, format!("{line}\n")).unwrap();
        assert!(log.refresh());
        assert_eq!(kinds(&log), ["user"]);
    }

    #[test]
    fn fenced_visuals_and_questions_come_out_of_the_words() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        let text = "Here it is:\n\n```visual\n{\"mark\": \"bar\"}\n```\n\nAnd a question:\n\n```questions\n{\"title\": \"Pick\", \"questions\": []}\n```\n";
        t.add(&[assistant("a1", json!({ "type": "text", "text": text }))]);
        log.refresh();
        assert_eq!(kinds(&log), ["text", "visual", "text", "questions"]);
        assert!(matches!(&log.entries()[1], ChatEntry::Visual { spec, .. } if spec == &json!({"mark": "bar"})));
    }

    #[test]
    fn board_tools_show_what_they_show_and_answers_mark_them() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        t.add(&[
            assistant(
                "a1",
                json!({ "type": "tool_use", "id": "v1", "name": "mcp__nodus__show_visual", "input": { "visual": { "mark": "line" } } }),
            ),
            assistant(
                "a2",
                json!({ "type": "tool_use", "id": "q1", "name": "mcp__nodus__ask", "input": { "title": "Layout", "questions": [] } }),
            ),
            assistant(
                "a3",
                json!({ "type": "tool_use", "id": "p1", "name": "mcp__nodus__show_page", "input": { "title": "Mockups", "html": "<p>" } }),
            ),
            result("r3", "p1", "{\"view_id\": \"view-9\"}", false),
        ]);
        log.refresh();
        assert_eq!(kinds(&log), ["visual", "questions", "page"]);
        assert!(
            matches!(&log.entries()[2], ChatEntry::Page { view_id: Some(v), title, .. } if v == "view-9" && title == "Mockups")
        );

        let form = Answers {
            to: "q1".into(),
            title: "Layout".into(),
            answers: json!({ "Which?": "tabs" }),
        };
        let page = Answers {
            to: "view-9".into(),
            title: "Mockups".into(),
            answers: json!({ "seen": true }),
        };
        t.add(&[
            user("u1", json!(form.to_message())),
            user("u2", json!(page.to_message())),
        ]);
        log.refresh();
        assert!(
            matches!(&log.entries()[1], ChatEntry::Questions { answered: Some(a), .. } if a == &json!({ "Which?": "tabs" }))
        );
        assert!(matches!(&log.entries()[2], ChatEntry::Page { answered: Some(_), .. }));
        assert!(matches!(&log.entries()[3], ChatEntry::User { answers: Some(a), .. } if a.to == "q1"));
    }

    #[test]
    fn a_show_the_board_turned_down_is_a_failed_line() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        t.add(&[
            assistant(
                "a1",
                json!({ "type": "tool_use", "id": "v1", "name": "mcp__nodus__show_visual", "input": { "visual": {} } }),
            ),
            result("r1", "v1", "mark: missing\nmore", true),
        ]);
        log.refresh();
        assert!(
            matches!(&log.entries()[0], ChatEntry::Tool { failed: true, name, detail, .. } if name == "show_visual" && detail == "mark: missing")
        );
    }

    #[test]
    fn a_queued_prompt_is_the_users_and_meta_lines_are_not() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        let mut queued = user(
            "u1",
            json!(format!(
                "Stop hook feedback:\n{}",
                nodus_domain::queued_reason("run the tests")
            )),
        );
        queued["isMeta"] = json!(true);
        let mut meta = user("u2", json!("Caveat: something"));
        meta["isMeta"] = json!(true);
        t.add(&[queued, meta, user("u3", json!("[Request interrupted by user]"))]);
        log.refresh();
        assert_eq!(kinds(&log), ["user", "notice"]);
        assert!(matches!(&log.entries()[0], ChatEntry::User { queued: true, text, .. } if text == "run the tests"));
    }

    #[test]
    fn sidechain_lines_and_noise_stay_out() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        let mut side = assistant("a1", json!({ "type": "text", "text": "agent words" }));
        side["isSidechain"] = json!(true);
        t.add(&[side, user("u1", json!("<system-reminder>x</system-reminder>"))]);
        log.refresh();
        assert!(log.entries().is_empty());
    }

    #[test]
    fn a_replaced_transcript_is_read_again() {
        let t = Transcript::new();
        let mut log = ChatLog::new(t.path.clone());
        t.add(&[user("u1", json!("one")), user("u2", json!("two"))]);
        log.refresh();
        fs::write(&t.path, format!("{}\n", user("u3", json!("three")))).unwrap();
        assert!(log.refresh());
        assert_eq!(log.entries().len(), 1);
    }
}
