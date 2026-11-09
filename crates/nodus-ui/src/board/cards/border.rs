//! Rounded borders drawn as Chromium draws them, for the archive frame, its
//! placeholders and the note pad (ARCH-002…ARCH-004, ARCH-010, ARCH-033,
//! NOTEUI-040): `1.5px dashed` at the board's zoom.
//!
//! GPUI snaps a border's width to whole device pixels (1.5px at 80% is 2.4
//! device pixels; GPUI draws 2) and its dashes follow that width, so the
//! frames' dashes came out thinner and closer together than 0.9.4's. Here
//! the border is a path at its exact width, and dashed as Blink dashes a
//! rounded border: the centre line of the border (the box inset by half the
//! width, its radius less half the width) stroked clockwise from the top
//! edge's left end, dashes twice the width long, the gaps about the width,
//! evened out so a whole number of dashes and gaps goes round
//! (`StrokeData::SelectBestDashGap`).

use std::f32::consts::{FRAC_PI_2, PI};

use gpui::{App, Bounds, IntoElement, PathBuilder, Pixels, Point, Rgba, Styled, Window, canvas, point, px};

use super::Z;

/// A border's look: `width` CSS px, solid or dashed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: Rgba,
    pub dashed: bool,
}

/// The border of a box `size` (CSS px) with corners `radius` round, drawn
/// over a box of that size at the board's scale; position it over the box
/// (absolute, at the box's top-left).
pub fn border(size: (f32, f32), radius: f32, stroke: Stroke, z: Z) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
            paint(bounds.origin, (0.0, 0.0), size, radius, stroke, z, window);
        },
    )
    .absolute()
    .left_0()
    .top_0()
    .w(z.px(size.0))
    .h(z.px(size.1))
}

/// The centre line of the border: where the four straight sides and the
/// four corners lie along it, clockwise from the top edge's left end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Centreline {
    /// Its box (CSS px, from the border box's corner): x, y, w, h.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
}

impl Centreline {
    pub fn of(size: (f32, f32), radius: f32, width: f32) -> Self {
        let half = width / 2.0;
        let (w, h) = (size.0 - width, size.1 - width);
        Self {
            x: half,
            y: half,
            w,
            h,
            r: (radius - half).clamp(0.0, w.min(h) / 2.0),
        }
    }

    fn sides(&self) -> (f32, f32, f32) {
        let a = (self.w - 2.0 * self.r).max(0.0);
        let b = (self.h - 2.0 * self.r).max(0.0);
        (a, b, FRAC_PI_2 * self.r)
    }

    pub fn length(&self) -> f32 {
        let (a, b, c) = self.sides();
        2.0 * (a + b + 2.0 * c)
    }

    /// The point `s` along the line, and the direction there (radians).
    pub fn at(&self, s: f32) -> (f32, f32) {
        let (pos, _) = self.locate(s);
        pos
    }

    /// Where `s` falls: a point, and the segment (0 top, 1 top-right
    /// corner, 2 right, … 7 top-left corner) with how far into it.
    fn locate(&self, s: f32) -> ((f32, f32), (usize, f32)) {
        let (a, b, c) = self.sides();
        let lens = [a, c, b, c, a, c, b, c];
        let mut s = s.rem_euclid(self.length().max(f32::EPSILON));
        let mut seg = 0;
        while seg < 7 && s > lens[seg] {
            s -= lens[seg];
            seg += 1;
        }
        (self.point(seg, s), (seg, s))
    }

    /// The point `t` into segment `seg`.
    fn point(&self, seg: usize, t: f32) -> (f32, f32) {
        let Self { x, y, w, h, r } = *self;
        let arc = |cx: f32, cy: f32, from: f32| {
            let angle = from + if r > 0.0 { t / r } else { 0.0 };
            (cx + r * angle.cos(), cy + r * angle.sin())
        };
        match seg {
            0 => (x + r + t, y),
            1 => arc(x + w - r, y + r, -FRAC_PI_2),
            2 => (x + w, y + r + t),
            3 => arc(x + w - r, y + h - r, 0.0),
            4 => (x + w - r - t, y + h),
            5 => arc(x + r, y + h - r, FRAC_PI_2),
            6 => (x, y + h - r - t),
            _ => arc(x + r, y + r, PI),
        }
    }

