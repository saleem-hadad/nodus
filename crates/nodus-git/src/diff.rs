//! One changed file's working-tree diff against HEAD (`gitDiff`,
//! GIT-014…GIT-020).

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use nodus_domain::{GitFileChange, GitFileDiff, GitFileStatus};
use nodus_platform::process::ExecError;

use crate::nodepath::{self, Flavor};
use crate::runner::{DEFAULT_MAX_BUFFER, Git, MAX_DIFF_BYTES, RunError};

/// Why a diff couldn't be read. The text is what the Changes modal shows
/// under `Couldn’t read this diff`.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    /// GIT-014.
    #[error("Not a git repository")]
    NotARepository,
    /// GIT-015: a path from the window that reaches outside the repository.
    #[error("Not in the repository: {0}")]
    NotInRepository(String),
    /// GIT-019.
    #[error("git diff failed for {0}")]
    Failed(String),
    /// git couldn't be run, or was stopped.
    #[error(transparent)]
    Run(#[from] RunError),
}

/// The empty tree's id, from the first repository that needed it, kept for
/// the process (GIT-016). A SHA-256 repository after a SHA-1 one reuses the
/// SHA-1 id, as 0.9.4 does.
static EMPTY_TREE: Mutex<Option<String>> = Mutex::new(None);

/// GIT-020: `/^(Binary files .* differ|GIT binary patch)$/m`, where a line
/// ends at any of JavaScript's line terminators.
pub fn is_binary_patch(patch: &str) -> bool {
    const START: &str = "Binary files ";
    const END: &str = " differ";
    patch.split(['\n', '\r', '\u{2028}', '\u{2029}']).any(|line| {
        line == "GIT binary patch"
            || (line.len() >= START.len() + END.len() && line.starts_with(START) && line.ends_with(END))
    })
}

/// GIT-015: a path resolved against the root has to land inside it, and not
/// on the root itself. A root-level name that starts with `..` (`..foo`)
/// counts as outside, as it does in 0.9.4.
pub fn in_repository(flavor: Flavor, root: &str, p: &str) -> bool {
    let rel = nodepath::relative_to(flavor, root, p);
    !(rel.is_empty() || rel.starts_with("..") || nodepath::is_absolute(flavor, &rel))
}

/// GIT-017: the arguments after git's prefix.
pub fn diff_args(file: &GitFileChange, base: &str) -> Vec<String> {
    let common = ["--no-color", "--no-ext-diff"];
    let mut args = vec!["diff".to_owned()];
    args.extend(common.map(str::to_owned));
    if file.status == GitFileStatus::Untracked {
        args.extend(["--no-index", "--", "/dev/null"].map(str::to_owned));
        args.push(file.path.clone());
    } else {
        args.extend(["-M".to_owned(), base.to_owned(), "--".to_owned()]);
        // An empty old path is falsy in JavaScript.
        if let Some(old) = file.old_path.as_deref().filter(|p| !p.is_empty()) {
            args.push(old.to_owned());
        }
        args.push(file.path.clone());
    }
    args
}

impl Git {
    /// HEAD, or the empty tree in a repository with no commits yet (GIT-016).
    async fn base_revision(&self, repo_path: &Path) -> Result<String, RunError> {
        let head = self
            .run(
                repo_path,
                &["rev-parse", "--verify", "--quiet", "HEAD"],
                DEFAULT_MAX_BUFFER,
            )
            .await?;
        if head.code == 0 {
            return Ok("HEAD".into());
        }
        let known = EMPTY_TREE.lock().unwrap_or_else(PoisonError::into_inner).clone();
        // An empty id is falsy, so it's asked for again next time.
        if let Some(tree) = known.filter(|t| !t.is_empty()) {
            return Ok(tree);
        }
        let res = self
            .run(
                repo_path,
                &["hash-object", "-t", "tree", "/dev/null"],
                DEFAULT_MAX_BUFFER,
            )
            .await?;
        let tree = nodus_domain::js::trim(&res.stdout).to_owned();
        *EMPTY_TREE.lock().unwrap_or_else(PoisonError::into_inner) = Some(tree.clone());
        Ok(tree)
    }

