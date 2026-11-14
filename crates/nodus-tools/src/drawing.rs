//! What the drawing tools do on a board, added after 0.9.4: `list_board`
//! shows what's on it and where, and the links touching the session's card;
//! `draw_on_board` puts up shapes and the links between things;
//! `update_drawing` changes shapes and links and moves cards, notes and
//! views; `erase_from_board` takes shapes and links away (TOOL-101 to
//! TOOL-110).
//!
//! The functions work on a board in hand, and check everything they're given
//! before they change anything, so a call that fails leaves the board as it
//! was. The handlers hold the store (`Store::update`) and tell the window.

use std::collections::HashMap;

use nodus_board::cards::SessionListItem;
use nodus_board::geometry::{Point, Size, note_size, view_size};
use nodus_board::shapes::{
    LINK_MAX_LABEL, MAX_LINKS, MAX_SHAPES, SHAPE_MAX, SHAPE_MIN, clean_link, clean_shape, drawn_size, holds, item_rect,
    new_id, place_drawing, remove_links, remove_shapes, save_links, save_shapes,
};
use nodus_domain::{BoardLink, BoardShape, CreatedBy, InkColor, LinkEnd, Rect, ShapeKind, js};
use nodus_store::BoardState;
use serde_json::{Map, Value, json};

/// The kinds a session can draw: every one but a picture.
pub(crate) const DRAWN_KINDS: [ShapeKind; 5] = [
    ShapeKind::Rect,
    ShapeKind::Ellipse,
    ShapeKind::Diamond,
    ShapeKind::Text,
    ShapeKind::Sticker,
];

/// What a link's end names to mean the calling session's own card.
const SELF: &str = "self";
/// Space between the shapes laid out in a row, with room for an arrow.
const ROW_GAP: f64 = 64.0;
/// How much of a note's text list_board shows.
const NOTE_PREVIEW: usize = 200;
/// How much of a note's first line names it in a link.
const NAME_CHARS: usize = 60;

const KINDS: &str = "rect, ellipse, diamond, text or sticker";
const COLORS: &str = "graphite, red, orange, green, blue or purple";

// ---------------------------------------------------------------------------
// Reading what Claude sent

/// A list argument: none when it's left out (or null), an error when it
/// isn't a list.
fn list<'a>(args: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(format!("{key} must be a list")),
    }
}

fn number(item: &Value, key: &str, at: &str) -> Result<Option<f64>, String> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(|| format!("{at}: {key} must be a number")),
    }
}

fn string<'a>(item: &'a Value, key: &str, at: &str) -> Result<Option<&'a str>, String> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("{at}: {key} must be a string")),
    }
}

fn color(item: &Value, at: &str) -> Result<Option<InkColor>, String> {
    match string(item, "color", at)? {
        None => Ok(None),
        Some(name) => InkColor::parse(name)
            .map(Some)
            .ok_or_else(|| format!("{at}: color must be {COLORS}")),
    }
}

fn object<'a>(item: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    item.as_object().ok_or_else(|| format!("{at} must be an object"))
}

/// A shape's side, kept within what the board keeps.
fn side(n: f64) -> f64 {
    js::round(n.clamp(SHAPE_MIN, SHAPE_MAX))
}

/// The words a shape of `kind` holds, checked: words on their own need some,
/// and a sticker its emoji.
fn shape_text(kind: ShapeKind, text: &str, at: &str) -> Result<String, String> {
    let blank = js::trim(text).is_empty();
    match kind {
        ShapeKind::Text if blank => Err(format!("{at}: a text shape needs some text")),
        ShapeKind::Sticker if blank => Err(format!("{at}: a sticker needs its emoji in text")),
        ShapeKind::Image if !text.is_empty() => Err(format!("{at}: a picture has no text")),
        _ => Ok(text.to_owned()),
    }
}

/// `{x, y, w, h}`, rounded, for Claude to read.
fn put_box(item: &mut Value, rect: Rect) {
    item["x"] = json!(js::round(rect.x));
    item["y"] = json!(js::round(rect.y));
    item["w"] = json!(js::round(rect.w));
    item["h"] = json!(js::round(rect.h));
}

