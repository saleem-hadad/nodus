//! Parses a single-file unified diff (`git diff` output) into lines the
//! Changes modal draws: numbered on each side, with the changed part of a
//! line that pairs with one on the other side marked, the way GitHub
//! highlights edited words. Pure.
//!
//! Ports `src/renderer/src/components/git/diff.ts` (GIT-048…GIT-055). Text,
//! lengths and the changed ranges are in UTF-16 code units, as JavaScript
//! measures them.

use std::sync::LazyLock;

use nodus_domain::js::slice_utf16;
use regex::Regex;

/// What a line of the diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// An `@@ … @@` header.
    Hunk,
    Context,
    Add,
    Del,
    /// `\ No newline at end of file`.
    Note,
}

impl DiffLineKind {
    /// The sign drawn before the text (GIT-047): `+`, `-`, a space for
    /// context, nothing for hunks and notes.
    pub const fn sign(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Del => "-",
            Self::Context => " ",
            Self::Hunk | Self::Note => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// The line without its +/-/space prefix; the whole `@@` header for hunks.
    pub text: String,
    pub old_no: Option<u64>,
    pub new_no: Option<u64>,
    /// `[start, end)` of the part that changed, in UTF-16 units, when the line
    /// pairs with one on the other side.
    pub changed: Option<(usize, usize)>,
}

impl DiffLine {
    fn new(kind: DiffLineKind, text: String, old_no: Option<u64>, new_no: Option<u64>) -> Self {
        Self {
            kind,
            text,
            old_no,
            new_no,
            changed: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDiff {
    pub lines: Vec<DiffLine>,
    /// The highest line number shown, to size the gutters.
    pub max_line_no: u64,
    /// What a diff without hunks is about (a pure rename, a mode change).
    pub summary: Option<String>,
}

impl ParsedDiff {
    /// How many characters wide each line-number column is (GIT-047).
    pub fn number_width(&self) -> usize {
        self.max_line_no.to_string().len().max(2)
    }
}

// `\d` is ASCII in JavaScript without the `u` flag.
static HUNK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@@ -([0-9]+)(?:,[0-9]+)? \+([0-9]+)(?:,[0-9]+)? @@").expect("a valid pattern"));

/// `Number(digits)`, which never fails on digits; one past `u64` saturates.
fn number(digits: &str) -> u64 {
    digits.parse().unwrap_or(u64::MAX)
}

/// GIT-048…GIT-055.
pub fn parse_diff(patch: &str) -> ParsedDiff {
    let mut lines = Vec::new();
    let mut header: Vec<&str> = Vec::new();
    let mut old_no: u64 = 0;
    let mut new_no: u64 = 0;
    let mut in_hunk = false;

    // GIT-048
    let mut raw: Vec<&str> = patch.split('\n').collect();
    if raw.last() == Some(&"") {
        raw.pop();
    }

    for line in raw {
        if let Some(hunk) = HUNK.captures(line) {
            // GIT-049
            in_hunk = true;
            old_no = number(&hunk[1]);
            new_no = number(&hunk[2]);
            lines.push(DiffLine::new(DiffLineKind::Hunk, line.to_owned(), None, None));
        } else if !in_hunk {
            // GIT-050
            header.push(line);
        } else if let Some(text) = line.strip_prefix('+') {
            // GIT-051
            lines.push(DiffLine::new(DiffLineKind::Add, text.to_owned(), None, Some(new_no)));
            new_no = new_no.saturating_add(1);
        } else if let Some(text) = line.strip_prefix('-') {
            lines.push(DiffLine::new(DiffLineKind::Del, text.to_owned(), Some(old_no), None));
            old_no = old_no.saturating_add(1);
        } else if line.starts_with('\\') {
            lines.push(DiffLine::new(
                DiffLineKind::Note,
                slice_utf16(line, 2, usize::MAX),
                None,
                None,
            ));
        } else {
            // Context lines start with a space; some tools strip it from blank lines.
            let text = slice_utf16(line, 1, usize::MAX);
            lines.push(DiffLine::new(DiffLineKind::Context, text, Some(old_no), Some(new_no)));
            old_no = old_no.saturating_add(1);
            new_no = new_no.saturating_add(1);
        }
    }

    mark_changed_spans(&mut lines);
    // GIT-053: the counters stop one past the last line shown.
    let max_line_no = old_no.saturating_sub(1).max(new_no.saturating_sub(1));
    // GIT-054
    let summary = lines.is_empty().then(|| summarize(&header));
    ParsedDiff {
        lines,
        max_line_no,
        summary,
    }
}

/// GIT-052: within each block of removed lines followed by as many added
/// lines, pairs the lines up and marks what differs between them (everything
/// between a common prefix and suffix). Notes can sit between a pair.
fn mark_changed_spans(all: &mut [DiffLine]) {
    let lines: Vec<usize> = (0..all.len()).filter(|&i| all[i].kind != DiffLineKind::Note).collect();
    let kind = |all: &[DiffLine], i: usize| all[lines[i]].kind;
    let mut i = 0;
    while i < lines.len() {
        if kind(all, i) != DiffLineKind::Del {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < lines.len() && kind(all, i) == DiffLineKind::Del {
            i += 1;
        }
        let add_start = i;
        while i < lines.len() && kind(all, i) == DiffLineKind::Add {
            i += 1;
        }
        let n = add_start - del_start;
        if n == 0 || i - add_start != n {
            continue;
        }
        for k in 0..n {
            let a: Vec<u16> = all[lines[del_start + k]].text.encode_utf16().collect();
            let b: Vec<u16> = all[lines[add_start + k]].text.encode_utf16().collect();
            let max = a.len().min(b.len());
            let mut pre = 0;
            while pre < max && a[pre] == b[pre] {
                pre += 1;
            }
            let mut suf = 0;
            while suf < max - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
                suf += 1;
            }
            // Nothing in common: highlighting the whole line adds nothing.
            if pre + suf == 0 {
                continue;
            }
            all[lines[del_start + k]].changed = Some((pre, a.len() - suf));
            all[lines[add_start + k]].changed = Some((pre, b.len() - suf));
        }
    }
}

/// GIT-055.
fn summarize(header: &[&str]) -> String {
    // An empty value is falsy in JavaScript.
    let find = |prefix: &str| {
        header
            .iter()
            .find_map(|l| l.strip_prefix(prefix))
            .filter(|v| !v.is_empty())
    };
    let mut parts = Vec::new();
    if let (Some(from), Some(_)) = (find("rename from "), find("rename to ")) {
        parts.push(format!("Renamed from {from} without changes"));
    }
    if let (Some(old), Some(new)) = (find("old mode "), find("new mode ")) {
        parts.push(format!("File mode changed from {old} to {new}"));
    }
    if find("new file mode ").is_some() {
        parts.push("Empty file".to_owned());
    }
    if parts.is_empty() {
        "No changes to show".to_owned()
    } else {
        parts.join(". ")
    }
}

#[cfg(test)]
mod tests {
    //! diff.test.ts.
    use super::*;
    use DiffLineKind::{Add, Context, Del, Hunk, Note};

    const HEADER: [&str; 4] = [
        "diff --git a/a.ts b/a.ts",
        "index 1111111..2222222 100644",
        "--- a/a.ts",
        "+++ b/a.ts",
    ];

    fn patch(lines: &[&str]) -> String {
        HEADER.iter().chain(lines).copied().collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn numbers_lines_on_each_side() {
        let parsed = parse_diff(&patch(&[
            "@@ -3,3 +3,4 @@ function f() {",
            " keep",
            "-old",
            "+new",
            "+added",
            " end",
            "",
        ]));
        let lines: Vec<_> = parsed
            .lines
            .iter()
            .map(|l| (l.kind, l.text.as_str(), l.old_no, l.new_no))
            .collect();
        assert_eq!(
            lines,
            [
                (Hunk, "@@ -3,3 +3,4 @@ function f() {", None, None),
                (Context, "keep", Some(3), Some(3)),
                (Del, "old", Some(4), None),
                (Add, "new", None, Some(4)),
                (Add, "added", None, Some(5)),
                (Context, "end", Some(5), Some(6)),
            ]
        );
        assert_eq!(parsed.max_line_no, 6);
    }

    #[test]
    fn marks_the_changed_part_of_a_line_that_pairs_with_another() {
        let parsed = parse_diff(&patch(&["@@ -1 +1 @@", "-const a = 1", "+const a = 2"]));
        assert_eq!(parsed.lines[1].changed, Some((10, 11)));
        assert_eq!(parsed.lines[2].changed, Some((10, 11)));
    }

    #[test]
    fn marks_nothing_when_the_blocks_differ_in_length_or_share_nothing() {
        let uneven = parse_diff(&patch(&["@@ -1,2 +1 @@", "-one", "-two", "+three"]));
        assert!(uneven.lines.iter().all(|l| l.changed.is_none()));
        let unrelated = parse_diff(&patch(&["@@ -1 +1 @@", "-abc", "+xyz"]));
        assert!(unrelated.lines.iter().all(|l| l.changed.is_none()));
    }

    #[test]
    fn pairs_lines_across_a_missing_newline_note() {
        let parsed = parse_diff(&patch(&[
            "@@ -1 +1 @@",
            "-let x = 1",
            "\\ No newline at end of file",
            "+let x = 2",
        ]));
        assert_eq!(
            parsed.lines[2],
            DiffLine::new(Note, "No newline at end of file".into(), None, None)
        );
        assert_eq!(parsed.lines[1].changed, Some((8, 9)));
        assert_eq!(parsed.lines[3].changed, Some((8, 9)));
    }

    #[test]
    fn reads_a_blank_context_line_that_lost_its_space() {
        let parsed = parse_diff(&patch(&["@@ -1,2 +1,2 @@", "", " b"]));
        assert_eq!(parsed.lines[1], DiffLine::new(Context, String::new(), Some(1), Some(1)));
    }

    #[test]
    fn sums_up_a_diff_with_no_hunks() {
        let summary = |extra: &[&str]| {
            let mut lines = vec!["diff --git a/a b/b"];
            lines.extend(extra);
            parse_diff(&lines.join("\n")).summary
        };
        assert_eq!(
            summary(&["similarity index 100%", "rename from a.ts", "rename to b.ts"]).as_deref(),
            Some("Renamed from a.ts without changes")
        );
        assert_eq!(
            summary(&["old mode 100644", "new mode 100755"]).as_deref(),
            Some("File mode changed from 100644 to 100755")
        );
        assert_eq!(summary(&["new file mode 100644"]).as_deref(), Some("Empty file"));
        assert_eq!(parse_diff("").summary.as_deref(), Some("No changes to show"));
        assert_eq!(parse_diff(&patch(&["@@ -1 +1 @@", "-a", "+b"])).summary, None);
    }

    // Beyond diff.test.ts.

    #[test]
    fn joins_summaries_and_measures_in_utf16() {
        assert_eq!(
            parse_diff("diff --git a/a b/b\nold mode 100644\nnew mode 100755\nnew file mode 100755").summary,
            Some("File mode changed from 100644 to 100755. Empty file".into())
        );
        // An emoji is two units, so the change starts at 2.
        let parsed = parse_diff(&patch(&["@@ -1 +1 @@", "-😀a", "+😀b"]));
        assert_eq!(parsed.lines[1].changed, Some((2, 3)));
        // An empty range when one line is the other plus something.
        let parsed = parse_diff(&patch(&["@@ -1 +1 @@", "-ab", "+abc"]));
        assert_eq!(parsed.lines[1].changed, Some((2, 2)));
        assert_eq!(parsed.lines[2].changed, Some((2, 3)));
    }

    #[test]
    fn reads_only_ascii_digits_in_hunk_headers() {
        // Arabic-Indic digits are not `\d` in JavaScript.
        let parsed = parse_diff(&patch(&["@@ -١ +١ @@", "+x"]));
        assert!(parsed.lines.is_empty());
        let parsed = parse_diff(&patch(&["@@ -9 +98 @@", "+x"]));
        assert_eq!(parsed.max_line_no, 98);
        assert_eq!(parsed.number_width(), 2);
        assert_eq!(parse_diff(&patch(&["@@ -999 +1000 @@", "+x"])).number_width(), 4);
        assert_eq!(Context.sign(), " ");
        assert_eq!(Hunk.sign(), "");
    }
}
