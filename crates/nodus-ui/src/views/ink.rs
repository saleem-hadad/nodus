//! Ink over live pages (spec 11 INKUI-025 over spec 04's pages, decision
//! D3). 0.9.4's ink layer was SVG above every card, its iframes included,
//! and passed the pointer through to them. Here a page is a native webview
//! above everything GPUI draws, so the marks that cross a live page are
//! drawn again above it, in a native overlay that never takes the pointer
//! (`mac_ink.rs`), the same as GPUI draws them around it: same paths and
//! widths at the zoom, same colours, the selection outline, labels set where
//! GPUI sets them. Under a snapshot the overlay goes with the page, and the
//! ink layer GPUI draws shows instead.

use std::rc::Rc;

use gpui::{App, Bounds, Font, FontWeight, Pixels, Point, Rgba, SharedString, TextRun, Window, px};
use nodus_board::ink_draw::{LABEL_FONT, LABEL_LINE, Pt, Seg};

use crate::board::canvas::BoardCanvas;
use crate::board::overlays::ink::{InkMark, InkScene};
use crate::theme::fonts;

/// The open board's ink this frame, and where the board is: its origin in
/// the window and its viewport (x, y, zoom).
#[derive(Clone)]
pub struct BoardInk {
    pub scene: Rc<InkScene>,
    pub origin: Point<Pixels>,
    pub viewport: (f64, f64, f64),
}

/// A mark in a page's own coordinates: logical px from its body's top left.
#[derive(Debug, Clone, PartialEq)]
pub enum PageMark {
    /// A pen line or arrow, round caps and joins.
    Line { segs: Vec<Seg>, width: f64, color: Rgba },
    /// A label: where its text starts and its baseline, its font size, its
    /// text and colour (the display font at 600, as `.ink-label`).
    Label {
        x: f64,
        baseline: f64,
        font_size: f64,
        text: SharedString,
        color: Rgba,
    },
    /// A dashed rounded rectangle (the selection outline): its box, corner
    /// radius, line width and dash.
    Dashed {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
        width: f64,
        dash: (f64, f64),
        color: Rgba,
    },
}

/// What a page's overlay shows: the marks over it, and the part of its body
/// the overlay's picture covers, in logical px snapped to device pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInk {
    /// x, y, w, h in the body's coordinates.
    pub rect: (f64, f64, f64, f64),
    /// Device pixels per logical px.
    pub scale: f64,
    pub marks: Vec<PageMark>,
}

/// Every board render: the ink as the layer draws it this frame, for the
/// frames' overlays (none while no view has a page, or there's no ink).
pub fn sync(canvas: &BoardCanvas, window: &Window, cx: &App) {
    let Some(rc) = super::frames::existing(cx) else { return };
    if rc.borrow().frames.is_empty() {
        rc.borrow_mut().ink = None;
        return;
    }
    let scene = canvas.ink_scene(window, cx);
    let vp = canvas.viewport;
    rc.borrow_mut().ink = (!scene.marks.is_empty()).then(|| BoardInk {
        scene: Rc::new(scene),
        origin: canvas.bounds.origin,
        viewport: (vp.x, vp.y, vp.zoom),
    });
}

/// Lengths are kept to 1/64 px, so a board that hasn't moved draws the same
/// picture and a page's overlay isn't drawn again for rounding noise.
fn q(v: f64) -> f64 {
    (v * 64.0).round() / 64.0
}

