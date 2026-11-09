//! The status dot at the board's scale (CARD-011…CARD-015): the shared
//! `.status-dot` with its breath and pulse, every length through [`Z`].

use std::time::Duration;

use gpui::{App, Div, ElementId, IntoElement, ParentElement, Styled, Window, div};
use nodus_domain::SessionStatus;

use super::{Shade, Shadow, Z, anim, fade};
use crate::components::status_dot::{breathe_at, pulse_at};
use crate::theme;

/// A 10px dot, or 7px when `small`.
pub fn status_dot(
    key: impl Into<ElementId>,
    status: SessionStatus,
    small: bool,
    z: Z,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let t = theme::tokens(cx);
    let color = t.status(status);
    let size = if small { 7.0 } else { 10.0 };
    let (spread, share) = match status {
        SessionStatus::Done => (0.0, 0.0),
        // CARD-012, CARD-015: 1.6s ease-in-out each way.
        SessionStatus::Working => breathe_at(anim::cycle(window, cx, key, Duration::from_millis(1600)), small),
        // CARD-014: 1.1s ease-out, 0 → 8px.
        SessionStatus::Waiting => pulse_at(anim::cycle(window, cx, key, Duration::from_millis(1100))),
    };
    let dot = div().flex_none().size(z.px(size)).rounded_full().bg(color);
    if share > 0.0 {
        dot.shadow(Shadow::of(&[Shade::ring(spread, fade(color, share))]).gpui(z))
    } else {
        dot
    }
}

/// A dot inside a box `height` tall with `top` above it: the card head's
/// dot sits 5px down to centre on the title's first line.
pub fn dot_at(dot: Div, top: f32, z: Z) -> impl IntoElement {
    div().flex_none().pt(z.px(top)).child(dot)
}
