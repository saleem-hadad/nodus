//! The views service: their files and runtime, what the window and the pages
//! ask of them, their commands and their schedules (views.ts).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nodus_board::geometry::{GAP, Point, Size, card_size, dock_rect, find_free_spot};
use nodus_board::notes::{active_cards, obstacles, rightmost_card};
use nodus_board::{Board, BoardSessions, Clock};
use nodus_domain::{
    BoardView, CommandResult, CreateViewOptions, Event, EventSink, Project, ViewEvent, ViewPatch, ViewReport, js,
};
use nodus_platform::Env;
use nodus_platform::runner::Runner;
use nodus_store::{BoardState, Store};
use serde_json::{Map, Value, json};
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};
use tokio::task::AbortHandle;

use crate::brains::AskOrigin;
use crate::check::{
    MAX_DESCRIPTION_CHARS, MAX_STATE_CHARS, MAX_TITLE_CHARS, check_json, clip, field, is_allowed, is_denied,
    js_string_or_empty,
};
use crate::command::execute;
use crate::page::{EMPTY_PAGE, ViewBoot, build_view_page};
use crate::runtime::{PageError, Runtime, delete_dir, read_html, read_runtime, write_runtime};
use crate::serve::{PageResponse, view_id_from_url};
use crate::{BrainLauncher, ViewError, ViewHost};

/// How long a runtime change waits for more before it's written (VIEW-006).
const RUNTIME_SAVE_DELAY: Duration = Duration::from_millis(500);
/// How often schedules are looked at (VIEW-085).
const TICK: Duration = Duration::from_millis(30_000);
/// A page can ask its brain something at most this often (VIEW-069).
const PAGE_ASK_GAP_MS: f64 = 15_000.0;
/// How many errors a view keeps (VIEW-119).
const KEPT_ERRORS: usize = 20;
/// How many command lines a view remembers the user's answer for (VIEW-081).
const REMEMBERED_RUNS: usize = 50;

/// What the views service works with.
pub struct ViewsConfig {
    /// The data folder; views' files go in its `views/`.
    pub user_data: PathBuf,
    pub store: Arc<Store>,
    /// The boards, on the same store: session lists and transcripts.
    pub board: Arc<Board>,
    /// Whether a brain's process is live, its status, stopping it.
    pub sessions: Arc<dyn BoardSessions>,
    /// Starting and typing into brains.
    pub brains: Arc<dyn BrainLauncher>,
    pub host: Arc<dyn ViewHost>,
    /// `Event::View` and `Event::BoardChanged` go here.
    pub events: Arc<dyn EventSink>,
    /// The user's environment, once the login shell has given it (`envReady`).
    pub env: watch::Receiver<Option<Arc<Env>>>,
    /// Where views' commands run: here, or in the session host on a server
    /// (SRV-005, SRV-145).
    pub runner: Arc<dyn Runner>,
    /// `Date.now()`.
    pub clock: Clock,
    /// Where timers and commands run.
    pub runtime: Handle,
}

/// Every view's files, runtime, commands, schedules and brains. Cheap to
/// clone; clones share it.
#[derive(Clone)]
pub struct Views {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) user_data: PathBuf,
    pub(crate) store: Arc<Store>,
    pub(crate) board: Arc<Board>,
    pub(crate) sessions: Arc<dyn BoardSessions>,
    pub(crate) brains: Arc<dyn BrainLauncher>,
    pub(crate) host: Arc<dyn ViewHost>,
    pub(crate) events: Arc<dyn EventSink>,
    pub(crate) env: watch::Receiver<Option<Arc<Env>>>,
    pub(crate) runner: Arc<dyn Runner>,
    pub(crate) clock: Clock,
    pub(crate) runtime: Handle,
    pub(crate) state: Mutex<State>,
}

