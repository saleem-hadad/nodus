//! `look_at_board` (TOOL-111): the board as a picture, drawn from what's
//! stored on it rather than taken from the window, so a session sees any
//! part of its board at any size, whether the user has it open or not, on
//! this computer or on a server. It shows what list_board can't put in
//! words: the pictures, the ink and how things sit together.
//!
//! Each thing in the picture carries a short tag (`c1` a card, `n2` a note,
//! `p3` a picture…) that the key maps to its id, so what Claude sees joins
//! up with the ids the other board tools take. The board is written out as
//! SVG and drawn with resvg; text uses the system's fonts.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

use nodus_board::cards::SessionListItem;
use nodus_board::frames::board_frames;
use nodus_board::geometry::{note_size, view_size};
use nodus_board::ink_draw::{LABEL_FONT, LABEL_LINE, PEN_WIDTH, Seg, fallback_width, stroke_box, stroke_path};
use nodus_board::pictures::{picture_kind, picture_size};
use nodus_board::shapes::item_rect;
use nodus_domain::{FolderColor, InkColor, InkKind, InkStroke, Rect, ShapeKind, js};
use nodus_store::BoardState;
use resvg::{tiny_skia, usvg};
use serde_json::{Map, Value, json};

/// The longest side of the picture, in pixels: as much as Claude takes in
/// without scaling it down.
const MAX_PX: f64 = 1568.0;
/// The most a small area is blown up.
const MAX_SCALE: f64 = 2.0;
/// Room around the thing `around` names, in board units, by default.
const MARGIN: f64 = 240.0;
/// Room around the whole board.
const PAD: f64 = 80.0;
/// A tag's text, in pixels on the picture.
const TAG_PX: f64 = 14.0;
/// Text never comes out smaller than this, in pixels.
const MIN_TEXT_PX: f64 = 9.0;
/// The most ink strokes tagged in one picture; the newest are.
const MAX_INK_TAGS: usize = 60;
/// A picture bigger than this either way is shown as a box: decoding it
/// would take more memory than it's worth.
const MAX_PICTURE_SIDE: u32 = 8000;

const FONT: &str = "'Helvetica Neue', Helvetica, 'Segoe UI', Arial, 'DejaVu Sans', sans-serif";
const EMOJI_FONT: &str = "'Apple Color Emoji', 'Segoe UI Emoji', 'Noto Color Emoji', sans-serif";

// The light board's colours (theme tokens, spec 08).
const BOARD: &str = "#f3f3f1";
const CARD: &str = "#ffffff";
const EDGE: &str = "#d4d4d8";
const TEXT: &str = "#1d1d1f";
const FAINT: &str = "#6e6e73";
const ACCENT: &str = "#007aff";

/// What `look_at_board` answers: the picture, and the key to its tags.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub png: Vec<u8>,
    pub key: Value,
}

/// A picture's bytes by its file name, from wherever the board keeps them.
pub(crate) type Pictures<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

fn ink_color(color: InkColor) -> &'static str {
    match color {
        InkColor::Graphite => "#1d1d1f",
        InkColor::Red => "#ff3b30",
        InkColor::Orange => "#ff9500",
        InkColor::Green => "#28cd41",
        InkColor::Blue => "#007aff",
        InkColor::Purple => "#af52de",
    }
}

/// A note's paper: its colour, light.
fn note_paper(color: FolderColor) -> &'static str {
    match color {
        FolderColor::Blue => "#d6ecfc",
        FolderColor::Purple => "#ebdffb",
        FolderColor::Pink => "#fcdcea",
        FolderColor::Red => "#fcdcda",
        FolderColor::Orange => "#feead2",
        FolderColor::Yellow => "#fff3c4",
        FolderColor::Green => "#dcf3df",
        FolderColor::Graphite => "#e8e8ea",
    }
}

fn status_color(status: &str) -> &'static str {
    match status {
        "working" => "#ff9500",
        "waiting for the user" => "#ff3b30",
        "done" => "#007aff",
        _ => "#8e8e93",
    }
}

/// The system's fonts, read once.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

// ---------------------------------------------------------------------------
// What's asked

fn rect_arg(value: &Value) -> Option<Rect> {
    let n = |key: &str| value.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
    let rect = Rect {
        x: n("x")?,
        y: n("y")?,
        w: n("w")?,
        h: n("h")?,
    };
    (rect.w > 0.0 && rect.h > 0.0).then_some(rect)
}

