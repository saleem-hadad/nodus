//! Shapes and links on the board: boxes, ellipses, diamonds, words, stickers
//! and pictures, and the arrows between them and the cards, notes and views.
//! Added after 0.9.4. What the window or a session's board tools send is
//! checked here before it's kept.
//!
//! The functions work on a board in hand; whoever calls them holds the store
//! and saves it (`Store::update` does).

use nodus_domain::js;
use nodus_domain::{BoardLink, BoardShape, LinkEnd, Rect, ShapeKind};
use nodus_store::BoardState;
use serde_json::Map;

use crate::cards::set;
use crate::geometry::{GAP, Point, Size, card_size, dock_rect, find_free_spot, find_spot_beside, note_size, view_size};
use crate::notes::{obstacles, rightmost_card};

/// The smallest and largest a shape is, in board units.
pub const SHAPE_MIN: f64 = 16.0;
pub const SHAPE_MAX: f64 = 4000.0;
/// How much text a shape keeps, in UTF-16 units.
pub const SHAPE_MAX_TEXT: usize = 2000;
/// How long a sticker's emoji is at most.
pub const STICKER_MAX_TEXT: usize = 16;
/// How much text a link's label keeps.
pub const LINK_MAX_LABEL: usize = 200;
/// How many shapes and links a board keeps.
pub const MAX_SHAPES: usize = 2000;
pub const MAX_LINKS: usize = 2000;

/// The longest id kept, as ink's.
const MAX_ID: usize = 64;

fn id_ok(id: &str) -> bool {
    !id.is_empty() && js::utf16_len(id) <= MAX_ID
}

/// `Math.round(n * 10) / 10`.
fn r1(n: f64) -> f64 {
    js::round(n * 10.0) / 10.0
}

/// A picture's file name: a plain name in the images folder, never a path.
pub fn image_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The shape worth keeping, cleaned up; none when it's malformed.
pub fn clean_shape(shape: &BoardShape) -> Option<BoardShape> {
    if !id_ok(&shape.id) {
        return None;
    }
    let numbers = [shape.x, shape.y, shape.w, shape.h];
    if numbers.iter().any(|n| !n.is_finite()) {
        return None;
    }
    let text = match shape.kind {
        ShapeKind::Sticker => js::slice_utf16(js::trim(&shape.text), 0, STICKER_MAX_TEXT),
        ShapeKind::Image => String::new(),
        _ => js::slice_utf16(js::trim_end(&shape.text), 0, SHAPE_MAX_TEXT),
    };
    if shape.kind == ShapeKind::Sticker && text.is_empty() {
        return None;
    }
    let image = match shape.kind {
        ShapeKind::Image => Some(shape.image.clone().filter(|n| image_name_ok(n))?),
        _ => None,
    };
    Some(BoardShape {
        id: shape.id.clone(),
        kind: shape.kind,
        x: js::round(shape.x),
        y: js::round(shape.y),
        w: js::round(shape.w.clamp(SHAPE_MIN, SHAPE_MAX)),
        h: js::round(shape.h.clamp(SHAPE_MIN, SHAPE_MAX)),
        color: shape.color,
        text,
        image,
        by: shape.by.clone(),
        created_at: shape.created_at,
        extra: Map::new(),
    })
}

fn clean_end(end: &LinkEnd) -> Option<LinkEnd> {
    if !end.x.is_finite() || !end.y.is_finite() {
        return None;
    }
    let node = match end.node.as_deref() {
        Some(id) if !id_ok(id) => return None,
        node => node.map(str::to_owned),
    };
    Some(LinkEnd {
        node,
        x: r1(end.x),
        y: r1(end.y),
    })
}

/// The link worth keeping, cleaned up; none when it's malformed or both of
/// its ends hold the same thing.
pub fn clean_link(link: &BoardLink) -> Option<BoardLink> {
    if !id_ok(&link.id) {
        return None;
    }
    let (from, to) = (clean_end(&link.from)?, clean_end(&link.to)?);
    if from.node.is_some() && from.node == to.node {
        return None;
    }
    Some(BoardLink {
        id: link.id.clone(),
        from,
        to,
        color: link.color,
        label: js::slice_utf16(js::trim(&link.label), 0, LINK_MAX_LABEL),
        by: link.by.clone(),
        created_at: link.created_at,
        extra: Map::new(),
    })
}

/// Whether something with this id is on the board, archived cards included
/// (their links wait for them to come back).
pub fn holds(board: &BoardState, id: &str) -> bool {
    board.nodes.contains_key(id)
        || board.notes.contains_key(id)
        || board.views.contains_key(id)
        || board.shapes.contains_key(id)
}

/// Whether both of a link's ends are there to hold it.
pub fn ends_held(board: &BoardState, link: &BoardLink) -> bool {
    [&link.from, &link.to]
        .iter()
        .all(|end| end.node.as_deref().is_none_or(|id| holds(board, id)))
}