fn sized(x: f64, y: f64, size: Size) -> Rect {
    Rect {
        x,
        y,
        w: size.w,
        h: size.h,
    }
}

/// Oldest first; stable, as Array.prototype.sort is.
fn oldest_first<T>(items: &mut [&T], created_at: impl Fn(&T) -> f64) {
    items.sort_by(|a, b| {
        created_at(a)
            .partial_cmp(&created_at(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn center(rect: Rect) -> Point {
    Point {
        x: rect.x + rect.w / 2.0,
        y: rect.y + rect.h / 2.0,
    }
}

/// A link's end on something drawn on the board, at its middle for now.
fn end_on(id: &str, rect: Rect) -> LinkEnd {
    let Point { x, y } = center(rect);
    LinkEnd {
        node: Some(id.to_owned()),
        x,
        y,
    }
}

/// What a link's end names on the board: `"self"` for the calling session's
/// card, or the id of a card, note, view or shape that's drawn there.
fn board_end(board: &BoardState, self_id: &str, name: &str, at: &str) -> Result<(String, Rect), String> {
    let id = if name == SELF { self_id } else { name };
    if let Some(rect) = item_rect(board, id) {
        return Ok((id.to_owned(), rect));
    }
    if name == SELF {
        return Err(format!("{at}: this session has no card on the board to link"));
    }
    if holds(board, id) {
        return Err(format!(
            "{at}: “{id}” isn’t drawn on the board right now (an archived card, an action run or a note stuck to a card can’t be linked)"
        ));
    }
    Err(format!(
        "{at}: “{id}” isn’t a key from shapes or the id of anything on this board; list_board lists what’s there"
    ))
}

// ---------------------------------------------------------------------------
// list_board

/// Calls a thing on the board by what it is and what it says: a card by its
/// title, a note by its first line, a view by its title, a shape by its words.
fn name_of(board: &BoardState, id: &str, cards: &HashMap<&str, &SessionListItem>) -> (&'static str, String) {
    if board.nodes.contains_key(id) {
        let title = cards.get(id).map_or("New session", |c| c.title.as_str());
        return ("card", title.to_owned());
    }
    if let Some(note) = board.notes.get(id) {
        let first = note
            .text
            .lines()
            .map(js::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default();
        return ("note", clip(first, NAME_CHARS));
    }
    if let Some(view) = board.views.get(id) {
        let title = if view.title.is_empty() {
            "A view"
        } else {
            view.title.as_str()
        };
        return ("view", title.to_owned());
    }
    let shape = board.shapes.get(id);
    let words = shape.map(|s| js::trim(&s.text)).filter(|t| !t.is_empty());
    match (shape, words) {
        (_, Some(words)) => ("shape", clip(words, NAME_CHARS)),
        (Some(s), None) => ("shape", s.kind.as_str().to_owned()),
        (None, None) => ("shape", String::new()),
    }
}

/// `text`, cut to `max` UTF-16 units with an ellipsis when it's longer.
fn clip(text: &str, max: usize) -> String {
    if js::utf16_len(text) <= max {
        return text.to_owned();
    }
    format!("{}…", js::trim_end(&js::slice_utf16(text, 0, max)))
}

fn describe_end(board: &BoardState, end: &LinkEnd, cards: &HashMap<&str, &SessionListItem>) -> Value {
    match end.node.as_deref() {
        Some(id) => {
            let (kind, name) = name_of(board, id, cards);
            json!({ "id": id, "kind": kind, "name": name })
        }
        None => json!({ "x": js::round(end.x), "y": js::round(end.y) }),
    }
}

/// One line on a link to or from the calling session's card: `→ card “Fix
/// cart rounding” (label: blocks)` for one it starts, `←` for one it ends.
fn link_line(board: &BoardState, link: &BoardLink, self_id: &str, cards: &HashMap<&str, &SessionListItem>) -> String {
    let (arrow, other) = if link.from.node.as_deref() == Some(self_id) {
        ("→", &link.to)
    } else {
        ("←", &link.from)
    };
    let what = match other.node.as_deref() {
        Some(id) => {
            let (kind, name) = name_of(board, id, cards);
            format!("{kind} “{name}”")
        }
        None => format!("a loose end at ({}, {})", js::round(other.x), js::round(other.y)),
    };
    if link.label.is_empty() {
        format!("{arrow} {what}")
    } else {
        format!("{arrow} {what} (label: {})", link.label)
    }
}

/// Whether a link is drawn: each end loose, or on something drawn on the board.
fn drawn(board: &BoardState, link: &BoardLink) -> bool {
    [&link.from, &link.to]
        .iter()
        .all(|end| end.node.as_deref().is_none_or(|id| item_rect(board, id).is_some()))
}

/// Everything drawn on the board, with each thing's box, and what the
/// calling session's card is linked to and from, in words (TOOL-102).
/// `cards` are the board's active cards as `list_sessions` would list them.
pub(crate) fn list_board(board: &BoardState, self_id: &str, cards: &[SessionListItem]) -> Value {
    let by_id: HashMap<&str, &SessionListItem> = cards.iter().map(|c| (c.id.as_str(), c)).collect();
    let card_items: Vec<Value> = cards
        .iter()
        .filter_map(|c| {
            let rect = item_rect(board, &c.id)?;
            let mut item = json!({ "id": c.id, "title": c.title, "status": c.status });
            put_box(&mut item, rect);
            if c.id == self_id {
                item["this_session"] = Value::Bool(true);
            }
            Some(item)
        })
        .collect();

    let mut notes: Vec<_> = board
        .notes
        .values()
        .filter(|n| item_rect(board, &n.id).is_some())
        .collect();
    oldest_first(&mut notes, |n| n.created_at);
    let note_items: Vec<Value> = notes
        .into_iter()
        .map(|n| {
            let mut item = json!({ "id": n.id, "text": clip(&n.text, NOTE_PREVIEW), "color": n.color });
            put_box(&mut item, sized(n.x, n.y, note_size(n)));
            item
        })
        .collect();

    let mut views: Vec<_> = board.views.values().collect();
    oldest_first(&mut views, |v| v.created_at);
    let view_items: Vec<Value> = views
        .into_iter()
        .map(|v| {
            let mut item = json!({ "id": v.id, "title": v.title });
            put_box(&mut item, sized(v.x, v.y, view_size(v)));
            item
        })
        .collect();

    let shape_items: Vec<Value> = board
        .shapes
        .values()
        .map(|s| {
            let mut item = json!({ "id": s.id, "kind": s.kind });
            if !s.text.is_empty() {
                item["text"] = json!(s.text);
            }
            item["color"] = json!(s.color);
            put_box(&mut item, s.rect());
            if s.by.as_deref() == Some(self_id) {
                item["yours"] = Value::Bool(true);
            }
            item
        })
        .collect();

    let links: Vec<&BoardLink> = board.links.values().filter(|l| drawn(board, l)).collect();
    let link_items: Vec<Value> = links
        .iter()
        .map(|l| {
            let mut item = json!({
                "id": l.id,
                "from": describe_end(board, &l.from, &by_id),
                "to": describe_end(board, &l.to, &by_id),
            });
            if !l.label.is_empty() {
                item["label"] = json!(l.label);
            }
            item["color"] = json!(l.color);
            if l.by.as_deref() == Some(self_id) {
                item["yours"] = Value::Bool(true);
            }
            item
        })
        .collect();
    let mine: Vec<String> = links
        .iter()
        .filter(|l| l.touches(self_id))
        .map(|l| link_line(board, l, self_id, &by_id))
        .collect();

    let mut listed = json!({
        "linked_to_this_session": mine,
        "cards": card_items,
        "notes": note_items,
        "views": view_items,
        "shapes": shape_items,
        "links": link_items,
    });
    // FRAME-040, GROUP-040: frames and groups, when there are any.
    let frame_items: Vec<Value> = nodus_board::frames::board_frames(board)
        .into_iter()
        .map(|(s, r)| {
            let mut item = json!({ "id": s.id, "name": s.name });
            put_box(&mut item, r);
            item
        })
        .collect();
    if !frame_items.is_empty() {
        listed["frames"] = Value::Array(frame_items);
    }
    let groups: Vec<Value> = board.groups.values().map(|g| json!(g.members)).collect();
    if !groups.is_empty() {
        listed["groups"] = Value::Array(groups);
    }
    listed
}

// ---------------------------------------------------------------------------
// draw_on_board

/// A shape asked for, checked, before it has a place on the board.
struct Planned {
    /// Its key, or its place in the list when it has none.
    name: String,
    kind: ShapeKind,
    text: String,
    color: InkColor,
    /// Where it goes in the drawing, when Claude said.
    at: Option<Point>,
    size: Size,
}

fn plan_shape(item: &Value, index: usize, names: &mut Vec<String>) -> Result<Planned, String> {
    let at = format!("shapes[{index}]");
    object(item, &at)?;
    let kind = match string(item, "kind", &at)?.and_then(ShapeKind::parse) {
        Some(ShapeKind::Image) => {
            return Err(format!(
                "{at}: pictures can’t be drawn with the board tools; kind must be {KINDS}"
            ));
        }
        Some(kind) => kind,
        None => return Err(format!("{at}: kind must be {KINDS}")),
    };
    let name = match string(item, "key", &at)?.map(js::trim) {
        None => index.to_string(),
        Some("") => return Err(format!("{at}: key can’t be blank")),
        Some(SELF) => {
            return Err(format!(
                "{at}: “self” means this session’s card; give the shape another key"
            ));
        }
        Some(key) => key.to_owned(),
    };
    if names.contains(&name) {
        return Err(format!(
            "{at}: another shape goes by “{name}”; give each shape its own key"
        ));
    }
    names.push(name.clone());
    let text = shape_text(kind, string(item, "text", &at)?.unwrap_or_default(), &at)?;
    let color = color(item, &at)?.unwrap_or(InkColor::Graphite);
    let (x, y) = (number(item, "x", &at)?, number(item, "y", &at)?);
    let at_point = match (x, y) {
        (Some(x), Some(y)) => Some(Point { x, y }),
        (None, None) => None,
        _ => return Err(format!("{at}: give both x and y, or neither to put it in the row")),
    };
    let default = drawn_size(kind);
    let size = Size {
        w: side(number(item, "w", &at)?.unwrap_or(default.w)),
        h: side(number(item, "h", &at)?.unwrap_or(default.h)),
    };
    Ok(Planned {
        name,
        kind,
        text,
        color,
        at: at_point,
        size,
    })
}

/// Where each shape goes in the drawing: where Claude said, or else in a row
/// from left to right, lined up on their middles, under the ones it placed.
fn lay_out(shapes: &[Planned]) -> Vec<Rect> {
    let placed: Vec<Rect> = shapes
        .iter()
        .filter_map(|s| s.at.map(|p| sized(p.x, p.y, s.size)))
        .collect();
    let (mut x, row_y) = if placed.is_empty() {
        (0.0, 0.0)
    } else {
        let left = placed.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
        let bottom = placed.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
        (left, bottom + ROW_GAP)
    };
    let row_h = shapes
        .iter()
        .filter(|s| s.at.is_none())
        .map(|s| s.size.h)
        .fold(0.0, f64::max);
    shapes
        .iter()
        .map(|s| {
            if let Some(p) = s.at {
                return sized(p.x, p.y, s.size);
            }
            let rect = sized(x, row_y + js::round((row_h - s.size.h) / 2.0), s.size);
            x += s.size.w + ROW_GAP;
            rect
        })
        .collect()
}

/// The box around them all.
fn bounds(rects: &[Rect]) -> Rect {
    let left = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let top = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let right = rects.iter().map(|r| r.x + r.w).fold(f64::NEG_INFINITY, f64::max);
    let bottom = rects.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
    Rect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    }
}

fn at_point(args: &Value) -> Result<Option<Point>, String> {
    let bad = || "at must be {\"x\": …, \"y\": …}, in board coordinates as list_board gives them".to_owned();
    match args.get("at") {
        None | Some(Value::Null) => Ok(None),
        Some(at @ Value::Object(_)) => {
            let n = |key: &str| at.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
            match (n("x"), n("y")) {
                (Some(x), Some(y)) => Ok(Some(Point { x, y })),
                _ => Err(bad()),
            }
        }
        Some(_) => Err(bad()),
    }
}

/// Draws shapes and links for the session `by`, and returns their ids: the
/// shapes' by key (or by their place in the list), the links' in order, and
/// the box the drawing took on the board (TOOL-103 to TOOL-106).
pub(crate) fn draw(
    board: &mut BoardState,
    by: &str,
    starter: Option<&str>,
    args: &Value,
    now: f64,
) -> Result<Value, String> {
    let (shape_args, link_args) = (list(args, "shapes")?, list(args, "links")?);
    if shape_args.is_empty() && link_args.is_empty() {
        return Err("Nothing to draw: give shapes, links or both".into());
    }
    let at = at_point(args)?;
    let mut names = Vec::new();
    let planned = shape_args
        .iter()
        .enumerate()
        .map(|(i, item)| plan_shape(item, i, &mut names))
        .collect::<Result<Vec<_>, _>>()?;
    let count = board.shapes.values().count();
    if count + planned.len() > MAX_SHAPES {
        return Err(format!("The board keeps at most {MAX_SHAPES} shapes, and has {count}"));
    }
    let count = board.links.values().count();
    if count + link_args.len() > MAX_LINKS {
        return Err(format!("The board keeps at most {MAX_LINKS} links, and has {count}"));
    }

    // Where the drawing goes: at the point given, or beside the session's card.
    let rects = lay_out(&planned);
    let area = (!rects.is_empty()).then(|| bounds(&rects));
    let offset = match (at, area) {
        (Some(at), _) => at,
        (None, Some(area)) => {
            let spot = place_drawing(board, by, Size { w: area.w, h: area.h });
            Point {
                x: spot.x - area.x,
                y: spot.y - area.y,
            }
        }
        (None, None) => Point { x: 0.0, y: 0.0 },
    };
    let mut shapes = Vec::new();
    let mut ids: HashMap<String, (String, Rect)> = HashMap::new();
    let mut out_shapes = Map::new();
    for (i, (plan, rect)) in planned.iter().zip(&rects).enumerate() {
        let shape = BoardShape {
            id: new_id(),
            kind: plan.kind,
            x: js::round(rect.x + offset.x),
            y: js::round(rect.y + offset.y),
            w: rect.w,
            h: rect.h,
            color: plan.color,
            text: plan.text.clone(),
            image: None,
            by: Some(by.to_owned()),
            created_at: now,
            extra: Map::new(),
        };
        if clean_shape(&shape).is_none() {
            return Err(format!(
                "shapes[{i}] can’t go there; give it a place nearer the rest of the board"
            ));
        }
        ids.insert(plan.name.clone(), (shape.id.clone(), shape.rect()));
        out_shapes.insert(plan.name.clone(), json!(shape.id));
        shapes.push(shape);
    }

    let mut links = Vec::new();
    for (i, item) in link_args.iter().enumerate() {
        let at = format!("links[{i}]");
        object(item, &at)?;
        let end = |key: &str| -> Result<(String, Rect), String> {
            let Some(name) = string(item, key, &at)?.filter(|s| !s.is_empty()) else {
                return Err(format!(
                    "{at}: {key} must be a key from shapes, the id of something on the board, or “self”"
                ));
            };
            match ids.get(name) {
                Some(new) => Ok(new.clone()),
                None => board_end(board, by, name, &at),
            }
        };
        let (from, to) = (end("from")?, end("to")?);
        if from.0 == to.0 {
            return Err(format!("{at}: a link needs two different ends"));
        }
        let link = BoardLink {
            id: new_id(),
            from: end_on(&from.0, from.1),
            to: end_on(&to.0, to.1),
            color: color(item, &at)?.unwrap_or(InkColor::Graphite),
            label: string(item, "label", &at)?.unwrap_or_default().to_owned(),
            by: Some(by.to_owned()),
            created_at: now,
            extra: Map::new(),
        };
        if clean_link(&link).is_none() {
            return Err(format!("{at} can’t be drawn"));
        }
        links.push(link);
    }

    save_shapes(board, &shapes, now);
    save_links(board, &links, now);
    // SRV-183: on a server, they're the session's starter's.
    if starter.is_some() {
        for shape in &shapes {
            if let Some(saved) = board.shapes.get_mut(&shape.id) {
                saved.set_created_by(starter);
            }
        }
        for link in &links {
            if let Some(saved) = board.links.get_mut(&link.id) {
                saved.set_created_by(starter);
            }
        }
    }
    let mut out = json!({
        "shapes": out_shapes,
        "links": links.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
    });
    if let Some(area) = area {
        let mut placed = json!({});
        put_box(
            &mut placed,
            Rect {
                x: area.x + offset.x,
                y: area.y + offset.y,
                ..area
            },
        );
        out["area"] = placed;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// update_drawing

/// What an item in update_drawing changes.
enum Change {
    Shape(BoardShape),
    /// A card, note or view, moved.
    Move {
        what: Moved,
        id: String,
        x: Option<f64>,
        y: Option<f64>,
    },
}

#[derive(Clone, Copy)]
enum Moved {
    Card,
    Note,
    View,
}

/// What a card, note or view can't be given in update_drawing, which only moves it.
fn only_moves(item: &Value, what: &str, id: &str, elsewhere: &str, at: &str) -> Result<(), String> {
    match ["w", "h", "text", "color"]
        .iter()
        .find(|k| item.get(**k).is_some_and(|v| !v.is_null()))
    {
        Some(key) => Err(format!(
            "{at}: “{id}” is {what}, which update_drawing only moves, so it takes x and y but not {key}{elsewhere}"
        )),
        None => Ok(()),
    }
}

fn plan_item(board: &BoardState, item: &Value, index: usize, pending: &[BoardShape]) -> Result<Change, String> {
    let at = format!("items[{index}]");
    object(item, &at)?;
    let Some(id) = string(item, "id", &at)?.filter(|s| !s.is_empty()) else {
        return Err(format!("{at}: id is required"));
    };
    let (x, y) = (number(item, "x", &at)?, number(item, "y", &at)?);
    // The latest change to it in this call, so a shape given twice gets both.
    let shape = pending
        .iter()
        .rev()
        .find(|s| s.id == id)
        .or_else(|| board.shapes.get(id));
    if let Some(stored) = shape {
        let (w, h) = (number(item, "w", &at)?, number(item, "h", &at)?);
        let text = string(item, "text", &at)?;
        let color = color(item, &at)?;
        if [x, y, w, h].iter().all(Option::is_none) && text.is_none() && color.is_none() {
            return Err(format!("{at}: nothing to change; give x, y, w, h, text or color"));
        }
        let mut shape = stored.clone();
        shape.x = x.map_or(shape.x, js::round);
        shape.y = y.map_or(shape.y, js::round);
        shape.w = w.map_or(shape.w, side);
        shape.h = h.map_or(shape.h, side);
        if let Some(text) = text {
            shape.text = shape_text(shape.kind, text, &at)?;
        }
        shape.color = color.unwrap_or(shape.color);
        if clean_shape(&shape).is_none() {
            return Err(format!("{at}: “{id}” can’t go there"));
        }
        return Ok(Change::Shape(shape));
    }
    if board.links.get(id).is_some() {
        return Err(format!("{at}: “{id}” is a link; change it under links"));
    }
    let what = if board.nodes.contains_key(id) {
        Some((Moved::Card, "a card", ""))
    } else if board.notes.contains_key(id) {
        Some((Moved::Note, "a note", "; update_note changes its text and colour"))
    } else if board.views.contains_key(id) {
        Some((Moved::View, "a view", "; update_view changes its size and title"))
    } else {
        None
    };
    let Some((moved, name, elsewhere)) = what else {
        return Err(format!(
            "{at}: there’s nothing with the id “{id}” on this board; list_board lists what’s there"
        ));
    };
    if item_rect(board, id).is_none() {
        return Err(format!(
            "{at}: “{id}” isn’t drawn on the board right now, so it can’t be moved"
        ));
    }
    only_moves(item, name, id, elsewhere, &at)?;
    if x.is_none() && y.is_none() {
        return Err(format!("{at}: nothing to change; give x, y or both"));
    }
    Ok(Change::Move {
        what: moved,
        id: id.to_owned(),
        x,
        y,
    })
}

fn plan_link(
    board: &BoardState,
    self_id: &str,
    item: &Value,
    index: usize,
    pending: &[BoardLink],
) -> Result<BoardLink, String> {
    let at = format!("links[{index}]");
    object(item, &at)?;
    let Some(id) = string(item, "id", &at)?.filter(|s| !s.is_empty()) else {
        return Err(format!("{at}: id is required"));
    };
    let stored = pending
        .iter()
        .rev()
        .find(|l| l.id == id)
        .or_else(|| board.links.get(id));
    let Some(stored) = stored else {
        return Err(format!(
            "{at}: there’s no link with the id “{id}” on this board; list_board lists them"
        ));
    };
    let mut link = stored.clone();
    let label = string(item, "label", &at)?;
    let color = color(item, &at)?;
    let end = |key: &str| -> Result<Option<LinkEnd>, String> {
        match string(item, key, &at)? {
            None => Ok(None),
            Some("") => Err(format!("{at}: {key} can’t be blank")),
            Some(name) => board_end(board, self_id, name, &at).map(|(id, rect)| Some(end_on(&id, rect))),
        }
    };
    let (from, to) = (end("from")?, end("to")?);
    if label.is_none() && color.is_none() && from.is_none() && to.is_none() {
        return Err(format!("{at}: nothing to change; give label, color, from or to"));
    }
    if let Some(label) = label {
        link.label = js::slice_utf16(js::trim(label), 0, LINK_MAX_LABEL);
    }
    link.color = color.unwrap_or(link.color);
    link.from = from.unwrap_or(link.from);
    link.to = to.unwrap_or(link.to);
    if link.from.node.is_some() && link.from.node == link.to.node {
        return Err(format!("{at}: a link needs two different ends"));
    }
    if clean_link(&link).is_none() {
        return Err(format!("{at}: “{id}” can’t be drawn that way"));
    }
    Ok(link)
}

/// Changes shapes and links, and moves cards, notes and views, as a card
/// dragged on the board moves: nothing else about them changes (TOOL-107,
/// TOOL-108). Returns the ids it changed, in order.
pub(crate) fn update(board: &mut BoardState, self_id: &str, args: &Value, now: f64) -> Result<Value, String> {
    let (item_args, link_args) = (list(args, "items")?, list(args, "links")?);
    if item_args.is_empty() && link_args.is_empty() {
        return Err("Nothing to change: give items, links or both".into());
    }
    let mut shapes: Vec<BoardShape> = Vec::new();
    let mut moves = Vec::new();
    let mut updated: Vec<String> = Vec::new();
    for (i, item) in item_args.iter().enumerate() {
        match plan_item(board, item, i, &shapes)? {
            Change::Shape(shape) => {
                updated.push(shape.id.clone());
                shapes.push(shape);
            }
            Change::Move { what, id, x, y } => {
                updated.push(id.clone());
                moves.push((what, id, x, y));
            }
        }
    }
    let mut links: Vec<BoardLink> = Vec::new();
    for (i, item) in link_args.iter().enumerate() {
        let link = plan_link(board, self_id, item, i, &links)?;
        updated.push(link.id.clone());
        links.push(link);
    }

    // Everything checked: now change the board.
    save_shapes(board, &shapes, now);
    for (what, id, x, y) in moves {
        match what {
            // As board:save-nodes moves a card (SESS-092).
            Moved::Card => {
                if let Some(node) = board.nodes.get_mut(&id) {
                    node.x = x.unwrap_or(node.x);
                    node.y = y.unwrap_or(node.y);
                }
            }
            // As board:save-note and views:save do (SESS-103, VIEW-116).
            Moved::Note => {
                if let Some(note) = board.notes.get_mut(&id) {
                    note.x = x.map_or(note.x, js::round);
                    note.y = y.map_or(note.y, js::round);
                }
            }
            Moved::View => {
                if let Some(view) = board.views.get_mut(&id) {
                    view.x = x.map_or(view.x, js::round);
                    view.y = y.map_or(view.y, js::round);
                }
            }
        }
    }
    save_links(board, &links, now);
    let mut seen = Vec::new();
    updated.retain(|id| {
        let first = !seen.contains(id);
        seen.push(id.clone());
        first
    });
    Ok(json!({ "updated": updated }))
}

// ---------------------------------------------------------------------------
// erase_from_board

/// Takes shapes and links off the board, a shape's links with it; cards,
/// notes and views stay (TOOL-109). Returns what went and what was skipped.
pub(crate) fn erase(board: &mut BoardState, args: &Value) -> Result<Value, String> {
    let ids: Vec<&str> = match args.get("ids") {
        Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_str).collect(),
        _ => return Err("ids must be a list of the ids of shapes and links, from list_board".into()),
    };
    if ids.is_empty() {
        return Err("Nothing to erase: give the ids of shapes and links, from list_board".into());
    }
    let mut shapes: Vec<String> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    for id in ids {
        if shapes.iter().chain(&links).any(|done| done == id) {
            continue;
        }
        if board.shapes.get(id).is_some() {
            shapes.push(id.to_owned());
            continue;
        }
        if board.links.get(id).is_some() {
            links.push(id.to_owned());
            continue;
        }
        let reason = if board.nodes.contains_key(id) {
            "it is a card; only shapes and links can be erased"
        } else if board.notes.contains_key(id) {
            "it is a note; only shapes and links can be erased"
        } else if board.views.contains_key(id) {
            "it is a view; only shapes and links can be erased"
        } else {
            "not on the board"
        };
        skipped.push(json!({ "id": id, "reason": reason }));
    }
    // A shape's links go with it.
    let held: Vec<String> = board
        .links
        .values()
        .filter(|l| !links.contains(&l.id) && shapes.iter().any(|s| l.touches(s)))
        .map(|l| l.id.clone())
        .collect();
    links.extend(held);
    remove_links(board, &links);
    remove_shapes(board, &shapes);
    let erased: Vec<&String> = shapes.iter().chain(&links).collect();
    Ok(json!({ "erased": erased, "skipped": skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(name: &str, at: Option<(f64, f64)>, w: f64, h: f64) -> Planned {
        Planned {
            name: name.into(),
            kind: ShapeKind::Rect,
            text: String::new(),
            color: InkColor::Graphite,
            at: at.map(|(x, y)| Point { x, y }),
            size: Size { w, h },
        }
    }

    #[test]
    fn lays_shapes_without_a_place_out_in_a_row_under_the_placed_ones() {
        let rects = lay_out(&[
            planned("a", None, 180.0, 90.0),
            planned("b", Some((40.0, 0.0)), 100.0, 100.0),
            planned("c", None, 64.0, 64.0),
        ]);
        let boxes: Vec<(f64, f64)> = rects.iter().map(|r| (r.x, r.y)).collect();
        // The row starts at the placed shape's left, ROW_GAP under it, and
        // lines its shapes up on their middles.
        assert_eq!(boxes, [(40.0, 164.0), (40.0, 0.0), (40.0 + 180.0 + ROW_GAP, 177.0)]);
        let alone = lay_out(&[planned("a", None, 180.0, 90.0), planned("b", None, 160.0, 110.0)]);
        assert_eq!(
            (alone[0].x, alone[0].y, alone[1].x, alone[1].y),
            (0.0, 10.0, 244.0, 0.0)
        );
    }

    #[test]
    fn clips_long_text_with_an_ellipsis() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdefghij klm", 11), "abcdefghij…");
        assert_eq!(clip(&"😀".repeat(5), 4), "😀😀…");
    }
}
