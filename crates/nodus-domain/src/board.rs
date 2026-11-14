//! A project's board: its session cards, sections, sticky notes, ink, shapes and links.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::project::{FolderColor, Project};
use crate::session::{SessionCard, SessionKind, SessionModel};
use crate::view::BoardView;

/// A rectangle in board coordinates. Like [`Viewport`], a closed shape with
/// nothing more it could hold, so unlike the board's records it keeps no
/// fields it doesn't know.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where the board is scrolled to, and how far it's zoomed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// Board-level state the app owns for a session card.
///
/// Fields are in the order the TypeScript first writes them, so state.json
/// keeps its shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardNode {
    pub session_id: String,
    /// Absent on cards saved before terminals existed; those are Claude sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SessionKind>,
    pub x: f64,
    pub y: f64,
    pub archived: bool,
    /// User-provided title; overrides the transcript title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Session ids whose context was merged to create this one (drawn as edges).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parents: Option<Vec<String>>,
    /// True when the session was started from Nodus (vs. imported).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_app: Option<bool>,
    /// The action this session is a run of. Runs aren't drawn on the board, and
    /// go when their turn ends; one the user takes over joins the board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_of: Option<String>,
    /// The card is expanded in place, showing its terminal on the board. Active cards only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// The view this session is the brain of: it builds the view and keeps it up
    /// to date. A brain isn't drawn on the board; its view shows what it's doing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_of: Option<String>,
    /// Runs in auto mode, as sessions started on the board do, and resumes in it too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_mode: Option<bool>,
    /// The view whose page started this session, drawn as a dashed line from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_view: Option<String>,
    pub created_at: f64,
    /// Size of the expanded card, once it has been resized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    /// When the card was last archived; puts it on top of the archive stack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<f64>,
    /// Section the card is grouped under. Archived cards never belong to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_id: Option<String>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl BoardNode {
    /// The model it was started on, and resumes on too: a [`SessionModel`]
    /// alias in `model`. None is Claude
    /// Code's own default. It's kept with the fields this version doesn't
    /// know, so a `model` of another shape, from a newer version, is kept as
    /// it was rather than making state.json unreadable.
    pub fn model(&self) -> Option<&str> {
        self.extra.get("model").and_then(Value::as_str)
    }

    pub fn set_model(&mut self, model: Option<SessionModel>) {
        match model {
            Some(model) => {
                self.extra.insert("model".into(), Value::from(model.as_str()));
            }
            None => {
                self.extra.remove("model");
            }
        }
    }
}

/// A frame on the board (spec 17): a named, tinted box. Whatever lies in it
/// moves with it; it keeps its size as they move. x/y is its top-left corner.
///
/// Stored as a section, its name before frames: a section saved before then
/// has no size, and is drawn around the cards that name it in `sectionId`
/// (BOARD-081) until the board gives it the size it has there (FRAME-003).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSection {
    pub id: String,
    /// Empty until the user names it.
    pub name: String,
    pub color: FolderColor,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    pub created_at: f64,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Things on the board glued together (spec 17): they're selected, and
