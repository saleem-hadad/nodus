//! Watching a project's transcript folder, subagent transcripts included, to
//! tell the board to reload when a session's metadata or subagents change or
//! a new session appears (SESS-063…070).
//!
//! The timing is discovery.ts's, with tokio's clock: a change schedules a
//! check 500 ms later unless one is already waiting (new changes don't push it
//! back), checks run at least 1500 ms after the last time the board was told,
//! a folder that isn't there yet is looked for every 3 s, and a watcher that
//! reports an error is closed for good.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher as _};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until};

use crate::meta::{MetaCache, meta_signature};
use crate::paths::{agents_signature, transcript_dir};

/// How long after a change the folder is checked; not restarted by more changes.
pub const CHECK_DELAY: Duration = Duration::from_millis(500);
/// The board is told of changes at most this often.
pub const MIN_INTERVAL: Duration = Duration::from_millis(1500);
/// How often a folder that doesn't exist yet is looked for.
pub const POLL_INTERVAL: Duration = Duration::from_millis(3000);

/// What the board is told when a project's sessions change.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

/// What a watched folder reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    Changed,
    Failed,
}

/// Something that watches a folder and reports into a channel while it's
/// held; dropping it stops it.
type Attached = Box<dyn Send>;

/// Starts watching a folder recursively, when it's there to watch.
type Attach = Arc<dyn Fn(&Path, UnboundedSender<Signal>) -> Option<Attached> + Send + Sync>;

struct Watch {
    repo_path: String,
    task: JoinHandle<()>,
}

/// Every project's transcript watch.
pub struct Watcher {
    home: PathBuf,
    meta: Arc<MetaCache>,
    runtime: tokio::runtime::Handle,
    attach: Attach,
    watches: Mutex<HashMap<String, Watch>>,
}

impl Watcher {
    /// Watches with the operating system's file events (FSEvents,
    /// ReadDirectoryChangesW), reading metadata through `meta`. Watches run on
    /// `runtime`.
    pub fn new(home: PathBuf, meta: Arc<MetaCache>, runtime: tokio::runtime::Handle) -> Self {
        Self::with_attach(home, meta, runtime, Arc::new(attach_notify))
    }

    fn with_attach(home: PathBuf, meta: Arc<MetaCache>, runtime: tokio::runtime::Handle, attach: Attach) -> Self {
        Self {
            home,
            meta,
            runtime,
            attach,
            watches: Mutex::new(HashMap::new()),
        }
    }

    /// Watches a project's transcript folder, and calls `on_change` (throttled)
    /// when its sessions change. Watching it again for the same repo keeps
    /// the first watch and its `on_change`; another repo replaces it (SESS-063).
    pub fn watch_project(&self, project_id: &str, repo_path: &str, on_change: OnChange) {
        let mut watches = self.lock();
        if let Some(existing) = watches.get(project_id) {
            if existing.repo_path == repo_path {
                return;
            }
            if let Some(old) = watches.remove(project_id) {
                old.task.abort();
            }
        }
        // SESS-064: what's there now, without telling anyone.
        let signatures = snapshot_signatures(&self.home, &self.meta, repo_path);
        let run = Run {
            home: self.home.clone(),
            meta: Arc::clone(&self.meta),
            repo_path: repo_path.to_owned(),
            attach: Arc::clone(&self.attach),
            on_change,
            signatures,
        };
        let task = self.runtime.spawn(run.run());
        watches.insert(
            project_id.to_owned(),
            Watch {
                repo_path: repo_path.to_owned(),
                task,
            },
        );
    }

    /// Stops watching a project: closes its watcher, stops looking for its
    /// folder and drops a pending check (SESS-070).
    pub fn unwatch_project(&self, project_id: &str) {
        if let Some(watch) = self.lock().remove(project_id) {
            watch.task.abort();
        }
    }

