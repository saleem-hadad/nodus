//! Geometry for the ink drawn on the board: paths to draw, bounds, what's
//! under the pointer, and the undo history. Everything is in board
//! coordinates.
//!
//! Ports `src/renderer/src/components/board/ink.ts` and the history of
//! `InkLayer.tsx` (spec 11, Data & formats › Ink).

use std::collections::HashSet;

use nodus_domain::json::number_to_string;
use nodus_domain::{InkKind, InkStroke, js};

/// A pen line and an arrow's shaft, and its head, at size 1 (board units).
pub const PEN_WIDTH: f64 = 3.0;
pub const ARROW_HEAD: f64 = 14.0;
/// A label's type at size 1.
pub const LABEL_FONT: f64 = 16.0;
pub const LABEL_LINE: f64 = 20.0;
/// How many changes ⌘Z can take back.
pub const HISTORY: usize = 100;
/// How close to a stroke the pointer counts as on it, in screen pixels.
pub const HIT_PX: f64 = 6.0;
/// Points closer together than this on screen add nothing to a line.
pub const MIN_STEP_PX: f64 = 1.5;
/// An arrow shorter than this on screen is a slip, not an arrow.
pub const MIN_ARROW_PX: f64 = 8.0;
/// How far the pointer travels on screen before a stroke follows it.
pub const MOVE_PX: f64 = 3.0;

pub type Pt = [f64; 2];

/// A box in board units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InkBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// How wide a label's text is at a size (`labelWidth`): the display font,
/// weight 600, `16 × size` px. The UI measures it; tests use [`fallback_width`].
pub type Measure<'a> = &'a dyn Fn(&str, f64) -> f64;

/// `labelWidth` with no text measurer: `length × 16·size × 0.56`, the
/// length in UTF-16 units.
pub fn fallback_width(text: &str, size: f64) -> f64 {
    js::utf16_len(text) as f64 * LABEL_FONT * size * 0.56
}

/// `Math.round(n * 10) / 10`.
pub fn r1(n: f64) -> f64 {
    js::round(n * 10.0) / 10.0
}

/// Ink keeps the thickness it had on screen when it was drawn (INKUI-010).
pub fn size_at(zoom: f64) -> f64 {
    js::round((1.0 / zoom).clamp(0.5, 4.0) * 100.0) / 100.0
}

pub fn points_of(s: &InkStroke) -> Vec<Pt> {
    s.points.chunks_exact(2).map(|p| [p[0], p[1]]).collect()
}

pub fn flatten(pts: &[Pt]) -> Vec<f64> {
    pts.iter().flat_map(|&[x, y]| [r1(x), r1(y)]).collect()
}

pub fn distance(a: Pt, b: Pt) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// How far `p` is from the segment `a`–`b`.
pub fn segment_distance(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx * dx + dy * dy;
    let t = if len != 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// One step of a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(Pt),
    Line(Pt),
    /// A quadratic curve: control, end.
    Quad(Pt, Pt),
}

/// A smooth path through a pen stroke: curves between the midpoints, so its
/// corners round off. A single point is a dot (a 0.01-long line).
pub fn pen_path(pts: &[Pt]) -> Vec<Seg> {
    let Some(&first) = pts.first() else {
        return Vec::new();
    };
    match pts.len() {
        1 => vec![Seg::Move(first), Seg::Line([first[0] + 0.01, first[1]])],
        2 => vec![Seg::Move(first), Seg::Line(pts[1])],
        n => {
            let mut d = vec![Seg::Move(first)];
            for i in 1..n - 1 {
                let ([x, y], [nx, ny]) = (pts[i], pts[i + 1]);
                d.push(Seg::Quad([x, y], [r1((x + nx) / 2.0), r1((y + ny) / 2.0)]));
            }
            d.push(Seg::Line(pts[n - 1]));
            d
        }
    }
}

/// `penPath`'s SVG text, as 0.9.4 wrote it.
pub fn pen_path_string(pts: &[Pt]) -> String {
    let n = |v: f64| number_to_string(v);
    match pts.len() {
        0 => String::new(),
        1 => format!("M{} {}h0.01", n(pts[0][0]), n(pts[0][1])),
        _ => pen_path(pts)
            .iter()
            .map(|seg| match *seg {
                Seg::Move([x, y]) => format!("M{} {}", n(x), n(y)),
                Seg::Line([x, y]) => format!("L{} {}", n(x), n(y)),
                Seg::Quad([cx, cy], [x, y]) => format!("Q{} {} {} {}", n(cx), n(cy), n(x), n(y)),
            })
            .collect(),
    }
}

