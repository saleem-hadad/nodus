//! What the title bar's branch chip and Changes button and the Changes modal
//! say, and how the modal picks its file: the pure parts of `GitControls.tsx`
//! and `DiffModal.tsx`, for nodus-ui to draw.

use std::time::Duration;

use nodus_domain::js::slice_utf16;
use nodus_domain::{GitFileChange, GitFileDiff, GitFileStatus, GitStatus};

/// How often the branch, the change count and the selected file's diff are
/// read again while the window has focus (GIT-023, GIT-041).
pub const GIT_POLL: Duration = Duration::from_millis(4000);
/// Pointing at the branch waits this long before the log opens, and leaving
/// waits as long before it closes (GIT-027).
pub const LOG_OPEN_DELAY: Duration = Duration::from_millis(150);
pub const LOG_CLOSE_DELAY: Duration = Duration::from_millis(150);

/// An oid's first seven characters.
fn short(oid: &str) -> String {
    slice_utf16(oid, 0, 7)
}

/// GIT-026 / GIT-034: the branch, else the commit HEAD is at, else `HEAD`.
pub fn branch_label(git: &GitStatus) -> String {
    match (&git.branch, &git.oid) {
        (Some(branch), _) => branch.clone(),
        (None, Some(oid)) => short(oid),
        (None, None) => "HEAD".into(),
    }
}

/// GIT-026: what follows the chip's label: `↑2`, `↓1`, both, or neither.
pub fn ahead_behind(git: &GitStatus) -> Vec<String> {
    let mut out = Vec::new();
    if git.ahead > 0 {
        out.push(format!("↑{}", git.ahead));
    }
    if git.behind > 0 {
        out.push(format!("↓{}", git.behind));
    }
    out
}

/// GIT-029: the popover's first line.
pub fn log_branch(git: &GitStatus) -> &str {
    git.branch.as_deref().unwrap_or("Detached HEAD")
}

/// GIT-029: the popover's tracking line.
pub fn tracking_label(git: &GitStatus) -> String {
    // Empty strings are falsy in JavaScript.
    let Some(oid) = git.oid.as_deref().filter(|o| !o.is_empty()) else {
        return "No commits yet".into();
    };
    if git.branch.as_deref().is_none_or(str::is_empty) {
        return format!("At {}", short(oid));
    }
    // An empty upstream is falsy in JavaScript.
    let Some(upstream) = git.upstream.as_deref().filter(|u| !u.is_empty()) else {
        return "Not published to a remote".into();
    };
    let mut ab = Vec::new();
    if git.ahead > 0 {
        ab.push(format!("{} ahead", git.ahead));
    }
    if git.behind > 0 {
        ab.push(format!("{} behind", git.behind));
    }
    if ab.is_empty() {
        format!("Tracking {upstream}, up to date")
    } else {
        format!("Tracking {upstream} · {}", ab.join(", "))
    }
}

/// The key a log is cached under: HEAD moves with a commit, an amend, a reset
/// or a checkout, and the log with it (GIT-028).
pub fn log_key(project_id: &str, git: &GitStatus) -> String {
    // `${git.oid}` writes null as "null".
    format!("{project_id}:{}", git.oid.as_deref().unwrap_or("null"))
}

/// GIT-025: the Changes button's tooltip.
pub fn changes_title(count: usize) -> String {
    match count {
        0 => "No local changes".into(),
        1 => "Show 1 changed file".into(),
        n => format!("Show {n} changed files"),
    }
}

/// GIT-025: the count badge, when there are changes.
pub fn changes_badge(count: usize) -> Option<String> {
    match count {
        0 => None,
        n if n > 999 => Some("999+".into()),
        n => Some(n.to_string()),
    }
}

/// GIT-034: the modal's count, beside the branch.
pub fn changed_files(count: usize) -> String {
    match count {
        0 => "No changes".into(),
        1 => "1 changed file".into(),
        n => format!("{n} changed files"),
    }
}

/// GIT-036: how a file's status is named.
pub const fn status_label(status: GitFileStatus) -> &'static str {
    match status {
        GitFileStatus::Modified => "Modified",
        GitFileStatus::Added => "Added",
        GitFileStatus::Untracked => "New",
        GitFileStatus::Deleted => "Deleted",
        GitFileStatus::Renamed => "Renamed",
        GitFileStatus::Conflicted => "Conflicted",
    }
}

/// GIT-037: the lucide icon a status is drawn with.
pub const fn status_icon(status: GitFileStatus) -> &'static str {
    match status {
        GitFileStatus::Added | GitFileStatus::Untracked => "square-plus",
        GitFileStatus::Deleted => "square-minus",
        GitFileStatus::Renamed => "square-arrow-right",
        GitFileStatus::Conflicted => "triangle-alert",
        GitFileStatus::Modified => "square-dot",
    }
}

