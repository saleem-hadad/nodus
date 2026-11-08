//! Sticky notes that sessions write on the board with their board tools
//! (nodus-tools). A session's notes go up beside its card, one under another.
//! On another project's board, where its card isn't, they go beside the
//! rightmost card, and say where they came from.
//!
//! Ports `src/main/notes.ts`. The functions work on a board in hand; whoever
//! calls them holds the store and saves it (`Store::update` does).

use nodus_domain::js;
use nodus_domain::{BoardNode, BoardNote, BoardView, FolderColor, Rect};
use nodus_store::BoardState;
use serde_json::{Value, json};

use crate::cards::set;
use crate::geometry::{GAP, NOTE_H, NOTE_W, Point, Size, card_size, dock_rect, find_free_spot, note_size, view_size};
/// How much text a note keeps (NOTE-001), and the bounds for a note the user
/// resizes (NOTE-015, `board:save-note`).
pub use crate::handlers::{NOTE_MAX_CHARS, clamp_note_size};

/// A note Claude writes grows to fit its text up to this, and scrolls past it.
const FIT_MAX_W: f64 = 360.0;
const FIT_MAX_H: f64 = 480.0;
/// Note text is 15px with a 1.4 line height, inside 16px by 18px of padding (board.css).
const LINE_H: f64 = 21.0;
const CHAR_W: f64 = 7.4;
const PAD_X: f64 = 36.0;
const PAD_Y: f64 = 32.0;
/// The line naming the session that wrote it.
const FROM_H: f64 = 22.0;

/// What a note tool may change about a note (boardTools.ts's `NotePatch`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotePatch {
    pub text: Option<String>,
    pub color: Option<FolderColor>,
}

/// Who wrote a note on another project's board: their project, and their title then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwayAuthor {
    pub project: String,
    pub title: String,
}

/// Why a note tool failed, in the words Claude is shown.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteError {
    #[error("A note needs some text")]
    NeedsText,
    #[error("There’s no note with that id on this board")]
    NotFound,
}

/// `Math.round(Math.min(max, Math.max(min, n)))`.
fn clamp(n: f64, min: f64, max: f64) -> f64 {
    js::round(n.clamp(min, max))
}

/// A size that shows `text` whole, as far as a note grows by itself. A short
/// note stays square (NOTE-002).
pub fn fit_note(text: &str) -> Size {
    let lines: Vec<usize> = text.split('\n').map(js::utf16_len).collect();
    let longest = lines.iter().copied().max().unwrap_or(0) as f64;
    let w = clamp(longest * CHAR_W + PAD_X, NOTE_W, FIT_MAX_W);
    // Lines wrap at words, so they fit fewer characters than the width holds.
    let per_line = (((w - PAD_X) / CHAR_W) * 0.9).floor().max(1.0);
    let rows: f64 = lines.iter().map(|&len| (len as f64 / per_line).ceil().max(1.0)).sum();
    Size {
        w,
        h: clamp(rows * LINE_H + PAD_Y + FROM_H, NOTE_H, FIT_MAX_H),
    }
}

/// Cards that are drawn on the board: not archived, not a run, not a view's brain.
pub(crate) fn active_cards(board: &BoardState) -> impl Iterator<Item = &BoardNode> {
    board
        .nodes
        .values()
        .filter(|n| !n.archived && !set(n.run_of.as_deref()) && !set(n.view_of.as_deref()))
}

/// The rightmost active card; on a tie, the earlier one (NOTE-006 (4)).
pub fn rightmost_card(board: &BoardState) -> Option<&BoardNode> {
    active_cards(board).fold(None, |a: Option<&BoardNode>, b| match a {
        Some(a) if a.x >= b.x => Some(a),
        _ => Some(b),
    })
}

/// What a note or view put up by a session keeps clear of: active cards,
/// notes not stuck to a card, and views, but the one called `except`
/// (NOTE-005; views.ts's `obstacles`).
pub fn obstacles(board: &BoardState, except: Option<&str>) -> Vec<Rect> {
    let rect = |x: f64, y: f64, size: Size| Rect {
        x,
        y,
        w: size.w,
        h: size.h,
    };
    active_cards(board)
        .map(|n| rect(n.x, n.y, card_size(Some(n))))
        .chain(
            board
                .notes
                .values()
                .filter(|n| !set(n.session_id.as_deref()))
                .map(|n| rect(n.x, n.y, note_size(n))),
        )
        .chain(
            board
                .views
                .values()
                .filter(|v| Some(v.id.as_str()) != except)
                .map(|v| rect(v.x, v.y, view_size(v))),
        )
        .collect()
}

