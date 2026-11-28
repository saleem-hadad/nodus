//! Shapes and links on the board (spec 14): the canvas's side of them.
//! Changes made here show at once and are saved, and ⌘Z takes them back
//! with the ink (SHAPE-040). Links run between the boxes the canvas draws,
//! so they follow a card, note, view or shape as it moves.

use nodus_board::drawing::{Change, Outcome};
use nodus_board::geometry::Point;
use nodus_board::shapes::SHAPE_CORNER;
use nodus_domain::{BoardLink, BoardShape, InkColor, LinkEnd, Rect, ShapeKind};

use super::canvas::BoardCanvas;
use super::scene::{FlowNode, NodeKind};
use gpui::Context;

/// How close the pointer must come to a link to take it, in screen px.
pub const LINK_HIT_PX: f64 = 6.0;
/// How thick a link is drawn, and its arrowhead, in board units.
pub const LINK_WIDTH: f64 = 1.75;
pub const LINK_HEAD: f64 = 10.0;
/// A link drawn shorter than this on screen, both ends loose, is a slip.
pub const MIN_LINK_PX: f64 = 8.0;

pub use nodus_board::routes::{End, at, distance, head, line_part, route};

/// How much of a rectangle, ellipse or diamond Claude has drawn `p` (0…1)
/// through it (MOTION-021): its outline first, then its fill and words.
pub fn drawn_share(p: f32) -> (f32, f32, f32) {
    use crate::theme::motion::{ease_in_out, ease_out};
    let outline = ease_in_out((p / 0.7).min(1.0));
    let fill = ease_out(((p - 0.45) / 0.4).clamp(0.0, 1.0));
    let words = ((p - 0.55) / 0.45).clamp(0.0, 1.0);
    (outline, fill, words)
}

/// A rectangle's (rounded corners), ellipse's or diamond's outline as a
/// line, clockwise from where Claude starts it: the top-left corner's end
/// of the top edge, or the top.
pub fn outline_line(kind: ShapeKind, r: Rect) -> Vec<Point> {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let pt = |x: f64, y: f64| Point { x, y };
    match kind {
        ShapeKind::Ellipse => {
            let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
            (0..=72)
                .map(|i| {
                    let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 72.0;
                    pt(cx + rx * a.cos(), cy + ry * a.sin())
                })
                .collect()
        }
        ShapeKind::Diamond => vec![
            pt(x + w / 2.0, y),
            pt(x + w, y + h / 2.0),
            pt(x + w / 2.0, y + h),
            pt(x, y + h / 2.0),
            pt(x + w / 2.0, y),
        ],
        _ => {
            use std::f64::consts::{FRAC_PI_2, PI};
            let rad = SHAPE_CORNER.min(w / 2.0).min(h / 2.0);
            let arc = |cx: f64, cy: f64, from: f64| {
                (1..=8).map(move |i| {
                    let a = from + FRAC_PI_2 * f64::from(i) / 8.0;
                    pt(cx + rad * a.cos(), cy + rad * a.sin())
                })
            };
            let mut line = vec![pt(x + rad, y), pt(x + w - rad, y)];
            line.extend(arc(x + w - rad, y + rad, -FRAC_PI_2));
            line.push(pt(x + w, y + h - rad));
            line.extend(arc(x + w - rad, y + h - rad, 0.0));
            line.push(pt(x + rad, y + h));
            line.extend(arc(x + rad, y + h - rad, FRAC_PI_2));
            line.push(pt(x, y + rad));
            line.extend(arc(x + rad, y + rad, PI));
            line
        }
    }
}

/// Whether a link can hold on to a node: an active card, a free note, a
/// view or a shape, drawn.
fn attachable(node: &FlowNode) -> bool {
    !node.hidden
        && match node.kind {
            NodeKind::Card { stack, .. } => stack.is_none(),
            NodeKind::Note { index } => index.is_some(),
            NodeKind::View { .. } | NodeKind::Shape { .. } => true,
            NodeKind::Archive { .. }
            | NodeKind::Pad
            | NodeKind::Section { .. }
            | NodeKind::Tile { .. }
            | NodeKind::Goal { .. } => false,
        }
}

/// A square-ish box from two corners; Shift keeps it square (a circle).
pub fn box_between(a: Point, b: Point, square: bool) -> Rect {
    let (mut w, mut h) = (b.x - a.x, b.y - a.y);
    if square {
        let side = w.abs().max(h.abs());
        w = side.copysign(if w == 0.0 { 1.0 } else { w });
        h = side.copysign(if h == 0.0 { 1.0 } else { h });
    }
    Rect {
        x: a.x.min(a.x + w),
        y: a.y.min(a.y + h),
        w: w.abs(),
        h: h.abs(),
    }
}