/// GIT-036: a file row's tooltip.
pub fn file_title(file: &GitFileChange) -> String {
    let from = match file.old_path.as_deref().filter(|p| !p.is_empty()) {
        Some(old) => format!("{old} → "),
        None => String::new(),
    };
    format!("{}: {from}{}", status_label(file.status), file.path)
}

/// GIT-036, GIT-044: a path's folder (up to and including its last `/`) and
/// its file name.
pub fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('/') {
        Some(slash) => p.split_at(slash + 1),
        None => ("", p),
    }
}

/// GIT-041: what a file's diff is cached under.
pub fn file_key(file: &GitFileChange) -> String {
    format!(
        "{}\0{}\0{}",
        file.status,
        file.old_path.as_deref().unwrap_or_default(),
        file.path
    )
}

/// GIT-041: whether a re-read diff can be left as it's drawn: the same patch
/// text and the same binary flag. A diff that turns too large while both
/// patches are empty counts as the same, as it does in 0.9.4.
pub fn same_diff(prev: &GitFileDiff, next: &GitFileDiff) -> bool {
    prev.patch == next.patch && prev.binary.unwrap_or(false) == next.binary.unwrap_or(false)
}

/// GIT-038, GIT-039: the Changes list's selection, remembered by path. If
/// that file drops out of the list, the one now at its index takes over; the
/// path is kept, so the file is selected again if it comes back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSelection {
    remembered: Option<(String, usize)>,
}

/// A key the focused list handles (GIT-039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKey {
    Down,
    Up,
    Home,
    End,
}

impl DiffSelection {
    /// The selected index in `files`, if any.
    pub fn index(&self, files: &[GitFileChange]) -> Option<usize> {
        let found = self
            .remembered
            .as_ref()
            .and_then(|(path, _)| files.iter().position(|f| &f.path == path));
        if found.is_some() {
            return found;
        }
        let wanted = self.remembered.as_ref().map_or(0, |(_, index)| *index);
        (!files.is_empty()).then(|| wanted.min(files.len() - 1))
    }

    /// Selects the file at `i`, kept to the list.
    pub fn select(&mut self, files: &[GitFileChange], i: isize) {
        let Some(last) = files.len().checked_sub(1) else {
            return;
        };
        let at = usize::try_from(i.max(0)).unwrap_or(0).min(last);
        self.remembered = Some((files[at].path.clone(), at));
    }

