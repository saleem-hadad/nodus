//! Placing things on the board: the archive stack's geometry, where new
//! cards go, frames (stored as sections) and framing, and the note pad (ports of
//! `components/board/layout.ts`, `sections.ts`, `notes.ts` and `views.ts`).
//! What the core shares (sizes, the free-spot search) is `nodus_board::geometry`.

use nodus_board::geometry::{
    self as geo, ARCHIVE_HEADER_H, ARCHIVE_PAD, CARD_H, CARD_W, GAP, NOTE_H, NOTE_W, PAD_HEADER_H, PAD_W, Point,
    STACK_H, STACK_W, Size,
};
use nodus_domain::{BoardNote, BoardSection, BoardView, Rect, SessionCard, SessionKind, SessionStatus, js};

pub use nodus_board::geometry::{find_free_spot, find_spot_beside, intersects, note_pad_rect};

/// The archive node's id.
pub const ARCHIVE_ID: &str = "__archive__";
/// The note pad's id.
pub const NOTEPAD_ID: &str = "__notepad__";

/// Rows of results the open panel shows at once, two cards each; the rest scroll.
pub const STACK_ROWS: f64 = 2.0;
/// Height of the band the open panel's results scroll through.
pub const STACK_BAND_H: f64 = STACK_ROWS * CARD_H + (STACK_ROWS - 1.0) * GAP;
/// Cards drawn in the folded stack: the top one plus two peeking out behind it.
pub const STACK_DEPTH: usize = 3;
/// Where the fanned-out results start, relative to the archive's corner.
pub const STACK_RESULTS_X: f64 = ARCHIVE_PAD + CARD_W + 40.0;

/// A frame's header strip, its drag handle, and the room round what it holds.
pub const SECTION_HEADER_H: f64 = nodus_board::frames::FRAME_HEADER_H;
pub const SECTION_PAD: f64 = nodus_board::frames::FRAME_PAD;
/// Framing tidies things at most this many to a row.
const GROUP_MAX_COLS: f64 = 4.0;

pub use nodus_board::frames::{frame_around, holds_centre};

/// Notes come off the pad this big, in classic yellow.
pub const NOTE_SIZE: Size = Size { w: NOTE_W, h: NOTE_H };
/// Where the top sheet sits on the pad, centred under the archive's top card.
pub const PAD_SHEET_X: f64 = (PAD_W - NOTE_W) / 2.0;
/// A view starts this big.
pub const VIEW_SIZE: Size = Size {
    w: geo::VIEW_W,
    h: geo::VIEW_H,
};
/// A new card opens in its window, then stays expanded on the board.
pub const NEW_CARD_SIZE: Size = Size {
    w: geo::EXPANDED_W,
    h: geo::EXPANDED_H,
};

/// `isTerminal(card)`.
pub fn is_terminal(card: &SessionCard) -> bool {
    card.node.kind == Some(SessionKind::Terminal)
}

/// `cardTitle(card)`: the user's title, else the transcript's, else a placeholder.
pub fn card_title(card: &SessionCard) -> String {
    if let Some(title) = card.node.title.as_deref().filter(|t| !t.is_empty()) {
        return title.to_owned();
    }
    if let Some(title) = card.meta.as_ref().map(|m| m.title.as_str()).filter(|t| !t.is_empty()) {
        return title.to_owned();
    }
    if is_terminal(card) { "Terminal" } else { "New session" }.to_owned()
}

/// `isExpanded(card)`.
pub fn is_expanded(card: &SessionCard) -> bool {
    geo::is_expanded(Some(&card.node))
}

/// `cardSize(card)`.
pub fn card_size(card: &SessionCard) -> Size {
    geo::card_size(Some(&card.node))
}

/// A card's rect where it is stored.
pub fn card_rect(card: &SessionCard) -> Rect {
    rect_at(
        Point {
            x: card.node.x,
            y: card.node.y,
        },
        card_size(card),
    )
}

pub fn rect_at(p: Point, size: Size) -> Rect {
    Rect {
        x: p.x,
        y: p.y,
        w: size.w,
        h: size.h,
    }
}

