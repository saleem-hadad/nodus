//! The service: every domain composed into one, started and stopped in the
//! order the main process does it (`src/main/index.ts`, `registerIpc` and
//! `shutdown` in `src/main/ipc.ts`).

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use nodus_actions::Actions;
use nodus_board::Board;
use nodus_domain::{AppSettings, BoardNode, Event, EventSink, OpenCardRequest, SessionStatus};
use nodus_platform::Env;
use nodus_platform::secrets::SafeStorage;
use nodus_sessions::agents::Agents;
use nodus_sessions::client::{ClientError, HostClient, HostLauncher, ProcessLauncher, Wiring};
use nodus_sessions::hooks::{self, HookEndpoint, StatusMachine, SystemClock};
use nodus_sessions::launch::{self, SpawnOptions};
use nodus_sessions::notify::{NotificationId, Notifier, Notify};
use nodus_store::Store;
use nodus_tools::{McpRelay, McpServer, StoreHandlers, StoreHandlersConfig};
use nodus_transcripts::Transcripts;
use nodus_updater::{RealSystem, Updater};
use nodus_views::{Views, ViewsConfig};
use nodus_voice::Voice;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, watch};

use crate::notifications::{NoNotifier, Notifications, SystemNotifier};
use crate::shell::{AppShell, Headless, ShellPrompt, ShellWindow};
use crate::wiring::{
    Broadcast, CoreBrains, CoreMetas, CoreProjects, CoreSessions, CoreUpdaterApp, CoreViewHost, CoreViewTools,
    StatusChange, StatusFanout,
};
use crate::{
    ActionsApi, AppApi, BoardApi, GitApi, GroupsApi, MenuApi, ProjectsApi, SessionsApi, SettingsApi, UpdatesApi,
    ViewsApi, VoiceApi,
};

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

/// What the service starts with.
pub struct NodusConfig {
    /// The data folder: state.json and the rest (PLAT-003, PLAT-004).
    pub user_data: PathBuf,
    pub env: EnvSource,
    /// The windowed dialogs; none for a headless server or a test.
    pub dialogs: Option<Arc<dyn Dialogs>>,
    /// Where the core's own work runs (the login shell, transcript watchers,
    /// the session host client, timers).
    pub runtime: Handle,
    /// The home folder whose `.claude/projects` holds the transcripts
    /// (`os.homedir()`).
    pub home: PathBuf,
    /// The app around the core: its window, quitting, and the question about
    /// another version's sessions. None for a headless Nodus ([`Headless`]).
    pub shell: Option<Arc<dyn AppShell>>,
    /// How the user hears about sessions while the app is in the background.
    pub notifications: Notifications,
    /// How a session host is started when none is running. None runs this
    /// binary as `Nodus host <socket> <key>` (HOST-091).
    pub host_launcher: Option<Arc<dyn HostLauncher>>,
    /// GitHub and the OS as the updater reaches them. None for the real ones.
    pub updates: Option<Arc<dyn nodus_updater::System>>,
    /// Where the xAI key is sealed (0.9.4's safeStorage). None for the
    /// system's: the Keychain on macOS, `Local State` and DPAPI on Windows.
    pub secrets: Option<Arc<SafeStorage>>,
}

impl NodusConfig {
    /// A headless service on `user_data`: the login shell's environment, the
    /// user's home folder, no window, dialogs or notifications, and hosts
    /// started from this binary. Change what differs with struct update
    /// syntax.
    pub fn new(user_data: impl Into<PathBuf>, runtime: Handle) -> Self {
        Self {
            user_data: user_data.into(),
            env: EnvSource::Capture,
            dialogs: None,
            runtime,
            home: std::env::home_dir().unwrap_or_default(),
            shell: None,
            notifications: Notifications::Off,
            host_launcher: None,
            updates: None,
            secrets: None,
        }
    }
}

/// The user chose Quit when sessions from another version were running: the
/// app leaves at once, with code 0 and without its quit path (HOST-103).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the user chose to quit")]
pub struct QuitAtOnce;