/// What's kept in memory, keyed by view id or brain session id.
#[derive(Default)]
pub(crate) struct State {
    /// Each view's runtime, read from its file the first time (VIEW-005).
    runtimes: HashMap<String, Runtime>,
    /// Pending runtime writes, by view, with the generation that set each.
    runtime_timers: HashMap<String, (u64, AbortHandle)>,
    generation: u64,
    /// When each view's page last asked its brain (VIEW-069).
    last_page_ask: HashMap<String, f64>,
    /// Commands running, by `<view id>\0<name>` (VIEW-074).
    running: HashMap<String, watch::Receiver<Option<CommandResult>>>,
    /// What a brain was asked while it was busy, for when its turn ends (VIEW-094).
    pub(crate) pending_asks: HashMap<String, String>,
    /// Brains' idle timers. An entry whose timer was cancelled by typing into
    /// the brain stays, so its input is still noted until the next status.
    pub(crate) idle_timers: HashMap<String, Option<AbortHandle>>,
    /// When the user last typed into each brain while it was idle (VIEW-098).
    pub(crate) last_input: HashMap<String, f64>,
    /// Board tools waiting for a view's page to load (VIEW-108).
    load_waiters: HashMap<String, Vec<(u64, oneshot::Sender<()>)>>,
    ticker: Option<AbortHandle>,
}

/// A command run going, or done, that callers can wait on together (VIEW-074).
pub struct CommandRun(watch::Receiver<Option<CommandResult>>);

impl CommandRun {
    /// The run's result, once it's finished.
    pub async fn result(mut self) -> Result<CommandResult, ViewError> {
        match self.0.wait_for(Option::is_some).await {
            Ok(result) => Ok(result.clone().expect("waited for a result")),
            Err(_) => Err(ViewError::new("The command was stopped")),
        }
    }
}

pub(crate) fn no_view() -> ViewError {
    ViewError::new("There’s no view with that id on this board")
}

/// `{id, title, description, data}` of a view, for a page.
fn summary(view: &BoardView, data: Option<Value>) -> Value {
    let mut out = json!({ "id": view.id, "title": view.title, "description": view.description });
    if let Some(data) = data {
        out["data"] = data;
    }
    out
}

impl Views {
    pub fn new(config: ViewsConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                user_data: config.user_data,
                store: config.store,
                board: config.board,
                sessions: config.sessions,
                brains: config.brains,
                host: config.host,
                events: config.events,
                env: config.env,
                runner: config.runner,
                clock: config.clock,
                runtime: config.runtime,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn now(&self) -> f64 {
        (self.inner.clock)()
    }

    pub(crate) fn spawn<F>(&self, task: F) -> AbortHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.inner.runtime.spawn(task).abort_handle()
    }

    /// Waits for the login-shell environment, which commands and git need.
    pub(crate) async fn env(&self) -> Arc<Env> {
        let mut env = self.inner.env.clone();
        if let Ok(env) = env.wait_for(Option::is_some).await
            && let Some(env) = env.as_ref()
        {
            return Arc::clone(env);
        }
        // Whatever was to read it is gone.
        Arc::new(Env::from_process())
    }

    pub(crate) fn notify(&self, project_id: &str, view_id: &str, event: ViewEvent, payload: Value) {
        self.inner.events.emit(Event::View {
            project_id: project_id.to_owned(),
            view_id: view_id.to_owned(),
            event,
            payload,
        });
    }

    pub(crate) fn board_changed(&self, project_id: &str) {
        self.inner.events.emit(Event::BoardChanged {
            project_id: project_id.to_owned(),
        });
    }

    pub(crate) fn project(&self, project_id: &str) -> Result<Project, ViewError> {
        self.inner
            .store
            .read(|state| state.project(project_id).cloned())
            .map_err(|err| ViewError::new(err.to_string()))
    }

    /// A view, by an id from Claude, a page or the window, on this board only:
    /// `__proto__` finds nothing (VIEW-016).
    pub(crate) fn find_view(&self, project_id: &str, view_id: Option<&Value>) -> Result<BoardView, ViewError> {
        let Some(Value::String(id)) = view_id else {
            return Err(no_view());
        };
        self.view(project_id, id).ok_or_else(no_view)
    }

    pub(crate) fn view(&self, project_id: &str, view_id: &str) -> Option<BoardView> {
        self.inner
            .store
            .read(|state| state.board(project_id)?.views.get(view_id).cloned())
    }

