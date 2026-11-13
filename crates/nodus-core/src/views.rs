//! `window.api.views`: the views on a board, what their pages ask for, and
//! their brains (`views:*` in `src/main/ipc.ts`), and the pages their frames
//! load (`nodus-view://`).

use nodus_domain::{BoardView, CommandResult as CommandRunResult, CreateViewOptions, CreatedBy, ViewPatch, ViewReport};
use nodus_remote::Request;
use nodus_views::PageResponse;
use serde_json::Value;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.views`.
pub struct ViewsApi<'a> {
    pub(crate) nodus: &'a Nodus,
    /// The member a server makes these calls for (SRV-183, [`Nodus::by`]).
    pub(crate) by: Option<&'a str>,
}

impl ViewsApi<'_> {
    /// `views:create` (VIEW-100): puts up a view for what the user
    /// described, and starts its brain on building it, once the login
    /// shell's environment is in. `Unknown project <id>`.
    pub async fn create(&self, project_id: &str, opts: &CreateViewOptions) -> CommandResult<BoardView> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let view: BoardView = route
                .call(Request::ViewsCreate {
                    project_id: project_id.to_owned(),
                    opts: opts.clone(),
                })
                .await?;
            if let Some(brain) = &view.session_id {
                route.remember(brain);
            }
            return Ok(view);
        }
        self.nodus
            .on_core(async {
                let mut view = self
                    .nodus
                    .inner
                    .views
                    .create(project_id, opts)
                    .await
                    .on("views:create")?;
                if let Some(by) = self.by {
                    // SRV-183: the view, and its brain, are the member's.
                    view.set_created_by(Some(by));
                    self.nodus.inner.store.update(|state| {
                        let board = state.board_mut(project_id);
                        if let Some(stored) = board.views.get_mut(&view.id) {
                            stored.set_created_by(Some(by));
                        }
                        let brain = view.session_id.as_deref().and_then(|id| board.nodes.get_mut(id));
                        if let Some(brain) = brain {
                            brain.set_created_by(Some(by));
                        }
                    });
                }
                Ok(view)
            })
            .await
    }

    /// `views:save` (VIEW-115, VIEW-116): moves, resizes or renames a view.
    pub async fn save(&self, project_id: &str, patch: &ViewPatch) -> CommandResult<()> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, patch) = (project_id.to_owned(), patch.clone());
            return route.call(Request::ViewsSave { project_id, patch }).await;
        }
        self.nodus
            .on_core(async { self.nodus.inner.views.save(project_id, patch).on("views:save") })
            .await
    }

    /// `views:remove` (VIEW-117): takes a view off the board, with its page,
    /// its data and its brain.
    pub async fn remove(&self, project_id: &str, view_id: &str) -> CommandResult<()> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, view_id) = (project_id.to_owned(), view_id.to_owned());
            return route.call(Request::ViewsRemove { project_id, view_id }).await;
        }
        self.nodus
            .on_core(async { self.nodus.inner.views.remove(project_id, view_id).on("views:remove") })
            .await
    }

    /// `views:call` (VIEW-060…071): a request from a view's page
    /// (`window.nodus`), answered for that view only.
    /// A server view's calls go only to its server (SRV-163).
    pub async fn call(&self, project_id: &str, view_id: &str, method: &str, args: &Value) -> CommandResult<Value> {
        if let Some(route) = self.nodus.route_project(project_id) {
            return route
                .call(Request::ViewsCall {
                    project_id: project_id.to_owned(),
                    view_id: view_id.to_owned(),
                    method: method.to_owned(),
                    args: args.clone(),
                })
                .await;
        }
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
        if let Some(route) = self.nodus.route_project(project_id) {
            return route.send(&Request::ViewsReport {
                project_id: project_id.to_owned(),
                view_id: view_id.to_owned(),
                report: report.clone(),
            });
        }
        self.nodus.inner.views.report(project_id, view_id, report);
    }

    /// `views:approve` (VIEW-081, VIEW-082): the user allowing, or turning
    /// down, one of a view's commands.
    pub async fn approve(&self, project_id: &str, view_id: &str, name: &str, allow: bool) -> CommandResult<()> {
        if let Some(route) = self.nodus.route_project(project_id) {
            return route
                .call(Request::ViewsApprove {
                    project_id: project_id.to_owned(),
                    view_id: view_id.to_owned(),
                    name: name.to_owned(),
                    allow,
                })
                .await;
        }
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .views
                    .approve(project_id, view_id, name, allow)
                    .on("views:approve")?;
                // SRV-145: the member who allowed it, on a server.
                if let Some(by) = self.by {
                    self.nodus.inner.store.update(|state| {
                        let command = state
                            .board_mut(project_id)
                            .views
                            .get_mut(view_id)
                            .and_then(|view| view.commands.as_mut()?.iter_mut().find(|c| c.name == name));
                        if let Some(command) = command {
                            command.set_approved_by(allow.then_some(by));
                        }
                    });
                }
                Ok(())
            })
            .await
    }

    /// The command line a view's command runs, as the board has it now.
    pub fn command_line(&self, project_id: &str, view_id: &str, name: &str) -> Option<String> {
        self.nodus.inner.store.read(|state| {
            let view = state.board(project_id)?.views.get(view_id)?;
            view.commands
                .iter()
                .flatten()
                .find(|c| c.name == name)
                .map(|c| c.run.clone())
        })
    }

    /// `views:refresh` (VIEW-084, VIEW-120): runs the view's commands now, and
    /// asks its brain to refresh it if it has a refresh prompt.
    pub async fn refresh(&self, project_id: &str, view_id: &str) -> CommandResult<Vec<CommandRunResult>> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, view_id) = (project_id.to_owned(), view_id.to_owned());
            return route.call(Request::ViewsRefresh { project_id, view_id }).await;
        }
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
        if let Some(route) = self.nodus.route_project(project_id) {
            return route
                .call(Request::ViewsAsk {
                    project_id: project_id.to_owned(),
                    view_id: view_id.to_owned(),
                    prompt: prompt.to_owned(),
                })
                .await;
        }
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
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, view_id) = (project_id.to_owned(), view_id.to_owned());
            let brain: String = route.call(Request::ViewsBrain { project_id, view_id }).await?;
            route.remember(&brain);
            return Ok(brain);
        }
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
    /// webview's custom protocol (VIEW-018…021). A view on a server board
    /// is that server's page, never the local core's (SRV-163).
    pub fn serve_page(&self, url: &str) -> PageResponse {
        if let Some(view_id) = nodus_views::view_id_from_url(url)
            && let Some(server_id) = self.nodus.view_server(view_id)
        {
            return self.nodus.server_page(&server_id, view_id);
        }
        self.nodus.inner.views.serve_page(url)
    }
}