/// Every domain composed into one service. Cheap to clone; clones share it.
#[derive(Clone)]
pub struct Nodus {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) store: Arc<Store>,
    /// The boards: their cards merged with the transcripts on disk (03).
    pub(crate) board: Arc<Board>,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) dialogs: Arc<dyn Dialogs>,
    pub(crate) settings_hooks: Mutex<Vec<SettingsHook>>,
    /// A card to open that someone set with [`Nodus::set_open_request`];
    /// notifications keep their own.
    pub(crate) open_request: Mutex<Option<OpenCardRequest>>,
    /// The user's environment, once the login shell has given it.
    pub(crate) env: watch::Receiver<Option<Arc<Env>>>,
    pub(crate) runtime: Handle,
    pub(crate) user_data: PathBuf,
    pub(crate) shell: Arc<dyn AppShell>,
    /// The session host and the sessions in it (`pty.ts`).
    pub(crate) host: HostClient,
    /// Each session's status, agents and usage (`hooks.ts`).
    pub(crate) status: StatusMachine,
    /// Where claude's hooks post (`hookSettings`, `hookEnv`).
    pub(crate) endpoint: Arc<HookEndpoint>,
    /// Subagents (`agents.ts`).
    pub(crate) agents: Agents,
    pub(crate) views: Views,
    pub(crate) notify: Notify,
    pub(crate) updater: Arc<Updater>,
    /// The actions rail's buttons (05).
    pub(crate) actions: Actions,
    /// The xAI key and transcribing recordings (05).
    pub(crate) voice: Voice,
    /// Floating capture (07).
    pub(crate) capture: crate::capture::CaptureCore,
    /// `BOARD_TOOL_RULES`, as arguments.
    board_tool_rules: Vec<String>,
}

impl Nodus {
    /// Starts the service as the main process starts before its window opens
    /// (PLAT-010 steps 2 to 7): loads state.json, starts reading the user's
    /// environment, sets up the session host client, the status machine,
    /// notifications, the boards, views and board tools, and starts the
    /// updater. Settings hooks the app adds run at once (step 3).
    ///
    /// Then await [`Nodus::launch`] (steps 8 and 9) before the window opens.
    pub fn start(config: NodusConfig) -> std::io::Result<Self> {
        let store = Arc::new(Store::open(&config.user_data)?);
        // Timers and tasks the domains start as they're made run here.
        let _runtime = config.runtime.enter();
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
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
        let host_launcher = match config.host_launcher {
            Some(launcher) => launcher,
            None => Arc::new(ProcessLauncher::current_exe()?),
        };
        let shell = config.shell.unwrap_or_else(|| Arc::new(Headless));
        let (changes, status_changes) = mpsc::unbounded_channel();
        let inner = Arc::new_cyclic(|core: &Weak<Inner>| {
            let broadcast = Broadcast(events.clone());
            let sink: Arc<dyn EventSink> = Arc::new(broadcast.clone());
            let sessions = Arc::new(CoreSessions(core.clone()));
            let transcripts = Arc::new(Transcripts::new(config.home.clone()));
            let board = Arc::new(Board::new(
                Arc::clone(&store),
                Arc::clone(&transcripts),
                Arc::clone(&sessions) as _,
                Arc::clone(&sink),
                config.runtime.clone(),
            ));
            let views = Views::new(ViewsConfig {
                user_data: config.user_data.clone(),
                store: Arc::clone(&store),
                board: Arc::clone(&board),
                sessions: Arc::clone(&sessions) as _,
                brains: Arc::new(CoreBrains(core.clone())),
                host: Arc::new(CoreViewHost(core.clone())),
                events: Arc::clone(&sink),
                env: env.clone(),
                clock: nodus_board::real_clock(),
                runtime: config.runtime.clone(),
            });
            let tools = StoreHandlers::new(StoreHandlersConfig {
                store: Arc::clone(&store),
                board: Arc::clone(&board),
                sessions: Arc::clone(&sessions) as _,
                events: Arc::clone(&sink),
                projects: Arc::new(CoreProjects(core.clone())),
                views: Arc::new(CoreViewTools(views.clone())),
                clock: nodus_board::real_clock(),
                home: Some(config.home.clone()),
            });
            let server = Arc::new(McpServer::new(crate::VERSION, nodus_views::VIEW_GUIDE, Some(tools)));
            let agents = Agents::new(&config.home, Arc::new(SystemClock));
            let pending_work = agents.clone();
            let status = StatusMachine::new(
                Arc::new(StatusFanout {
                    events: broadcast.clone(),
                    changes,
                }),
                Arc::new(SystemClock),
                move |transcript| pending_work.has_pending_work(transcript),
            );
            let endpoint = Arc::new(HookEndpoint::new());
            let host = HostClient::new(
                &config.user_data,
                Wiring {
                    events: Arc::clone(&sink),
                    status: Arc::new(status.clone()),
                    endpoint: Arc::clone(&endpoint),
                    board_tools: Arc::new(McpRelay(server)),
                    prompt: Arc::new(ShellPrompt(Arc::clone(&shell))),
                    launcher: host_launcher,
                },
            );
            let notifier: Arc<dyn Notifier> = match config.notifications {
                Notifications::Off => Arc::new(NoNotifier),
                Notifications::System => Arc::new(SystemNotifier::new(core.clone())),
                Notifications::Custom(notifier) => notifier,
            };
            let notify = Notify::new(
                Arc::clone(&store),
                status.clone(),
                notifier,
                Arc::new(ShellWindow(Arc::clone(&shell))),
                Arc::new(CoreMetas(transcripts)),
                Arc::clone(&sink),
            );
            let capture = crate::capture::CaptureCore::new(
                core,
                &store,
                config.user_data.clone(),
                env.clone(),
                Arc::clone(&sink),
                &config.runtime,
            );
            let updater = Updater::new(
                Arc::new(CoreUpdaterApp(core.clone())),
                config
                    .updates
                    .unwrap_or_else(|| Arc::new(RealSystem::new(std::env::current_exe().unwrap_or_default()))),
                sink,
            );
            Inner {
                store: Arc::clone(&store),
                board,
                events: events.clone(),
                dialogs: config.dialogs.unwrap_or_else(|| Arc::new(NoDialogs)),
                settings_hooks: Mutex::new(Vec::new()),
                open_request: Mutex::new(None),
                env,
                runtime: config.runtime.clone(),
                user_data: config.user_data.clone(),
                shell,
                host,
                status,
                endpoint,
                agents,
                views,
                notify,
                updater,
                actions: Actions::new(Arc::clone(&store)),
                voice: Voice::new(
                    Arc::clone(&store),
                    config
                        .secrets
                        .unwrap_or_else(|| Arc::new(SafeStorage::system(&config.user_data))),
                ),
                capture,
                board_tool_rules: nodus_tools::BOARD_TOOL_RULES.iter().map(|&r| r.to_owned()).collect(),
            }
        });
        config
            .runtime
            .spawn(follow_status(Arc::downgrade(&inner), status_changes));
        let nodus = Self { inner };
        // PLAT-066 step 2: the Dock badge is recounted whenever settings are
        // applied (nothing until notifications are armed, at launch).
        let core = Arc::downgrade(&nodus.inner);
        nodus.on_settings_applied(Arc::new(move |_| {
            if let Some(core) = core.upgrade() {
                core.notify.update_badge();
            }
        }));
        // VIEW-085: views' schedules tick from now on.
        nodus.inner.views.start_schedules();
        // PLAT-010 step 7.
        nodus.inner.updater.start();
        Ok(nodus)
    }

