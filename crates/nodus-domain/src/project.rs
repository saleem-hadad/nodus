//! Projects, the groups they're filed under, and what's running for them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::action::ProjectAction;

string_enum! {
    /// The folder colours a project, section or note can have. `ALL` is the
    /// TypeScript's FOLDER_COLORS.
    pub enum FolderColor {
        Blue = "blue",
        Purple = "purple",
        Pink = "pink",
        Red = "red",
        Orange = "orange",
        Yellow = "yellow",
        Green = "green",
        Graphite = "graphite",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Absolute path to the local repository the sessions run in.
    pub repo_path: String,
    pub color: FolderColor,
    pub created_at: f64,
    /// Last time the project board was opened; used for "recent" ordering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<f64>,
    /// The group the project is filed under; ungrouped when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// The project's action buttons, in the order the dock lists them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ProjectAction>>,
    /// One of the projects in focus: with focus mode on, the grid and the rail show only these.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
    /// Archived projects are listed apart on the projects grid and left out of the board's project rail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A named set of projects (e.g. infra repos), shown as its own section on the projects grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGroup {
    pub id: String,
    pub name: String,
    pub created_at: f64,
    /// Folded away on the projects grid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStats {
    pub total: u32,
    pub active: u32,
    pub working: u32,
    pub waiting: u32,
    /// Processes the app is running for the project right now.
    pub running: RunningCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningCounts {
    /// Claude sessions drawn as cards.
    pub sessions: u32,
    /// Claude sessions not drawn on the board: views' brains and action runs.
    pub background: u32,
    /// Terminal cards' shells.
    pub terminals: u32,
}

/// The changes `projects.update` makes to a project.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<FolderColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// `Some(None)`, sent as `groupId: null`, takes the project out of its group.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::nullable")]
    pub group_id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
}

/// The changes `groups.update` makes to a group.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
}