/// The marks of the board's ink over a page whose body is `document` (in
/// the window), or None when none cross it.
pub fn page_ink(ink: &BoardInk, document: Bounds<Pixels>, window: &Window) -> Option<PageInk> {
    let scale = f64::from(window.scale_factor());
    let (vx, vy, z) = ink.viewport;
    let (ox, oy) = (f64::from(f32::from(ink.origin.x)), f64::from(f32::from(ink.origin.y)));
    let (dx, dy) = (
        f64::from(f32::from(document.origin.x)),
        f64::from(f32::from(document.origin.y)),
    );
    let (dw, dh) = (
        f64::from(f32::from(document.size.width)),
        f64::from(f32::from(document.size.height)),
    );
    if dw < 1.0 || dh < 1.0 {
        return None;
    }
    // Board units to the body's px, as the ink layer places them.
    let at = |p: Pt| [q(ox + vx + p[0] * z - dx), q(oy + vy + p[1] * z - dy)];
    let mut marks = Vec::new();
    // The union of what the marks over the page cover, in the body's px.
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (b, mark) in &ink.scene.marks {
        let [bx, by] = at([b.x, b.y]);
        let (bw, bh) = (b.w * z, b.h * z);
        // A label's glyphs can reach a little past its box.
        let slack = if matches!(mark, InkMark::Label { .. }) {
            4.0
        } else {
            1.0
        };
        let (l, t, r, btm) = (bx - slack, by - slack, bx + bw + slack, by + bh + slack);
        if r <= 0.0 || btm <= 0.0 || l >= dw || t >= dh {
            continue;
        }
        x0 = x0.min(l);
        y0 = y0.min(t);
        x1 = x1.max(r);
        y1 = y1.max(btm);
        marks.push(match mark {
            InkMark::Line { segs, width, color } => PageMark::Line {
                segs: segs
                    .iter()
                    .map(|s| match *s {
                        Seg::Move(p) => Seg::Move(at(p)),
                        Seg::Line(p) => Seg::Line(at(p)),
                        Seg::Quad(c, p) => Seg::Quad(at(c), at(p)),
                    })
                    .collect(),
                width: q(width * z),
                color: *color,
            },
            InkMark::Selection { b, size, color } => {
                // `render_ink_layer`: 6 units out, 4-unit corners, a
                // non-scaling 1.5px line dashed 5 4.
                let pad = 6.0 * size;
                let [x, y] = at([b.x - pad, b.y - pad]);
                PageMark::Dashed {
                    x,
                    y,
                    w: q((b.w + 2.0 * pad) * z),
                    h: q((b.h + 2.0 * pad) * z),
                    radius: q(4.0 * size * z),
                    width: 1.5,
                    dash: (5.0, 4.0),
                    color: *color,
                }
            }
            InkMark::Label {
                x,
                y,
                size,
                text,
                color,
            } => {
                let s = size * z;
                let (font_size, line) = (LABEL_FONT * s, LABEL_LINE * s);
                // Where GPUI sets the label's baseline: Blink's rounding of
                // the line box at the window's device pixels.
                let top = oy + vy + y * z;
                let font = Font {
                    weight: FontWeight::SEMIBOLD,
                    ..gpui::font(fonts::DISPLAY)
                };
                // The line as GPUI shapes it: its ascent and descent are
                // its fonts' (fallbacks' included).
                let run = TextRun {
                    len: text.len(),
                    font,
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window
                    .text_system()
                    .shape_line(text.clone(), px(font_size as f32), &[run], None);
                let offset = gpui::chromium_baseline_offset(
                    px(top as f32),
                    px(line as f32),
                    shaped.ascent,
                    shaped.descent,
                    window.scale_factor(),
                );
                let [lx, _] = at([*x, *y]);
                PageMark::Label {
                    x: lx,
                    baseline: q(top + f64::from(f32::from(offset)) - dy),
                    font_size: q(font_size),
                    text: text.clone(),
                    color: *color,
                }
            }
        });
    }
    if marks.is_empty() {
        return None;
    }
    // The picture covers what the marks do, inside the body, on whole
    // device pixels.
    let snap_down = |v: f64| (v * scale).floor() / scale;
    let snap_up = |v: f64| (v * scale).ceil() / scale;
    let (l, t) = (snap_down(x0.max(0.0)), snap_down(y0.max(0.0)));
    let (r, b) = (snap_up(x1.min(dw)), snap_up(y1.min(dh)));
    if r - l < 0.5 || b - t < 0.5 {
        return None;
    }
    Some(PageInk {
        rect: (l, t, r - l, b - t),
        scale,
        marks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_lengths_to_a_64th() {
        assert_eq!(q(1.0 / 3.0), 21.0 / 64.0);
        assert_eq!(q(-2.5), -2.5);
    }
}
