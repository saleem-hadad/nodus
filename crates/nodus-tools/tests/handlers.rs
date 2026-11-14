//! The handlers ipc.ts gives the board tools, on a real store and board with
//! fake session processes (TOOL-026 to TOOL-039), and the drawing tools
//! added since (TOOL-101 to TOOL-110).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nodus_board::geometry::{GAP, intersects};
use nodus_board::notes::NotePatch;
use nodus_board::{Board, BoardSessions, BoxFuture as BoardFuture};
use nodus_domain::{
    BoardLink, BoardNode, BoardNote, BoardShape, BoardView, Event, EventSink, FolderColor, InkColor, LinkEnd, Project,
    RecordingSink, Rect, SessionCard, SessionStatus, SessionUsage, ShapeKind,
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
        // As the core's real path does: rebuilt from its parts, so Windows
        // gets its own separators, as Node's realpath gave 0.9.4.
        let folder: std::path::PathBuf = folder.components().collect();
        let folder = folder.to_string_lossy().into_owned();
        if let Some(existing) = self
            .0
            .read(|s| s.projects.iter().find(|p| p.repo_path == folder).cloned())
        {
            return Ok((existing, false));
        }
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(
            || folder.rsplit(['/', '\\']).next().unwrap_or_default().to_owned(),
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
        Self::made(false)
    }

    /// [`Fixture::new`], as the handlers run on a Nodus server (SRV-146).
    fn on_a_server() -> Self {
        Self::made(true)
    }

    fn made(on_server: bool) -> Self {
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
            on_server,
            pictures: vec![data.path().join("board-images")],
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
async fn a_server_makes_no_project_and_no_folder_for_a_session() {
    let f = Fixture::on_a_server();
    let folder = f.home.path().join("made-by-a-session");
    assert_eq!(
        err(f.handlers.create_project(Some(&json!(folder.to_string_lossy())), None)),
        nodus_tools::SERVER_PROJECTS
    );
    assert!(!folder.exists());
    assert!(f.events.take().is_empty());
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
    let server = McpServer::new("0.9.4", Some(f.handlers));
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

// ---------------------------------------------------------------------------
// The drawing tools, added after 0.9.4 (TOOL-101 to TOOL-110)

impl Fixture {
    /// Card s2, "Fix cart rounding", at (600, 0); a free note; an archived
    /// card; and a view, all on p1.
    fn furnish(&self) {
        let mut s2 = node("s2", 600.0);
        s2.title = Some("Fix cart rounding".into());
        let mut old = node("old", 1000.0);
        old.archived = true;
        self.add("p1", s2);
        self.add("p1", old);
        self.store.update(|s| {
            let board = s.board_mut("p1");
            let note: BoardNote = serde_json::from_value(json!({
                "id": "n1", "text": "Release checklist\n- tag\n- notes", "color": "green",
                "x": 600, "y": 600, "createdAt": 1
            }))
            .unwrap();
            board.notes.insert("n1".into(), note);
            let view: BoardView = serde_json::from_value(json!({
                "id": "v1", "title": "Deploys", "description": "", "x": 1200, "y": 600,
                "version": 1, "createdAt": 1, "updatedAt": 1
            }))
            .unwrap();
            board.views.insert("v1".into(), view);
        });
    }

    fn shape(&self, id: &str) -> Option<BoardShape> {
        self.store
            .read(|s| s.board("p1").and_then(|b| b.shapes.get(id).cloned()))
    }

    fn link(&self, id: &str) -> Option<BoardLink> {
        self.store
            .read(|s| s.board("p1").and_then(|b| b.links.get(id).cloned()))
    }

    fn counts(&self) -> (usize, usize) {
        self.store.read(|s| {
            let board = s.board("p1").unwrap();
            (board.shapes.values().count(), board.links.values().count())
        })
    }
}

fn ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn changed(project_id: &str) -> Vec<Event> {
    vec![Event::BoardChanged {
        project_id: project_id.into(),
    }]
}

#[tokio::test]
async fn draws_shapes_in_a_row_beside_the_sessions_card_and_links_them() {
    let f = Fixture::new();
    f.furnish();
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({
                "shapes": [
                    { "key": "api", "kind": "rect", "text": "API" },
                    { "key": "db", "kind": "ellipse", "text": "DB", "color": "blue" },
                    { "kind": "sticker", "text": "🚀" }
                ],
                "links": [
                    { "from": "api", "to": "db", "label": "reads" },
                    { "from": "self", "to": "api" },
                    { "from": "s2", "to": "self", "label": "blocks" }
                ]
            }),
        )
        .unwrap();
    assert_eq!(f.events.take(), changed("p1"));
    let names: Vec<&String> = drawn["shapes"].as_object().unwrap().keys().collect();
    assert_eq!(names, ["api", "db", "2"]);
    let shape = |name: &str| f.shape(drawn["shapes"][name].as_str().unwrap()).unwrap();
    // TOOL-104: right of s1's card at (2000, 0), lined up with its top; in a
    // row 64 apart, on their middles.
    let (api, db, sticker) = (shape("api"), shape("db"), shape("2"));
    assert_eq!(
        (api.kind, api.x, api.y, api.w, api.h),
        (ShapeKind::Rect, 2320.0, 0.0, 180.0, 90.0)
    );
    assert_eq!((db.kind, db.x, db.color), (ShapeKind::Ellipse, 2564.0, InkColor::Blue));
    assert_eq!(
        (sticker.x, sticker.y, sticker.w, sticker.text.as_str()),
        (2808.0, 13.0, 64.0, "🚀")
    );
    assert_eq!(
        (api.color, api.by.as_deref(), api.created_at),
        (InkColor::Graphite, Some("s1"), NOW)
    );
    assert_eq!(drawn["area"], json!({ "x": 2320.0, "y": 0.0, "w": 552.0, "h": 90.0 }));

    let links = ids(&drawn["links"]);
    assert_eq!(links.len(), 3);
    let reads = f.link(&links[0]).unwrap();
    assert_eq!(
        reads.from,
        LinkEnd {
            node: Some(api.id.clone()),
            x: 2410.0,
            y: 45.0
        }
    );
    assert_eq!(
        (reads.to.node.as_deref(), reads.label.as_str()),
        (Some(db.id.as_str()), "reads")
    );
    let mine = f.link(&links[1]).unwrap();
    assert_eq!(
        (mine.from.node.as_deref(), mine.by.as_deref()),
        (Some("s1"), Some("s1"))
    );
    let blocks = f.link(&links[2]).unwrap();
    assert_eq!(
        (blocks.from.node.as_deref(), blocks.to.node.as_deref()),
        (Some("s2"), Some("s1"))
    );
}

#[tokio::test]
async fn draws_at_the_point_given_keeping_the_shapes_places() {
    let f = Fixture::new();
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({
                "at": { "x": 100, "y": -50 },
                "shapes": [
                    { "kind": "diamond", "x": 0, "y": 0 },
                    { "kind": "text", "text": "Ship it", "x": 200.4, "y": 20, "w": 5, "h": 9000 }
                ]
            }),
        )
        .unwrap();
    let diamond = f.shape(drawn["shapes"]["0"].as_str().unwrap()).unwrap();
    let words = f.shape(drawn["shapes"]["1"].as_str().unwrap()).unwrap();
    assert_eq!(
        (diamond.x, diamond.y, diamond.w, diamond.h),
        (100.0, -50.0, 160.0, 110.0)
    );
    // Sizes stay within what the board keeps.
    assert_eq!((words.x, words.y, words.w, words.h), (300.0, -30.0, 16.0, 4000.0));
}

#[tokio::test]
async fn keeps_a_drawing_clear_of_what_is_on_the_board() {
    let f = Fixture::new();
    f.furnish();
    let row = json!({ "shapes": [{ "kind": "rect" }, { "kind": "rect" }, { "kind": "rect" }] });
    let first = f.handlers.draw_on_board("p1", "s1", &row).unwrap();
    let second = f.handlers.draw_on_board("p1", "s1", &row).unwrap();
    let area = |drawn: &Value| Rect {
        x: drawn["area"]["x"].as_f64().unwrap(),
        y: drawn["area"]["y"].as_f64().unwrap(),
        w: drawn["area"]["w"].as_f64().unwrap(),
        h: drawn["area"]["h"].as_f64().unwrap(),
    };
    let (a, b) = (area(&first), area(&second));
    assert!(!intersects(a, b, GAP / 2.0), "{a:?} {b:?}");
    let card = Rect {
        x: 2000.0,
        y: 0.0,
        w: 288.0,
        h: 216.0,
    };
    assert!(!intersects(b, card, GAP / 2.0));
    // Still beside the card, not a whole card's step away.
    assert!(b.x <= 2000.0 + 288.0 + GAP && b.y <= 216.0 + GAP, "{b:?}");

    // A run has no card on the board, so it draws beside the rightmost one.
    let mut run = node("run", 0.0);
    run.run_of = Some("a1".into());
    f.add("p1", run);
    let drawn = f
        .handlers
        .draw_on_board("p1", "run", &json!({ "shapes": [{ "kind": "sticker", "text": "✅" }] }))
        .unwrap();
    let c = area(&drawn);
    assert!(
        c.x >= 2000.0 - GAP - 64.0 && c.x <= 2000.0 + 288.0 + GAP + 600.0,
        "{c:?}"
    );

    // A view's brain draws beside its view, at (1200, 600) and 480 wide.
    let mut brain = node("brain", 0.0);
    brain.view_of = Some("v1".into());
    f.add("p1", brain);
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "brain",
            &json!({ "shapes": [{ "kind": "sticker", "text": "📈" }] }),
        )
        .unwrap();
    assert_eq!(
        (drawn["area"]["x"].as_f64(), drawn["area"]["y"].as_f64()),
        (Some(1712.0), Some(600.0))
    );
}

