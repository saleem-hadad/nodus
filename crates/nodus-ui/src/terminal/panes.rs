//! The session window's panes (SPLIT-001…SPLIT-024): up to four sessions
//! side by side in one window, the one the keyboard is in, and the lists a
//! board keeps so a card opens again with the cards it was beside.

use gpui::{Pixels, Size, px};

/// SPLIT-002: at most this many panes.
pub const MAX_PANES: usize = 4;

/// The narrowest share a column or row can be dragged to (SPLIT-012).
const MIN_SHARE: f32 = 0.25;

/// The sessions on show in the session window, in order, and the focused one.
/// Empty while the window is closed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Panes {
    ids: Vec<String>,
    focused: usize,
}

impl Panes {
    /// The window on `ids` (at most [`MAX_PANES`], no repeats), `focus` in front.
    pub fn open(ids: Vec<String>, focus: &str) -> Self {
        let mut unique: Vec<String> = Vec::new();
        for id in ids {
            if !unique.contains(&id) && unique.len() < MAX_PANES {
                unique.push(id);
            }
        }
        if !unique.iter().any(|id| id == focus) {
            unique = vec![focus.to_owned()];
        }
        let focused = unique.iter().position(|id| id == focus).unwrap_or(0);
        Self { ids: unique, focused }
    }

    pub fn is_open(&self) -> bool {
        !self.ids.is_empty()
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.iter().any(|i| i == id)
    }

    /// The pane the keyboard is in.
    pub fn focused(&self) -> Option<&str> {
        self.ids.get(self.focused).map(String::as_str)
    }

    /// SPLIT-003: the window is open and has room for another pane.
    pub fn has_room(&self) -> bool {
        self.is_open() && self.ids.len() < MAX_PANES
    }

    /// SPLIT-004: `id` in a new pane after the others, focused. A card
    /// already on show is only focused. False when nothing changed.
    pub fn add(&mut self, id: &str) -> bool {
        if let Some(at) = self.ids.iter().position(|i| i == id) {
            let moved = at != self.focused;
            self.focused = at;
            return moved;
        }
        if !self.has_room() {
            return false;
        }
        self.ids.push(id.to_owned());
        self.focused = self.ids.len() - 1;
        true
    }

    /// SPLIT-006: `id`'s pane goes. The focus stays where it was, or moves
    /// to the pane before the one that went. False when it wasn't on show.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(at) = self.ids.iter().position(|i| i == id) else {
            return false;
        };
        self.ids.remove(at);
        if self.ids.is_empty() {
            self.focused = 0;
        } else if at < self.focused || (at == self.focused && at > 0) {
            self.focused -= 1;
        }
        true
    }

    /// SPLIT-008: the keyboard moves to `id`'s pane.
    pub fn focus(&mut self, id: &str) -> bool {
        match self.ids.iter().position(|i| i == id) {
            Some(at) if at != self.focused => {
                self.focused = at;
                true
            }
            _ => false,
        }
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.focused = 0;
    }
}

/// SPLIT-021: the panes a card opens in: the list it was last shown in, of
/// those cards still `open`able, or the card alone.
pub fn panes_for(lists: &[Vec<String>], id: &str, openable: impl Fn(&str) -> bool) -> Vec<String> {
    let Some(list) = lists.iter().find(|l| l.iter().any(|i| i == id)) else {
        return vec![id.to_owned()];
    };
    let mut panes: Vec<String> = list
        .iter()
        .filter(|i| *i == id || openable(i))
        .take(MAX_PANES)
        .cloned()
        .collect();
    if !panes.iter().any(|i| i == id) {
        panes = vec![id.to_owned()];
    }
    panes
}

