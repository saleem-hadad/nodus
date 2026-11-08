//! The service: the store, the event stream, and what the app was started with.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nodus_board::{Board, BoardSessions, BoxFuture};
use nodus_domain::{AppSettings, Event, EventSink, OpenCardRequest, SessionCard, SessionStatus, SessionUsage};
use nodus_platform::Env;
use nodus_store::Store;
use nodus_transcripts::Transcripts;
use tokio::sync::{broadcast, watch};

use crate::{AppApi, GroupsApi, ProjectsApi, SettingsApi};

/// How many events a slow subscriber may fall behind before it misses some.
const EVENT_BACKLOG: usize = 4096;

/// What `projects.add` without a path waits on: the folder the user picked,
/// or none when they cancelled.
pub type PickFolder = Pin<Box<dyn Future<Output = Option<PathBuf>> + Send>>;

/// The native dialogs the main process showed for a command. The app gives
/// the windowed ones; a headless server has none to give.
pub trait Dialogs: Send + Sync {
    /// The folder picker of Add Project (PLAT-068): title and button
    /// `Add Project`, message `Choose the local repository for this project`,
    /// folders only, with New Folder; a sheet on the main window when there
    /// is one.
    fn pick_folder(&self) -> PickFolder;
}

/// No dialogs: every picker comes back cancelled.
struct NoDialogs;

impl Dialogs for NoDialogs {
    fn pick_folder(&self) -> PickFolder {
        Box::pin(async { None })
    }
}

/// Run every time settings are applied (`applySettings`, PLAT-066): at
/// startup, before the window opens, and after every `settings.update`. The
/// app sets the theme source and syncs View › Focus Mode here; the
/// notifications and capture tracks hang the Dock badge and the double tap
/// on it.
pub type SettingsHook = Arc<dyn Fn(&AppSettings) + Send + Sync>;

/// Where the user's environment and `claude` come from.
pub enum EnvSource {
    /// Read the login shell's environment in the background, as the app does
    /// at startup (PLAT-010 step 4).
    Capture,
    /// This environment, now (tests, and a server that has its own).
    Given(Env),
}

/// No session processes: what the board sees before the core connects the
/// session host (02). Nothing is live, so every card reads done and there's
/// nothing to stop.
struct NoProcesses;

impl BoardSessions for NoProcesses {
    fn is_live(&self, _: &str) -> bool {
        false
    }
    fn status(&self, _: &str) -> SessionStatus {
        SessionStatus::Done
    }
    fn usage(&self, _: &str) -> Option<SessionUsage> {
        None
    }
    fn attach_agents(&self, _: &mut SessionCard, _: &str, _: Option<f64>) {}
    fn stop(&self, _: &str) {}
    fn stop_and_forget(&self, _: &str) {}
    fn kill(&self, _: &str) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn forget_status(&self, _: &str) {}
}

/// What the service starts with.
pub struct NodusConfig {
    /// The data folder: state.json and the rest (PLAT-003, PLAT-004).
    pub user_data: PathBuf,
    pub env: EnvSource,
    /// The windowed dialogs; none for a headless server or a test.
    pub dialogs: Option<Arc<dyn Dialogs>>,
    /// Where the core's own work runs (the login shell, transcript watchers).
    pub runtime: tokio::runtime::Handle,
    /// The home folder whose `.claude/projects` holds the transcripts
    /// (`os.homedir()`).
    pub home: PathBuf,
}

/// Every domain composed into one service. Cheap to clone; clones share it.
#[derive(Clone)]
pub struct Nodus {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) store: Arc<Store>,
    /// The boards: their cards merged with the transcripts on disk (03).
    pub(crate) board: Board,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) dialogs: Arc<dyn Dialogs>,
    pub(crate) settings_hooks: Mutex<Vec<SettingsHook>>,
    /// The card a notification asked to open while no window was there
    /// (`takeOpenRequest`, 02). The notifications track sets it.
    pub(crate) open_request: Mutex<Option<OpenCardRequest>>,
    /// The user's environment, once the login shell has given it.
    pub(crate) env: watch::Receiver<Option<Arc<Env>>>,
}