    /// Changes a view on its board and saves, or says it isn't there.
    pub(crate) fn update_view<R>(
        &self,
        project_id: &str,
        view_id: &str,
        change: impl FnOnce(&mut BoardState, &str) -> Result<R, ViewError>,
    ) -> Result<R, ViewError> {
        if self.view(project_id, view_id).is_none() {
            return Err(no_view());
        }
        self.inner.store.update(|state| {
            let board = state.board_mut(project_id);
            if !board.views.contains_key(view_id) {
                return Err(no_view());
            }
            change(board, view_id)
        })
    }

    // -----------------------------------------------------------------------
    // Runtime

    /// Reads or changes a view's runtime, reading it from its file the first time (VIEW-005).
    pub(crate) fn with_runtime<R>(&self, view_id: &str, f: impl FnOnce(&mut Runtime) -> R) -> R {
        let mut state = self.lock();
        let rt = state
            .runtimes
            .entry(view_id.to_owned())
            .or_insert_with(|| read_runtime(&self.inner.user_data, view_id));
        f(rt)
    }

    /// Writes a view's runtime 500 ms after the last change to it (VIEW-006).
    pub(crate) fn save_runtime(&self, view_id: &str) {
        let mut state = self.lock();
        state.generation += 1;
        let generation = state.generation;
        let weak = Arc::downgrade(&self.inner);
        let id = view_id.to_owned();
        let task = self.spawn(async move {
            tokio::time::sleep(RUNTIME_SAVE_DELAY).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let views = Views { inner };
            let runtime = {
                let mut state = views.lock();
                if state.runtime_timers.get(&id).is_some_and(|(g, _)| *g == generation) {
                    state.runtime_timers.remove(&id);
                }
                state.runtimes.get(&id).cloned()
            };
            if let Some(runtime) = runtime
                && let Err(err) = write_runtime(&views.inner.user_data, &id, &runtime)
            {
                tracing::error!(view = %id, %err, "[views] save failed");
            }
        });
        if let Some((_, old)) = state.runtime_timers.insert(view_id.to_owned(), (generation, task)) {
            old.abort();
        }
    }

    /// Writes what's pending now, and stops the schedules: the app is quitting (VIEW-007).
    pub fn flush(&self) {
        let pending: Vec<(String, Option<Runtime>)> = {
            let mut state = self.lock();
            if let Some(ticker) = state.ticker.take() {
                ticker.abort();
            }
            let timers: Vec<(String, (u64, AbortHandle))> = state.runtime_timers.drain().collect();
            timers
                .into_iter()
                .map(|(id, (_, timer))| {
                    timer.abort();
                    let runtime = state.runtimes.get(&id).cloned();
                    (id, runtime)
                })
                .collect()
        };
        for (id, runtime) in pending {
            if let Some(runtime) = runtime
                && let Err(err) = write_runtime(&self.inner.user_data, &id, &runtime)
            {
                tracing::error!(view = %id, %err, "[views] flush failed");
            }
        }
    }

    /// Forgets a view's runtime and removes its files (VIEW-008).
    pub(crate) fn delete_files(&self, view_id: &str) {
        {
            let mut state = self.lock();
            if let Some((_, timer)) = state.runtime_timers.remove(view_id) {
                timer.abort();
            }
            state.runtimes.remove(view_id);
        }
        if let Err(err) = delete_dir(&self.inner.user_data, view_id) {
            tracing::error!(view = %view_id, %err, "[views] remove failed");
        }
    }

    /// Forgets the files of a project's views when the project goes. Their
    /// brains stop with its sessions (VIEW-118).
    pub fn remove_project_views(&self, board: Option<&BoardState>) {
        for view_id in board
            .map(|b| b.views.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            self.delete_files(&view_id);
        }
    }

    // -----------------------------------------------------------------------
    // Serving pages

    /// Answers the frame's request for `nodus-view://<id>/`: the view's page
    /// with what every view gets, booted with its title, data, state and
    /// results; `404 Not found` for anything else (VIEW-019 to VIEW-021).
    pub fn serve_page(&self, url: &str) -> PageResponse {
        let Some(view_id) = view_id_from_url(url) else {
            return PageResponse::not_found();
        };
        let view = self.inner.store.read(|state| {
            let project_id = state.project_id_for_view(view_id)?;
            state.board(project_id)?.views.get(view_id).cloned()
        });
        let Some(view) = view else {
            return PageResponse::not_found();
        };
        let (data, state, results) =
            self.with_runtime(view_id, |rt| (rt.data.clone(), rt.state.clone(), rt.results.clone()));
        let html = read_html(&self.inner.user_data, view_id);
        let boot = ViewBoot {
            view_id: view_id.to_owned(),
            title: view.title.clone(),
            starts_sessions: view.starts_sessions,
            data,
            state,
            results: Some(results),
        };
        let html = if html.is_empty() { EMPTY_PAGE } else { &html };
        PageResponse::page(build_view_page(html, &boot))
    }

