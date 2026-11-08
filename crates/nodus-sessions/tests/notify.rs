//! Notifications and the Dock badge: notify.test.ts ported, and the rest of
//! notify.ts with a fake window and notification centre.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nodus_domain::{Event, OpenCardRequest, RecordingSink, SessionMeta, SessionStatus};
use nodus_sessions::hooks::{StatusMachine, TokioClock};
use nodus_sessions::notify::{
    AppWindow, Notice, Notification, NotificationId, Notifier, Notify, SessionMetas, notice_for, waiting_count,
};
use nodus_store::{State, Store};
use serde_json::{Value, json};

use SessionStatus::{Done, Waiting, Working};

// --- noticeFor ---------------------------------------------------------------

#[test]
fn notice_for_tells_when_a_session_starts_needing_the_user() {
    assert_eq!(notice_for(Some(Working), Waiting, true), Some(Notice::Waiting));
    assert_eq!(notice_for(Some(Done), Waiting, true), Some(Notice::Waiting));
    assert_eq!(notice_for(None, Waiting, true), Some(Notice::Waiting));
}

#[test]
fn notice_for_tells_once_while_it_keeps_waiting() {
    assert_eq!(notice_for(Some(Waiting), Waiting, true), None);
}

#[test]
fn notice_for_tells_when_a_turn_ends() {
    assert_eq!(notice_for(Some(Working), Done, true), Some(Notice::Finished));
}

#[test]
fn notice_for_stays_quiet_when_a_process_goes_or_nothing_was_running() {
    assert_eq!(notice_for(Some(Working), Done, false), None);
    assert_eq!(notice_for(Some(Waiting), Done, true), None);
    assert_eq!(notice_for(None, Done, true), None);
    assert_eq!(notice_for(None, Working, true), None);
    assert_eq!(notice_for(Some(Waiting), Working, true), None);
}

// --- waitingCount ------------------------------------------------------------

fn card(id: &str, extra: &Value) -> Value {
    let mut node = json!({"sessionId": id, "x": 0, "y": 0, "archived": false, "createdAt": 0});
    for (key, value) in extra.as_object().unwrap() {
        node[key] = value.clone();
    }
    node
}

#[test]
fn waiting_count_counts_the_waiting_sessions_on_every_board_action_runs_too() {
    let state = State::from_json(json!({"boards": {
        "a": {"nodes": {
            "w1": card("w1", &json!({})),
            "w2": card("w2", &json!({})),
            "r1": card("r1", &json!({"runOf": "act"}))
        }, "archive": {"x": 0, "y": 0, "w": 0, "h": 0}},
        "b": {"nodes": {"w3": card("w3", &json!({}))}, "archive": {"x": 0, "y": 0, "w": 0, "h": 0}}
    }}))
    .unwrap();
    let statuses = HashMap::from([("w1", Waiting), ("w2", Working), ("r1", Waiting), ("w3", Waiting)]);
    let count = waiting_count(
        &state,
        |id| statuses.contains_key(id),
        |id| statuses.get(id).copied().unwrap_or(Done),
    );
    assert_eq!(count, 3);
}

#[test]
fn waiting_count_leaves_out_views_brains_archived_cards_and_sessions_that_arent_running() {
    let state = State::from_json(json!({"boards": {
        "a": {"nodes": {
            "v1": card("v1", &json!({"viewOf": "view"})),
            "x1": card("x1", &json!({"archived": true})),
            "d1": card("d1", &json!({}))
        }, "archive": {"x": 0, "y": 0, "w": 0, "h": 0}}
    }}))
    .unwrap();
    let statuses = HashMap::from([("v1", Waiting), ("x1", Waiting)]);
    let count = waiting_count(
        &state,
        |id| statuses.contains_key(id),
        |id| statuses.get(id).copied().unwrap_or(Done),
    );
    assert_eq!(count, 0);
}

// --- the notifier --------------------------------------------------------------

#[derive(Default)]
struct FakeCentre {
    unsupported: AtomicBool,
    no_dock: AtomicBool,
    shown: Mutex<Vec<(NotificationId, Notification)>>,
    closed: Mutex<Vec<NotificationId>>,
    badges: Mutex<Vec<String>>,
}

