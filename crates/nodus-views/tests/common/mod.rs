//! A views service on a temporary data folder, with fake session processes,
//! brains and host, and a board that loads a changed page as soon as it
//! hears of it, as views.test.ts sets up.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use nodus_board::{Board, BoardSessions, BoxFuture, Clock};
use nodus_domain::{
    BoardNode, Event, EventSink, Project, SessionCard, SessionStatus, SessionUsage, ViewEvent, ViewReport,
};
use nodus_platform::Env;
use nodus_sessions::launch::SpawnOptions;
use nodus_store::Store;
use nodus_transcripts::Transcripts;
use nodus_views::{BrainLauncher, ViewHost, Views, ViewsConfig};
use serde_json::{Value, json};

#[derive(Default)]
pub struct FakeSessions {
    pub live: Mutex<HashSet<String>>,
    pub status: Mutex<HashMap<String, SessionStatus>>,
    pub log: Mutex<Vec<String>>,
}

impl FakeSessions {
    pub fn set(&self, id: &str, live: bool, status: SessionStatus) {
        if live {
            self.live.lock().unwrap().insert(id.into());
        } else {
            self.live.lock().unwrap().remove(id);
        }
        self.status.lock().unwrap().insert(id.into(), status);
    }

    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl BoardSessions for FakeSessions {
    fn is_live(&self, session_id: &str) -> bool {
        self.live.lock().unwrap().contains(session_id)
    }
    fn status(&self, session_id: &str) -> SessionStatus {
        self.status
            .lock()
            .unwrap()
            .get(session_id)
            .copied()
            .unwrap_or(SessionStatus::Done)
    }
    fn usage(&self, _session_id: &str) -> Option<SessionUsage> {
        None
    }
    fn attach_agents(&self, _card: &mut SessionCard, _repo_path: &str, _last_prompt_at: Option<f64>) {}
    fn stop(&self, session_id: &str) {
        self.log.lock().unwrap().push(format!("stop:{session_id}"));
    }
    fn stop_and_forget(&self, session_id: &str) {
        self.log.lock().unwrap().push(format!("stop+forget:{session_id}"));
    }
    fn kill(&self, session_id: &str) -> BoxFuture<'_, ()> {
        self.log.lock().unwrap().push(format!("kill:{session_id}"));
        Box::pin(async {})
    }
    fn forget_status(&self, session_id: &str) {
        self.log.lock().unwrap().push(format!("forget:{session_id}"));
    }
}

/// Records the brains it's asked to start and what's typed into them.
#[derive(Default)]
pub struct FakeBrains {
    pub spawns: Mutex<Vec<SpawnOptions>>,
    pub writes: Mutex<Vec<(String, String)>>,
    pub fail: Mutex<Option<String>>,
}

impl FakeBrains {
    pub fn spawns(&self) -> Vec<SpawnOptions> {
        self.spawns.lock().unwrap().clone()
    }

    pub fn writes(&self) -> Vec<(String, String)> {
        self.writes.lock().unwrap().clone()
    }
}

impl BrainLauncher for FakeBrains {
    fn spawn(&self, opts: SpawnOptions) -> BoxFuture<'_, Result<(), String>> {
        self.spawns.lock().unwrap().push(opts);
        let result = self.fail.lock().unwrap().clone().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }
    fn write(&self, session_id: &str, data: &str) {
        self.writes.lock().unwrap().push((session_id.into(), data.into()));
    }
}

#[derive(Default)]
pub struct FakeHost {
    pub opened: Mutex<Vec<String>>,
}

impl ViewHost for FakeHost {
    fn git_status(&self, _repo_path: &str) -> BoxFuture<'_, Value> {
        Box::pin(async { Value::Null })
    }
    fn open_url(&self, url: &str) {
        self.opened.lock().unwrap().push(url.into());
    }
}

/// Keeps every event, and like the board in views.test.ts, loads every view's
/// page on a board as soon as it hears the board changed.
#[derive(Default)]
pub struct Window {
    pub events: Mutex<Vec<Event>>,
    pub views: OnceLock<Views>,
    pub store: OnceLock<Arc<Store>>,
    /// Don't load pages: the board isn't showing.
    pub hidden: Mutex<bool>,
}

impl Window {
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    pub fn view_events(&self) -> Vec<(String, ViewEvent, Value)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::View {
                    view_id,
                    event,
                    payload,
                    ..
                } => Some((view_id.clone(), *event, payload.clone())),
                _ => None,
            })
            .collect()
    }
}

