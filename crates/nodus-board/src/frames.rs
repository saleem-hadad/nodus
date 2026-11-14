//! Frames and groups (spec 17), added after 1.0.14.
//!
//! A frame is a named box on the board, stored as a section: what lies in it
//! (the centre inside) moves with it, and it keeps its size as they move. A
//! group glues things together so they're selected and moved as one; it
//! isn't drawn. What the window sends is checked here before it's kept.

use nodus_domain::js;
use nodus_domain::{BoardGroup, BoardSection, Rect};
use nodus_store::BoardState;
use serde_json::Map;

use crate::geometry::{CARD_H, CARD_W, Point, card_size};
use crate::shapes::holds;

/// A frame's header strip, its handle, and the room left round what it holds.
pub const FRAME_HEADER_H: f64 = 44.0;
pub const FRAME_PAD: f64 = 24.0;
/// The smallest and largest a frame is.
pub const FRAME_MIN_W: f64 = 200.0;
pub const FRAME_MIN_H: f64 = 120.0;
pub const FRAME_MAX: f64 = 40_000.0;
/// How many things a group holds at most, and groups a board keeps.
pub const GROUP_MAX_MEMBERS: usize = 2000;
pub const MAX_GROUPS: usize = 2000;

/// The longest id kept, as shapes'.
const MAX_ID: usize = 64;

fn id_ok(id: &str) -> bool {
    !id.is_empty() && js::utf16_len(id) <= MAX_ID
}

/// `frameAround`: the frame around `rects`, or an empty one-card frame at
/// `corner` (BOARD-081).
pub fn frame_around(corner: Point, rects: &[Rect]) -> Rect {
    if rects.is_empty() {
        return Rect {
            x: corner.x,
            y: corner.y,
            w: FRAME_PAD * 2.0 + CARD_W,
            h: FRAME_HEADER_H + CARD_H + FRAME_PAD,
        };
    }
    let x = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let y = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let right = rects.iter().map(|r| r.x + r.w).fold(f64::NEG_INFINITY, f64::max);
    let bottom = rects.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
    Rect {
        x: x - FRAME_PAD,
        y: y - FRAME_HEADER_H,
        w: right - x + FRAME_PAD * 2.0,
        h: bottom - y + FRAME_HEADER_H + FRAME_PAD,
    }
}

/// FRAME-002, FRAME-003: where a frame is. One saved before frames had a
/// size is drawn round `members`, the cards that name it in `sectionId`, as
/// sections were.
pub fn frame_rect(section: &BoardSection, members: &[Rect]) -> Rect {
    match (section.w, section.h) {
        (Some(w), Some(h)) => Rect {
            x: section.x,
            y: section.y,
            w,
            h,
        },
        _ => frame_around(
            Point {
                x: section.x,
                y: section.y,
            },
            members,
        ),
    }
}

/// FRAME-010: whether something at `r` is in a frame at `frame`: its centre
/// is, edges counting.
pub fn holds_centre(frame: Rect, r: Rect) -> bool {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    cx >= frame.x && cx <= frame.x + frame.w && cy >= frame.y && cy <= frame.y + frame.h
}

/// The frames on a stored board, with where each is, oldest first.
pub fn board_frames(board: &BoardState) -> Vec<(&BoardSection, Rect)> {
    let mut sections: Vec<&BoardSection> = board.sections.values().collect();
    sections.sort_by(|a, b| {
        a.created_at
            .partial_cmp(&b.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sections
        .into_iter()
        .map(|s| {
            let members: Vec<Rect> = board
                .nodes
                .values()
                .filter(|n| !n.archived && n.section_id.as_deref() == Some(s.id.as_str()))
                .map(|n| {
                    let size = card_size(Some(n));
                    Rect {
                        x: n.x,
                        y: n.y,
                        w: size.w,
                        h: size.h,
                    }
                })
                .collect();
            (s, frame_rect(s, &members))
        })
        .collect()
}

/// The frame something at `r` is in: the smallest that holds its centre.
pub fn frame_holding<'a>(frames: &[(&'a BoardSection, Rect)], r: Rect) -> Option<&'a BoardSection> {
    frames
        .iter()
        .filter(|(_, f)| holds_centre(*f, r))
        .min_by(|a, b| {
            (a.1.w * a.1.h)
                .partial_cmp(&(b.1.w * b.1.h))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(s, _)| *s)
}

/// A frame's size, kept within bounds; none for one that isn't a number.
pub fn clamp_frame(w: f64, h: f64) -> Option<(f64, f64)> {
    (w.is_finite() && h.is_finite()).then(|| {
        (
            js::round(w.clamp(FRAME_MIN_W, FRAME_MAX)),
            js::round(h.clamp(FRAME_MIN_H, FRAME_MAX)),
        )
    })
}

/// The group worth keeping, cleaned up: each member once, two or more of
/// them; none when it's malformed.
pub fn clean_group(group: &BoardGroup) -> Option<BoardGroup> {
    if !id_ok(&group.id) {
        return None;
    }
    let mut members: Vec<String> = Vec::new();
    for m in &group.members {
        if id_ok(m) && !members.contains(m) && members.len() < GROUP_MAX_MEMBERS {
            members.push(m.clone());
        }
    }
    (members.len() >= 2).then(|| BoardGroup {
        id: group.id.clone(),
        members,
        created_at: group.created_at,
        extra: Map::new(),
    })
}

