//! Claude sessions and terminals: their cards, status, subagents, usage and transcripts.

use serde::{Deserialize, Serialize};

use crate::board::BoardNode;

string_enum! {
    /// Live state of a Claude session, driven by Claude Code hooks.
    pub enum SessionStatus {
        /// Claude is processing a turn (orange).
        Working = "working",
        /// Claude is blocked on the user, e.g. a permission prompt or a question (pulsing red).
        Waiting = "waiting",
        /// The last turn finished, the process is gone, or the card is archived (blue).
        Done = "done",
    }
}

string_enum! {
    /// What a card runs.
    pub enum SessionKind {
        /// A Claude Code session (the default).
        Claude = "claude",
        /// A plain login shell in the project folder; never archived, no transcript.
        Terminal = "terminal",
    }
}

/// Metadata read from a Claude Code transcript (~/.claude/projects/<encoded-path>/<id>.jsonl).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub session_id: String,
    /// custom-title > ai-title > first prompt (truncated).
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    /// Last plain-text reply from Claude, truncated; shown as the card preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_assistant_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Number of user + assistant messages (tool noise excluded).
    pub message_count: u32,
    pub created_at: f64,
    pub updated_at: f64,
    pub transcript_path: String,
}

/// A subagent a session started with Claude's Agent tool. Claude Code keeps its
/// transcript beside the session's, in <session id>/subagents/agent-<id>.jsonl.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub agent_id: String,
    /// e.g. general-purpose, Explore, fork, or a custom agent's name.
    pub agent_type: String,
    /// The short task description Claude gave the agent.
    pub description: String,
    /// Driven by the hooks Claude runs inside the agent; done once the session's process is gone.
    pub status: SessionStatus,
    /// Set when the agent was started by another agent rather than by the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    /// The user stopped it before it finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<bool>,
    /// The tool it is running (or last ran), when its latest step was a tool call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// One line on its latest step: what the tool acted on, or what it said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Background commands it is waiting on, having ended its turn. Each wakes it
    /// when it finishes, so it reads as working meanwhile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<u32>,
    pub tool_count: u32,
    pub started_at: f64,
    pub updated_at: f64,
}

/// Everything the UI needs to draw a card: the board node, and what's known
/// about its session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCard {
    #[serde(flatten)]
    pub node: BoardNode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<SessionMeta>,
    pub status: SessionStatus,
    /// Whether a claude process is currently running for this session.
    pub live: bool,
    /// Subagents of the current run, oldest first: those still running, and those
    /// that finished since the last prompt. Active Claude cards only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<AgentInfo>>,
    /// How many subagents the session has started in all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_count: Option<u32>,
    /// What Claude last reported about the session's model, context and cost. Claude cards only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
}

/// What Claude Code reports to its status line, which sessions started from
/// the app send to us instead of drawing in the terminal. Numbers are passed
/// on as Claude reports them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    /// e.g. Opus 5.5
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Share of the context window in use, 0–100. Unknown before the first reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_percent: Option<f64>,
    /// Tokens in the context window, and its size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Time the session has been running as of `at`, carried across resumes.
    /// It keeps counting while the process is live.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionOptions {
    pub x: f64,
    pub y: f64,
    /// Defaults to claude. Terminals ignore name, initial_prompt and parents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SessionKind>,
    /// Display name passed to `claude --name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// First message sent to the new session (used by the merge feature).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parents: Option<Vec<String>>,
    /// Runs this project action: its prompt, auto mode and board tools. Claude sessions only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    /// The card starts expanded on the board, showing its terminal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// Runs in auto mode: Claude Code approves routine steps by itself and blocks risky ones. Claude sessions only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_mode: Option<bool>,
    /// The view on this board whose page started it. Claude sessions only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_view: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryResult {
    pub session_id: String,
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

string_enum! {
    /// Where a session's summary has got to.
    pub enum SummarizePhase {
        Queued = "queued",
        Running = "running",
        Done = "done",
        Error = "error",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeProgress {
    pub request_id: String,
    pub session_id: String,
    pub phase: SummarizePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    /// What the call acted on, in one line: a command, a file name, a pattern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

string_enum! {
    /// Who said a transcript message.
    pub enum MessageRole {
        User = "user",
        Assistant = "assistant",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptMessage {
    pub role: MessageRole,
    pub text: String,
    pub timestamp: f64,
    /// Names of tools Claude called in this message, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// The same calls with what each acted on; agent transcripts only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calls: Option<Vec<ToolCall>>,
    /// A note from Claude Code rather than a message, such as a background task finishing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSessionResult {
    /// Buffered terminal output to replay into a fresh terminal.
    pub replay: String,
    pub live: bool,
}
