//! The terminal model behind every terminal in the app, with no UI: a grid
//! over alacritty_terminal that behaves as xterm.js 6 did in 0.9.4 (key
//! encoding, answers to the queries programs send a terminal, Unicode 11
//! widths, selection, links), and the registry that keeps one terminal per
//! session and replays its buffered output in order. The GPUI element that
//! draws it is in `nodus-ui`.
//!
//! Ports the xterm.js behavior in `src/renderer/src/terminal/`.

mod handler;
pub mod keys;
pub mod palette;
pub mod paste;
mod queries;
pub mod registry;
pub mod selection;
pub mod snapshot;
pub mod terminal;
pub mod unicode11;
mod unicode11_tables;

pub use handler::{CursorKind, MouseEncoding, MouseProtocol};
pub use keys::{KeyAction, KeyEvent, Modifiers, Platform};
pub use registry::{ContainerId, Registry, Sessions};
pub use selection::BufferPoint;
pub use snapshot::{CellSnapshot, CursorSnapshot, Snapshot, Underline};
pub use terminal::{Modes, Terminal};