/// A new shape's size when it's placed with a click, in board units at
/// zoom 1 (the dock's tools scale it to keep its size on screen).
pub fn default_size(kind: ShapeKind) -> (f64, f64) {
    match kind {
        ShapeKind::Rect | ShapeKind::Ellipse => (180.0, 96.0),
        ShapeKind::Diamond => (160.0, 112.0),
        ShapeKind::Text => (240.0, 40.0),
        ShapeKind::Image => (320.0, 240.0),
    }
}

impl BoardCanvas {
    // ---- what's on the board ---------------------------------------------------------

    pub(crate) fn shapes(&self) -> &[BoardShape] {
        self.board.as_ref().map_or(&[], |b| &b.shapes)
    }

    pub(crate) fn links(&self) -> &[BoardLink] {
        self.board.as_ref().map_or(&[], |b| &b.links)
    }

    pub(crate) fn shape(&self, id: &str) -> Option<&BoardShape> {
        self.shapes().iter().find(|s| s.id == id)
    }

    pub(crate) fn link(&self, id: &str) -> Option<&BoardLink> {
        self.links().iter().find(|l| l.id == id)
    }

    pub(crate) fn selected_shapes(&self) -> Vec<String> {
        self.selected_ids(|k| matches!(k, NodeKind::Shape { .. }))
    }

    // ---- changes ---------------------------------------------------------------------

    /// What a change leaves goes on the board at once and is saved.
    pub(crate) fn apply_outcome(&mut self, out: Outcome, cx: &mut Context<'_, Self>) {
        for board in [self.board.as_mut(), self.source.as_mut()].into_iter().flatten() {
            for s in &out.save_shapes {
                match board.shapes.iter_mut().find(|x| x.id == s.id) {
                    Some(existing) => *existing = s.clone(),
                    None => board.shapes.push(s.clone()),
                }
            }
            board.shapes.retain(|s| !out.remove_shapes.contains(&s.id));
            for l in &out.save_links {
                match board.links.iter_mut().find(|x| x.id == l.id) {
                    Some(existing) => *existing = l.clone(),
                    None => board.links.push(l.clone()),
                }
            }
            board
                .links
                .retain(|l| !out.remove_links.contains(&l.id) && !out.remove_shapes.iter().any(|id| l.touches(id)));
        }
        if let Some(link) = &self.overlays.ink.link
            && (out.remove_links.contains(link) || self.link(link).is_none())
        {
            self.overlays.ink.link = None;
        }
        let Outcome {
            save_shapes,
            remove_shapes,
            save_links,
            remove_links,
        } = out;
        self.state.update(cx, |s, cx| {
            s.save_shapes(save_shapes, cx);
            s.save_links(save_links, cx);
            s.remove_links(remove_links, cx);
            s.remove_shapes(remove_shapes, cx);
        });
        self.rebuild(cx);
    }

    /// A change ⌘Z can take back.
    pub(crate) fn commit_change(&mut self, change: Change, cx: &mut Context<'_, Self>) {
        if change.is_empty() {
            return;
        }
        let out = change.outcome();
        self.overlays.ink.history.commit(None, change);
        self.apply_outcome(out, cx);
    }

