//! A small clock with a ring round it that fills toward the next run of a
//! schedule (SCHED-032): on a scheduled action's tile and in a view's caption.

use std::f32::consts::{FRAC_PI_2, TAU};

use gpui::{
    AbsoluteLength, App, IntoElement, ParentElement, PathBuilder, RenderOnce, Rgba, Styled, Window, canvas, div, point,
    px,
};
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};

use crate::theme;

/// A clock ring.
#[derive(IntoElement)]
pub struct ClockRing {
    size: AbsoluteLength,
    progress: f32,
    /// The ring's filled part; the accent unless set.
    fill: Option<Rgba>,
    /// Whether the ring is drawn at all (a paused schedule has none).
    ring: bool,
}

/// A clock `size` across (px, or board units inside a node) whose ring is
/// `progress` (0…1) full.
pub fn clock_ring(size: impl Into<AbsoluteLength>, progress: f32) -> ClockRing {
    ClockRing {
        size: size.into(),
        progress: progress.clamp(0.0, 1.0),
        fill: None,
        ring: true,
    }
}

impl ClockRing {
    pub fn fill(mut self, color: Rgba) -> Self {
        self.fill = Some(color);
        self
    }

    /// Just the clock, without its ring.
    pub fn without_ring(mut self) -> Self {
        self.ring = false;
        self
    }
}

fn stroke(width: f32) -> PathBuilder {
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    PathBuilder::stroke(px(width)).with_style(gpui::PathStyle::Stroke(options))
}

impl RenderOnce for ClockRing {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::tokens(cx);
        let (track, fill, hands) = (t.border, self.fill.unwrap_or(t.accent), t.text_secondary);
        let (progress, ring) = (self.progress, self.ring);
        div().flex_none().size(self.size).child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let side = f32::from(bounds.size.width.min(bounds.size.height));
                    let centre = bounds.center();
                    let at = |r: f32, angle: f32| centre + point(px(r * angle.cos()), px(r * angle.sin()));
                    // The ring just inside the bounds, the clock inside it.
                    let width = (side * 0.075).max(1.25);
                    let r = side / 2.0 - width / 2.0;
                    if ring {
                        let mut path = stroke(width);
                        path.move_to(at(r, 0.0));
                        for i in 1..=64 {
                            path.line_to(at(r, TAU * i as f32 / 64.0));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, track);
                        }
                        if progress > 0.0 {
                            let steps = ((64.0 * progress).ceil() as usize).max(2);
                            let mut path = stroke(width);
                            path.move_to(at(r, -FRAC_PI_2));
                            for i in 1..=steps {
                                path.line_to(at(r, -FRAC_PI_2 + TAU * progress * i as f32 / steps as f32));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, fill);
                            }
                        }
                    }
                    // The face: a circle and two hands, as lucide's clock. Under
                    // 18px a face inside the ring fills in, so a ringed clock
                    // there has only its hands, thinner.
                    let small = ring && side < 18.0;
                    let face = if ring { r * 0.62 } else { side / 2.0 - width };
                    let hand = if small { (side * 0.09).max(0.9) } else { width };
                    let reach = if small { r * 0.95 } else { face };
                    let mut path = stroke(hand);
                    if !small {
                        path.move_to(at(face, 0.0));
                        for i in 1..=48 {
                            path.line_to(at(face, TAU * i as f32 / 48.0));
                        }
                    }
                    path.move_to(at(reach * 0.55, -FRAC_PI_2));
                    path.line_to(centre);
                    path.line_to(at(reach * 0.42, 0.55));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, hands);
                    }
                },
            )
            .size_full(),
        )
    }
}
