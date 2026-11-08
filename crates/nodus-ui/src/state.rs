//! The window's state: a port of `store.ts` (the zustand store) and
//! `projectsUi.ts` (the grid's selection and toast, and the project
//! actions). Nothing here is saved by the UI itself: settings, projects,
//! groups and boards persist through the core (UI-048).

use std::time::Duration;

use gpui::{Context, EventEmitter, Task};
use indexmap::IndexMap;
use nodus_core::Nodus;
use nodus_domain::focus::is_focusing;
use nodus_domain::{
    AppSettings, BoardSnapshot, Event, GroupPatch, MenuCommand, OpenCardRequest, Project, ProjectAction, ProjectGroup,
    ProjectPatch, ProjectStats, SettingsPatch, UpdateStatus,
};

/// Where the window is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Projects,
    Board(String),
}

impl Route {
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Projects => None,
            Self::Board(id) => Some(id),
        }
    }
}

/// How long a toast stays (UI-251).
pub const TOAST_FOR: Duration = Duration::from_millis(3500);
/// The trailing debounce on stats refreshes after status events (UI-033).
pub const STATS_DEBOUNCE: Duration = Duration::from_millis(250);

/// Something the window should do that the state can't do itself.
#[derive(Debug, Clone, PartialEq)]
pub enum StateEvent {
    /// The grid should start renaming the selected folder, etc.; also any
    /// change the window chrome follows (the menu's Focus Mode tick).
    SettingsChanged,
}

/// `useApp` and `useProjectsUi`.
pub struct AppState {
    pub nodus: Nodus,
    pub route: Route,
    pub projects: Vec<Project>,
    pub groups: Vec<ProjectGroup>,
    pub stats: IndexMap<String, ProjectStats>,
    /// Actions in every project's rail. The actions API comes with its track.
    pub global_actions: Vec<ProjectAction>,
    /// Whether the first projects refresh has come back.
    pub projects_loaded: bool,

    /// The open board; boards load once the board track's API is in.
    pub board: Option<BoardSnapshot>,
    pub board_loading: bool,
    pub diff_open: bool,
    pub terminal_session_id: Option<String>,
    pub preview_session_id: Option<String>,
    pub agent_view: Option<(String, Option<String>)>,
    pub selection: Vec<String>,

    pub settings_open: bool,
    /// None until loaded.
    pub settings: Option<AppSettings>,
    pub open_request: Option<OpenCardRequest>,
    /// How updating is going; None until the updater answers (spec 06).
    pub update_status: Option<UpdateStatus>,

    /// The grid's selected folder, kept across navigation (UI-204).
    pub selected_id: Option<String>,
    /// The grid's toast, and a counter so a newer one restarts the timer.
    pub toast: Option<(String, u64)>,
    toast_seq: u64,
    _toast_timer: Option<Task<()>>,
    _stats_timer: Option<Task<()>>,
    _events: Task<()>,
}

impl EventEmitter<StateEvent> for AppState {}

