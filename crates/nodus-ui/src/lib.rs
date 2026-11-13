//! The native UI, drawn with GPUI: theme tokens in light and dark, the shared
//! components, the title bar, the projects grid and rail, the board canvas
//! with its cards, notes and overlays, the terminal element, settings, and the
//! floating capture panel. It reaches the rest of the app only through
//! `nodus-core`.
//!
//! Ports `src/renderer/`.

extern crate gpui_kit as gpui;

pub mod actions;
pub mod app;
pub mod automation;
pub mod board;
pub mod capture;
pub mod components;
pub mod feedback;
pub mod format;
pub mod git;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod parity;
pub mod platform;
pub mod projects;
pub mod servers;
pub mod settings;
pub mod shell;
pub mod state;
pub mod terminal;
pub mod theme;
pub mod views;
pub mod voice;

pub use app::run;
