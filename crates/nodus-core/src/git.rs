//! `window.api.git`: the project folder's branch and changes, a file's diff,
//! and the branch's newest commits (`git:*` in `src/main/ipc.ts`). Each waits
//! for the login shell's environment, so git is the one on the user's PATH
//! (GIT-003).

use std::path::PathBuf;

use nodus_domain::{GitCommit, GitFileChange, GitFileDiff, GitStatus};
use nodus_git::{Git, clamp_limit};
use nodus_remote::Request;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.git`.
pub struct GitApi<'a> {
    pub(crate) nodus: &'a Nodus,
    /// The card it's for: on a server, its worktree's git, not the main
    /// checkout's (SRV-128). None for the project folder, as in the app.
    pub(crate) session_id: Option<&'a str>,
}

impl GitApi<'_> {
    /// `git:status` (GIT-004…GIT-010): the branch and uncommitted changes,
    /// none when the folder isn't in a git repository. Fails when git can't
    /// run; the window reads that as none too (GIT-021).
    pub async fn status(&self, project_id: &str) -> CommandResult<Option<GitStatus>> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, session_id) = (project_id.to_owned(), self.session_id.map(str::to_owned));
            return route.call(Request::GitStatus { project_id, session_id }).await;
        }
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "git:status";
                let (git, repo) = self.git(project_id, CHANNEL).await?;
                git.status(&repo).await.on(CHANNEL)
            })
            .await
    }

    /// `git:diff` (GIT-014…GIT-020): one changed file against HEAD.
    pub async fn diff(&self, project_id: &str, file: &GitFileChange) -> CommandResult<GitFileDiff> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, file) = (project_id.to_owned(), file.clone());
            let session_id = self.session_id.map(str::to_owned);
            return route
                .call(Request::GitDiff {
                    project_id,
                    file,
                    session_id,
                })
                .await;
        }
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "git:diff";
                let (git, repo) = self.git(project_id, CHANNEL).await?;
                git.diff(&repo, file).await.on(CHANNEL)
            })
            .await
    }

    /// `git:log` (GIT-011…GIT-013): the newest commits on the branch, newest
    /// first. A whole-number `limit` is kept to 1–50; anything else is 5.
    pub async fn log(&self, project_id: &str, limit: Option<f64>) -> CommandResult<Vec<GitCommit>> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, session_id) = (project_id.to_owned(), self.session_id.map(str::to_owned));
            return route
                .call(Request::GitLog {
                    project_id,
                    limit,
                    session_id,
                })
                .await;
        }
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "git:log";
                let (git, repo) = self.git(project_id, CHANNEL).await?;
                git.log(&repo, clamp_limit(limit)).await.on(CHANNEL)
            })
            .await
    }

    /// GIT-003: the env first, then the project (`Unknown project <id>`).
    /// For a card with a worktree, its worktree (SRV-128).
    async fn git(&self, project_id: &str, channel: &'static str) -> CommandResult<(Git, PathBuf)> {
        let env = self.nodus.env().await;
        let session_id = self.session_id;
        let repo = self
            .nodus
            .inner
            .store
            .read(|state| {
                let project = state.project(project_id)?;
                let worktree = session_id
                    .and_then(|id| state.board(project_id)?.nodes.get(id))
                    .and_then(nodus_domain::BoardNode::worktree);
                Ok::<_, nodus_store::NotFound>(PathBuf::from(
                    worktree.map_or_else(|| project.repo_path.clone(), |w| w.path),
                ))
            })
            .on(channel)?;
        Ok((self.nodus.inner.git(&env), repo))
    }
}
