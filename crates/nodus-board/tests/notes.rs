//! notes.test.ts, ported: each test gets its own board, with one card at (2000, 0).

use nodus_board::notes::{AwayAuthor, NOTE_MAX_CHARS, NoteError, NotePatch, create_note, list_notes, update_note};
use nodus_domain::{BoardNode, BoardNote, FolderColor, Rect, js};
use nodus_store::BoardState;
use serde_json::json;

use nodus_board::geometry::{CARD_H, CARD_W, GAP, NOTE_H, NOTE_W, dock_rect};

fn card(id: &str, x: f64) -> BoardNode {
    serde_json::from_value(json!({ "sessionId": id, "x": x, "y": 0, "archived": false, "createdAt": 0 })).unwrap()
}

/// A new board with card s1 at (2000, 0).
fn board() -> BoardState {
    let mut board = BoardState::default();
    board.nodes.insert("s1".into(), card("s1", 2000.0));
    board
}

fn text(t: &str) -> NotePatch {
    NotePatch {
        text: Some(t.into()),
        color: None,
    }
}

fn write(board: &mut BoardState, from: &str, patch: &NotePatch) -> BoardNote {
    create_note(board, from, patch, None, 1.0).unwrap()
}

#[test]
fn create_note_puts_a_sessions_first_note_beside_its_card() {
    let mut board = board();
    let note = write(&mut board, "s1", &text("  Findings  "));
    assert_eq!(note.text, "Findings");
    assert_eq!(note.color, FolderColor::Yellow);
    assert_eq!(note.from.as_deref(), Some("s1"));
    assert_eq!((note.x, note.y), (2000.0 + CARD_W + GAP, 0.0));
    assert_eq!(board.notes[&note.id], note);
    // A uuid v4.
    assert!(uuid::Uuid::parse_str(&note.id).is_ok_and(|id| id.get_version_num() == 4));
}

#[test]
fn create_note_puts_the_next_one_under_the_last() {
    let mut board = board();
    let first = write(&mut board, "s1", &text("One"));
    let second = create_note(
        &mut board,
        "s1",
        &NotePatch {
            text: Some("Two".into()),
            color: Some(FolderColor::Green),
        },
        None,
        2.0,
    )
    .unwrap();
    assert_eq!((second.x, second.y), (first.x, first.y + NOTE_H + GAP));
    assert_eq!(second.color, FolderColor::Green);
}

#[test]
fn create_note_keeps_clear_of_the_boards_cards() {
    let mut board = board();
    board.nodes.insert("s2".into(), card("s2", 2000.0 + CARD_W + GAP));
    let note = write(&mut board, "s1", &text("Elsewhere"));
    let c = Rect {
        x: 2000.0 + CARD_W + GAP,
        y: 0.0,
        w: CARD_W,
        h: CARD_H,
    };
    let overlaps = note.x < c.x + c.w && note.x + NOTE_W > c.x && note.y < c.y + c.h && note.y + NOTE_H > c.y;
    assert!(!overlaps);
}

#[test]
fn create_note_stays_square_for_short_text_and_grows_to_a_limit_for_long_text() {
    let mut board = board();
    let short = write(&mut board, "s1", &text("Short"));
    assert_eq!((short.w, short.h), (None, None));
    let json = serde_json::to_value(&short).unwrap();
    assert!(json.get("w").is_none() && json.get("h").is_none());
    let long = write(&mut board, "s1", &text(&"word ".repeat(400)));
    assert_eq!((long.w, long.h), (Some(360.0), Some(480.0)));
}

#[test]
fn create_note_needs_some_text_and_keeps_no_more_than_the_limit() {
    let mut board = board();
    assert_eq!(
        create_note(&mut board, "s1", &text("   "), None, 1.0),
        Err(NoteError::NeedsText)
    );
    assert_eq!(
        create_note(&mut board, "s1", &NotePatch::default(), None, 1.0)
            .unwrap_err()
            .to_string(),
        "A note needs some text"
    );
    let long = write(&mut board, "s1", &text(&"x".repeat(NOTE_MAX_CHARS + 50)));
    assert_eq!(js::utf16_len(&long.text), NOTE_MAX_CHARS);
}

