//! The archive: where archived and newly found sessions are laid out, in a
//! grid of card slots that grows to fit them (board.ts).

use std::collections::HashSet;

use nodus_domain::{BoardNode, Rect, SessionKind, SessionMeta};
use nodus_store::BoardState;
use nodus_transcripts::DiscoveredSession;
use serde_json::Map;

use crate::Effects;

// SESS-082: the archive's own spacing, not the shared geometry's gap of 32.
pub const CARD_W: f64 = 288.0;
pub const CARD_H: f64 = 216.0;
const GAP: f64 = 24.0;
const PAD_X: f64 = 40.0;
const PAD_TOP: f64 = 64.0;
const PAD_BOTTOM: f64 = 40.0;

/// How many card columns the archive has room for (SESS-083).
pub fn columns_for(archive: Rect) -> f64 {
    ((archive.w - 2.0 * PAD_X + GAP) / (CARD_W + GAP)).floor().max(1.0)
}

/// Where slot `index` is (SESS-083).
pub fn slot_position(archive: Rect, index: f64, cols: f64) -> (f64, f64) {
    (
        archive.x + PAD_X + (index % cols) * (CARD_W + GAP),
        archive.y + PAD_TOP + (index / cols).floor() * (CARD_H + GAP),
    )
}

/// The slot an archived card sits in, if it's in one (SESS-084).
pub fn slot_index_of(archive: Rect, node: &BoardNode, cols: f64) -> Option<f64> {
    let col = nodus_domain::js::round((node.x - archive.x - PAD_X) / (CARD_W + GAP));
    let row = nodus_domain::js::round((node.y - archive.y - PAD_TOP) / (CARD_H + GAP));
    if col < 0.0 || col >= cols || row < 0.0 {
        return None;
    }
    Some(row * cols + col)
}

/// Grows the archive (never shrinks it) so every archived card fits inside (SESS-085).
pub fn grow_archive(board: &mut BoardState) {
    let a = board.archive;
    let mut right = a.x + a.w;
    let mut bottom = a.y + a.h.max(PAD_TOP + CARD_H + PAD_BOTTOM);
    for node in board.nodes.values().filter(|n| n.archived) {
        right = right.max(node.x + CARD_W + PAD_X);
        bottom = bottom.max(node.y + CARD_H + PAD_BOTTOM);
    }
    board.archive = Rect {
        w: right - a.x,
        h: bottom - a.y,
        ..a
    };
}

/// A board node with only the fields every node has.
pub(crate) fn bare_node(session_id: &str, x: f64, y: f64, archived: bool, created_at: f64) -> BoardNode {
    BoardNode {
        session_id: session_id.to_owned(),
        kind: None,
        x,
        y,
        archived,
        title: None,
        parents: None,
        created_by_app: None,
        run_of: None,
        expanded: None,
        view_of: None,
        auto_mode: None,
        from_view: None,
        created_at,
        w: None,
        h: None,
        archived_at: None,
        section_id: None,
        extra: Map::new(),
    }
}

/// Places newly discovered sessions in free grid slots inside the archive, in
/// the order given (SESS-074).
pub fn import_sessions(board: &mut BoardState, sessions: &[&DiscoveredSession]) {
    if sessions.is_empty() {
        return;
    }
    let cols = columns_for(board.archive);
    let mut taken = HashSet::new();
    for node in board.nodes.values().filter(|n| n.archived) {
        if let Some(slot) = slot_index_of(board.archive, node, cols) {
            taken.insert(slot as u64);
        }
    }
    let mut slot: u64 = 0;
    for found in sessions {
        while taken.contains(&slot) {
            slot += 1;
        }
        taken.insert(slot);
        let (x, y) = slot_position(board.archive, slot as f64, cols);
        let id = &found.meta.session_id;
        board
            .nodes
            .insert(id.clone(), bare_node(id, x, y, true, found.meta.created_at));
    }
}

/// Archives a card the way the board does: stops its process, takes it out of
/// its section and puts it on top of the archive stack. For changes made
/// outside the window, such as an action run's board tools. Doesn't save;
/// callers do. Whether it archived it (SESS-086).
pub fn archive_node(board: &mut BoardState, session_id: &str, now: f64, effects: &mut Effects) -> bool {
    let archive = board.archive;
    let Some(node) = board.nodes.get_mut(session_id) else {
        return false;
    };
    if node.archived {
        return false;
    }
    effects.stop.push(session_id.to_owned());
    let (x, y) = slot_position(archive, 0.0, columns_for(archive));
    node.x = x;
    node.y = y;
    node.archived = true;
    node.archived_at = Some(now);
    node.section_id = None;
    node.expanded = None;
    true
}

