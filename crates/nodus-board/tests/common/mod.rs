//! A board on a temporary data folder and home, with fake session processes.

#![allow(dead_code, clippy::needless_pass_by_value)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nodus_board::{Board, BoardSessions, BoxFuture, Launcher, Resume, StartNew};
use nodus_domain::{BoardNode, Project, RecordingSink, SessionCard, SessionStatus, SessionUsage};
use nodus_store::Store;
use nodus_transcripts::Transcripts;
use serde_json::{Value, json};

pub const NOW: f64 = 1_800_000_000_000.0;

/// Session processes and statuses the test sets, and a log of what the board asked of them.
#[derive(Default)]
pub struct FakeSessions {
    pub live: Mutex<HashSet<String>>,
    pub status: Mutex<HashMap<String, SessionStatus>>,
    pub log: Mutex<Vec<String>>,
}

impl FakeSessions {
    pub fn set_live(&self, id: &str, status: SessionStatus) {
        self.live.lock().unwrap().insert(id.into());
        self.status.lock().unwrap().insert(id.into(), status);
    }

    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn note(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
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

    fn usage(&self, session_id: &str) -> Option<SessionUsage> {
        (session_id == "with-usage").then(|| SessionUsage {
            model: Some("Opus".into()),
            context_percent: None,
            context_tokens: None,
            context_size: None,
            cost_usd: None,
            duration_ms: None,
            at: 1.0,
        })
    }

    fn attach_agents(&self, card: &mut SessionCard, _repo_path: &str, last_prompt_at: Option<f64>) {
        self.note(format!("agents:{}:{last_prompt_at:?}", card.node.session_id));
    }

    fn stop(&self, session_id: &str) {
        self.note(format!("stop:{session_id}"));
    }

    fn stop_and_forget(&self, session_id: &str) {
        self.note(format!("stop+forget:{session_id}"));
    }

    fn kill(&self, session_id: &str) -> BoxFuture<'_, ()> {
        self.note(format!("kill:{session_id}"));
        Box::pin(async {})
    }

    fn forget_status(&self, session_id: &str) {
        self.note(format!("forget:{session_id}"));
    }
}

/// Starts processes by recording what it was asked, failing when told to.
pub struct FakeLauncher {
    pub store: Arc<Store>,
    pub fail: Option<String>,
    pub log: Mutex<Vec<String>>,
    pub starts: Mutex<Vec<StartNew>>,
    pub resumes: Mutex<Vec<Resume>>,
}

impl FakeLauncher {
    pub fn new(store: &Arc<Store>, fail: Option<&str>) -> Self {
        Self {
            store: Arc::clone(store),
            fail: fail.map(Into::into),
            log: Mutex::new(Vec::new()),
            starts: Mutex::new(Vec::new()),
            resumes: Mutex::new(Vec::new()),
        }
    }

    fn on_board(&self, project_id: &str, session_id: &str) -> bool {
        self.store
            .read(|s| s.board(project_id).is_some_and(|b| b.nodes.contains_key(session_id)))
    }
}

impl Launcher for FakeLauncher {
    fn start_new(&self, start: StartNew) -> BoxFuture<'_, Result<(), String>> {
        let on_board = self.on_board(&start.project.id, &start.node.session_id);
        self.log
            .lock()
            .unwrap()
            .push(format!("start:{}:on-board={on_board}", start.node.session_id));
        self.starts.lock().unwrap().push(start);
        let result = self.fail.clone().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }

    fn resume_with_prompt(&self, resume: Resume) -> BoxFuture<'_, Result<(), String>> {
        self.log
            .lock()
            .unwrap()
            .push(format!("resume:{}:{}", resume.node.session_id, resume.prompt));
        self.resumes.lock().unwrap().push(resume);
        let result = self.fail.clone().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }

    fn type_prompt(&self, session_id: &str, text: &str) {
        self.log.lock().unwrap().push(format!("type:{session_id}:{text}"));
    }
}

