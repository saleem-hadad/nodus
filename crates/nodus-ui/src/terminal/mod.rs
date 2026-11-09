//! Terminals and the windows around them (spec 12's UI): the GPUI element
//! that draws `nodus-terminal`'s model as 0.9.4's xterm.js did, the host
//! that shows a session's terminal in the session window or an expanded
//! card, the session window itself (`TerminalModal.tsx`), the transcript
//! preview (`TranscriptPreview.tsx`) and the agents viewer
//! (`AgentViewer.tsx`).
//!
//! Ports `src/renderer/src/terminal/`.

/// `--font-mono` as 0.9.4 drew it in the page as well as in the terminal:
/// Chromium resolved `'SF Mono', ui-monospace, Menlo, …` to Menlo on a Mac
/// (the goldens' mono text has Menlo's 0.6 em advance and shapes), Cascadia
/// Mono on Windows.
#[cfg(target_os = "macos")]
pub const MONO: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
pub const MONO: &str = crate::theme::fonts::MONO;

pub mod icons;
pub mod card;
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
pub mod agents;
pub mod messages;
pub mod preview;
pub mod text;
