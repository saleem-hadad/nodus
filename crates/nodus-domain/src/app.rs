//! The app itself: what it runs on, its menu commands, and cards to open.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub claude_path: Option<String>,
    pub claude_version: Option<String>,
    /// Node's name for the OS: darwin, win32 or linux.
    pub platform: String,
}

/// A card to open, from a notification the user clicked. Without a session, just its project's board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCardRequest {
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

string_enum! {
    /// Commands sent from the native application menu. The menu owns the
    /// shortcuts so they work even while a terminal has keyboard focus.
    pub enum MenuCommand {
        /// Cmd+W: closes the open modal, else goes back to projects (never closes the window).
        Close = "close",
        /// Cmd+N: new Claude session on the open board.
        NewSession = "new-session",
        /// Cmd+T: new shell in the project folder on the open board.
        NewTerminal = "new-terminal",
        /// Cmd+Shift+N: add a project folder.
        NewProject = "new-project",
        /// Cmd+,: open the Settings window.
        Settings = "settings",
        /// Settings, open at Capture & Voice: a voice prompt found no key.
        VoiceSettings = "voice-settings",
        /// Settings, open at Updates: Check for Updates… shows the check there.
        UpdateSettings = "update-settings",
        /// Cmd+K: search the board's sessions.
        Search = "search",
        /// Cmd+Shift+F: flip the title bar's Focus switch (only while focus is offered in Settings).
        ToggleFocus = "toggle-focus",
        /// Help › Report a Problem…: the report dialog (RPT-001).
        ReportProblem = "report-problem",
    }
}