/// SPLIT-020: the lists once a window shows `shown`: it replaces whatever
/// lists held its cards, the cards it no longer shows leave theirs, and a
/// list of one card isn't kept. Lists keep only cards still `kept`.
pub fn remember(
    lists: &[Vec<String>],
    before: &[String],
    shown: &[String],
    kept: impl Fn(&str) -> bool,
) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = lists
        .iter()
        .map(|l| {
            l.iter()
                .filter(|i| !shown.contains(i) && !before.contains(i) && kept(i))
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|l| l.len() > 1)
        .collect();
    let shown: Vec<String> = shown.iter().filter(|i| kept(i)).cloned().collect();
    if shown.len() > 1 {
        out.push(shown);
    }
    out
}

/// SPLIT-010: the panes' columns, top to bottom: side by side for two; the
/// third under the second; the fourth under the first.
pub fn columns(count: usize) -> Vec<Vec<usize>> {
    match count {
        0 => Vec::new(),
        1 => vec![vec![0]],
        2 => vec![vec![0], vec![1]],
        3 => vec![vec![0], vec![1, 2]],
        _ => vec![vec![0, 3], vec![1, 2]],
    }
}

/// SPLIT-013: the pane whose header holds the window's Split and close
/// buttons: the top one on the right.
pub fn corner_pane(count: usize) -> usize {
    if count > 1 { 1 } else { 0 }
}

/// Which outer corners of the window a pane has, for rounding them
/// (top-left, top-right, bottom-right, bottom-left).
pub fn outer_corners(count: usize, pane: usize) -> [bool; 4] {
    let cols = columns(count);
    let Some((c, col)) = cols.iter().enumerate().find(|(_, col)| col.contains(&pane)) else {
        return [false; 4];
    };
    let row = col.iter().position(|&p| p == pane).unwrap_or(0);
    let first_col = c == 0;
    let last_col = c + 1 == cols.len();
    let top = row == 0;
    let bottom = row + 1 == col.len();
    [
        first_col && top,
        last_col && top,
        last_col && bottom,
        first_col && bottom,
    ]
}

/// SPLIT-011: MODAL-032's min(1400px, 92vw) × 88vh for one pane; for more,
/// 96vw × 92vh (the modal keeps 24px clear of each edge).
pub fn window_size(viewport: Size<Pixels>, count: usize) -> Size<Pixels> {
    if count > 1 {
        Size {
            width: viewport.width * 0.96,
            height: viewport.height * 0.92,
        }
    } else {
        Size {
            width: px(1400.).min(viewport.width * 0.92),
            height: viewport.height * 0.88,
        }
    }
}

/// Where the lines between panes sit, as shares of the window (SPLIT-012):
/// between the columns, and between the rows of each column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shares {
    pub columns: f32,
    pub rows: [f32; 2],
}

impl Default for Shares {
    fn default() -> Self {
        Self {
            columns: 0.5,
            rows: [0.5, 0.5],
        }
    }
}