impl EventSink for Window {
    fn emit(&self, event: Event) {
        if let Event::BoardChanged { project_id } = &event
            && !*self.hidden.lock().unwrap()
            && let (Some(views), Some(store)) = (self.views.get(), self.store.get())
        {
            let (views, store, project_id) = (views.clone(), Arc::clone(store), project_id.clone());
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                let loaded: Vec<(String, u32)> = store.read(|s| {
                    s.board(&project_id)
                        .map(|b| b.views.values().map(|v| (v.id.clone(), v.version)).collect())
                        .unwrap_or_default()
                });
                for (id, version) in loaded {
                    views.report(
                        &project_id,
                        &id,
                        &ViewReport {
                            error: None,
                            loaded: Some(version),
                        },
                    );
                }
            });
        }
        self.events.lock().unwrap().push(event);
    }
}

pub struct Fixture {
    pub _home: tempfile::TempDir,
    pub user_data: tempfile::TempDir,
    pub repo: String,
    pub store: Arc<Store>,
    pub board: Arc<Board>,
    pub sessions: Arc<FakeSessions>,
    pub brains: Arc<FakeBrains>,
    pub host: Arc<FakeHost>,
    pub window: Arc<Window>,
    pub views: Views,
}

pub const P: &str = "p1";

pub fn node(id: &str, x: f64) -> BoardNode {
    serde_json::from_value(json!({ "sessionId": id, "x": x, "y": 0, "archived": false, "createdAt": 0 })).unwrap()
}

impl Fixture {
    /// Project p1, whose folder is the data folder as in views.test.ts, with
    /// card s1 at (2000, 0), telling time by `clock`.
    pub fn with_clock(clock: Clock) -> Self {
        let home = tempfile::TempDir::new().unwrap();
        let user_data = tempfile::TempDir::new().unwrap();
        let repo = user_data.path().to_string_lossy().into_owned();
        let store = Arc::new(Store::open(user_data.path()).unwrap());
        let project: Project = serde_json::from_value(json!({
            "id": P, "name": P, "repoPath": repo, "color": "blue", "createdAt": 0
        }))
        .unwrap();
        store.update(|s| {
            s.projects.push(project);
            s.board_mut(P).nodes.insert("s1".into(), node("s1", 2000.0));
        });
        let transcripts = Arc::new(Transcripts::new(home.path().to_owned()));
        let sessions = Arc::new(FakeSessions::default());
        let window = Arc::new(Window::default());
        let board = Arc::new(
            Board::new(
                Arc::clone(&store),
                transcripts,
                Arc::clone(&sessions) as Arc<dyn BoardSessions>,
                Arc::clone(&window) as Arc<dyn EventSink>,
                tokio::runtime::Handle::current(),
            )
            .with_clock(Arc::clone(&clock)),
        );
        let brains = Arc::new(FakeBrains::default());
        let host = Arc::new(FakeHost::default());
        let (_, env) = tokio::sync::watch::channel(Some(Arc::new(Env::from_process())));
        let views = Views::new(ViewsConfig {
            user_data: user_data.path().to_owned(),
            store: Arc::clone(&store),
            board: Arc::clone(&board),
            sessions: Arc::clone(&sessions) as Arc<dyn BoardSessions>,
            brains: Arc::clone(&brains) as Arc<dyn BrainLauncher>,
            host: Arc::clone(&host) as Arc<dyn ViewHost>,
            events: Arc::clone(&window) as Arc<dyn EventSink>,
            env,
            runner: Arc::new(nodus_platform::runner::LocalRunner),
            clock,
            runtime: tokio::runtime::Handle::current(),
        });
        let _ = window.views.set(views.clone());
        let _ = window.store.set(Arc::clone(&store));
        Self {
            _home: home,
            user_data,
            repo,
            store,
            board,
            sessions,
            brains,
            host,
            window,
            views,
        }
    }

    /// The same on the wall clock.
    pub fn new() -> Self {
        Self::with_clock(nodus_board::real_clock())
    }

    /// The same on a clock that moves with tokio's, from `start`.
    pub fn paused(start: f64) -> Self {
        let since = tokio::time::Instant::now();
        Self::with_clock(Arc::new(move || start + since.elapsed().as_millis() as f64))
    }

    pub fn view(&self, id: &str) -> Option<nodus_domain::BoardView> {
        self.store.read(|s| s.board(P)?.views.get(id).cloned())
    }

    pub fn node(&self, id: &str) -> Option<BoardNode> {
        self.store.read(|s| s.board(P)?.nodes.get(id).cloned())
    }

    pub fn add_node(&self, node: BoardNode) {
        self.store
            .update(|s| s.board_mut(P).nodes.insert(node.session_id.clone(), node));
    }

    pub fn views_dir(&self, id: &str) -> std::path::PathBuf {
        self.user_data.path().join("views").join(id)
    }
}

pub const HTML: &str = r#"<!doctype html><html><head></head><body><p id="n"></p></body></html>"#;

/// Lets what's due run, as the paused clock reaches it.
pub async fn advance(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}