fn grow(rect: Rect, by: f64) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        w: rect.w + by * 2.0,
        h: rect.h + by * 2.0,
    }
}

fn union(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().reduce(|a, b| {
        let (x, y) = (a.x.min(b.x), a.y.min(b.y));
        Rect {
            x,
            y,
            w: (a.x + a.w).max(b.x + b.w) - x,
            h: (a.y + a.h).max(b.y + b.h) - y,
        }
    })
}

fn meets(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn ink_rect(stroke: &InkStroke) -> Rect {
    let b = stroke_box(stroke, &fallback_width);
    Rect {
        x: b.x,
        y: b.y,
        w: b.w,
        h: b.h,
    }
}

/// The box of the thing an id names: a card, note, view or shape drawn on
/// the board, an ink stroke, or a frame.
pub(crate) fn box_of(board: &BoardState, id: &str) -> Option<Rect> {
    if let Some(rect) = item_rect(board, id) {
        return Some(rect);
    }
    if let Some(stroke) = board.ink.as_deref().unwrap_or_default().iter().find(|s| s.id == id) {
        return Some(ink_rect(stroke));
    }
    board_frames(board)
        .into_iter()
        .find(|(s, _)| s.id == id)
        .map(|(_, r)| r)
}

/// Everything there is to see on the board, as boxes.
fn everything(board: &BoardState, cards: &[SessionListItem]) -> Vec<Rect> {
    let mut rects: Vec<Rect> = cards.iter().filter_map(|c| item_rect(board, &c.id)).collect();
    rects.extend(board.notes.keys().filter_map(|id| item_rect(board, id)));
    rects.extend(board.views.keys().filter_map(|id| item_rect(board, id)));
    rects.extend(board.shapes.values().map(nodus_domain::BoardShape::rect));
    rects.extend(board.ink.as_deref().unwrap_or_default().iter().map(ink_rect));
    rects.extend(board_frames(board).into_iter().map(|(_, r)| r));
    rects
}

/// The area to show: the one asked for, around something, or the whole board.
fn area(board: &BoardState, self_id: &str, cards: &[SessionListItem], args: &Value) -> Result<Rect, String> {
    let given = |key: &str| args.get(key).filter(|v| !v.is_null());
    let margin = match given("margin") {
        None => MARGIN,
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or("margin must be a number of board units, 0 or more")?
            .min(4000.0),
    };
    match (given("area"), given("around")) {
        (Some(_), Some(_)) => Err("give area or around, not both".into()),
        (Some(area), None) => rect_arg(area).ok_or_else(|| {
            "area must be {\"x\": …, \"y\": …, \"w\": …, \"h\": …} in board coordinates as list_board gives them, w and h above 0".into()
        }),
        (None, Some(around)) => {
            let ids: Vec<&str> = match around {
                Value::String(id) => vec![id.as_str()],
                Value::Array(ids) => ids.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            if ids.is_empty() {
                return Err("around must be an id from list_board, a list of them, or \"self\"".into());
            }
            let mut rects = Vec::new();
            for id in ids {
                if id == "self" {
                    rects.push(item_rect(board, self_id).ok_or("this session has no card on the board to look around")?);
                    continue;
                }
                rects.push(box_of(board, id).ok_or_else(|| {
                    format!("“{id}” isn’t drawn on this board; list_board lists what’s there")
                })?);
            }
            Ok(grow(union(rects).expect("at least one box"), margin))
        }
        (None, None) => Ok(union(everything(board, cards)).map_or(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1200.0,
                h: 800.0,
            },
            |all| grow(all, PAD),
        )),
    }
}

// ---------------------------------------------------------------------------
// Writing the SVG

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no place for most control characters.
            c if (c as u32) < 0x20 && c != '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn n(v: f64) -> String {
    nodus_domain::json::number_to_string((v * 100.0).round() / 100.0)
}

/// Lines of `text` that fit `width` at `size`, by a rough measure of the
/// font (`width_of` ems a character), broken at spaces where it can be.
fn wrap(text: &str, width: f64, size: f64, width_of: f64) -> Vec<String> {
    let per_line = ((width / (size * width_of)).floor() as usize).max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let mut word: Vec<char> = word.chars().collect();
            loop {
                let room = per_line.saturating_sub(line.chars().count() + usize::from(!line.is_empty()));
                if word.len() <= room {
                    if !line.is_empty() {
                        line.push(' ');
                    }
                    line.extend(word.iter());
                    break;
                }
                if line.is_empty() {
                    // A word longer than the line breaks where it must.
                    lines.push(word.drain(..per_line).collect());
                    continue;
                }
                lines.push(std::mem::take(&mut line));
            }
        }
        lines.push(line);
    }
    lines
}

