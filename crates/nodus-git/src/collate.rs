//! `a.localeCompare(b)`, which git.ts sorts changed files with (GIT-009).
//!
//! V8 compares with ICU's collator for the default locale at the default
//! strength (tertiary: case and accents count, but only after the letters
//! do), punctuation not ignored and numbers compared digit by digit. The
//! Electron main process's default locale is the system's; this uses ICU4X's
//! root collation, which is what `en` and most other locales use. A few
//! locales order some letters differently (Swedish puts `ä` after `z`); paths
//! with those sort as in English here.
//!
//! So `B.txt` sorts between `a.txt` and `c.txt`, `a.txt` before `A.txt`, and
//! `_x` before `a` and `1`, as JavaScript sorts them, not as bytes do.

use std::cmp::Ordering;
use std::sync::LazyLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};

static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(Default::default(), CollatorOptions::default()).expect("the root collation is compiled in")
});

/// `a.localeCompare(b)`, as an ordering.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    COLLATOR.compare(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut items: Vec<&str>) -> Vec<&str> {
        items.sort_by(|a, b| locale_compare(a, b));
        items
    }

    #[test]
    fn sorts_as_javascript_does() {
        // Checked against `[…].sort((a, b) => a.localeCompare(b))` in Node 22.
        assert_eq!(sorted(vec!["c.txt", "B.txt", "a.txt"]), ["a.txt", "B.txt", "c.txt"]);
        assert_eq!(sorted(vec!["A.txt", "a.txt"]), ["a.txt", "A.txt"]);
        assert_eq!(
            sorted(vec!["new name.txt", "gone.txt", "dir/un tracked.txt", "c.txt", "a.txt"]),
            ["a.txt", "c.txt", "dir/un tracked.txt", "gone.txt", "new name.txt"]
        );
        assert_eq!(sorted(vec!["b", "_x", "1", "a"]), ["_x", "1", "a", "b"]);
        assert_eq!(sorted(vec!["é", "f", "e"]), ["e", "é", "f"]);
        assert_eq!(
            sorted(vec!["src/z.ts", "src-a/a.ts", "src/a.ts"]),
            ["src-a/a.ts", "src/a.ts", "src/z.ts"]
        );
        assert_eq!(sorted(vec!["file10", "file2", "file1"]), ["file1", "file10", "file2"]);
    }
}
