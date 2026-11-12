//! The status dot shared by folders, the rail, cards and the terminal header
//! (`.status-dot`, UI-019…UI-022).

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{App, BoxShadow, ElementId, IntoElement, Pixels, RenderOnce, Rgba, Styled, Window, div, px};
use nodus_domain::SessionStatus;

use crate::components::ambient;
use crate::parity;
use crate::theme::motion;
use crate::theme::tokens::{alpha, ring};
use crate::theme::{self};

/// A status dot.
#[derive(IntoElement)]
pub struct StatusDot {
    id: ElementId,
    status: SessionStatus,
    size: Pixels,
    small: bool,
    /// The folder badges' dot: no ring and no breathing while working.
    still_working: bool,
    /// The rail's dot: a 1.5px `--bg-elevated` border inside its size.
    border: Option<(Pixels, Rgba)>,
}

/// A 10px dot (`.status-dot`).
pub fn status_dot(id: impl Into<ElementId>, status: SessionStatus) -> StatusDot {
    StatusDot {
        id: id.into(),
        status,
        size: px(10.),
        small: false,
        still_working: false,
        border: None,
    }
}

impl StatusDot {
    /// `.is-small`: 7px, a smaller breath (UI-022).
    pub fn small(mut self) -> Self {
        self.small = true;
        self.size = px(7.);
        self
    }

    /// A 7px dot whose working state neither rings nor breathes (`.pv-badge`).
    pub fn badge(mut self) -> Self {
        self.size = px(7.);
        self.still_working = true;
        self
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn border(mut self, width: Pixels, color: Rgba) -> Self {
        self.border = Some((width, color));
        self
    }
}

/// Keyframes of the working dot's ring: (spread, alpha) at rest and at the
/// breath's peak.
fn breath(small: bool) -> ((f32, f32), (f32, f32)) {
    if small {
        ((2.0, 0.22), (3.5, 0.10))
    } else {
        ((3.0, 0.22), (5.0, 0.10))
    }
}

/// The ring at `t` (0…1 linear) through the breathe animation: 1.6s,
/// `ease-in-out` each way between rest and peak.
pub fn breathe_at(t: f32, small: bool) -> (f32, f32) {
    let ((s0, a0), (s1, a1)) = breath(small);
    let half = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
    let e = motion::ease_in_out(half);
    (s0 + (s1 - s0) * e, a0 + (a1 - a0) * e)
}

/// The ring at `t` (0…1 linear) through the pulse: 1.1s `ease-out`, from 0px
/// at 55% to 8px transparent.
pub fn pulse_at(t: f32) -> (f32, f32) {
    let e = motion::ease_out_keyword(t);
    (8.0 * e, 0.55 * (1.0 - e))
}

/// The ring outside the dot (outside its border too, as CSS draws it).
fn ring_shadow(color: Rgba, spread: f32, share: f32) -> BoxShadow {
    let mut shadow = ring(spread, color);
    shadow.color = alpha(color, share);
    shadow
}

impl RenderOnce for StatusDot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::tokens(cx);
        let color = t.status(self.status);
        let frozen = parity::frozen(cx);
        let small = self.small;
        let base = div()
            .flex_none()
            .size(self.size)
            .rounded_full()
            .bg(color)
            .when_some(self.border, |d, (w, c)| d.border(w).border_color(c));
        match self.status {
            SessionStatus::Done => base.into_any_element(),
            SessionStatus::Working if self.still_working => base.into_any_element(),
            SessionStatus::Working => {
                let phase = if frozen {
                    0.0
                } else {
                    ambient::cycle(window, cx, self.id, Duration::from_millis(1600))
                };
                let (spread, share) = breathe_at(phase, small);
                base.shadow(vec![ring_shadow(color, spread, share)]).into_any_element()
            }
            SessionStatus::Waiting => {
                let phase = if frozen {
                    0.0
                } else {
                    ambient::cycle(window, cx, self.id, Duration::from_millis(1100))
                };
                let (spread, share) = pulse_at(phase);
                base.shadow(vec![ring_shadow(color, spread, share)]).into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breathes_between_rest_and_peak() {
        assert_eq!(breathe_at(0.0, false), (3.0, 0.22));
        let (s, a) = breathe_at(0.5, false);
        assert!((s - 5.0).abs() < 1e-5 && (a - 0.10).abs() < 1e-5);
        assert_eq!(breathe_at(0.0, true), (2.0, 0.22));
        let (s, _) = breathe_at(0.5, true);
        assert!((s - 3.5).abs() < 1e-5);
    }

    #[test]
    fn pulses_out_to_nothing() {
        assert_eq!(pulse_at(0.0), (0.0, 0.55));
        let (s, a) = pulse_at(1.0);
        assert!((s - 8.0).abs() < 1e-5 && a.abs() < 1e-5);
    }
}