/// A share as dragged, kept off the window's edges.
pub fn clamp_share(share: f32) -> f32 {
    share.clamp(MIN_SHARE, 1. - MIN_SHARE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn split_002_open_takes_four_unique_with_focus() {
        let p = Panes::open(ids(&["a", "b", "a", "c", "d", "e"]), "c");
        assert_eq!(p.ids(), ids(&["a", "b", "c", "d"]).as_slice());
        assert_eq!(p.focused(), Some("c"));
        // A focus that didn't fit opens alone.
        let p = Panes::open(ids(&["a", "b", "c", "d", "e"]), "e");
        assert_eq!(p.ids(), ids(&["e"]).as_slice());
    }

    #[test]
    fn split_004_add_appends_and_focuses_up_to_four() {
        let mut p = Panes::open(ids(&["a"]), "a");
        assert!(p.add("b"));
        assert!(p.add("c"));
        assert!(p.add("d"));
        assert_eq!(p.focused(), Some("d"));
        assert!(!p.has_room());
        assert!(!p.add("e"));
        assert_eq!(p.len(), 4);
        // On show already: only focused.
        assert!(p.add("a"));
        assert_eq!(p.focused(), Some("a"));
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn split_006_remove_moves_focus_to_the_pane_before() {
        let mut p = Panes::open(ids(&["a", "b", "c"]), "c");
        assert!(p.remove("c"));
        assert_eq!(p.focused(), Some("b"));
        let mut p = Panes::open(ids(&["a", "b", "c"]), "c");
        assert!(p.remove("a"));
        assert_eq!(p.focused(), Some("c"));
        let mut p = Panes::open(ids(&["a", "b"]), "a");
        assert!(p.remove("a"));
        assert_eq!(p.focused(), Some("b"));
        assert!(p.remove("b"));
        assert!(!p.is_open());
        assert!(!p.remove("b"));
    }

    #[test]
    fn split_021_panes_for_restores_the_list() {
        let lists = vec![ids(&["a", "b", "c"]), ids(&["x", "y"])];
        assert_eq!(panes_for(&lists, "b", |_| true), ids(&["a", "b", "c"]));
        assert_eq!(panes_for(&lists, "z", |_| true), ids(&["z"]));
        // Cards that can't be opened any more are left out.
        assert_eq!(panes_for(&lists, "a", |i| i != "b"), ids(&["a", "c"]));
    }

    #[test]
    fn split_020_remember_replaces_and_drops_singles() {
        let all = |_: &str| true;
        // A window of two is kept.
        let lists = remember(&[], &ids(&["a"]), &ids(&["a", "b"]), all);
        assert_eq!(lists, vec![ids(&["a", "b"])]);
        // b moves to a new window with c: a is left alone, so its list goes.
        let lists = remember(&lists, &ids(&["c"]), &ids(&["c", "b"]), all);
        assert_eq!(lists, vec![ids(&["c", "b"])]);
        // Closing a pane: the card that left is forgotten with the list.
        let lists = remember(&[ids(&["a", "b", "c"])], &ids(&["a", "b", "c"]), &ids(&["a", "c"]), all);
        assert_eq!(lists, vec![ids(&["a", "c"])]);
        let lists = remember(&lists, &ids(&["a", "c"]), &ids(&["a"]), all);
        assert!(lists.is_empty());
        // Cards gone from the board are dropped from every list.
        let lists = remember(&[ids(&["x", "y", "z"])], &ids(&["a"]), &ids(&["a", "b"]), |i| i != "y");
        assert_eq!(lists, vec![ids(&["x", "z"]), ids(&["a", "b"])]);
    }

    #[test]
    fn split_010_columns_and_corners() {
        assert_eq!(columns(1), vec![vec![0]]);
        assert_eq!(columns(3), vec![vec![0], vec![1, 2]]);
        assert_eq!(columns(4), vec![vec![0, 3], vec![1, 2]]);
        assert_eq!(corner_pane(1), 0);
        assert_eq!(corner_pane(3), 1);
        assert_eq!(outer_corners(1, 0), [true; 4]);
        assert_eq!(outer_corners(2, 0), [true, false, false, true]);
        assert_eq!(outer_corners(3, 1), [false, true, false, false]);
        assert_eq!(outer_corners(3, 2), [false, false, true, false]);
        assert_eq!(outer_corners(4, 3), [false, false, false, true]);
    }

    #[test]
    fn split_011_window_size() {
        let viewport = Size {
            width: px(2000.),
            height: px(1000.),
        };
        assert_eq!(
            window_size(viewport, 1),
            Size {
                width: px(1400.),
                height: px(880.)
            }
        );
        assert_eq!(
            window_size(viewport, 2),
            Size {
                width: px(1920.),
                height: px(920.)
            }
        );
    }

    #[test]
    fn split_012_shares_stay_off_the_edges() {
        assert_eq!(clamp_share(0.1), 0.25);
        assert_eq!(clamp_share(0.9), 0.75);
        assert_eq!(clamp_share(0.4), 0.4);
    }
}
