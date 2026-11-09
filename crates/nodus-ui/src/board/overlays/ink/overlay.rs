//! The ink as a list of marks, for what draws it over views' live pages
//! (INKUI-025 over VIEW pages): 0.9.4's ink layer was SVG above every card,
//! iframes included, while the pages under it kept the pointer. A view's
//! page is a native webview above everything GPUI draws (decision D3), so
//! the views draw the marks that cross a live page again, in a native
//! overlay above it (`views::ink`). The marks are the ones `render_ink_layer`
//! draws, in its order: lines, the live stroke, the selection outline, then
//! labels.

use nodus_board::ink_draw::InkBox;

use super::*;

/// One thing the ink layer draws, in board units.
#[derive(Debug, Clone, PartialEq)]
pub enum InkMark {
    /// A pen line or an arrow: its path, its width, its colour.
    Line { segs: Vec<Seg>, width: f64, color: Rgba },
    /// A label: its top left, its size (the stroke's), its text, its colour.
    Label {
        x: f64,
        y: f64,
        size: f64,
        text: SharedString,
        color: Rgba,
    },
    /// The selected stroke's outline: the stroke's box, its size, the accent.
    Selection { b: InkBox, size: f64, color: Rgba },
}

/// The ink the layer draws this frame: each mark with the box it covers,
/// in board units.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InkScene {
    pub marks: Vec<(InkBox, InkMark)>,
}

impl BoardCanvas {
    /// What `render_ink_layer` draws this frame, as marks.
    pub(crate) fn ink_scene(&self, window: &Window, cx: &App) -> InkScene {
        let t = theme::tokens(cx);
        let ink = &self.overlays.ink;
        let mut shown = self.ink_strokes();
        if let Some(InkGesture::Move {
            id,
            dx,
            dy,
            moved: true,
            ..
        }) = &ink.gesture
        {
            for s in &mut shown {
                if &s.id == id {
                    *s = translate(s, *dx, *dy);
                }
            }
        }
        let erasing: Vec<String> = match &ink.gesture {
            Some(InkGesture::Erase { gone, .. }) => gone.clone(),
            _ => Vec::new(),
        };
        let live = match &ink.gesture {
            Some(InkGesture::Draw { base, pts, .. }) => Some(InkStroke {
                points: flatten(pts),
                ..base.clone()
            }),
            _ => None,
        };
        let editing = ink.label.as_ref().and_then(|l| l.id.clone());
        let measure = measurer(window);
        let color = |s: &InkStroke| {
            let mut color = ink_rgba(&t, s.color);
            if erasing.contains(&s.id) {
                color.a *= 0.25;
            }
            color
        };
        let mut marks = Vec::new();
        for s in shown.iter().filter(|s| s.kind != InkKind::Text).chain(live.iter()) {
            marks.push((
                stroke_box(s, &measure),
                InkMark::Line {
                    segs: ink_draw::stroke_path(s),
                    width: PEN_WIDTH * s.size,
                    color: color(s),
                },
            ));
        }
        if let Some(s) = ink.selected.as_ref().and_then(|id| shown.iter().find(|s| &s.id == id)) {
            let b = stroke_box(s, &measure);
            // The outline sits 6 units out, and its 1.5px line doesn't scale.
            let pad = 6.0 * s.size + 2.0;
            marks.push((
                InkBox {
                    x: b.x - pad,
                    y: b.y - pad,
                    w: b.w + 2.0 * pad,
                    h: b.h + 2.0 * pad,
                },
                InkMark::Selection {
                    b,
                    size: s.size,
                    color: t.accent,
                },
            ));
        }
        for s in shown
            .iter()
            .filter(|s| s.kind == InkKind::Text && Some(&s.id) != editing.as_ref())
        {
            let b = stroke_box(s, &measure);
            marks.push((
                b,
                InkMark::Label {
                    x: s.points[0],
                    y: s.points.get(1).copied().unwrap_or(0.0),
                    size: s.size,
                    text: s.text.clone().unwrap_or_default().into(),
                    color: color(s),
                },
            ));
        }
        InkScene { marks }
    }
}
