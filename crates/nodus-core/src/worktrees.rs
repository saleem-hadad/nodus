//! A worktree per card in the app (SESS-217…221). A project with
//! `worktrees` on gives each new Claude card a worktree of its own, which
//! Claude Code makes when the card's process starts (`--worktree`,
//! [`nodus_sessions::worktree`]), so sessions side by side never write over
//! each other's files. Removing the card removes its worktree, its
//! uncommitted changes saved first, as a server's are (SRV-124).
//!
//! A server makes its cards' worktrees itself (`hosting.rs`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nodus_domain::{BoardNode, Project, SessionKind, Worktree};
use nodus_platform::Env;
use nodus_sessions::worktree;

use crate::hosting::{GIT_TIMEOUT, SAVED_DAYS, time_stamp};
use crate::nodus::Inner;

impl Inner {
    /// SESS-217, SESS-218: a new Claude card's own worktree, by the name
    /// Claude Code makes it under, recorded on the card. None for a project
    /// without worktrees, a terminal, an action run, a folder outside a git
    /// repository, or one Claude Code doesn't trust yet: that card starts in
    /// the project's folder, where Claude Code asks about trust as it would.
    pub(crate) async fn claim_worktree(&self, project: &Project, node: &BoardNode, run: bool) -> Option<String> {
        if self.hosted().is_some() || project.worktrees != Some(true) || run || node.kind == Some(SessionKind::Terminal)
        {
            return None;
        }
        let folder = PathBuf::from(&project.repo_path);
        let root = worktree::git_root(&folder)?;
        let env = self.env().await;
        if !claude_trusts(&env, &folder, &root).await {
            tracing::info!("[worktree] Claude Code doesn't trust {} yet", root.display());
            return None;
        }
        let made = worktree::for_card(&root, &node.session_id);
        let name = worktree::claude_name(&made);
        self.store.update(|state| {
            if let Some(node) = state.board_mut(&project.id).nodes.get_mut(&node.session_id) {
                node.set_worktree(Some(&made));
            }
        });
        name
    }

    /// SESS-220: a card that left the board takes the worktree Claude Code
    /// made for it along. Uncommitted changes are saved first in
    /// `<userData>/saved/<cardId>-<time>/changes.patch`; its branch stays
    /// while it has commits no other branch or remote has. When saving
    /// fails, the worktree stays. Saved changes older than 30 days go.
    pub(crate) async fn retire_claude_worktree(&self, project: &Project, node: &BoardNode) {
        let Some(worktree) = node.worktree().filter(|w| worktree::claude_name(w).is_some()) else {
            return;
        };
        let mut env = self.env().await.spawn_env();
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        let path = PathBuf::from(&worktree.path);
        let main = PathBuf::from(&project.repo_path);
        let saved = self.user_data.join("saved");
        if path.exists() {
            if let Err(err) = self.save_changes(&env, &saved, &path, &node.session_id).await {
                tracing::error!("[worktree] couldn't save {}: {err}", worktree.path);
                return;
            }
            // Twice forced: Claude Code locks the worktrees it makes.
            let removed = self
                .git_ok(
                    &env,
                    &main,
                    &["worktree", "remove", "--force", "--force", &worktree.path],
                    GIT_TIMEOUT,
                )
                .await;
            if let Err(err) = removed {
                tracing::error!("[worktree] couldn't remove {}: {err}", worktree.path);
            }
        }
        let _ = self.git_run(&env, &main, &["worktree", "prune"], GIT_TIMEOUT).await;
        if self.only_on_branch(&env, &main, &worktree).await == Some(0) {
            let _ = self
                .git_run(&env, &main, &["branch", "-D", &worktree.branch], GIT_TIMEOUT)
                .await;
        }
        prune_saved(&saved);
    }

    /// The worktree's uncommitted changes, new files too, as a patch in a
    /// folder of their own under `saved`. Nothing when there are none.
    async fn save_changes(
        &self,
        env: &nodus_platform::env::EnvMap,
        saved: &Path,
        path: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let status = self
            .git_ok(
                env,
                path,
                &["status", "--porcelain", "--untracked-files=all"],
                GIT_TIMEOUT,
            )
            .await?;
        if status.trim().is_empty() {
            return Ok(());
        }
        let dir = saved.join(format!("{session_id}-{}", time_stamp()));
        std::fs::create_dir_all(&dir).map_err(|err| format!("Can't make {}: {err}", dir.display()))?;
        // Staged here, since the worktree goes next.
        self.git_ok(env, path, &["add", "-A"], GIT_TIMEOUT).await?;
        let patch = format!("--output={}", dir.join("changes.patch").display());
        self.git_ok(
            env,
            path,
            &["diff", "--binary", "--cached", "HEAD", &patch],
            GIT_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// How many commits only the worktree's branch has: none on another
    /// branch or a remote. None when git can't tell.
    async fn only_on_branch(&self, env: &nodus_platform::env::EnvMap, main: &Path, worktree: &Worktree) -> Option<u64> {
        // As `--branches` names them: without `refs/heads/`.
        let own = format!("--exclude={}", worktree.branch);
        let count = self
            .git_ok(
                env,
                main,
                &[
                    "rev-list",
                    "--count",
                    &worktree.branch,
                    "--not",
                    &own,
                    "--branches",
                    "--remotes",
                ],
                GIT_TIMEOUT,
            )
            .await
            .ok()?;
        count.trim().parse().ok()
    }
}

/// Whether Claude Code trusts `folder` (SESS-218), from its `.claude.json`,
/// for the folder as the project names it or as the disk does.
async fn claude_trusts(env: &Env, folder: &Path, root: &Path) -> bool {
    let home = env.var("HOME").map(PathBuf::from).or_else(std::env::home_dir);
    let Some(config) = worktree::claude_config(env.var("CLAUDE_CONFIG_DIR"), home.as_deref()) else {
        return false;
    };
    let Ok(text) = tokio::fs::read_to_string(&config).await else {
        return false;
    };
    let Ok(config) = serde_json::from_str(&text) else {
        return false;
    };
    if worktree::is_trusted(&config, folder, root) {
        return true;
    }
    match (std::fs::canonicalize(folder), std::fs::canonicalize(root)) {
        (Ok(folder), Ok(root)) => worktree::is_trusted(&config, &folder, &root),
        _ => false,
    }
}

/// Saved changes older than 30 days go (SESS-220).
fn prune_saved(saved: &Path) {
    let Ok(entries) = std::fs::read_dir(saved) else {
        return;
    };
    let keep = Duration::from_secs(u64::from(SAVED_DAYS) * 86_400);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > keep);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