    // -----------------------------------------------------------------------
    // Views from the window

    /// `views:create`: puts up a view for what the user described, and
    /// starts its brain on building it (VIEW-100).
    pub async fn create(&self, project_id: &str, opts: &CreateViewOptions) -> Result<BoardView, ViewError> {
        self.env().await;
        self.project(project_id)?;
        let description = clip(Some(&Value::String(opts.description.clone())), MAX_DESCRIPTION_CHARS);
        if description.is_empty() {
            return Err(ViewError::new("Describe what the view should show"));
        }
        if !opts.x.is_finite() || !opts.y.is_finite() {
            return Err(ViewError::new("A view needs a place on the board"));
        }
        let now = self.now();
        let view = BoardView {
            id: uuid::Uuid::new_v4().to_string(),
            title: "New view".into(),
            description: description.clone(),
            x: js::round(opts.x),
            y: js::round(opts.y),
            version: 0,
            created_at: now,
            updated_at: now,
            session_id: None,
            from: None,
            commands: None,
            refresh: None,
            w: None,
            h: None,
            starts_sessions: None,
            allowed_runs: None,
            denied_runs: None,
            extra: Map::new(),
        };
        self.inner.store.update(|state| {
            state.board_mut(project_id).views.insert(view.id.clone(), view.clone());
        });
        if let Err(err) = self
            .start_brain(project_id, &view.id, Some(format!("Build this view:\n\n{description}")))
            .await
        {
            self.inner.store.update(|state| {
                state.board_mut(project_id).views.shift_remove(&view.id);
            });
            return Err(err);
        }
        self.board_changed(project_id);
        Ok(self.view(project_id, &view.id).unwrap_or(view))
    }

    /// `views:save`: where the user moved or resized a view, its new title,
    /// or that it may no longer start sessions. Only its brain lets a view
    /// start sessions; the user can always stop it (VIEW-116, VIEW-115).
    pub fn save(&self, project_id: &str, patch: &ViewPatch) -> Result<(), ViewError> {
        let size =
            nodus_board::geometry::clamp_view_size(nodus_board::geometry::PartialSize { w: patch.w, h: patch.h });
        let title = clip(
            patch.title.as_ref().map(|t| Value::String(t.clone())).as_ref(),
            MAX_TITLE_CHARS,
        );
        self.update_view(project_id, &patch.id, |board, id| {
            let view = &mut board.views[id];
            if let Some(x) = patch.x.filter(|x| x.is_finite()) {
                view.x = js::round(x);
            }
            if let Some(y) = patch.y.filter(|y| y.is_finite()) {
                view.y = js::round(y);
            }
            if size.w.is_some() {
                view.w = size.w;
            }
            if size.h.is_some() {
                view.h = size.h;
            }
            if patch.starts_sessions == Some(false) {
                view.starts_sessions = None;
            }
            if !title.is_empty() {
                set_title(board, id, &title);
            }
            Ok(())
        })
    }

    /// `views:remove`: takes a view off the board, with its page, its data
    /// and its brain, whose transcript stays on disk (VIEW-117).
    pub fn remove(&self, project_id: &str, view_id: &str) -> Result<(), ViewError> {
        let brain = self.update_view(project_id, view_id, |board, id| {
            let view = board.views.shift_remove(id).expect("found");
            let brain = view
                .session_id
                .as_ref()
                .and_then(|sid| board.nodes.shift_remove(sid))
                .map(|node| node.session_id);
            if let Some(brain) = &brain
                && !board.hidden.contains(brain)
            {
                board.hidden.push(brain.clone());
            }
            Ok(brain)
        })?;
        if let Some(brain) = brain {
            self.forget_brain(&brain);
            let sessions = Arc::clone(&self.inner.sessions);
            self.spawn(async move {
                sessions.kill(&brain).await;
                sessions.forget_status(&brain);
            });
        }
        self.delete_files(view_id);
        Ok(())
    }