/// Nothing was said in the session: no prompt typed (slash commands aside),
/// and no reply (SESS-052). Shared with the window, which takes such cards
/// off the board straight away.
pub fn is_empty_session(meta: Option<&SessionMeta>) -> bool {
    let set = |s: Option<&String>| s.is_some_and(|s| !s.is_empty());
    !meta.is_some_and(|m| set(m.first_prompt.as_ref()) || set(m.last_assistant_text.as_ref()))
}

/// Whether a card is a Claude session nothing was said in, which isn't worth
/// archiving: archiving it removes it instead. `meta` is its transcript's
/// (SESS-087).
pub fn is_empty(node: &BoardNode, meta: Option<&SessionMeta>) -> bool {
    node.kind != Some(SessionKind::Terminal) && is_empty_session(meta)
}

/// Takes a card off the board: a note stuck to it falls off, and its process
/// stops, its status forgotten once it's gone. It isn't hidden (SESS-088).
pub fn drop_node(board: &mut BoardState, session_id: &str, effects: &mut Effects) {
    let Some(node) = board.nodes.get(session_id).cloned() else {
        return;
    };
    unstick_notes(board, &node);
    board.nodes.shift_remove(session_id);
    effects.stop_and_forget.push(session_id.to_owned());
}

/// A note stuck on a card that leaves the board falls off it: onto the board
/// where the card was, or, from an archived card, back where the note was
/// (SESS-089).
pub fn unstick_notes(board: &mut BoardState, node: &BoardNode) {
    for note in board.notes.values_mut() {
        if note.session_id.as_deref() != Some(node.session_id.as_str()) {
            continue;
        }
        note.session_id = None;
        if !node.archived {
            note.x = node.x;
            note.y = node.y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardNote, FolderColor};

    fn archive() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: nodus_store::DEFAULT_ARCHIVE_W,
            h: 320.0,
        }
    }

    fn found(id: &str, created_at: f64) -> DiscoveredSession {
        DiscoveredSession {
            meta: SessionMeta {
                session_id: id.into(),
                title: "T".into(),
                first_prompt: None,
                last_prompt: None,
                last_assistant_text: None,
                git_branch: None,
                message_count: 0,
                created_at,
                updated_at: created_at,
                transcript_path: String::new(),
            },
            eligible: true,
            has_content: true,
            last_prompt_at: None,
        }
    }

    fn note(id: &str, session_id: Option<&str>) -> BoardNote {
        BoardNote {
            id: id.into(),
            text: String::new(),
            color: FolderColor::Yellow,
            x: 5.0,
            y: 6.0,
            w: None,
            h: None,
            session_id: session_id.map(Into::into),
            from: None,
            from_project: None,
            from_title: None,
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    #[test]
    fn lays_out_slots_in_rows_of_columns() {
        let a = archive();
        assert_eq!(columns_for(a), 6.0);
        assert_eq!(columns_for(Rect { w: 10.0, ..a }), 1.0);
        assert_eq!(slot_position(a, 0.0, 6.0), (40.0, 64.0));
        assert_eq!(slot_position(a, 7.0, 6.0), (40.0 + 312.0, 64.0 + 240.0));
        let at = |x, y| bare_node("s", x, y, true, 0.0);
        assert_eq!(slot_index_of(a, &at(352.0, 304.0), 6.0), Some(7.0));
        // Near a slot rounds to it; off the grid has none.
        assert_eq!(slot_index_of(a, &at(352.0 + 150.0, 304.0 - 100.0), 6.0), Some(7.0));
        assert_eq!(slot_index_of(a, &at(-200.0, 64.0), 6.0), None);
        assert_eq!(slot_index_of(a, &at(40.0 + 6.0 * 312.0, 64.0), 6.0), None);
        assert_eq!(slot_index_of(a, &at(40.0, -200.0), 6.0), None);
        // Math.round takes -0.5 up to 0.
        assert_eq!(slot_index_of(a, &at(40.0 - 156.0, 64.0), 6.0), Some(0.0));
    }

    #[test]
    fn imports_into_free_slots_and_grows_to_fit() {
        let mut board = BoardState::default();
        // Slots 0 and 2 are taken.
        board.nodes.insert("a".into(), bare_node("a", 40.0, 64.0, true, 0.0));
        board
            .nodes
            .insert("b".into(), bare_node("b", 40.0 + 624.0, 64.0, true, 0.0));
        // Not archived, so no slot.
        board
            .nodes
            .insert("c".into(), bare_node("c", 40.0 + 312.0, 64.0, false, 0.0));
        let fresh: Vec<DiscoveredSession> = (0..6).map(|i| found(&format!("n{i}"), f64::from(i))).collect();
        import_sessions(&mut board, &fresh.iter().collect::<Vec<_>>());
        let at = |id: &str| (board.nodes[id].x, board.nodes[id].y);
        assert_eq!(at("n0"), (40.0 + 312.0, 64.0));
        assert_eq!(at("n1"), (40.0 + 3.0 * 312.0, 64.0));
        assert_eq!(at("n3"), (40.0 + 5.0 * 312.0, 64.0));
        assert_eq!(at("n4"), (40.0, 64.0 + 240.0));
        assert_eq!(at("n5"), (40.0 + 312.0, 64.0 + 240.0));
        assert_eq!(
            nodus_domain::json::to_string(&board.nodes["n5"]).unwrap(),
            r#"{"sessionId":"n5","x":352,"y":304,"archived":true,"createdAt":5}"#
        );
        grow_archive(&mut board);
        assert_eq!(board.archive.w, nodus_store::DEFAULT_ARCHIVE_W);
        assert_eq!(board.archive.h, 304.0 + 216.0 + 40.0);
        // Never shrinks.
        board.nodes.clear();
        grow_archive(&mut board);
        assert_eq!(board.archive.h, 304.0 + 216.0 + 40.0);
    }

    #[test]
    fn archives_a_card_onto_slot_zero() {
        let mut board = BoardState::default();
        let mut node = bare_node("a", 900.0, 900.0, false, 0.0);
        node.section_id = Some("s".into());
        node.expanded = Some(true);
        board.nodes.insert("a".into(), node);
        let mut effects = Effects::default();
        assert!(archive_node(&mut board, "a", 1234.0, &mut effects));
        let node = &board.nodes["a"];
        assert_eq!(
            (node.x, node.y, node.archived, node.archived_at),
            (40.0, 64.0, true, Some(1234.0))
        );
        assert_eq!((node.section_id.as_deref(), node.expanded), (None, None));
        assert_eq!(effects.stop, ["a"]);
        // An archived card is left alone.
        assert!(!archive_node(&mut board, "a", 9.0, &mut effects));
        assert_eq!(effects.stop.len(), 1);
    }

    #[test]
    fn drops_a_card_and_its_notes_fall_where_it_was() {
        let mut board = BoardState::default();
        board.nodes.insert("a".into(), bare_node("a", 100.0, 200.0, false, 0.0));
        board.notes.insert("n1".into(), note("n1", Some("a")));
        board.notes.insert("n2".into(), note("n2", Some("other")));
        let mut effects = Effects::default();
        drop_node(&mut board, "a", &mut effects);
        assert!(board.nodes.is_empty());
        assert_eq!(
            (
                board.notes["n1"].session_id.as_deref(),
                board.notes["n1"].x,
                board.notes["n1"].y
            ),
            (None, 100.0, 200.0)
        );
        assert_eq!(board.notes["n2"].session_id.as_deref(), Some("other"));
        assert_eq!(effects.stop_and_forget, ["a"]);
        assert!(board.hidden.is_empty());
        // From an archived card a note stays where it is.
        board.notes.insert("n3".into(), note("n3", Some("b")));
        unstick_notes(&mut board, &bare_node("b", 1.0, 1.0, true, 0.0));
        assert_eq!(
            (board.notes["n3"].x, board.notes["n3"].session_id.as_deref()),
            (5.0, None)
        );
    }

    #[test]
    fn tells_empty_sessions() {
        let mut meta = found("a", 0.0).meta;
        assert!(is_empty_session(None));
        assert!(is_empty_session(Some(&meta)));
        meta.first_prompt = Some(String::new());
        assert!(is_empty_session(Some(&meta)));
        meta.last_assistant_text = Some("Done".into());
        assert!(!is_empty_session(Some(&meta)));
        let mut terminal = bare_node("t", 0.0, 0.0, false, 0.0);
        terminal.kind = Some(SessionKind::Terminal);
        assert!(!is_empty(&terminal, None));
        assert!(is_empty(&bare_node("c", 0.0, 0.0, false, 0.0), None));
    }
}
