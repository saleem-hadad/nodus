//! The branch and uncommitted changes (`gitStatus`, GIT-004…GIT-009).

use std::path::Path;
use std::sync::LazyLock;

use nodus_domain::{GitFileChange, GitFileStatus, GitStatus};
use regex::Regex;

use crate::collate::locale_compare;
use crate::runner::{DEFAULT_MAX_BUFFER, Git, RunError};

/// GIT-004.
pub const STATUS_ARGS: [&str; 5] = ["status", "--porcelain=v2", "--branch", "--untracked-files=all", "-z"];

// `\d` is ASCII in JavaScript without the `u` flag.
static AHEAD_BEHIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+([0-9]+) -([0-9]+)$").expect("a valid pattern"));

impl Git {
    /// Branch and uncommitted changes, or `None` when the folder isn't in a
    /// git repository (GIT-005). Fails when git can't be run, is stopped, or
    /// prints more than 32 MiB.
    pub async fn status(&self, repo_path: &Path) -> Result<Option<GitStatus>, RunError> {
        let res = self.run(repo_path, &STATUS_ARGS, DEFAULT_MAX_BUFFER).await?;
        if res.code != 0 {
            return Ok(None);
        }
        Ok(Some(parse_status(&res.stdout)))
    }
}

/// GIT-008: `A` is added; a `D` on either side is deleted; anything else
/// (including type changes) is modified.
fn status_of(xy: &str) -> GitFileStatus {
    let mut chars = xy.chars();
    let (x, y) = (chars.next(), chars.next());
    if x == Some('A') {
        GitFileStatus::Added
    } else if x == Some('D') || y == Some('D') {
        GitFileStatus::Deleted
    } else {
        GitFileStatus::Modified
    }
}

/// `Number(digits)` for the ahead/behind counts.
fn count(digits: &str) -> u32 {
    digits.parse().unwrap_or(u32::MAX)
}

/// Reads `git status --porcelain=v2 --branch -z` (GIT-006…GIT-009).
pub fn parse_status(stdout: &str) -> GitStatus {
    let mut status = GitStatus {
        branch: None,
        oid: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        files: Vec::new(),
    };
    let records: Vec<&str> = stdout.split('\0').collect();
    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;
        if rec.is_empty() {
            continue;
        }
        if let Some(header) = rec.strip_prefix("# ") {
            // GIT-006
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                "branch.oid" if value != "(initial)" => status.oid = Some(value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    if let Some(m) = AHEAD_BEHIND.captures(value) {
                        status.ahead = count(&m[1]);
                        status.behind = count(&m[2]);
                    }
                }
                _ => {}
            }
            continue;
        }

        // GIT-007: fields are space-separated; the path is last and may itself contain spaces.
        let parts: Vec<&str> = rec.split(' ').collect();
        let path_after = |n: usize| parts.get(n..).map(|rest| rest.join(" ")).unwrap_or_default();
        let change = match rec.chars().next() {
            Some('1') => Some(GitFileChange {
                path: path_after(8),
                old_path: None,
                status: status_of(parts.get(1).copied().unwrap_or_default()),
            }),
            Some('2') => {
                // A rename's original path is the next NUL-separated record.
                let old_path = records.get(i).map(|&s| s.to_owned());
                i += 1;
                Some(GitFileChange {
                    path: path_after(9),
                    old_path,
                    status: GitFileStatus::Renamed,
                })
            }
            Some('u') => Some(GitFileChange {
                path: path_after(10),
                old_path: None,
                status: GitFileStatus::Conflicted,
            }),
            Some('?') => Some(GitFileChange {
                path: nodus_domain::js::slice_utf16(rec, 2, usize::MAX),
                old_path: None,
                status: GitFileStatus::Untracked,
            }),
            _ => None,
        };
        if let Some(change) = change {
            status.files.push(change);
        }
    }
    // GIT-009
    status.files.sort_by(|a, b| locale_compare(&a.path, &b.path));
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, status: GitFileStatus) -> GitFileChange {
        GitFileChange {
            path: path.into(),
            old_path: None,
            status,
        }
    }

    #[test]
    fn reads_every_kind_of_record() {
        let out = [
            "# branch.oid 0123456789abcdef0123456789abcdef01234567",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -3",
            "# stash 4",
            "1 .M N... 100644 100644 100644 aaaa bbbb a.txt",
            "1 A. N... 000000 100644 100644 0000 cccc c.txt",
            "1 MD N... 100644 100644 000000 aaaa bbbb md.txt",
            "1 AD N... 000000 100644 000000 0000 cccc ad.txt",
            "1 .T N... 100644 120000 120000 aaaa bbbb link",
            "2 R. N... 100644 100644 100644 aaaa aaaa R100 new name.txt",
            "old name.txt",
            "u UU N... 100644 100644 100644 100644 a b c both ways.txt",
            "? dir/un tracked.txt",
            "! ignored.txt",
            "",
        ]
        .join("\0");
        let status = parse_status(&out);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.oid.as_deref(), Some("0123456789abcdef0123456789abcdef01234567"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 3));
        use GitFileStatus::*;
        let mut renamed = file("new name.txt", Renamed);
        renamed.old_path = Some("old name.txt".into());
        assert_eq!(
            status.files,
            [
                file("a.txt", Modified),
                file("ad.txt", Added),
                file("both ways.txt", Conflicted),
                file("c.txt", Added),
                file("dir/un tracked.txt", Untracked),
                file("link", Modified),
                file("md.txt", Deleted),
                renamed,
            ]
        );
    }

    #[test]
    fn reads_a_detached_head_and_a_repository_with_no_commits() {
        let detached = parse_status("# branch.oid abc\0# branch.head (detached)\0");
        assert_eq!((detached.branch, detached.oid.as_deref()), (None, Some("abc")));
        let fresh = parse_status("# branch.oid (initial)\0# branch.head main\0");
        assert_eq!((fresh.branch.as_deref(), fresh.oid), (Some("main"), None));
        assert_eq!(fresh.upstream, None);
        // A malformed ahead/behind count is ignored.
        let odd = parse_status("# branch.ab +1 -x\0");
        assert_eq!((odd.ahead, odd.behind), (0, 0));
    }
}
