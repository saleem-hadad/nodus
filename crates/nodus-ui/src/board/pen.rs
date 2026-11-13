//! Claude's pen (MOTION-024): while a session draws on the board, a
//! sparkle with its name sets off from its card, travels to each thing in
//! turn and follows it as it's drawn (round a shape's outline, along a
//! link, across words), then rests a moment and fades. It never takes the
//! pointer.

use std::time::Instant;

use gpui::{AnyElement, FontWeight, IntoElement, ParentElement, Styled, div, linear_color_stop, linear_gradient, px};
use nodus_board::geometry::Point;
use nodus_domain::ShapeKind;

use super::canvas::BoardCanvas;
use super::moments::{Drawing, PenAt, Step, What};
use super::shapes;
use crate::theme::ai;
use crate::theme::tokens::hex;

impl BoardCanvas {
    /// Each drawing's pen, on screen.
    pub(crate) fn render_pens(&self, now: Instant) -> Vec<AnyElement> {
        let vp = self.viewport;
        self.moments
            .drawings
            .iter()
            .filter_map(|d| {
                let (at, opacity) = d.pen(now);
                let p = self.pen_point(d, at)?;
                let ms = now.saturating_duration_since(d.start).as_secs_f64() * 1000.0;
                let (x, y) = super::viewport::to_screen(vp, p.x, p.y);
                Some(pen(x as f32, y as f32, &d.title, ms, opacity))
            })
            .collect()
    }

    /// Where a drawing's pen is, on the board.
    fn pen_point(&self, d: &Drawing, at: PenAt) -> Option<Point> {
        match at {
            PenAt::Travel { to, p } => {
                let dest = self.tip(&d.steps[to], 0.0)?;
                let from = match to {
                    0 => d.origin.unwrap_or(dest),
                    i => self.tip(&d.steps[i - 1], 1.0).unwrap_or(dest),
                };
                let p = f64::from(p);
                Some(Point {
                    x: from.x + (dest.x - from.x) * p,
                    y: from.y + (dest.y - from.y) * p,
                })
            }
            PenAt::Draw { at, p } => self.tip(&d.steps[at], p),
            PenAt::Rest => self.tip(d.steps.last()?, 1.0),
        }
    }

    /// Where a thing being drawn is laid down, `p` (0…1) through it: as
    /// the shape, link and words draw themselves (MOTION-021, MOTION-022).
    fn tip(&self, step: &Step, p: f32) -> Option<Point> {
        if step.what == What::Link {
            let curve = self.link_curve(self.link(&step.id)?)?;
            return Some(shapes::at(&curve, f64::from(crate::theme::motion::ease_in_out(p))));
        }
        let r = self.node(&step.id)?.rect();
        let centre = Point {
            x: r.x + r.w / 2.0,
            y: r.y + r.h / 2.0,
        };
        Some(match step.what {
            What::Shape(kind @ (ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Diamond)) => {
                // The outline's middle, inside the box (shape.rs).
                let inner = nodus_domain::Rect {
                    x: r.x + 1.0,
                    y: r.y + 1.0,
                    w: (r.w - 2.0).max(0.0),
                    h: (r.h - 2.0).max(0.0),
                };
                let (traced, _, _) = shapes::drawn_share(p);
                shapes::line_part(&shapes::outline_line(kind, inner), f64::from(traced)).1
            }
            // Across the words as they're typed.
            What::Shape(ShapeKind::Text) => Point {
                x: r.x + r.w * 0.9 * f64::from(p),
                y: centre.y,
            },
            _ => centre,
        })
    }
}

/// The pen at (`x`, `y`) on screen: a dot of ink in Claude's colours where
/// it draws, the twinkling sparkle above it and the session's name below,
/// `ms` into the drawing.
fn pen(x: f32, y: f32, title: &str, ms: f64, opacity: f32) -> AnyElement {
    let hue = ai::hue(ms / 2400.0);
    let tw = ai::twinkle(ms / ai::TWINKLE_MS);
    let name: String = if title.chars().count() > 32 {
        title.chars().take(31).chain(['…']).collect()
    } else {
        title.to_owned()
    };
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .size_0()
        .group_opacity(opacity)
        .child(
            div()
                .absolute()
                .left(px(-7.))
                .top(px(-7.))
                .size(px(14.))
                .rounded_full()
                .bg(gpui::Rgba { a: 0.25, ..hue }),
        )
        .child(
            div()
                .absolute()
                .left(px(-3.))
                .top(px(-3.))
                .size(px(6.))
                .rounded_full()
                .bg(hue),
        )
        .child(
            div()
                .absolute()
                .left(px(5.))
                .top(px(-23.))
                .child(ai::sparkle(16., 2., hue, tw)),
        )
        .child(
            div()
                .absolute()
                .left(px(12.))
                .top(px(9.))
                .h(px(20.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(hex(0x5e5ce6), 0.),
                    linear_color_stop(ai::SPARKLE, 1.),
                ))
                .text_color(gpui::white())
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .child(name),
        )
        .into_any_element()
}
