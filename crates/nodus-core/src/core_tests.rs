//! The glue between the domains: what follows a status change, a project
//! leaving, settings and the updater, and the quit paths. Headless, with no
//! session host: the status machine is driven directly.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{
    ActionRunResult, BoardNode, Event, MenuCommand, ProjectAction, SessionStatus, SettingsPatch, UpdateState,
};
use nodus_sessions::notify::{Notification, NotificationId, Notifier};
use nodus_updater::{Body, BoxFuture as UpdaterFuture, HttpResponse, LATEST_URL, Platform, RunOutput, System};
use serde_json::{Map, json};
use tokio::sync::broadcast;

use crate::shell::BoxFuture;
use crate::{AppShell, EnvSource, Nodus, NodusConfig, Notifications, ReplaceChoice};

/// Remembers every notification shown.
#[derive(Default)]
struct Shown(Mutex<Vec<Notification>>);

impl Notifier for Shown {
    fn is_supported(&self) -> bool {
        true
    }
    fn show(&self, _: NotificationId, notification: &Notification) {
        self.0.lock().unwrap().push(notification.clone());
    }
    fn close(&self, _: NotificationId) {}
    fn has_dock(&self) -> bool {
        false
    }
    fn set_badge(&self, _: &str) {}
}

/// A shell whose window is closed, counting how often it was asked to quit.
#[derive(Default)]
struct Shell {
    quits: AtomicUsize,
}

impl AppShell for Shell {
    fn is_open(&self) -> bool {
        false
    }
    fn is_focused(&self) -> bool {
        false
    }
    fn is_loading(&self) -> bool {
        false
    }
    fn show(&self) {}
    fn quit(&self) {
        self.quits.fetch_add(1, Ordering::SeqCst);
    }
    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        Box::pin(async { ReplaceChoice::StopSessions })
    }
}

/// GitHub with this version as its latest, on a packaged Windows build (never off).
struct LatestIsThis;

struct Once(Option<Vec<u8>>);

impl Body for Once {
    fn next_chunk(&mut self) -> UpdaterFuture<'_, Option<io::Result<Vec<u8>>>> {
        let chunk = self.0.take().map(Ok);
        Box::pin(async move { chunk })
    }
}

impl System for LatestIsThis {
    fn platform(&self) -> Platform {
        Platform::Windows
    }
    fn is_packaged(&self) -> bool {
        true
    }
    fn arm64_machine(&self) -> bool {
        false
    }
    fn bundle_path(&self) -> PathBuf {
        PathBuf::new()
    }
    fn is_in_applications_folder(&self) -> bool {
        true
    }
    fn get(&self, url: &str, _: Vec<(&'static str, String)>) -> UpdaterFuture<'_, io::Result<HttpResponse>> {
        let latest = url == LATEST_URL;
        Box::pin(async move {
            let body = json!({ "tag_name": format!("v{}", crate::VERSION), "assets": [] }).to_string();
            Ok(HttpResponse {
                status: if latest { 200 } else { 404 },
                body: Some(Box::new(Once(Some(body.into_bytes())))),
            })
        })
    }
    fn run(&self, _: &str, _: Vec<String>) -> UpdaterFuture<'_, Result<RunOutput, String>> {
        Box::pin(async { Ok(RunOutput::default()) })
    }
    fn spawn_detached(&self, _: &Path, _: Vec<String>) -> io::Result<()> {
        Ok(())
    }
}

struct World {
    dir: tempfile::TempDir,
    nodus: Nodus,
    shown: Arc<Shown>,
    shell: Arc<Shell>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Shown::default());
    let shell = Arc::new(Shell::default());
    let nodus = Nodus::start(NodusConfig {
        env: EnvSource::Given(nodus_platform::Env::from_process()),
        home: dir.path().join("home"),
        shell: Some(Arc::clone(&shell) as _),
        notifications: Notifications::Custom(Arc::clone(&shown) as _),
        updates: Some(Arc::new(LatestIsThis)),
        secrets: Some(Arc::new(crate::SafeStorage::with_mac_password(b"synthetic"))),
        ..NodusConfig::new(dir.path().join("userData"), tokio::runtime::Handle::current())
    })
    .unwrap();
    World {
        dir,
        nodus,
        shown,
        shell,
    }
}

async fn next(events: &mut broadcast::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("an event")
        .unwrap()
}

/// The next event that isn't the updater's.
async fn next_board_event(events: &mut broadcast::Receiver<Event>) -> Event {
    loop {
        match next(events).await {
            Event::UpdateStatus(_) => {}
            other => return other,
        }
    }
}

