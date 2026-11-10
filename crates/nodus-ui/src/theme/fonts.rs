//! The font stacks of `tokens.css`, as the families they resolve to.
//!
//! `--font-ui` and `--font-display` both start with `-apple-system`, so on a
//! Mac they are the system font (SF Pro; CoreText picks the optical size).
//! Windows takes Segoe UI Variable Text / Display.
//!
//! `--font-mono` is `'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono',
//! Consolas, monospace`, and 0.9.4 drew every use of it in Menlo on a Mac:
//! SF Mono is a hidden system font Chromium can't find by name, and Chromium
//! has no `ui-monospace`, so the first family it found was Menlo (the goldens
//! show Menlo's glyphs and its 0.6 em advance). On Windows it found Cascadia
//! Mono, or Consolas where that isn't installed ([`mono`]).

use gpui::{App, FontFeatures};

/// `--font-ui`.
#[cfg(target_os = "macos")]
pub const UI: &str = ".SystemUIFont";
#[cfg(not(target_os = "macos"))]
pub const UI: &str = "Segoe UI Variable Text";

/// `--font-display`.
#[cfg(target_os = "macos")]
pub const DISPLAY: &str = ".SystemUIFont";
#[cfg(not(target_os = "macos"))]
pub const DISPLAY: &str = "Segoe UI Variable Display";

/// `--font-mono` off a Mac, once [`register`] has looked for Cascadia Mono.
#[cfg(not(target_os = "macos"))]
static MONO: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// `--font-mono`: the first of its families Chromium finds. Menlo on a Mac;
/// on Windows Cascadia Mono, or Consolas where it isn't installed (Windows
/// 10). GPUI doesn't go down a list of families: one the system lacks falls
/// to GPUI's own fallbacks (Segoe UI), so [`register`] picks the family.
pub fn mono() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Menlo"
    }
    #[cfg(not(target_os = "macos"))]
    {
        MONO.get().copied().unwrap_or("Cascadia Mono")
    }
}

/// Draws all text as `global.css` has it (`-webkit-font-smoothing:
/// antialiased`) and, off a Mac, picks `--font-mono`'s family.
pub fn register(cx: &mut App) {
    cx.set_font_smoothing(gpui::FontSmoothing::Antialiased);
    #[cfg(not(target_os = "macos"))]
    {
        let cascadia = cx.text_system().all_font_names().iter().any(|f| f == "Cascadia Mono");
        let _ = MONO.set(if cascadia { "Cascadia Mono" } else { "Consolas" });
    }
}

/// `font-variant-numeric: tabular-nums`.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}