pub struct Fixture {
    pub _data: tempfile::TempDir,
    pub _home: tempfile::TempDir,
    pub repo: String,
    pub store: Arc<Store>,
    pub transcripts: Arc<Transcripts>,
    pub sessions: Arc<FakeSessions>,
    pub events: Arc<RecordingSink>,
    pub board: Board,
}

impl Fixture {
    /// A board for project `p`, whose repo is `<home>/repo`.
    pub fn new() -> Self {
        let data = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let repo = repo.to_string_lossy().into_owned();
        let store = Arc::new(Store::open(data.path()).unwrap());
        let project: Project = serde_json::from_value(json!({
            "id": "p", "name": "repo", "repoPath": repo, "color": "blue", "createdAt": 1
        }))
        .unwrap();
        store.update(|s| s.projects.push(project));
        let transcripts = Arc::new(Transcripts::new(home.path().to_owned()));
        let sessions = Arc::new(FakeSessions::default());
        let events = Arc::new(RecordingSink::default());
        let board = Board::new(
            Arc::clone(&store),
            Arc::clone(&transcripts),
            Arc::clone(&sessions) as Arc<dyn BoardSessions>,
            Arc::clone(&events) as Arc<dyn nodus_domain::EventSink>,
            tokio::runtime::Handle::current(),
        )
        .with_clock(Arc::new(|| NOW));
        Self {
            _data: data,
            _home: home,
            repo,
            store,
            transcripts,
            sessions,
            events,
            board,
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.transcripts.transcript_dir(&self.repo)
    }

    /// Writes a transcript.
    pub fn write(&self, session_id: &str, lines: &[Value]) {
        fs::create_dir_all(self.dir()).unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(self.dir().join(format!("{session_id}.jsonl")), text).unwrap();
    }

    /// A conversation: a prompt at `s` seconds and a reply a second later.
    pub fn talk(&self, session_id: &str, prompt: &str, reply: &str, s: u32) {
        self.write(session_id, &conversation(prompt, reply, s));
    }

    /// Puts a node on the board as it's stored.
    pub fn put(&self, node: Value) -> BoardNode {
        let node: BoardNode = serde_json::from_value(node).unwrap();
        let stored = node.clone();
        self.store
            .update(|s| s.board_mut("p").nodes.insert(stored.session_id.clone(), stored));
        node
    }

    pub fn node(&self, id: &str) -> Option<BoardNode> {
        self.store.read(|s| s.board("p").and_then(|b| b.nodes.get(id).cloned()))
    }

    pub fn board_json(&self) -> Value {
        self.store
            .read(|s| serde_json::to_value(s.board("p").expect("a board")).unwrap())
    }
}

pub fn ts(s: u32) -> String {
    format!("2026-11-01T10:{:02}:{:02}.000Z", s / 60, s % 60)
}

pub fn ms(s: u32) -> f64 {
    nodus_transcripts::date::parse(&ts(s)).unwrap()
}

pub fn conversation(prompt: &str, reply: &str, s: u32) -> Vec<Value> {
    vec![
        json!({ "type": "user", "timestamp": ts(s), "message": { "content": prompt } }),
        json!({ "type": "assistant", "timestamp": ts(s + 1), "message": { "id": format!("m{s}"), "content": [{ "type": "text", "text": reply }] } }),
    ]
}

/// A node's JSON with the fields every node has.
pub fn node(id: &str, extra: Value) -> Value {
    let mut v = json!({ "sessionId": id, "x": 0, "y": 0, "archived": false, "createdAt": 5 });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    v
}

pub const A: &str = "00000000-0000-4000-8000-00000000000a";
pub const B: &str = "00000000-0000-4000-8000-00000000000b";
pub const C: &str = "00000000-0000-4000-8000-00000000000c";
pub const D: &str = "00000000-0000-4000-8000-00000000000d";
pub const E: &str = "00000000-0000-4000-8000-00000000000e";
