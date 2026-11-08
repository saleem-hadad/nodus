//! The font stacks of `tokens.css`, as the families they resolve to.
//!
//! `--font-ui` and `--font-display` both start with `-apple-system`, so on a
//! Mac they are the system font (SF Pro; CoreText picks the optical size).
//! Windows takes Segoe UI Variable Text / Display. `--font-mono` is SF Mono
//! on a Mac, which isn't a family CoreText lists: it's loaded from the system
//! at startup ([`register`]) and goes by `.SF NS Mono`. It's never bundled
//! (Apple's licence).

use std::borrow::Cow;

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

/// `--font-mono`.
#[cfg(target_os = "macos")]
pub const MONO: &str = ".SF NS Mono";
#[cfg(not(target_os = "macos"))]
pub const MONO: &str = "Cascadia Mono";

/// Loads the fonts the system has but doesn't list (SF Mono).
pub fn register(cx: &mut App) {
    #[cfg(target_os = "macos")]
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNSMono.ttf")
        && let Err(err) = cx.text_system().add_fonts(vec![Cow::Owned(bytes)])
    {
        tracing::warn!(%err, "couldn't load SF Mono");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (cx, Cow::<[u8]>::Borrowed(&[]));
}

/// `font-variant-numeric: tabular-nums`.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}
