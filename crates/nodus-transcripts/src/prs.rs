//! The pull requests a session opened (DRAWER-011), read from its
//! transcript: the links a `gh pr create` printed, or a GitHub tool that
//! makes pull requests answered with.

use std::collections::HashSet;

use nodus_domain::PrRef;
use serde_json::Value;

use crate::lines::Json;
use crate::value::{as_str, field};

/// What a transcript says about pull requests the session opened.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenedPrs {
    /// Their links, without anything after the number, oldest first.
    pub urls: Vec<String>,
    /// Calls that open one, waiting for their result: tool use ids.
    pending: HashSet<String>,
}

impl OpenedPrs {
    /// Takes in one transcript line.
    pub fn apply(&mut self, line: &Json) {
        match line.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for b in blocks(line) {
                    if as_str(field(Some(b), "type")) == Some("tool_use")
                        && opens_pr(b)
                        && let Some(id) = as_str(field(Some(b), "id"))
                    {
                        self.pending.insert(id.to_owned());
                    }
                }
            }
            Some("user") if !self.pending.is_empty() => {
                let results: Vec<&Value> = blocks(line)
                    .iter()
                    .filter(|b| as_str(field(Some(b), "type")) == Some("tool_result"))
                    .collect();
                for b in &results {
                    if !as_str(field(Some(b), "tool_use_id")).is_some_and(|id| self.pending.remove(id)) {
                        continue;
                    }
                    let mut text = result_text(field(Some(b), "content"));
                    // `toolUseResult` is the line's one result: Bash's output in full.
                    if results.len() == 1
                        && let Some(out) = as_str(field(line.get("toolUseResult"), "stdout"))
                    {
                        text.push('\n');
                        text.push_str(out);
                    }
                    for url in pr_links(&text) {
                        if !self.urls.contains(&url) {
                            self.urls.push(url);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// A tool use that opens a pull request: a Bash command running
/// `gh pr create`, or a tool named for making one (GitHub's MCP server's
/// `create_pull_request`).
fn opens_pr(block: &Value) -> bool {
    let name = as_str(field(Some(block), "name")).unwrap_or_default();
    if name == "Bash" {
        let command = as_str(field(field(Some(block), "input"), "command")).unwrap_or_default();
        return command.contains("gh pr create") || command.contains("gh pr new");
    }
    name.ends_with("create_pull_request")
}

/// The pull request links in some text, each once, in order.
pub fn pr_links(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices("http") {
        let rest = &text[i..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | ')' | ']' | ','))
            .unwrap_or(rest.len());
        if let Some(at) = PrRef::parse(&rest[..end]) {
            let url = at.url();
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

/// The line's message content blocks.
fn blocks(line: &Json) -> &[Value] {
    match field(line.get("message"), "content") {
        Some(Value::Array(blocks)) => blocks,
        _ => &[],
    }
}

/// A tool result's text: its string, or its text parts joined.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| as_str(field(Some(p), "text")))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn line(v: &Value) -> Json {
        v.as_object().unwrap().clone()
    }

    fn bash(id: &str, command: &str) -> Json {
        line(&json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}
        ]}}))
    }

    fn result(id: &str, text: &str, stdout: Option<&str>) -> Json {
        let mut l = json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": id, "content": text}
        ]}});
        if let Some(out) = stdout {
            l["toolUseResult"] = json!({"stdout": out, "stderr": ""});
        }
        line(&l)
    }

    #[test]
    fn finds_the_link_gh_pr_create_printed() {
        let mut prs = OpenedPrs::default();
        prs.apply(&bash("t1", "git push -u origin HEAD && gh pr create --fill"));
        prs.apply(&result(
            "t1",
            "https://github.com/a/b/pull/12",
            Some("https://github.com/a/b/pull/12\n"),
        ));
        assert_eq!(prs.urls, ["https://github.com/a/b/pull/12"]);
        assert!(prs.pending.is_empty());
    }

    #[test]
    fn ignores_links_other_commands_printed() {
        let mut prs = OpenedPrs::default();
        prs.apply(&bash("t1", "gh pr view 9"));
        prs.apply(&result("t1", "https://github.com/a/b/pull/9", None));
        prs.apply(&bash("t2", "gh pr create --title x"));
        // Another call's result doesn't count.
        prs.apply(&result("t9", "https://github.com/a/b/pull/10", None));
        assert!(prs.urls.is_empty());
        prs.apply(&result(
            "t2",
            "Creating pull request for x into main\n\nhttps://github.com/a/b/pull/11\n",
            None,
        ));
        assert_eq!(prs.urls, ["https://github.com/a/b/pull/11"]);
    }

    #[test]
    fn a_github_tool_that_makes_one_counts_too() {
        let mut prs = OpenedPrs::default();
        prs.apply(&line(&json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "m1", "name": "mcp__github__create_pull_request", "input": {}}
        ]}})));
        prs.apply(&line(&json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "m1", "content": [{"type": "text", "text": "{\"html_url\":\"https://github.com/a/b/pull/4\"}"}]}
        ]}})));
        assert_eq!(prs.urls, ["https://github.com/a/b/pull/4"]);
    }

    #[test]
    fn reads_links_out_of_text() {
        assert_eq!(
            pr_links(
                "see (https://github.com/a/b/pull/1), https://github.com/a/b/pull/1/files and \"https://ghe.co/x/y/pull/2\""
            ),
            ["https://github.com/a/b/pull/1", "https://ghe.co/x/y/pull/2"]
        );
        assert!(pr_links("https://github.com/a/b/issues/1 http://").is_empty());
    }
}
