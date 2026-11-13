//! Board geometry the core needs as well as the window: the sizes of cards,
//! notes, views and the archive dock, and the search for a free spot. The
//! window draws the board; the core places the notes and views that sessions
//! put up with their board tools. A port of `src/shared/geometry.ts`.

use nodus_domain::js;
use nodus_domain::{BoardNode, BoardNote, BoardView, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// `Partial<Size>`: a size whose dimensions may be missing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PartialSize {
    pub w: Option<f64>,
    pub h: Option<f64>,
}

// SESS-196
/// Card size in flow units (4:3).
pub const CARD_W: f64 = 288.0;
pub const CARD_H: f64 = 216.0;
pub const GAP: f64 = 32.0;
pub const CARD_SIZE: Size = Size { w: CARD_W, h: CARD_H };

/// An expanded card starts with room for an 80-column terminal.
pub const EXPANDED_W: f64 = 720.0;
pub const EXPANDED_H: f64 = 480.0;
/// A new card starts twice as tall, with room for its chat or terminal.
pub const NEW_CARD_H: f64 = EXPANDED_H * 2.0;

/// Height of the archive header strip.
pub const ARCHIVE_HEADER_H: f64 = 52.0;
pub const ARCHIVE_PAD: f64 = 28.0;
/// The search field sits under the stack, below the peeking cards.
pub const STACK_SEARCH_Y: f64 = ARCHIVE_HEADER_H + CARD_H + 38.0;
pub const STACK_SEARCH_H: f64 = 34.0;
pub const STACK_W: f64 = ARCHIVE_PAD * 2.0 + CARD_W;
pub const STACK_H: f64 = STACK_SEARCH_Y + STACK_SEARCH_H + 22.0;

/// A note starts square, as tall as a card so the two line up in rows.
pub const NOTE_W: f64 = CARD_H;
pub const NOTE_H: f64 = CARD_H;

/// The pad sits under the archive stack, as wide as it, with a header like the archive's.
pub const PAD_HEADER_H: f64 = ARCHIVE_HEADER_H;
pub const PAD_W: f64 = STACK_W;
const PAD_GAP: f64 = 24.0;
/// Room under the top sheet for the sheets beneath it to show.
const PAD_BOTTOM: f64 = ARCHIVE_PAD;

/// A view starts with room for a small table, and can be resized from 240×160 to 2400×1800.
pub const VIEW_W: f64 = 480.0;
pub const VIEW_H: f64 = 360.0;
pub const VIEW_MIN_W: f64 = 240.0;
pub const VIEW_MIN_H: f64 = 160.0;
pub const VIEW_MAX_W: f64 = 2400.0;
pub const VIEW_MAX_H: f64 = 1800.0;

/// Whether a card is drawn expanded, with its terminal on the board. Archived
/// cards never are (SESS-197).
pub fn is_expanded(card: Option<&BoardNode>) -> bool {
    card.is_some_and(|c| c.expanded == Some(true) && !c.archived)
}

/// A card's size on the board: compact, or as big as it was last expanded to (SESS-197).
pub fn card_size(card: Option<&BoardNode>) -> Size {
    match card {
        Some(c) if is_expanded(card) => Size {
            w: c.w.unwrap_or(EXPANDED_W),
            h: c.h.unwrap_or(EXPANDED_H),
        },
        _ => CARD_SIZE,
    }
}

/// SESS-198
pub fn note_size(note: &BoardNote) -> Size {
    Size {
        w: note.w.unwrap_or(NOTE_W),
        h: note.h.unwrap_or(NOTE_H),
    }
}

/// SESS-198
pub fn view_size(view: &BoardView) -> Size {
    Size {
        w: view.w.unwrap_or(VIEW_W),
        h: view.h.unwrap_or(VIEW_H),
    }
}

/// Each finite dimension rounded into the view's bounds; others left out (SESS-198).
pub fn clamp_view_size(size: PartialSize) -> PartialSize {
    let clamp =
        |n: Option<f64>, min: f64, max: f64| n.filter(|n| n.is_finite()).map(|n| js::round(n.max(min).min(max)));
    PartialSize {
        w: clamp(size.w, VIEW_MIN_W, VIEW_MAX_W),
        h: clamp(size.h, VIEW_MIN_H, VIEW_MAX_H),
    }
}

/// The pad under the archive's stack. It goes by the folded stack, so the
/// open panel floats over it (SESS-199).
pub fn note_pad_rect(archive: Rect) -> Rect {
    Rect {
        x: archive.x,
        y: archive.y + STACK_H + PAD_GAP,
        w: PAD_W,
        h: PAD_HEADER_H + NOTE_H + PAD_BOTTOM,
    }
}