    /// The rest of the launch, before the window opens (PLAT-010 steps 8 and
    /// 9, HOST-221): adopts the sessions a running session host has, without
    /// starting one, then arms notifications so what the adoption replayed
    /// is never news. A host that can't be reached is logged as `[host]
    /// connect failed` and the launch carries on; the window waits for this
    /// either way (PLAT-011).
    ///
    /// Subscribe after this, as the window does: the statuses the adoption
    /// replays reach no window, and the window reads them from board loads.
    ///
    /// `Err` when the user chose Quit over another version's sessions: the
    /// shell has been asked to exit at once (HOST-103).
    pub async fn launch(&self) -> Result<(), QuitAtOnce> {
        self.on_core(async {
            match self.inner.host.connect_host(false).await {
                Ok(()) => {}
                Err(ClientError::Quit) => {
                    self.inner.shell.exit_now();
                    return Err(QuitAtOnce);
                }
                Err(ClientError::Failed(err)) => tracing::error!("[host] connect failed {err}"),
            }
            self.inner.notify.arm();
            Ok(())
        })
        .await
    }

    /// Runs `work` in the core's tokio runtime's context, whichever executor
    /// polls it: the window awaits commands on GPUI's, and they start tasks,
    /// timers, processes and HTTP requests on tokio's. Every async command
    /// goes through this, so the API can be awaited from anywhere.
    pub(crate) fn on_core<F: Future>(&self, work: F) -> OnCore<F> {
        OnCore {
            runtime: self.inner.runtime.clone(),
            work: Box::pin(work),
        }
    }

