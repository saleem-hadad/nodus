//! Stand-ins for the app, GitHub and the OS, shared by the updater's tests.
#![allow(dead_code, clippy::print_stderr)]

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{Event, RecordingSink, UpdateState, UpdateStatus};
use nodus_updater::{App, Body, BoxFuture, HttpResponse, LATEST_URL, Platform, RunOutput, System, Updater};
use tempfile::TempDir;

/// What happened, in order, across the app and the system.
pub type Log = Arc<Mutex<Vec<String>>>;

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

pub struct FakeApp {
    pub version: String,
    pub user_data: PathBuf,
    pub auto_update: AtomicBool,
    pub live: AtomicUsize,
    pub stop_fails: AtomicBool,
    pub user_data_override: Mutex<Option<String>>,
    pub log: Log,
}

impl FakeApp {
    pub fn count(&self, what: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.as_str() == what)
            .count()
    }
}

impl App for FakeApp {
    fn version(&self) -> String {
        self.version.clone()
    }
    fn user_data(&self) -> PathBuf {
        self.user_data.clone()
    }
    fn auto_update(&self) -> bool {
        self.auto_update.load(Ordering::SeqCst)
    }
    fn live_session_count(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }
    fn stop_all(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.log.lock().unwrap().push("stop_all".into());
            if self.stop_fails.load(Ordering::SeqCst) {
                return Err("the host didn't stop".into());
            }
            Ok(())
        })
    }
    fn quit(&self) {
        self.log.lock().unwrap().push("quit".into());
    }
    fn user_data_override(&self) -> Option<String> {
        self.user_data_override.lock().unwrap().clone()
    }
}

/// How GitHub answers a URL.
#[derive(Clone)]
pub enum Route {
    /// This status, and a body in these chunks.
    Answer { status: u16, chunks: Vec<Vec<u8>> },
    /// This status, with no body.
    NoBody(u16),
    /// No connection.
    Unreachable,
    /// Never answers.
    Hang,
    /// Answers 200, sends these chunks, then nothing more, ever.
    Stall(Vec<Vec<u8>>),
}

impl Route {
    pub fn json(text: &str) -> Self {
        Self::Answer {
            status: 200,
            chunks: vec![text.as_bytes().to_vec()],
        }
    }
    pub fn bytes(data: &[u8]) -> Self {
        Self::Answer {
            status: 200,
            chunks: vec![data.to_vec()],
        }
    }
}

struct ChunkBody {
    chunks: VecDeque<Vec<u8>>,
    then_hang: bool,
}

impl Body for ChunkBody {
    fn next_chunk(&mut self) -> BoxFuture<'_, Option<io::Result<Vec<u8>>>> {
        Box::pin(async move {
            match self.chunks.pop_front() {
                Some(chunk) => Some(Ok(chunk)),
                None if self.then_hang => std::future::pending().await,
                None => None,
            }
        })
    }
}