fn node(session_id: &str) -> BoardNode {
    BoardNode {
        session_id: session_id.into(),
        kind: None,
        x: 0.0,
        y: 0.0,
        archived: false,
        title: None,
        parents: None,
        created_by_app: Some(true),
        run_of: None,
        expanded: None,
        view_of: None,
        auto_mode: None,
        from_view: None,
        created_at: 1.0,
        w: None,
        h: None,
        archived_at: None,
        section_id: None,
        extra: Map::new(),
    }
}

async fn project(world: &World) -> nodus_domain::Project {
    let folder = world.dir.path().join("shop");
    std::fs::create_dir_all(&folder).unwrap();
    world
        .nodus
        .projects()
        .add(Some(folder.to_string_lossy().into_owned()))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_220_sess_154_the_window_hears_first_then_notifications_then_the_run_finishes() {
    let world = world();
    let project = project(&world).await;
    world.nodus.inner.store.update(|state| {
        let p = state.project_mut(&project.id).unwrap();
        p.actions = Some(vec![ProjectAction {
            id: "a1".into(),
            name: "Deploy".into(),
            icon: "rocket".into(),
            description: String::new(),
            prompt: "Ship it".into(),
            global: None,
            created_at: 1.0,
            extra: Map::new(),
        }]);
        let mut run = node("run-1");
        run.run_of = Some("a1".into());
        run.title = Some("Deploy now".into());
        state.board_mut(&project.id).nodes.insert("run-1".into(), run);
    });
    // HOST-221: no host to adopt, so the launch just arms notifications.
    world.nodus.launch().await.unwrap();
    let mut events = world.nodus.subscribe();

    let status = &world.nodus.inner.status;
    status.process_started("run-1", SessionStatus::Working);
    status.receive_hook("Stop", "run-1", &json!({}), None);

    assert_eq!(
        next_board_event(&mut events).await,
        Event::Status {
            session_id: "run-1".into(),
            status: SessionStatus::Working,
            live: true
        }
    );
    assert_eq!(
        next_board_event(&mut events).await,
        Event::Status {
            session_id: "run-1".into(),
            status: SessionStatus::Done,
            live: true
        }
    );
    assert_eq!(
        next_board_event(&mut events).await,
        Event::RunFinished(ActionRunResult {
            project_id: project.id.clone(),
            session_id: "run-1".into(),
            action_name: "Deploy".into(),
            message: None,
        })
    );
    assert_eq!(
        next_board_event(&mut events).await,
        Event::BoardChanged {
            project_id: project.id.clone()
        }
    );
    // The notification went out while the run was still on the board to be named.
    let shown = world.shown.0.lock().unwrap().clone();
    assert_eq!(
        shown,
        [Notification {
            title: "Deploy now".into(),
            subtitle: "shop".into(),
            body: "Finished".into(),
            group_id: project.id.clone(),
        }]
    );
    // SESS-155: off the board and hidden.
    world.nodus.inner.store.read(|state| {
        let board = state.board(&project.id).unwrap();
        assert!(board.nodes.is_empty());
        assert_eq!(board.hidden, ["run-1"]);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sess_157_a_session_that_asked_is_archived_when_its_turn_ends() {
    let world = world();
    let project = project(&world).await;
    world.nodus.inner.store.update(|state| {
        state.board_mut(&project.id).nodes.insert("s1".into(), node("s1"));
    });
    world.nodus.inner.board.archive_when_done("s1");
    let mut events = world.nodus.subscribe();
    let status = &world.nodus.inner.status;
    status.process_started("s1", SessionStatus::Working);
    status.receive_hook("Stop", "s1", &json!({}), None);
    loop {
        if let Event::BoardChanged { project_id } = next_board_event(&mut events).await {
            assert_eq!(project_id, project.id);
            break;
        }
    }
    let archived = world
        .nodus
        .inner
        .store
        .read(|state| state.board(&project.id).unwrap().nodes["s1"].archived);
    assert!(archived);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sess_203_events_nobody_listens_for_are_dropped() {
    let world = world();
    world.nodus.emit(Event::ProjectsChanged);
    let mut events = world.nodus.subscribe();
    world.nodus.app().menu_command("search").await;
    assert_eq!(next_board_event(&mut events).await, Event::Menu(MenuCommand::Search));
}

/// View ids are UUIDs: they end up in paths.
const VIEW: &str = "0b0e9c3e-1f7a-4d1e-9a51-6c2f3b8d4e10";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plat_075_a_removed_project_takes_its_views_folders_with_it() {
    let world = world();
    let project = project(&world).await;
    let view = world.nodus.inner.store.update(|state| {
        let view: nodus_domain::BoardView = serde_json::from_value(json!({
            "id": VIEW, "title": "Stats", "description": "d", "x": 0, "y": 0,
            "version": 1, "createdAt": 1, "updatedAt": 1
        }))
        .unwrap();
        state.board_mut(&project.id).views.insert(VIEW.into(), view.clone());
        view
    });
    let folder = world.dir.path().join("userData/views").join(&view.id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("index.html"), "<p>hi</p>").unwrap();
    world.nodus.projects().remove(&project.id).await;
    assert!(!folder.exists());
    assert!(world.nodus.projects().list().await.is_empty());
    assert!(world.nodus.inner.store.read(|state| state.boards.is_empty()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upd_013_turning_automatic_updates_on_checks_at_once() {
    let world = world();
    let mut events = world.nodus.subscribe();
    world
        .nodus
        .settings()
        .update(SettingsPatch {
            auto_update: Some(true),
            ..SettingsPatch::default()
        })
        .await;
    let mut states = Vec::new();
    while states.last() != Some(&UpdateState::Latest) {
        if let Event::UpdateStatus(status) = next(&mut events).await {
            states.push(status.state);
        }
    }
    assert_eq!(states, [UpdateState::Checking, UpdateState::Latest]);
    // Turning it off checks nothing.
    world
        .nodus
        .settings()
        .update(SettingsPatch {
            auto_update: Some(false),
            ..SettingsPatch::default()
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(events.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upd_076_078_status_check_and_install() {
    let world = world();
    let updates = world.nodus.updates();
    assert_eq!(updates.status().await.current, crate::VERSION);
    // UPD-077: done once the lookup is.
    updates.check().await;
    assert_eq!(updates.status().await.state, UpdateState::Latest);
    // UPD-078: nothing is downloaded, so there's nothing to install.
    assert_eq!(updates.install().await, crate::RestartOutcome::Nothing);
    assert_eq!(world.shell.quits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upd_079_check_for_updates_opens_settings_then_checks() {
    let world = world();
    let mut events = world.nodus.subscribe();
    world.nodus.menu().check_for_updates().await;
    assert_eq!(next(&mut events).await, Event::Menu(MenuCommand::UpdateSettings));
    assert_eq!(world.nodus.updates().status().await.state, UpdateState::Latest);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plat_034_host_225_quit_and_stop_sessions_quits_with_no_host_at_once() {
    let world = world();
    tokio::time::timeout(Duration::from_secs(2), world.nodus.menu().quit_and_stop_sessions())
        .await
        .expect("no host to wait for");
    assert_eq!(world.shell.quits.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plat_032_shutdown_writes_state_json_at_once() {
    let world = world();
    let file = world.dir.path().join("userData/state.json");
    world.nodus.groups().create("Infra").await;
    world.nodus.shutdown();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("\"name\": \"Infra\""), "{saved}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_212_213_a_card_to_open_waits_for_a_window_that_was_closed() {
    let world = world();
    let project = project(&world).await;
    let mut events = world.nodus.subscribe();
    let request = nodus_domain::OpenCardRequest {
        project_id: project.id.clone(),
        session_id: Some("s1".into()),
    };
    // The window is closed: it's shown, and picks the card up once it has loaded.
    world.nodus.open_card(request.clone());
    assert_eq!(world.nodus.app().take_open_request().await, Some(request));
    assert_eq!(world.nodus.app().take_open_request().await, None);
    assert!(events.try_recv().is_err());
}

/// Polls a future to its end on this thread, outside any tokio runtime, as
/// GPUI's executor polls the window's commands.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commands_can_be_awaited_from_another_executor() {
    let world = world();
    let nodus = world.nodus.clone();
    // A plain thread, with no tokio runtime of its own: the updater's lookup
    // (timers, HTTP), a kill's 4 s wait and Quit and Stop Sessions still run.
    let states = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        block_on(nodus.updates().check());
        block_on(nodus.sessions().kill("not-live"));
        block_on(nodus.sessions().stop(&["a".into(), "b".into()]));
        block_on(nodus.menu().quit_and_stop_sessions());
        block_on(nodus.updates().status()).state
    })
    .join()
    .unwrap();
    assert_eq!(states, UpdateState::Latest);
    assert_eq!(world.shell.quits.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn act_001_015_actions_are_saved_moved_and_removed() {
    let world = world();
    let project = project(&world).await;
    let actions = world.nodus.actions();
    assert!(actions.global().await.is_empty());
    let err = actions.save(&project.id, &json!({ "name": "Ship" })).await.unwrap_err();
    assert_eq!(
        err.ipc_message(),
        "Error invoking remote method 'actions:save': Error: An action needs a prompt"
    );
    let lists = actions
        .save(
            &project.id,
            &json!({ "id": "a1", "name": " Ship ", "prompt": "Deploy it", "icon": "rocket" }),
        )
        .await
        .unwrap();
    let own = lists.project.actions.clone().unwrap();
    assert_eq!((own[0].id.as_str(), own[0].name.as_str()), ("a1", "Ship"));
    assert!(lists.global.is_empty());
    // Made global, it moves out of the project's list.
    let lists = actions
        .save(
            &project.id,
            &json!({ "id": "a1", "name": "Ship", "prompt": "Deploy it", "global": true }),
        )
        .await
        .unwrap();
    assert_eq!(lists.project.actions, None);
    assert_eq!(actions.global().await.len(), 1);
    let lists = actions.remove(&project.id, "a1").await.unwrap();
    assert!(lists.global.is_empty());
    let err = actions.remove("nope", "a1").await.unwrap_err();
    assert_eq!(err.message, "Unknown project nope");
    // ACT-016: the project is checked before anything is drafted.
    let err = actions.draft("nope", "deploy", false).await.unwrap_err();
    assert_eq!(
        (err.channel, err.message.as_str()),
        ("actions:draft", "Unknown project nope")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_001_008_the_key_is_sealed_and_never_given_out() {
    let world = world();
    let voice = world.nodus.voice();
    let from_shell = std::env::var("XAI_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(|_| nodus_domain::VoiceKeySource::Env);
    assert_eq!(voice.key_source().await, from_shell);
    assert_eq!(
        voice.set_key(Some("  xai-synthetic  ")).await.unwrap(),
        Some(nodus_domain::VoiceKeySource::Settings)
    );
    let sealed = world.nodus.inner.store.read(|state| state.voice_key.clone()).unwrap();
    assert!(!sealed.contains("xai-synthetic"));
    assert_eq!(voice.key_source().await, Some(nodus_domain::VoiceKeySource::Settings));
    assert_eq!(voice.set_key(None).await.unwrap(), from_shell);
    assert!(world.nodus.inner.store.read(|state| state.voice_key.is_none()));
    // VOICE-002: the project is checked before anything is sent.
    let err = voice.transcribe("nope", vec![1, 2, 3], "audio/webm").await.unwrap_err();
    assert_eq!(
        err.ipc_message(),
        "Error invoking remote method 'voice:transcribe': Error: Unknown project nope"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_003_git_reads_the_project_folder_once_the_env_is_in() {
    let world = world();
    let project = project(&world).await;
    let git = world.nodus.git();
    // Not a repository yet.
    assert_eq!(git.status(&project.id).await.unwrap(), None);
    assert!(git.log(&project.id, Some(5.0)).await.unwrap().is_empty());
    let made = std::process::Command::new("git")
        .args(["init", "-q", "-b", "trunk"])
        .current_dir(&project.repo_path)
        .status();
    if made.is_ok_and(|s| s.success()) {
        std::fs::write(Path::new(&project.repo_path).join("a.txt"), "a\n").unwrap();
        let status = git.status(&project.id).await.unwrap().unwrap();
        assert_eq!(status.branch.as_deref(), Some("trunk"));
        assert_eq!(status.files.len(), 1);
        let diff = git.diff(&project.id, &status.files[0]).await.unwrap();
        assert!(diff.patch.contains("+a"), "{}", diff.patch);
    }
    let err = git.status("nope").await.unwrap_err();
    assert_eq!(
        err.ipc_message(),
        "Error invoking remote method 'git:status': Error: Unknown project nope"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn srv_026_a_session_belongs_to_the_project_whose_board_has_it() {
    let world = world();
    let project = project(&world).await;
    let board = world.nodus.board();
    assert_eq!(board.session_project("s1"), None);
    world.nodus.inner.store.update(|state| {
        state.board_mut(&project.id).nodes.insert("s1".into(), node("s1"));
    });
    assert_eq!(board.session_project("s1"), Some(project.id.clone()));
    board.remove_node(&project.id, "s1").await;
    assert_eq!(board.session_project("s1"), None);
}