/// Drops the links whose ends lost what held them.
pub fn prune_links(board: &mut BoardState) {
    let gone: Vec<String> = board
        .links
        .values()
        .filter(|l| !ends_held(board, l))
        .map(|l| l.id.clone())
        .collect();
    for id in gone {
        board.links.shift_remove(&id);
    }
}

/// Keeps these shapes, new or changed: each is cleaned, and one already on
/// the board keeps when it was made and who drew it.
pub fn save_shapes(board: &mut BoardState, shapes: &[BoardShape], now: f64) {
    for shape in shapes {
        let Some(mut clean) = clean_shape(shape) else {
            continue;
        };
        match board.shapes.get(&clean.id) {
            Some(stored) => {
                clean.created_at = stored.created_at;
                clean.by.clone_from(&stored.by);
                clean.extra.clone_from(&stored.extra);
            }
            None => {
                if board.shapes.values().count() >= MAX_SHAPES {
                    continue;
                }
                if !clean.created_at.is_finite() || clean.created_at <= 0.0 {
                    clean.created_at = now;
                }
            }
        }
        board.shapes.insert(clean.id.clone(), clean);
    }
}

/// Takes shapes off the board, and the links they held.
pub fn remove_shapes(board: &mut BoardState, ids: &[String]) {
    for id in ids {
        board.shapes.shift_remove(id);
    }
    prune_links(board);
}

/// Keeps these links, new or changed. One whose ends aren't on the board is
/// left out.
pub fn save_links(board: &mut BoardState, links: &[BoardLink], now: f64) {
    for link in links {
        let Some(mut clean) = clean_link(link) else {
            continue;
        };
        if !ends_held(board, &clean) {
            continue;
        }
        match board.links.get(&clean.id) {
            Some(stored) => {
                clean.created_at = stored.created_at;
                clean.by.clone_from(&stored.by);
                clean.extra.clone_from(&stored.extra);
            }
            None => {
                if board.links.values().count() >= MAX_LINKS {
                    continue;
                }
                if !clean.created_at.is_finite() || clean.created_at <= 0.0 {
                    clean.created_at = now;
                }
            }
        }
        board.links.insert(clean.id.clone(), clean);
    }
}

pub fn remove_links(board: &mut BoardState, ids: &[String]) {
    for id in ids {
        board.links.shift_remove(id);
    }
}

/// Where something on the board is drawn: an active card, a free note, a
/// view or a shape. None for anything else (an archived card, a run, a note
/// stuck to a card).
pub fn item_rect(board: &BoardState, id: &str) -> Option<Rect> {
    let rect = |x: f64, y: f64, w: f64, h: f64| Rect { x, y, w, h };
    if let Some(n) = board.nodes.get(id) {
        if n.archived || set(n.run_of.as_deref()) || set(n.view_of.as_deref()) {
            return None;
        }
        let size = card_size(Some(n));
        return Some(rect(n.x, n.y, size.w, size.h));
    }
    if let Some(n) = board.notes.get(id) {
        if set(n.session_id.as_deref()) {
            return None;
        }
        let size = note_size(n);
        return Some(rect(n.x, n.y, size.w, size.h));
    }
    if let Some(v) = board.views.get(id) {
        let size = view_size(v);
        return Some(rect(v.x, v.y, size.w, size.h));
    }
    board.shapes.get(id).map(BoardShape::rect)
}

/// A new shape's or link's id.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// How big a shape a session draws with its board tools is when it doesn't
/// say: room for a few words in a box, a line of words, an emoji.
pub fn drawn_size(kind: ShapeKind) -> Size {
    let size = |w: f64, h: f64| Size { w, h };
    match kind {
        ShapeKind::Rect | ShapeKind::Ellipse => size(180.0, 90.0),
        ShapeKind::Diamond => size(160.0, 110.0),
        ShapeKind::Text => size(220.0, 44.0),
        ShapeKind::Sticker => size(64.0, 64.0),
        ShapeKind::Image => size(240.0, 180.0),
    }
}

/// What a drawing a session puts up keeps clear of: what its notes keep
/// clear of (cards, free notes, views), and the shapes already there.
pub fn drawing_obstacles(board: &BoardState) -> Vec<Rect> {
    let mut out = obstacles(board, None);
    out.extend(board.shapes.values().map(BoardShape::rect));
    out
}

