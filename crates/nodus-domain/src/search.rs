//! What a search of a board's sessions finds.

use serde::{Deserialize, Serialize};

string_enum! {
    /// Where in a session a search match is.
    pub enum SearchSnippetKind {
        /// Something the user asked.
        Prompt = "prompt",
        /// Something Claude said.
        Reply = "reply",
        /// A tool call.
        Tool = "tool",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSnippet {
    pub kind: SearchSnippetKind,
    /// The tool's name, for a tool call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The file the tool call acted on, when the match is in what it wrote rather than the path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// One of the session's subagents said or did this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<bool>,
    /// The text around the match, on one line.
    pub text: String,
    /// [start, end) ranges of `text` that match the query, in UTF-16 units as JavaScript counts them.
    pub ranges: Vec<[u32; 2]>,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchHit {
    pub session_id: String,
    /// Messages and tool calls that match the whole query (or, if none does, any of its words).
    pub matches: u32,
    /// The best few matches, best first. None when only the title matched.
    pub snippets: Vec<SearchSnippet>,
}