    /// SHAPE-002: a new shape, selected; with `edit`, its words open for writing.
    pub(crate) fn add_shape(
        &mut self,
        kind: ShapeKind,
        rect: Rect,
        text: String,
        image: Option<String>,
        edit: bool,
        cx: &mut Context<'_, Self>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let shape = BoardShape {
            id: id.clone(),
            kind,
            x: nodus_domain::js::round(rect.x),
            y: nodus_domain::js::round(rect.y),
            w: nodus_domain::js::round(rect.w.max(nodus_board::shapes::SHAPE_MIN)),
            h: nodus_domain::js::round(rect.h.max(nodus_board::shapes::SHAPE_MIN)),
            color: self.overlays.ink.shape_color(),
            text,
            image,
            by: None,
            created_at: crate::parity::now_ms(cx),
            extra: serde_json::Map::new(),
        };
        let mut change = Change::default();
        change.shape(None, Some(shape));
        self.commit_change(change, cx);
        self.overlays.ink.link = None;
        for n in &mut self.nodes {
            n.selected = n.id == id;
        }
        self.selection_rect = false;
        self.sync_selection(cx);
        if edit {
            self.set_renaming(Some(id.clone()), cx);
        }
        cx.notify();
        id
    }

    /// SHAPE-020: shapes off the board, and every link they held, as one change.
    pub(crate) fn delete_shapes(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let mut change = Change::default();
        for id in ids {
            if let Some(s) = self.shape(id).cloned() {
                change.shape(Some(s), None);
            }
        }
        for l in self.links().to_vec() {
            if ids.iter().any(|id| l.touches(id)) {
                change.link(Some(l), None);
            }
        }
        self.commit_change(change, cx);
    }

    pub(crate) fn delete_link(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let mut change = Change::default();
        change.link(self.link(id).cloned(), None);
        self.overlays.ink.link = None;
        self.commit_change(change, cx);
    }

    /// SHAPE-012: the words written in a shape. Words on their own that
    /// were emptied go.
    pub(crate) fn set_shape_text(&mut self, id: &str, text: &str, cx: &mut Context<'_, Self>) {
        let Some(before) = self.shape(id).cloned() else { return };
        let text = nodus_domain::js::trim_end(text).to_owned();
        if before.kind == ShapeKind::Text && nodus_domain::js::trim(&text).is_empty() {
            self.delete_shapes(&[id.to_owned()], cx);
            return;
        }
        let mut change = Change::default();
        change.shape(Some(before.clone()), Some(BoardShape { text, ..before }));
        self.commit_change(change, cx);
    }

    /// LINK-020: a link's words.
    pub(crate) fn set_link_label(&mut self, id: &str, label: &str, cx: &mut Context<'_, Self>) {
        let Some(before) = self.link(id).cloned() else { return };
        let label = nodus_domain::js::trim(label).to_owned();
        let mut change = Change::default();
        change.link(Some(before.clone()), Some(BoardLink { label, ..before }));
        self.commit_change(change, cx);
    }

    /// SHAPE-015, LINK-021: the palette's colour on the selected shapes and link.
    pub(crate) fn recolor_selected(&mut self, color: InkColor, cx: &mut Context<'_, Self>) {
        let mut change = Change::default();
        for id in self.selected_shapes() {
            if let Some(s) = self.shape(&id).cloned()
                && s.color != color
            {
                change.shape(Some(s.clone()), Some(BoardShape { color, ..s }));
            }
        }
        if let Some(l) = self.overlays.ink.link.as_deref().and_then(|id| self.link(id)).cloned()
            && l.color != color
        {
            change.link(Some(l.clone()), Some(BoardLink { color, ..l }));
        }
        self.commit_change(change, cx);
    }

    /// SHAPE-010: shapes dragged to where they were dropped.
    pub(crate) fn move_shapes(&mut self, moved: &[(String, Point)], cx: &mut Context<'_, Self>) {
        let mut change = Change::default();
        for (id, to) in moved {
            if let Some(s) = self.shape(id).cloned() {
                let x = nodus_domain::js::round(to.x);
                let y = nodus_domain::js::round(to.y);
                change.shape(Some(s.clone()), Some(BoardShape { x, y, ..s }));
            }
        }
        self.commit_change(change, cx);
    }

    /// SHAPE-011: a shape resized from its edges.
    pub(crate) fn resize_shape(&mut self, id: &str, rect: Rect, cx: &mut Context<'_, Self>) {
        let Some(s) = self.shape(id).cloned() else { return };
        let mut change = Change::default();
        change.shape(
            Some(s.clone()),
            Some(BoardShape {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                ..s
            }),
        );
        self.commit_change(change, cx);
    }

    // ---- where links run -------------------------------------------------------------

    /// The box a link holds on to, as drawn now; none while it isn't drawn
    /// (an archived card, a note stuck on a card).
    pub(crate) fn attach_rect(&self, id: &str) -> Option<Rect> {
        self.node(id).filter(|n| attachable(n)).map(FlowNode::rect)
    }

    /// The top thing at a board point a link can hold on to, but `except`.
    pub(crate) fn attachable_at(&self, p: Point, except: Option<&str>) -> Option<String> {
        super::scene::paint_order(&self.nodes)
            .into_iter()
            .rev()
            .map(|i| &self.nodes[i])
            .filter(|n| attachable(n) && Some(n.id.as_str()) != except)
            .find(|n| {
                let r = n.rect();
                p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h
            })
            .map(|n| n.id.clone())
    }

    fn end(&self, end: &LinkEnd) -> Option<End> {
        match end.node.as_deref() {
            Some(id) => self.attach_rect(id).map(End::Node),
            None => Some(End::Point(Point { x: end.x, y: end.y })),
        }
    }

    /// LINK-001: where a link runs now, as a line; none while either end
    /// isn't drawn.
    pub(crate) fn link_line(&self, link: &BoardLink) -> Option<Vec<Point>> {
        Some(route(self.end(&link.from)?, self.end(&link.to)?))
    }

    /// The top link within `tol` board units of a point.
    pub(crate) fn link_at(&self, p: Point, tol: f64) -> Option<String> {
        self.links()
            .iter()
            .rev()
            .find(|l| self.link_line(l).is_some_and(|line| distance(&line, p) <= tol))
            .map(|l| l.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_from_corners_either_way() {
        let r = box_between(Point { x: 100.0, y: 80.0 }, Point { x: 20.0, y: 10.0 }, false);
        assert_eq!(
            r,
            Rect {
                x: 20.0,
                y: 10.0,
                w: 80.0,
                h: 70.0
            }
        );
        let sq = box_between(Point { x: 0.0, y: 0.0 }, Point { x: -30.0, y: 10.0 }, true);
        assert_eq!(
            sq,
            Rect {
                x: -30.0,
                y: 0.0,
                w: 30.0,
                h: 30.0
            }
        );
    }
}