/// A straight arrow from `a` to `b`, its head two strokes back from the tip.
pub fn arrow_path(a: Pt, b: Pt, head: f64) -> Vec<Seg> {
    let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
    let wing = |turn: f64| {
        [
            r1(b[0] - head * (angle + turn).cos()),
            r1(b[1] - head * (angle + turn).sin()),
        ]
    };
    vec![
        Seg::Move(a),
        Seg::Line(b),
        Seg::Move(wing(-0.45)),
        Seg::Line(b),
        Seg::Line(wing(0.45)),
    ]
}

/// `arrowPath`'s SVG text.
pub fn arrow_path_string(a: Pt, b: Pt, head: f64) -> String {
    let n = |v: f64| number_to_string(v);
    arrow_path(a, b, head)
        .iter()
        .map(|seg| match *seg {
            Seg::Move([x, y]) => format!("M{} {}", n(x), n(y)),
            Seg::Line([x, y]) => format!("L{} {}", n(x), n(y)),
            Seg::Quad(..) => unreachable!("arrows are straight"),
        })
        .collect()
}

/// The path a stroke draws: an arrow's shaft and head, or a pen line.
pub fn stroke_path(s: &InkStroke) -> Vec<Seg> {
    let pts = points_of(s);
    if s.kind == InkKind::Arrow && pts.len() > 1 {
        arrow_path(pts[0], pts[1], ARROW_HEAD * s.size)
    } else {
        pen_path(&pts)
    }
}