/// Where the top left of a drawing of `size` goes when the session `by`
/// doesn't say: in the nearest free spot beside its card, or beside its view
/// for a view's brain, clear of the archive dock. A session without a card
/// here (a run, an archived card) draws beside the rightmost card instead.
pub fn place_drawing(board: &BoardState, by: &str, size: Size) -> Point {
    let obstacles = drawing_obstacles(board);
    let dock = dock_rect(board.archive);
    let view = board
        .nodes
        .get(by)
        .and_then(|n| n.view_of.as_deref())
        .filter(|id| !id.is_empty())
        .and_then(|id| item_rect(board, id));
    let anchor = item_rect(board, by)
        .or(view)
        .or_else(|| rightmost_card(board).and_then(|n| item_rect(board, &n.session_id)));
    match anchor {
        Some(anchor) => find_spot_beside(anchor, size, &obstacles, Some(dock), GAP),
        None => find_free_spot(
            Point {
                x: dock.x + dock.w + 80.0,
                y: dock.y,
            },
            &obstacles,
            Some(dock),
            size,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardNote, FolderColor, InkColor};

    pub(crate) fn shape(id: &str, kind: ShapeKind) -> BoardShape {
        BoardShape {
            id: id.into(),
            kind,
            x: 10.4,
            y: 20.6,
            w: 160.0,
            h: 100.0,
            color: InkColor::Blue,
            text: "Ship it".into(),
            image: None,
            by: None,
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    fn link(id: &str, from: LinkEnd, to: LinkEnd) -> BoardLink {
        BoardLink {
            id: id.into(),
            from,
            to,
            color: InkColor::Graphite,
            label: String::new(),
            by: None,
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    fn note(id: &str) -> BoardNote {
        BoardNote {
            id: id.into(),
            text: "n".into(),
            color: FolderColor::Yellow,
            x: 0.0,
            y: 0.0,
            w: None,
            h: None,
            session_id: None,
            from: None,
            from_project: None,
            from_title: None,
            created_at: 1.0,
            extra: Map::new(),
        }
    }

    #[test]
    fn cleans_shapes() {
        let s = clean_shape(&shape("a", ShapeKind::Rect)).unwrap();
        assert_eq!((s.x, s.y, s.w, s.h), (10.0, 21.0, 160.0, 100.0));
        let tiny = BoardShape {
            w: 2.0,
            h: 9000.0,
            ..shape("a", ShapeKind::Ellipse)
        };
        let s = clean_shape(&tiny).unwrap();
        assert_eq!((s.w, s.h), (SHAPE_MIN, SHAPE_MAX));
        // A sticker needs its emoji; a picture its file, by a plain name.
        assert!(
            clean_shape(&BoardShape {
                text: " ".into(),
                ..shape("s", ShapeKind::Sticker)
            })
            .is_none()
        );
        assert!(clean_shape(&shape("i", ShapeKind::Image)).is_none());
        for bad in ["../x.png", "a/b.png", ".hidden", ""] {
            let s = BoardShape {
                image: Some(bad.into()),
                ..shape("i", ShapeKind::Image)
            };
            assert!(clean_shape(&s).is_none(), "{bad}");
        }
        let pic = clean_shape(&BoardShape {
            image: Some("3f2a.png".into()),
            ..shape("i", ShapeKind::Image)
        })
        .unwrap();
        assert_eq!((pic.image.as_deref(), pic.text.as_str()), (Some("3f2a.png"), ""));
        assert!(
            clean_shape(&BoardShape {
                x: f64::NAN,
                ..shape("a", ShapeKind::Rect)
            })
            .is_none()
        );
    }

    #[test]
    fn saving_keeps_when_a_shape_was_made_and_who_drew_it() {
        let mut board = BoardState::default();
        let drawn = BoardShape {
            by: Some("sess".into()),
            ..shape("a", ShapeKind::Rect)
        };
        save_shapes(&mut board, &[drawn], 5.0);
        let moved = BoardShape {
            x: 300.0,
            by: None,
            created_at: 99.0,
            ..shape("a", ShapeKind::Rect)
        };
        save_shapes(&mut board, &[moved], 9.0);
        let kept = board.shapes.get("a").unwrap();
        assert_eq!(
            (kept.x, kept.created_at, kept.by.as_deref()),
            (300.0, 5.0, Some("sess"))
        );
    }

    #[test]
    fn links_need_their_ends_on_the_board() {
        let mut board = BoardState::default();
        board.notes.insert("n".into(), note("n"));
        save_shapes(&mut board, &[shape("s", ShapeKind::Diamond)], 1.0);
        save_links(
            &mut board,
            &[
                link("ok", LinkEnd::on("n"), LinkEnd::on("s")),
                link("loose", LinkEnd::on("n"), LinkEnd::at(5.04, 6.0)),
                link("gone", LinkEnd::on("n"), LinkEnd::on("nothing")),
                link("self", LinkEnd::on("s"), LinkEnd::on("s")),
            ],
            2.0,
        );
        let ids: Vec<&str> = board.links.values().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["ok", "loose"]);
        assert_eq!(board.links.get("loose").unwrap().to.x, 5.0);
        // Taking a shape off takes its links with it.
        remove_shapes(&mut board, &["s".into()]);
        let ids: Vec<&str> = board.links.values().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["loose"]);
        board.notes.shift_remove("n");
        prune_links(&mut board);
        assert!(board.links.is_empty());
    }
}