/// The folded archive stack and the pad under it, which new cards and notes
/// keep clear of (SESS-199).
pub fn dock_rect(archive: Rect) -> Rect {
    let pad = note_pad_rect(archive);
    Rect {
        x: archive.x,
        y: archive.y,
        w: STACK_W,
        h: pad.y + pad.h - archive.y,
    }
}

/// Whether two rects overlap, or come within `margin` of each other; touching
/// edges don't count at margin 0 (SESS-200).
pub fn intersects(a: Rect, b: Rect, margin: f64) -> bool {
    a.x < b.x + b.w + margin && a.x + a.w + margin > b.x && a.y < b.y + b.h + margin && a.y + a.h + margin > b.y
}

fn rect_at(p: Point, size: Size) -> Rect {
    Rect {
        x: p.x,
        y: p.y,
        w: size.w,
        h: size.h,
    }
}

fn blocked(r: Rect, obstacles: &[Rect], avoid: Option<Rect>) -> bool {
    avoid.is_some_and(|avoid| intersects(r, avoid, GAP)) || obstacles.iter().any(|o| intersects(r, *o, GAP / 2.0))
}

/// Finds a free spot for a card (or anything of `size`) near `preferred` that
/// overlaps neither the given obstacles nor (optionally) the archive zone. It
/// searches rings of card-sized grid cells around the preferred position,
/// closest first (SESS-201).
pub fn find_free_spot(preferred: Point, obstacles: &[Rect], avoid: Option<Rect>, size: Size) -> Point {
    let step_x = CARD_W + GAP;
    let step_y = CARD_H + GAP;
    if !blocked(rect_at(preferred, size), obstacles, avoid) {
        return preferred;
    }
    // Prefer spots to the right of / below the preferred point, then nearest.
    let score = |p: &Point| {
        let dx = p.x - preferred.x;
        let dy = p.y - preferred.y;
        let behind = (if dx < 0.0 { 1.4 } else { 1.0 }) * (if dy < 0.0 { 1.2 } else { 1.0 });
        hypot(dx, dy) * behind
    };
    for ring in 1..40_i32 {
        let mut candidates = Vec::new();
        for dx in -ring..=ring {
            for dy in -ring..=ring {
                if dx.abs().max(dy.abs()) != ring {
                    continue;
                }
                candidates.push(Point {
                    x: preferred.x + f64::from(dx) * step_x,
                    y: preferred.y + f64::from(dy) * step_y,
                });
            }
        }
        candidates.sort_by(|a, b| score(a).partial_cmp(&score(b)).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(hit) = candidates
            .into_iter()
            .find(|c| !blocked(rect_at(*c, size), obstacles, avoid))
        {
            return hit;
        }
    }
    Point {
        x: preferred.x,
        y: preferred.y + 40.0 * step_y,
    }
}

/// Finds a free spot for something of `size` right beside `anchor`, `gap`
/// away: to its right if there's room, else below, left of or above it, and
/// lined up with it where it can be; then further out. Unlike
/// [`find_free_spot`], which steps a whole card at a time, it slides along the
/// anchor's sides, so what's placed lands next to the anchor even when the
/// board around it is busy. Further than twice its size away, it falls back
/// to [`find_free_spot`] (SESS-202).
pub fn find_spot_beside(anchor: Rect, size: Size, obstacles: &[Rect], avoid: Option<Rect>, gap: f64) -> Point {
    let reach = 2.0 * size.w.max(size.h);
    let right = anchor.x + anchor.w + gap;
    let left = anchor.x - gap - size.w;
    let below = anchor.y + anchor.h + gap;
    let above = anchor.y - gap - size.h;
    // Positions on one axis: out from either side of the anchor, and along it in between.
    let axis = |start: f64, before: f64, after: f64| {
        let mut out = Vec::new();
        let mut d = 0.0;
        while d <= reach {
            out.push(after + d);
            out.push(before - d);
            d += GAP;
        }
        let mut v = start - ((start - before) / GAP).floor() * GAP;
        while v < after {
            out.push(v);
            v += GAP;
        }
        out
    };
    let xs = axis(anchor.x, left, right);
    let ys = axis(anchor.y, above, below);

    let mut best: Option<Point> = None;
    let mut best_score = f64::INFINITY;
    for &x in &xs {
        for &y in &ys {
            let gx = (anchor.x - (x + size.w)).max(x - (anchor.x + anchor.w));
            let gy = (anchor.y - (y + size.h)).max(y - (anchor.y + anchor.h));
            if gx.max(gy) < gap - 1.0 {
                continue;
            }
            // Beside it rather than above or below it, and how far it is from lining up.
            let sideways = gx >= gy;
            let weight = if sideways {
                if x > anchor.x { 1.0 } else { 1.3 }
            } else if y > anchor.y {
                1.15
            } else {
                1.45
            };
            let offset = if sideways {
                (y - anchor.y).abs()
            } else {
                (x - anchor.x).abs()
            };
            let score = hypot(gx.max(0.0), gy.max(0.0)) * weight + offset / 2.0;
            if score >= best_score {
                continue;
            }
            if blocked(rect_at(Point { x, y }, size), obstacles, avoid) {
                continue;
            }
            best = Some(Point { x, y });
            best_score = score;
        }
    }
    best.unwrap_or_else(|| find_free_spot(Point { x: right, y: anchor.y }, obstacles, avoid, size))
}

/// `Math.hypot(a, b)` as V8 computes it: scaled by the larger, with Kahan
/// summation, so near-ties sort as they do in the window.
pub fn hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let max = a.max(b);
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for v in [a, b] {
        let n = v / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64) -> Rect {
        sized(x, y, CARD_W, CARD_H)
    }

    fn sized(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn node(expanded: Option<bool>, archived: bool, w: Option<f64>, h: Option<f64>) -> BoardNode {
        serde_json::from_value(serde_json::json!({
            "sessionId": "s", "x": 0, "y": 0, "archived": archived, "createdAt": 0,
            "expanded": expanded, "w": w, "h": h
        }))
        .unwrap()
    }

    // intersects
    #[test]
    fn tells_overlapping_rects_from_ones_that_only_touch() {
        assert!(intersects(rect(0.0, 0.0), rect(100.0, 100.0), 0.0));
        assert!(!intersects(rect(0.0, 0.0), rect(CARD_W, 0.0), 0.0));
        assert!(!intersects(rect(0.0, 0.0), rect(0.0, CARD_H + 1.0), 0.0));
    }

    #[test]
    fn counts_rects_within_the_margin_as_overlapping() {
        assert!(intersects(rect(0.0, 0.0), rect(CARD_W + 10.0, 0.0), 11.0));
        assert!(!intersects(rect(0.0, 0.0), rect(CARD_W + 10.0, 0.0), 10.0));
    }

    // cardSize
    #[test]
    fn is_compact_unless_expanded_and_never_expanded_in_the_archive() {
        assert_eq!(card_size(None), CARD_SIZE);
        assert_eq!(
            card_size(Some(&node(Some(true), false, None, None))),
            Size {
                w: EXPANDED_W,
                h: EXPANDED_H
            }
        );
        assert_eq!(
            card_size(Some(&node(Some(true), false, Some(900.0), Some(600.0)))),
            Size { w: 900.0, h: 600.0 }
        );
        assert_eq!(
            card_size(Some(&node(Some(true), true, Some(900.0), Some(600.0)))),
            CARD_SIZE
        );
    }

    // findFreeSpot
    fn clear(p: Point, size: Size, obstacles: &[Rect]) -> bool {
        obstacles.iter().all(|o| !intersects(rect_at(p, size), *o, GAP / 2.0))
    }

    #[test]
    fn keeps_the_preferred_spot_when_it_is_free() {
        assert_eq!(
            find_free_spot(at(0.0, 0.0), &[rect(1000.0, 1000.0)], None, CARD_SIZE),
            at(0.0, 0.0)
        );
    }

    #[test]
    fn moves_to_the_nearest_free_grid_cell_below_before_above() {
        assert_eq!(
            find_free_spot(at(0.0, 0.0), &[rect(0.0, 0.0)], None, CARD_SIZE),
            at(0.0, CARD_H + GAP)
        );
    }

    #[test]
    fn finds_room_among_many_cards() {
        let mut obstacles = Vec::new();
        for i in -2..=2 {
            for j in -2..=2 {
                obstacles.push(rect(f64::from(i) * (CARD_W + GAP), f64::from(j) * (CARD_H + GAP)));
            }
        }
        let spot = find_free_spot(at(0.0, 0.0), &obstacles, None, CARD_SIZE);
        assert!(clear(spot, CARD_SIZE, &obstacles));
        // The nearest ring that has room.
        let ring = (spot.x / (CARD_W + GAP)).abs().max((spot.y / (CARD_H + GAP)).abs());
        assert_eq!(ring, 3.0);
    }

    #[test]
    fn keeps_out_of_the_area_to_avoid() {
        let avoid = sized(0.0, 0.0, 2000.0, 2000.0);
        let spot = find_free_spot(at(100.0, 100.0), &[], Some(avoid), CARD_SIZE);
        assert!(!intersects(rect_at(spot, CARD_SIZE), avoid, GAP));
    }

    #[test]
    fn fits_something_of_another_size() {
        let size = Size { w: 600.0, h: 400.0 };
        let spot = find_free_spot(at(0.0, 0.0), &[rect(300.0, 0.0)], None, size);
        assert!(!intersects(rect_at(spot, size), rect(300.0, 0.0), GAP / 2.0));
    }

    // findSpotBeside
    const VIEW: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 480.0,
        h: 360.0,
    };
    const EXPANDED: Size = Size {
        w: EXPANDED_W,
        h: EXPANDED_H,
    };

    #[test]
    fn goes_right_of_the_anchor_lined_up_with_its_top() {
        assert_eq!(
            find_spot_beside(VIEW, EXPANDED, &[VIEW], None, GAP),
            at(480.0 + GAP, 0.0)
        );
    }

    #[test]
    fn goes_below_then_left_when_the_right_is_taken() {
        let right = rect(480.0 + GAP, 0.0);
        assert_eq!(
            find_spot_beside(VIEW, EXPANDED, &[VIEW, right], None, GAP),
            at(0.0, 360.0 + GAP)
        );
        let below = rect(0.0, 360.0 + GAP);
        assert_eq!(
            find_spot_beside(VIEW, EXPANDED, &[VIEW, right, below], None, GAP),
            at(-GAP - EXPANDED_W, 0.0)
        );
    }

    #[test]
    fn stays_next_to_the_anchor_where_find_free_spot_steps_past_what_is_in_the_way() {
        // A column of cards down the view's right side, past its bottom.
        let mut obstacles = vec![VIEW];
        obstacles.extend((-1..=2).map(|i| rect(480.0 + GAP, f64::from(i) * (CARD_H + GAP))));
        let gap = |p: Point| {
            (VIEW.x - (p.x + EXPANDED.w))
                .max(p.x - (VIEW.x + VIEW.w))
                .max(VIEW.y - (p.y + EXPANDED.h))
                .max(p.y - (VIEW.y + VIEW.h))
        };
        let spot = find_spot_beside(VIEW, EXPANDED, &obstacles, None, GAP);
        assert!(clear(spot, EXPANDED, &obstacles));
        assert_eq!(gap(spot), GAP);
        assert!(gap(find_free_spot(at(480.0 + GAP, 0.0), &obstacles, None, EXPANDED)) > CARD_W);
    }

    #[test]
    fn keeps_the_gap_it_is_given() {
        assert_eq!(
            find_spot_beside(VIEW, EXPANDED, &[VIEW], None, 3.0 * GAP),
            at(480.0 + 3.0 * GAP, 0.0)
        );
    }

    #[test]
    fn keeps_out_of_the_area_to_avoid_beside() {
        let avoid = sized(480.0, -2000.0, 4000.0, 4000.0);
        let spot = find_spot_beside(VIEW, EXPANDED, &[VIEW], Some(avoid), GAP);
        assert!(!intersects(rect_at(spot, EXPANDED), avoid, GAP));
        assert!(spot.x < 0.0);
    }

    #[test]
    fn falls_back_to_find_free_spot_when_there_is_no_room_nearby() {
        let wall = sized(-5000.0, -5000.0, 10000.0, 10000.0);
        let spot = find_spot_beside(VIEW, EXPANDED, &[wall], None, GAP);
        assert!(clear(spot, EXPANDED, &[wall]));
    }

    // Constants and the rest (SESS-196…199), without a TS counterpart.
    #[test]
    fn has_the_sizes_the_window_draws() {
        assert_eq!((STACK_SEARCH_Y, STACK_W, STACK_H), (306.0, 344.0, 362.0));
        let archive = sized(10.0, 20.0, 1928.0, 320.0);
        assert_eq!(note_pad_rect(archive), sized(10.0, 20.0 + 362.0 + 24.0, 344.0, 296.0));
        assert_eq!(dock_rect(archive), sized(10.0, 20.0, 344.0, 682.0));
        assert_eq!(
            clamp_view_size(PartialSize {
                w: Some(100.4),
                h: Some(f64::NAN)
            }),
            PartialSize {
                w: Some(240.0),
                h: None
            }
        );
        assert_eq!(
            clamp_view_size(PartialSize {
                w: Some(500.5),
                h: Some(9999.0)
            }),
            PartialSize {
                w: Some(501.0),
                h: Some(1800.0)
            }
        );
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, -0.0), 0.0);
        assert!((hypot(320.0, 248.0) - 320.0_f64.hypot(248.0)).abs() < 1e-9);
    }
}
