//! `window.api.views`: the views on a board, what their pages ask for, and
//! their brains (`views:*` in `src/main/ipc.ts`), and the pages their frames
//! load (`nodus-view://`).

use nodus_domain::{BoardView, CommandResult as CommandRunResult, CreateViewOptions, ViewPatch, ViewReport};
use nodus_views::PageResponse;
use serde_json::Value;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.views`.
pub struct ViewsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl ViewsApi<'_> {
    /// `views:create` (VIEW-100): puts up a view for what the user
    /// described, and starts its brain on building it, once the login
    /// shell's environment is in. `Unknown project <id>`.
    pub async fn create(&self, project_id: &str, opts: &CreateViewOptions) -> CommandResult<BoardView> {
        self.nodus
            .on_core(async { self.nodus.inner.views.create(project_id, opts).await.on("views:create") })
            .await
    }

    /// `views:save` (VIEW-115, VIEW-116): moves, resizes or renames a view.
    pub async fn save(&self, project_id: &str, patch: &ViewPatch) -> CommandResult<()> {
        self.nodus
            .on_core(async { self.nodus.inner.views.save(project_id, patch).on("views:save") })
            .await
    }

    /// `views:remove` (VIEW-117): takes a view off the board, with its page,
    /// its data and its brain.
    pub async fn remove(&self, project_id: &str, view_id: &str) -> CommandResult<()> {
        self.nodus
            .on_core(async { self.nodus.inner.views.remove(project_id, view_id).on("views:remove") })
            .await
    }

    /// `views:call` (VIEW-060…071): a request from a view's page
    /// (`window.nodus`), answered for that view only.
    pub async fn call(&self, project_id: &str, view_id: &str, method: &str, args: &Value) -> CommandResult<Value> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .views
                    .call(project_id, view_id, method, args)
                    .await
                    .on("views:call")
            })
            .await
    }

    /// `views:report` (VIEW-119), fire and forget: a JavaScript error the
    /// page hit, or that the page at a version loaded. A view that has gone
    /// is ignored.
    pub fn report(&self, project_id: &str, view_id: &str, report: &ViewReport) {
        self.nodus.inner.views.report(project_id, view_id, report);
    }

    /// `views:approve` (VIEW-081, VIEW-082): the user allowing, or turning
    /// down, one of a view's commands.
    pub async fn approve(&self, project_id: &str, view_id: &str, name: &str, allow: bool) -> CommandResult<()> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .views
                    .approve(project_id, view_id, name, allow)
                    .on("views:approve")
            })
            .await
    }

    /// `views:refresh` (VIEW-084, VIEW-120): runs the view's commands now, and
    /// asks its brain to refresh it if it has a refresh prompt.
    pub async fn refresh(&self, project_id: &str, view_id: &str) -> CommandResult<Vec<CommandRunResult>> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .views
                    .refresh(project_id, view_id)
                    .await
                    .on("views:refresh")
            })
            .await
    }

    /// `views:ask` (VIEW-093, VIEW-120): passes what the user asked for on to
    /// the view's brain, starting one if it has none.
    pub async fn ask(&self, project_id: &str, view_id: &str, prompt: &str) -> CommandResult<()> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .views
                    .ask(project_id, view_id, prompt)
                    .await
                    .on("views:ask")
            })
            .await
    }

    /// `views:brain` (VIEW-092, VIEW-120): the view's brain, started idle if
    /// it has none yet, so its session can be opened.
    pub async fn brain(&self, project_id: &str, view_id: &str) -> CommandResult<String> {
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .views
                    .brain(project_id, view_id)
                    .await
                    .on("views:brain")
            })
            .await
    }

    /// The page a view's frame loads from `nodus-view://<id>/` (or WebView2's
    /// `http(s)://nodus-view.<id>/`): status, headers and body, for the
    /// webview's custom protocol (VIEW-018…021).
    pub fn serve_page(&self, url: &str) -> PageResponse {
        self.nodus.inner.views.serve_page(url)
    }
}