/// `containsPoint`: edges count.
pub fn contains_point(r: Rect, p: Point) -> bool {
    p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h
}

/// The smallest rect holding all of `rects`.
pub fn bounds_of(rects: &[Rect]) -> Rect {
    let x = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let y = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let right = rects.iter().map(|r| r.x + r.w).fold(f64::NEG_INFINITY, f64::max);
    let bottom = rects.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
    Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    }
}

/// `isInArchive`: something of `size` at `position` counts as in `zone` when
/// its centre lies inside.
pub fn is_in_archive(position: Point, zone: Rect, size: Size) -> bool {
    contains_point(
        zone,
        Point {
            x: position.x + size.w / 2.0,
            y: position.y + size.h / 2.0,
        },
    )
}

/// `cardObstacles`: every card's rect but the excluded ones.
pub fn card_obstacles<'a>(cards: impl IntoIterator<Item = &'a SessionCard>, exclude: &[&str]) -> Vec<Rect> {
    cards
        .into_iter()
        .filter(|c| !exclude.contains(&c.node.session_id.as_str()))
        .map(card_rect)
        .collect()
}

/// `stackZone`: the folded stack, or with `results` the open panel.
pub fn stack_zone(archive: Rect, results: Option<usize>) -> Rect {
    let Some(results) = results else {
        return Rect {
            x: archive.x,
            y: archive.y,
            w: STACK_W,
            h: STACK_H,
        };
    };
    let cols = if results > 1 { 2.0 } else { 1.0 };
    let rows = STACK_ROWS.min(1f64.max((results as f64 / 2.0).ceil()));
    Rect {
        x: archive.x,
        y: archive.y,
        w: STACK_RESULTS_X + cols * CARD_W + (cols - 1.0) * GAP + ARCHIVE_PAD,
        h: STACK_H.max(ARCHIVE_HEADER_H + rows * CARD_H + (rows - 1.0) * GAP + ARCHIVE_PAD),
    }
}

/// Where every card in the stack sits.
pub fn stack_top(archive: Rect) -> Point {
    Point {
        x: archive.x + ARCHIVE_PAD,
        y: archive.y + ARCHIVE_HEADER_H,
    }
}

/// Where a result sits in the open panel, scrolled up by `scroll`.
pub fn stack_result_spot(archive: Rect, slot: usize, scroll: f64) -> Point {
    Point {
        x: archive.x + STACK_RESULTS_X + (slot % 2) as f64 * (CARD_W + GAP),
        y: archive.y + ARCHIVE_HEADER_H + (slot / 2) as f64 * (CARD_H + GAP) - scroll,
    }
}

/// The band of the open panel the results scroll through: (top, bottom).
pub fn stack_results_band(archive: Rect) -> (f64, f64) {
    let top = archive.y + ARCHIVE_HEADER_H;
    (top, top + STACK_BAND_H)
}

/// How far the results can scroll.
pub fn stack_scroll_max(results: usize) -> f64 {
    0f64.max(((results as f64 / 2.0).ceil() - STACK_ROWS) * (CARD_H + GAP))
}

/// Stack order: most recently archived or active first (BOARD-011).
pub fn stack_rank(card: &SessionCard) -> f64 {
    let updated = card.meta.as_ref().map_or(card.node.created_at, |m| m.updated_at);
    card.node.archived_at.unwrap_or(0.0).max(updated)
}

/// `spotRightOfArchive`.
pub fn spot_right_of_archive(
    archive: Rect,
    cards: &[SessionCard],
    exclude: &[&str],
    size: Size,
    others: &[Rect],
) -> Point {
    let mut obstacles = card_obstacles(cards, exclude);
    obstacles.extend_from_slice(others);
    find_free_spot(
        Point {
            x: archive.x + archive.w + 80.0,
            y: archive.y,
        },
        &obstacles,
        Some(archive),
        size,
    )
}

