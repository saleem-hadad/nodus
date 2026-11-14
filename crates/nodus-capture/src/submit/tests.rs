//! Making captures over a synthetic state.json, with the other domains faked.

use std::path::Path;
use std::sync::Mutex;

use nodus_board::geometry::find_free_spot;
use nodus_domain::{RecordingSink, Rect};
use serde_json::json;

use super::*;
use crate::targets::tests::{FakeSessions, board, node, project};

const NOW: f64 = 1_800_000_000_000.0;

/// What the fake handlers were asked, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    Create(String, CreateSessionOptions),
    Send(String, String, String),
    View(String, String, Point),
    OpenCard(OpenCardRequest),
    OpenSettings,
}

#[derive(Default)]
pub(crate) struct Handlers {
    calls: Mutex<Vec<Call>>,
    pub window: Option<(f64, f64)>,
    pub fail: Option<String>,
}

impl Handlers {
    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn answer<T: Send + 'static>(&self, call: Call, ok: T) -> BoxFuture<'_, Result<T, String>> {
        self.calls.lock().unwrap().push(call);
        let result = match &self.fail {
            Some(err) => Err(err.clone()),
            None => Ok(ok),
        };
        Box::pin(async move { result })
    }
}

impl CaptureHandlers for Handlers {
    fn window_size(&self) -> Option<(f64, f64)> {
        self.window
    }

    fn create_session(&self, project_id: &str, opts: CreateSessionOptions) -> BoxFuture<'_, Result<String, String>> {
        self.answer(Call::Create(project_id.into(), opts), "new-session".into())
    }

    fn send_to_session(&self, project_id: &str, session_id: &str, text: &str) -> BoxFuture<'_, Result<(), String>> {
        self.answer(Call::Send(project_id.into(), session_id.into(), text.into()), ())
    }

    fn session_title(&self, _project_id: &str, session_id: &str) -> String {
        format!("Title of {session_id}")
    }

    fn create_view(&self, project_id: &str, description: &str, at: Point) -> BoxFuture<'_, Result<String, String>> {
        self.answer(Call::View(project_id.into(), description.into(), at), "new-view".into())
    }

    fn open_card(&self, request: OpenCardRequest) {
        self.calls.lock().unwrap().push(Call::OpenCard(request));
    }

    fn open_settings(&self) {
        self.calls.lock().unwrap().push(Call::OpenSettings);
    }
}

/// A store over a state.json holding `raw`, in its own folder.
pub(crate) fn store_with(dir: &Path, raw: &Value) -> Store {
    let user_data = dir.join("userdata");
    std::fs::create_dir_all(&user_data).unwrap();
    std::fs::write(user_data.join("state.json"), raw.to_string()).unwrap();
    Store::open(&user_data).unwrap()
}

pub(crate) fn seeded() -> Value {
    json!({
        "projects": [project("p", "store", json!({}))],
        "boards": { "p": board(&[
            node("s1", json!({ "x": 400, "y": 600 })),
            node("gone", json!({ "archived": true })),
        ], json!({})) },
    })
}

struct Setup {
    _dir: tempfile::TempDir,
    store: Store,
    handlers: Handlers,
    sessions: FakeSessions,
    events: RecordingSink,
}

impl Setup {
    fn new(raw: &Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with(dir.path(), raw);
        Self {
            _dir: dir,
            store,
            handlers: Handlers::default(),
            sessions: FakeSessions::default(),
            events: RecordingSink::default(),
        }
    }

    async fn submit(&self, raw: Value) -> Result<CaptureReceipt, SubmitError> {
        let now = || NOW;
        Maker {
            store: &self.store,
            handlers: &self.handlers,
            sessions: &self.sessions,
            events: &self.events,
            now: &now,
        }
        .submit(&raw)
        .await
    }

    fn board(&self) -> BoardState {
        self.store.read(|s| s.board("p").cloned().unwrap())
    }

    fn records(&self) -> usize {
        self.store.read(|s| s.captures.as_ref().map_or(0, Vec::len))
    }
}

