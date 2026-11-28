//! Where a link runs (spec 14, LINK-001): straight runs at right angles
//! with rounded bends, from the side of what holds one end to the facing
//! side of what holds the other. The window draws links along it, and so
//! does a session's picture of the board.

use nodus_domain::Rect;

use crate::geometry::Point;

/// A link's end as drawn: the box holding it, or its point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum End {
    Node(Rect),
    Point(Point),
}

/// How far apart two boxes must be, in board units, for a link to leave one
/// box's side and come into the other's facing side.
pub const APART: f64 = 24.0;
/// How round a bend is, in board units (less where a run is short).
pub const BEND: f64 = 10.0;
/// How much of their sides two boxes must share for a link to run straight
/// across between them.
const SHARED: f64 = 16.0;
/// How many pieces a bend is drawn in.
const BEND_STEPS: usize = 8;

fn centre(r: Rect) -> Point {
    Point {
        x: r.x + r.w / 2.0,
        y: r.y + r.h / 2.0,
    }
}

fn pt(x: f64, y: f64) -> Point {
    Point { x, y }
}

/// The point on a box's edge on the way from its middle to `to`.
fn edge_toward(r: Rect, to: Point) -> Point {
    let c = centre(r);
    let (dx, dy) = (to.x - c.x, to.y - c.y);
    if dx == 0.0 && dy == 0.0 {
        return c;
    }
    let tx = if dx == 0.0 {
        f64::INFINITY
    } else {
        (r.w / 2.0) / dx.abs()
    };
    let ty = if dy == 0.0 {
        f64::INFINITY
    } else {
        (r.h / 2.0) / dy.abs()
    };
    let t = tx.min(ty).min(1.0);
    pt(c.x + dx * t, c.y + dy * t)
}

/// The corners a link runs through, from its start to its end: a line
/// straight across when what holds its ends lines up, else one bend (an
/// L) or two (a step).
pub fn corners(from: End, to: End) -> Vec<Point> {
    match (from, to) {
        (End::Node(a), End::Node(b)) => between(a, b),
        (End::Node(a), End::Point(p)) => out_to(a, p),
        (End::Point(p), End::Node(b)) => {
            let mut line = out_to(b, p);
            line.reverse();
            line
        }
        (End::Point(p), End::Point(q)) => vec![p, q],
    }
}

/// Between two boxes.
fn between(a: Rect, b: Rect) -> Vec<Point> {
    let (ca, cb) = (centre(a), centre(b));
    // How far apart they are each way; below 0 they overlap that way.
    let apart_x = (b.x - (a.x + a.w)).max(a.x - (b.x + b.w));
    let apart_y = (b.y - (a.y + a.h)).max(a.y - (b.y + b.h));
    // The sides that face each other.
    let (from_x, to_x) = if cb.x >= ca.x {
        (a.x + a.w, b.x)
    } else {
        (a.x, b.x + b.w)
    };
    let (from_y, to_y) = if cb.y >= ca.y {
        (a.y + a.h, b.y)
    } else {
        (a.y, b.y + b.h)
    };
    let across = || {
        // Straight across where their sides share enough, else a step
        // halfway between them.
        let (lo, hi) = (a.y.max(b.y), (a.y + a.h).min(b.y + b.h));
        if hi - lo >= SHARED {
            let y = ((ca.y + cb.y) / 2.0).clamp(lo + SHARED / 2.0, hi - SHARED / 2.0);
            return vec![pt(from_x, y), pt(to_x, y)];
        }
        let mid = (from_x + to_x) / 2.0;
        vec![pt(from_x, ca.y), pt(mid, ca.y), pt(mid, cb.y), pt(to_x, cb.y)]
    };
    let down = || {
        let (lo, hi) = (a.x.max(b.x), (a.x + a.w).min(b.x + b.w));
        if hi - lo >= SHARED {
            let x = ((ca.x + cb.x) / 2.0).clamp(lo + SHARED / 2.0, hi - SHARED / 2.0);
            return vec![pt(x, from_y), pt(x, to_y)];
        }
        let mid = (from_y + to_y) / 2.0;
        vec![pt(ca.x, from_y), pt(ca.x, mid), pt(cb.x, mid), pt(cb.x, to_y)]
    };
    match (apart_x >= APART, apart_y >= APART) {
        // Apart both ways: one bend, leaving along the longer way.
        (true, true) => {
            if (cb.x - ca.x).abs() >= (cb.y - ca.y).abs() {
                vec![pt(from_x, ca.y), pt(cb.x, ca.y), pt(cb.x, to_y)]
            } else {
                vec![pt(ca.x, from_y), pt(ca.x, cb.y), pt(to_x, cb.y)]
            }
        }
        (true, false) => across(),
        (false, true) => down(),
        // Close: the way they're further apart, if they don't overlap.
        _ if apart_x.max(apart_y) > 0.0 => {
            if apart_x >= apart_y {
                across()
            } else {
                down()
            }
        }
        // One over the other: straight between their edges.
        _ => vec![edge_toward(a, cb), edge_toward(b, ca)],
    }
}

