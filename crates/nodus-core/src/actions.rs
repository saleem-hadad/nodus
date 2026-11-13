//! `window.api.actions`: the actions rail's buttons, a project's own and the
//! global ones, and Claude drafting one (`actions:*` in `src/main/ipc.ts`).
//! An action runs through `sessions.create` with its `action_id`.

use std::path::PathBuf;

use nodus_domain::{ActionDraft, ActionLists, ProjectAction};
use nodus_remote::Request;
use serde_json::Value;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.actions`.
pub struct ActionsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl ActionsApi<'_> {
    /// `actions:global` (ACT-013): the actions shown in every project.
    pub async fn global(&self) -> Vec<ProjectAction> {
        self.nodus.on_core(async { self.nodus.inner.actions.global() }).await
    }

    /// `actions:save` (ACT-001…ACT-015): adds an action, or replaces the one
    /// with its id, to the project or app-wide when it's global, moving it
    /// out of the other list. `raw` is what the window sent, checked and
    /// trimmed as 0.9.4 does: `An action needs a prompt`, `Unknown project <id>`.
    pub async fn save(&self, project_id: &str, raw: &Value) -> CommandResult<ActionLists> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, action) = (project_id.to_owned(), raw.clone());
            return route.call(Request::ActionsSave { project_id, action }).await;
        }
        self.nodus
            .on_core(async { self.nodus.inner.actions.save(project_id, raw).on("actions:save") })
            .await
    }

    /// `actions:remove` (ACT-015): deletes one of the project's actions, or a
    /// global one from every project.
    pub async fn remove(&self, project_id: &str, action_id: &str) -> CommandResult<ActionLists> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, action_id) = (project_id.to_owned(), action_id.to_owned());
            return route.call(Request::ActionsRemove { project_id, action_id }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .actions
                    .remove(project_id, action_id)
                    .on("actions:remove")
            })
            .await
    }

    /// `actions:draft` (ACT-016…ACT-027): Claude turns a description into a
    /// name, icon and prompt, in the project's folder, once the login
    /// shell's environment is in. A global one fits any repository.
    pub async fn draft(&self, project_id: &str, description: &str, global: bool) -> CommandResult<ActionDraft> {
        if let Some(route) = self.nodus.route_project(project_id) {
            return route
                .call(Request::ActionsDraft {
                    project_id: project_id.to_owned(),
                    description: description.to_owned(),
                    global,
                })
                .await;
        }
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "actions:draft";
                let env = self.nodus.env().await;
                let repo = self
                    .nodus
                    .inner
                    .store
                    .read(|state| state.project(project_id).map(|p| PathBuf::from(&p.repo_path)))
                    .on(CHANNEL)?;
                nodus_actions::draft_action(&*self.nodus.inner.runner, &env, &repo, description, global)
                    .await
                    .on(CHANNEL)
            })
            .await
    }
}