#[tokio::test]
async fn refuses_a_drawing_it_cant_make_and_draws_none_of_it() {
    let f = Fixture::new();
    f.furnish();
    let mut run = node("run", 0.0);
    run.run_of = Some("a1".into());
    f.add("p1", run);
    let cases = [
        (json!({}), "Nothing to draw: give shapes, links or ink"),
        (json!({ "shapes": "box" }), "shapes must be a list"),
        (
            json!({ "shapes": [{ "kind": "star" }] }),
            "shapes[0]: kind must be rect, ellipse, diamond, text, sticker or image",
        ),
        (
            json!({ "shapes": [{ "kind": "rect" }, { "kind": "image" }] }),
            "shapes[1]: a picture needs path, its file’s absolute path",
        ),
        (
            json!({ "shapes": [{ "kind": "image", "path": "pic.png" }] }),
            "shapes[0]: path must be absolute",
        ),
        (
            json!({ "ink": [{ "kind": "squiggle" }] }),
            "ink[0]: kind must be pen, arrow, rect, text, circle, underline",
        ),
        (
            json!({ "ink": [{ "kind": "circle", "around": "nope" }] }),
            "ink[0]: “nope” isn’t a key from shapes or the id of anything drawn on this board; list_board lists what’s there",
        ),
        (
            json!({ "ink": [{ "kind": "underline" }] }),
            "ink[0]: under must be what it marks: an id from list_board, a key from shapes, \"self\", or a list of them",
        ),
        (
            json!({ "ink": [{ "kind": "pen", "points": [[0, 0]] }] }),
            "ink[0]: a pen stroke needs points, 2 to 2000 of them, each [x, y] in board coordinates",
        ),
        (
            json!({ "ink": [{ "kind": "arrow", "from": "s1" }] }),
            "ink[0]: an arrow needs from and to",
        ),
        (
            json!({ "ink": [{ "kind": "arrow", "from": { "x": 0, "y": 0 }, "to": [3, 4] }] }),
            "ink[0]: the arrow’s ends are too close together to draw",
        ),
        (
            json!({ "ink": [{ "kind": "text", "text": "hi" }] }),
            "ink[0]: a text mark needs x and y, or beside",
        ),
        (
            json!({ "ink": [{ "kind": "text", "x": 0, "y": 0 }] }),
            "ink[0]: a text mark needs some text",
        ),
        (
            json!({ "ink": [{ "kind": "circle", "around": "self", "size": 9 }] }),
            "ink[0]: size must be between 0.5 and 4",
        ),
        (
            json!({ "shapes": [{ "kind": "text", "text": "  " }] }),
            "shapes[0]: a text shape needs some text",
        ),
        (
            json!({ "shapes": [{ "kind": "sticker" }] }),
            "shapes[0]: a sticker needs its emoji in text",
        ),
        (
            json!({ "shapes": [{ "kind": "rect", "color": "yellow" }] }),
            "shapes[0]: color must be graphite, red, orange, green, blue or purple",
        ),
        (
            json!({ "shapes": [{ "kind": "rect", "x": 5 }] }),
            "shapes[0]: give both x and y, or neither to put it in the row",
        ),
        (
            json!({ "shapes": [{ "kind": "rect", "w": "big" }] }),
            "shapes[0]: w must be a number",
        ),
        (
            json!({ "shapes": [{ "key": "a", "kind": "rect" }, { "key": "a", "kind": "rect" }] }),
            "shapes[1]: another shape goes by “a”; give each shape its own key",
        ),
        (
            json!({ "shapes": [{ "key": "self", "kind": "rect" }] }),
            "shapes[0]: “self” means this session’s card; give the shape another key",
        ),
        (
            json!({ "shapes": [{ "kind": "rect" }], "at": { "x": 1 } }),
            "at must be {\"x\": …, \"y\": …}, in board coordinates as list_board gives them",
        ),
        (
            json!({ "shapes": [{ "key": "a", "kind": "rect" }], "links": [{ "from": "a", "to": "nope" }] }),
            "links[0]: “nope” isn’t a key from shapes or the id of anything on this board; list_board lists what’s there",
        ),
        (
            json!({ "links": [{ "from": "self", "to": "old" }] }),
            "links[0]: “old” isn’t drawn on the board right now (an archived card, an action run or a note stuck to a card can’t be linked)",
        ),
        (
            json!({ "links": [{ "from": "s1", "to": "self" }] }),
            "links[0]: a link needs two different ends",
        ),
        (
            json!({ "links": [{ "to": "s2" }] }),
            "links[0]: from must be a key from shapes, the id of something on the board, or “self”",
        ),
    ];
    for (args, message) in cases {
        assert_eq!(err(f.handlers.draw_on_board("p1", "s1", &args)), message, "{args}");
    }
    assert_eq!(
        err(f
            .handlers
            .draw_on_board("p1", "run", &json!({ "links": [{ "from": "self", "to": "s2" }] }))),
        "links[0]: this session has no card on the board to link"
    );
    assert_eq!(
        err(f
            .handlers
            .draw_on_board("gone", "s1", &json!({ "shapes": [{ "kind": "rect" }] }))),
        "Unknown project gone"
    );
    assert_eq!(f.counts(), (0, 0));
    assert!(f.events.take().is_empty());
}

