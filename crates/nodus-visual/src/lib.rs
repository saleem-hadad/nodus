//! What Claude can put into a chat besides words: charts, stat tiles, tables
//! and text laid out together (a "visual"), and forms of questions for the
//! user to answer. Claude writes both as JSON, in a tool call or a fenced
//! block in its reply.
//!
//! This crate is pure: it finds the fenced blocks in a reply ([`fences`]).
//! It has no UI; `nodus-ui`'s `chat` module draws what it finds.

pub mod fences;

pub use fences::{Segment, split_fences};
