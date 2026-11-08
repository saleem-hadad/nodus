//! The handlers ipc.ts gives the board tools, on a real store and board with
//! fake session processes (TOOL-026 to TOOL-039).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nodus_board::notes::NotePatch;
use nodus_board::{Board, BoardSessions, BoxFuture as BoardFuture};
use nodus_domain::{
    BoardNode, Event, EventSink, FolderColor, Project, RecordingSink, SessionCard, SessionStatus, SessionUsage,
};
use nodus_store::Store;
use nodus_tools::{
    BoardToolHandlers, BoxFuture, McpServer, ProjectAdder, StoreHandlers, StoreHandlersConfig, ToolError, ViewTools,
    expand_home, find_project, is_absolute,
};
use nodus_transcripts::Transcripts;
use serde_json::{Value, json};

const NOW: f64 = 1_800_000_000_000.0;

#[derive(Default)]
struct FakeSessions {
    status: Mutex<HashMap<String, SessionStatus>>,
    log: Mutex<Vec<String>>,
}

impl BoardSessions for FakeSessions {
    fn is_live(&self, session_id: &str) -> bool {
        self.status.lock().unwrap().contains_key(session_id)
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
    fn kill(&self, _session_id: &str) -> BoardFuture<'_, ()> {
        Box::pin(async {})
    }
    fn forget_status(&self, _session_id: &str) {}
}

/// `addProject` as the core has it, for these tests: an existing project by
/// folder, or a new one called `name` or the folder's name.
struct Adder(Arc<Store>);

impl ProjectAdder for Adder {
    fn add_project(&self, folder: &Path, name: Option<&str>) -> Result<(Project, bool), String> {
        let folder = folder.to_string_lossy().into_owned();
        if let Some(existing) = self
            .0
            .read(|s| s.projects.iter().find(|p| p.repo_path == folder).cloned())
        {
            return Ok((existing, false));
        }
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(
            || folder.rsplit('/').next().unwrap_or_default().to_owned(),
            str::to_owned,
        );
        let project = project("new", &name, &folder);
        self.0.update(|s| s.projects.push(project.clone()));
        Ok((project, true))
    }
}

struct NoViews;

impl ViewTools for NoViews {
    fn list_views(&self, _: &str, _: &str) -> Result<Value, String> {
        Ok(json!([]))
    }
    fn get_view(&self, _: &str, _: &str, _: Option<&Value>) -> Result<Value, String> {
        Err("There’s no view with that id on this board".into())
    }
    fn create_view<'a>(&'a self, _: &'a str, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(json!({ "view_id": "v" })) })
    }
    fn update_view<'a>(&'a self, _: &'a str, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Err("A view needs a title".into()) })
    }
}

fn project(id: &str, name: &str, repo: &str) -> Project {
    serde_json::from_value(json!({ "id": id, "name": name, "repoPath": repo, "color": "blue", "createdAt": 1 }))
        .unwrap()
}

fn node(id: &str, x: f64) -> BoardNode {
    serde_json::from_value(json!({ "sessionId": id, "x": x, "y": 0, "archived": false, "createdAt": 0 })).unwrap()
}

struct Fixture {
    _data: tempfile::TempDir,
    home: tempfile::TempDir,
    store: Arc<Store>,
    board: Arc<Board>,
    transcripts: Arc<Transcripts>,
    sessions: Arc<FakeSessions>,
    events: Arc<RecordingSink>,
    handlers: StoreHandlers,
    repo: String,
}

impl Fixture {
    /// Project p1 (repo `<home>/repo`) with card s1 at (2000, 0), and project
    /// p2, "Docs", with card d1.
    fn new() -> Self {
        let data = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let repo = home.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let repo = repo.to_string_lossy().into_owned();
        let store = Arc::new(Store::open(data.path()).unwrap());
        store.update(|s| {
            s.projects.push(project("p1", "Shop", &repo));
            s.projects.push(project("p2", "Docs", &repo));
            s.board_mut("p1").nodes.insert("s1".into(), node("s1", 2000.0));
            s.board_mut("p2").nodes.insert("d1".into(), node("d1", 0.0));
        });
        let transcripts = Arc::new(Transcripts::new(home.path().to_owned()));
        let sessions = Arc::new(FakeSessions::default());
        let events = Arc::new(RecordingSink::default());
        let board = Arc::new(
            Board::new(
                Arc::clone(&store),
                Arc::clone(&transcripts),
                Arc::clone(&sessions) as Arc<dyn BoardSessions>,
                Arc::clone(&events) as Arc<dyn EventSink>,
                tokio::runtime::Handle::current(),
            )
            .with_clock(Arc::new(|| NOW)),
        );
        let handlers = StoreHandlers::new(StoreHandlersConfig {
            store: Arc::clone(&store),
            board: Arc::clone(&board),
            sessions: Arc::clone(&sessions) as Arc<dyn BoardSessions>,
            events: Arc::clone(&events) as Arc<dyn EventSink>,
            projects: Arc::new(Adder(Arc::clone(&store))),
            views: Arc::new(NoViews),
            clock: Arc::new(|| NOW),
            home: Some(home.path().to_owned()),
        });
        Self {
            _data: data,
            home,
            store,
            board,
            transcripts,
            sessions,
            events,
            handlers,
            repo,
        }
    }

