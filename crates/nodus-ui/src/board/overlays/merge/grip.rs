//! The instruction field's resize grip (MERGE-013: `resize: vertical`):
//! Chromium's two diagonal strokes at the bottom-right corner, dragged to
//! make the field taller or shorter.

use gpui::{
    AnyElement, Context, DispatchPhase, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, PathBuilder, Pixels, Rgba, Styled, canvas, div, point, px,
};

use crate::board::canvas::BoardCanvas;
use crate::theme::tokens::{Tokens, rgba};

/// The field's height at 3 rows: 3 × 18.85 + 20 padding, as Chromium lays out
/// `rows="3"` at 13px / 1.45.
pub(crate) const INSTRUCTION_H: f32 = 3.0 * 18.85 + 20.0;
/// The least it shrinks to: its padding and one line.
const MIN_H: f32 = 18.85 + 20.0;

/// A drag of the grip: where it started and the height then.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GripDrag {
    pub start_y: Pixels,
    pub start_h: f32,
}

impl BoardCanvas {
    /// The grip, 12px square in the field's bottom-right corner.
    pub(crate) fn render_instruction_grip(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let dark = t.dark;
        let (line, shade): (Rgba, Option<Rgba>) = if dark {
            (rgba(255, 255, 255, 0.62), Some(rgba(0, 0, 0, 0.9)))
        } else {
            (rgba(0, 0, 0, 0.55), None)
        };
        let me = cx.entity();
        let press = cx.entity().downgrade();
        let dragging = self.overlays.merge.as_ref().is_some_and(|d| d.grip.is_some());
        let strokes = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                // The corner, inside the field's border.
                let c = point(
                    bounds.origin.x + bounds.size.width,
                    bounds.origin.y + bounds.size.height,
                );
                let seg = |a: (f32, f32), b: (f32, f32), dx: f32, dy: f32, color: Rgba, window: &mut gpui::Window| {
                    let mut path = PathBuilder::stroke(px(1.));
                    path.move_to(point(c.x + px(a.0 + dx), c.y + px(a.1 + dy)));
                    path.line_to(point(c.x + px(b.0 + dx), c.y + px(b.1 + dy)));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                };
                let long = ((-7.8, -1.45), (-0.9, -8.35));
                let short = ((-3.75, -1.15), (-0.9, -3.95));
                for (a, b) in [long, short] {
                    if let Some(shade) = shade {
                        seg(a, b, -0.5, -0.5, shade, window);
                    }
                    seg(a, b, 0., 0., line, window);
                }
                if dragging {
                    let m = me.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            m.update(cx, |c, cx| c.grip_drag(e.position.y, cx));
                        }
                    });
                    let m = me.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            m.update(cx, |c, cx| {
                                if let Some(d) = c.overlays.merge.as_mut() {
                                    d.grip = None;
                                    cx.notify();
                                }
                            });
                        }
                    });
                }
            },
        )
        .size_full();
        div()
            .id("merge-instruction-grip")
            .absolute()
            .right(px(0.5))
            .bottom(px(0.5))
            .size(px(12.))
            .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                if let Some(c) = press.upgrade() {
                    c.update(cx, |c, cx| {
                        if let Some(d) = c.overlays.merge.as_mut() {
                            d.grip = Some(GripDrag {
                                start_y: e.position.y,
                                start_h: d.instruction_h.unwrap_or(INSTRUCTION_H),
                            });
                            cx.notify();
                        }
                    });
                }
            })
            .child(strokes)
            .into_any_element()
    }

    fn grip_drag(&mut self, y: Pixels, cx: &mut Context<'_, Self>) {
        let Some(d) = self.overlays.merge.as_mut() else { return };
        let Some(g) = d.grip else { return };
        d.instruction_h = Some((g.start_h + f32::from(y - g.start_y)).max(MIN_H));
        cx.notify();
    }
}
