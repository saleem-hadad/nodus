//! Words to expect (VOICE-012): the project's name, its branch and the files
//! it has changed, so code names like "BoardView" come back spelled as in the
//! repository rather than as "board view".

use std::path::Path;

use nodus_domain::GitStatus;
use nodus_domain::js::{trim, utf16_len};
use nodus_git::Git;
use nodus_git::nodepath::{Flavor, parse_name};

/// xAI takes up to 100 hint terms…
pub const MAX_KEYTERMS: usize = 100;
/// …of up to 50 characters each.
pub const MAX_KEYTERM_CHARS: usize = 50;
/// Shorter terms aren't worth sending.
pub const MIN_KEYTERM_CHARS: usize = 3;

/// The hint words for a project, from its git status (`None` when the
/// folder isn't a repository or git failed).
pub fn keyterms_from(project_name: &str, status: Option<&GitStatus>) -> Vec<String> {
    let mut terms = vec![project_name.to_owned()];
    if let Some(status) = status {
        // An empty branch is falsy in JavaScript.
        if let Some(branch) = status.branch.as_deref().filter(|b| !b.is_empty()) {
            terms.push(branch.to_owned());
        }
        // PD-8: Node's platform `path.parse`.
        terms.extend(status.files.iter().map(|f| parse_name(Flavor::NATIVE, &f.path)));
    }
    let mut out: Vec<String> = Vec::new();
    for term in &terms {
        let term = trim(term);
        let len = utf16_len(term);
        if (MIN_KEYTERM_CHARS..=MAX_KEYTERM_CHARS).contains(&len) && !out.iter().any(|t| t == term) {
            out.push(term.to_owned());
        }
    }
    out.truncate(MAX_KEYTERMS);
    out
}

/// VOICE-012: reads the project's git status (GIT-004) and builds its hint
/// words; a status that fails gives just the project's name.
pub async fn keyterms(git: &Git, project_name: &str, repo_path: &Path) -> Vec<String> {
    let status = git.status(repo_path).await.ok().flatten();
    keyterms_from(project_name, status.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{GitFileChange, GitFileStatus};

    fn status(branch: Option<&str>, files: &[&str]) -> GitStatus {
        GitStatus {
            branch: branch.map(Into::into),
            oid: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            files: files
                .iter()
                .map(|p| GitFileChange {
                    path: (*p).into(),
                    old_path: None,
                    status: GitFileStatus::Modified,
                })
                .collect(),
        }
    }

    #[test]
    fn builds_hint_words_from_the_repository() {
        let s = status(
            Some("feature/voice"),
            &[
                "src/BoardView.tsx",
                "src/board/BoardView.css",
                "a.ts",
                ".gitignore",
                "README.md",
            ],
        );
        assert_eq!(
            keyterms_from("  Nodus Demo ", Some(&s)),
            ["Nodus Demo", "feature/voice", "BoardView", ".gitignore", "README"]
        );
        // Not a repository: just the name, if it's long enough.
        assert_eq!(keyterms_from("Demo", None), ["Demo"]);
        assert!(keyterms_from("ab", None).is_empty());
        // Detached: no branch.
        assert_eq!(
            keyterms_from("Demo", Some(&status(None, &["main.rs"]))),
            ["Demo", "main"]
        );
    }

    #[test]
    fn keeps_terms_xai_takes() {
        let long = "x".repeat(51);
        let fifty = "y".repeat(50);
        let emoji = "😀a"; // three UTF-16 units
        let files: Vec<String> = (0..150).map(|i| format!("dir/file{i:03}.rs")).collect();
        let mut paths: Vec<&str> = vec![&long, &fifty, emoji];
        paths.extend(files.iter().map(String::as_str));
        let terms = keyterms_from("Demo", Some(&status(Some("main"), &paths)));
        assert_eq!(terms.len(), MAX_KEYTERMS);
        assert_eq!(&terms[..4], ["Demo", "main", fifty.as_str(), emoji]);
        assert_eq!(terms[4], "file000");
        assert_eq!(terms[99], "file095");
    }
}