#[tokio::test]
async fn lists_the_board_and_says_in_words_what_this_sessions_card_is_linked_to() {
    let f = Fixture::new();
    f.furnish();
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({
                "shapes": [{ "key": "api", "kind": "rect", "text": "API" }],
                "links": [
                    { "from": "self", "to": "api" },
                    { "from": "s2", "to": "self", "label": "blocks" },
                    { "from": "n1", "to": "self" },
                    { "from": "v1", "to": "s2" }
                ]
            }),
        )
        .unwrap();
    let api = drawn["shapes"]["api"].as_str().unwrap().to_owned();
    let links = ids(&drawn["links"]);
    // The user drew one to the archived card, which isn't shown while it's archived.
    f.store.update(|s| {
        let board = s.board_mut("p1");
        let hidden = BoardLink {
            id: "hidden".into(),
            from: LinkEnd::on("s1"),
            to: LinkEnd::on("old"),
            color: InkColor::Red,
            label: String::new(),
            by: None,
            created_at: 1.0,
            extra: Default::default(),
        };
        board.links.insert("hidden".into(), hidden);
    });

    let listed = f.handlers.list_board("p1", "s1").unwrap();
    assert_eq!(
        listed["linked_to_this_session"],
        json!([
            "→ shape “API”",
            "← card “Fix cart rounding” (label: blocks)",
            "← note “Release checklist”"
        ])
    );
    assert_eq!(
        listed["cards"],
        json!([
            { "id": "s1", "title": "New session", "status": "done", "x": 2000.0, "y": 0.0, "w": 288.0, "h": 216.0, "this_session": true },
            { "id": "s2", "title": "Fix cart rounding", "status": "done", "x": 600.0, "y": 0.0, "w": 288.0, "h": 216.0 }
        ])
    );
    assert_eq!(
        listed["notes"],
        json!([{ "id": "n1", "text": "Release checklist\n- tag\n- notes", "color": "green", "x": 600.0, "y": 600.0, "w": 216.0, "h": 216.0 }])
    );
    assert_eq!(
        listed["views"],
        json!([{ "id": "v1", "title": "Deploys", "x": 1200.0, "y": 600.0, "w": 480.0, "h": 360.0 }])
    );
    assert_eq!(
        listed["shapes"],
        json!([{ "id": api, "kind": "rect", "text": "API", "color": "graphite", "x": 2320.0, "y": 0.0, "w": 180.0, "h": 90.0, "yours": true }])
    );
    let shown: Vec<&str> = listed["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].as_str().unwrap())
        .collect();
    assert_eq!(shown, links.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(
        listed["links"][1],
        json!({
            "id": links[1],
            "from": { "id": "s2", "kind": "card", "name": "Fix cart rounding" },
            "to": { "id": "s1", "kind": "card", "name": "New session" },
            "label": "blocks",
            "color": "graphite",
            "yours": true
        })
    );
    assert_eq!(
        listed["links"][3]["from"],
        json!({ "id": "v1", "kind": "view", "name": "Deploys" })
    );

    // Another session sees the same links from its own side, and none of them as its own.
    let theirs = f.handlers.list_board("p1", "s2").unwrap();
    assert_eq!(
        theirs["linked_to_this_session"],
        json!(["→ card “New session” (label: blocks)", "← view “Deploys”"])
    );
    assert!(theirs["links"][0].get("yours").is_none());
    assert!(theirs["cards"][0].get("this_session").is_none());
    assert!(f.handlers.list_board("gone", "s1").is_err());
}

#[tokio::test]
async fn changes_shapes_and_links_and_moves_cards_notes_and_views_as_dragging_would() {
    let f = Fixture::new();
    f.furnish();
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({
                "shapes": [{ "key": "a", "kind": "rect", "text": "API" }, { "key": "s", "kind": "sticker", "text": "🚀" }],
                "links": [{ "from": "a", "to": "self" }]
            }),
        )
        .unwrap();
    let (a, sticker) = (
        drawn["shapes"]["a"].as_str().unwrap().to_owned(),
        drawn["shapes"]["s"].as_str().unwrap().to_owned(),
    );
    let link = ids(&drawn["links"])[0].clone();
    f.events.take();
    let before = f.node("p1", "s2").unwrap();

    let updated = f
        .handlers
        .update_drawing(
            "p1",
            "s1",
            &json!({
                "items": [
                    { "id": a, "x": 10.4, "y": 20, "w": 300, "text": "Gateway", "color": "blue" },
                    { "id": "s2", "x": 500.5, "y": 600 },
                    { "id": "n1", "x": 7.6 },
                    { "id": "v1", "y": -40.2 },
                    { "id": a, "h": 120 }
                ],
                "links": [{ "id": link, "label": " calls ", "color": "red", "to": "s2" }]
            }),
        )
        .unwrap();
    assert_eq!(updated, json!({ "updated": [a, "s2", "n1", "v1", link] }));
    assert_eq!(f.events.take(), changed("p1"));
    let shape = f.shape(&a).unwrap();
    assert_eq!(
        (shape.x, shape.y, shape.w, shape.h, shape.text.as_str(), shape.color),
        (10.0, 20.0, 300.0, 120.0, "Gateway", InkColor::Blue)
    );
    // Who drew it and when stay.
    assert_eq!((shape.by.as_deref(), shape.created_at), (Some("s1"), NOW));
    // A card moves as a card dragged on the board does, and nothing else changes.
    let moved = f.node("p1", "s2").unwrap();
    assert_eq!((moved.x, moved.y), (500.5, 600.0));
    assert_eq!(
        BoardNode {
            x: before.x,
            y: before.y,
            ..moved
        },
        before
    );
    let (note, view) = f.store.read(|s| {
        let board = s.board("p1").unwrap();
        (board.notes["n1"].clone(), board.views["v1"].clone())
    });
    assert_eq!((note.x, note.y, view.x, view.y), (8.0, 600.0, 1200.0, -40.0));
    let changed_link = f.link(&link).unwrap();
    assert_eq!(
        (
            changed_link.label.as_str(),
            changed_link.color,
            changed_link.to.node.as_deref()
        ),
        ("calls", InkColor::Red, Some("s2"))
    );
    assert_eq!(changed_link.from.node.as_deref(), Some(a.as_str()));

    // What it can't do, it says, and it changes nothing of the call.
    let cases = [
        (json!({}), "Nothing to change: give items, links or both".to_owned()),
        (
            json!({ "items": [{ "id": a, "x": 0 }, { "id": "s2", "w": 400 }] }),
            "items[1]: “s2” is a card, which update_drawing only moves, so it takes x and y but not w".to_owned(),
        ),
        (
            json!({ "items": [{ "id": "n1", "text": "New" }] }),
            "items[0]: “n1” is a note, which update_drawing only moves, so it takes x and y but not text; update_note changes its text and colour".to_owned(),
        ),
        (
            json!({ "items": [{ "id": "old", "x": 0 }] }),
            "items[0]: “old” isn’t drawn on the board right now, so it can’t be moved".to_owned(),
        ),
        (
            json!({ "items": [{ "id": link, "x": 0 }] }),
            format!("items[0]: “{link}” is a link; change it under links"),
        ),
        (
            json!({ "items": [{ "id": "nope", "x": 0 }] }),
            "items[0]: there’s nothing with the id “nope” on this board; list_board lists what’s there".to_owned(),
        ),
        (json!({ "items": [{ "id": a }] }), "items[0]: nothing to change; give x, y, w, h, text or color".to_owned()),
        (json!({ "items": [{ "x": 1 }] }), "items[0]: id is required".to_owned()),
        (
            json!({ "items": [{ "id": sticker, "text": "" }] }),
            "items[0]: a sticker needs its emoji in text".to_owned(),
        ),
        (
            json!({ "links": [{ "id": link, "from": "s2" }] }),
            "links[0]: a link needs two different ends".to_owned(),
        ),
        (
            json!({ "links": [{ "id": "nope", "label": "x" }] }),
            "links[0]: there’s no link with the id “nope” on this board; list_board lists them".to_owned(),
        ),
        (
            json!({ "links": [{ "id": link, "to": "old" }] }),
            "links[0]: “old” isn’t drawn on the board right now (an archived card, an action run or a note stuck to a card can’t be linked)".to_owned(),
        ),
    ];
    for (args, message) in cases {
        assert_eq!(err(f.handlers.update_drawing("p1", "s1", &args)), message, "{args}");
    }
    assert_eq!(f.shape(&a).unwrap().x, 10.0);
    assert!(f.events.take().is_empty());
}