    fn add(&self, project_id: &str, node: BoardNode) {
        self.store
            .update(|s| s.board_mut(project_id).nodes.insert(node.session_id.clone(), node));
    }

    /// A transcript with a prompt and a reply, so the session isn't empty.
    fn said_something(&self, session_id: &str) {
        let dir = self.transcripts.transcript_dir(&self.repo);
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            json!({ "type": "user", "sessionId": session_id, "timestamp": "2026-01-01T00:00:00.000Z",
                    "message": { "role": "user", "content": "Fix the build" } }),
            json!({ "type": "assistant", "sessionId": session_id, "timestamp": "2026-01-01T00:00:01.000Z",
                    "message": { "role": "assistant", "content": [{ "type": "text", "text": "Fixed." }] } }),
        ];
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(dir.join(format!("{session_id}.jsonl")), text).unwrap();
    }

    fn node(&self, project_id: &str, id: &str) -> Option<BoardNode> {
        self.store
            .read(|s| s.board(project_id).and_then(|b| b.nodes.get(id).cloned()))
    }
}

fn err(result: Result<impl std::fmt::Debug, ToolError>) -> String {
    result.unwrap_err().0
}

#[test]
fn finds_a_project_by_id_or_by_name_in_any_case() {
    let projects = [
        project("p1", "Shop", "/a"),
        project("p2", "Docs", "/b"),
        project("p3", "docs", "/c"),
        project("Shop", "Other", "/d"),
    ];
    // TOOL-028: an exact id first.
    assert_eq!(find_project(&projects, Some(&json!("p1"))).unwrap(), "p1");
    assert_eq!(find_project(&projects, Some(&json!("  Shop "))).unwrap(), "Shop");
    assert_eq!(find_project(&projects, Some(&json!("OTHER"))).unwrap(), "Shop");
    assert_eq!(
        err(find_project(&projects, Some(&json!("DOCS")))),
        "More than one project is called “DOCS”; use its id from list_projects"
    );
    assert_eq!(
        err(find_project(&projects, Some(&json!("Nowhere")))),
        "There’s no project called “Nowhere”; list_projects lists them"
    );
    for bad in [json!(""), json!("   "), json!(null), json!(5)] {
        assert_eq!(
            err(find_project(&projects, Some(&bad))),
            "project must be a project’s id or name, from list_projects"
        );
    }
    assert!(find_project(&projects, None).is_err());
}

#[test]
fn expands_a_leading_tilde_and_knows_an_absolute_path() {
    let home = Path::new("/Users/example");
    assert_eq!(expand_home("~", Some(home)), "/Users/example");
    assert_eq!(expand_home("~/Code/app", Some(home)), "/Users/example/Code/app");
    assert_eq!(expand_home("~\\Code", Some(home)), "/Users/example\\Code");
    assert_eq!(expand_home("~other/Code", Some(home)), "~other/Code");
    assert_eq!(expand_home("/a/~", Some(home)), "/a/~");
    assert!(is_absolute("/a"));
    assert!(!is_absolute("a/b"));
    assert!(!is_absolute("~/a"));
    if cfg!(windows) {
        assert!(is_absolute("C:\\Code") && is_absolute("\\Code") && !is_absolute("C:Code"));
    }
}

#[tokio::test]
async fn knows_the_project_a_session_is_on_and_whether_it_is_a_run() {
    let f = Fixture::new();
    let mut run = node("run", 0.0);
    run.run_of = Some("a1".into());
    f.add("p1", run);
    assert_eq!(f.handlers.project_for_session("s1").as_deref(), Some("p1"));
    assert_eq!(f.handlers.project_for_session("nobody"), None);
    assert!(f.handlers.is_run("run"));
    assert!(!f.handlers.is_run("s1"));
    assert!(!f.handlers.is_run("nobody"));
}

#[tokio::test]
async fn lists_projects_saying_which_is_archived_and_which_is_this_sessions() {
    let f = Fixture::new();
    f.store.update(|s| s.projects[1].archived = Some(true));
    assert_eq!(
        f.handlers.list_projects("p1").unwrap(),
        json!([
            { "id": "p1", "name": "Shop", "folder": f.repo, "thisSessionsBoard": true },
            { "id": "p2", "name": "Docs", "folder": f.repo, "archived": true }
        ])
    );
}