/// The first `max` of `lines`, the last of them ending in … when some are left out.
fn clamp_lines(mut lines: Vec<String>, max: usize) -> Vec<String> {
    if lines.len() > max {
        lines.truncate(max.max(1));
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines
}

#[derive(Clone, Copy)]
enum Align {
    Start,
    Middle,
}

struct Text<'a> {
    text: &'a str,
    size: f64,
    weight: u16,
    fill: &'a str,
    align: Align,
}

struct Svg {
    out: String,
    /// Pixels per board unit.
    scale: f64,
}

impl Svg {
    /// A text size in board units that comes out at least [`MIN_TEXT_PX`].
    fn size(&self, units: f64) -> f64 {
        units.max(MIN_TEXT_PX / self.scale)
    }

    /// A line width in board units that comes out at least a pixel.
    fn line(&self, units: f64) -> f64 {
        units.max(1.0 / self.scale)
    }

    /// Words wrapped into the box, as many lines as fit; the height they took.
    fn words(&mut self, x: f64, y: f64, w: f64, h: f64, t: &Text<'_>) -> f64 {
        let line_h = t.size * 1.3;
        if w <= 0.0 || h < line_h * 0.8 || js::trim(t.text).is_empty() {
            return 0.0;
        }
        let width_of = if t.weight >= 600 { 0.58 } else { 0.53 };
        let max = ((h / line_h).floor() as usize).max(1);
        let lines = clamp_lines(wrap(js::trim(t.text), w, t.size, width_of), max);
        let (anchor, tx) = match t.align {
            Align::Start => ("start", x),
            Align::Middle => ("middle", x + w / 2.0),
        };
        let top = match t.align {
            Align::Start => y,
            Align::Middle => y + (h - line_h * lines.len() as f64) / 2.0,
        };
        let _ = write!(
            self.out,
            r#"<text font-family="{FONT}" font-size="{}" font-weight="{}" fill="{}" text-anchor="{anchor}">"#,
            n(t.size),
            t.weight,
            t.fill
        );
        for (i, line) in lines.iter().enumerate() {
            let baseline = top + line_h * i as f64 + t.size;
            let _ = write!(
                self.out,
                r#"<tspan x="{}" y="{}">{}</tspan>"#,
                n(tx),
                n(baseline),
                esc(line)
            );
        }
        self.out.push_str("</text>");
        line_h * lines.len() as f64
    }