    /// The outward normal at `t` into segment `seg`.
    fn normal(&self, seg: usize, t: f32) -> (f32, f32) {
        let r = self.r.max(f32::EPSILON);
        let arc = |from: f32| {
            let angle = from + t / r;
            (angle.cos(), angle.sin())
        };
        match seg {
            0 => (0.0, -1.0),
            1 => arc(-FRAC_PI_2),
            2 => (1.0, 0.0),
            3 => arc(0.0),
            4 => (0.0, 1.0),
            5 => arc(FRAC_PI_2),
            6 => (-1.0, 0.0),
            _ => arc(PI),
        }
    }

    /// The outline of the band `width` wide along the line from `s0` to
    /// `s1`: its outer edge forward, then its inner edge back.
    pub fn band(&self, s0: f32, s1: f32, width: f32) -> Vec<(f32, f32)> {
        let (a, b, c) = self.sides();
        let lens = [a, c, b, c, a, c, b, c];
        // Every segment boundary in between, and a point every ~2px on corners.
        let mut stops = vec![s0];
        let mut acc = 0.0;
        for (seg, len) in lens.iter().enumerate() {
            let (from, to) = (acc, acc + len);
            if seg % 2 == 1 && *len > 0.0 {
                let steps = (len / 2.0).ceil().max(1.0) as usize;
                for k in 1..steps {
                    let s = from + len * k as f32 / steps as f32;
                    if s > s0 && s < s1 {
                        stops.push(s);
                    }
                }
            }
            if to > s0 && to < s1 {
                stops.push(to);
            }
            acc = to;
        }
        stops.push(s1);
        stops.sort_by(f32::total_cmp);
        stops.dedup_by(|p, q| (*p - *q).abs() < 1e-4);
        let half = width / 2.0;
        let edge = |s: f32, out: f32| {
            // At a boundary, the point belongs to the segment before it for
            // the band's end and after it for its start; both agree there.
            let (_, (seg, t)) = self.locate(s);
            let (px, py) = self.point(seg, t);
            let (nx, ny) = self.normal(seg, t);
            (px + nx * out, py + ny * out)
        };
        let mut outline: Vec<(f32, f32)> = stops.iter().map(|&s| edge(s, half)).collect();
        outline.extend(stops.iter().rev().map(|&s| edge(s, -half)));
        outline
    }
}

/// Blink's `StrokeData::SelectBestDashGap` for a closed path `length` long:
/// the gap that fits a whole number of `dash`es and gaps, nearest `gap`.
pub fn best_dash_gap(length: f32, dash: f32, gap: f32) -> f32 {
    let fewer = (length / (dash + gap)).floor().max(1.0);
    let more = fewer + 1.0;
    let fewer_gap = (length - fewer * dash) / fewer;
    let more_gap = (length - more * dash) / more;
    if more_gap <= 0.0 || (fewer_gap - gap).abs() < (more_gap - gap).abs() {
        fewer_gap
    } else {
        more_gap
    }
}

/// Where the dashes go along a centre line `length` long, for a border
/// `width` wide: dashes `2 × width`, gaps about `width`.
pub fn dashes(length: f32, width: f32) -> Vec<(f32, f32)> {
    let dash = 2.0 * width;
    if length <= 2.0 * dash {
        return vec![(0.0, length)];
    }
    let gap = best_dash_gap(length, dash, width);
    let period = dash + gap;
    // A whole number of periods goes round (the gap was chosen so).
    let count = (length / period).round().max(1.0) as usize;
    (0..count)
        .map(|k| {
            let s = k as f32 * period;
            (s, (s + dash).min(length))
        })
        .collect()
}

