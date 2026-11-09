//! Terminals and the windows around them (spec 12's UI): the GPUI element
//! that draws `nodus-terminal`'s model as 0.9.4's xterm.js did, the host
//! that shows a session's terminal in the session window or an expanded
//! card, the session window itself (`TerminalModal.tsx`), the transcript
//! preview (`TranscriptPreview.tsx`) and the agents viewer
//! (`AgentViewer.tsx`).
//!
//! Ports `src/renderer/src/terminal/`.

pub mod icons;
pub mod element;
pub mod glyphs;
pub mod host;
pub mod keymap;
pub mod metrics;
pub mod store;
pub mod actions;
pub mod confirm;
pub mod layers;
pub mod overlays;
pub mod session_window;
pub mod driver;