/// Drops the points that stray less than `tolerance` from the line through
/// their neighbours (Douglas–Peucker).
pub fn simplify(pts: &[Pt], tolerance: f64) -> Vec<Pt> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((from, to)) = stack.pop() {
        let mut worst: isize = -1;
        let mut far = 0.0;
        for (i, &p) in pts.iter().enumerate().take(to).skip(from + 1) {
            let d = segment_distance(p, pts[from], pts[to]);
            if d > far {
                far = d;
                worst = i as isize;
            }
        }
        if worst > 0 && far > tolerance {
            let w = worst as usize;
            keep[w] = true;
            stack.push((from, w));
            stack.push((w, to));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// The box a stroke covers, its line's thickness included.
pub fn stroke_box(s: &InkStroke, measure: Measure<'_>) -> InkBox {
    let pts = points_of(s);
    if s.kind == InkKind::Text {
        let [x, y] = pts.first().copied().unwrap_or([0.0, 0.0]);
        return InkBox {
            x,
            y,
            w: measure(s.text.as_deref().unwrap_or(""), s.size),
            h: LABEL_LINE * s.size,
        };
    }
    let pad = if s.kind == InkKind::Arrow {
        ARROW_HEAD
    } else {
        PEN_WIDTH / 2.0
    } * s.size;
    let min = |f: fn(&Pt) -> f64| pts.iter().map(f).fold(f64::INFINITY, f64::min);
    let max = |f: fn(&Pt) -> f64| pts.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
    let x = min(|p| p[0]) - pad;
    let y = min(|p| p[1]) - pad;
    InkBox {
        x,
        y,
        w: max(|p| p[0]) + pad - x,
        h: max(|p| p[1]) + pad - y,
    }
}

/// Whether `p` is on the stroke, or within `tolerance` of its line.
pub fn hit_stroke(s: &InkStroke, p: Pt, tolerance: f64, measure: Measure<'_>) -> bool {
    let b = stroke_box(s, measure);
    if p[0] < b.x - tolerance || p[0] > b.x + b.w + tolerance || p[1] < b.y - tolerance || p[1] > b.y + b.h + tolerance
    {
        return false;
    }
    // A label is hit anywhere in its box.
    if s.kind == InkKind::Text {
        return true;
    }
    let pts = points_of(s);
    let reach = tolerance + PEN_WIDTH * s.size / 2.0;
    if pts.len() == 1 {
        return distance(p, pts[0]) <= reach;
    }
    if pts.windows(2).any(|w| segment_distance(p, w[0], w[1]) <= reach) {
        return true;
    }
    // An arrow's head is part of it too.
    s.kind == InkKind::Arrow && pts.len() > 1 && distance(p, pts[1]) <= ARROW_HEAD * s.size
}

/// The topmost stroke under `p`. Labels draw above lines, so they're found
/// first.
pub fn stroke_at<'a>(strokes: &'a [InkStroke], p: Pt, tolerance: f64, measure: Measure<'_>) -> Option<&'a InkStroke> {
    let text = strokes
        .iter()
        .rev()
        .find(|s| s.kind == InkKind::Text && hit_stroke(s, p, tolerance, measure));
    text.or_else(|| {
        strokes
            .iter()
            .rev()
            .find(|s| s.kind != InkKind::Text && hit_stroke(s, p, tolerance, measure))
    })
}

/// The strokes an eraser dragged from `a` to `b` touches, in stroke order.
pub fn strokes_along(strokes: &[InkStroke], a: Pt, b: Pt, tolerance: f64, measure: Measure<'_>) -> Vec<String> {
    let steps = (distance(a, b) / (tolerance / 2.0).max(0.5)).ceil().max(1.0) as usize;
    strokes
        .iter()
        .filter(|s| {
            (0..=steps).any(|i| {
                let t = i as f64 / steps as f64;
                hit_stroke(
                    s,
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                    tolerance,
                    measure,
                )
            })
        })
        .map(|s| s.id.clone())
        .collect()
}

pub fn translate(s: &InkStroke, dx: f64, dy: f64) -> InkStroke {
    InkStroke {
        points: s
            .points
            .iter()
            .enumerate()
            .map(|(i, &n)| r1(n + if i % 2 == 0 { dx } else { dy }))
            .collect(),
        ..s.clone()
    }
}

/// The ink's undo history (`past` and `future` in InkLayer.tsx): whole lists
/// of strokes, up to [`HISTORY`] changes back.
#[derive(Debug, Clone, Default)]
pub struct InkHistory {
    past: Vec<Vec<InkStroke>>,
    future: Vec<Vec<InkStroke>>,
}

impl InkHistory {
    /// A change from `current`: it can be taken back, and the redo list
    /// goes (INKUI-021).
    pub fn commit(&mut self, current: Vec<InkStroke>) {
        self.past.push(current);
        if self.past.len() > HISTORY {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// What ⌘Z goes back to from `current`, if anything.
    pub fn undo(&mut self, current: Vec<InkStroke>) -> Option<Vec<InkStroke>> {
        let prev = self.past.pop()?;
        self.future.push(current);
        Some(prev)
    }

    /// What ⇧⌘Z goes forward to. Redo pushes onto the undo list without
    /// trimming it, as 0.9.4 did (spec 11, Edge cases).
    pub fn redo(&mut self, current: Vec<InkStroke>) -> Option<Vec<InkStroke>> {
        let next = self.future.pop()?;
        self.past.push(current);
        Some(next)
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn len(&self) -> usize {
        self.past.len()
    }

    pub fn is_empty(&self) -> bool {
        self.past.is_empty()
    }
}

/// The ids in `gone` as a set (the eraser's marks).
pub fn id_set(ids: impl IntoIterator<Item = String>) -> HashSet<String> {
    ids.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::InkColor;
    use serde_json::Map;

    fn stroke(id: &str, kind: InkKind, color: InkColor, points: &[f64], size: f64, text: Option<&str>) -> InkStroke {
        InkStroke {
            id: id.into(),
            kind,
            color,
            points: points.to_vec(),
            size,
            text: text.map(str::to_owned),
            extra: Map::new(),
        }
    }

    fn pen(id: &str, points: &[f64], size: f64) -> InkStroke {
        stroke(id, InkKind::Pen, InkColor::Red, points, size, None)
    }

    const M: Measure<'static> = &fallback_width;

    // penPath · draws a dot, a line, and curves through the midpoints
    #[test]
    fn pen_path_draws_a_dot_a_line_and_curves_through_the_midpoints() {
        assert_eq!(pen_path_string(&[[1., 2.]]), "M1 2h0.01");
        assert_eq!(pen_path_string(&[[0., 0.], [10., 0.]]), "M0 0L10 0");
        assert_eq!(
            pen_path_string(&[[0., 0.], [10., 0.], [10., 10.]]),
            "M0 0Q10 0 10 5L10 10"
        );
    }

    // arrowPath · puts the head on the tip
    #[test]
    fn arrow_path_puts_the_head_on_the_tip() {
        let d = arrow_path_string([0., 0.], [100., 0.], 10.);
        assert!(d.starts_with("M0 0L100 0M"), "{d}");
        assert!(d.contains("L100 0L"), "{d}");
    }

    // simplify · drops points on a straight line and keeps corners
    #[test]
    fn simplify_drops_points_on_a_straight_line_and_keeps_corners() {
        let line = [[0., 0.], [1., 0.01], [2., 0.], [3., 0.02], [4., 0.]];
        assert_eq!(simplify(&line, 0.5), vec![[0., 0.], [4., 0.]]);
        let corner = [[0., 0.], [5., 0.], [10., 0.], [10., 5.], [10., 10.]];
        assert_eq!(simplify(&corner, 0.5), vec![[0., 0.], [10., 0.], [10., 10.]]);
    }

    // hitStroke · hits near the line, not away from it
    #[test]
    fn hit_stroke_hits_near_the_line_not_away_from_it() {
        let line = pen("a", &[0., 0., 100., 0.], 1.);
        assert!(hit_stroke(&line, [50., 4.], 3., M));
        assert!(!hit_stroke(&line, [50., 9.], 3., M));
        assert!(!hit_stroke(&line, [120., 0.], 3., M));
    }

    // hitStroke · reaches further for thicker ink
    #[test]
    fn hit_stroke_reaches_further_for_thicker_ink() {
        assert!(hit_stroke(&pen("b", &[0., 0., 100., 0.], 4.), [50., 9.], 3., M));
    }

    // hitStroke · hits a label anywhere in its box
    #[test]
    fn hit_stroke_hits_a_label_anywhere_in_its_box() {
        let label = stroke("t", InkKind::Text, InkColor::Blue, &[10., 10.], 1., Some("Ship it"));
        assert!((stroke_box(&label, M).w - 62.72).abs() < 1e-9);
        assert!(hit_stroke(&label, [14., 20.], 0., M));
        assert!(!hit_stroke(&label, [14., 40.], 0., M));
    }

    // strokeAt · finds the topmost stroke, labels first
    #[test]
    fn stroke_at_finds_the_topmost_stroke_labels_first() {
        let under = pen("under", &[0., 0., 100., 0.], 1.);
        let over = pen("over", &[0., 0., 100., 0.], 1.);
        let both = [under.clone(), over];
        assert_eq!(stroke_at(&both, [50., 0.], 3., M).map(|s| s.id.as_str()), Some("over"));
        let label = stroke("t", InkKind::Text, InkColor::Blue, &[40., -10.], 1., Some("here"));
        let with_label = [label, under.clone()];
        assert_eq!(
            stroke_at(&with_label, [45., 0.], 3., M).map(|s| s.id.as_str()),
            Some("t")
        );
        assert!(stroke_at(&[under], [50., 50.], 3., M).is_none());
    }

    // strokesAlong · catches strokes the eraser passes over between two pointer events
    #[test]
    fn strokes_along_catches_strokes_the_eraser_passes_over_between_two_pointer_events() {
        let strokes = [
            pen("a", &[50., -20., 50., 20.], 1.),
            pen("b", &[200., -20., 200., 20.], 1.),
        ];
        assert_eq!(
            strokes_along(&strokes, [0., 0.], [100., 0.], 3., M),
            vec!["a".to_owned()]
        );
    }

    // translate · moves every point
    #[test]
    fn translate_moves_every_point() {
        assert_eq!(
            translate(&pen("a", &[0., 0., 10., 20.], 1.), 5., -5.).points,
            vec![5., -5., 15., 15.]
        );
    }

    #[test]
    fn sizes_keep_the_thickness_on_screen() {
        assert_eq!(size_at(1.0), 1.0);
        assert_eq!(size_at(0.8), 1.25);
        assert_eq!(size_at(3.0), 0.5);
        assert_eq!(size_at(0.1), 4.0);
        assert_eq!(size_at(0.7), 1.43);
        // Halves round up, as Math.round does.
        assert_eq!(r1(-0.25), -0.2);
        assert_eq!(r1(0.25), 0.3);
    }

    #[test]
    fn history_keeps_a_hundred_changes_and_redo_does_not_trim() {
        let mut h = InkHistory::default();
        for i in 0..105 {
            h.commit(vec![pen(&i.to_string(), &[0., 0.], 1.)]);
        }
        assert_eq!(h.len(), HISTORY);
        let back = h.undo(Vec::new()).unwrap();
        assert_eq!(back[0].id, "104");
        let forward = h.redo(back).unwrap();
        assert!(forward.is_empty());
        assert_eq!(h.len(), HISTORY);
        // A new change clears the redo list.
        h.undo(Vec::new());
        h.commit(Vec::new());
        assert!(h.redo(Vec::new()).is_none());
    }
}