/// From a box out to a loose point: from the side facing it, straight when
/// the point is level with that side, else with one bend.
fn out_to(a: Rect, p: Point) -> Vec<Point> {
    let c = centre(a);
    let inside_x = p.x >= a.x && p.x <= a.x + a.w;
    let inside_y = p.y >= a.y && p.y <= a.y + a.h;
    if inside_x && inside_y {
        return vec![edge_toward(a, p), p];
    }
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    if dx.abs() / a.w.max(1.0) > dy.abs() / a.h.max(1.0) {
        let side = if dx > 0.0 { a.x + a.w } else { a.x };
        if inside_y {
            vec![pt(side, p.y), p]
        } else {
            vec![pt(side, c.y), pt(p.x, c.y), p]
        }
    } else {
        let side = if dy > 0.0 { a.y + a.h } else { a.y };
        if inside_x {
            vec![pt(p.x, side), p]
        } else {
            vec![pt(c.x, side), pt(c.x, p.y), p]
        }
    }
}

fn len(a: Point, b: Point) -> f64 {
    (b.x - a.x).hypot(b.y - a.y)
}

/// The corners as a line to draw: each bend rounded, `radius` round or less
/// where a run beside it is short, in small pieces.
pub fn rounded(corners: &[Point], radius: f64) -> Vec<Point> {
    let mut pts: Vec<Point> = Vec::with_capacity(corners.len());
    for &p in corners {
        if pts.last().is_none_or(|&q| len(q, p) > 1e-6) {
            pts.push(p);
        }
    }
    if pts.len() < 3 {
        return pts;
    }
    let mut out = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let (p, c, q) = (pts[i - 1], pts[i], pts[i + 1]);
        let (l1, l2) = (len(p, c), len(c, q));
        let r = radius.min(l1 / 2.0).min(l2 / 2.0);
        let (u, v) = (
            pt((c.x - p.x) / l1, (c.y - p.y) / l1),
            pt((q.x - c.x) / l2, (q.y - c.y) / l2),
        );
        // Straight on through: no bend to round.
        if r <= 1e-6 || (u.x * v.y - u.y * v.x).abs() < 1e-9 {
            out.push(c);
            continue;
        }
        let a = pt(c.x - u.x * r, c.y - u.y * r);
        let b = pt(c.x + v.x * r, c.y + v.y * r);
        out.push(a);
        for s in 1..BEND_STEPS {
            let t = s as f64 / BEND_STEPS as f64;
            let w = 1.0 - t;
            out.push(pt(
                w * w * a.x + 2.0 * w * t * c.x + t * t * b.x,
                w * w * a.y + 2.0 * w * t * c.y + t * t * b.y,
            ));
        }
        out.push(b);
    }
    out.push(pts[pts.len() - 1]);
    out
}

