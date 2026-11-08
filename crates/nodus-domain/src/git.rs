//! The project's repository, as the title bar's branch chip and the Changes modal show it.

use serde::{Deserialize, Serialize};

string_enum! {
    /// How a changed file differs from HEAD. Untracked files are new files git
    /// doesn't know about yet; they read as added in the UI.
    pub enum GitFileStatus {
        Modified = "modified",
        Added = "added",
        Deleted = "deleted",
        Renamed = "renamed",
        Conflicted = "conflicted",
        Untracked = "untracked",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileChange {
    /// Path relative to the repository root.
    pub path: String,
    /// For renames, the path in HEAD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: GitFileStatus,
}

/// The project's repository: its branch and uncommitted changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// Null on a detached HEAD.
    pub branch: Option<String>,
    /// The commit HEAD points at; null before the first commit.
    pub oid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    /// Commits ahead of / behind the upstream branch.
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDiff {
    /// Unified diff of the file against HEAD; empty for binary or oversized diffs.
    pub patch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub too_large: Option<bool>,
}

/// One commit in the branch's recent history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub oid: String,
    /// The message's first line.
    pub subject: String,
    pub author: String,
    /// Author date, in ms.
    pub time: f64,
}