    /// `views:approve`: the user allowing, or turning down, one of a view's
    /// command lines. One they allow runs straight away (VIEW-081, VIEW-082).
    pub fn approve(&self, project_id: &str, view_id: &str, name: &str, allow: bool) -> Result<(), ViewError> {
        self.update_view(project_id, view_id, |board, id| {
            let view = &mut board.views[id];
            let Some(cmd) = view.commands.iter_mut().flatten().find(|c| c.name == name) else {
                return Err(ViewError::new("That command is gone"));
            };
            let run = cmd.run.clone();
            if allow {
                cmd.approved = Some(run.clone());
                cmd.denied = None;
            } else {
                cmd.denied = Some(run.clone());
                cmd.approved = None;
            }
            let without = |runs: &Option<Vec<String>>| -> Vec<String> {
                runs.iter().flatten().filter(|r| **r != run).cloned().collect()
            };
            let (gains, loses) = if allow {
                (&mut view.allowed_runs, &mut view.denied_runs)
            } else {
                (&mut view.denied_runs, &mut view.allowed_runs)
            };
            let mut kept = without(gains);
            kept.push(run.clone());
            let skip = kept.len().saturating_sub(REMEMBERED_RUNS);
            *gains = Some(kept.split_off(skip));
            *loses = Some(without(loses));
            Ok(())
        })?;
        self.board_changed(project_id);
        if allow && let Ok(run) = self.start_command(project_id, view_id, name) {
            self.spawn(async move {
                let _ = run.result().await;
            });
        }
        Ok(())
    }

    /// `views:refresh`: asks the view's brain for fresh data when it has a
    /// refresh, then runs its allowed commands together (VIEW-084, VIEW-120).
    pub async fn refresh(&self, project_id: &str, view_id: &str) -> Result<Vec<CommandResult>, ViewError> {
        self.env().await;
        let view = self.view(project_id, view_id).ok_or_else(no_view)?;
        if let Some(refresh) = &view.refresh {
            let now = self.now();
            self.with_runtime(view_id, |rt| rt.refreshed_at = Some(now));
            self.save_runtime(view_id);
            self.ask_brain(project_id, view_id, &refresh.prompt, AskOrigin::User)
                .await?;
        }
        let names: Vec<String> = self
            .view(project_id, view_id)
            .ok_or_else(no_view)?
            .commands
            .iter()
            .flatten()
            .filter(|c| is_allowed(c))
            .map(|c| c.name.clone())
            .collect();
        let runs = names
            .iter()
            .map(|name| self.start_command(project_id, view_id, name))
            .collect::<Result<Vec<_>, _>>()?;
        let mut results = Vec::with_capacity(runs.len());
        for run in runs {
            results.push(run.result().await?);
        }
        Ok(results)
    }

    /// `views:report`: a JavaScript error the page hit, or that the page at
    /// version `loaded` is up. A view that has gone is ignored (VIEW-119).
    pub fn report(&self, project_id: &str, view_id: &str, report: &ViewReport) {
        let Some(view) = self.view(project_id, view_id) else {
            return;
        };
        if let Some(error) = report.error.as_deref().filter(|e| !e.is_empty()) {
            let at = self.now();
            self.with_runtime(view_id, |rt| {
                rt.errors.push(PageError {
                    message: js::slice_utf16(error, 0, 2000),
                    at,
                });
                let skip = rt.errors.len().saturating_sub(KEPT_ERRORS);
                rt.errors.drain(..skip);
            });
            self.save_runtime(view_id);
        }
        if report.loaded.is_some_and(|loaded| loaded >= view.version) {
            let waiters = self.lock().load_waiters.remove(view_id).unwrap_or_default();
            for (_, waiter) in waiters {
                let _ = waiter.send(());
            }
        }
    }

    // -----------------------------------------------------------------------
    // The page's window.nodus