type Runner = Box<dyn Fn(&FakeSystem, &str, &[String]) -> Result<RunOutput, String> + Send + Sync>;
/// Which paths are writable.
pub type Writable = Box<dyn Fn(&Path) -> bool + Send + Sync>;
/// A request: its URL and headers.
pub type Request = (String, Vec<(&'static str, String)>);

pub struct FakeSystem {
    pub platform: Platform,
    pub packaged: bool,
    pub arm64: bool,
    pub bundle: PathBuf,
    pub in_applications: bool,
    /// Which paths are writable; None asks the file system.
    pub writable: Option<Writable>,
    pub routes: Mutex<HashMap<String, Route>>,
    pub requests: Mutex<Vec<Request>>,
    pub runs: Mutex<Vec<Vec<String>>>,
    pub runner: Runner,
    pub spawns: Mutex<Vec<(PathBuf, Vec<String>)>>,
    pub spawn_fails: AtomicBool,
    pub log: Log,
}

impl FakeSystem {
    pub fn route(&self, url: &str, route: Route) {
        self.routes.lock().unwrap().insert(url.to_owned(), route);
    }

    /// How many times `url` was asked for.
    pub fn hits(&self, url: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(asked, _)| asked == url)
            .count()
    }

    pub fn runs(&self) -> Vec<Vec<String>> {
        self.runs.lock().unwrap().clone()
    }

    pub fn spawns(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.spawns.lock().unwrap().clone()
    }
}

impl System for FakeSystem {
    fn platform(&self) -> Platform {
        self.platform
    }
    fn is_packaged(&self) -> bool {
        self.packaged
    }
    fn arm64_machine(&self) -> bool {
        self.arm64
    }
    fn bundle_path(&self) -> PathBuf {
        self.bundle.clone()
    }
    fn is_in_applications_folder(&self) -> bool {
        self.in_applications
    }
    fn writable(&self, path: &Path) -> bool {
        match &self.writable {
            Some(writable) => writable(path),
            None => nodus_updater::writable(path),
        }
    }
    fn pid(&self) -> u32 {
        4242
    }
    fn get(&self, url: &str, headers: Vec<(&'static str, String)>) -> BoxFuture<'_, io::Result<HttpResponse>> {
        self.requests.lock().unwrap().push((url.to_owned(), headers));
        let route = self.routes.lock().unwrap().get(url).cloned();
        Box::pin(async move {
            match route {
                None => Ok(HttpResponse {
                    status: 404,
                    body: Some(Box::new(ChunkBody {
                        chunks: VecDeque::from([b"{\"message\":\"Not Found\"}".to_vec()]),
                        then_hang: false,
                    })),
                }),
                Some(Route::Answer { status, chunks }) => Ok(HttpResponse {
                    status,
                    body: Some(Box::new(ChunkBody {
                        chunks: chunks.into(),
                        then_hang: false,
                    })),
                }),
                Some(Route::NoBody(status)) => Ok(HttpResponse { status, body: None }),
                Some(Route::Unreachable) => Err(io::Error::other("getaddrinfo ENOTFOUND api.github.com")),
                Some(Route::Hang) => std::future::pending().await,
                Some(Route::Stall(chunks)) => Ok(HttpResponse {
                    status: 200,
                    body: Some(Box::new(ChunkBody {
                        chunks: chunks.into(),
                        then_hang: true,
                    })),
                }),
            }
        })
    }
    fn run(&self, program: &str, args: Vec<String>) -> BoxFuture<'_, Result<RunOutput, String>> {
        let mut argv = vec![program.to_owned()];
        argv.extend(args.iter().cloned());
        self.runs.lock().unwrap().push(argv);
        let result = (self.runner)(self, program, &args);
        Box::pin(async move { result })
    }
    fn spawn_detached(&self, program: &Path, args: Vec<String>) -> io::Result<()> {
        self.log.lock().unwrap().push("spawn".into());
        self.spawns.lock().unwrap().push((program.to_owned(), args));
        if self.spawn_fails.load(Ordering::SeqCst) {
            return Err(io::Error::other("blocked"));
        }
        Ok(())
    }
}

/// A runner for a machine where every tool fails: Windows never runs any.
pub fn no_tools() -> Runner {
    Box::new(|_, program, _| Err(format!("{program} isn't here")))
}

/// A world for one test: a temp userData and the fakes around it.
pub struct World {
    pub dir: TempDir,
    pub app: Arc<FakeApp>,
    pub system: Arc<FakeSystem>,
    pub events: Arc<RecordingSink>,
}

impl World {
    pub fn new(platform: Platform, version: &str, runner: Runner) -> Self {
        let dir = TempDir::new().unwrap();
        let log: Log = Arc::default();
        let user_data = dir.path().join("userData");
        std::fs::create_dir_all(&user_data).unwrap();
        let bundle = dir.path().join("Applications").join("Nodus.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let app = Arc::new(FakeApp {
            version: version.into(),
            user_data,
            auto_update: AtomicBool::new(false),
            live: AtomicUsize::new(0),
            stop_fails: AtomicBool::new(false),
            user_data_override: Mutex::new(None),
            log: log.clone(),
        });
        let system = Arc::new(FakeSystem {
            platform,
            packaged: true,
            arm64: false,
            bundle,
            in_applications: true,
            writable: None,
            routes: Mutex::default(),
            requests: Mutex::default(),
            runs: Mutex::default(),
            runner,
            spawns: Mutex::default(),
            spawn_fails: AtomicBool::new(false),
            log,
        });
        Self {
            dir,
            app,
            system,
            events: Arc::default(),
        }
    }

    /// The same world with the system changed before anything runs.
    pub fn with_system(mut self, change: impl FnOnce(&mut FakeSystem)) -> Self {
        change(Arc::get_mut(&mut self.system).expect("nothing holds the system yet"));
        self
    }

    pub fn updates(&self) -> PathBuf {
        self.app.user_data.join("updates")
    }

    /// The updater as a fresh launch starts it.
    pub fn launch(&self) -> Arc<Updater> {
        let updater = Updater::new(self.app.clone(), self.system.clone(), self.events.clone());
        updater.start();
        updater
    }

    /// The states sent so far, oldest first.
    pub fn states(&self) -> Vec<UpdateState> {
        self.events
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::UpdateStatus(UpdateStatus { state, .. }) => Some(state),
                _ => None,
            })
            .collect()
    }

    /// The names in updates/, sorted.
    pub fn update_files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.updates())
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

/// The updater's state, without the version running.
pub fn state(updater: &Updater) -> UpdateState {
    updater.status().state
}

/// Waits until the state is one that `done` accepts, and returns it.
pub async fn settle(updater: &Updater, done: impl Fn(&UpdateState) -> bool) -> UpdateState {
    settle_within(updater, Duration::from_secs(5), done).await
}

/// [`settle`], for up to `limit` (of the paused clock, in tests that pause it).
pub async fn settle_within(updater: &Updater, limit: Duration, done: impl Fn(&UpdateState) -> bool) -> UpdateState {
    let started = tokio::time::Instant::now();
    while started.elapsed() < limit {
        let now = state(updater);
        if done(&now) {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never settled; last state {:?}", state(updater));
}

fn finished(state: &UpdateState) -> bool {
    matches!(state, UpdateState::Ready { .. } | UpdateState::Error { .. })
}

/// Waits for the download to end, one way or the other.
pub async fn prepared(updater: &Updater) -> UpdateState {
    settle(updater, finished).await
}

/// [`prepared`], for up to `limit`.
pub async fn prepared_within(updater: &Updater, limit: Duration) -> UpdateState {
    settle_within(updater, limit, finished).await
}

pub fn ready(version: &str) -> UpdateState {
    UpdateState::Ready {
        version: version.into(),
    }
}

pub fn error(message: &str) -> UpdateState {
    UpdateState::Error {
        message: message.into(),
    }
}

/// GitHub's answer for `releases/latest`.
pub fn release_json(tag: &str, assets: &[String]) -> String {
    format!(
        r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[{}]}}"#,
        assets.join(",")
    )
}

/// One asset in GitHub's answer; `digest` is the JSON value (`null`, `"sha256:…"`).
pub fn asset_json(name: &str, size: usize, digest: &str, url: &str) -> String {
    format!(
        r#"{{"name":"{name}","state":"uploaded","size":{size},"digest":{digest},"browser_download_url":"{url}","label":null}}"#
    )
}

pub fn digest_of(data: &[u8]) -> String {
    format!("\"sha256:{}\"", sha256_hex(data))
}

pub fn latest() -> &'static str {
    LATEST_URL
}
