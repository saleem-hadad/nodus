//! The status dot at the board's scale (CARD-011…CARD-015): the shared
//! `.status-dot` with its breath and pulse, every length through [`Z`].

use std::time::Duration;

use gpui::{App, Div, ElementId, IntoElement, ParentElement, Styled, Window, div};
use nodus_domain::SessionStatus;

use super::{Z, anim, fade};
use crate::components::child_id;
use crate::components::status_dot::{breathe_at, pulse_at};
use crate::components::tween::transition;
use crate::theme::{self, motion};

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
    let key: ElementId = key.into();
    // MOTION-012: from one status's colour to the next, not at once.
    let color = transition(
        window,
        cx,
        child_id(key.clone(), "color"),
        t.status(status),
        Duration::from_millis(300),
        motion::ease,
    );
    let size = if small { 7.0 } else { 10.0 };
    let (spread, share) = match status {
        // CARD-013: no ring, still.
        SessionStatus::Done => (0.0, 0.0),
        // CARD-012, CARD-015: 1.6s ease-in-out each way.
        SessionStatus::Working => breathe_at(anim::cycle(window, cx, key, Duration::from_millis(1600)), small),
        // CARD-014: 1.1s ease-out, 0 → 8px.
        SessionStatus::Waiting => pulse_at(anim::cycle(window, cx, key, Duration::from_millis(1100))),
    };
    let dot = div().flex_none().relative().size(z.px(size)).rounded_full().bg(color);
    if share > 0.0 && spread > 0.0 {
        // The ring is a circle `spread` bigger all round (a box-shadow's
        // spread keeps a circle round; GPUI's keeps the radius).
        dot.child(
            div()
                .absolute()
                .left(z.px(-spread))
                .top(z.px(-spread))
                .size(z.px(size + 2.0 * spread))
                .rounded_full()
                .bg(fade(color, share)),
        )
        .child(div().absolute().inset_0().rounded_full().bg(color))
    } else {
        dot
    }
}

/// A dot inside a box `height` tall with `top` above it: the card head's
/// dot sits 5px down to centre on the title's first line.
pub fn dot_at(dot: Div, top: f32, z: Z) -> impl IntoElement {
    div().flex_none().pt(z.px(top)).child(dot)
}
