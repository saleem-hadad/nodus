//! A card's own worktree in the app (SESS-217…221). Claude Code makes it,
//! given `--worktree <name>`: `<repository>/.claude/worktrees/<name>` on a
//! branch `worktree-<name>`, cut from where the main checkout is. A resume
//! goes back into it by itself, from the worktree its transcript records.
//! Its transcript is under the worktree's folder (SESS-211).
//!
//! A server makes its cards' worktrees itself (SRV-122), elsewhere, and
//! never passes `--worktree`.

use std::path::{Path, PathBuf};

use nodus_domain::Worktree;
use serde_json::Value;

/// The folder Claude Code keeps its worktrees in, under the repository.
const CLAUDE_DIR: &str = ".claude";
const WORKTREES_DIR: &str = "worktrees";

/// The worktree Claude Code makes for a card in the repository at `root`:
/// named `nodus-<first 8 of its id>` (SESS-217).
pub fn for_card(root: &Path, session_id: &str) -> Worktree {
    let name = format!("nodus-{}", session_id.chars().take(8).collect::<String>());
    let path = root.join(CLAUDE_DIR).join(WORKTREES_DIR).join(&name);
    Worktree {
        path: path.to_string_lossy().into_owned(),
        branch: format!("worktree-{name}"),
    }
}

/// Claude Code's name for a worktree it made: the folder's own name, when
/// that folder is in a `.claude/worktrees`. None for a server's (SRV-122).
pub fn claude_name(worktree: &Worktree) -> Option<String> {
    let path = Path::new(&worktree.path);
    let parent = path.parent()?;
    let in_claude = parent.file_name()? == WORKTREES_DIR && parent.parent()?.file_name()? == CLAUDE_DIR;
    in_claude.then(|| path.file_name()?.to_str().map(str::to_owned))?
}

/// The repository `folder` is in, as Claude Code finds it: the nearest
/// folder up from it holding a `.git` (a folder, or a file in a worktree).
pub fn git_root(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_owned)
}

/// Where Claude Code keeps whether it trusts a folder: `.claude.json` in
/// `CLAUDE_CONFIG_DIR` when that's set, else in the home folder.
pub fn claude_config(config_dir: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    match config_dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => home.map(|home| home.join(".claude.json")),
    }
}

/// Whether Claude Code trusts `folder`, a folder in the repository at
/// `root`, so `--worktree` works there (SESS-218): it has accepted its trust
/// dialog for `folder` or a folder above it, up to the repository's root.
/// Folders above the root don't count, as they don't for Claude Code.
pub fn is_trusted(config: &Value, folder: &Path, root: &Path) -> bool {
    let Some(projects) = config.get("projects").and_then(Value::as_object) else {
        return false;
    };
    let accepted = |dir: &Path| {
        dir.to_str()
            .and_then(|key| projects.get(key))
            .and_then(|project| project.get("hasTrustDialogAccepted"))
            == Some(&Value::Bool(true))
    };
    folder.ancestors().take_while(|dir| dir.starts_with(root)).any(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "0123abcd-0000-4000-8000-000000000001";

    #[test]
    fn names_a_cards_worktree_for_claude_code() {
        let worktree = for_card(Path::new("/code/shop"), ID);
        assert_eq!(
            worktree,
            Worktree {
                path: "/code/shop/.claude/worktrees/nodus-0123abcd".into(),
                branch: "worktree-nodus-0123abcd".into(),
            }
        );
        assert_eq!(claude_name(&worktree).as_deref(), Some("nodus-0123abcd"));
    }

    #[test]
    fn a_servers_worktree_isnt_claude_codes() {
        let worktree = Worktree {
            path: "/home/nodus-run/projects/shop/worktrees/0123abcd".into(),
            branch: "nodus/fix-login-0123".into(),
        };
        assert_eq!(claude_name(&worktree), None);
    }

    #[test]
    fn finds_the_repository_a_folder_is_in() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("shop");
        let inner = repo.join("web/src");
        std::fs::create_dir_all(&inner).unwrap();
        assert_eq!(git_root(&inner), None);
        std::fs::create_dir(repo.join(".git")).unwrap();
        assert_eq!(git_root(&inner), Some(repo.clone()));
        assert_eq!(git_root(&repo), Some(repo));
    }

    #[test]
    fn trusts_as_claude_code_does_up_to_the_repository() {
        let config = |key: &str, accepted: bool| json!({ "projects": { key: { "hasTrustDialogAccepted": accepted } } });
        let (root, web) = (Path::new("/code/shop"), Path::new("/code/shop/web"));
        assert!(is_trusted(&config("/code/shop", true), root, root));
        assert!(is_trusted(&config("/code/shop", true), web, root));
        assert!(is_trusted(&config("/code/shop/web", true), web, root));
        assert!(!is_trusted(&config("/code/shop/web", true), root, root));
        assert!(!is_trusted(&config("/code/shop", false), root, root));
        // A folder above the repository doesn't count.
        assert!(!is_trusted(&config("/code", true), root, root));
        assert!(!is_trusted(&json!({}), root, root));
    }

    #[test]
    fn finds_claude_codes_config() {
        let home = Path::new("/home/me");
        assert_eq!(
            claude_config(None, Some(home)),
            Some(PathBuf::from("/home/me/.claude.json"))
        );
        assert_eq!(
            claude_config(Some("/etc/claude"), Some(home)),
            Some(PathBuf::from("/etc/claude/.claude.json"))
        );
        assert_eq!(claude_config(Some(""), None), None);
    }
}
