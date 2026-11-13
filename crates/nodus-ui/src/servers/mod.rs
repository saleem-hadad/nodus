//! Nodus servers in the app (SRV-060, SRV-161, SRV-165): **Connect to
//! Server…**, which asks for an address, then sets the server up or signs
//! in with GitHub ([`connect`]), and **Show Server Projects…**, which ticks
//! the server's projects in or out of the grid. Both open from the grid's
//! New Project menu and from Settings → Servers. That menu also has **Add
//! from GitHub on <server>…** (SRV-120), which clones one of the server's
//! GitHub App's repositories as a project.

mod connect;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui::component::input::{Escape, IndentInline, InputEvent, InputState, OutdentInline};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Corners, Entity, FocusHandle, Focusable as _, FontWeight, Global, InteractiveElement,
    IntoElement, KeyBinding, KeyDownEvent, ObjectFit, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, StyledImage as _, Window, actions, div, img, px,
};
use nodus_core::GithubRepo;
use nodus_core::hosted::Member;
use nodus_domain::{Project, ServerInfo, ServerState};

use connect::Step;

use crate::components::button::button;
use crate::components::focus_ring::focus_ring;
use crate::components::folder_icon::folder_icon;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::text_field::{focus_all, input, new_input};
use crate::state::{AppState, ServerDialog};
use crate::theme::tokens::*;
use crate::theme::{self};

actions!(servers, [NextField, PreviousField]);

/// The dialog's key context: Tab goes between its fields, not out of it.
const KEY_CONTEXT: &str = "ServerDialog";

/// Tab and Shift+Tab between the address and the token.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextField, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", PreviousField, Some(KEY_CONTEXT)),
    ]);
}

/// How a server's status reads in Settings and the picker.
pub fn status_text(server: &ServerInfo) -> String {
    match server.state {
        ServerState::Connecting => "Connecting…".into(),
        ServerState::Connected => "Connected".into(),
        ServerState::Offline => match &server.error {
            Some(error) => format!("Offline: {error}"),
            None => "Offline".into(),
        },
        ServerState::ProtocolMismatch => format!(
            "This server runs Nodus {}. Update it in Settings → Servers.",
            server.version.as_deref().unwrap_or("of another version")
        ),
    }
}

/// A server's projects, as the picker has them.
enum Listing {
    Loading,
    Loaded(Vec<Project>),
    Failed(String),
}

/// A server's GitHub repositories, as Add from GitHub has them (SRV-120).
enum Repos {
    Loading,
    Loaded(Vec<GithubRepo>),
    Failed(String),
}

/// Whether a repository matches what's typed in the search: every word
/// somewhere in its name, ignoring case.
pub fn repo_matches(full_name: &str, query: &str) -> bool {
    let name = full_name.to_lowercase();
    query.split_whitespace().all(|word| name.contains(&word.to_lowercase()))
}

pub struct ServersDialog {
    state: Entity<AppState>,
    /// What had focus before, to give it back on close.
    pub previous_focus: Option<FocusHandle>,
    panel: FocusHandle,
    /// Where Connect to Server… is.
    step: Step,
    address: Entity<InputState>,
    token: Entity<InputState>,
    /// The setup code, the server's name, the organization that owns its
    /// App, and a GitHub login to invite.
    code: Entity<InputState>,
    name: Entity<InputState>,
    org: Entity<InputState>,
    invite: Entity<InputState>,
    /// The App goes under an organization rather than the person's account.
    for_org: bool,
    /// "Let sessions change GitHub Actions workflows" (SRV-062).
    workflows: bool,
    /// GitHub's page that makes the App, once opened.
    app_page: Option<String>,
    /// Counts steps, so a poll of a step left behind stops.
    poll: u64,
    /// The sign-in code was copied.
    copied: bool,
    /// The members so far, on the wizard's Members step.
    members: Vec<Member>,
    /// Avatars by address: the file once fetched, none until then.
    avatars: HashMap<String, Option<PathBuf>>,
    /// Empties the invite field on the next draw, and focuses it.
    clear_invite: bool,
    focus_invite: bool,
    /// Something is being asked of the server.
    connecting: bool,
    /// Why the last step failed, as the core said it.
    error: Option<String>,
    /// Each server's projects, once asked for.
    listings: HashMap<String, Listing>,
    /// Projects being linked or unlinked.
    busy: HashSet<String>,
    /// Why the last link or unlink failed.
    link_error: Option<String>,
    /// Add from GitHub's search, the repositories by server, the page that
    /// installs the App on more, the one being added, and why adding failed.
    search: Entity<InputState>,
    repos: HashMap<String, Repos>,
    install_url: HashMap<String, String>,
    adding: Option<String>,
    add_error: Option<String>,
    serial: usize,
}