/// `spotForNewSession`: right of the rightmost active card.
pub fn spot_for_new_session(archive: Rect, cards: &[SessionCard], size: Size, others: &[Rect]) -> Point {
    let mut active = cards.iter().filter(|c| !c.node.archived);
    let Some(first) = active.next() else {
        return spot_right_of_archive(archive, cards, &[], size, others);
    };
    // `reduce((a, b) => (b.x > a.x ? b : a))`: the first of a tie stays.
    let rightmost = active.fold(first, |a, b| if b.node.x > a.node.x { b } else { a });
    let mut obstacles = card_obstacles(cards, &[]);
    obstacles.extend_from_slice(others);
    find_free_spot(
        Point {
            x: rightmost.node.x + card_size(rightmost).w + GAP * 2.0,
            y: rightmost.node.y,
        },
        &obstacles,
        Some(archive),
        size,
    )
}

/// `spotBelow`: centred below the sources, never in the archive.
pub fn spot_below(sources: &[&SessionCard], archive: Rect, size: Size) -> Point {
    let x = sources.iter().map(|c| c.node.x).sum::<f64>() / sources.len() as f64;
    let y = sources
        .iter()
        .map(|c| c.node.y + card_size(c).h)
        .fold(f64::NEG_INFINITY, f64::max)
        + 96.0;
    let spot = Point { x, y };
    if !intersects(rect_at(spot, size), archive, GAP) {
        return spot;
    }
    Point {
        x: archive.x + archive.w + 80.0,
        y: spot.y.max(archive.y).min(archive.y + archive.h - size.h),
    }
}

/// `truncate`: `max` UTF-16 units at most, cut with `…`.
pub fn truncate(text: &str, max: usize) -> String {
    if js::utf16_len(text) <= max {
        return text.to_owned();
    }
    format!("{}…", js::trim_end(&js::slice_utf16(text, 0, max - 1)))
}

// ---- frames ------------------------------------------------------------------

/// A frame (stored as a section) with its box and the active cards in it.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionView {
    pub section: BoardSection,
    pub frame: Rect,
    /// Indices into the board's cards: the active ones whose centre is in it.
    pub members: Vec<usize>,
}

/// Every frame with its box and the active cards in it, oldest first
/// (FRAME-002, FRAME-003, FRAME-010). One saved before frames had a size is
/// drawn round the cards that name it, as sections were (BOARD-081).
pub fn section_views(sections: &[BoardSection], cards: &[SessionCard]) -> Vec<SectionView> {
    sections
        .iter()
        .map(|section| {
            let named: Vec<Rect> = cards
                .iter()
                .filter(|c| !c.node.archived && c.node.section_id.as_deref() == Some(section.id.as_str()))
                .map(card_rect)
                .collect();
            let frame = nodus_board::frames::frame_rect(section, &named);
            let members = cards
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.node.archived && holds_centre(frame, card_rect(c)))
                .map(|(i, _)| i)
                .collect();
            SectionView {
                frame,
                section: section.clone(),
                members,
            }
        })
        .collect()
}

/// The frame that holds the centre of something of `size` at `position`:
/// the smallest, where frames overlap (FRAME-010).
pub fn section_at(position: Point, views: &[SectionView], size: Size) -> Option<String> {
    let r = rect_at(position, size);
    views
        .iter()
        .filter(|v| holds_centre(v.frame, r))
        .min_by(|a, b| {
            (a.frame.w * a.frame.h)
                .partial_cmp(&(b.frame.w * b.frame.h))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|v| v.section.id.clone())
}

/// `sectionStatus` (BOARD-083).
pub fn section_status<'a>(members: impl IntoIterator<Item = &'a SessionCard> + Clone) -> Option<SessionStatus> {
    if members.clone().into_iter().any(|c| c.status == SessionStatus::Waiting) {
        Some(SessionStatus::Waiting)
    } else if members.into_iter().any(|c| c.status == SessionStatus::Working) {
        Some(SessionStatus::Working)
    } else {
        None
    }
}