#[test]
fn a_note_from_another_projects_board_goes_beside_the_rightmost_card_under_the_last_one_and_says_where_it_came_from() {
    let mut board = board();
    board.nodes.insert("s2".into(), card("s2", 0.0));
    let away = AwayAuthor {
        project: "elsewhere".into(),
        title: "Release checklist".into(),
    };
    let first = create_note(&mut board, "visitor", &text("From afar"), Some(&away), 1.0).unwrap();
    assert_eq!(first.from.as_deref(), Some("visitor"));
    assert_eq!(first.from_project.as_deref(), Some("elsewhere"));
    assert_eq!(first.from_title.as_deref(), Some("Release checklist"));
    assert_eq!((first.x, first.y), (2000.0 + CARD_W + GAP, 0.0));
    let second = create_note(&mut board, "visitor", &text("And again"), Some(&away), 2.0).unwrap();
    assert_eq!((second.x, second.y), (first.x, first.y + NOTE_H + GAP));
}

#[test]
fn a_note_from_another_projects_board_leaves_the_fields_out_for_a_note_from_this_board() {
    let mut board = board();
    let note = write(&mut board, "s1", &text("Home"));
    assert_eq!((note.from_project, note.from_title), (None, None));
}

#[test]
fn update_note_changes_the_text_and_colour_and_keeps_the_size() {
    let mut board = board();
    let id = write(&mut board, "s1", &text("Before")).id;
    board.notes[&id].w = Some(500.0);
    update_note(
        &mut board,
        &id,
        &NotePatch {
            text: Some(" After ".into()),
            color: Some(FolderColor::Red),
        },
    )
    .unwrap();
    let note = &board.notes[&id];
    assert_eq!(
        (note.text.as_str(), note.color, note.w),
        ("After", FolderColor::Red, Some(500.0))
    );
}

#[test]
fn update_note_leaves_out_what_the_patch_doesnt_say() {
    let mut board = board();
    let id = create_note(
        &mut board,
        "s1",
        &NotePatch {
            text: Some("Keep".into()),
            color: Some(FolderColor::Blue),
        },
        None,
        1.0,
    )
    .unwrap()
    .id;
    update_note(&mut board, &id, &NotePatch::default()).unwrap();
    assert_eq!(
        (board.notes[&id].text.as_str(), board.notes[&id].color),
        ("Keep", FolderColor::Blue)
    );
    // A blank text fails before the colour is applied (NOTE-012).
    let blank = NotePatch {
        text: Some(" ".into()),
        color: Some(FolderColor::Red),
    };
    assert_eq!(update_note(&mut board, &id, &blank), Err(NoteError::NeedsText));
    assert_eq!(board.notes[&id].color, FolderColor::Blue);
}

#[test]
fn update_note_only_finds_the_boards_own_notes() {
    let mut board = board();
    assert_eq!(
        update_note(&mut board, "missing", &text("x")).unwrap_err().to_string(),
        "There’s no note with that id on this board"
    );
    for id in ["__proto__", "constructor", "toString", "hasOwnProperty"] {
        assert_eq!(update_note(&mut board, id, &text("polluted")), Err(NoteError::NotFound));
    }
    assert!(board.notes.is_empty());
}

#[test]
fn list_notes_lists_notes_oldest_first_and_says_which_the_session_wrote() {
    let mut board = board();
    let note = |id: &str, text: &str, color: &str, from: Option<&str>, at: f64| -> BoardNote {
        let mut n = json!({ "id": id, "text": text, "color": color, "x": 0, "y": 0, "createdAt": at });
        if let Some(from) = from {
            n["from"] = json!(from);
        }
        serde_json::from_value(n).unwrap()
    };
    board
        .notes
        .insert("user".into(), note("user", "Mine", "yellow", None, 2.0));
    board
        .notes
        .insert("claude".into(), note("claude", "Claude’s", "blue", Some("s1"), 1.0));
    assert_eq!(
        list_notes(&board, "s1"),
        json!([
            { "id": "claude", "text": "Claude’s", "color": "blue", "writtenByYou": true },
            { "id": "user", "text": "Mine", "color": "yellow" }
        ])
    );
}

#[test]
fn a_view_brain_writes_beside_its_view() {
    let mut board = board();
    let view = json!({
        "id": "v", "title": "V", "description": "V", "x": 5000, "y": 400,
        "version": 1, "createdAt": 0, "updatedAt": 0
    });
    board.views.insert("v".into(), serde_json::from_value(view).unwrap());
    let mut brain = card("brain", 0.0);
    brain.view_of = Some("v".into());
    board.nodes.insert("brain".into(), brain);
    let note = write(&mut board, "brain", &text("From the brain"));
    assert_eq!((note.x, note.y), (5000.0 + 480.0 + GAP, 400.0));
}

#[test]
fn a_board_with_no_cards_puts_notes_right_of_the_dock() {
    let mut board = BoardState::default();
    let note = write(&mut board, "run", &text("Alone"));
    let dock = dock_rect(board.archive);
    assert_eq!((note.x, note.y), (dock.x + dock.w + 80.0, dock.y));
}