    /// `views:call`: a request from a view's page, answered for that view and
    /// its board only (VIEW-060 to VIEW-071).
    pub async fn call(&self, project_id: &str, view_id: &str, method: &str, args: &Value) -> Result<Value, ViewError> {
        let view = self.view(project_id, view_id).ok_or_else(no_view)?;
        let args = match args {
            Value::Object(_) | Value::Array(_) => Some(args),
            _ => None,
        };
        let arg = |key: &str| field(args, key);
        match method {
            // VIEW-062: the board's cards, without brains.
            "sessions" => {
                let items = self
                    .inner
                    .board
                    .session_list(project_id, None, false)
                    .map_err(|err| ViewError::new(err.to_string()))?;
                serde_json::to_value(items).map_err(|err| ViewError::new(err.to_string()))
            }
            // VIEW-063
            "notes" => Ok(self.inner.store.read(|state| {
                let mut notes: Vec<_> = state
                    .board(project_id)
                    .map(|b| b.notes.values().collect())
                    .unwrap_or_default();
                notes.sort_by(|a, b| {
                    a.created_at
                        .partial_cmp(&b.created_at)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                Value::Array(
                    notes
                        .into_iter()
                        .map(|n| json!({ "id": n.id, "text": n.text, "color": n.color }))
                        .collect(),
                )
            })),
            // VIEW-064: the other views, in the board's order.
            "views" => {
                let others: Vec<BoardView> = self.inner.store.read(|state| {
                    state
                        .board(project_id)
                        .map(|b| b.views.values().filter(|v| v.id != view_id).cloned().collect())
                        .unwrap_or_default()
                });
                Ok(Value::Array(
                    others
                        .iter()
                        .map(|v| summary(v, self.with_runtime(&v.id, |rt| rt.data.clone())))
                        .collect(),
                ))
            }
            // VIEW-065
            "view" => {
                let other = self.find_view(project_id, arg("id"))?;
                let data = self.with_runtime(&other.id, |rt| rt.data.clone());
                Ok(summary(&other, data))
            }
            // VIEW-066
            "git" => {
                self.env().await;
                let project = self.project(project_id)?;
                Ok(self.inner.host.git_status(&project.repo_path).await)
            }
            // VIEW-067
            "setState" => {
                let state = arg("state").cloned();
                check_json(state.as_ref(), MAX_STATE_CHARS, "The state")?;
                self.with_runtime(view_id, |rt| rt.state = state.clone());
                self.save_runtime(view_id);
                self.notify(project_id, view_id, ViewEvent::State, state.unwrap_or(Value::Null));
                Ok(Value::Null)
            }
            // VIEW-068
            "run" => {
                let name = js_string_or_empty(arg("name"));
                let result = self.run_command(project_id, view_id, &name).await?;
                serde_json::to_value(result).map_err(|err| ViewError::new(err.to_string()))
            }
            // VIEW-069: the time is noted before asking, so a failing ask still waits.
            "ask" => {
                let now = self.now();
                {
                    let mut state = self.lock();
                    let last = state.last_page_ask.get(view_id).copied().unwrap_or(0.0);
                    if now - last < PAGE_ASK_GAP_MS {
                        return Err(ViewError::new(
                            "Asked too soon: a view can ask its brain once every 15 seconds",
                        ));
                    }
                    state.last_page_ask.insert(view_id.to_owned(), now);
                }
                let prompt = js_string_or_empty(arg("prompt"));
                self.ask_brain(project_id, &view.id, &prompt, AskOrigin::Page).await?;
                Ok(Value::Null)
            }
            // VIEW-070
            "openUrl" => {
                let url = arg("url").and_then(Value::as_str).unwrap_or_default();
                let web = url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
                    || url.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"));
                if !web {
                    return Err(ViewError::new("Only http and https links open"));
                }
                self.inner.host.open_url(url);
                Ok(Value::Null)
            }
            // CHAT-031: a page's answers, to the session that put it up.
            "submit" => {
                let Some(session) = view.from.clone().filter(|s| !s.is_empty()) else {
                    return Err(ViewError::new(
                        "No session put this view up, so there’s no one to send the answers to",
                    ));
                };
                let answers = arg("answers").cloned().unwrap_or(Value::Null);
                check_json(Some(&answers), MAX_STATE_CHARS, "The answers")?;
                let message = nodus_domain::Answers {
                    to: view.id.clone(),
                    title: view.title.clone(),
                    answers,
                }
                .to_message();
                self.inner
                    .host
                    .deliver(project_id, &session, &message)
                    .await
                    .map_err(ViewError::new)?;
                Ok(Value::Null)
            }
            // VIEW-071, VIEW-072: openSession and startSession are the board's.
            _ => Err(ViewError::new(format!("nodus.{method} isn’t something a view can do"))),
        }
    }

    // -----------------------------------------------------------------------
    // Commands

    /// Starts one of a view's commands, if the user allowed its command line,
    /// or joins the run of it already going (VIEW-073, VIEW-074). When it
    /// finishes and the view is still there, its result is kept and sent to
    /// the page (VIEW-075).
    pub fn start_command(&self, project_id: &str, view_id: &str, name: &str) -> Result<CommandRun, ViewError> {
        let view = self.view(project_id, view_id).ok_or_else(no_view)?;
        let Some(cmd) = view.commands.iter().flatten().find(|c| c.name == name).cloned() else {
            return Err(ViewError::new(format!("This view has no command called “{name}”")));
        };
        if !is_allowed(&cmd) {
            return Err(ViewError::new(if is_denied(&cmd) {
                format!("The user didn’t allow “{name}” to run")
            } else {
                format!("“{name}” is waiting for the user to allow it on the view")
            }));
        }
        let key = format!("{view_id}\0{name}");
        let repo = self.project(project_id).map(|p| p.repo_path);
        let (tx, rx) = watch::channel(None);
        {
            let mut state = self.lock();
            if let Some(going) = state.running.get(&key) {
                return Ok(CommandRun(going.clone()));
            }
            let _ = repo.as_ref().map_err(Clone::clone)?;
            state.running.insert(key.clone(), rx.clone());
        }
        let repo = repo?;
        let views = self.clone();
        let (project_id, view_id) = (project_id.to_owned(), view_id.to_owned());
        self.spawn(async move {
            let env = views.env().await;
            let runner = Arc::clone(&views.inner.runner);
            let result = execute(&*runner, &env, &repo, &cmd.name, &cmd.run, || views.now()).await;
            // The view may have gone while it ran.
            if views.view(&project_id, &view_id).is_some() {
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                views.with_runtime(&view_id, |rt| rt.results.insert(cmd.name.clone(), value.clone()));
                views.save_runtime(&view_id);
                views.notify(&project_id, &view_id, ViewEvent::Result, value);
            }
            views.lock().running.remove(&key);
            tx.send_replace(Some(result));
        });
        Ok(CommandRun(rx))
    }

    /// Runs one of a view's commands and waits for its result (`runCommand`).
    pub async fn run_command(&self, project_id: &str, view_id: &str, name: &str) -> Result<CommandResult, ViewError> {
        self.start_command(project_id, view_id, name)?.result().await
    }

    // -----------------------------------------------------------------------
    // Schedules

    /// Looks at the schedules every 30 s while the app is open, whether or
    /// not a board is showing (VIEW-085). Once only.
    pub fn start_schedules(&self) {
        let mut state = self.lock();
        if state.ticker.is_some() {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        state.ticker = Some(self.spawn(async move {
            let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                Views { inner }.tick();
            }
        }));
    }

    /// Runs the commands and refreshes that are due, as the 30 s ticker does
    /// (VIEW-086, VIEW-087).
    pub fn tick(&self) {
        let now = self.now();
        let views: Vec<(String, BoardView)> = self.inner.store.read(|state| {
            state
                .projects
                .iter()
                .filter(|p| p.archived != Some(true))
                .flat_map(|p| {
                    state
                        .board(&p.id)
                        .map(|b| b.views.values().map(|v| (p.id.clone(), v.clone())).collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .collect()
        });
        for (project_id, view) in views {
            for cmd in view.commands.iter().flatten() {
                let Some(every) = cmd.every.filter(|e| *e != 0.0 && !e.is_nan()) else {
                    continue;
                };
                if !is_allowed(cmd) {
                    continue;
                }
                let last = self.with_runtime(&view.id, |rt| {
                    rt.results
                        .get(&cmd.name)
                        .and_then(|r| r.get("at"))
                        .and_then(Value::as_f64)
                });
                if now - last.unwrap_or(0.0) < every * 60_000.0 {
                    continue;
                }
                match self.start_command(&project_id, &view.id, &cmd.name) {
                    Ok(run) => {
                        self.spawn(async move {
                            if let Err(err) = run.result().await {
                                tracing::warn!(%err, "[views] command failed");
                            }
                        });
                    }
                    Err(err) => tracing::warn!(%err, "[views] command failed"),
                }
            }
            if let Some(refresh) = &view.refresh {
                let since = self
                    .with_runtime(&view.id, |rt| rt.refreshed_at)
                    .unwrap_or(view.created_at);
                if now - since >= refresh.every * 60_000.0 {
                    self.with_runtime(&view.id, |rt| rt.refreshed_at = Some(now));
                    self.save_runtime(&view.id);
                    let (views, prompt, view_id) = (self.clone(), refresh.prompt.clone(), view.id.clone());
                    let project_id = project_id.clone();
                    self.spawn(async move {
                        if let Err(err) = views
                            .ask_brain(&project_id, &view_id, &prompt, AskOrigin::Schedule)
                            .await
                        {
                            tracing::warn!(%err, "[views] refresh failed");
                        }
                    });
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Board tools

    /// Waits for the board to load a changed page, then for its scripts to
    /// settle, and returns what went wrong (VIEW-108).
    pub(crate) async fn check_page(&self, view_id: &str, since: f64) -> Map<String, Value> {
        const LOAD_WAIT: Duration = Duration::from_millis(3000);
        const SETTLE: Duration = Duration::from_millis(500);
        let (tx, rx) = oneshot::channel();
        let token = {
            let mut state = self.lock();
            state.generation += 1;
            let token = state.generation;
            state
                .load_waiters
                .entry(view_id.to_owned())
                .or_default()
                .push((token, tx));
            token
        };
        let loaded = matches!(tokio::time::timeout(LOAD_WAIT, rx).await, Ok(Ok(())));
        let mut out = Map::new();
        if !loaded {
            let mut state = self.lock();
            if let Some(waiters) = state.load_waiters.get_mut(view_id) {
                waiters.retain(|(t, _)| *t != token);
                if waiters.is_empty() {
                    state.load_waiters.remove(view_id);
                }
            }
            out.insert(
                "checked".into(),
                Value::String(
                    "The board isn’t showing this view right now, so the page couldn’t be checked for errors.".into(),
                ),
            );
            return out;
        }
        tokio::time::sleep(SETTLE).await;
        let errors = self.with_runtime(view_id, |rt| {
            rt.errors
                .iter()
                .filter(|e| e.at >= since)
                .map(|e| Value::String(e.message.clone()))
                .collect()
        });
        out.insert("errors".into(), Value::Array(errors));
        out
    }

    /// Where a session's new view goes: beside its card, or the rightmost
    /// card, in the nearest free spot (VIEW-103).
    pub(crate) fn place_view(&self, project_id: &str, from: &str, size: Size) -> Point {
        self.inner.store.read(|state| {
            let empty = BoardState::default();
            let board = state.board(project_id).unwrap_or(&empty);
            let dock = dock_rect(board.archive);
            let card = active_cards(board)
                .find(|n| n.session_id == from)
                .or_else(|| rightmost_card(board));
            let preferred = match card {
                Some(card) => Point {
                    x: card.x + card_size(Some(card)).w + GAP,
                    y: card.y,
                },
                None => Point {
                    x: dock.x + dock.w + 80.0,
                    y: dock.y,
                },
            };
            find_free_spot(preferred, &obstacles(board, None), Some(dock), size)
        })
    }
}

/// A view's new title, which its brain's card takes too (VIEW-091).
pub(crate) fn set_title(board: &mut BoardState, view_id: &str, title: &str) {
    let Some(view) = board.views.get_mut(view_id) else {
        return;
    };
    view.title = title.to_owned();
    let brain = view.session_id.clone();
    if let Some(node) = brain.and_then(|id| board.nodes.get_mut(&id)) {
        node.title = Some(crate::brains::brain_title(title));
    }
}