/// CAP-125: what's checked, in order, with 0.9.4's words.
#[tokio::test]
async fn checks_what_came_in_order() {
    let s = Setup::new(&seeded());
    let err = |raw: Value| async { s.submit(raw).await.unwrap_err().to_string() };
    assert_eq!(
        err(json!({ "text": "   ", "kind": "party" })).await,
        "Type something first"
    );
    assert_eq!(err(json!({ "text": 7, "kind": "note" })).await, "Type something first");
    assert_eq!(err(json!(null)).await, "Type something first");
    assert_eq!(err(json!({ "text": "x", "kind": "party" })).await, "Pick what to make");
    assert_eq!(
        err(json!({ "text": "x", "kind": "note", "projectId": "nope" })).await,
        "Unknown project nope"
    );
    assert_eq!(
        err(json!({ "text": "x", "kind": "note", "projectId": 5 })).await,
        "Unknown project 5"
    );
    assert_eq!(
        err(json!({ "text": "x", "kind": "note" })).await,
        "Unknown project undefined"
    );
    assert!(s.handlers.calls().is_empty());
    assert_eq!(s.records(), 0);
}

/// CAP-126, CAP-130: a session in auto mode, expanded in a free spot, with
/// the typed words stuck on it as a note; remembered; the board told.
#[tokio::test]
async fn makes_a_session_with_the_words_stuck_on() {
    let s = Setup::new(&seeded());
    let receipt = s
        .submit(json!({
            "text": "  Refunds fail with a coupon. Fix it  ",
            "kind": "session",
            "projectId": "p",
            "attachments": [{ "kind": "file", "path": "/logs/a.log", "name": "a.log", "size": 3 }],
        }))
        .await
        .unwrap();
    let at = spot_on_board(&seeded_board(), Size { w: 720.0, h: 480.0 }, DEFAULT_WINDOW);
    assert_eq!(
        s.handlers.calls(),
        vec![Call::Create(
            "p".into(),
            CreateSessionOptions {
                x: at.x,
                y: at.y,
                kind: None,
                name: None,
                initial_prompt: Some(
                    "Refunds fail with a coupon. Fix it\n\nFrom my screen as I wrote this:\n\n- A file: /logs/a.log"
                        .into()
                ),
                parents: None,
                action_id: None,
                expanded: Some(true),
                auto_mode: Some(true),
                from_view: None,
                model: None,
                effort: None,
            }
        )]
    );
    assert_eq!(
        receipt,
        CaptureReceipt {
            kind: CaptureKind::Session,
            project_id: "p".into(),
            project_name: "store".into(),
            title: "Refunds fail with a coupon".into(),
            session_id: Some("new-session".into()),
            note_id: None,
            view_id: None,
        }
    );
    let notes: Vec<BoardNote> = s.board().notes.values().cloned().collect();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(
        (note.text.as_str(), note.color, note.x, note.y, note.w, note.h),
        (
            "Refunds fail with a coupon. Fix it",
            FolderColor::Yellow,
            at.x,
            at.y,
            None,
            None
        )
    );
    assert_eq!(
        (note.session_id.as_deref(), note.created_at),
        (Some("new-session"), NOW)
    );
    let json = nodus_domain::json::to_string(note).unwrap();
    assert!(json.ends_with(&format!(
        r#""color":"yellow","x":{},"y":{},"sessionId":"new-session","createdAt":1800000000000}}"#,
        at.x, at.y
    )));
    let records = s.store.read(|st| st.captures.clone().unwrap());
    assert_eq!(
        (
            records[0].kind,
            records[0].title.as_str(),
            records[0].session_id.as_deref(),
            records[0].created_at
        ),
        (
            CaptureKind::Session,
            "Refunds fail with a coupon",
            Some("new-session"),
            NOW
        )
    );
    assert_eq!(s.events.events(), vec![Event::BoardChanged { project_id: "p".into() }]);
}

fn seeded_board() -> BoardState {
    serde_json::from_value(seeded()["boards"]["p"].clone()).unwrap()
}

/// CAP-132: a failure part-way leaves nothing behind.
#[tokio::test]
async fn a_failure_leaves_nothing() {
    let mut s = Setup::new(&seeded());
    s.handlers.fail = Some("spawn claude ENOENT".into());
    for kind in ["session", "into", "view"] {
        let err = s
            .submit(json!({ "text": "x", "kind": kind, "projectId": "p", "sessionId": "s1" }))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "spawn claude ENOENT");
    }
    assert!(s.board().notes.is_empty());
    assert_eq!(s.records(), 0);
    assert!(s.events.events().is_empty());
}