#[tokio::test]
async fn erases_shapes_with_their_links_but_never_cards_notes_or_views() {
    let f = Fixture::new();
    f.furnish();
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({
                "shapes": [{ "key": "a", "kind": "rect" }, { "key": "b", "kind": "ellipse" }],
                "links": [{ "from": "a", "to": "b" }, { "from": "self", "to": "a" }, { "from": "s2", "to": "self" }]
            }),
        )
        .unwrap();
    let (a, b) = (
        drawn["shapes"]["a"].as_str().unwrap().to_owned(),
        drawn["shapes"]["b"].as_str().unwrap().to_owned(),
    );
    let links = ids(&drawn["links"]);
    f.events.take();
    let erased = f
        .handlers
        .erase_from_board(
            "p1",
            "s1",
            &json!({ "ids": [a, "s1", "n1", "v1", "nope", 5, links[2], a] }),
        )
        .unwrap();
    assert_eq!(
        erased,
        json!({
            "erased": [a, links[2], links[0], links[1]],
            "skipped": [
                { "id": "s1", "reason": "it is a card; only shapes, links and ink can be erased" },
                { "id": "n1", "reason": "it is a note; only shapes, links and ink can be erased" },
                { "id": "v1", "reason": "it is a view; only shapes, links and ink can be erased" },
                { "id": "nope", "reason": "not on the board" }
            ]
        })
    );
    assert_eq!(f.events.take(), changed("p1"));
    assert!(f.shape(&b).is_some());
    assert_eq!(f.counts(), (1, 0));
    assert!(f.node("p1", "s1").is_some());

    // Nothing erased, nothing said.
    let none = f
        .handlers
        .erase_from_board("p1", "s1", &json!({ "ids": ["s2"] }))
        .unwrap();
    assert_eq!(none["erased"], json!([]));
    assert!(f.events.take().is_empty());
    assert_eq!(
        err(f.handlers.erase_from_board("p1", "s1", &json!({ "ids": "all" }))),
        "ids must be a list of the ids of shapes, links and ink, from list_board"
    );
    assert_eq!(
        err(f.handlers.erase_from_board("p1", "s1", &json!({ "ids": [] }))),
        "Nothing to erase: give the ids of shapes, links and ink, from list_board"
    );
}

