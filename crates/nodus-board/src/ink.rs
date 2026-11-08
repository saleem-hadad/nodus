//! Ink drawn on the board with the ink tools. The window sends the whole list
//! whenever it changes, so it's checked here before it's saved.
//!
//! Ports `src/main/ink.ts`.

use std::collections::HashSet;

use nodus_domain::js;
use nodus_domain::{InkColor, InkKind, InkStroke};
use nodus_store::BoardState;
use serde_json::{Map, Value};

/// How many strokes a board keeps; past this, the oldest go first (INK-002).
pub const INK_MAX_STROKES: usize = 2000;
/// How many points a pen stroke keeps (INK-007).
pub const INK_MAX_POINTS: usize = 2000;
/// How much text a label keeps (INK-008).
pub const INK_MAX_TEXT: usize = 500;

/// `Math.round(n * 10) / 10` (INK-009).
fn round(n: f64) -> f64 {
    js::round(n * 10.0) / 10.0
}

/// A finite size within 0.25–10, to two places; 1 otherwise (INK-010).
fn clamp_size(n: Option<&Value>) -> f64 {
    match n.and_then(Value::as_f64) {
        Some(n) if n.is_finite() => js::round(n.clamp(0.25, 10.0) * 100.0) / 100.0,
        _ => 1.0,
    }
}

/// How many numbers each kind of stroke has: a path, an arrow's two ends, a
/// label's corner (INK-007).
fn points_for(kind: InkKind, points: Vec<f64>) -> Option<Vec<f64>> {
    if points.len() < 2 || !points.len().is_multiple_of(2) {
        return None;
    }
    match kind {
        InkKind::Pen => Some(points.into_iter().take(INK_MAX_POINTS * 2).collect()),
        InkKind::Arrow => (points.len() == 4).then_some(points),
        InkKind::Text => (points.len() == 2).then_some(points),
    }
}

/// The strokes worth keeping, cleaned up. Anything malformed is dropped
/// (INK-001 to INK-011).
pub fn clean_ink(raw: &Value) -> Vec<InkStroke> {
    let Value::Array(items) = raw else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in &items[items.len().saturating_sub(INK_MAX_STROKES)..] {
        // An array is an object to JavaScript too, but has none of the fields.
        let Value::Object(item) = item else {
            continue;
        };
        if let Some(stroke) = clean_stroke(item, &seen) {
            seen.insert(stroke.id.clone());
            out.push(stroke);
        }
    }
    out
}

fn clean_stroke(item: &Map<String, Value>, seen: &HashSet<String>) -> Option<InkStroke> {
    let id = item.get("id")?.as_str()?;
    if id.is_empty() || js::utf16_len(id) > 64 || seen.contains(id) {
        return None;
    }
    let kind = InkKind::parse(item.get("kind")?.as_str()?)?;
    let color = InkColor::parse(item.get("color")?.as_str()?)?;
    // JSON numbers are always finite.
    let points = item
        .get("points")?
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<Vec<f64>>>()?;
    let kept = points_for(kind, points)?;
    let text = match (kind, item.get("text")) {
        (InkKind::Text, Some(Value::String(text))) => js::slice_utf16(js::trim(text), 0, INK_MAX_TEXT),
        _ => String::new(),
    };
    if kind == InkKind::Text && text.is_empty() {
        return None;
    }
    Some(InkStroke {
        id: id.to_owned(),
        kind,
        color,
        points: kept.into_iter().map(round).collect(),
        size: clamp_size(item.get("size")),
        text: (!text.is_empty()).then_some(text),
        extra: Map::new(),
    })
}

/// `board:save-ink`: the board's ink becomes what's worth keeping of `raw`
/// (INK-012). The caller saves.
pub fn save_ink(board: &mut BoardState, raw: &Value) {
    board.ink = Some(clean_ink(raw));
}