/// CAP-127, CAP-163, CAP-164: into a session that passes the check, through
/// the board's send path, titled as its card is.
#[tokio::test]
async fn sends_into_a_session() {
    let mut s = Setup::new(&seeded());
    for missing in [json!(null), json!(""), json!("gone"), json!("nope"), json!(5)] {
        let err = s
            .submit(json!({ "text": "x", "kind": "into", "projectId": "p", "sessionId": missing }))
            .await
            .unwrap_err();
        assert_eq!(err, SubmitError::NotOnBoard);
    }
    assert!(s.handlers.calls().is_empty());
    let receipt = s
        .submit(json!({ "text": " also the coupon path ", "kind": "into", "projectId": "p", "sessionId": "s1", "attachments": "junk" }))
        .await
        .unwrap();
    assert_eq!(
        s.handlers.calls(),
        vec![Call::Send("p".into(), "s1".into(), "also the coupon path".into())]
    );
    assert_eq!(
        (receipt.title.as_str(), receipt.session_id.as_deref()),
        ("Title of s1", Some("s1"))
    );
    // The board's own refusal comes back as it is.
    s.handlers.fail = Some("It’s waiting for your answer. Open it to reply.".into());
    let err = s
        .submit(json!({ "text": "x", "kind": "into", "projectId": "p", "sessionId": "s1" }))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "It’s waiting for your answer. Open it to reply.");
    assert_eq!(s.records(), 1);
}

/// CAP-128: the whole prompt on a note sized to fit it.
#[tokio::test]
async fn puts_up_a_note() {
    let s = Setup::new(&seeded());
    let receipt = s
        .submit(json!({
            "text": "idea: a refunds dashboard",
            "kind": "note",
            "projectId": "p",
            "attachments": [{ "kind": "text", "text": "TypeError" }],
        }))
        .await
        .unwrap();
    let prompt = "idea: a refunds dashboard\n\nFrom my screen as I wrote this:\n\nText I copied:\n```\nTypeError\n```";
    let size = fit_note(prompt);
    let at = spot_on_board(&seeded_board(), size, DEFAULT_WINDOW);
    let note = s
        .board()
        .notes
        .get(receipt.note_id.as_deref().unwrap())
        .cloned()
        .unwrap();
    assert_eq!(
        (note.text.as_str(), note.x, note.y, note.w, note.h, note.session_id),
        (prompt, at.x, at.y, Some(size.w), Some(size.h), None)
    );
    assert_eq!(receipt.title, "idea: a refunds dashboard");
    assert!(s.handlers.calls().is_empty());
}

/// CAP-129: a view at a 480 × 360 spot, described by the prompt.
#[tokio::test]
async fn builds_a_view() {
    let mut s = Setup::new(&seeded());
    s.handlers.window = Some((1000.0, 800.0));
    let receipt = s
        .submit(json!({ "text": "A table of failed refunds", "kind": "view", "projectId": "p" }))
        .await
        .unwrap();
    let at = spot_on_board(&seeded_board(), Size { w: 480.0, h: 360.0 }, (1000.0, 800.0));
    assert_eq!(
        s.handlers.calls(),
        vec![Call::View("p".into(), "A table of failed refunds".into(), at)]
    );
    assert_eq!(
        (receipt.view_id.as_deref(), receipt.title.as_str()),
        (Some("new-view"), "A table of failed refunds")
    );
}

/// A project without a board gets an empty one (getBoard).
#[tokio::test]
async fn makes_a_board_for_a_project_without_one() {
    let s = Setup::new(&json!({ "projects": [project("p", "store", json!({}))] }));
    s.submit(json!({ "text": "note this", "kind": "note", "projectId": "p" }))
        .await
        .unwrap();
    assert_eq!(s.board().notes.len(), 1);
}

/// CAP-135
#[test]
fn cleans_attachments_from_the_page() {
    assert!(clean_attachments(&json!("junk")).is_empty());
    assert!(clean_attachments(&json!(null)).is_empty());
    let long = "x".repeat(5000);
    let raw = json!([
        null,
        7,
        { "kind": "tab", "url": "HTTPS://example.com", "title": long, "app": long },
        { "kind": "tab", "url": "file:///etc", "title": "t", "app": "a" },
        { "kind": "tab", "url": 5 },
        { "kind": "image", "path": "/a.png", "thumb": "data:image/png;base64,AAAA", "copied": true },
        { "kind": "image", "path": "/b.png", "copied": "yes" },
        { "kind": "image", "path": "" },
        { "kind": "file", "path": "/c.log", "name": 9, "size": "12" },
        { "kind": "file", "path": "/d.log", "name": "d.log", "size": "lots" },
        { "kind": "file", "name": "no path" },
        { "kind": "text", "text": "  keep my spaces  " },
        { "kind": "text", "text": " \n " },
        { "kind": "party" },
    ]);
    let cleaned = clean_attachments(&raw);
    assert_eq!(
        cleaned,
        vec![
            CaptureAttachment::Tab(TabAttachment {
                url: "HTTPS://example.com".into(),
                title: "x".repeat(500),
                app: "x".repeat(100),
            }),
            CaptureAttachment::Image(ImageAttachment {
                path: "/a.png".into(),
                thumb: String::new(),
                copied: Some(true),
            }),
            CaptureAttachment::Image(ImageAttachment {
                path: "/b.png".into(),
                thumb: String::new(),
                copied: Some(false),
            }),
            CaptureAttachment::File(FileAttachment {
                path: "/c.log".into(),
                name: String::new(),
                size: 12,
            }),
            CaptureAttachment::File(FileAttachment {
                path: "/d.log".into(),
                name: "d.log".into(),
                size: 0,
            }),
            CaptureAttachment::Text(TextAttachment {
                text: "  keep my spaces  ".into(),
            }),
        ]
    );
    // Only the first 20 are looked at.
    let many: Vec<Value> = (0..25)
        .map(|i| json!({ "kind": "file", "path": format!("/{i}") }))
        .collect();
    assert_eq!(clean_attachments(&Value::Array(many)).len(), 20);
    // Typed attachments go through the same rules.
    let typed = clean(&[CaptureAttachment::Text(TextAttachment { text: "  ".into() })]);
    assert!(typed.is_empty());
}