/// GROUP-030: keeps these groups, new or changed. A thing is in one group
/// at most, so the last one saved with it takes it from any other; members
/// that aren't on the board are left out.
pub fn save_groups(board: &mut BoardState, groups: &[BoardGroup], now: f64) {
    for group in groups {
        let Some(mut clean) = clean_group(group) else {
            continue;
        };
        clean.members.retain(|m| holds(board, m));
        if clean.members.len() < 2 {
            continue;
        }
        match board.groups.get(&clean.id) {
            Some(stored) => {
                clean.created_at = stored.created_at;
                clean.extra.clone_from(&stored.extra);
            }
            None => {
                if board.groups.values().count() >= MAX_GROUPS {
                    continue;
                }
                if !clean.created_at.is_finite() || clean.created_at <= 0.0 {
                    clean.created_at = now;
                }
            }
        }
        for other in board.groups.values_mut() {
            if other.id != clean.id {
                other.members.retain(|m| !clean.members.contains(m));
            }
        }
        board.groups.insert(clean.id.clone(), clean);
    }
    prune_groups(board);
}

/// GROUP-031: takes groups off the board; what was in them stays.
pub fn remove_groups(board: &mut BoardState, ids: &[String]) {
    for id in ids {
        board.groups.shift_remove(id);
    }
}

/// GROUP-032: members that left the board go from their groups, and a group
/// left with fewer than two goes.
pub fn prune_groups(board: &mut BoardState) {
    let kept: Vec<(String, Vec<String>)> = board
        .groups
        .values()
        .map(|g| {
            (
                g.id.clone(),
                g.members.iter().filter(|m| holds(board, m)).cloned().collect(),
            )
        })
        .collect();
    for (id, members) in kept {
        if members.len() < 2 {
            board.groups.shift_remove(&id);
        } else if let Some(g) = board.groups.get_mut(&id) {
            g.members = members;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardNote, FolderColor};

    fn note(id: &str) -> BoardNote {
        BoardNote {
            id: id.into(),
            text: String::new(),
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

    fn board(notes: &[&str]) -> BoardState {
        let mut b = BoardState::default();
        for id in notes {
            b.notes.insert((*id).to_owned(), note(id));
        }
        b
    }

    fn group(id: &str, members: &[&str]) -> BoardGroup {
        BoardGroup {
            id: id.into(),
            members: members.iter().map(|m| (*m).to_owned()).collect(),
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    fn members(b: &BoardState, id: &str) -> Option<Vec<String>> {
        b.groups.get(id).map(|g| g.members.clone())
    }

    #[test]
    fn keeps_a_group_of_two_or_more_things_on_the_board() {
        let mut b = board(&["a", "b", "c"]);
        save_groups(&mut b, &[group("g", &["a", "b", "a", "gone"])], 5.0);
        assert_eq!(members(&b, "g"), Some(vec!["a".into(), "b".into()]));
        assert_eq!(b.groups.get("g").map(|g| g.created_at), Some(5.0));
        // One thing alone isn't a group.
        save_groups(&mut b, &[group("h", &["c", "gone"])], 6.0);
        assert_eq!(members(&b, "h"), None);
    }

    #[test]
    fn takes_a_thing_from_the_group_it_was_in() {
        let mut b = board(&["a", "b", "c", "d"]);
        save_groups(&mut b, &[group("g", &["a", "b", "c"])], 1.0);
        save_groups(&mut b, &[group("h", &["c", "d"])], 2.0);
        assert_eq!(members(&b, "g"), Some(vec!["a".into(), "b".into()]));
        // A group left with one goes.
        save_groups(&mut b, &[group("i", &["b", "d"])], 3.0);
        assert_eq!(members(&b, "g"), None);
        assert_eq!(members(&b, "h"), None);
        assert_eq!(members(&b, "i"), Some(vec!["b".into(), "d".into()]));
    }

    #[test]
    fn prunes_what_left_the_board() {
        let mut b = board(&["a", "b", "c"]);
        save_groups(&mut b, &[group("g", &["a", "b", "c"])], 1.0);
        b.notes.shift_remove("c");
        prune_groups(&mut b);
        assert_eq!(members(&b, "g"), Some(vec!["a".into(), "b".into()]));
        b.notes.shift_remove("b");
        prune_groups(&mut b);
        assert!(b.groups.is_empty());
    }

    #[test]
    fn frames_keep_their_size_or_wrap_their_old_cards() {
        let mut s = BoardSection {
            id: "s".into(),
            name: String::new(),
            color: FolderColor::Blue,
            x: 10.0,
            y: 20.0,
            w: None,
            h: None,
            created_at: 0.0,
            extra: Map::new(),
        };
        let card = Rect {
            x: 100.0,
            y: 100.0,
            w: 288.0,
            h: 216.0,
        };
        assert_eq!(
            frame_rect(&s, &[card]),
            Rect {
                x: 76.0,
                y: 56.0,
                w: 336.0,
                h: 284.0
            }
        );
        assert_eq!(
            frame_rect(&s, &[]),
            Rect {
                x: 10.0,
                y: 20.0,
                w: 336.0,
                h: 284.0
            }
        );
        s.w = Some(900.0);
        s.h = Some(500.0);
        assert_eq!(
            frame_rect(&s, &[card]),
            Rect {
                x: 10.0,
                y: 20.0,
                w: 900.0,
                h: 500.0
            }
        );
        assert!(holds_centre(frame_rect(&s, &[]), card));
        assert_eq!(clamp_frame(10.0, 1e9), Some((FRAME_MIN_W, FRAME_MAX)));
        assert_eq!(clamp_frame(f64::NAN, 1.0), None);
    }
}
