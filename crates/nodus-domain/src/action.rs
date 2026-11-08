//! The board's actions rail: buttons that each start a session on their prompt.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::project::Project;

string_enum! {
    /// Icons an action can have, by lucide name. The UI maps each to its
    /// drawing; the main process asks Claude to pick one. `ALL` is the
    /// TypeScript's ACTION_ICONS, and `parse` its isActionIcon.
    pub enum ActionIcon {
        Zap = "zap",
        RefreshCw = "refresh-cw",
        GitMerge = "git-merge",
        GitPullRequest = "git-pull-request",
        GitCommitHorizontal = "git-commit-horizontal",
        Upload = "upload",
        Download = "download",
        Archive = "archive",
        BrushCleaning = "brush-cleaning",
        Eraser = "eraser",
        Sparkles = "sparkles",
        WandSparkles = "wand-sparkles",
        TestTube = "test-tube",
        Bug = "bug",
        Rocket = "rocket",
        Package = "package",
        FileText = "file-text",
        BookOpen = "book-open",
        ListChecks = "list-checks",
        Search = "search",
        ShieldCheck = "shield-check",
        Hammer = "hammer",
        SquareTerminal = "square-terminal",
        Gauge = "gauge",
        Lightbulb = "lightbulb",
        MessageSquare = "message-square",
    }
}

impl ActionIcon {
    /// DEFAULT_ACTION_ICON.
    pub const DEFAULT: Self = Self::Zap;
}

/// A button in the board's actions rail. Claude drafts it from what the user
/// describes; each press starts a session on its prompt, in auto mode, that
/// shows beside the button rather than on the board and goes once it finishes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAction {
    pub id: String,
    pub name: String,
    /// One of [`ActionIcon`]'s names. Kept as written, as the TypeScript does.
    pub icon: String,
    /// What the user asked for, kept so the action can be redrafted.
    pub description: String,
    /// The run's first prompt.
    pub prompt: String,
    /// In every project's rail, and kept app-wide rather than with one project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<bool>,
    pub created_at: f64,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A project's own actions, and the global ones every project shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLists {
    pub project: Project,
    pub global: Vec<ProjectAction>,
}

/// Claude's draft of an action, for the user to review before saving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDraft {
    pub name: String,
    pub icon: String,
    pub prompt: String,
}

/// An action run that finished and left the board, with Claude's last word on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRunResult {
    pub project_id: String,
    pub session_id: String,
    pub action_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