/// CAP-133: the middle of the board as last shown, else right of the
/// right-most card, else right of the dock; then a free spot, rounded.
#[test]
fn finds_a_spot_on_the_board() {
    let size = Size { w: 100.0, h: 50.0 };
    let mut b: BoardState = serde_json::from_value(board(&[], json!({}))).unwrap();
    let dock = dock_rect(b.archive);
    assert_eq!(
        spot_on_board(&b, size, DEFAULT_WINDOW),
        find_free_spot(
            Point {
                x: dock.x + dock.w + 80.0,
                y: dock.y
            },
            &[],
            Some(dock),
            size
        )
    );
    // Right of the right-most card; the earlier one on a tie; runs, brains
    // and archived cards don't count.
    b = serde_json::from_value(board(
        &[
            node("a", json!({ "x": 3000, "y": 10 })),
            node("b", json!({ "x": 3000, "y": 99 })),
            node("run", json!({ "x": 9000, "y": 0, "runOf": "x" })),
            node("old", json!({ "x": 9000, "y": 0, "archived": true })),
        ],
        json!({}),
    ))
    .unwrap();
    assert_eq!(
        spot_on_board(&b, size, DEFAULT_WINDOW),
        Point {
            x: 3000.0 + 288.0 + 32.0,
            y: 10.0
        }
    );
    // A saved view of the board, zoomed: its middle, rounded.
    b.viewport = Some(nodus_domain::Viewport {
        x: -5000.0,
        y: -100.3,
        zoom: 2.0,
    });
    assert_eq!(
        spot_on_board(&b, size, (1001.0, 800.0)),
        Point {
            x: js::round((500.5 + 5000.0) / 2.0 - 50.0),
            y: js::round((400.0 + 100.3) / 2.0 - 25.0)
        }
    );
    // A zoom of 0 isn't a view of it.
    b.viewport = Some(nodus_domain::Viewport {
        x: 0.0,
        y: 0.0,
        zoom: 0.0,
    });
    assert_eq!(spot_on_board(&b, size, DEFAULT_WINDOW).x, 3320.0);
}

/// The cards, free notes and views a new thing keeps clear of (nodus-board's `obstacles`).
#[test]
fn keeps_clear_of_cards_notes_and_views() {
    let b: BoardState = serde_json::from_value(board(
        &[
            node("a", json!({ "x": 1, "y": 2, "expanded": true })),
            node("brain", json!({ "viewOf": "v" })),
        ],
        json!({
            "notes": {
                "n": { "id": "n", "text": "", "color": "yellow", "x": 5, "y": 6, "w": 300, "createdAt": 0 },
                "stuck": { "id": "stuck", "text": "", "color": "yellow", "x": 5, "y": 6, "sessionId": "a", "createdAt": 0 },
            },
            "views": { "v": { "id": "v", "title": "V", "description": "", "x": 7, "y": 8, "version": 0, "createdAt": 0, "updatedAt": 0 } },
        }),
    ))
    .unwrap();
    assert_eq!(
        obstacles(&b, None),
        vec![
            Rect {
                x: 1.0,
                y: 2.0,
                w: 720.0,
                h: 480.0
            },
            Rect {
                x: 5.0,
                y: 6.0,
                w: 300.0,
                h: 216.0
            },
            Rect {
                x: 7.0,
                y: 8.0,
                w: 480.0,
                h: 360.0
            },
        ]
    );
}
