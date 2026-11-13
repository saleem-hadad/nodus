//! What Claude can put into a chat besides words: charts, stat tiles, tables
//! and text laid out together (a "visual"), and forms of questions for the
//! user to answer. Claude writes both as JSON, in a tool call or a fenced
//! block in its reply.
//!
//! This crate is pure: it finds the fenced blocks in a reply ([`fences`]),
//! reads visuals ([`spec`]) and forms ([`questions`]), lays charts out as
//! plain shapes and text ([`layout`], [`scene`]), and builds a form's
//! answers. It has no UI; `nodus-ui`'s `chat` module draws what it makes.

pub mod fences;
pub mod format;
pub mod layout;
pub mod questions;
pub mod scale;
pub mod scene;
pub mod spec;
pub mod time;

pub use fences::{Segment, split_fences};
pub use questions::{Choice, Form, FormState, Question, answers_json, parse_form};
pub use scene::{Anchor, Hover, Item, Rect, Rgba, Scene, Text, TipLine, VisualTheme, Weight};
pub use spec::{Cell, Chart, Part, PartKind, Stat, Table, Tone, Visual, parse_visual};
