//! Floating capture: what the user typed in the panel, what came along from
//! their screen, and where it went.

use serde::{Deserialize, Serialize};

use crate::project::FolderColor;
use crate::session::{SessionModel, SessionStatus};

string_enum! {
    /// What floating capture makes of what the user typed. `ALL` is the
    /// TypeScript's CAPTURE_KINDS: Tab cycles through them in this order.
    pub enum CaptureKind {
        /// A new Claude session on it, in auto mode.
        Session = "session",
        /// The next prompt of a session already on the board.
        Into = "into",
        /// A sticky note on the board.
        Note = "note",
        /// A live view, built by its own Claude session.
        View = "view",
    }
}

/// How much copied text goes along with a capture, in UTF-16 units as JavaScript counts them.
pub const MAX_COPIED_CHARS: usize = 20_000;

/// Something from the user's screen that goes along with a capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CaptureAttachment {
    /// The browser tab in front when the panel opened.
    Tab(TabAttachment),
    /// A screenshot, or an image that was copied, saved in the app's data folder.
    Image(ImageAttachment),
    /// A file dropped on the panel or picked.
    File(FileAttachment),
    /// Text that was copied.
    Text(TextAttachment),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabAttachment {
    pub url: String,
    pub title: String,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub path: String,
    /// A small data URL.
    pub thumb: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAttachment {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextAttachment {
    pub text: String,
}

/// A session the panel can send into, or show on a recent capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSession {
    pub project_id: String,
    pub session_id: String,
    pub title: String,
    pub status: SessionStatus,
    /// Whether its process is running.
    pub live: bool,
    pub updated_at: f64,
}

/// Something captured earlier, with how it's doing now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRecent {
    pub id: String,
    pub kind: CaptureKind,
    pub project_id: String,
    pub title: String,
    pub created_at: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    /// A session's status now. Notes and views have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SessionStatus>,
}

/// What the panel can send to: every project, their sessions and the recent captures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTargets {
    pub projects: Vec<CaptureProject>,
    /// Claude sessions on the projects' boards, most recently active first.
    pub sessions: Vec<CaptureSession>,
    pub recents: Vec<CaptureRecent>,
    /// Where a capture goes before Claude has guessed: the board used last.
    pub default_project_id: Option<String>,
}

/// A project as the panel lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProject {
    pub id: String,
    pub name: String,
    pub color: FolderColor,
}

string_enum! {
    /// Whether the double-tap can be heard: on a Mac it needs Accessibility permission.
    pub enum CaptureShortcut {
        Ready = "ready",
        NeedsPermission = "needs-permission",
        /// The setting is off.
        Off = "off",
        /// The key hook couldn't load.
        Unavailable = "unavailable",
    }
}

/// What the panel opened onto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureContext {
    /// The app in front when it opened, unless that was Nodus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// That app's browser tab, which arrives a moment after the panel opens.
    /// Written as the tab attachment it is, `kind` and all.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tagged_tab")]
    pub tab: Option<TabAttachment>,
    /// What's on the clipboard, offered rather than attached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<ClipboardOffer>,
    pub shortcut: CaptureShortcut,
}

/// What's on the clipboard as the panel opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClipboardOffer {
    Text { text: String },
    Image { thumb: String },
}

/// Claude's guess at what a capture is and where it goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureGuess {
    pub kind: CaptureKind,
    pub project_id: String,
    /// For `into`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// One short line saying why, for the user.
    pub reason: String,
}

/// What the board's bubble makes of what was typed into it: where it goes,
/// and for a session, the model that suits the ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleRoute {
    Session(SessionModel),
    Note,
    View,
}

/// Claude's guess at a bubble's route, with one short line saying why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BubbleGuess {
    pub route: BubbleRoute,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSubmission {
    pub text: String,
    pub kind: CaptureKind,
    pub project_id: String,
    /// For `into`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub attachments: Vec<CaptureAttachment>,
}

/// What a capture became, for the receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureReceipt {
    pub kind: CaptureKind,
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
}

/// A [`TabAttachment`] on its own, still tagged `kind: 'tab'` as the
/// TypeScript's `Extract<CaptureAttachment, { kind: 'tab' }>` is.
mod tagged_tab {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{CaptureAttachment, TabAttachment};

    pub(super) fn serialize<S: Serializer>(tab: &Option<TabAttachment>, serializer: S) -> Result<S::Ok, S::Error> {
        tab.clone().map(CaptureAttachment::Tab).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<TabAttachment>, D::Error> {
        match Option::<CaptureAttachment>::deserialize(deserializer)? {
            None => Ok(None),
            Some(CaptureAttachment::Tab(tab)) => Ok(Some(tab)),
            Some(_) => Err(D::Error::custom("expected an attachment of kind tab")),
        }
    }
}
