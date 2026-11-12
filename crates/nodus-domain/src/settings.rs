//! App-wide preferences.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

string_enum! {
    /// The app's appearance.
    pub enum ThemePreference {
        /// Follows the system appearance.
        System = "system",
        Light = "light",
        Dark = "dark",
    }
}

/// App-wide preferences, persisted with the rest of the state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: ThemePreference,
    /// Download new versions in the background and install them when the app quits.
    pub auto_update: bool,
    /// While the app is in the background, notify when a session needs the user or finishes.
    pub notify: bool,
    /// Count the sessions waiting on the user on the Dock icon.
    pub dock_badge: bool,
    /// Keep the project rail out beside the board, instead of sliding it in from the window's left edge.
    pub pin_project_rail: bool,
    /// Offers focus mode: the title bar's Focus switch, and Add to Focus on projects. Off, none of it shows.
    pub focus_feature: bool,
    /// The title bar's Focus switch: show only the projects in focus, in the grid and the rail.
    pub focus_mode: bool,
    /// Double-tapping Shift in any app opens the floating capture panel.
    pub capture: bool,
    /// A mouse's wheel zooms the board; a trackpad's scroll still pans.
    /// Added after 1.0.7 and written only when off, so a file that never
    /// turned it off is written back as it was read.
    #[serde(default = "on", skip_serializing_if = "is_on")]
    pub wheel_zoom: bool,
    /// Settings this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for AppSettings {
    /// What a fresh install starts with.
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            auto_update: true,
            notify: true,
            dock_badge: true,
            pin_project_rail: false,
            focus_feature: false,
            focus_mode: false,
            capture: true,
            wheel_zoom: true,
            extra: Map::new(),
        }
    }
}

/// A switch that starts on, when the file doesn't have it.
fn on() -> bool {
    true
}

/// A switch left on, which isn't written.
fn is_on(value: &bool) -> bool {
    *value
}

/// The settings `settings.update` changes; the rest stay as they are.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemePreference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_update: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dock_badge: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_project_rail: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_feature: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wheel_zoom: Option<bool>,
}