impl Notifier for FakeCentre {
    fn is_supported(&self) -> bool {
        !self.unsupported.load(Ordering::SeqCst)
    }
    fn show(&self, id: NotificationId, notification: &Notification) {
        self.shown.lock().unwrap().push((id, notification.clone()));
    }
    fn close(&self, id: NotificationId) {
        self.closed.lock().unwrap().push(id);
    }
    fn has_dock(&self) -> bool {
        !self.no_dock.load(Ordering::SeqCst)
    }
    fn set_badge(&self, text: &str) {
        self.badges.lock().unwrap().push(text.to_owned());
    }
}

#[derive(Default)]
struct FakeWindow {
    open: AtomicBool,
    focused: AtomicBool,
    loading: AtomicBool,
    shows: Mutex<u32>,
}

impl AppWindow for FakeWindow {
    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
    fn is_focused(&self) -> bool {
        self.focused.load(Ordering::SeqCst)
    }
    fn is_loading(&self) -> bool {
        self.loading.load(Ordering::SeqCst)
    }
    fn show(&self) {
        *self.shows.lock().unwrap() += 1;
    }
}

#[derive(Default)]
struct FakeMetas(Mutex<HashMap<String, SessionMeta>>);

impl SessionMetas for FakeMetas {
    fn session_meta(&self, repo_path: &str, session_id: &str) -> Option<SessionMeta> {
        assert_eq!(repo_path, "/repos/alpha");
        self.0.lock().unwrap().get(session_id).cloned()
    }
}

fn meta(session_id: &str, title: &str, last: Option<&str>) -> SessionMeta {
    SessionMeta {
        session_id: session_id.into(),
        title: title.into(),
        first_prompt: None,
        last_prompt: None,
        last_assistant_text: last.map(str::to_owned),
        git_branch: None,
        message_count: 2,
        created_at: 0.0,
        updated_at: 0.0,
        transcript_path: String::new(),
    }
}

struct World {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    status: StatusMachine,
    centre: Arc<FakeCentre>,
    window: Arc<FakeWindow>,
    metas: Arc<FakeMetas>,
    sink: Arc<RecordingSink>,
    notify: Notify,
}

impl World {
    /// Project `p1` (alpha) with cards s1 (titled), s2, a terminal, a view's
    /// brain and an archived card; project `p2` (beta) with b1. The window is
    /// open and in the background.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.update(|state| {
            *state = State::from_json(json!({
                "projects": [
                    {"id": "p1", "name": "alpha", "repoPath": "/repos/alpha", "color": "blue", "createdAt": 0},
                    {"id": "p2", "name": "beta", "repoPath": "/repos/alpha", "color": "red", "createdAt": 0}
                ],
                "boards": {
                    "p1": {"nodes": {
                        "s1": card("s1", &json!({"title": "Fix the tests"})),
                        "s2": card("s2", &json!({"title": ""})),
                        "t1": card("t1", &json!({"kind": "terminal"})),
                        "v1": card("v1", &json!({"viewOf": "view"})),
                        "x1": card("x1", &json!({"archived": true}))
                    }, "archive": {"x": 0, "y": 0, "w": 0, "h": 0}},
                    "p2": {"nodes": {"b1": card("b1", &json!({}))}, "archive": {"x": 0, "y": 0, "w": 0, "h": 0}}
                }
            }))
            .unwrap();
        });
        let status = StatusMachine::new(
            Arc::new(RecordingSink::default()),
            Arc::new(TokioClock::starting_at(0.0)),
            |_: &str| false,
        );
        let centre = Arc::new(FakeCentre::default());
        let window = Arc::new(FakeWindow::default());
        window.open.store(true, Ordering::SeqCst);
        let metas = Arc::new(FakeMetas::default());
        let sink = Arc::new(RecordingSink::default());
        let notify = Notify::new(
            store.clone(),
            status.clone(),
            centre.clone(),
            window.clone(),
            metas.clone(),
            sink.clone(),
        );
        Self {
            _dir: dir,
            store,
            status,
            centre,
            window,
            metas,
            sink,
            notify,
        }
    }

    /// Puts a session in a status in the status machine, and tells the notifier.
    fn set(&self, id: &str, status: SessionStatus) {
        if !self.status.is_live(id) {
            self.status.process_started(id, Done);
        }
        let event = match status {
            Waiting => "PermissionRequest",
            Working => "UserPromptSubmit",
            Done => "Stop",
        };
        self.status.receive_hook(event, id, &json!({}), None);
        self.notify.status_changed(id, status, true);
    }

    fn shown(&self) -> Vec<Notification> {
        self.centre
            .shown
            .lock()
            .unwrap()
            .iter()
            .map(|(_, n)| n.clone())
            .collect()
    }

    fn shown_ids(&self) -> Vec<NotificationId> {
        self.centre.shown.lock().unwrap().iter().map(|(id, _)| *id).collect()
    }

    fn closed(&self) -> Vec<NotificationId> {
        self.centre.closed.lock().unwrap().clone()
    }

    fn badges(&self) -> Vec<String> {
        self.centre.badges.lock().unwrap().clone()
    }
}