/// FRAME-020: the frame for a new frame round `items` (each by id, where it
/// is), and where each one goes when they had to be tidied: when the frame
/// round them would cross something else (`others`) or come near a `zone`,
/// they're packed in rows, in reading order, at the nearest free spot. A
/// row holds up to four, as `layoutGroup` laid cards out (BOARD-088).
pub fn layout_frame(items: &[(String, Rect)], others: &[Rect], zones: &[Rect]) -> (Rect, Vec<(String, Point)>) {
    let rects: Vec<Rect> = items.iter().map(|(_, r)| *r).collect();
    let around = frame_around(Point { x: 0.0, y: 0.0 }, &rects);
    let clear =
        !others.iter().any(|o| intersects(*o, around, 0.0)) && !zones.iter().any(|z| intersects(*z, around, GAP));
    if clear || items.is_empty() {
        return (around, Vec::new());
    }
    let n = items.len() as f64;
    let cols = GROUP_MAX_COLS.min(n.sqrt().ceil()) as usize;
    let tallest = rects.iter().map(|r| r.h).fold(f64::NEG_INFINITY, f64::max);
    // Reading order: rows top to bottom, then left to right (a stable sort).
    let row = |r: &Rect| js::round(r.y / (tallest + GAP));
    let mut sorted: Vec<&(String, Rect)> = items.iter().collect();
    sorted.sort_by(|(_, a), (_, b)| {
        let d = row(a) - row(b);
        let d = if d != 0.0 { d } else { a.x - b.x };
        d.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
    });
    // Each row as wide as its things and as tall as its tallest.
    let mut offsets: Vec<(String, Point)> = Vec::new();
    let (mut y, mut width) = (0.0f64, 0.0f64);
    for chunk in sorted.chunks(cols) {
        let mut x = 0.0;
        for (id, r) in chunk {
            offsets.push((id.clone(), Point { x, y }));
            x += r.w + GAP;
        }
        width = width.max(x - GAP);
        y += chunk.iter().map(|(_, r)| r.h).fold(0.0, f64::max) + GAP;
    }
    let size = Size {
        w: SECTION_PAD * 2.0 + width,
        h: SECTION_HEADER_H + (y - GAP) + SECTION_PAD,
    };
    let mut obstacles = others.to_vec();
    obstacles.extend_from_slice(zones);
    let corner = find_free_spot(
        Point {
            x: around.x,
            y: around.y,
        },
        &obstacles,
        None,
        size,
    );
    let positions = offsets
        .into_iter()
        .map(|(id, o)| {
            (
                id,
                Point {
                    x: js::round(corner.x + SECTION_PAD + o.x),
                    y: js::round(corner.y + SECTION_HEADER_H + o.y),
                },
            )
        })
        .collect();
    (
        Rect {
            x: corner.x,
            y: corner.y,
            w: size.w,
            h: size.h,
        },
        positions,
    )
}

/// `spotInSection` (BOARD-097, FRAME-024): where something of `size` goes in
/// a frame: the first free slot of a grid from its top-left corner, rows on
/// past its bottom when it's full (the frame then grows to hold it). Free
/// is as `findFreeSpot` has it, so a card started there stays there.
pub fn spot_in_section(frame: Rect, obstacles: &[Rect], avoid: Option<Rect>, size: Size) -> Point {
    let step_x = size.w + GAP;
    let step_y = size.h + GAP;
    let cols = 1f64.max(((frame.w - SECTION_PAD * 2.0 + GAP) / step_x).floor()) as usize;
    let at = |row: usize, col: usize| Point {
        x: frame.x + SECTION_PAD + col as f64 * step_x,
        y: frame.y + SECTION_HEADER_H + row as f64 * step_y,
    };
    let blocked = |p: Point| {
        let r = rect_at(p, size);
        avoid.is_some_and(|a| intersects(r, a, GAP)) || obstacles.iter().any(|o| intersects(r, *o, GAP / 2.0))
    };
    (0..40)
        .flat_map(|row| (0..cols).map(move |col| (row, col)))
        .map(|(row, col)| at(row, col))
        .find(|p| !blocked(*p))
        .unwrap_or_else(|| at(40, 0))
}

/// `createSectionAt`'s spot for a new empty frame of `size` with its header
/// at `p` (BOARD-092, FRAME-005): there when it's clear, else the nearest
/// free spot. (0.9.4 sent a section it couldn't place there 40 rows down;
/// that was a small section that grew round its cards, but a frame keeps
/// its size, so it goes where it can be seen.)
pub fn new_section_corner(p: Point, obstacles: &[Rect], size: Size) -> Point {
    let preferred = Point {
        x: p.x - SECTION_PAD,
        y: p.y - SECTION_HEADER_H / 2.0,
    };
    find_free_spot(preferred, obstacles, None, size)
}