#[tokio::test]
async fn draws_and_lists_through_the_server_end_to_end() {
    let f = Fixture::new();
    let server = McpServer::new("1.0.8", Some(f.handlers));
    let call = |id: i64, name: &str, args: Value| json!({ "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let drawn = server
        .answer(
            "s1",
            &call(
                1,
                "draw_on_board",
                json!({ "shapes": [{ "kind": "text", "text": "Plan" }] }),
            ),
        )
        .await;
    assert!(drawn["result"].get("isError").is_none(), "{drawn}");
    let listed = server.answer("s1", &call(2, "list_board", json!({}))).await;
    let text = listed["result"]["content"][0]["text"].as_str().unwrap();
    let listed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(listed["shapes"][0]["text"], "Plan");
    let bad = server
        .answer(
            "s1",
            &call(3, "draw_on_board", json!({ "shapes": [{ "kind": "image" }] })),
        )
        .await;
    assert_eq!(bad["result"]["isError"], true);
}

// ---------------------------------------------------------------------------
// Pictures, ink and the board as a picture (TOOL-111 to TOOL-113)

/// A PNG's first bytes, enough for its kind and size.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&w.to_be_bytes());
    png.extend_from_slice(&h.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    png
}

/// A whole 1×1 PNG.
const DOT_PNG: [u8; 69] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49,
    0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0xb0,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

impl Fixture {
    fn pictures(&self) -> std::path::PathBuf {
        self._data.path().join("board-images")
    }

    fn pictures_kept(&self) -> usize {
        std::fs::read_dir(self.pictures()).map_or(0, Iterator::count)
    }

    /// A file in the home folder, by name.
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.home.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }
}

#[tokio::test]
async fn puts_up_a_picture_from_a_file_at_its_own_proportions() {
    // TOOL-112
    let f = Fixture::new();
    let wide = f.file("wide.png", &png_header(1200, 600));
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({ "shapes": [
                { "key": "shot", "kind": "image", "path": wide },
                { "kind": "image", "path": wide, "w": 200 }
            ] }),
        )
        .unwrap();
    let shapes: Vec<BoardShape> = f
        .store
        .read(|s| s.board("p1").unwrap().shapes.values().cloned().collect());
    assert_eq!(shapes.len(), 2);
    assert_eq!(shapes[0].id, drawn["shapes"]["shot"].as_str().unwrap());
    assert_eq!(
        (shapes[0].kind, shapes[0].w, shapes[0].h),
        (ShapeKind::Image, 480.0, 240.0)
    );
    assert_eq!((shapes[1].w, shapes[1].h), (200.0, 100.0));
    // Each a copy, under a name of its own, kept with the board's pictures.
    let name = shapes[0].image.clone().unwrap();
    assert!(name.ends_with(".png") && name != shapes[1].image.clone().unwrap());
    assert_eq!(std::fs::read(f.pictures().join(&name)).unwrap(), png_header(1200, 600));
    assert_eq!(shapes[0].by.as_deref(), Some("s1"));

    // What isn't a picture, or isn't there, isn't put up.
    let text = f.file("notes.txt", b"just words");
    assert_eq!(
        err(f
            .handlers
            .draw_on_board("p1", "s1", &json!({ "shapes": [{ "kind": "image", "path": text }] }))),
        format!("shapes[0]: “{text}” isn’t a PNG, JPEG, GIF or WebP picture")
    );
    let missing = f.home.path().join("gone.png").to_string_lossy().into_owned();
    assert_eq!(
        err(f
            .handlers
            .draw_on_board("p1", "s1", &json!({ "shapes": [{ "kind": "image", "path": missing }] }))),
        format!("shapes[0]: there’s no file at “{missing}”")
    );
    // A drawing that fails leaves no copy behind.
    assert_eq!(f.pictures_kept(), 2);
    let failed = f.handlers.draw_on_board(
        "p1",
        "s1",
        &json!({ "shapes": [{ "kind": "image", "path": wide }], "links": [{ "from": "0", "to": "nope" }] }),
    );
    assert!(failed.is_err());
    assert_eq!(f.pictures_kept(), 2);
}

