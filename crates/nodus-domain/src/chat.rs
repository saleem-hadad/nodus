//! A Claude session as a chat: what the chat view draws (read from the
//! session's transcript), the prompts waiting to go to it, and the envelope
//! a form's or a page's answers travel back in.
//!
//! The chat is a view of the transcript, never a second record: everything
//! here can be rebuilt from the JSONL Claude Code writes, so it needs no
//! storage of its own.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A prompt waiting for the session's turn to end. The session host keeps
/// them, in order, and hands the first to Claude as its Stop hook's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedPrompt {
    /// Made by the app when it queues the prompt; never reused.
    pub id: String,
    pub text: String,
    /// When it was queued, in milliseconds since the epoch.
    pub at: f64,
}

/// What Claude Code puts before a Stop hook's reason when it hands it to
/// Claude (and writes to the transcript as a user line).
pub const STOP_FEEDBACK_PREFIX: &str = "Stop hook feedback:\n";

/// The first line of a queued prompt as the host hands it over, so Claude
/// reads it as the user's next message and the chat can show it as one.
pub const QUEUED_PROMPT_LEAD: &str =
    "[Nodus] The user queued this message while you were working. Treat it as their next message:";

/// The Stop hook's reason for a queued prompt.
pub fn queued_reason(text: &str) -> String {
    format!("{QUEUED_PROMPT_LEAD}\n\n{text}")
}

/// The user's words in a Stop hook line that delivered a queued prompt;
/// `None` for any other text.
pub fn queued_text(line: &str) -> Option<&str> {
    let rest = line.strip_prefix(STOP_FEEDBACK_PREFIX).unwrap_or(line);
    let rest = rest.strip_prefix(QUEUED_PROMPT_LEAD)?;
    Some(rest.trim_start_matches(['\n', '\r']))
}

/// How many lines a call that changes a file adds and takes away, for the
/// chat's `+12 −3` (CHAT-047).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineChanges {
    pub added: u32,
    pub removed: u32,
}

/// The answers to a form in the chat, or to a page on the board, as they go
/// to Claude: a tag naming what they answer, then the answers as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answers {
    /// The form's id (its tool call's id, or the id of the reply it was in)
    /// or the page's view id.
    pub to: String,
    /// The form's or page's title, so the message reads well on its own.
    pub title: String,
    /// What was picked and typed, by question.
    pub answers: Value,
}

const ANSWERS_OPEN: &str = "<nodus-answers";
const ANSWERS_CLOSE: &str = "</nodus-answers>";

impl Answers {
    /// The message that carries them: `<nodus-answers to="…" title="…">`,
    /// the JSON pretty-printed, and the closing tag.
    pub fn to_message(&self) -> String {
        let json = serde_json::to_string_pretty(&self.answers).unwrap_or_else(|_| "{}".into());
        format!(
            "{ANSWERS_OPEN} to=\"{}\" title=\"{}\">\n{json}\n{ANSWERS_CLOSE}",
            attr(&self.to),
            attr(&self.title)
        )
    }

    /// The answers in a message, when it is one.
    pub fn from_message(text: &str) -> Option<Self> {
        let text = text.trim();
        let rest = text.strip_prefix(ANSWERS_OPEN)?;
        let close = rest.find('>')?;
        let (head, body) = (&rest[..close], &rest[close + 1..]);
        let body = body.trim().strip_suffix(ANSWERS_CLOSE)?.trim();
        Some(Self {
            to: unattr(read_attr(head, "to")?),
            title: unattr(read_attr(head, "title").unwrap_or_default()),
            answers: serde_json::from_str(body).ok()?,
        })
    }
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('\n', " ")
}

fn unattr(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&amp;", "&")
}

fn read_attr<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = head.find(&key)? + key.len();
    let len = head[start..].find('"')?;
    Some(&head[start..start + len])
}

/// One thing the chat shows, in transcript order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatEntry {
    /// Something the user said: typed, queued (handed over by the Stop
    /// hook), or a form's or page's answers.
    #[serde(rename_all = "camelCase")]
    User {
        id: String,
        at: f64,
        text: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        queued: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answers: Option<Answers>,
    },
    /// Claude's words, as Markdown.
    #[serde(rename_all = "camelCase")]
    Text { id: String, at: f64, markdown: String },
    /// A tool Claude called, summed up in a line.
    #[serde(rename_all = "camelCase")]
    Tool {
        /// The call's `tool_use` id.
        id: String,
        at: f64,
        /// As Claude Code names it, `mcp__…` prefixes taken off.
        name: String,
        /// What it acted on, in a line: a command, a pattern, a path.
        detail: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        failed: bool,
        /// Its result has come in.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        done: bool,
        /// The lines an edit or a write adds and takes away (CHAT-047).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        changes: Option<LineChanges>,
    },
    /// A chart, stats, a table or text laid out (`show_visual`, or a
    /// fenced `visual` block): the spec as Claude wrote it.
    #[serde(rename_all = "camelCase")]
    Visual { id: String, at: f64, spec: Value },
    /// Questions for the user (`ask`, or a fenced `questions` block).
    #[serde(rename_all = "camelCase")]
    Questions {
        id: String,
        at: f64,
        spec: Value,
        /// The answers sent, once the transcript shows them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered: Option<Value>,
    },
    /// A page Claude put on the board (`show_page`, `create_view`).
    #[serde(rename_all = "camelCase")]
    Page {
        /// The call's `tool_use` id.
        id: String,
        at: f64,
        title: String,
        /// The view, once the call's result names it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered: Option<Value>,
    },
    /// Something Claude Code noted: an interrupt, a compaction, a background
    /// task finishing, a command's output.
    #[serde(rename_all = "camelCase")]
    Notice { id: String, at: f64, text: String },
}

impl ChatEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::User { id, .. }
            | Self::Text { id, .. }
            | Self::Tool { id, .. }
            | Self::Visual { id, .. }
            | Self::Questions { id, .. }
            | Self::Page { id, .. }
            | Self::Notice { id, .. } => id,
        }
    }

    pub fn at(&self) -> f64 {
        match self {
            Self::User { at, .. }
            | Self::Text { at, .. }
            | Self::Tool { at, .. }
            | Self::Visual { at, .. }
            | Self::Questions { at, .. }
            | Self::Page { at, .. }
            | Self::Notice { at, .. } => *at,
        }
    }

    /// The user's side of the chat.
    pub fn is_user(&self) -> bool {
        matches!(self, Self::User { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_survive_their_message() {
        let answers = Answers {
            to: "toolu_1".into(),
            title: "Layout \"A\" & B".into(),
            answers: json!({"Which layout?": "tabs", "Notes": ["a", "b"]}),
        };
        let message = answers.to_message();
        assert!(message.starts_with("<nodus-answers to=\"toolu_1\" title=\"Layout &quot;A&quot; &amp; B\">\n"));
        assert_eq!(Answers::from_message(&message), Some(answers));
        assert_eq!(Answers::from_message("hello"), None);
    }

    #[test]
    fn a_queued_prompt_reads_back_from_its_stop_hook_line() {
        let reason = queued_reason("run the tests");
        assert_eq!(
            queued_text(&format!("{STOP_FEEDBACK_PREFIX}{reason}")),
            Some("run the tests")
        );
        assert_eq!(queued_text(&reason), Some("run the tests"));
        assert_eq!(queued_text("Stop hook feedback:\nsomething else"), None);
    }
}