// ---- notes and views ----------------------------------------------------------

/// `noteObstacles`: the free notes' rects (stickers ride on their cards).
pub fn note_obstacles(notes: &[BoardNote]) -> Vec<Rect> {
    notes
        .iter()
        .filter(|n| n.session_id.is_none())
        .map(|n| rect_at(Point { x: n.x, y: n.y }, geo::note_size(n)))
        .collect()
}

/// `viewObstacles`.
pub fn view_obstacles(views: &[BoardView]) -> Vec<Rect> {
    views
        .iter()
        .map(|v| rect_at(Point { x: v.x, y: v.y }, geo::view_size(v)))
        .collect()
}

/// `splitNotes`: the notes on the board, and the ones stuck on a card that's
/// on the board, by session.
pub fn split_notes<'a>(
    notes: &'a [BoardNote],
    cards: &[SessionCard],
) -> (Vec<&'a BoardNote>, Vec<(String, Vec<&'a BoardNote>)>) {
    let mut free = Vec::new();
    let mut stuck: Vec<(String, Vec<&BoardNote>)> = Vec::new();
    for note in notes {
        match note.session_id.as_deref() {
            Some(sid) if cards.iter().any(|c| c.node.session_id == sid) => {
                if let Some((_, list)) = stuck.iter_mut().find(|(id, _)| id == sid) {
                    list.push(note);
                } else {
                    stuck.push((sid.to_owned(), vec![note]));
                }
            }
            _ => free.push(note),
        }
    }
    (free, stuck)
}

/// `padSheetSpot`: where the pad's top sheet sits.
pub fn pad_sheet_spot(pad: Rect) -> Point {
    Point {
        x: pad.x + PAD_SHEET_X,
        y: pad.y + PAD_HEADER_H,
    }
}