    /// What quitting does before the app goes, on every path that quits
    /// normally (PLAT-032, HOST-225): lets go of the session host without
    /// stopping its sessions (step 2), stops watching transcripts (step 3),
    /// writes views' pending runtime and stops their schedules (step 4), and
    /// writes a pending state.json save (step 5). Then, with an update
    /// downloaded on macOS, starts the swap that puts it in once the app has
    /// gone (UPD-057, `will-quit`). Capture's key hook (step 1) is capture's.
    pub fn shutdown(&self) {
        // Step 1: capture's key hook and permission poll (CAP-039).
        self.inner.capture.stop();
        self.inner.host.disconnect();
        self.inner.board.watcher().unwatch_all();
        self.inner.views.flush();
        self.inner.store.flush();
        self.inner.updater.on_will_quit();
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

    /// `window.api.board`.
    pub fn board(&self) -> BoardApi<'_> {
        BoardApi { nodus: self }
    }

    /// `window.api.sessions`.
    pub fn sessions(&self) -> SessionsApi<'_> {
        SessionsApi { nodus: self }
    }

    /// `window.api.views`, and the pages their frames load.
    pub fn views(&self) -> ViewsApi<'_> {
        ViewsApi { nodus: self }
    }

    /// `window.api.updates`.
    pub fn updates(&self) -> UpdatesApi<'_> {
        UpdatesApi { nodus: self }
    }

    /// `window.api.capture` (07).
    pub fn capture(&self) -> crate::CaptureApi<'_> {
        crate::CaptureApi { nodus: self }
    }

    /// The values 0.9.4's pages kept in `localStorage` (CAP-157).
    pub fn local_storage(&self) -> crate::LocalStorageApi<'_> {
        crate::LocalStorageApi { nodus: self }
    }

    /// The UI's page zoom (PLAT-057, PLAT-139).
    pub fn zoom(&self) -> crate::ZoomApi<'_> {
        crate::ZoomApi { nodus: self }
    }

    /// What the application menu's own items do (01): Check for Updates…
    /// and Quit and Stop Sessions.
    pub fn menu(&self) -> MenuApi<'_> {
        MenuApi { nodus: self }
    }

    /// `window.api.git`.
    pub fn git(&self) -> GitApi<'_> {
        GitApi { nodus: self }
    }

    /// `window.api.actions`.
    pub fn actions(&self) -> ActionsApi<'_> {
        ActionsApi { nodus: self }
    }

    /// `window.api.voice`.
    pub fn voice(&self) -> VoiceApi<'_> {
        VoiceApi { nodus: self }
    }

    /// Every event from now on (`window.api.on.*`). Each subscriber gets each
    /// event; drop the receiver to unsubscribe (PLAT-127).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    /// Where domains send their events, to reach every subscriber.
    pub fn sink(&self) -> Arc<dyn EventSink> {
        Arc::new(Broadcast(self.inner.events.clone()))
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

    /// Records a card to open, for `takeOpenRequest`.
    pub fn set_open_request(&self, request: Option<OpenCardRequest>) {
        *lock(&self.inner.open_request) = request;
    }

    /// `notify.openCard` (HOST-212): brings the window up on a card, or a
    /// project's board. Capture's open uses it (07).
    pub fn open_card(&self, request: OpenCardRequest) {
        self.inner.notify.open_card(request);
    }

    /// The user clicked a notification a [`Notifications::Custom`] notifier
    /// showed (HOST-211). The system's report their clicks themselves.
    pub fn notification_clicked(&self, id: NotificationId) {
        self.inner.notify.clicked(id);
    }

    /// The card to open that's waiting for the window, once (UI-044).
    pub(crate) fn take_open_request(&self) -> Option<OpenCardRequest> {
        self.inner
            .notify
            .take_open_request()
            .or_else(|| lock(&self.inner.open_request).take())
    }

    /// The user's environment, once the login shell has given it (PLAT-128).
    pub(crate) async fn env(&self) -> Arc<Env> {
        self.inner.env().await
    }

    pub(crate) fn apply_settings(&self, settings: &AppSettings) {
        let hooks = lock(&self.inner.settings_hooks).clone();
        for hook in hooks {
            hook(settings);
        }
    }

    /// What `settings:update` does after applying and saving (PLAT-065 step
    /// 4, UPD-013): turning automatic updates on checks straight away.
    pub(crate) fn settings_updated(&self, patch: &serde_json::Value) {
        if truthy(patch.get("autoUpdate")) {
            let updater = Arc::clone(&self.inner.updater);
            self.inner.runtime.spawn(async move { updater.check(false).await });
        }
    }

    /// What goes before a project leaves state.json (PLAT-075 steps 1 to 3):
    /// its processes are stopped (not waited for), its transcripts no longer
    /// watched, and its views' folders deleted.
    pub(crate) fn project_removing(&self, project_id: &str) {
        let inner = &self.inner;
        inner.host.kill_project(project_id);
        inner.board.watcher().unwatch_project(project_id);
        let board = inner.store.read(|state| state.board(project_id).cloned());
        inner.views.remove_project_views(board.as_ref());
    }
}