/// Where a link runs, as a line to draw (LINK-001).
pub fn route(from: End, to: End) -> Vec<Point> {
    rounded(&corners(from, to), BEND)
}

/// The first `t` (0…1) of a line's length, and the point it ends at.
pub fn line_part(line: &[Point], t: f64) -> (Vec<Point>, Point) {
    let Some(&first) = line.first() else {
        return (Vec::new(), pt(0.0, 0.0));
    };
    let total: f64 = line.windows(2).map(|w| len(w[0], w[1])).sum();
    let mut left = total * t.clamp(0.0, 1.0);
    let mut out = vec![first];
    for w in line.windows(2) {
        let l = len(w[0], w[1]);
        if left <= l {
            let k = if l > 0.0 { left / l } else { 0.0 };
            let end = pt(w[0].x + (w[1].x - w[0].x) * k, w[0].y + (w[1].y - w[0].y) * k);
            out.push(end);
            return (out, end);
        }
        left -= l;
        out.push(w[1]);
    }
    let end = *out.last().unwrap_or(&first);
    (out, end)
}

/// The point `t` (0…1) of the way along a line.
pub fn at(line: &[Point], t: f64) -> Point {
    line_part(line, t).1
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l = dx * dx + dy * dy;
    let t = if l > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}

/// How far a point is from a line.
pub fn distance(line: &[Point], p: Point) -> f64 {
    match line {
        [] => f64::INFINITY,
        [only] => len(*only, p),
        _ => line
            .windows(2)
            .map(|w| segment_distance(p, w[0], w[1]))
            .fold(f64::INFINITY, f64::min),
    }
}