    /// The working-tree diff of one changed file against HEAD, staged changes
    /// included (GIT-014…GIT-020). `project_path` is the project's folder,
    /// which may sit inside the repository; the file's paths are relative to
    /// the repository's root.
    pub async fn diff(&self, project_path: &Path, file: &GitFileChange) -> Result<GitFileDiff, DiffError> {
        // GIT-014: status paths are relative to the repository root, which may sit above the project folder.
        let top = self
            .run(project_path, &["rev-parse", "--show-toplevel"], DEFAULT_MAX_BUFFER)
            .await?;
        if top.code != 0 {
            return Err(DiffError::NotARepository);
        }
        let root = nodus_domain::js::trim(&top.stdout).to_owned();

        // GIT-015: paths come from the window: never let one reach outside the repository.
        for p in [Some(file.path.as_str()), file.old_path.as_deref()]
            .into_iter()
            .flatten()
        {
            if !in_repository(Flavor::NATIVE, &root, p) {
                return Err(DiffError::NotInRepository(p.to_owned()));
            }
        }

        let base = if file.status == GitFileStatus::Untracked {
            String::new()
        } else {
            self.base_revision(Path::new(&root)).await?
        };
        let args = diff_args(file, &base);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let res = match self.run(Path::new(&root), &args, MAX_DIFF_BYTES).await {
            Ok(res) => res,
            // GIT-018
            Err(ExecError::MaxBuffer { .. }) => {
                return Ok(GitFileDiff {
                    patch: String::new(),
                    binary: None,
                    too_large: Some(true),
                });
            }
            Err(err) => return Err(err.into()),
        };
        // GIT-019: `diff --no-index` exits 1 when the files differ.
        if res.code != 0 && !(file.status == GitFileStatus::Untracked && res.code == 1) {
            return Err(DiffError::Failed(file.path.clone()));
        }
        // GIT-020
        if is_binary_patch(&res.stdout) {
            return Ok(GitFileDiff {
                patch: String::new(),
                binary: Some(true),
                too_large: None,
            });
        }
        Ok(GitFileDiff {
            patch: res.stdout,
            binary: None,
            too_large: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, old_path: Option<&str>, status: GitFileStatus) -> GitFileChange {
        GitFileChange {
            path: path.into(),
            old_path: old_path.map(Into::into),
            status,
        }
    }

    #[test]
    fn keeps_paths_inside_the_repository() {
        use Flavor::{Posix, Win32};
        assert!(in_repository(Posix, "/repo", "a.txt"));
        assert!(in_repository(Posix, "/repo", "dir/../a.txt"));
        assert!(!in_repository(Posix, "/repo", "."));
        assert!(!in_repository(Posix, "/repo", "../outside.txt"));
        assert!(!in_repository(Posix, "/repo", "/etc/hosts"));
        assert!(!in_repository(Posix, "/repo", "../../etc/passwd"));
        // A quirk kept from 0.9.4: a root-level name starting with `..`.
        assert!(!in_repository(Posix, "/repo", "..foo"));
        assert!(in_repository(Posix, "/repo", "dir/..foo"));
        assert!(in_repository(Win32, "C:/repo", "a.txt"));
        assert!(in_repository(Win32, "C:/repo", "A\\b.txt"));
        assert!(!in_repository(Win32, "C:/repo", "D:\\x"));
        assert!(!in_repository(Win32, "C:/repo", "\\etc\\hosts"));
        assert!(!in_repository(Win32, "C:/Repo", "c:\\repo"));
    }

    #[test]
    fn diffs_against_the_base_or_dev_null() {
        let untracked = change("new.txt", None, GitFileStatus::Untracked);
        assert_eq!(
            diff_args(&untracked, ""),
            [
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-index",
                "--",
                "/dev/null",
                "new.txt"
            ]
        );
        let renamed = change("b.txt", Some("a.txt"), GitFileStatus::Renamed);
        assert_eq!(
            diff_args(&renamed, "HEAD"),
            [
                "diff",
                "--no-color",
                "--no-ext-diff",
                "-M",
                "HEAD",
                "--",
                "a.txt",
                "b.txt"
            ]
        );
        let modified = change("a.txt", None, GitFileStatus::Modified);
        assert_eq!(
            diff_args(&modified, "4b825dc"),
            ["diff", "--no-color", "--no-ext-diff", "-M", "4b825dc", "--", "a.txt"]
        );
    }

    #[test]
    fn tells_binary_patches() {
        assert!(is_binary_patch("diff --git a/x b/x\nBinary files a/x and b/x differ\n"));
        assert!(is_binary_patch("index 1..2\nGIT binary patch\nliteral 3\n"));
        assert!(is_binary_patch("x\rBinary files  differ\r\n"));
        assert!(!is_binary_patch("+Binary files a/x and b/x differ\n"));
        assert!(!is_binary_patch("Binary files a\nb differ\n"));
        assert!(!is_binary_patch("Binary files differ"));
        assert!(!is_binary_patch("GIT binary patch "));
    }
}