impl AppState {
    /// UI-023, UI-024: starts on the grid and loads projects and settings.
    pub fn new(nodus: Nodus, cx: &mut Context<'_, Self>) -> Self {
        let mut rx = nodus.subscribe();
        let events = cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if this.update(cx, |state, cx| state.on_event(event, cx)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        let mut state = Self {
            nodus,
            route: Route::Projects,
            projects: Vec::new(),
            groups: Vec::new(),
            stats: IndexMap::new(),
            global_actions: Vec::new(),
            projects_loaded: false,
            board: None,
            board_loading: false,
            diff_open: false,
            terminal_session_id: None,
            preview_session_id: None,
            agent_view: None,
            selection: Vec::new(),
            settings_open: false,
            settings: None,
            open_request: None,
            update_status: None,
            selected_id: None,
            toast: None,
            toast_seq: 0,
            _toast_timer: None,
            _stats_timer: None,
            _events: events,
        };
        state.refresh_projects(cx).detach();
        state.load_settings(cx);
        // UI-044: a notification clicked while the window was closed.
        let nodus = state.nodus.clone();
        cx.spawn(async move |this, cx| {
            if let Some(request) = nodus.app().take_open_request().await {
                let _ = this.update(cx, |state, cx| state.show_card(request, cx));
            }
        })
        .detach();
        state
    }

    /// The events 0.9.4's App.tsx and store listened to (UI-033…UI-038,
    /// UI-043).
    fn on_event(&mut self, event: Event, cx: &mut Context<'_, Self>) {
        match event {
            Event::Status { .. } | Event::PtyExit { .. } => {
                // Status changes on the open board's cards come with the board track.
                self.schedule_stats_refresh(cx);
            }
            Event::BoardChanged { project_id } => {
                if self.route.project_id() == Some(project_id.as_str()) {
                    self.load_board(&project_id, cx);
                }
            }
            Event::ProjectsChanged => self.refresh_projects(cx).detach(),
            Event::Menu(command) => self.on_menu(command, cx),
            Event::OpenCard(request) => self.show_card(request, cx),
            Event::UpdateStatus(status) => {
                self.update_status = Some(status);
                cx.notify();
            }
            _ => {}
        }
    }

    /// What App.tsx and TitleBar.tsx do with a menu command.
    pub fn on_menu(&mut self, command: MenuCommand, cx: &mut Context<'_, Self>) {
        match command {
            MenuCommand::Settings => self.open_settings(cx),
            MenuCommand::Close => self.close_top_layer(cx),
            MenuCommand::NewProject => self.add_project(None, false, cx),
            MenuCommand::ToggleFocus => self.toggle_focus_mode(cx),
            // The board's own (09, 11).
            MenuCommand::NewSession | MenuCommand::NewTerminal | MenuCommand::Search => {}
        }
    }

    /// UI-040, UI-041: ⌘W closes the first open layer, else leaves the board;
    /// never the window.
    pub fn close_top_layer(&mut self, cx: &mut Context<'_, Self>) {
        if self.settings_open {
            self.close_settings(cx);
        } else if self.agent_view.is_some() {
            self.agent_view = None;
            cx.notify();
        } else if self.terminal_session_id.is_some() {
            self.terminal_session_id = None;
            cx.notify();
        } else if self.preview_session_id.is_some() {
            self.preview_session_id = None;
            cx.notify();
        } else if self.diff_open {
            self.diff_open = false;
            cx.notify();
        } else if matches!(self.route, Route::Board(_)) {
            self.navigate(Route::Projects, cx);
        }
    }

    /// UI-031, UI-032: goes to the grid or a board, clearing what belonged to
    /// the old route.
    pub fn navigate(&mut self, route: Route, cx: &mut Context<'_, Self>) {
        self.route = route.clone();
        self.board = None;
        self.selection.clear();
        self.terminal_session_id = None;
        self.preview_session_id = None;
        self.agent_view = None;
        self.diff_open = false;
        match route {
            Route::Board(id) => self.load_board(&id, cx),
            Route::Projects => self.refresh_projects(cx).detach(),
        }
        cx.notify();
    }

    /// UI-049: projects, groups, stats and global actions, replaced together.
    pub fn refresh_projects(&mut self, cx: &mut Context<'_, Self>) -> Task<()> {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let projects = nodus.projects().list().await;
            let groups = nodus.groups().list().await;
            let stats = nodus.projects().stats().await;
            let _ = this.update(cx, |state, cx| {
                state.projects = projects;
                state.groups = groups;
                state.stats = stats;
                state.projects_loaded = true;
                // UI-205: a selection whose project left goes with it.
                if let Some(id) = &state.selected_id
                    && !state.projects.iter().any(|p| &p.id == id)
                {
                    state.selected_id = None;
                }
                cx.notify();
            });
        })
    }

    /// UI-033: a refresh 250ms after the last status or exit.
    fn schedule_stats_refresh(&mut self, cx: &mut Context<'_, Self>) {
        self._stats_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATS_DEBOUNCE).await;
            let _ = this.update(cx, |state, cx| state.refresh_projects(cx).detach());
        }));
    }

    /// UI-050, UI-051. The board's API comes with the board track; until then
    /// a board opens empty.
    pub fn load_board(&mut self, project_id: &str, cx: &mut Context<'_, Self>) {
        let _ = project_id;
        self.board_loading = false;
        cx.notify();
    }

    pub fn load_settings(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let settings = nodus.settings().get().await;
            let _ = this.update(cx, |state, cx| {
                state.settings = Some(settings);
                cx.emit(StateEvent::SettingsChanged);
                cx.notify();
            });
        })
        .detach();
    }

    /// UI-079: merges at once, then takes the core's copy.
    pub fn update_settings(&mut self, patch: SettingsPatch, cx: &mut Context<'_, Self>) {
        if let Some(settings) = &mut self.settings {
            merge_settings(settings, &patch);
        }
        cx.notify();
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let settings = nodus.settings().update(patch).await;
            let _ = this.update(cx, |state, cx| {
                state.settings = Some(settings);
                cx.emit(StateEvent::SettingsChanged);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_settings(&mut self, cx: &mut Context<'_, Self>) {
        self.settings_open = true;
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<'_, Self>) {
        self.settings_open = false;
        cx.notify();
    }

    /// UI-045: shows a notification's card.
    pub fn show_card(&mut self, request: OpenCardRequest, cx: &mut Context<'_, Self>) {
        if self.route.project_id() != Some(request.project_id.as_str()) {
            self.navigate(Route::Board(request.project_id.clone()), cx);
        } else {
            self.terminal_session_id = None;
            self.preview_session_id = None;
            self.agent_view = None;
            self.diff_open = false;
        }
        self.settings_open = false;
        self.open_request = request.session_id.is_some().then_some(request);
        cx.notify();
    }

    /// The grid's selection (UI-202).
    pub fn select(&mut self, id: Option<String>, cx: &mut Context<'_, Self>) {
        if self.selected_id != id {
            self.selected_id = id;
            cx.notify();
        }
    }

    /// UI-251: a toast for 3500ms; a newer one replaces it.
    pub fn show_toast(&mut self, message: String, cx: &mut Context<'_, Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some((message, seq));
        self._toast_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            let _ = this.update(cx, |state, cx| {
                if state.toast.as_ref().is_some_and(|(_, s)| *s == seq) {
                    state.toast = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// `addProject` (UI-289…UI-292): adds a folder (or one picked in the
    /// native picker), then selects it on the grid, or with `open`, opens it.
    pub fn add_project(&mut self, path: Option<String>, open: bool, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let added = nodus.projects().add(path.clone()).await;
            let _ = this.update(cx, |state, cx| {
                let project = match added {
                    Err(_) => {
                        state.show_toast(add_failure(path.as_deref()), cx);
                        return;
                    }
                    // UI-291: cancelled.
                    Ok(None) => return,
                    Ok(Some(project)) => project,
                };
                if open {
                    let id = project.id.clone();
                    let refresh = state.refresh_projects(cx);
                    cx.spawn(async move |this, cx| {
                        refresh.await;
                        let _ = this.update(cx, |state, cx| {
                            if state.route.project_id() != Some(id.as_str()) {
                                state.navigate(Route::Board(id), cx);
                            }
                        });
                    })
                    .detach();
                    return;
                }
                state.select(Some(project.id), cx);
                if state.route == Route::Projects {
                    state.refresh_projects(cx).detach();
                } else {
                    state.navigate(Route::Projects, cx);
                }
            });
        })
        .detach();
    }

    /// Saves a project change and refreshes (rename, colour, archive, focus,
    /// group: UI-217…UI-219, UI-240, UI-287, UI-293).
    pub fn update_project(&mut self, id: String, patch: ProjectPatch, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let _ = nodus.projects().update(&id, patch).await;
            let _ = this.update(cx, |state, cx| state.refresh_projects(cx).detach());
        })
        .detach();
    }

    /// `moveToGroup` (UI-287).
    pub fn move_to_group(&mut self, id: String, group_id: Option<String>, cx: &mut Context<'_, Self>) {
        self.update_project(
            id,
            ProjectPatch {
                group_id: Some(group_id),
                ..ProjectPatch::default()
            },
            cx,
        );
    }

    /// Creates a group and files the project under it (UI-245).
    pub fn create_group_with(&mut self, project_id: String, name: String, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let group = nodus.groups().create(&name).await;
            let _ = this.update(cx, |state, cx| state.move_to_group(project_id, Some(group.id), cx));
        })
        .detach();
    }

    /// `updateGroup` (UI-288): at once here, then saved.
    pub fn update_group(&mut self, id: String, patch: GroupPatch, cx: &mut Context<'_, Self>) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            if let Some(name) = &patch.name {
                group.name.clone_from(name);
            }
            match patch.collapsed {
                Some(true) => group.collapsed = Some(true),
                Some(false) => group.collapsed = Some(false),
                None => {}
            }
        }
        cx.notify();
        let nodus = self.nodus.clone();
        cx.spawn(async move |_, _| {
            let _ = nodus.groups().update(&id, patch).await;
        })
        .detach();
    }

    /// UI-223: deletes a group at once; its projects become ungrouped.
    pub fn remove_group(&mut self, id: String, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            nodus.groups().remove(&id).await;
            let _ = this.update(cx, |state, cx| state.refresh_projects(cx).detach());
        })
        .detach();
    }

    /// UI-249: removes a project, and the selection with it.
    pub fn remove_project(&mut self, id: String, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            nodus.projects().remove(&id).await;
            let _ = this.update(cx, |state, cx| {
                if state.selected_id.as_deref() == Some(id.as_str()) {
                    state.selected_id = None;
                }
                state.refresh_projects(cx).detach();
            });
        })
        .detach();
    }

    pub fn reveal(&mut self, id: String, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |_, _| {
            let _ = nodus.projects().reveal_in_finder(&id).await;
        })
        .detach();
    }

    /// UI-047, UI-294: flips focus mode, only while it's offered.
    pub fn toggle_focus_mode(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(settings) = &self.settings
            && settings.focus_feature
        {
            let focus_mode = !settings.focus_mode;
            self.update_settings(
                SettingsPatch {
                    focus_mode: Some(focus_mode),
                    ..SettingsPatch::default()
                },
                cx,
            );
        }
    }

    /// `focusModeOn(settings)`.
    pub fn focus_mode(&self) -> bool {
        nodus_domain::focus::focus_mode_on(self.settings.as_ref())
    }

    /// `isFocusing(projects, focusMode)`.
    pub fn focusing(&self) -> bool {
        is_focusing(&self.projects, self.focus_mode())
    }

    pub fn focus_feature(&self) -> bool {
        self.settings.as_ref().is_some_and(|s| s.focus_feature)
    }

    pub fn pin_rail(&self) -> bool {
        self.settings.as_ref().is_some_and(|s| s.pin_project_rail)
    }

    /// The board's project for the title bar: the loaded board's, else the
    /// list's entry for the route (UI-092).
    pub fn route_project(&self) -> Option<&Project> {
        let id = self.route.project_id()?;
        self.board
            .as_ref()
            .map(|b| &b.project)
            .filter(|p| p.id == id)
            .or_else(|| self.projects.iter().find(|p| p.id == id))
    }

    /// Lets a timer elsewhere keep a reference to the same toast clock.
    pub fn toast_message(&self) -> Option<&str> {
        self.toast.as_ref().map(|(m, _)| m.as_str())
    }
}