#[tokio::test]
async fn adds_a_project_making_its_folder() {
    let f = Fixture::new();
    assert_eq!(err(f.handlers.create_project(None, None)), "path is required");
    assert_eq!(
        err(f.handlers.create_project(Some(&json!("  ")), None)),
        "path is required"
    );
    assert_eq!(
        err(f.handlers.create_project(Some(&json!("Code/app")), None)),
        "path must be absolute"
    );

    let made = f
        .handlers
        .create_project(Some(&json!(" ~/Code/new-app ")), Some(&json!("New app")))
        .unwrap();
    let folder = f.home.path().join("Code").join("new-app");
    assert!(folder.is_dir());
    assert_eq!(
        made,
        json!({ "projectId": "new", "name": "New app", "folder": folder.to_string_lossy(), "madeFolder": true })
    );
    assert_eq!(f.events.take(), [Event::ProjectsChanged]);

    // A folder that's already a project gives that project back.
    let again = f
        .handlers
        .create_project(Some(&json!(folder.to_string_lossy())), Some(&json!(5)))
        .unwrap();
    assert_eq!(again["alreadyAProject"], true);
    assert!(again.get("madeFolder").is_none());
}

#[tokio::test]
async fn lists_the_boards_sessions_for_an_action_run() {
    let f = Fixture::new();
    let listed = f.handlers.list_sessions("p1", "run", false).unwrap();
    assert_eq!(listed[0]["id"], "s1");
    assert_eq!(listed[0]["title"], "New session");
    assert_eq!(listed[0]["status"], "done");
    assert!(
        f.handlers
            .list_sessions("p1", "s1", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn archives_sessions_skipping_what_it_shouldnt_and_removing_empty_ones() {
    let f = Fixture::new();
    let mut brain = node("brain", 0.0);
    brain.view_of = Some("v".into());
    let mut terminal = node("term", 0.0);
    terminal.kind = Some(nodus_domain::SessionKind::Terminal);
    let mut old = node("old", 0.0);
    old.archived = true;
    for n in [
        brain,
        terminal,
        old,
        node("busy", 0.0),
        node("asking", 0.0),
        node("full", 0.0),
        node("empty", 0.0),
    ] {
        f.add("p1", n);
    }
    f.said_something("full");
    f.said_something("busy");
    f.sessions.status.lock().unwrap().extend([
        ("busy".to_owned(), SessionStatus::Working),
        ("asking".to_owned(), SessionStatus::Waiting),
    ]);
    let ids: Vec<String> = [
        "__proto__",
        "run",
        "brain",
        "term",
        "old",
        "busy",
        "asking",
        "full",
        "empty",
    ]
    .map(str::to_owned)
    .to_vec();
    f.add("p1", node("run", 0.0));
    let answer = f.handlers.archive_sessions("p1", "run", &ids, false).unwrap();
    assert_eq!(
        answer,
        json!({
            "archived": ["full"],
            "removed": ["empty"],
            "skipped": [
                { "id": "__proto__", "reason": "not on the board" },
                { "id": "run", "reason": "it is the session running this action" },
                { "id": "brain", "reason": "it is a view’s brain, which goes with its view" },
                { "id": "term", "reason": "terminals can’t be archived" },
                { "id": "old", "reason": "already archived" },
                { "id": "busy", "reason": "it is working" },
                { "id": "asking", "reason": "it is waiting for the user" }
            ]
        })
    );
    assert!(f.node("p1", "full").unwrap().archived);
    assert_eq!(f.node("p1", "full").unwrap().archived_at, Some(NOW));
    assert!(f.node("p1", "empty").is_none());
    assert_eq!(*f.sessions.log.lock().unwrap(), ["stop:full", "stop+forget:empty"]);
    assert_eq!(
        f.events.take(),
        [Event::BoardChanged {
            project_id: "p1".into()
        }]
    );

    // Forced, a busy session goes too; with nothing changed, nothing is said.
    let forced = f
        .handlers
        .archive_sessions("p1", "run", &["busy".to_owned()], true)
        .unwrap();
    assert_eq!(forced["archived"], json!(["busy"]));
    f.events.take();
    let none = f
        .handlers
        .archive_sessions("p1", "run", &["nobody".to_owned()], true)
        .unwrap();
    assert_eq!(none["archived"], json!([]));
    assert!(f.events.take().is_empty());
    assert_eq!(
        err(f.handlers.archive_sessions("gone", "run", &[], false)),
        "Unknown project gone"
    );
}

#[tokio::test]
async fn archives_the_calling_session_once_its_turn_ends() {
    let f = Fixture::new();
    let mut brain = node("brain", 0.0);
    brain.view_of = Some("v".into());
    let mut run = node("run", 0.0);
    run.run_of = Some("a".into());
    let mut terminal = node("term", 0.0);
    terminal.kind = Some(nodus_domain::SessionKind::Terminal);
    let mut old = node("old", 0.0);
    old.archived = true;
    for n in [brain, run, terminal, old] {
        f.add("p1", n);
    }
    // TOOL-037, in order.
    assert_eq!(
        err(f.handlers.archive_self("p1", "nobody")),
        "This session is no longer on the board"
    );
    assert_eq!(
        err(f.handlers.archive_self("p1", "term")),
        "Terminals can’t be archived"
    );
    assert_eq!(
        err(f.handlers.archive_self("p1", "old")),
        "This session is already archived"
    );
    assert_eq!(
        err(f.handlers.archive_self("p1", "brain")),
        "A view’s brain stays with its view; deleting the view stops it"
    );
    assert_eq!(
        err(f.handlers.archive_self("p1", "run")),
        "An action run leaves the board by itself when its turn ends"
    );
    assert_eq!(
        f.handlers.archive_self("p1", "s1").unwrap(),
        json!({ "archiving": "This session moves to the archive when this turn ends." })
    );
    assert!(!f.node("p1", "s1").unwrap().archived);
    // TOOL-038: at its next `done`.
    f.board.on_done("s1");
    assert!(f.node("p1", "s1").unwrap().archived);
}

#[tokio::test]
async fn writes_notes_here_and_on_another_projects_board_signed_with_where_they_came_from() {
    let f = Fixture::new();
    let note = |text: &str| NotePatch {
        text: Some(text.into()),
        color: Some(FolderColor::Green),
    };
    let home = f.handlers.create_note("p1", "s1", &note("Findings")).unwrap();
    let id = home["noteId"].as_str().unwrap().to_owned();
    let stored = f.store.read(|s| s.board("p1").unwrap().notes[&id].clone());
    assert_eq!(
        (stored.x, stored.from_project.as_deref()),
        (2000.0 + 288.0 + 32.0, None)
    );
    assert_eq!(stored.created_at, NOW);
    assert_eq!(
        f.events.take(),
        [Event::BoardChanged {
            project_id: "p1".into()
        }]
    );

    // TOOL-029, TOOL-030: on Docs' board it's signed with this project and the card's title.
    f.store
        .update(|s| s.board_mut("p1").nodes["s1"].title = Some("Release checklist".into()));
    let away = f.handlers.create_note("p2", "s1", &note("For the docs")).unwrap();
    let id = away["noteId"].as_str().unwrap().to_owned();
    let stored = f.store.read(|s| s.board("p2").unwrap().notes[&id].clone());
    assert_eq!(stored.from_project.as_deref(), Some("p1"));
    assert_eq!(stored.from_title.as_deref(), Some("Release checklist"));
    assert_eq!(
        f.events.take(),
        [Event::BoardChanged {
            project_id: "p2".into()
        }]
    );

    assert_eq!(
        err(f.handlers.create_note("p1", "s1", &NotePatch::default())),
        "A note needs some text"
    );
    assert_eq!(
        f.handlers.list_notes("p2", "s1").unwrap(),
        json!([{ "id": id, "text": "For the docs", "color": "green", "writtenByYou": true }])
    );
    assert_eq!(f.handlers.list_notes("nowhere", "s1").unwrap(), json!([]));
}

#[tokio::test]
async fn changes_a_note_by_id() {
    let f = Fixture::new();
    let created = f
        .handlers
        .create_note(
            "p1",
            "s1",
            &NotePatch {
                text: Some("One".into()),
                color: None,
            },
        )
        .unwrap();
    let id = created["noteId"].as_str().unwrap().to_owned();
    f.events.take();
    let patch = NotePatch {
        text: Some("Two".into()),
        color: Some(FolderColor::Red),
    };
    assert_eq!(
        f.handlers.update_note("p1", &id, &patch).unwrap(),
        json!({ "noteId": id })
    );
    assert_eq!(
        f.events.take(),
        [Event::BoardChanged {
            project_id: "p1".into()
        }]
    );
    assert_eq!(
        err(f.handlers.update_note("p1", "__proto__", &patch)),
        "There’s no note with that id on this board"
    );
}

#[tokio::test]
async fn answers_through_the_server_end_to_end() {
    let f = Fixture::new();
    let server = McpServer::new("0.9.4", "guide", Some(f.handlers));
    let reply = server
        .answer(
            "s1",
            &json!({ "id": 1, "method": "tools/call", "params": { "name": "create_note", "arguments": { "text": "Hi", "project": "docs" } } }),
        )
        .await;
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("{\n  \"noteId\": \""), "{text}");
    let views = server
        .answer(
            "s1",
            &json!({ "id": 2, "method": "tools/call", "params": { "name": "update_view", "arguments": {} } }),
        )
        .await;
    assert_eq!(views["result"]["content"][0]["text"], "A view needs a title");
    assert_eq!(views["result"]["isError"], true);
}