    /// ↓ / ↑ move by one, Home / End jump to the ends.
    pub fn key(&mut self, files: &[GitFileChange], key: ListKey) {
        // With nothing selected the index is -1, as in JavaScript.
        let index = self.index(files).map_or(-1, |i| i as isize);
        let to = match key {
            ListKey::Down => index + 1,
            ListKey::Up => index - 1,
            ListKey::Home => 0,
            ListKey::End => files.len() as isize - 1,
        };
        self.select(files, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(branch: Option<&str>, oid: Option<&str>, upstream: Option<&str>, ahead: u32, behind: u32) -> GitStatus {
        GitStatus {
            branch: branch.map(Into::into),
            oid: oid.map(Into::into),
            upstream: upstream.map(Into::into),
            ahead,
            behind,
            files: Vec::new(),
        }
    }

    fn file(path: &str) -> GitFileChange {
        GitFileChange {
            path: path.into(),
            old_path: None,
            status: GitFileStatus::Modified,
        }
    }

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn labels_the_branch_chip() {
        assert_eq!(branch_label(&status(Some("main"), Some(OID), None, 0, 0)), "main");
        assert_eq!(branch_label(&status(None, Some(OID), None, 0, 0)), "0123456");
        assert_eq!(branch_label(&status(None, None, None, 0, 0)), "HEAD");
        assert_eq!(ahead_behind(&status(Some("m"), None, None, 2, 0)), ["↑2"]);
        assert_eq!(ahead_behind(&status(Some("m"), None, None, 2, 3)), ["↑2", "↓3"]);
        assert!(ahead_behind(&status(Some("m"), None, None, 0, 0)).is_empty());
    }

    #[test]
    fn says_how_the_branch_tracks() {
        let t = |b, o, u, a, be| tracking_label(&status(b, o, u, a, be));
        assert_eq!(t(Some("main"), None, None, 0, 0), "No commits yet");
        assert_eq!(t(None, Some(OID), None, 0, 0), "At 0123456");
        assert_eq!(t(Some("main"), Some(OID), None, 0, 0), "Not published to a remote");
        assert_eq!(
            t(Some("main"), Some(OID), Some("origin/main"), 0, 0),
            "Tracking origin/main, up to date"
        );
        assert_eq!(
            t(Some("main"), Some(OID), Some("origin/main"), 2, 0),
            "Tracking origin/main · 2 ahead"
        );
        assert_eq!(
            t(Some("main"), Some(OID), Some("origin/main"), 0, 1),
            "Tracking origin/main · 1 behind"
        );
        assert_eq!(
            t(Some("main"), Some(OID), Some("origin/main"), 2, 1),
            "Tracking origin/main · 2 ahead, 1 behind"
        );
        assert_eq!(log_branch(&status(None, Some(OID), None, 0, 0)), "Detached HEAD");
        assert_eq!(log_key("p", &status(None, None, None, 0, 0)), "p:null");
        assert_eq!(log_key("p", &status(None, Some("abc"), None, 0, 0)), "p:abc");
    }

    #[test]
    fn counts_changes() {
        assert_eq!(changes_title(0), "No local changes");
        assert_eq!(changes_title(1), "Show 1 changed file");
        assert_eq!(changes_title(4), "Show 4 changed files");
        assert_eq!(changes_badge(0), None);
        assert_eq!(changes_badge(999).as_deref(), Some("999"));
        assert_eq!(changes_badge(1000).as_deref(), Some("999+"));
        assert_eq!(changed_files(0), "No changes");
        assert_eq!(changed_files(1), "1 changed file");
        assert_eq!(changed_files(2), "2 changed files");
    }

    #[test]
    fn names_files_in_the_list() {
        let mut renamed = file("src/new.ts");
        renamed.status = GitFileStatus::Renamed;
        renamed.old_path = Some("src/old.ts".into());
        assert_eq!(file_title(&renamed), "Renamed: src/old.ts → src/new.ts");
        assert_eq!(file_title(&file("a.txt")), "Modified: a.txt");
        assert_eq!(status_label(GitFileStatus::Untracked), "New");
        assert_eq!(status_icon(GitFileStatus::Untracked), "square-plus");
        assert_eq!(split_path("src/ui/a.ts"), ("src/ui/", "a.ts"));
        assert_eq!(split_path("a.ts"), ("", "a.ts"));
        assert_eq!(file_key(&renamed), "renamed\0src/old.ts\0src/new.ts");
        assert_eq!(file_key(&file("a")), "modified\0\0a");
    }

    #[test]
    fn redraws_a_diff_only_when_it_changed() {
        let diff = |patch: &str, binary: Option<bool>, too_large: Option<bool>| GitFileDiff {
            patch: patch.into(),
            binary,
            too_large,
        };
        assert!(same_diff(&diff("x", None, None), &diff("x", None, None)));
        assert!(!same_diff(&diff("x", None, None), &diff("y", None, None)));
        assert!(!same_diff(&diff("", None, None), &diff("", Some(true), None)));
        // Kept from 0.9.4: an empty patch turning too large isn't redrawn.
        assert!(same_diff(&diff("", None, None), &diff("", None, Some(true))));
    }

    #[test]
    fn remembers_the_selection_by_path() {
        let mut files = vec![file("a"), file("b"), file("c")];
        let mut sel = DiffSelection::default();
        assert_eq!(sel.index(&files), Some(0));
        sel.select(&files, 1);
        assert_eq!(sel.index(&files), Some(1));
        // b is committed: the file now at its index takes over.
        files.remove(1);
        assert_eq!(sel.index(&files), Some(1));
        files.truncate(1);
        assert_eq!(sel.index(&files), Some(0));
        // b comes back, and is selected again.
        files = vec![file("a"), file("c"), file("b")];
        assert_eq!(sel.index(&files), Some(2));
        assert_eq!(DiffSelection::default().index(&[]), None);
    }

    #[test]
    fn moves_the_selection_with_keys() {
        let files = vec![file("a"), file("b"), file("c")];
        let mut sel = DiffSelection::default();
        sel.key(&files, ListKey::Down);
        assert_eq!(sel.index(&files), Some(1));
        sel.key(&files, ListKey::End);
        assert_eq!(sel.index(&files), Some(2));
        sel.key(&files, ListKey::Down);
        assert_eq!(sel.index(&files), Some(2));
        sel.key(&files, ListKey::Home);
        sel.key(&files, ListKey::Up);
        assert_eq!(sel.index(&files), Some(0));
        let mut empty = DiffSelection::default();
        empty.key(&[], ListKey::Down);
        assert_eq!(empty, DiffSelection::default());
    }
}