impl Nodus {
    /// Loads state.json from the data folder and starts reading the user's
    /// environment, as the main process does before its window opens
    /// (PLAT-010 steps 2 and 4).
    pub fn start(config: NodusConfig) -> std::io::Result<Self> {
        let store = Arc::new(Store::open(&config.user_data)?);
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        let board = Board::new(
            Arc::clone(&store),
            Arc::new(Transcripts::new(config.home)),
            Arc::new(NoProcesses),
            Arc::new(Sink(events.clone())),
            config.runtime.clone(),
        );
        let (env_tx, env) = watch::channel(None);
        match config.env {
            EnvSource::Given(env) => {
                env_tx.send_replace(Some(Arc::new(env)));
            }
            EnvSource::Capture => {
                config.runtime.spawn(async move {
                    let env = Env::init().await;
                    env_tx.send_replace(Some(Arc::new(env)));
                });
            }
        }
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                board,
                events,
                dialogs: config.dialogs.unwrap_or_else(|| Arc::new(NoDialogs)),
                settings_hooks: Mutex::new(Vec::new()),
                open_request: Mutex::new(None),
                env,
            }),
        })
    }

    /// `window.api` (top level): app info, the open-card request, menu commands.
    pub fn app(&self) -> AppApi<'_> {
        AppApi { nodus: self }
    }

    /// `window.api.settings`.
    pub fn settings(&self) -> SettingsApi<'_> {
        SettingsApi { nodus: self }
    }

    /// `window.api.projects`.
    pub fn projects(&self) -> ProjectsApi<'_> {
        ProjectsApi { nodus: self }
    }

    /// `window.api.groups`.
    pub fn groups(&self) -> GroupsApi<'_> {
        GroupsApi { nodus: self }
    }

    /// Every event from now on (`window.api.on.*`). Each subscriber gets each
    /// event; drop the receiver to unsubscribe (PLAT-127).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    /// Where domains send their events, to reach every subscriber.
    pub fn sink(&self) -> Arc<dyn EventSink> {
        Arc::new(Sink(self.inner.events.clone()))
    }

    /// Sends an event to every subscriber. Dropped when no one listens, as
    /// 0.9.4 drops events while there's no window (PLAT-123).
    pub fn emit(&self, event: Event) {
        let _ = self.inner.events.send(event);
    }

    /// Runs `hook` every time settings are applied, and once now with the
    /// current settings, so the window opens in the saved theme (UI-004).
    pub fn on_settings_applied(&self, hook: SettingsHook) {
        let settings = self.inner.store.read(|state| state.settings.clone());
        hook(&settings);
        lock(&self.inner.settings_hooks).push(hook);
    }

    /// Records the card a notification asked to open, for `takeOpenRequest`.
    pub fn set_open_request(&self, request: Option<OpenCardRequest>) {
        *lock(&self.inner.open_request) = request;
    }

    /// What quitting does before the app goes (PLAT-032): stops watching
    /// transcripts (step 3) and writes a pending state.json save (step 5).
    /// Letting go of the session host (step 2) comes with the host wiring.
    pub fn shutdown(&self) {
        self.inner.board.watcher().unwatch_all();
        self.inner.store.flush();
    }

    /// The user's environment, once the login shell has given it (PLAT-128).
    pub(crate) async fn env(&self) -> Arc<Env> {
        let mut env = self.inner.env.clone();
        if let Ok(env) = env.wait_for(Option::is_some).await
            && let Some(env) = env.as_ref()
        {
            return Arc::clone(env);
        }
        // The capture task ended without an answer (its runtime went away).
        Arc::new(Env::from_process())
    }

    pub(crate) fn apply_settings(&self, settings: &AppSettings) {
        let hooks = lock(&self.inner.settings_hooks).clone();
        for hook in hooks {
            hook(settings);
        }
    }
}

struct Sink(broadcast::Sender<Event>);

impl EventSink for Sink {
    fn emit(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