    /// Stops every watch, as the app quits (SESS-070).
    pub fn unwatch_all(&self) {
        for (_, watch) in self.lock().drain() {
            watch.task.abort();
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Watch>> {
        self.watches.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.unwatch_all();
    }
}

/// Each session's metadata and subagents fingerprint (SESS-067).
fn snapshot_signatures(home: &Path, meta: &MetaCache, repo_path: &str) -> HashMap<String, String> {
    meta.scan_repo(home, repo_path)
        .into_iter()
        .map(|(id, found)| {
            let signature = format!(
                "{}|{}",
                meta_signature(&found.meta),
                agents_signature(home, repo_path, &id)
            );
            (id, signature)
        })
        .collect()
}

/// One project's watch, as it runs.
struct Run {
    home: PathBuf,
    meta: Arc<MetaCache>,
    repo_path: String,
    attach: Attach,
    on_change: OnChange,
    signatures: HashMap<String, String>,
}

impl Run {
    async fn run(mut self) {
        let (tx, mut rx) = unbounded_channel();
        let (home, repo_path, attach) = (self.home.clone(), self.repo_path.clone(), Arc::clone(&self.attach));
        let try_attach = |tx: &UnboundedSender<Signal>| {
            let dir = transcript_dir(&home, &repo_path);
            if !dir.exists() {
                return None;
            }
            attach(&dir, tx.clone())
        };
        // SESS-065: held while it watches; dropping it closes it.
        let mut _watcher = try_attach(&tx);
        // SESS-068: the folder only exists after the first session in the repo; look until then.
        let mut poll = _watcher.is_none().then(|| {
            let mut poll = interval_at(Instant::now() + POLL_INTERVAL, POLL_INTERVAL);
            poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
            poll
        });
        let mut check_at: Option<Instant> = None;
        let mut last_emit: Option<Instant> = None;

        loop {
            tokio::select! {
                signal = rx.recv() => match signal {
                    // SESS-065: a check already pending will see this change too.
                    Some(Signal::Changed) => {
                        check_at.get_or_insert_with(|| Instant::now() + CHECK_DELAY);
                    }
                    // SESS-069: closed, and never attached again.
                    Some(Signal::Failed) => _watcher = None,
                    None => return,
                },
                () = sleep_until(check_at.unwrap_or_else(Instant::now)), if check_at.is_some() => {
                    check_at = None;
                    // SESS-066
                    if let Some(last) = last_emit {
                        let due = last + MIN_INTERVAL;
                        if due > Instant::now() {
                            check_at = Some(due);
                            continue;
                        }
                    }
                    if self.check() {
                        last_emit = Some(Instant::now());
                        (self.on_change)();
                    }
                },
                _ = async { poll.as_mut().expect("polling").tick().await }, if poll.is_some() => {
                    if let Some(attached) = try_attach(&tx) {
                        _watcher = Some(attached);
                        poll = None;
                        check_at.get_or_insert_with(|| Instant::now() + CHECK_DELAY);
                    }
                },
            }
        }
    }

    /// Whether the sessions changed since the last check; the new signatures
    /// are kept either way (SESS-067).
    fn check(&mut self) -> bool {
        let next = snapshot_signatures(&self.home, &self.meta, &self.repo_path);
        let changed =
            next.len() != self.signatures.len() || next.iter().any(|(id, sig)| self.signatures.get(id) != Some(sig));
        self.signatures = next;
        changed
    }
}

/// Watches a folder and everything under it with notify. Reads aren't
/// changes: Node's watcher never reports them.
fn attach_notify(dir: &Path, tx: UnboundedSender<Signal>) -> Option<Attached> {
    let handler = move |event: notify::Result<notify::Event>| {
        let signal = match event {
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
            Ok(_) => Signal::Changed,
            Err(_) => Signal::Failed,
        };
        let _ = tx.send(signal);
    };
    let mut watcher = notify::recommended_watcher(handler).ok()?;
    watcher.watch(dir, RecursiveMode::Recursive).ok()?;
    Some(Box::new(watcher))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ID: &str = "00000000-0000-4000-8000-000000000001";

    /// A fake watcher: tests send its events by hand.
    #[derive(Default)]
    struct Fake {
        senders: Mutex<Vec<UnboundedSender<Signal>>>,
        attaches: AtomicUsize,
    }

    impl Fake {
        fn send(&self, signal: Signal) {
            for tx in self.senders.lock().unwrap().iter() {
                let _ = tx.send(signal);
            }
        }
    }

    struct Fixture {
        home: tempfile::TempDir,
        repo: String,
        fake: Arc<Fake>,
        watcher: Watcher,
        changes: Arc<AtomicUsize>,
    }

    impl Fixture {
        fn new() -> Self {
            let home = tempfile::TempDir::new().unwrap();
            let repo = home.path().join("repo");
            fs::create_dir(&repo).unwrap();
            let repo = repo.to_string_lossy().into_owned();
            let fake = Arc::new(Fake::default());
            let attach: Attach = {
                let fake = Arc::clone(&fake);
                Arc::new(move |_: &Path, tx| {
                    fake.attaches.fetch_add(1, Ordering::SeqCst);
                    fake.senders.lock().unwrap().push(tx);
                    Some(Box::new(()) as Attached)
                })
            };
            let watcher = Watcher::with_attach(
                home.path().to_owned(),
                Arc::new(MetaCache::new()),
                tokio::runtime::Handle::current(),
                attach,
            );
            Self {
                home,
                repo,
                fake,
                watcher,
                changes: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn dir(&self) -> PathBuf {
            transcript_dir(self.home.path(), &self.repo)
        }

        fn write_session(&self, id: &str, prompts: usize) {
            fs::create_dir_all(self.dir()).unwrap();
            let line = r#"{"type":"user","message":{"content":"p"}}"#;
            let text: String = (0..prompts).map(|_| format!("{line}\n")).collect();
            fs::write(self.dir().join(format!("{id}.jsonl")), text).unwrap();
        }

        async fn watch(&self) {
            let changes = Arc::clone(&self.changes);
            self.watcher.watch_project(
                "p",
                &self.repo,
                Arc::new(move || {
                    changes.fetch_add(1, Ordering::SeqCst);
                }),
            );
            // The watch attaches once its task runs.
            advance(0).await;
        }

        fn changes(&self) -> usize {
            self.changes.load(Ordering::SeqCst)
        }
    }

    async fn advance(ms: u64) {
        // Let the watch take what was sent before time moves.
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(ms)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn checks_500ms_after_a_change_and_tells_the_board_once() {
        let f = Fixture::new();
        f.write_session(ID, 1);
        f.watch().await;
        advance(0).await;
        // Nothing changed yet: the baseline isn't reported.
        f.fake.send(Signal::Changed);
        advance(600).await;
        assert_eq!(f.changes(), 0);

        f.write_session(ID, 2);
        f.fake.send(Signal::Changed);
        advance(300).await;
        // More changes don't push the check back.
        f.fake.send(Signal::Changed);
        advance(150).await;
        assert_eq!(f.changes(), 0);
        advance(100).await;
        assert_eq!(f.changes(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tells_the_board_at_most_every_1500ms() {
        let f = Fixture::new();
        f.write_session(ID, 1);
        f.watch().await;
        f.write_session(ID, 2);
        f.fake.send(Signal::Changed);
        advance(510).await;
        assert_eq!(f.changes(), 1);
        // Its check, 500 ms on, waits until 1500 ms have passed since the last report.
        f.write_session(ID, 3);
        f.fake.send(Signal::Changed);
        advance(1400).await;
        assert_eq!(f.changes(), 1);
        advance(110).await;
        assert_eq!(f.changes(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn counts_new_sessions_and_subagents() {
        let f = Fixture::new();
        f.write_session(ID, 1);
        f.watch().await;
        f.write_session("00000000-0000-4000-8000-000000000002", 1);
        f.fake.send(Signal::Changed);
        advance(510).await;
        assert_eq!(f.changes(), 1);
        let agents = f.dir().join(ID).join("subagents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("agent-a1.jsonl"), "{}\n").unwrap();
        advance(1500).await;
        f.fake.send(Signal::Changed);
        advance(510).await;
        assert_eq!(f.changes(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn looks_for_a_missing_folder_every_3s() {
        let f = Fixture::new();
        f.watch().await;
        advance(2900).await;
        assert_eq!(f.fake.attaches.load(Ordering::SeqCst), 0);
        f.write_session(ID, 1);
        advance(110).await;
        assert_eq!(f.fake.attaches.load(Ordering::SeqCst), 1);
        // And checks once it's there.
        advance(500).await;
        assert_eq!(f.changes(), 1);
        advance(6000).await;
        assert_eq!(f.fake.attaches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn stops_for_good_after_a_watcher_error() {
        let f = Fixture::new();
        f.write_session(ID, 1);
        f.watch().await;
        f.fake.send(Signal::Failed);
        advance(0).await;
        // The same project and repo keep the dead watch.
        f.watch().await;
        advance(10_000).await;
        assert_eq!(f.fake.attaches.load(Ordering::SeqCst), 1);
        // Unwatched and watched again, it attaches anew.
        f.watcher.unwatch_project("p");
        f.watch().await;
        advance(0).await;
        assert_eq!(f.fake.attaches.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn unwatching_drops_a_pending_check() {
        let f = Fixture::new();
        f.write_session(ID, 1);
        f.watch().await;
        f.write_session(ID, 2);
        f.fake.send(Signal::Changed);
        advance(100).await;
        f.watcher.unwatch_all();
        advance(1000).await;
        assert_eq!(f.changes(), 0);
    }

    #[tokio::test]
    async fn sees_real_file_events() {
        let home = tempfile::TempDir::new().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let repo = repo.to_string_lossy().into_owned();
        let dir = transcript_dir(home.path(), &repo);
        fs::create_dir_all(&dir).unwrap();
        let watcher = Watcher::new(
            home.path().to_owned(),
            Arc::new(MetaCache::new()),
            tokio::runtime::Handle::current(),
        );
        let (tx, mut rx) = unbounded_channel();
        watcher.watch_project(
            "p",
            &repo,
            Arc::new(move || {
                let _ = tx.send(());
            }),
        );
        // FSEvents can take a moment to start.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let line = r#"{"type":"user","message":{"content":"p"}}"#;
        fs::write(dir.join(format!("{ID}.jsonl")), format!("{line}\n")).unwrap();
        let told = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await;
        assert!(matches!(told, Ok(Some(()))), "the board wasn't told");
    }
}