/// The `box-shadow` rings (no blur, no offset: `0 0 0 <spread> <colour>`)
/// round a box `size` (CSS px) with corners `radius` round, bottom layer
/// first, drawn as CSS draws them: outside the box, `spread` wide at their
/// exact width, the corners `radius + spread` round. GPUI keeps the box's
/// radius for a spread shadow and snaps it outward to whole device pixels,
/// so a 2px selection ring at 80% came out 4 device pixels wide and
/// squarer at the corners. It fills its parent, the box.
pub fn rings(radius: f32, rings: Vec<(f32, Rgba)>, z: Z) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
            let size = (f32::from(bounds.size.width) / z.0, f32::from(bounds.size.height) / z.0);
            for &(spread, color) in &rings {
                let band = (size.0 + 2.0 * spread, size.1 + 2.0 * spread);
                let r = if radius > 0.0 { radius + spread } else { 0.0 };
                let stroke = Stroke {
                    width: spread,
                    color,
                    dashed: false,
                };
                paint(bounds.origin, (-spread, -spread), band, r, stroke, z, window);
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Paints the border of a box `size` (CSS px) whose corner is `offset` from
/// `origin` (in CSS px at the board's scale).
fn paint(
    origin: Point<Pixels>,
    offset: (f32, f32),
    size: (f32, f32),
    radius: f32,
    stroke: Stroke,
    z: Z,
    window: &mut Window,
) {
    if stroke.color.a <= 0.0 || stroke.width <= 0.0 {
        return;
    }
    let line = Centreline::of(size, radius, stroke.width);
    let length = line.length();
    let runs = if stroke.dashed {
        dashes(length, stroke.width)
    } else {
        vec![(0.0, length)]
    };
    let (ox, oy) = (
        f32::from(origin.x) + offset.0 * z.0,
        f32::from(origin.y) + offset.1 * z.0,
    );
    let to_screen = |(x, y): (f32, f32)| point(px(ox + x * z.0), px(oy + y * z.0));
    let mut path = PathBuilder::fill();
    for (s0, s1) in runs {
        let outline = line.band(s0, s1, stroke.width);
        let mut points = outline.into_iter().map(to_screen);
        if let Some(first) = points.next() {
            path.move_to(first);
            for p in points {
                path.line_to(p);
            }
            path.close();
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, stroke.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_the_archive_frame_as_blink_does() {
        // ARCH-002: 344×362, radius 24, 1.5px. The centre line is 1366.08px
        // round; 304 dashes of 3px fit with gaps of 1.4937px, which the
        // golden's 7.19 device-pixel period at 80% (×1.6) shows.
        let line = Centreline::of((344.0, 362.0), 24.0, 1.5);
        assert!((line.length() - 1366.08).abs() < 0.01, "{}", line.length());
        let gap = best_dash_gap(line.length(), 3.0, 1.5);
        assert!((gap - 1.4937).abs() < 1e-3, "{gap}");
        let runs = dashes(line.length(), 1.5);
        assert_eq!(runs.len(), 304);
        assert_eq!(runs[0], (0.0, 3.0));
        // The first dash starts where the top edge leaves the top-left corner.
        assert_eq!(line.at(0.0), (24.0, 0.75));
    }

    #[test]
    fn walks_the_centre_line_clockwise() {
        let line = Centreline::of((100.0, 60.0), 10.0, 2.0);
        // Top edge from x 10 to 90, then the top-right corner.
        assert_eq!(line.at(0.0), (10.0, 1.0));
        let (x, y) = line.at(80.0);
        assert!((x - 90.0).abs() < 1e-4 && (y - 1.0).abs() < 1e-4);
        let quarter = FRAC_PI_2 * 9.0;
        let (x, y) = line.at(80.0 + quarter);
        assert!((x - 99.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "{x},{y}");
        // A band along the top is a rectangle.
        let band = line.band(0.0, 4.0, 2.0);
        assert_eq!(band, vec![(10.0, 0.0), (14.0, 0.0), (14.0, 2.0), (10.0, 2.0)]);
    }
}
