//! ⌘C / ⌘V for notes (BOARD-178, added after 1.0.0): the selected free
//! notes go on the clipboard as their text, with the notes themselves
//! beside it, so a paste on any board can tell its own notes from other
//! text.

use gpui::{App, ClipboardEntry, ClipboardItem, Context};
use nodus_board::geometry::{Point, Size, note_size};
use nodus_domain::{BoardNote, js};
use serde::{Deserialize, Serialize};

use super::canvas::BoardCanvas;
use super::layout::{bounds_of, card_obstacles, find_free_spot, rect_at};

/// What ⌘C puts beside the notes' text.
#[derive(Serialize, Deserialize)]
struct CopiedNotes {
    nodus_notes: Vec<BoardNote>,
}

/// The notes a clipboard item carries, when ⌘C put it there.
fn copied_notes(item: &ClipboardItem) -> Option<Vec<BoardNote>> {
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::String(s) => s
            .metadata_json::<CopiedNotes>()
            .map(|c| c.nodus_notes)
            .filter(|notes| !notes.is_empty()),
        _ => None,
    })
}

/// The copies of `notes` as a group whose corner moves from `from` to `to`:
/// new ids, made now, by the user and on no card.
fn placed(notes: Vec<BoardNote>, from: Point, to: Point, now: f64) -> Vec<BoardNote> {
    notes
        .into_iter()
        .map(|note| BoardNote {
            id: uuid::Uuid::new_v4().to_string(),
            x: js::round(note.x - from.x + to.x),
            y: js::round(note.y - from.y + to.y),
            session_id: None,
            from: None,
            from_project: None,
            from_title: None,
            created_at: now,
            ..note
        })
        .collect()
}

impl BoardCanvas {
    /// ⌘C: the selected free notes, top to bottom, their texts a paragraph
    /// each. False when none is selected.
    pub(crate) fn copy_notes(&self, cx: &mut App) -> bool {
        let Some(board) = &self.board else { return false };
        let ids = self.selected_notes();
        let mut notes: Vec<BoardNote> = board
            .notes
            .iter()
            .filter(|n| n.session_id.is_none() && ids.contains(&n.id))
            .cloned()
            .collect();
        if notes.is_empty() {
            return false;
        }
        notes.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        let text = notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>().join("\n\n");
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            CopiedNotes { nodus_notes: notes },
        ));
        true
    }

    /// ⌘V: copied notes land as a group centred on the pointer (on the
    /// middle of the view when the pointer is off the board), moved to the
    /// nearest spot clear of cards, notes and the dock, and become the
    /// selection. False when the clipboard holds no notes.
    pub(crate) fn paste_notes(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.board.is_none() {
            return false;
        }
        let Some(notes) = cx.read_from_clipboard().as_ref().and_then(copied_notes) else {
            return false;
        };
        let rects: Vec<_> = notes
            .iter()
            .map(|n| rect_at(Point { x: n.x, y: n.y }, note_size(n)))
            .collect();
        let group = bounds_of(&rects);
        let size = Size { w: group.w, h: group.h };
        let centre = self.to_flow(
            self.pointer
                .filter(|p| self.bounds.contains(p))
                .unwrap_or_else(|| self.bounds.center()),
        );
        let mut obstacles = card_obstacles(self.cards(), &[]);
        obstacles.extend(self.notes_in_the_way());
        let spot = find_free_spot(
            Point {
                x: centre.x - group.w / 2.0,
                y: centre.y - group.h / 2.0,
            },
            &obstacles,
            Some(self.dock()),
            size,
        );
        let pasted = placed(notes, Point { x: group.x, y: group.y }, spot, crate::parity::now_ms(cx));
        let ids: Vec<String> = pasted.iter().map(|n| n.id.clone()).collect();
        self.state.update(cx, |s, cx| {
            for note in pasted {
                s.save_note(note, cx);
            }
        });
        // The store has them now: draw them, then select only them.
        self.sync(cx);
        for n in &mut self.nodes {
            n.selected = ids.contains(&n.id);
        }
        self.selection_rect = false;
        self.sync_selection(cx);
        self.ensure_visible(spot, size, cx);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::FolderColor;

    fn note(id: &str, x: f64, y: f64) -> BoardNote {
        BoardNote {
            id: id.to_owned(),
            text: format!("note {id}"),
            color: FolderColor::Blue,
            x,
            y,
            w: Some(300.0),
            h: None,
            session_id: Some("s1".to_owned()),
            from: Some("s2".to_owned()),
            from_project: Some("p".to_owned()),
            from_title: Some("t".to_owned()),
            created_at: 1.0,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn copies_keep_their_layout_text_colour_and_size_but_nothing_else() {
        let out = placed(
            vec![note("a", 100.0, 50.0), note("b", 340.0, 90.5)],
            Point { x: 100.0, y: 50.0 },
            Point { x: -20.0, y: 400.0 },
            42.0,
        );
        assert_eq!((out[0].x, out[0].y), (-20.0, 400.0));
        assert_eq!((out[1].x, out[1].y), (220.0, 441.0));
        assert!(out.iter().all(|n| n.id != "a" && n.id != "b"));
        assert_ne!(out[0].id, out[1].id);
        assert_eq!(out[1].text, "note b");
        assert_eq!(
            (out[1].color, out[1].w, out[1].h),
            (FolderColor::Blue, Some(300.0), None)
        );
        assert!(out.iter().all(|n| n.session_id.is_none()
            && n.from.is_none()
            && n.from_project.is_none()
            && n.from_title.is_none()
            && n.created_at == 42.0));
    }

    #[test]
    fn reads_back_only_its_own_notes() {
        let ours = ClipboardItem::new_string_with_json_metadata(
            "note a".to_owned(),
            CopiedNotes {
                nodus_notes: vec![note("a", 0.0, 0.0)],
            },
        );
        assert_eq!(copied_notes(&ours).map(|n| n[0].id.clone()), Some("a".to_owned()));
        assert!(copied_notes(&ClipboardItem::new_string("note a".to_owned())).is_none());
        // Another app's metadata (Zed writes this pasteboard type too).
        let other = ClipboardItem::new_string_with_metadata("x".to_owned(), r#"[{"len":1}]"#.to_owned());
        assert!(copied_notes(&other).is_none());
        let empty = ClipboardItem::new_string_with_json_metadata("".to_owned(), CopiedNotes { nodus_notes: vec![] });
        assert!(copied_notes(&empty).is_none());
    }
}
