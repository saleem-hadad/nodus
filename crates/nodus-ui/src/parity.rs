//! Parity mode, for the golden screenshots (migration/parity/README.md, "How
//! a shot is made"): infinite animations are held at their first keyframe,
//! and "now" is frozen, so a shot comes out the same every time. Off unless
//! the automation driver turns it on.

use gpui::{App, Global};

#[derive(Debug, Clone, Copy, Default)]
pub struct Parity {
    /// Infinite animations hold at time 0; the text caret is hidden.
    pub frozen: bool,
    /// The UI's "now", in ms since the epoch, when frozen.
    pub now_ms: Option<f64>,
}

impl Global for Parity {}

/// Whether infinite animations are held still.
pub fn frozen(cx: &App) -> bool {
    cx.try_global::<Parity>().is_some_and(|p| p.frozen)
}

/// `Date.now()`, or the frozen clock in parity mode.
pub fn now_ms(cx: &App) -> f64 {
    if let Some(now) = cx.try_global::<Parity>().and_then(|p| p.now_ms) {
        return now;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}