#[tokio::test]
async fn a_server_puts_up_no_picture_from_a_sessions_file() {
    // TOOL-112: the session's files aren't the server's to read.
    let f = Fixture::on_a_server();
    let wide = f.file("wide.png", &png_header(1200, 600));
    assert_eq!(
        err(f
            .handlers
            .draw_on_board("p1", "s1", &json!({ "shapes": [{ "kind": "image", "path": wide }] }))),
        "Pictures can’t be put up from a file on a Nodus server’s board yet"
    );
    assert_eq!(f.pictures_kept(), 0);
}

#[tokio::test]
async fn draws_ink_marks_on_and_around_what_is_on_the_board() {
    // TOOL-113: s1's card is at (2000, 0), 288 × 216.
    let f = Fixture::new();
    f.furnish();
    let drawn = f
        .handlers
        .draw_on_board(
            "p1",
            "s1",
            &json!({
                "shapes": [{ "key": "api", "kind": "rect", "text": "API", "x": 0, "y": 0 }],
                "at": { "x": 2600, "y": 0 },
                "ink": [
                    { "kind": "circle", "around": "self", "color": "red" },
                    { "kind": "underline", "under": "n1" },
                    { "kind": "arrow", "from": "self", "to": "api", "color": "blue", "size": 2 },
                    { "kind": "text", "text": "start here", "beside": "api" },
                    { "kind": "pen", "points": [[0, 0], { "x": 10, "y": 5 }, [20, 0]] },
                    { "kind": "rect", "around": ["s1", "s2"] }
                ]
            }),
        )
        .unwrap();
    let ids = ids(&drawn["ink"]);
    assert_eq!(ids.len(), 6);
    let ink = f.store.read(|s| s.board("p1").unwrap().ink.clone().unwrap());
    let kinds: Vec<&str> = ink.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, ["pen", "pen", "arrow", "text", "pen", "rect"]);
    assert!(ink.iter().all(|s| s.by() == Some("s1")));
    // The circle goes round the card; the arrow leaves its right edge for the box.
    let circle = &ink[0];
    let xs: Vec<f64> = circle.points.iter().step_by(2).copied().collect();
    assert!(xs.iter().copied().fold(f64::INFINITY, f64::min) < 2000.0);
    assert!(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) > 2288.0);
    assert_eq!(circle.color, InkColor::Red);
    let arrow = &ink[2];
    assert!(
        arrow.points[0] > 2288.0 && arrow.points[2] < 2600.0,
        "{:?}",
        arrow.points
    );
    assert_eq!(arrow.size, 2.0);
    assert_eq!(ink[3].text.as_deref(), Some("start here"));
    assert_eq!(ink[4].points, [0.0, 0.0, 10.0, 5.0, 20.0, 0.0]);
    // The box spans both cards, s2 at (600, 0).
    assert!(ink[5].points[0] < 600.0 && ink[5].points[2] > 2288.0);

    // list_board says what each points at and goes around, and whose it is.
    let listed = f.handlers.list_board("p1", "s1").unwrap();
    let listed_ink = listed["ink"].as_array().unwrap();
    assert_eq!(listed_ink.len(), 6);
    assert_eq!(
        listed_ink[0]["around"],
        json!([{ "id": "s1", "kind": "card", "name": "New session" }])
    );
    assert_eq!(listed_ink[2]["from"]["id"], "s1");
    assert_eq!(listed_ink[2]["to"]["name"], "API");
    assert_eq!(listed_ink[3]["text"], "start here");
    assert!(listed_ink.iter().all(|s| s["yours"] == true));
    let theirs = f.handlers.list_board("p1", "s2").unwrap();
    assert!(
        theirs["ink"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s.get("yours").is_none())
    );

    // Ink isn't changed in place, but it's erased by id.
    assert_eq!(
        err(f
            .handlers
            .update_drawing("p1", "s1", &json!({ "items": [{ "id": ids[0], "x": 5 }] }))),
        format!(
            "items[0]: “{}” is ink, which update_drawing doesn’t change; erase it and draw it again",
            ids[0]
        )
    );
    let erased = f
        .handlers
        .erase_from_board("p1", "s1", &json!({ "ids": [ids[0], ids[1]] }))
        .unwrap();
    assert_eq!(erased["erased"], json!([ids[0], ids[1]]));
    assert_eq!(f.store.read(|s| s.board("p1").unwrap().ink.as_ref().unwrap().len()), 4);
}