/// The settings change applied to loaded settings, as `{...s, ...patch}`.
pub fn merge_settings(settings: &mut AppSettings, patch: &SettingsPatch) {
    if let Some(v) = patch.theme {
        settings.theme = v;
    }
    if let Some(v) = patch.auto_update {
        settings.auto_update = v;
    }
    if let Some(v) = patch.notify {
        settings.notify = v;
    }
    if let Some(v) = patch.dock_badge {
        settings.dock_badge = v;
    }
    if let Some(v) = patch.pin_project_rail {
        settings.pin_project_rail = v;
    }
    if let Some(v) = patch.focus_feature {
        settings.focus_feature = v;
    }
    if let Some(v) = patch.focus_mode {
        settings.focus_mode = v;
    }
    if let Some(v) = patch.capture {
        settings.capture = v;
    }
}

/// UI-290: the toast for a folder the core turned down. The last segment is
/// split on `/` only, as 0.9.4 does.
pub fn add_failure(path: Option<&str>) -> String {
    match path {
        Some(path) => {
            let name = path.split('/').rfind(|s| !s.is_empty()).unwrap_or(path);
            format!("Couldn’t add “{name}”. Only folders can be projects.")
        }
        None => "Couldn’t add that folder.".into(),
    }
}

/// `sortProjects`: most recently opened first (never opened counts as 0),
/// then newest added (UI-187).
pub fn sort_projects(projects: &[Project]) -> Vec<Project> {
    let mut sorted = projects.to_vec();
    sorted.sort_by(|a, b| {
        let opened = b.opened_at.unwrap_or(0.0) - a.opened_at.unwrap_or(0.0);
        let key = if opened != 0.0 {
            opened
        } else {
            b.created_at - a.created_at
        };
        key.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

/// `sortGroups`: by creation, oldest first (UI-285).
pub fn sort_groups(groups: &[ProjectGroup]) -> Vec<ProjectGroup> {
    let mut sorted = groups.to_vec();
    sorted.sort_by(|a, b| {
        a.created_at
            .partial_cmp(&b.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

/// `groupProjects`: the ungrouped projects, and each group's, keeping their
/// order; a project whose group is gone is ungrouped (UI-286).
pub fn group_projects(
    projects: &[Project],
    groups: &[ProjectGroup],
) -> (Vec<Project>, Vec<(ProjectGroup, Vec<Project>)>) {
    let known = |id: &Option<String>| id.as_ref().is_some_and(|id| groups.iter().any(|g| &g.id == id));
    let ungrouped = projects.iter().filter(|p| !known(&p.group_id)).cloned().collect();
    let by_group = sort_groups(groups)
        .into_iter()
        .map(|g| {
            let members = projects
                .iter()
                .filter(|p| p.group_id.as_deref() == Some(g.id.as_str()))
                .cloned()
                .collect();
            (g, members)
        })
        .collect();
    (ungrouped, by_group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::FolderColor;
    use serde_json::Map;

    fn project(id: &str, created: f64, opened: Option<f64>, group: Option<&str>) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            repo_path: format!("/repos/{id}"),
            color: FolderColor::Blue,
            created_at: created,
            opened_at: opened,
            group_id: group.map(Into::into),
            actions: None,
            focused: None,
            archived: None,
            extra: Map::new(),
        }
    }

    fn group(id: &str, created: f64) -> ProjectGroup {
        ProjectGroup {
            id: id.into(),
            name: id.into(),
            created_at: created,
            collapsed: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn sorts_by_last_opened_then_newest() {
        let sorted = sort_projects(&[
            project("old", 1.0, None, None),
            project("new", 5.0, None, None),
            project("opened", 2.0, Some(10.0), None),
            project("recent", 3.0, Some(20.0), None),
        ]);
        let ids: Vec<&str> = sorted.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["recent", "opened", "new", "old"]);
    }

    #[test]
    fn splits_projects_by_group_in_creation_order() {
        let projects = [
            project("a", 0.0, None, Some("g2")),
            project("b", 0.0, None, Some("gone")),
            project("c", 0.0, None, None),
            project("d", 0.0, None, Some("g1")),
        ];
        let (ungrouped, by_group) = group_projects(&projects, &[group("g2", 2.0), group("g1", 1.0)]);
        let ids: Vec<&str> = ungrouped.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        let groups: Vec<(&str, Vec<&str>)> = by_group
            .iter()
            .map(|(g, ps)| (g.id.as_str(), ps.iter().map(|p| p.id.as_str()).collect()))
            .collect();
        assert_eq!(groups, [("g1", vec!["d"]), ("g2", vec!["a"])]);
    }

    #[test]
    fn words_add_failures_as_0_9_4_did() {
        assert_eq!(
            add_failure(Some("/Users/example/notes.txt")),
            "Couldn’t add “notes.txt”. Only folders can be projects."
        );
        assert_eq!(
            add_failure(Some("/a/b/")),
            "Couldn’t add “b”. Only folders can be projects."
        );
        assert_eq!(
            add_failure(Some(r"C:\Users\example\x")),
            r"Couldn’t add “C:\Users\example\x”. Only folders can be projects."
        );
        assert_eq!(add_failure(None), "Couldn’t add that folder.");
    }
}