/// `isOnPad`.
pub fn is_on_pad(position: Point, pad: Rect, size: Size) -> bool {
    is_in_archive(position, pad, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_board::geometry::CARD_SIZE;
    use nodus_domain::{BoardNode, FolderColor, SessionMeta};
    use serde_json::Map;

    pub(crate) fn card(id: &str, x: f64, y: f64) -> SessionCard {
        SessionCard {
            node: BoardNode {
                session_id: id.into(),
                kind: None,
                x,
                y,
                archived: false,
                title: None,
                parents: None,
                created_by_app: None,
                run_of: None,
                expanded: None,
                view_of: None,
                auto_mode: None,
                from_view: None,
                created_at: 0.0,
                w: None,
                h: None,
                archived_at: None,
                section_id: None,
                extra: Map::new(),
            },
            meta: None,
            status: SessionStatus::Done,
            live: false,
            agents: None,
            agent_count: None,
            usage: None,
        }
    }

    fn section(id: &str, x: f64, y: f64) -> BoardSection {
        BoardSection {
            id: id.into(),
            name: String::new(),
            color: FolderColor::Blue,
            x,
            y,
            w: None,
            h: None,
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    const ARCHIVE: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 344.0,
        h: 362.0,
    };

    #[test]
    fn contains_points_on_its_edges() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        assert!(contains_point(r, Point { x: 10.0, y: 0.0 }));
        assert!(!contains_point(r, Point { x: 10.1, y: 0.0 }));
        assert_eq!(
            bounds_of(&[
                r,
                Rect {
                    x: 20.0,
                    y: -5.0,
                    w: 5.0,
                    h: 5.0
                }
            ]),
            Rect {
                x: 0.0,
                y: -5.0,
                w: 25.0,
                h: 15.0
            }
        );
        assert!(is_in_archive(
            Point { x: -100.0, y: 0.0 },
            r,
            Size { w: 210.0, h: 10.0 }
        ));
    }

    #[test]
    fn sizes_the_stack_zone_for_its_results() {
        assert_eq!(
            stack_zone(ARCHIVE, None),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 344.0,
                h: 362.0
            }
        );
        let sizes: Vec<(f64, f64)> = [0, 1, 2, 3, 4, 5]
            .iter()
            .map(|&n| {
                let z = stack_zone(ARCHIVE, Some(n));
                (z.w, z.h)
            })
            .collect();
        assert_eq!(
            sizes,
            [
                (672.0, 362.0),
                (672.0, 362.0),
                (992.0, 362.0),
                (992.0, 544.0),
                (992.0, 544.0),
                (992.0, 544.0)
            ]
        );
        assert_eq!(stack_top(ARCHIVE), Point { x: 28.0, y: 52.0 });
        assert_eq!(stack_result_spot(ARCHIVE, 3, 10.0), Point { x: 676.0, y: 290.0 });
        assert_eq!(stack_results_band(ARCHIVE), (52.0, 516.0));
        assert_eq!(stack_scroll_max(4), 0.0);
        assert_eq!(stack_scroll_max(5), 248.0);
        assert_eq!(stack_scroll_max(8), 496.0);
    }

    #[test]
    fn ranks_the_stack_by_last_archived_or_active() {
        let mut a = card("a", 0.0, 0.0);
        a.node.created_at = 5.0;
        a.node.archived_at = Some(3.0);
        assert_eq!(stack_rank(&a), 5.0);
        a.meta = Some(SessionMeta {
            session_id: "a".into(),
            title: "t".into(),
            first_prompt: None,
            last_prompt: None,
            last_assistant_text: None,
            git_branch: None,
            message_count: 0,
            created_at: 0.0,
            updated_at: 2.0,
            transcript_path: String::new(),
        });
        assert_eq!(stack_rank(&a), 3.0);
    }

    #[test]
    fn starts_new_sessions_right_of_the_rightmost_card() {
        let cards = [card("a", 500.0, 0.0), card("b", 500.0, 600.0)];
        // A tie keeps the first.
        assert_eq!(
            spot_for_new_session(ARCHIVE, &cards, CARD_SIZE, &[]),
            Point { x: 852.0, y: 0.0 }
        );
        assert_eq!(
            spot_for_new_session(ARCHIVE, &[], CARD_SIZE, &[]),
            Point { x: 424.0, y: 0.0 }
        );
    }

    #[test]
    fn puts_a_merge_below_its_sources_or_beside_the_archive() {
        let a = card("a", 400.0, 0.0);
        let b = card("b", 800.0, 100.0);
        assert_eq!(spot_below(&[&a, &b], ARCHIVE, CARD_SIZE), Point { x: 600.0, y: 412.0 });
        let c = card("c", 0.0, 0.0);
        assert_eq!(spot_below(&[&c], ARCHIVE, CARD_SIZE), Point { x: 424.0, y: 146.0 });
    }

    #[test]
    fn truncates_in_utf16_units() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 7), "hello…");
        // The emoji is two UTF-16 units.
        assert_eq!(truncate("ab😀cd", 5), "ab😀…");
    }

    #[test]
    fn frames_sections_around_their_cards() {
        assert_eq!(
            frame_around(Point { x: 5.0, y: 6.0 }, &[]),
            Rect {
                x: 5.0,
                y: 6.0,
                w: 336.0,
                h: 284.0
            }
        );
        let cards = [card("a", 100.0, 100.0), card("b", 500.0, 200.0)];
        let mut cards = cards.to_vec();
        cards[0].node.section_id = Some("s".into());
        cards[1].node.section_id = Some("s".into());
        let mut archived = card("c", 0.0, 0.0);
        archived.node.archived = true;
        archived.node.section_id = Some("s".into());
        cards.push(archived);
        let mut stray = card("d", 0.0, 0.0);
        stray.node.section_id = Some("gone".into());
        cards.push(stray);
        // A frame from before frames had a size wraps the cards naming it;
        // what's in it is what lies in it, the stray's centre included.
        let views = section_views(&[section("s", 0.0, 0.0)], &cards);
        assert_eq!(views[0].members, [0, 1, 3]);
        assert_eq!(
            views[0].frame,
            Rect {
                x: 76.0,
                y: 56.0,
                w: 736.0,
                h: 384.0
            }
        );
        // FRAME-010: one with a size holds whatever's centre is in it.
        let mut sized = section("t", 0.0, 0.0);
        sized.w = Some(450.0);
        sized.h = Some(450.0);
        let views = section_views(&[sized], &cards);
        assert_eq!(views[0].members, [0, 3]);
        assert_eq!(views[0].frame.w, 450.0);
    }

    #[test]
    fn finds_the_section_under_a_cards_centre() {
        let views = vec![
            SectionView {
                section: section("old", 0.0, 0.0),
                frame: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 400.0,
                    h: 400.0,
                },
                members: vec![],
            },
            SectionView {
                section: section("new", 0.0, 0.0),
                frame: Rect {
                    x: 100.0,
                    y: 0.0,
                    w: 400.0,
                    h: 400.0,
                },
                members: vec![],
            },
        ];
        let at = Point { x: 50.0, y: 50.0 };
        assert_eq!(section_at(at, &views, CARD_SIZE).as_deref(), Some("old"));
        assert_eq!(
            section_at(at, &views, Size { w: 10.0, h: 10.0 }).as_deref(),
            Some("old")
        );
        let right = Point { x: 300.0, y: 50.0 };
        assert_eq!(section_at(right, &views, CARD_SIZE).as_deref(), Some("new"));
        assert_eq!(section_at(Point { x: 900.0, y: 0.0 }, &views, CARD_SIZE), None);
        // The smaller wins where they overlap.
        let mut small = views[1].clone();
        small.section.id = "small".into();
        small.frame = Rect {
            x: 100.0,
            y: 0.0,
            w: 300.0,
            h: 300.0,
        };
        let views = vec![views[0].clone(), small];
        assert_eq!(
            section_at(Point { x: 150.0, y: 50.0 }, &views, Size { w: 10.0, h: 10.0 }).as_deref(),
            Some("small")
        );
    }

    #[test]
    fn rates_a_section_by_its_most_urgent_card() {
        let mut a = card("a", 0.0, 0.0);
        let mut b = card("b", 0.0, 0.0);
        assert_eq!(section_status([&a, &b]), None);
        a.status = SessionStatus::Working;
        assert_eq!(section_status([&a, &b]), Some(SessionStatus::Working));
        b.status = SessionStatus::Waiting;
        assert_eq!(section_status([&a, &b]), Some(SessionStatus::Waiting));
    }

    fn items(cards: &[SessionCard]) -> Vec<(String, Rect)> {
        cards
            .iter()
            .map(|c| (c.node.session_id.clone(), card_rect(c)))
            .collect()
    }

    #[test]
    fn frames_things_where_they_are_when_clear() {
        let cards = [card("a", 1000.0, 1000.0), card("b", 1320.0, 1000.0)];
        let (frame, moves) = layout_frame(&items(&cards), &[], &[ARCHIVE]);
        assert!(moves.is_empty());
        assert_eq!(
            frame,
            Rect {
                x: 976.0,
                y: 956.0,
                w: 656.0,
                h: 284.0
            }
        );
    }

    #[test]
    fn packs_things_of_different_sizes_row_by_row() {
        let things = vec![
            (
                "view".to_owned(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 480.0,
                    h: 360.0,
                },
            ),
            (
                "sticker".to_owned(),
                Rect {
                    x: 600.0,
                    y: 0.0,
                    w: 64.0,
                    h: 64.0,
                },
            ),
            (
                "note".to_owned(),
                Rect {
                    x: 0.0,
                    y: 500.0,
                    w: 216.0,
                    h: 216.0,
                },
            ),
        ];
        let blocker = Rect {
            x: 300.0,
            y: 400.0,
            w: 10.0,
            h: 10.0,
        };
        let (frame, moves) = layout_frame(&things, &[blocker], &[]);
        // Two to a row: the view and the sticker, then the note.
        assert_eq!(
            (frame.w, frame.h),
            (24.0 * 2.0 + 480.0 + 32.0 + 64.0, 44.0 + 360.0 + 32.0 + 216.0 + 24.0)
        );
        assert_eq!(moves[1].1.x - moves[0].1.x, 480.0 + 32.0);
        assert_eq!(moves[2].1.y - moves[0].1.y, 360.0 + 32.0);
    }

    #[test]
    fn tidies_a_blocked_frame_into_rows_in_reading_order() {
        let cards = [
            card("c", 1320.0, 1300.0),
            card("a", 1000.0, 1000.0),
            card("b", 1320.0, 1000.0),
        ];
        let blocker = Rect {
            x: 1200.0,
            y: 1200.0,
            w: 10.0,
            h: 10.0,
        };
        let (frame, moves) = layout_frame(&items(&cards), &[blocker], &[]);
        // 2 columns, 2 rows of 288×216 cells.
        assert_eq!((frame.w, frame.h), (656.0, 532.0));
        let ids: Vec<&str> = moves.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(
            moves[0].1,
            Point {
                x: frame.x + 24.0,
                y: frame.y + 44.0
            }
        );
        assert_eq!(
            moves[2].1,
            Point {
                x: frame.x + 24.0,
                y: frame.y + 44.0 + 248.0
            }
        );
    }

    #[test]
    fn finds_a_slot_in_a_frame_or_below_its_bottom() {
        let frame = Rect {
            x: 0.0,
            y: 0.0,
            w: 656.0,
            h: 284.0,
        };
        let spot = |obstacles: &[Rect]| spot_in_section(frame, obstacles, None, CARD_SIZE);
        assert_eq!(spot(&[]), Point { x: 24.0, y: 44.0 });
        let first = rect_at(Point { x: 24.0, y: 44.0 }, CARD_SIZE);
        assert_eq!(spot(&[first]), Point { x: 344.0, y: 44.0 });
        let second = rect_at(Point { x: 344.0, y: 44.0 }, CARD_SIZE);
        // Full: the next row, past the frame's bottom.
        assert_eq!(spot(&[first, second]), Point { x: 24.0, y: 292.0 });
        // An expanded card takes a whole row of this frame.
        let big = spot_in_section(frame, &[first], None, NEW_CARD_SIZE);
        assert_eq!(big, Point { x: 24.0, y: 556.0 });
    }

    #[test]
    fn places_a_new_frame_by_the_click_or_the_nearest_free_spot() {
        let p = Point { x: 1000.0, y: 1000.0 };
        let size = Size { w: 768.0, h: 548.0 };
        assert_eq!(new_section_corner(p, &[], size), Point { x: 976.0, y: 978.0 });
        let near = Rect {
            x: 1100.0,
            y: 1100.0,
            w: 10.0,
            h: 10.0,
        };
        let moved = new_section_corner(p, &[near], size);
        assert!(!intersects(rect_at(moved, size), near, GAP / 2.0));
        assert!((moved.y - 978.0).abs() < 2000.0, "{moved:?}");
    }

    #[test]
    fn splits_notes_into_free_and_stuck() {
        let note = |id: &str, sid: Option<&str>| BoardNote {
            id: id.into(),
            text: String::new(),
            color: FolderColor::Yellow,
            x: 0.0,
            y: 0.0,
            w: None,
            h: None,
            session_id: sid.map(Into::into),
            from: None,
            from_project: None,
            from_title: None,
            created_at: 0.0,
            extra: Map::new(),
        };
        let notes = [note("1", None), note("2", Some("a")), note("3", Some("gone"))];
        let (free, stuck) = split_notes(&notes, &[card("a", 0.0, 0.0)]);
        let free: Vec<&str> = free.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(free, ["1", "3"]);
        assert_eq!(stuck[0].0, "a");
        assert_eq!(note_obstacles(&notes).len(), 1);
    }

    #[test]
    fn titles_cards_as_0_9_4_does() {
        let mut c = card("a", 0.0, 0.0);
        assert_eq!(card_title(&c), "New session");
        c.node.kind = Some(SessionKind::Terminal);
        assert_eq!(card_title(&c), "Terminal");
        c.node.title = Some(String::new());
        assert_eq!(card_title(&c), "Terminal");
        c.node.title = Some("Mine".into());
        assert_eq!(card_title(&c), "Mine");
    }
}
