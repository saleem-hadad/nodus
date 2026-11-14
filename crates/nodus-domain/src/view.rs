//! Views: small live pages on the board that Claude builds.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A view: a small live web page on the board, which Claude builds from what
/// the user describes. Its page runs in a sandboxed frame with no network, and
/// reaches the board only through what `window.nodus` offers it. Its data
/// comes from shell commands the user allowed, or from its brain, a Claude
/// session of its own. The page and its data live in the app's data folder,
/// never in the repository.
///
/// Fields are in the order the TypeScript first writes them, so state.json
/// keeps its shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardView {
    pub id: String,
    pub title: String,
    /// What the user asked for, which its brain builds from.
    pub description: String,
    pub x: f64,
    pub y: f64,
    /// Bumped whenever its page changes, so its frame reloads. 0 until there's a page.
    pub version: u32,
    pub created_at: f64,
    pub updated_at: f64,
    /// Its brain, once it has one: started with the view, or when first needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The session that put it up with its board tools, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Shell commands the page may run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<ViewCommand>>,
    /// How often its brain is asked to bring it up to date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<ViewRefresh>,
    /// Size, once resized or set by Claude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    /// The page may start Claude sessions on the board, from something the user
    /// clicks in it. Its brain turns this on when the user asks for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_sessions: Option<bool>,
    /// Whether its brain, and the sessions its page starts, run in auto mode
    /// (VIEW-155). Unset in views saved before it: see [`Self::runs_in_auto_mode`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_mode: Option<bool>,
    /// Every command line the user allowed for this view, or turned down, so one Claude puts back isn't asked about again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_runs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_runs: Option<Vec<String>>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A shell command a view's page may run, in the project folder with the user's shell environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCommand {
    /// What the page calls it by: nodus.run(name).
    pub name: String,
    /// The command line.
    pub run: String,
    /// Minutes between runs while the app is open; it only runs when asked otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every: Option<f64>,
    /// The command line the user allowed. A changed command needs allowing again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved: Option<String>,
    /// The command line the user turned down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied: Option<String>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl BoardView {
    /// Whether its brain, and the sessions its page starts, run in auto mode
    /// (VIEW-155): a view the user made does; one a session put up only when
    /// that session ran in auto mode, and only until a session that doesn't
    /// changes it. A view saved before this was recorded counts as the
    /// user's when no session put it up, and otherwise not.
    pub fn runs_in_auto_mode(&self) -> bool {
        self.auto_mode.unwrap_or(self.from.is_none())
    }
}

/// Asks a view's brain to bring the view up to date, every so often. Each one costs tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewRefresh {
    /// Minutes between refreshes.
    pub every: f64,
    /// What the brain is asked each time.
    pub prompt: String,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// How a view's command last went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub name: String,
    /// Exit code; null when it didn't exit by itself (timed out, or couldn't start).
    pub code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
    /// When it finished.
    pub at: f64,
    pub duration_ms: f64,
    /// Why it didn't run to the end, when it didn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

string_enum! {
    /// Events the app sends a view's page through the board.
    pub enum ViewEvent {
        Data = "data",
        State = "state",
        Result = "result",
    }
}

/// The changes `views.save` makes to a view.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPatch {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The user can stop a view starting sessions; only its brain turns it on,
    /// so only `false` is ever sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_sessions: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateViewOptions {
    pub description: String,
    pub x: f64,
    pub y: f64,
}

/// What a view's page reported: a JavaScript error, or that the page at `loaded` (its version) loaded.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded: Option<u32>,
}
