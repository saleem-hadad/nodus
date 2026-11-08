//! Targets, recents and the default project, over synthetic state.

use std::collections::HashMap;

use nodus_domain::CaptureReceipt;
use serde_json::{Value, json};

use super::*;

/// Sessions as the test says they are.
#[derive(Default)]
pub(crate) struct FakeSessions {
    pub live: HashSet<String>,
    pub status: HashMap<String, SessionStatus>,
    /// Transcript title and last activity, by session id.
    pub meta: HashMap<String, (String, f64)>,
}

impl CaptureSessions for FakeSessions {
    fn is_live(&self, session_id: &str) -> bool {
        self.live.contains(session_id)
    }

    fn status(&self, session_id: &str) -> SessionStatus {
        self.status.get(session_id).copied().unwrap_or(SessionStatus::Done)
    }

    fn meta(&self, repo_path: &str, session_id: &str) -> Option<SessionMeta> {
        let (title, updated_at) = self.meta.get(session_id)?;
        Some(SessionMeta {
            session_id: session_id.into(),
            title: title.clone(),
            first_prompt: None,
            last_prompt: None,
            last_assistant_text: None,
            git_branch: None,
            message_count: 1,
            created_at: 0.0,
            updated_at: *updated_at,
            transcript_path: format!("{repo_path}/{session_id}.jsonl"),
        })
    }
}

