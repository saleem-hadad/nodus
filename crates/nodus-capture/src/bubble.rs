//! Claude's guess at what's typed into the board's bubble (BUBBLE-020…BUBBLE-024):
//! a new session, a sticky note or a view, and for a session, the model the
//! ask needs. The same quick headless call as a floating capture's guess
//! (Haiku, no thinking, tools or MCP servers, outside any repository), on its
//! own [`Guesser`] so the two never stop each other.

use nodus_domain::js;
use nodus_domain::{BubbleGuess, BubbleRoute, SessionModel};
use nodus_platform::ClaudeNotFound;
use serde_json::Value;

use crate::guess::{Guesser, MAX_REASON_CHARS, MAX_TEXT_CHARS, SETTINGS, flat};

/// The system prompt, verbatim.
pub const SYSTEM_PROMPT: &str = "You route what the user typed into a bubble on their Nodus board, a canvas of Claude Code sessions for one project. Decide what it becomes:
- session: a new Claude Code session in the project's repository that does what the text asks. The default for work to be done, questions about the code and anything to look into.
- note: a sticky note on the board, and nothing runs. For ideas, reminders and things to do later or to ask someone, not work to start now (\"remember to…\", \"idea:\", \"todo:\", \"ask finance about…\").
- view: a live page on the board that Claude builds and keeps up to date. Only when the text asks for something to watch or show: a dashboard, a table, a chart, a list or the status of something.

For a session, pick the model the ask needs:
- haiku: small, clear asks: a question about the code, a lookup, running a command, a typo, a rename, a one-line change.
- sonnet: everyday coding work: a bug fix, a small feature, tests, a change across a few files, a review.
- opus: hard or open-ended work: designing or planning something, a change across much of the codebase, a subtle bug, performance or security work, anything that needs careful reasoning.
When torn between two, take the larger. For a note or a view, answer sonnet.

Answer with kind, model and reason: one short sentence for the user, under 80 characters, saying why. For example: \"A one-line rename, quick work.\" or \"It's an idea to keep, not work to start now.\"";

/// `JSON.stringify(SCHEMA)`: what Claude answers in.
pub const SCHEMA: &str = r#"{"type":"object","properties":{"kind":{"type":"string","enum":["session","note","view"]},"model":{"type":"string","enum":["haiku","sonnet","opus"]},"reason":{"type":"string"}},"required":["kind","model","reason"],"additionalProperties":false}"#;

/// The question: the project, and what was typed (BUBBLE-021).
pub fn build_prompt(text: &str, project: &str) -> String {
    let text = js::slice_utf16(js::trim(text), 0, MAX_TEXT_CHARS);
    format!("Project: {}\n\nThe bubble:\n\"\"\"\n{text}\n\"\"\"", flat(project, 120))
}

/// The arguments after `claude` (BUBBLE-020).
pub fn bubble_args(text: &str, project: &str) -> Vec<String> {
    [
        "-p",
        "--model",
        "haiku",
        "--settings",
        SETTINGS,
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--json-schema",
        SCHEMA,
        "--output-format",
        "json",
        "--system-prompt",
        SYSTEM_PROMPT,
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([build_prompt(text, project)])
    .collect()
}

/// Claude's answer (BUBBLE-022): a kind it didn't name is a session, and a
/// model it didn't name is Sonnet.
pub fn read_answer(raw: &Value) -> Option<BubbleGuess> {
    let fields = raw.as_object()?;
    let field = |name: &str| fields.get(name).and_then(Value::as_str);
    let model = field("model")
        .and_then(SessionModel::parse)
        .unwrap_or(SessionModel::Sonnet);
    let route = match field("kind") {
        Some("note") => BubbleRoute::Note,
        Some("view") => BubbleRoute::View,
        _ => BubbleRoute::Session(model),
    };
    let reason = field("reason").map(|r| flat(r, MAX_REASON_CHARS)).unwrap_or_default();
    Some(BubbleGuess { route, reason })
}

/// What `claude -p` printed, read as the guess: JSON, not an error, with the
/// schema's answer in `structured_output`.
pub fn read_reply(stdout: &str) -> Option<BubbleGuess> {
    let parsed: Value = nodus_domain::json::from_str(js::trim(stdout)).ok()?;
    if parsed.get("is_error").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    read_answer(parsed.get("structured_output")?)
}

impl Guesser {
    /// Asks Claude where the bubble's text goes; `None` if it can't say, or
    /// a newer guess (or the bubble closing) stopped this one (BUBBLE-023).
    pub async fn guess_bubble(&self, text: &str, project: &str) -> Result<Option<BubbleGuess>, ClaudeNotFound> {
        self.cancel();
        if js::trim(text).is_empty() {
            return Ok(None);
        }
        let stdout = self.ask(bubble_args(text, project)).await?;
        Ok(stdout.as_deref().and_then(read_reply))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reply(answer: &Value) -> String {
        json!({ "type": "result", "is_error": false, "structured_output": answer }).to_string()
    }

    #[test]
    fn reads_a_session_on_its_model() {
        let guess = read_reply(&reply(
            &json!({ "kind": "session", "model": "opus", "reason": "A redesign." }),
        ));
        assert_eq!(
            guess,
            Some(BubbleGuess {
                route: BubbleRoute::Session(SessionModel::Opus),
                reason: "A redesign.".into(),
            })
        );
    }

    #[test]
    fn reads_notes_and_views_whatever_the_model() {
        let note = read_reply(&reply(&json!({ "kind": "note", "model": "haiku", "reason": "" })));
        assert_eq!(note.map(|g| g.route), Some(BubbleRoute::Note));
        let view = read_reply(&reply(&json!({ "kind": "view", "model": "opus", "reason": "" })));
        assert_eq!(view.map(|g| g.route), Some(BubbleRoute::View));
    }

    #[test]
    fn falls_back_to_a_session_on_sonnet() {
        let guess = read_reply(&reply(&json!({ "kind": "into", "model": "fable" })));
        assert_eq!(
            guess,
            Some(BubbleGuess {
                route: BubbleRoute::Session(SessionModel::Sonnet),
                reason: String::new(),
            })
        );
    }

    #[test]
    fn no_guess_from_an_error_or_garbage() {
        let error = json!({ "is_error": true, "structured_output": { "kind": "note" } }).to_string();
        assert_eq!(read_reply(&error), None);
        assert_eq!(read_reply("not json"), None);
        assert_eq!(read_reply(&json!({ "result": "hi" }).to_string()), None);
    }

    #[test]
    fn asks_haiku_with_the_project_and_the_text_last() {
        let args = bubble_args("  fix the login bug ", "storefront");
        let at = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[at + 1], "haiku");
        let prompt = args.last().unwrap();
        assert!(prompt.starts_with("Project: storefront\n"));
        assert!(prompt.ends_with("\"\"\"\nfix the login bug\n\"\"\""));
    }
}