fn notification(title: &str, subtitle: &str, body: &str, group: &str) -> Notification {
    Notification {
        title: title.into(),
        subtitle: subtitle.into(),
        body: body.into(),
        group_id: group.into(),
    }
}

#[tokio::test(start_paused = true)]
async fn says_nothing_until_armed() {
    // HOST-206
    let w = World::new();
    w.set("s1", Waiting);
    assert!(w.shown().is_empty());
    assert!(w.badges().is_empty());
    w.notify.arm();
    assert_eq!(w.badges(), vec!["1"]);
}

#[tokio::test(start_paused = true)]
async fn tells_when_a_session_needs_the_user_and_takes_it_back_once_it_doesnt() {
    // HOST-202, HOST-204, HOST-208
    let w = World::new();
    w.notify.arm();
    w.set("s1", Working);
    w.set("s1", Waiting);
    assert_eq!(
        w.shown(),
        vec![notification("Fix the tests", "alpha", "Needs you", "p1")]
    );
    // Still waiting: told once.
    w.notify.status_changed("s1", Waiting, true);
    assert_eq!(w.shown().len(), 1);
    w.set("s1", Working);
    assert_eq!(w.closed(), w.shown_ids());
}

#[tokio::test(start_paused = true)]
async fn tells_when_a_turn_ends_with_the_last_reply() {
    // HOST-202, HOST-208
    let w = World::new();
    w.notify.arm();
    w.metas.0.lock().unwrap().insert(
        "s2".into(),
        meta("s2", "From the transcript", Some("  All\n\ndone \u{FEFF} here ")),
    );
    w.set("s2", Working);
    w.set("s2", Done);
    assert_eq!(
        w.shown(),
        vec![notification("From the transcript", "alpha", "All done here", "p1")]
    );
    // A finished notification stays when the session moves on.
    w.set("s2", Working);
    assert!(w.closed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn falls_back_to_finished_and_new_session() {
    // HOST-208
    let w = World::new();
    w.notify.arm();
    w.metas
        .0
        .lock()
        .unwrap()
        .insert("b1".into(), meta("b1", "", Some(" \n ")));
    w.set("b1", Working);
    w.set("b1", Done);
    w.store.update(|state| {
        state.board_mut("p1").nodes.get_mut("s2").unwrap().title = None;
    });
    w.set("s2", Working);
    w.set("s2", Done);
    assert_eq!(
        w.shown(),
        vec![
            notification("New session", "beta", "Finished", "p2"),
            notification("New session", "alpha", "Finished", "p1"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn says_nothing_about_a_process_that_went() {
    // HOST-202, HOST-203
    let w = World::new();
    w.notify.arm();
    w.set("s1", Working);
    w.notify.status_changed("s1", Done, false);
    assert!(w.shown().is_empty());
    // Its last status was forgotten, so working → waiting on a new process tells.
    w.notify.status_changed("s1", Waiting, true);
    assert_eq!(w.shown().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn only_tells_in_the_background_with_notifications_on() {
    // HOST-205
    let w = World::new();
    w.notify.arm();
    w.window.focused.store(true, Ordering::SeqCst);
    w.set("s1", Waiting);
    assert!(w.shown().is_empty());
    w.set("s1", Working);
    // A closed window is in the background, focused or not.
    w.window.open.store(false, Ordering::SeqCst);
    w.set("s1", Waiting);
    assert_eq!(w.shown().len(), 1);
    w.set("s1", Working);
    w.store.update(|state| state.settings.notify = false);
    w.set("s1", Waiting);
    assert_eq!(w.shown().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn only_cards_on_a_board_that_arent_brains_or_terminals_notify() {
    // HOST-207, HOST-210
    let w = World::new();
    w.notify.arm();
    for id in ["t1", "v1", "nowhere"] {
        w.set(id, Waiting);
    }
    assert!(w.shown().is_empty());
    w.centre.unsupported.store(true, Ordering::SeqCst);
    w.set("s1", Waiting);
    assert!(w.shown().is_empty());
}

#[tokio::test(start_paused = true)]
async fn shows_one_notification_per_session() {
    // HOST-209
    let w = World::new();
    w.notify.arm();
    w.set("s1", Working);
    w.set("s1", Done);
    w.set("s1", Waiting);
    let ids = w.shown_ids();
    assert_eq!(ids.len(), 2);
    assert_eq!(w.closed(), vec![ids[0]]);
}

#[tokio::test(start_paused = true)]
async fn a_click_opens_the_card() {
    // HOST-211, HOST-212
    let w = World::new();
    w.notify.arm();
    w.set("s1", Waiting);
    let id = w.shown_ids()[0];
    w.notify.clicked(id);
    assert_eq!(*w.window.shows.lock().unwrap(), 1);
    assert_eq!(
        w.sink.take(),
        vec![Event::OpenCard(OpenCardRequest {
            project_id: "p1".into(),
            session_id: Some("s1".into())
        })]
    );
    // Clicked once; it's gone.
    w.notify.clicked(id);
    assert!(w.sink.take().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_click_on_a_session_that_left_the_board_opens_the_board() {
    // HOST-211
    let w = World::new();
    w.notify.arm();
    w.set("s1", Working);
    w.set("s1", Done);
    w.store.update(|state| {
        state.board_mut("p1").nodes.shift_remove("s1");
    });
    w.notify.clicked(w.shown_ids()[0]);
    assert_eq!(
        w.sink.take(),
        vec![Event::OpenCard(OpenCardRequest {
            project_id: "p1".into(),
            session_id: None
        })]
    );
}

#[tokio::test(start_paused = true)]
async fn a_window_that_was_closed_or_is_loading_takes_the_request_itself() {
    // HOST-212, HOST-213
    let w = World::new();
    let request = OpenCardRequest {
        project_id: "p1".into(),
        session_id: Some("s1".into()),
    };
    w.window.open.store(false, Ordering::SeqCst);
    w.notify.open_card(request.clone());
    assert!(w.sink.events().is_empty());
    assert_eq!(*w.window.shows.lock().unwrap(), 1);
    assert_eq!(w.notify.take_open_request(), Some(request.clone()));
    assert_eq!(w.notify.take_open_request(), None);

    w.window.open.store(true, Ordering::SeqCst);
    w.window.loading.store(true, Ordering::SeqCst);
    w.notify.open_card(request.clone());
    assert_eq!(w.notify.take_open_request(), Some(request));
}

#[tokio::test(start_paused = true)]
async fn a_project_that_is_gone_only_brings_the_window_up() {
    // HOST-212
    let w = World::new();
    w.notify.open_card(OpenCardRequest {
        project_id: "gone".into(),
        session_id: Some("s1".into()),
    });
    assert_eq!(*w.window.shows.lock().unwrap(), 1);
    assert!(w.sink.events().is_empty());
    assert_eq!(w.notify.take_open_request(), None);
}

#[tokio::test(start_paused = true)]
async fn the_dock_badge_counts_waiting_sessions_when_its_on() {
    // HOST-214, HOST-215
    let w = World::new();
    w.notify.arm();
    w.set("s1", Waiting);
    w.set("b1", Waiting);
    w.set("v1", Waiting);
    // Archived cards don't count.
    w.set("x1", Waiting);
    w.set("s1", Working);
    assert_eq!(w.badges(), vec!["1", "2", "1"]);
    w.store.update(|state| state.settings.dock_badge = false);
    w.notify.update_badge();
    w.notify.update_badge();
    assert_eq!(w.badges(), vec!["1", "2", "1", ""]);
}

#[tokio::test(start_paused = true)]
async fn there_is_no_badge_without_a_dock() {
    // HOST-215: macOS only.
    let w = World::new();
    w.centre.no_dock.store(true, Ordering::SeqCst);
    w.notify.arm();
    w.set("s1", Waiting);
    assert!(w.badges().is_empty());
}