impl ServersDialog {
    pub fn new(
        state: Entity<AppState>,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let panel = cx.focus_handle();
        window.focus(&panel, cx);
        let address = new_input("", "nodus-box, 100.101.102.103 or 127.0.0.1:7420", false, window, cx);
        let token = new_input("", "The server’s token", true, window, cx);
        let code = new_input("", "K7QM-4RXD", false, window, cx);
        let name = new_input("", "Box", false, window, cx);
        let org = new_input("", "Organization’s GitHub name", false, window, cx);
        let invite = new_input("", "octocat", false, window, cx);
        for field in [&address, &token, &code, &name, &org, &invite] {
            cx.subscribe_in(field, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    this.primary(window, cx);
                }
                cx.notify();
            })
            .detach();
        }
        let search = new_input("", "Search repositories", false, window, cx);
        cx.subscribe_in(&search, window, |_, _, _: &InputEvent, _, cx| cx.notify())
            .detach();
        match state.read(cx).server_dialog {
            Some(ServerDialog::Connect) => focus_all(&address, window, cx),
            Some(ServerDialog::Github { .. }) => focus_all(&search, window, cx),
            _ => {}
        }
        // Servers coming and going, and projects linked, redraw it.
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            state,
            previous_focus,
            panel,
            step: Step::Address,
            address,
            token,
            code,
            name,
            org,
            invite,
            for_org: false,
            workflows: false,
            app_page: None,
            poll: 0,
            copied: false,
            members: Vec::new(),
            avatars: HashMap::new(),
            clear_invite: false,
            focus_invite: false,
            connecting: false,
            error: None,
            listings: HashMap::new(),
            busy: HashSet::new(),
            link_error: None,
            search,
            repos: HashMap::new(),
            install_url: HashMap::new(),
            adding: None,
            add_error: None,
            serial: 1,
        }
    }

    /// SRV-120: asks the server for its repositories once, and where the
    /// App is installed on more.
    fn load_repos(&mut self, server_id: &str, cx: &mut Context<'_, Self>) {
        if self.repos.contains_key(server_id) {
            return;
        }
        self.repos.insert(server_id.to_owned(), Repos::Loading);
        let nodus = self.state.read(cx).nodus.clone();
        let id = server_id.to_owned();
        cx.spawn(async move |this, cx| {
            let listed = nodus.servers().github_repos(&id).await;
            let install = nodus.servers().github_install_url(&id).await;
            let _ = this.update(cx, |this, cx| {
                let repos = match listed {
                    Ok(repos) => Repos::Loaded(repos),
                    Err(err) => Repos::Failed(err.message),
                };
                this.repos.insert(id.clone(), repos);
                if let Ok(Some(url)) = install {
                    this.install_url.insert(id, url);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// SRV-120: clones the repository on the server, puts it in the grid,
    /// and opens its board.
    fn add_repo(&mut self, server_id: String, full_name: String, cx: &mut Context<'_, Self>) {
        if self.adding.is_some() {
            return;
        }
        self.adding = Some(full_name.clone());
        self.add_error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let added = nodus.servers().add_from_github(&server_id, &full_name).await;
            let _ = this.update(cx, |this, cx| {
                this.adding = None;
                match added {
                    Ok(project) => {
                        this.close(cx);
                        this.state.update(cx, |s, cx| {
                            let refresh = s.refresh_projects(cx);
                            let id = project.id;
                            cx.spawn(async move |state, cx| {
                                refresh.await;
                                let _ = state.update(cx, |s, cx| s.navigate(crate::state::Route::Board(id), cx));
                            })
                            .detach();
                        });
                    }
                    Err(err) => this.add_error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.state.update(cx, AppState::close_server_dialog);
    }

    /// SRV-060: checks the server and saves it, then shows its projects.
    /// What the core says when it can't is shown as it is.
    fn connect(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let url = self.address.read(cx).value().trim().to_owned();
        let token = self.token.read(cx).value().trim().to_owned();
        if self.connecting || url.is_empty() || token.is_empty() {
            return;
        }
        self.connecting = true;
        self.error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let added = nodus.servers().add(&url, &token).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.connecting = false;
                match added {
                    Ok(server) => {
                        // The fields go; the keyboard stays in the dialog.
                        window.focus(&this.panel, cx);
                        this.listings.remove(&server.id);
                        this.state.update(cx, |s, cx| {
                            s.refresh_servers(cx);
                            s.open_server_dialog(
                                ServerDialog::Projects {
                                    server_id: Some(server.id),
                                },
                                cx,
                            );
                        });
                        this.serial += 1;
                    }
                    Err(err) => this.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks each server shown for its projects, once it's connected; one
    /// that isn't is asked again when it is.
    fn load(&mut self, servers: &[ServerInfo], cx: &mut Context<'_, Self>) {
        for server in servers {
            if server.state != ServerState::Connected {
                self.listings.remove(&server.id);
                continue;
            }
            if self.listings.contains_key(&server.id) {
                continue;
            }
            self.listings.insert(server.id.clone(), Listing::Loading);
            let nodus = self.state.read(cx).nodus.clone();
            let id = server.id.clone();
            cx.spawn(async move |this, cx| {
                let listed = nodus.servers().projects(&id).await;
                let _ = this.update(cx, |this, cx| {
                    let listing = match listed {
                        Ok(projects) => Listing::Loaded(projects),
                        Err(err) => Listing::Failed(err.message),
                    };
                    this.listings.insert(id, listing);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// SRV-161: ticks a project into the grid, or out of it.
    fn toggle(&mut self, server_id: String, project_id: String, linked: bool, cx: &mut Context<'_, Self>) {
        if !self.busy.insert(project_id.clone()) {
            return;
        }
        self.link_error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let done = if linked {
                nodus.servers().unlink(&project_id).await
            } else {
                nodus.servers().link(&server_id, &project_id).await.map(|_| ())
            };
            let refreshed = this.update(cx, |this, cx| {
                this.busy.remove(&project_id);
                if let Err(err) = done {
                    this.link_error = Some(err.message);
                }
                cx.notify();
                this.state.update(cx, AppState::refresh_projects)
            });
            if let Ok(refreshed) = refreshed {
                refreshed.await;
            }
        })
        .detach();
    }

    /// Tab: the step's next field, round again after its last; Add from
    /// GitHub has only its search.
    fn focus_next(&mut self, back: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let fields = match self.state.read(cx).server_dialog {
            Some(ServerDialog::Github { .. }) => vec![self.search.clone()],
            _ => self.fields(),
        };
        if fields.is_empty() {
            return;
        }
        let at = fields
            .iter()
            .position(|f| f.read(cx).focus_handle(cx).is_focused(window));
        let next = match (at, back) {
            (Some(0) | None, true) => fields.len() - 1,
            (Some(i), true) => i - 1,
            (Some(i), false) if i + 1 < fields.len() => i + 1,
            _ => 0,
        };
        fields[next].update(cx, |f, cx| f.focus(window, cx));
    }

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if e.keystroke.key == "escape" {
            cx.stop_propagation();
            self.close(cx);
            return;
        }
        // Return on a step with no field: its main button (a field's own
        // Return arrives as `PressEnter`).
        if e.keystroke.key == "enter"
            && self.panel.is_focused(window)
            && self.state.read(cx).server_dialog == Some(ServerDialog::Connect)
        {
            cx.stop_propagation();
            self.primary(window, cx);
            return;
        }
        // Nothing typed here reaches the grid's or the board's keys; marked,
        // not stopped, so the fields still get what's typed.
        window.prevent_default();
    }
}

impl Render for ServersDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let dialog = self.state.read(cx).server_dialog.clone();
        let (width, content) = match dialog {
            Some(ServerDialog::Projects { server_id }) => {
                (480., self.render_projects(server_id.as_deref(), t, window, cx))
            }
            Some(ServerDialog::Github { server_id }) => (520., self.render_github(&server_id, t, window, cx)),
            _ => {
                let wide = !matches!(self.step, Step::Address | Step::Token);
                (if wide { 480. } else { 440. }, self.render_step(t, window, cx))
            }
        };
        let next = cx.listener(|d, _: &NextField, window, cx| {
            cx.stop_propagation();
            d.focus_next(false, window, cx);
        });
        let previous = cx.listener(|d, _: &PreviousField, window, cx| {
            cx.stop_propagation();
            d.focus_next(true, window, cx);
        });
        let panel = div()
            .id("server-dialog")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.panel)
            .flex()
            .flex_col()
            .min_h_0()
            .text_size(TEXT_MD)
            .text_color(t.text_primary)
            .on_action(next)
            .on_action(previous)
            .capture_action(cx.listener(|d, _: &IndentInline, window, cx| {
                cx.stop_propagation();
                d.focus_next(false, window, cx);
            }))
            .capture_action(cx.listener(|d, _: &OutdentInline, window, cx| {
                cx.stop_propagation();
                d.focus_next(true, window, cx);
            }))
            // Esc closes, even from a field.
            .capture_action(cx.listener(|d, _: &Escape, _, cx| {
                cx.stop_propagation();
                d.close(cx);
            }))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(content);
        let this = cx.entity();
        Modal::new(("server-dialog", self.serial), px(width), move |_, cx| {
            this.update(cx, Self::close);
        })
        .render(panel, window, cx)
    }
}

impl ServersDialog {
    /// UI-309: the address and a development token.
    fn render_connect(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let address = self.address.read(cx).value().trim().is_empty();
        let token = self.token.read(cx).value().trim().is_empty();
        let ready = !address && !token && !self.connecting;
        let this = cx.entity();
        let cancel = this.clone();
        let github = this.clone();
        div()
            .flex()
            .flex_col()
            .child(heading("Connect to Server"))
            .child(
                div()
                    .mt(px(6.))
                    .mb(px(16.))
                    .text_color(t.text_secondary)
                    .child("Its projects show in your grid beside your own, and open the same way."),
            )
            .child(label("Address", t))
            .child(field(&self.address, window, cx, t))
            .child(label("Token", t).mt(px(12.)))
            .child(field(&self.token, window, cx, t))
            .child(
                div()
                    .mt(px(6.))
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.text_tertiary)
                    .child("The token nodus-server serve was given with --dev-token."),
            )
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .id("server-connect-error")
                        .mt(px(12.))
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.status_waiting)
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(20.))
                    .child(
                        div()
                            .id("server-use-github")
                            .flex_1()
                            .text_size(TEXT_SM)
                            .text_color(t.accent)
                            .child(div().hover(Styled::underline).child("Sign in with GitHub instead"))
                            .on_click(move |_, window, cx| github.update(cx, |d, cx| d.go(Step::Address, window, cx))),
                    )
                    .child(
                        button("server-connect-cancel")
                            .label("Cancel")
                            .on_click(move |_, _, cx| cancel.update(cx, Self::close)),
                    )
                    .child(
                        button("server-connect")
                            .primary()
                            .label(if self.connecting { "Connecting…" } else { "Connect" })
                            .disabled(!ready)
                            .on_click(move |_, window, cx| this.update(cx, |d, cx| d.connect(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_projects(
        &mut self,
        server_id: Option<&str>,
        t: Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let (servers, linked) = {
            let s = self.state.read(cx);
            let servers: Vec<ServerInfo> = s
                .servers
                .iter()
                .filter(|server| server_id.is_none_or(|id| server.id == id))
                .cloned()
                .collect();
            let linked: HashSet<String> = s
                .projects
                .iter()
                .filter(|p| p.server.is_some())
                .map(|p| p.id.clone())
                .collect();
            (servers, linked)
        };
        self.load(&servers, cx);
        let title = match (server_id, servers.first()) {
            (Some(_), Some(server)) => format!("Projects on {}", server.name),
            _ => "Server Projects".to_owned(),
        };
        let one = server_id.is_some();
        let mut sections = Vec::new();
        for server in &servers {
            sections.push(self.server_projects(server, one, &linked, t, cx));
        }
        if servers.is_empty() {
            let state = self.state.clone();
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(10.))
                    .text_color(t.text_secondary)
                    .child("You haven’t connected to a server yet.")
                    .child(
                        button("server-picker-connect")
                            .label("Connect to Server…")
                            .on_click(move |_, _, cx| {
                                state.update(cx, |s, cx| s.open_server_dialog(ServerDialog::Connect, cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        let done = cx.entity();
        let _ = window;
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .pt(px(22.))
            .px(px(22.))
            .pb(px(18.))
            .child(heading_owned(title))
            .child(div().mt(px(6.)).mb(px(14.)).text_color(t.text_secondary).child(
                "Tick the projects to show in your grid. Projects other people add stay out of it until you tick them.",
            ))
            .child(
                div()
                    .id("server-projects")
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .max_h(px(340.))
                    .overflow_y_scroll()
                    .children(sections),
            )
            .when_some(self.link_error.clone(), |d, error| {
                d.child(
                    div()
                        .mt(px(12.))
                        .text_size(TEXT_SM)
                        .text_color(t.status_waiting)
                        .child(error),
                )
            })
            .child(
                div().flex().justify_end().mt(px(18.)).child(
                    button("server-projects-done")
                        .primary()
                        .label("Done")
                        .on_click(move |_, _, cx| done.update(cx, Self::close)),
                ),
            )
            .into_any_element()
    }

    /// Add from GitHub on <server>… (SRV-120): its repositories, searched,
    /// each added with a click, and a way to install the App on more.
    fn render_github(
        &mut self,
        server_id: &str,
        t: Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        self.load_repos(server_id, cx);
        let server_name = self
            .state
            .read(cx)
            .servers
            .iter()
            .find(|s| s.id == server_id)
            .map_or_else(|| "the server".to_owned(), |s| s.name.clone());
        let query = self.search.read(cx).value().trim().to_owned();
        let linked: HashSet<String> = self
            .state
            .read(cx)
            .projects
            .iter()
            .filter(|p| p.server.as_deref() == Some(server_id))
            .map(|p| p.name.to_lowercase())
            .collect();
        let body: AnyElement = match self.repos.get(server_id) {
            None | Some(Repos::Loading) => note("Loading its repositories…", t).into_any_element(),
            Some(Repos::Failed(error)) => div()
                .id("github-repos-error")
                .text_color(t.status_waiting)
                .child(error.clone())
                .into_any_element(),
            Some(Repos::Loaded(repos)) => {
                let shown: Vec<&GithubRepo> = repos.iter().filter(|r| repo_matches(&r.full_name, &query)).collect();
                if repos.is_empty() {
                    note("The App isn’t installed on any repository yet.", t).into_any_element()
                } else if shown.is_empty() {
                    note("No repository matches.", t).into_any_element()
                } else {
                    let rows = shown.into_iter().map(|repo| {
                        let this = cx.entity();
                        let (server, full_name) = (server_id.to_owned(), repo.full_name.clone());
                        let adding = self.adding.as_deref() == Some(repo.full_name.as_str());
                        let name = repo
                            .full_name
                            .rsplit('/')
                            .next()
                            .unwrap_or(&repo.full_name)
                            .to_lowercase();
                        let added = linked.contains(&name);
                        div()
                            .id(SharedString::from(format!("github-repo-{}", repo.full_name)))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .h(px(34.))
                            .px(px(10.))
                            .rounded(RADIUS_SM)
                            .hover(|s| s.bg(t.bg_hover))
                            .active(|s| s.bg(t.bg_pressed))
                            .on_click(move |_, _, cx| {
                                let (server, full_name) = (server.clone(), full_name.clone());
                                this.update(cx, |d, cx| d.add_repo(server, full_name, cx));
                            })
                            .child(icon(Icon::FolderPlus, 15., 2., t.text_secondary))
                            .child(div().flex_1().min_w_0().truncate().child(repo.full_name.clone()))
                            .when(repo.private, |d| {
                                d.child(div().text_size(TEXT_SM).text_color(t.text_tertiary).child("Private"))
                            })
                            .when(added && !adding, |d| {
                                d.child(
                                    div()
                                        .text_size(TEXT_SM)
                                        .text_color(t.text_tertiary)
                                        .child("In your grid"),
                                )
                            })
                            .when(adding, |d| {
                                d.child(div().text_size(TEXT_SM).text_color(t.text_secondary).child("Cloning…"))
                            })
                    });
                    div().flex().flex_col().gap(px(1.)).children(rows).into_any_element()
                }
            }
        };
        let install = self.install_url.get(server_id).cloned();
        let cancel = cx.entity();
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .pt(px(22.))
            .px(px(22.))
            .pb(px(18.))
            .child(heading_owned(format!("Add from GitHub on {server_name}")))
            .child(
                div().mt(px(6.)).mb(px(14.)).text_color(t.text_secondary).child(
                    "Pick a repository to clone on the server. Each card you start in it gets a branch of its own.",
                ),
            )
            .child(field(&self.search, window, cx, t))
            .child(
                div()
                    .id("github-repos")
                    .mt(px(10.))
                    .flex()
                    .flex_col()
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .child(body),
            )
            .when_some(self.add_error.clone(), |d, error| {
                d.child(
                    div()
                        .id("github-add-error")
                        .mt(px(12.))
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.status_waiting)
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(18.))
                    .when_some(install, |d, url| {
                        d.child(button("github-install-more").label("Install on more repos").on_click(
                            move |_, _, _| {
                                nodus_platform::shell::open_external(&url);
                            },
                        ))
                    })
                    .child(div().flex_1())
                    .child(
                        button("github-cancel")
                            .label("Cancel")
                            .on_click(move |_, _, cx| cancel.update(cx, Self::close)),
                    ),
            )
            .into_any_element()
    }

    /// One server's projects, each with its checkbox.
    fn server_projects(
        &self,
        server: &ServerInfo,
        alone: bool,
        linked: &HashSet<String>,
        t: Tokens,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let body: AnyElement = match self.listings.get(&server.id) {
            None if server.state != ServerState::Connected => div().into_any_element(),
            None | Some(Listing::Loading) => note("Loading its projects…", t).into_any_element(),
            Some(Listing::Failed(error)) => div()
                .text_color(t.status_waiting)
                .child(error.clone())
                .into_any_element(),
            Some(Listing::Loaded(projects)) if projects.is_empty() => {
                note("No projects on this server yet.", t).into_any_element()
            }
            Some(Listing::Loaded(projects)) => {
                let rows = projects.iter().filter(|p| p.archived != Some(true)).map(|project| {
                    let ticked = linked.contains(&project.id);
                    let busy = self.busy.contains(&project.id);
                    let this = cx.entity();
                    let (server_id, project_id) = (server.id.clone(), project.id.clone());
                    div()
                        .id(SharedString::from(format!("server-project-{}", project.id)))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(34.))
                        .px(px(10.))
                        .rounded(RADIUS_SM)
                        .hover(|s| s.bg(t.bg_hover))
                        .active(|s| s.bg(t.bg_pressed))
                        .on_click(move |_, _, cx| {
                            let (server_id, project_id) = (server_id.clone(), project_id.clone());
                            this.update(cx, |d, cx| d.toggle(server_id, project_id, ticked, cx));
                        })
                        .child(checkbox(ticked, busy, t))
                        .child(folder_icon(16., Some(project.color)))
                        .child(div().flex_1().min_w_0().truncate().child(project.name.clone()))
                });
                div().flex().flex_col().gap(px(1.)).children(rows).into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(!alone, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(icon(Icon::Server, 14., 2., t.text_secondary))
                        .child(server.name.clone()),
                )
            })
            .when(server.state != ServerState::Connected, |d| {
                d.child(note_owned(status_text(server), t))
            })
            .child(body)
            .into_any_element()
    }
}

fn heading(text: &'static str) -> gpui::Div {
    heading_owned(text.to_owned())
}

fn heading_owned(text: String) -> gpui::Div {
    div()
        .text_size(TEXT_LG)
        .line_height(px(21.))
        .font_weight(FontWeight::SEMIBOLD)
        .truncate()
        .child(text)
}

fn label(text: &'static str, t: Tokens) -> gpui::Div {
    div()
        .mb(px(6.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text_primary)
        .child(text)
}

fn note(text: &'static str, t: Tokens) -> gpui::Div {
    note_owned(text.to_owned(), t)
}

fn note_owned(text: String, t: Tokens) -> gpui::Div {
    div()
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .text_color(t.text_secondary)
        .child(text)
}

/// A field framed as Settings frames the xAI key's: a hairline, and the
/// focus ring in its place while it has the keyboard.
pub(crate) fn field(state: &Entity<InputState>, window: &Window, cx: &App, t: Tokens) -> gpui::Div {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    div()
        .relative()
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .border(px(0.5))
        .border_color(if focused {
            crate::components::button::transparent()
        } else {
            t.border_strong
        })
        .rounded(RADIUS_SM)
        .bg(t.bg_elevated)
        .child(input(state, px(18.), false))
        .when(focused, |d| {
            d.child(focus_ring(Corners::all(RADIUS_SM), px(0.), t.focus_ring))
        })
}

/// A checkbox: an accent square with a check when ticked, a hairline
/// square when not; a spinner's place while it changes.
fn checkbox(ticked: bool, busy: bool, t: Tokens) -> gpui::Div {
    div()
        .flex_none()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .map(|d| {
            if ticked {
                d.bg(t.accent)
            } else {
                d.bg(t.bg_elevated).border(px(1.)).border_color(t.border_strong)
            }
        })
        .when(busy, |d| d.opacity(0.5))
        .when(ticked, |d| d.child(icon(Icon::Check, 12., 3., t.text_on_accent)))
}

/// A member's avatar, round: GitHub's picture once the core has fetched it
/// (SRV-045), their login's first letter until then.
pub fn avatar(login: &str, file: Option<PathBuf>, size: f32, t: Tokens) -> gpui::Div {
    let initial = login
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    div()
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.bg_selected)
        .text_size(px(size * 0.45))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_secondary)
        .map(|d| match file {
            Some(file) => d.child(img(file).size(px(size)).rounded_full().object_fit(ObjectFit::Cover)),
            None => d.child(initial),
        })
}

/// The pages a driven app was asked to open, in order (D11).
#[derive(Default)]
pub struct OpenedUrls(pub Vec<String>);

impl Global for OpenedUrls {}

/// Opens a page in the browser: GitHub's App and sign-in pages. A driven
/// app (D11) only notes it, for the driver's `opened`, so a test never
/// opens the person's browser.
pub fn open_url(url: &str, cx: &mut App) {
    if crate::shell::window::quiet() {
        cx.default_global::<OpenedUrls>().0.push(url.to_owned());
        return;
    }
    nodus_platform::shell::open_external(url);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(state: ServerState, error: Option<&str>, version: Option<&str>) -> ServerInfo {
        ServerInfo {
            id: "s".into(),
            url: "http://box:7420".into(),
            name: "box".into(),
            state,
            version: version.map(Into::into),
            error: error.map(Into::into),
            login: None,
            github_id: None,
        }
    }

    #[test]
    fn finds_a_repository_by_every_word_typed() {
        assert!(repo_matches("acme/shop-api", ""));
        assert!(repo_matches("acme/shop-api", "SHOP"));
        assert!(repo_matches("acme/shop-api", "acme api"));
        assert!(!repo_matches("acme/shop-api", "acme web"));
    }

    #[test]
    fn words_a_servers_status() {
        assert_eq!(status_text(&server(ServerState::Connected, None, None)), "Connected");
        assert_eq!(status_text(&server(ServerState::Connecting, None, None)), "Connecting…");
        assert_eq!(
            status_text(&server(ServerState::Offline, Some("connection refused"), None)),
            "Offline: connection refused"
        );
        assert_eq!(
            status_text(&server(ServerState::ProtocolMismatch, None, Some("2.0.0"))),
            "This server runs Nodus 2.0.0. Update it in Settings → Servers."
        );
    }
}