fn into_map(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

pub(crate) fn project(id: &str, name: &str, extra: Value) -> Value {
    let mut p =
        json!({ "id": id, "name": name, "repoPath": format!("~/code/{name}"), "color": "blue", "createdAt": 1 });
    p.as_object_mut().unwrap().extend(into_map(extra));
    p
}

pub(crate) fn node(id: &str, extra: Value) -> Value {
    let mut n = json!({ "sessionId": id, "x": 0, "y": 0, "archived": false, "createdAt": 100 });
    n.as_object_mut().unwrap().extend(into_map(extra));
    n
}

pub(crate) fn board(nodes: &[Value], extra: Value) -> Value {
    let mut b = json!({
        "nodes": nodes.iter().map(|n| (n["sessionId"].as_str().unwrap().to_owned(), n.clone())).collect::<serde_json::Map<_, _>>(),
        "archive": { "x": 0, "y": 0, "w": 1928, "h": 320 },
    });
    b.as_object_mut().unwrap().extend(into_map(extra));
    b
}

pub(crate) fn state(raw: Value) -> State {
    State::from_json(raw).unwrap()
}

fn record(kind: &str, project: &str, title: &str, at: f64, id_field: Option<(&str, &str)>) -> Value {
    let mut r =
        json!({ "id": format!("r-{title}"), "kind": kind, "projectId": project, "title": title, "createdAt": at });
    if let Some((field, id)) = id_field {
        r[field] = json!(id);
    }
    r
}

/// CAP-148: what passes the session check, and the titles it goes by.
#[test]
fn checks_sessions_and_names_them() {
    let s = state(json!({
        "projects": [project("p", "store", json!({}))],
        "boards": { "p": board(&[
            node("named", json!({ "title": "Mine" })),
            node("meta", json!({ "title": "" })),
            node("captured", json!({})),
            node("bare", json!({})),
            node("archived", json!({ "archived": true })),
            node("terminal", json!({ "kind": "terminal" })),
            node("run", json!({ "runOf": "a1" })),
            node("brain", json!({ "viewOf": "v1" })),
            node("emptyrun", json!({ "runOf": "" })),
        ], json!({})) },
        "captures": [
            record("note", "p", "Not a session", 1.0, Some(("sessionId", "captured"))),
            record("session", "p", "From the panel", 1.0, Some(("sessionId", "captured"))),
        ],
    }));
    let mut sessions = FakeSessions::default();
    sessions
        .meta
        .insert("meta".into(), ("From the transcript".into(), 500.0));
    sessions.meta.insert("named".into(), ("Ignored".into(), 700.0));
    sessions.live.insert("named".into());
    sessions.status.insert("named".into(), SessionStatus::Working);
    let info = |id: &str| session_info(&s, "p", id, &sessions);

    let named = info("named").unwrap();
    assert_eq!(
        (named.title.as_str(), named.status, named.live, named.updated_at),
        ("Mine", SessionStatus::Working, true, 700.0)
    );
    assert_eq!(info("meta").unwrap().title, "From the transcript");
    assert_eq!(info("captured").unwrap().title, "From the panel");
    let bare = info("bare").unwrap();
    assert_eq!(
        (bare.title.as_str(), bare.updated_at, bare.live),
        ("Untitled session", 100.0, false)
    );
    // An empty runOf isn't a run.
    assert!(info("emptyrun").is_some());
    for gone in ["archived", "terminal", "run", "brain", "missing"] {
        assert_eq!(info(gone), None, "{gone}");
    }
    assert_eq!(session_info(&s, "nope", "named", &sessions), None);
}

/// CAP-149: projects in order without the archived ones, sessions by last
/// activity (stable), recents, default.
#[test]
fn lists_the_targets() {
    let s = state(json!({
        "projects": [
            project("p1", "one", json!({ "color": "green" })),
            project("old", "old", json!({ "archived": true })),
            project("p2", "two", json!({})),
            project("p3", "three", json!({})),
        ],
        "boards": {
            "p1": board(&[node("a", json!({ "createdAt": 5 })), node("b", json!({ "createdAt": 9 }))], json!({})),
            "old": board(&[node("z", json!({ "createdAt": 99 }))], json!({})),
            "p2": board(&[node("c", json!({ "createdAt": 5 }))], json!({})),
        },
    }));
    let sessions = FakeSessions::default();
    let t = targets(&s, None, &sessions);
    assert_eq!(
        t.projects,
        vec![
            CaptureProject {
                id: "p1".into(),
                name: "one".into(),
                color: nodus_domain::FolderColor::Green
            },
            CaptureProject {
                id: "p2".into(),
                name: "two".into(),
                color: nodus_domain::FolderColor::Blue
            },
            CaptureProject {
                id: "p3".into(),
                name: "three".into(),
                color: nodus_domain::FolderColor::Blue
            },
        ]
    );
    let ids: Vec<&str> = t.sessions.iter().map(|s| s.session_id.as_str()).collect();
    assert_eq!(ids, ["b", "a", "c"]);
    assert_eq!(t.default_project_id.as_deref(), Some("p1"));
    assert_eq!(missing_boards(&s), ["p3"]);
    // The JSON the window reads.
    let json = nodus_domain::json::to_string(&t).unwrap();
    assert!(json.starts_with(r#"{"projects":[{"id":"p1","name":"one","color":"green"}"#));
    assert!(json.ends_with(r#""recents":[],"defaultProjectId":"p1"}"#));
}

/// CAP-150, CAP-169: what each record resolves to, if anything.
#[test]
fn resolves_records_to_recents() {
    let s = state(json!({
        "projects": [project("p", "store", json!({})), project("old", "old", json!({ "archived": true }))],
        "boards": {
            "p": board(&[node("s1", json!({}))], json!({
                "notes": { "n1": { "id": "n1", "text": "x", "color": "yellow", "x": 0, "y": 0, "createdAt": 1 } },
                "views": {
                    "v1": { "id": "v1", "title": "New view", "description": "d", "x": 0, "y": 0, "version": 0, "createdAt": 1, "updatedAt": 1 },
                    "v2": { "id": "v2", "title": "Orders", "description": "d", "x": 0, "y": 0, "version": 0, "createdAt": 1, "updatedAt": 1 },
                },
            })),
            "old": board(&[node("s2", json!({}))], json!({})),
            "noproject": board(&[], json!({})),
        },
        "captures": [
            record("session", "p", "Typed", 10.0, Some(("sessionId", "s1"))),
            record("into", "p", "Into it", 9.0, Some(("sessionId", "s1"))),
            record("session", "p", "Gone", 8.0, Some(("sessionId", "gone"))),
            record("note", "p", "A note", 7.0, Some(("noteId", "n1"))),
            record("note", "p", "Removed note", 6.0, Some(("noteId", "n9"))),
            record("view", "p", "Asked for a view", 5.0, Some(("viewId", "v1"))),
            record("view", "p", "Asked for orders", 4.0, Some(("viewId", "v2"))),
            record("session", "old", "Archived project", 3.0, Some(("sessionId", "s2"))),
            record("note", "noproject", "Orphan", 2.0, Some(("noteId", "n1"))),
            record("note", "nowhere", "No board", 1.0, Some(("noteId", "n1"))),
            record("session", "p", "No id", 0.0, None),
        ],
    }));
    let mut sessions = FakeSessions::default();
    sessions.meta.insert("s1".into(), ("Claude's title".into(), 50.0));
    sessions.status.insert("s1".into(), SessionStatus::Waiting);
    let recents = targets(&s, None, &sessions).recents;
    let seen: Vec<(&str, &str)> = recents.iter().map(|r| (r.id.as_str(), r.title.as_str())).collect();
    assert_eq!(
        seen,
        [
            ("r-Typed", "Claude's title"),
            ("r-Into it", "Claude's title"),
            ("r-A note", "A note"),
            ("r-Asked for a view", "Asked for a view"),
            ("r-Asked for orders", "Orders"),
            // Untitled, so it goes by what was captured.
            ("r-Archived project", "Archived project"),
        ]
    );
    assert_eq!(recents[0].status, Some(SessionStatus::Waiting));
    assert_eq!(recents[0].session_id.as_deref(), Some("s1"));
    assert_eq!((recents[2].note_id.as_deref(), recents[2].status), (Some("n1"), None));
    assert_eq!(recents[4].view_id.as_deref(), Some("v2"));
    // Records that don't resolve stay in the file (CAP-156).
    assert_eq!(s.captures.as_ref().unwrap().len(), 11);
}

fn default_of(raw: Value, last_board: Option<&str>) -> Option<String> {
    let s = state(raw);
    let ids: HashSet<&str> = s
        .projects
        .iter()
        .filter(|p| p.archived != Some(true))
        .map(|p| p.id.as_str())
        .collect();
    default_project(&s, &ids, last_board)
}

/// CAP-151
#[test]
fn picks_the_default_project() {
    let projects = json!([
        project("a", "a", json!({ "openedAt": 100 })),
        project("b", "b", json!({ "openedAt": 300 })),
        project("c", "c", json!({ "openedAt": 300 })),
        project("x", "x", json!({ "archived": true, "openedAt": 900 })),
    ]);
    // Opened most recently; ties keep state.json's order.
    assert_eq!(default_of(json!({ "projects": projects }), None).as_deref(), Some("b"));
    // A capture after the last opening wins.
    let captured = |at: f64, project: &str| json!({ "projects": projects, "captures": [record("note", "x", "archived", 999.0, None), record("note", project, "t", at, None)] });
    assert_eq!(default_of(captured(301.0, "a"), None).as_deref(), Some("a"));
    // One before it doesn't; then the board opened this run, unless archived.
    assert_eq!(default_of(captured(300.0, "a"), None).as_deref(), Some("b"));
    assert_eq!(default_of(captured(300.0, "a"), Some("c")).as_deref(), Some("c"));
    assert_eq!(default_of(captured(300.0, "a"), Some("x")).as_deref(), Some("b"));
    // With no openedAt anywhere, the capture's project, else the first project.
    let plain = json!([project("a", "a", json!({})), project("b", "b", json!({}))]);
    assert_eq!(
        default_of(
            json!({ "projects": plain, "captures": [record("note", "b", "t", 1.0, None)] }),
            None
        )
        .as_deref(),
        Some("b")
    );
    assert_eq!(default_of(json!({ "projects": plain }), None).as_deref(), Some("a"));
    assert_eq!(
        default_of(json!({ "projects": plain }), Some("b")).as_deref(),
        Some("b")
    );
    assert_eq!(default_of(json!({}), Some("b")), None);
}

/// CAP-155: newest first, 20 at most, written in 0.9.4's field order.
#[test]
fn remembers_captures() {
    let mut s = state(json!({}));
    for i in 0..22 {
        let receipt = CaptureReceipt {
            kind: CaptureKind::Note,
            project_id: "p".into(),
            project_name: "store".into(),
            title: format!("t{i}"),
            session_id: None,
            note_id: Some(format!("n{i}")),
            view_id: None,
        };
        remember(&mut s, &receipt, format!("id{i}"), 1_800_000_000_000.0 + f64::from(i));
    }
    let records = s.captures.as_ref().unwrap();
    assert_eq!(records.len(), 20);
    assert_eq!(records[0].title, "t21");
    assert_eq!(records[19].title, "t2");
    assert_eq!(
        nodus_domain::json::to_string(&records[0]).unwrap(),
        r#"{"id":"id21","kind":"note","projectId":"p","title":"t21","createdAt":1800000000021,"noteId":"n21"}"#
    );
}
