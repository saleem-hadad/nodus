//! The native UI, drawn with GPUI: theme tokens in light and dark, the shared
//! components, the title bar, the projects grid and rail, the board canvas
//! with its cards, notes and overlays, the terminal element, settings, and the
//! floating capture panel. It reaches the rest of the app only through
//! `nodus-core`.
//!
//! Ports `src/renderer/`.

extern crate gpui_kit as gpui;

pub mod app;
pub mod automation;
pub mod board;
pub mod components;
pub mod format;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod parity;
pub mod platform;
pub mod projects;
pub mod settings;
pub mod shell;
pub mod state;
pub mod theme;

pub use app::run;