    fn rect(&mut self, r: Rect, radius: f64, fill: &str, stroke: Option<(&str, f64)>, dashed: bool) {
        let _ = write!(
            self.out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{fill}""#,
            n(r.x),
            n(r.y),
            n(r.w),
            n(r.h),
            n(radius)
        );
        if let Some((color, width)) = stroke {
            let _ = write!(self.out, r#" stroke="{color}" stroke-width="{}""#, n(width));
            if dashed {
                let _ = write!(self.out, r#" stroke-dasharray="{} {}""#, n(width * 4.0), n(width * 3.0));
            }
        }
        self.out.push_str("/>");
    }
}

fn path_string(segs: &[Seg]) -> String {
    let mut d = String::new();
    for seg in segs {
        let _ = match *seg {
            Seg::Move([x, y]) => write!(d, "M{} {}", n(x), n(y)),
            Seg::Line([x, y]) => write!(d, "L{} {}", n(x), n(y)),
            Seg::Quad([cx, cy], [x, y]) => write!(d, "Q{} {} {} {}", n(cx), n(cy), n(x), n(y)),
        };
    }
    d
}

/// Where the line from `r`'s middle toward `to` leaves `r`.
pub(crate) fn edge_toward(r: Rect, to: (f64, f64)) -> (f64, f64) {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (dx, dy) = (to.0 - cx, to.1 - cy);
    if dx == 0.0 && dy == 0.0 {
        return (cx, cy);
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
    (cx + dx * t, cy + dy * t)
}

// ---------------------------------------------------------------------------
// The picture

/// Where a tag goes from the point it's given: just above it, starting
/// there (a box's top left); to its left, level with it (where ink starts);
/// or just below it, centred (a link's middle).
#[derive(Clone, Copy)]
enum Place {
    Above,
    Left,
    Below,
}

/// What the tags stand for.
struct Tags {
    key: Map<String, Value>,
    counts: HashMap<char, usize>,
    /// Where each tag goes: its text, its point and place, whether it's the caller's.
    marks: Vec<(String, (f64, f64), Place, bool)>,
}

impl Tags {
    fn add(&mut self, letter: char, id: &str, kind: &str, name: &str, at: (f64, f64), mine: bool) -> String {
        self.add_at(letter, id, kind, name, at, Place::Above, mine)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_at(
        &mut self,
        letter: char,
        id: &str,
        kind: &str,
        name: &str,
        at: (f64, f64),
        place: Place,
        mine: bool,
    ) -> String {
        let count = self.counts.entry(letter).or_default();
        *count += 1;
        let tag = format!("{letter}{count}");
        let mut item = json!({ "id": id, "kind": kind });
        if !name.is_empty() {
            item["name"] = json!(clip(name, 60));
        }
        if mine {
            item["this_session"] = Value::Bool(true);
        }
        self.key.insert(tag.clone(), item);
        self.marks.push((tag.clone(), at, place, mine));
        tag
    }
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if js::utf16_len(&flat) <= max {
        return flat;
    }
    format!("{}…", js::trim_end(&js::slice_utf16(&flat, 0, max)))
}

/// The board, or the part of it `args` asks for, as a PNG, and the key to
/// the tags on it (TOOL-111). `cards` are the board's active cards as
/// list_board lists them; `pictures` reads a picture's file by its name.
pub(crate) fn look(
    board: &BoardState,
    self_id: &str,
    cards: &[SessionListItem],
    args: &Value,
    pictures: Pictures<'_>,
) -> Result<Look, String> {
    let area = area(board, self_id, cards, args)?;
    let scale = (MAX_PX / area.w.max(area.h)).min(MAX_SCALE);
    let (px_w, px_h) = (
        (area.w * scale).round().clamp(1.0, MAX_PX * 2.0) as u32,
        (area.h * scale).round().clamp(1.0, MAX_PX * 2.0) as u32,
    );
    let mut svg = Svg {
        out: String::new(),
        scale,
    };
    let mut tags = Tags {
        key: Map::new(),
        counts: HashMap::new(),
        marks: Vec::new(),
    };
    let _ = write!(
        svg.out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{px_w}" height="{px_h}" viewBox="{} {} {} {}">"#,
        n(area.x),
        n(area.y),
        n(area.w),
        n(area.h)
    );
    svg.rect(area, 0.0, BOARD, None, false);
    let corner = |r: Rect| (r.x, r.y);
    let seen = |r: Rect| meets(r, area);

    // Frames, under everything.
    for (section, r) in board_frames(board) {
        if !seen(r) {
            continue;
        }
        let width = svg.line(1.5);
        svg.rect(r, 12.0, "#00000005", Some(("#a1a1a6", width)), true);
        let size = svg.size(16.0);
        svg.words(
            r.x + 12.0,
            r.y + 8.0,
            r.w - 24.0,
            size * 1.4,
            &Text {
                text: &section.name,
                size,
                weight: 600,
                fill: FAINT,
                align: Align::Start,
            },
        );
        tags.add('f', &section.id, "frame", &section.name, corner(r), false);
    }

    // Shapes but stickers, in board order, then views and cards.
    let mut picture_files: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    for shape in board.shapes.values().filter(|s| s.kind != ShapeKind::Sticker) {
        let r = shape.rect();
        if !seen(r) {
            continue;
        }
        draw_shape(&mut svg, shape, pictures, &mut picture_files);
        let (letter, kind) = if shape.kind == ShapeKind::Image {
            ('p', "picture")
        } else {
            ('s', "shape")
        };
        tags.add(letter, &shape.id, kind, &shape.text, corner(r), false);
    }
    let mut views: Vec<_> = board.views.values().collect();
    views.sort_by(|a, b| {
        a.created_at
            .partial_cmp(&b.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for view in views {
        let size = view_size(view);
        let r = Rect {
            x: view.x,
            y: view.y,
            w: size.w,
            h: size.h,
        };
        if !seen(r) {
            continue;
        }
        draw_view(&mut svg, r, &view.title, &view.description);
        tags.add('v', &view.id, "view", &view.title, corner(r), false);
    }
    // This session's card last, as an outline over the rest.
    let mut own = None;
    for card in cards {
        let Some(r) = item_rect(board, &card.id) else { continue };
        if !seen(r) {
            continue;
        }
        if card.id == self_id {
            own = Some((card, r));
            continue;
        }
        draw_card(&mut svg, r, card);
        tags.add('c', &card.id, "card", &card.title, corner(r), false);
    }

    // Free notes and stickers, over the cards.
    let mut notes: Vec<_> = board.notes.values().collect();
    notes.sort_by(|a, b| {
        a.created_at
            .partial_cmp(&b.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for note in notes {
        let Some(r) = item_rect(board, &note.id) else { continue };
        if !seen(r) {
            continue;
        }
        let size = note_size(note);
        draw_note(
            &mut svg,
            Rect {
                w: size.w,
                h: size.h,
                ..r
            },
            &note.text,
            note.color,
        );
        tags.add(
            'n',
            &note.id,
            "note",
            note.text.lines().next().unwrap_or_default(),
            corner(r),
            false,
        );
    }
    for shape in board.shapes.values().filter(|s| s.kind == ShapeKind::Sticker) {
        let r = shape.rect();
        if !seen(r) {
            continue;
        }
        draw_shape(&mut svg, shape, pictures, &mut picture_files);
        tags.add('s', &shape.id, "sticker", &shape.text, corner(r), false);
    }

    // Links, then ink, over the things.
    for link in board.links.values() {
        let end = |e: &nodus_domain::LinkEnd| match e.node.as_deref() {
            Some(id) => item_rect(board, id).map(Some),
            None => Some(None),
        };
        let (Some(from), Some(to)) = (end(&link.from), end(&link.to)) else {
            continue;
        };
        let point =
            |r: Option<Rect>, e: &nodus_domain::LinkEnd| r.map_or((e.x, e.y), |r| (r.x + r.w / 2.0, r.y + r.h / 2.0));
        let (fc, tc) = (point(from, &link.from), point(to, &link.to));
        let a = from.map_or(fc, |r| edge_toward(r, tc));
        let b = to.map_or(tc, |r| edge_toward(r, fc));
        let line = Rect {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (a.0 - b.0).abs().max(1.0),
            h: (a.1 - b.1).abs().max(1.0),
        };
        if !seen(line) {
            continue;
        }
        let color = ink_color(link.color);
        let width = svg.line(2.0);
        let head = 12.0_f64.max(8.0 / scale);
        let d = path_string(&nodus_board::ink_draw::arrow_path([a.0, a.1], [b.0, b.1], head));
        let _ = write!(
            svg.out,
            r#"<path d="{d}" fill="none" stroke="{color}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#,
            n(width)
        );
        let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let mut under = mid;
        if !link.label.is_empty() {
            let size = svg.size(13.0);
            let w = (js::utf16_len(&link.label).min(40) as f64 * size * 0.58 + 16.0).min(360.0);
            let h = size * 1.7;
            let pill = Rect {
                x: mid.0 - w / 2.0,
                y: mid.1 - h / 2.0,
                w,
                h,
            };
            svg.rect(pill, 6.0, CARD, Some((color, svg.line(1.0))), false);
            under.1 = pill.y + pill.h;
            svg.words(
                pill.x + 6.0,
                pill.y + (h - size * 1.3) / 2.0,
                w - 12.0,
                size * 1.3,
                &Text {
                    text: &link.label,
                    size,
                    weight: 600,
                    fill: TEXT,
                    align: Align::Middle,
                },
            );
        }
        tags.add_at('l', &link.id, "link", &link.label, under, Place::Below, false);
    }

    let ink = board.ink.as_deref().unwrap_or_default();
    let shown: Vec<&InkStroke> = ink.iter().filter(|s| seen(ink_rect(s))).collect();
    let tag_from = shown.len().saturating_sub(MAX_INK_TAGS);
    for (i, stroke) in shown.iter().enumerate() {
        draw_ink(&mut svg, stroke);
        if i >= tag_from {
            // Beside where it starts: an arrow's tail, a label's first line.
            let (x, mut y) = (stroke.points[0], stroke.points[1]);
            if stroke.kind == InkKind::Text {
                y += LABEL_LINE * stroke.size / 2.0;
            }
            let name = stroke.text.as_deref().unwrap_or(stroke.kind.as_str());
            tags.add_at('i', &stroke.id, "ink", name, (x, y), Place::Left, false);
        }
    }

    if let Some((card, r)) = own {
        let width = svg.line(2.0);
        svg.rect(r, 14.0, "none", Some((ACCENT, width)), true);
        let size = svg.size(14.0);
        svg.words(
            r.x + 14.0,
            r.y + 12.0,
            r.w - 28.0,
            size * 1.4,
            &Text {
                text: "this session (you)",
                size,
                weight: 600,
                fill: ACCENT,
                align: Align::Start,
            },
        );
        tags.add('c', &card.id, "card", &card.title, corner(r), true);
    }

    // The tags, on top of it all, a fixed size on the picture: each just
    // above its thing's top left, clear of its words, kept in the picture.
    let size = TAG_PX / scale;
    let marks = std::mem::take(&mut tags.marks);
    let mut placed: Vec<Rect> = Vec::new();
    for (tag, (x, y), place, mine) in marks {
        let w = (tag.chars().count() as f64 * 0.62 * TAG_PX + 8.0) / scale;
        let h = TAG_PX * 1.45 / scale;
        let gap = 3.0 / scale;
        let (left, top) = match place {
            Place::Above => (x, y - h - gap),
            Place::Left => (x - w - gap * 2.0, y - h / 2.0),
            Place::Below => (x - w / 2.0, y + gap),
        };
        let mut r = Rect {
            x: left.clamp(area.x, area.x + area.w - w),
            y: top.clamp(area.y, area.y + area.h - h),
            w,
            h,
        };
        // Clear of the tags already down: moved along to the right.
        for _ in 0..4 {
            match placed.iter().find(|p| meets(**p, r)) {
                Some(p) => r.x = p.x + p.w + gap,
                None => break,
            }
        }
        placed.push(r);
        svg.rect(r, h / 2.0, if mine { ACCENT } else { "#3a3a3c" }, None, false);
        let _ = write!(
            svg.out,
            r##"<text font-family="{FONT}" font-size="{}" font-weight="700" fill="#ffffff" text-anchor="middle" x="{}" y="{}">{}</text>"##,
            n(size),
            n(r.x + w / 2.0),
            n(r.y + h * 0.76),
            esc(&tag)
        );
    }
    svg.out.push_str("</svg>");

    let png = render(&svg.out, px_w, px_h, picture_files)?;
    let mut key = json!({ "area": {
        "x": js::round(area.x),
        "y": js::round(area.y),
        "w": js::round(area.w),
        "h": js::round(area.h),
    }});
    key["scale"] = json!((scale * 1000.0).round() / 1000.0);
    key["tags"] = Value::Object(tags.key);
    if tag_from > 0 {
        key["untagged_ink"] = json!(tag_from);
    }
    Ok(Look { png, key })
}

fn draw_card(svg: &mut Svg, r: Rect, card: &SessionListItem) {
    let width = svg.line(1.5);
    svg.rect(r, 14.0, CARD, Some((EDGE, width)), false);
    let pad = 18.0_f64.min(r.w / 10.0);
    let (x, w) = (r.x + pad, r.w - pad * 2.0);
    let mut y = r.y + pad;
    let title = svg.size(20.0);
    y += svg.words(
        x,
        y,
        w,
        (title * 1.3 * 3.0).min(r.h - pad * 2.0),
        &Text {
            text: &card.title,
            size: title,
            weight: 700,
            fill: TEXT,
            align: Align::Start,
        },
    );
    let small = svg.size(14.0);
    if y + small * 1.5 > r.y + r.h - pad {
        return;
    }
    let dot = small * 0.35;
    let color = status_color(&card.status);
    let _ = write!(
        svg.out,
        r#"<circle cx="{}" cy="{}" r="{}" fill="{color}"/>"#,
        n(x + dot),
        n(y + small * 0.95),
        n(dot)
    );
    y += svg.words(
        x + dot * 3.0,
        y + small * 0.3,
        w - dot * 3.0,
        small * 1.4,
        &Text {
            text: &card.status,
            size: small,
            weight: 500,
            fill: FAINT,
            align: Align::Start,
        },
    ) + small * 0.8;
    if let Some(reply) = card.last_reply.as_deref().filter(|r| !js::trim(r).is_empty()) {
        svg.words(
            x,
            y,
            w,
            r.y + r.h - pad - y,
            &Text {
                text: &reply.replace(['#', '*', '`'], ""),
                size: small,
                weight: 400,
                fill: FAINT,
                align: Align::Start,
            },
        );
    }
}

fn draw_view(svg: &mut Svg, r: Rect, title: &str, description: &str) {
    let width = svg.line(1.5);
    svg.rect(r, 10.0, CARD, Some((EDGE, width)), false);
    let size = svg.size(15.0);
    let head = (size * 2.2).min(r.h);
    svg.rect(Rect { h: head, ..r }, 10.0, "#f5f5f7", None, false);
    let title = if title.is_empty() { "A view" } else { title };
    svg.words(
        r.x + 12.0,
        r.y + (head - size * 1.3) / 2.0,
        r.w - 24.0,
        size * 1.3,
        &Text {
            text: title,
            size,
            weight: 600,
            fill: TEXT,
            align: Align::Start,
        },
    );
    let small = svg.size(13.0);
    let body = format!("A view: a live page. {description}");
    svg.words(
        r.x + 12.0,
        r.y + head + 10.0,
        r.w - 24.0,
        r.h - head - 20.0,
        &Text {
            text: &body,
            size: small,
            weight: 400,
            fill: FAINT,
            align: Align::Start,
        },
    );
}

fn draw_note(svg: &mut Svg, r: Rect, text: &str, color: FolderColor) {
    svg.rect(r, 4.0, note_paper(color), Some(("#00000014", svg.line(1.0))), false);
    let size = svg.size(15.0);
    svg.words(
        r.x + 12.0,
        r.y + 10.0,
        r.w - 24.0,
        r.h - 20.0,
        &Text {
            text,
            size,
            weight: 400,
            fill: TEXT,
            align: Align::Start,
        },
    );
}

fn draw_shape(
    svg: &mut Svg,
    shape: &nodus_domain::BoardShape,
    pictures: Pictures<'_>,
    files: &mut HashMap<String, Arc<Vec<u8>>>,
) {
    let r = shape.rect();
    let color = ink_color(shape.color);
    let width = svg.line(2.0);
    let fill = format!("{color}24");
    match shape.kind {
        ShapeKind::Rect => svg.rect(r, 10.0, &fill, Some((color, width)), false),
        ShapeKind::Ellipse => {
            let _ = write!(
                svg.out,
                r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="{fill}" stroke="{color}" stroke-width="{}"/>"#,
                n(r.x + r.w / 2.0),
                n(r.y + r.h / 2.0),
                n(r.w / 2.0 - width / 2.0),
                n(r.h / 2.0 - width / 2.0),
                n(width)
            );
        }
        ShapeKind::Diamond => {
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let _ = write!(
                svg.out,
                r#"<path d="M{} {}L{} {}L{} {}L{} {}Z" fill="{fill}" stroke="{color}" stroke-width="{}" stroke-linejoin="round"/>"#,
                n(cx),
                n(r.y),
                n(r.x + r.w),
                n(cy),
                n(cx),
                n(r.y + r.h),
                n(r.x),
                n(cy),
                n(width)
            );
        }
        ShapeKind::Text => {
            let size = svg.size(18.0);
            svg.words(
                r.x,
                r.y,
                r.w,
                r.h.max(size * 1.3),
                &Text {
                    text: &shape.text,
                    size,
                    weight: 600,
                    fill: color,
                    align: Align::Middle,
                },
            );
            return;
        }
        ShapeKind::Sticker => {
            let size = r.h * 0.78;
            let _ = write!(
                svg.out,
                r#"<text font-family="{EMOJI_FONT}" font-size="{}" text-anchor="middle" x="{}" y="{}">{}</text>"#,
                n(size),
                n(r.x + r.w / 2.0),
                n(r.y + r.h * 0.8),
                esc(&shape.text)
            );
            return;
        }
        ShapeKind::Image => {
            let name = shape.image.as_deref().unwrap_or_default();
            let bytes = files.get(name).cloned().or_else(|| {
                let bytes = pictures(name)?;
                let (w, h) = picture_size(&bytes)?;
                if w > MAX_PICTURE_SIDE || h > MAX_PICTURE_SIDE || picture_kind(&bytes).is_none() {
                    return None;
                }
                let bytes = Arc::new(bytes);
                files.insert(name.to_owned(), Arc::clone(&bytes));
                Some(bytes)
            });
            if bytes.is_some() {
                let _ = write!(
                    svg.out,
                    r#"<image href="pic:{}" x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="xMidYMid meet"/>"#,
                    esc(name),
                    n(r.x),
                    n(r.y),
                    n(r.w),
                    n(r.h)
                );
            } else {
                svg.rect(r, 8.0, "#e5e5ea", Some((EDGE, width)), false);
                let size = svg.size(14.0);
                svg.words(
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    &Text {
                        text: "picture",
                        size,
                        weight: 500,
                        fill: FAINT,
                        align: Align::Middle,
                    },
                );
            }
            return;
        }
    }
    let size = svg.size(15.0);
    let inset = if shape.kind == ShapeKind::Diamond {
        r.w / 4.0
    } else {
        10.0
    };
    svg.words(
        r.x + inset,
        r.y + 6.0,
        r.w - inset * 2.0,
        r.h - 12.0,
        &Text {
            text: &shape.text,
            size,
            weight: 500,
            fill: TEXT,
            align: Align::Middle,
        },
    );
}

fn draw_ink(svg: &mut Svg, stroke: &InkStroke) {
    let color = ink_color(stroke.color);
    if stroke.kind == InkKind::Text {
        let [x, y] = [
            stroke.points.first().copied().unwrap_or_default(),
            stroke.points.get(1).copied().unwrap_or_default(),
        ];
        let size = LABEL_FONT * stroke.size;
        let _ = write!(
            svg.out,
            r#"<text font-family="{FONT}" font-size="{}" font-weight="600" fill="{color}" x="{}" y="{}">{}</text>"#,
            n(size),
            n(x),
            n(y + (LABEL_LINE * stroke.size + size) / 2.0 - size * 0.12),
            esc(stroke.text.as_deref().unwrap_or_default())
        );
        return;
    }
    let width = svg.line(PEN_WIDTH * stroke.size);
    let d = path_string(&stroke_path(stroke));
    let _ = write!(
        svg.out,
        r#"<path d="{d}" fill="none" stroke="{color}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#,
        n(width)
    );
}

/// Draws the SVG into a PNG, its `pic:` pictures from `files`.
fn render(svg: &str, w: u32, h: u32, files: HashMap<String, Arc<Vec<u8>>>) -> Result<Vec<u8>, String> {
    let resolve = move |href: &str, _: &usvg::Options<'_>| {
        let bytes = files.get(href.strip_prefix("pic:")?)?.clone();
        match picture_kind(&bytes)? {
            "png" => Some(usvg::ImageKind::PNG(bytes)),
            "jpg" => Some(usvg::ImageKind::JPEG(bytes)),
            "gif" => Some(usvg::ImageKind::GIF(bytes)),
            "webp" => Some(usvg::ImageKind::WEBP(bytes)),
            _ => None,
        }
    };
    let options = usvg::Options {
        fontdb: fonts(),
        font_family: "Helvetica".into(),
        image_href_resolver: usvg::ImageHrefResolver {
            // Nothing but the board's own pictures: never a file the SVG names.
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(resolve),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|err| format!("The board couldn’t be drawn: {err}"))?;
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("The board couldn’t be drawn at that size")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|err| format!("The board couldn’t be drawn: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_into_lines_that_fit() {
        assert_eq!(wrap("one two three", 100.0, 10.0, 1.0), ["one two", "three"]);
        assert_eq!(wrap("abcdefghijkl", 50.0, 10.0, 1.0), ["abcde", "fghij", "kl"]);
        assert_eq!(wrap("a\nb", 100.0, 10.0, 1.0), ["a", "b"]);
        assert_eq!(clamp_lines(vec!["a".into(), "b".into(), "c".into()], 2), ["a", "b…"]);
    }

    #[test]
    fn escapes_what_xml_would_read_as_markup() {
        assert_eq!(
            esc("<a href=\"x\">&'</a>\u{1}"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&apos;&lt;/a&gt; "
        );
    }

    #[test]
    fn finds_where_a_line_leaves_a_box() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        assert_eq!(edge_toward(r, (300.0, 25.0)), (100.0, 25.0));
        assert_eq!(edge_toward(r, (50.0, -100.0)), (50.0, 0.0));
    }
}