/// Where a session's next note goes: under the last one it wrote, or beside
/// its card, in the nearest free spot. A view's brain writes beside its view.
/// A run isn't on the board, nor is an archived card or a session from
/// another project, so their notes go beside the rightmost card instead
/// (NOTE-006 to NOTE-008).
fn place_note(board: &BoardState, from: &str, size: Size) -> Point {
    let obstacles = obstacles(board, None);
    let dock = dock_rect(board.archive);

    // The newest; on a tie, the first written.
    let last = board
        .notes
        .values()
        .filter(|n| !set(n.session_id.as_deref()) && n.from.as_deref() == Some(from))
        .fold(None, |a: Option<&BoardNote>, b| match a {
            Some(a) if a.created_at >= b.created_at => Some(a),
            _ => Some(b),
        });
    let view: Option<&BoardView> = board
        .nodes
        .get(from)
        .and_then(|n| n.view_of.as_deref())
        .filter(|id| !id.is_empty())
        .and_then(|id| board.views.get(id));
    let card = active_cards(board)
        .find(|n| n.session_id == from)
        .or_else(|| rightmost_card(board));
    let preferred = if let Some(last) = last {
        Point {
            x: last.x,
            y: last.y + note_size(last).h + GAP,
        }
    } else if let Some(view) = view {
        Point {
            x: view.x + view_size(view).w + GAP,
            y: view.y,
        }
    } else if let Some(card) = card {
        Point {
            x: card.x + card_size(Some(card)).w + GAP,
            y: card.y,
        }
    } else {
        Point {
            x: dock.x + dock.w + 80.0,
            y: dock.y,
        }
    };
    // NOTE-007
    find_free_spot(preferred, &obstacles, Some(dock), size)
}

/// `text?.trim().slice(0, NOTE_MAX_CHARS) ?? ''` (NOTE-001).
fn note_text(text: Option<&str>) -> String {
    text.map(|t| js::slice_utf16(js::trim(t), 0, NOTE_MAX_CHARS))
        .unwrap_or_default()
}

/// Writes a note from session `from` on `board`, beside its card or under its
/// last note, sized to fit, and returns it (NOTE-001 to NOTE-010). `away` says
/// where it came from when it was written from another project's board.
pub fn create_note(
    board: &mut BoardState,
    from: &str,
    patch: &NotePatch,
    away: Option<&AwayAuthor>,
    now: f64,
) -> Result<BoardNote, NoteError> {
    let body = note_text(patch.text.as_deref());
    if body.is_empty() {
        return Err(NoteError::NeedsText);
    }
    let size = fit_note(&body);
    let Point { x, y } = place_note(board, from, size);
    let note = BoardNote {
        id: uuid::Uuid::new_v4().to_string(),
        text: body,
        // NOTE-004
        color: patch.color.unwrap_or(FolderColor::Yellow),
        x,
        y,
        // A short note stays square, and says nothing of its size (NOTE-003).
        w: (size.w != NOTE_W).then_some(size.w),
        h: (size.h != NOTE_H).then_some(size.h),
        session_id: None,
        from: Some(from.to_owned()),
        from_project: away.map(|a| a.project.clone()),
        from_title: away.map(|a| a.title.clone()),
        created_at: now,
        extra: Default::default(),
    };
    board.notes.insert(note.id.clone(), note.clone());
    Ok(note)
}

/// Changes a note's text or colour. It keeps its size, which may be the
/// user's (NOTE-011 to NOTE-014). Ids are looked up as the board's own keys
/// only, so `__proto__` finds nothing.
pub fn update_note(board: &mut BoardState, note_id: &str, patch: &NotePatch) -> Result<BoardNote, NoteError> {
    let note = board.notes.get_mut(note_id).ok_or(NoteError::NotFound)?;
    if let Some(text) = &patch.text {
        let body = note_text(Some(text));
        if body.is_empty() {
            return Err(NoteError::NeedsText);
        }
        note.text = body;
    }
    if let Some(color) = patch.color {
        note.color = color;
    }
    Ok(note.clone())
}

/// Every note on the board, stickers included, oldest first, saying which
/// ones `self_id` wrote (NOTE-016): `[{id, text, color, writtenByYou?}]`.
pub fn list_notes(board: &BoardState, self_id: &str) -> Value {
    let mut notes: Vec<&BoardNote> = board.notes.values().collect();
    // Stable, as Array.prototype.sort is.
    notes.sort_by(|a, b| {
        a.created_at
            .partial_cmp(&b.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Value::Array(
        notes
            .into_iter()
            .map(|n| {
                let mut item = json!({ "id": n.id, "text": n.text, "color": n.color });
                if n.from.as_deref() == Some(self_id) {
                    item["writtenByYou"] = Value::Bool(true);
                }
                item
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_a_note_to_its_text() {
        let fit = |text: &str| {
            let size = fit_note(text);
            (size.w, size.h)
        };
        assert_eq!(fit("Short"), (216.0, 216.0));
        assert_eq!(fit(&"word ".repeat(400)), (360.0, 480.0));
        // 30 characters: 30 × 7.4 + 36 = 258.
        assert_eq!(fit(&"x".repeat(30)).0, 258.0);
        // Lines count in UTF-16 units, as JavaScript measures them.
        assert_eq!(fit(&"😀".repeat(20)), fit(&"xx".repeat(20)));
        // Ten lines: 10 × 21 + 54 = 264.
        assert_eq!(fit(&["a"; 10].join("\n")), (216.0, 264.0));
    }

    #[test]
    fn clamps_a_users_note_size() {
        assert_eq!(clamp_note_size(50.0), 120.0);
        assert_eq!(clamp_note_size(5000.0), 1200.0);
        assert_eq!(clamp_note_size(300.5), 301.0);
    }
}