/// The arrowhead at a line's end: its tip and the two corners behind it,
/// `len` long and as wide.
pub fn head(line: &[Point], length: f64) -> [Point; 3] {
    let tip = line.last().copied().unwrap_or(pt(0.0, 0.0));
    // The way it comes in: from the last point that isn't the tip.
    let from = line
        .iter()
        .rev()
        .find(|q| len(**q, tip) > 1e-6)
        .copied()
        .unwrap_or(pt(tip.x - 1.0, tip.y));
    let angle = (tip.y - from.y).atan2(tip.x - from.x);
    let back = pt(tip.x - length * angle.cos(), tip.y - length * angle.sin());
    let (nx, ny) = (-angle.sin() * length / 2.0, angle.cos() * length / 2.0);
    [tip, pt(back.x + nx, back.y + ny), pt(back.x - nx, back.y - ny)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn runs_straight_across_between_boxes_in_a_row() {
        let a = rect(0.0, 0.0, 180.0, 96.0);
        let b = rect(300.0, 10.0, 180.0, 96.0);
        assert_eq!(
            corners(End::Node(a), End::Node(b)),
            vec![pt(180.0, 53.0), pt(300.0, 53.0)]
        );
        // And back the other way.
        assert_eq!(
            corners(End::Node(b), End::Node(a)),
            vec![pt(300.0, 53.0), pt(180.0, 53.0)]
        );
    }

    #[test]
    fn steps_halfway_when_rows_barely_share_their_sides() {
        let a = rect(0.0, 0.0, 100.0, 40.0);
        let b = rect(200.0, 30.0, 100.0, 40.0);
        assert_eq!(
            corners(End::Node(a), End::Node(b)),
            vec![pt(100.0, 20.0), pt(150.0, 20.0), pt(150.0, 50.0), pt(200.0, 50.0)]
        );
    }

    #[test]
    fn runs_straight_down_between_boxes_in_a_column() {
        let a = rect(0.0, 0.0, 160.0, 112.0);
        let b = rect(0.0, 200.0, 180.0, 96.0);
        assert_eq!(
            corners(End::Node(a), End::Node(b)),
            vec![pt(85.0, 112.0), pt(85.0, 200.0)]
        );
    }

    #[test]
    fn bends_once_between_boxes_apart_both_ways() {
        // Further across than down: out of the side, into the top.
        let a = rect(0.0, 0.0, 100.0, 40.0);
        let b = rect(300.0, 120.0, 100.0, 40.0);
        assert_eq!(
            corners(End::Node(a), End::Node(b)),
            vec![pt(100.0, 20.0), pt(350.0, 20.0), pt(350.0, 120.0)]
        );
        // Further down than across: out of the bottom, into the side.
        let b = rect(150.0, 400.0, 100.0, 40.0);
        assert_eq!(
            corners(End::Node(a), End::Node(b)),
            vec![pt(50.0, 40.0), pt(50.0, 420.0), pt(150.0, 420.0)]
        );
    }

    #[test]
    fn runs_between_edges_when_boxes_overlap() {
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = rect(50.0, 50.0, 100.0, 100.0);
        assert_eq!(
            corners(End::Node(a), End::Node(b)),
            vec![pt(100.0, 100.0), pt(50.0, 50.0)]
        );
    }

    #[test]
    fn goes_out_to_a_loose_point_and_comes_back_in() {
        let a = rect(0.0, 0.0, 200.0, 100.0);
        // Level with the side: straight.
        let p = pt(400.0, 30.0);
        assert_eq!(corners(End::Node(a), End::Point(p)), vec![pt(200.0, 30.0), p]);
        // Off to the side and below: out of the side, then one bend.
        let p = pt(600.0, 200.0);
        assert_eq!(
            corners(End::Node(a), End::Point(p)),
            vec![pt(200.0, 50.0), pt(600.0, 50.0), p]
        );
        // Coming in, the last run goes into the box's side.
        assert_eq!(
            corners(End::Point(p), End::Node(a)),
            vec![p, pt(600.0, 50.0), pt(200.0, 50.0)]
        );
        // More below than beside: out of the bottom.
        let p = pt(500.0, 400.0);
        assert_eq!(
            corners(End::Node(a), End::Point(p)),
            vec![pt(100.0, 100.0), pt(100.0, 400.0), p]
        );
        // Two loose points: straight.
        let q = pt(0.0, 0.0);
        assert_eq!(corners(End::Point(q), End::Point(p)), vec![q, p]);
    }

    #[test]
    fn rounds_bends_and_keeps_the_ends() {
        let line = rounded(&[pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 100.0)], 10.0);
        assert_eq!(line.first(), Some(&pt(0.0, 0.0)));
        assert_eq!(line.last(), Some(&pt(100.0, 100.0)));
        // The bend starts 10 before the corner and ends 10 after it.
        assert!(line.contains(&pt(90.0, 0.0)) && line.contains(&pt(100.0, 10.0)));
        assert!(!line.contains(&pt(100.0, 0.0)));
        // A short run makes the bend tighter.
        let line = rounded(&[pt(0.0, 0.0), pt(6.0, 0.0), pt(6.0, 100.0)], 10.0);
        assert!(line.contains(&pt(3.0, 0.0)));
        // A straight line stays as it was.
        assert_eq!(rounded(&[pt(0.0, 0.0), pt(10.0, 0.0)], 10.0).len(), 2);
    }

    #[test]
    fn measures_along_and_points_the_head_the_way_in() {
        let line = vec![pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 100.0)];
        assert_eq!(at(&line, 0.5), pt(100.0, 0.0));
        assert_eq!(at(&line, 0.25), pt(50.0, 0.0));
        assert!((distance(&line, pt(50.0, 6.0)) - 6.0).abs() < 1e-9);
        let [tip, l, r] = head(&line, 12.0);
        assert_eq!(tip, pt(100.0, 100.0));
        assert!((l.y - 88.0).abs() < 1e-9 && (r.y - 88.0).abs() < 1e-9);
        assert!(((l.x - r.x).abs() - 12.0).abs() < 1e-9);
    }
}