/// move, as one. Unlike a frame it isn't drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardGroup {
    pub id: String,
    /// What's in it: cards by session id, and notes, views and shapes by id.
    pub members: Vec<String>,
    pub created_at: f64,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A sticky note on the board: a reminder or scratch text, with no session behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardNote {
    pub id: String,
    pub text: String,
    pub color: FolderColor,
    pub x: f64,
    pub y: f64,
    /// Size of the note, once it has been resized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    /// The session started from the note. It's drawn as a sticker on that
    /// session's card, rather than on the board, until the card leaves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The session that wrote the note with its board tools, when Claude wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// That session's project, when it wrote from another project's board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_project: Option<String>,
    /// That session's title as it wrote, since its card isn't on this board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_title: Option<String>,
    pub created_at: f64,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

string_enum! {
    /// Ink colours. Graphite is dark ink on the light board and light ink on
    /// the dark one. `ALL` is the TypeScript's INK_COLORS.
    pub enum InkColor {
        Graphite = "graphite",
        Red = "red",
        Orange = "orange",
        Green = "green",
        Blue = "blue",
        Purple = "purple",
    }
}

string_enum! {
    /// What an ink mark is. `ALL` is the TypeScript's INK_KINDS, and the
    /// rectangle added since (INKUI-027).
    pub enum InkKind {
        Pen = "pen",
        Arrow = "arrow",
        Text = "text",
        Rect = "rect",
    }
}

/// A mark drawn on the board with the ink tools, in board coordinates: a pen
/// stroke's path, an arrow's tail and head, a rectangle's two opposite
/// corners, or the top-left corner of a label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InkStroke {
    pub id: String,
    pub kind: InkKind,
    pub color: InkColor,
    /// Flat x, y pairs.
    pub points: Vec<f64>,
    /// How thick it's drawn, as a multiple of the default: ink keeps the size it had on screen when drawn.
    pub size: f64,
    /// A label's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

string_enum! {
    /// What a shape on the board is: a box, an ellipse or a diamond with
    /// words in it, words on their own, an emoji sticker, or a picture.
    pub enum ShapeKind {
        Rect = "rect",
        Ellipse = "ellipse",
        Diamond = "diamond",
        Text = "text",
        Sticker = "sticker",
        Image = "image",
    }
}

impl ShapeKind {
    /// The kinds that hold words the user writes in them.
    pub fn has_text(self) -> bool {
        matches!(self, Self::Rect | Self::Ellipse | Self::Diamond | Self::Text)
    }
}

/// A shape on the board, drawn by the user with the dock's tools or by a
/// session with its board tools. Like a note it moves, resizes and can be
/// linked; unlike ink it's a thing on the board, not a mark over it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardShape {
    pub id: String,
    pub kind: ShapeKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub color: InkColor,
    /// The words in it; a sticker's emoji.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// A picture's file, by name, in the app's board images folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The session that drew it with its board tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub created_at: f64,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl BoardShape {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

/// One end of a link: held by something on the board (a card's session id,
/// or a note, view or shape id), or loose at a point.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkEnd {
    /// What it's attached to; none for a loose end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// A loose end's point, in board coordinates. An attached end keeps
    /// where it last was.
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

impl LinkEnd {
    pub fn on(node: impl Into<String>) -> Self {
        Self {
            node: Some(node.into()),
            ..Self::default()
        }
    }

    pub fn at(x: f64, y: f64) -> Self {
        Self { node: None, x, y }
    }
}

/// An arrow on the board from one thing to another. Its ends follow what
/// they're attached to as it moves; it's drawn while both ends are on the
/// board (an archived card's links wait for it to come back).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardLink {
    pub id: String,
    pub from: LinkEnd,
    /// The end with the arrowhead.
    pub to: LinkEnd,
    pub color: InkColor,
    /// Words on the link, drawn at its middle.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The session that drew it with its board tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub created_at: f64,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl BoardLink {
    /// Whether either end is held by `node`.
    pub fn touches(&self, node: &str) -> bool {
        self.from.node.as_deref() == Some(node) || self.to.node.as_deref() == Some(node)
    }
}

/// A message one session on the board sent another with Claude Code's
/// SendMessage, read from their transcripts (BOARD-184).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardTalk {
    /// Claude Code's `msg_id`, the same at both ends.
    pub id: String,
    /// The session that sent it.
    pub from: String,
    /// The session it went to.
    pub to: String,
    /// When it was sent, or taken in when the sender's side isn't known (ms).
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSnapshot {
    pub project: Project,
    pub cards: Vec<SessionCard>,
    pub archive: Rect,
    pub sections: Vec<BoardSection>,
    pub notes: Vec<BoardNote>,
    pub views: Vec<BoardView>,
    /// Ink drawn on the board, oldest first.
    pub ink: Vec<InkStroke>,
    /// Shapes, oldest first (drawn in this order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<BoardShape>,
    /// Links, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<BoardLink>,
    /// Groups, oldest first (spec 17).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<BoardGroup>,
    /// Messages between the board's sessions, oldest first (BOARD-184).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub talks: Vec<BoardTalk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    /// A server board's last snapshot, given while the server can't be
    /// reached: show it read-only, under `Reconnecting…` (SRV-028).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub offline: bool,
}

/// The changes `board.saveNodes` makes to a card.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePatch {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `Some(None)`, sent as `sectionId: null`, takes the card out of its section.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::nullable")]
    pub section_id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
}