impl Inner {
    /// The user's environment, once the login shell has given it (PLAT-128).
    pub(crate) async fn env(&self) -> Arc<Env> {
        let mut env = self.env.clone();
        if let Ok(env) = env.wait_for(Option::is_some).await
            && let Some(env) = env.as_ref()
        {
            return Arc::clone(env);
        }
        // The capture task ended without an answer (its runtime went away).
        Arc::new(Env::from_process())
    }

    /// `pty.spawnSession` (SESS-119…131): starts a session's process in the
    /// session host, connecting to (or starting) one first. Its command line
    /// is built once connected, since the hook endpoint comes from the host,
    /// and the board tools go only to a host that relays them (HOST-107). A
    /// live session is left as it is, and starts of one session at once share
    /// the first (SESS-130). The error is shown as it is.
    pub(crate) async fn spawn_session(&self, opts: SpawnOptions) -> Result<(), String> {
        let env = self.env().await;
        let launch_env = launch::LaunchEnv::from_env(&env);
        let powershell = hooks::uses_powershell(&env);
        let endpoint = Arc::clone(&self.endpoint);
        let rules = self.board_tool_rules.clone();
        let spawned = self
            .host
            .spawn_session(&opts, move |opts, host| {
                let settings = endpoint.settings(powershell);
                let hook_env = endpoint.env(&opts.session_id);
                let tools = host.relays_mcp.then_some(launch::BoardTools {
                    mcp_config: nodus_tools::BOARD_MCP_CONFIG,
                    allowed_tools: &rules,
                });
                launch::command_for(
                    opts,
                    &launch_env,
                    launch::Hooks {
                        settings: &settings,
                        env: &hook_env,
                    },
                    tools,
                )
                .map_err(|err| err.to_string())
            })
            .await;
        match spawned {
            Ok(()) => Ok(()),
            Err(ClientError::Failed(err)) => Err(err),
            Err(ClientError::Quit) => {
                self.shell.exit_now();
                Err(QuitAtOnce.to_string())
            }
        }
    }

    /// A view's brain's extra arguments, every time it starts (SESS-133).
    pub(crate) fn brain_args(&self, project_id: &str, node: &BoardNode) -> Option<Vec<String>> {
        self.store.read(|state| {
            state
                .board(project_id)
                .and_then(|board| nodus_views::brain_args(board, node))
        })
    }
}

/// HOST-220, SESS-154: what follows each status change, in order, after
/// `ev:status` has gone to the window: the notifier sees it, then, when it's
/// `done`, an action run finishes and a session that asked to be is
/// archived, then the view brain hook. Runs on a task of its own, so the
/// status machine, the board and the session host never wait on it.
async fn follow_status(core: Weak<Inner>, mut changes: mpsc::UnboundedReceiver<StatusChange>) {
    while let Some(change) = changes.recv().await {
        let Some(core) = core.upgrade() else {
            return;
        };
        let StatusChange {
            session_id,
            status,
            live,
        } = change;
        // Before a finished run leaves the board, while it can still be named.
        core.notify.status_changed(&session_id, status, live);
        if status == SessionStatus::Done {
            core.board.on_done(&session_id);
        }
        core.views.brain_status(&session_id, status, live);
    }
}

/// A future polled inside the core's runtime context ([`Nodus::on_core`]).
pub(crate) struct OnCore<F: Future> {
    runtime: Handle,
    work: Pin<Box<F>>,
}

impl<F: Future> Future for OnCore<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
        let this = &mut *self;
        let _context = this.runtime.enter();
        this.work.as_mut().poll(cx)
    }
}

/// Whether JavaScript counts the value as true.
fn truthy(value: Option<&serde_json::Value>) -> bool {
    use serde_json::Value;
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