#[tokio::test]
async fn looks_at_the_board_as_a_picture_tagging_what_is_on_it() {
    // TOOL-111
    let f = Fixture::new();
    f.furnish();
    std::fs::create_dir_all(f.pictures()).unwrap();
    std::fs::write(f.pictures().join("dot.png"), DOT_PNG).unwrap();
    f.store.update(|s| {
        let board = s.board_mut("p1");
        let picture: BoardShape = serde_json::from_value(json!({
            "id": "pic", "kind": "image", "x": 2000, "y": 400, "w": 200, "h": 200,
            "color": "graphite", "image": "dot.png", "createdAt": 1
        }))
        .unwrap();
        board.shapes.insert("pic".into(), picture);
    });
    f.handlers
        .draw_on_board("p1", "s1", &json!({ "ink": [{ "kind": "circle", "around": "pic" }] }))
        .unwrap();

    let whole = f.handlers.look_at_board("p1", "s1", &json!({})).unwrap();
    assert!(whole.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let (w, h) = nodus_board::pictures::picture_size(&whole.png).unwrap();
    assert!(w.max(h) <= 1568 && w.max(h) >= 1500, "{w} × {h}");
    let tags = whole.key["tags"].as_object().unwrap();
    let tagged = |id: &str| {
        tags.iter()
            .find(|(_, t)| t["id"] == id)
            .map(|(tag, t)| (tag.clone(), t.clone()))
    };
    let (tag, own) = tagged("s1").unwrap();
    assert!(tag.starts_with('c') && own["this_session"] == true, "{own}");
    assert_eq!(tagged("s2").unwrap().1["kind"], "card");
    assert!(tagged("n1").unwrap().0.starts_with('n'));
    assert!(tagged("v1").unwrap().0.starts_with('v'));
    assert_eq!(tagged("pic").unwrap().0, "p1");
    assert_eq!(tagged("pic").unwrap().1["kind"], "picture");
    assert!(tags.iter().any(|(tag, t)| tag.starts_with('i') && t["kind"] == "ink"));

    // Around something: its box and the margin, and only what's in it.
    let near = f
        .handlers
        .look_at_board("p1", "s1", &json!({ "around": "pic", "margin": 50 }))
        .unwrap();
    assert_eq!(
        near.key["area"],
        json!({ "x": 1950.0, "y": 350.0, "w": 300.0, "h": 300.0 })
    );
    assert_eq!(near.key["scale"], 2.0);
    assert!(near.key["tags"].as_object().unwrap().values().all(|t| t["id"] != "s2"));
    let area = f
        .handlers
        .look_at_board("p1", "s1", &json!({ "area": { "x": 0, "y": 0, "w": 3136, "h": 100 } }))
        .unwrap();
    assert_eq!(area.key["scale"], 0.5);
    assert_eq!(nodus_board::pictures::picture_size(&area.png), Some((1568, 50)));

    let cases = [
        (
            json!({ "area": { "x": 0, "y": 0, "w": 10, "h": 10 }, "around": "s1" }),
            "give area or around, not both",
        ),
        (
            json!({ "area": { "x": 0, "y": 0, "w": 0, "h": 10 } }),
            "area must be {\"x\": …, \"y\": …, \"w\": …, \"h\": …} in board coordinates as list_board gives them, w and h above 0",
        ),
        (
            json!({ "around": "nope" }),
            "“nope” isn’t drawn on this board; list_board lists what’s there",
        ),
        (
            json!({ "around": 5 }),
            "around must be an id from list_board, a list of them, or \"self\"",
        ),
        (
            json!({ "margin": -1 }),
            "margin must be a number of board units, 0 or more",
        ),
    ];
    for (args, message) in cases {
        assert_eq!(err(f.handlers.look_at_board("p1", "s1", &args)), message, "{args}");
    }
}
