//! The board service: loading and merging, the board:* handlers, runs, and
//! creating and sending to sessions (spec 03).

#![allow(clippy::needless_pass_by_value)]

mod common;

use common::*;
use nodus_board::BoardError;
use nodus_domain::ProjectAction;
use nodus_domain::{
    ActionRunResult, CreateSessionOptions, Event, InkColor, InkKind, InkStroke, NodePatch, Rect, SessionKind,
    SessionStatus, SummarizePhase, SummarizeProgress, Viewport,
};
use nodus_platform::runner::LocalRunner;
use serde_json::{Map, json};

fn action(id: &str, name: &str) -> ProjectAction {
    serde_json::from_value(json!({
        "id": id, "name": name, "icon": "zap", "description": "d", "prompt": format!("Run {name}"), "createdAt": 1
    }))
    .unwrap()
}

fn board_changed() -> Event {
    Event::BoardChanged { project_id: "p".into() }
}

#[tokio::test]
async fn loads_a_board_merged_with_the_sessions_on_disk() {
    let f = Fixture::new();
    f.talk(A, "Older", "Reply A", 10);
    f.talk(B, "Newer", "Reply B", 20);
    f.talk(C, "Hidden", "Reply C", 30);
    f.write(
        D,
        &[json!({ "type": "user", "timestamp": ts(40), "message": { "content": "No reply yet" } })],
    );
    f.talk(E, "Archived already", "Reply E", 5);
    f.put(node("gone", json!({})));
    f.put(node("app", json!({ "createdByApp": true, "sectionId": "s1" })));
    f.put(node("live", json!({})));
    f.put(node(
        E,
        json!({ "archived": true, "x": 40, "y": 64, "sectionId": "s1" }),
    ));
    f.put(node("sec", json!({ "createdByApp": true, "sectionId": "missing" })));
    f.sessions.set_live("live", SessionStatus::Working);
    f.store.update(|s| {
        let board = s.board_mut("p");
        board.hidden.push(C.into());
        for (id, at) in [("s1", 2.0), ("s0", 1.0)] {
            let section = json!({ "id": id, "name": "", "color": "red", "x": 0, "y": 0, "createdAt": at });
            board.sections.insert(id.into(), serde_json::from_value(section).unwrap());
        }
        for (id, sticker) in [("n1", "gone"), ("n2", "app")] {
            let note = json!({ "id": id, "text": "", "color": "yellow", "x": 1, "y": 2, "sessionId": sticker, "createdAt": 3 });
            board.notes.insert(id.into(), serde_json::from_value(note).unwrap());
        }
    });

    let snapshot = f.board.load("p").unwrap();

    let ids: Vec<&str> = snapshot.cards.iter().map(|c| c.node.session_id.as_str()).collect();
    // SESS-072, SESS-073, SESS-079: imported cards come after, newest first.
    assert_eq!(ids, ["app", "live", E, "sec", B, A]);
    // SESS-074: slot 0 is taken by E.
    let at = |id: &str| f.node(id).map(|n| (n.x, n.y, n.archived, n.created_at)).unwrap();
    assert_eq!(at(B), (352.0, 64.0, true, ms(20)));
    assert_eq!(at(A), (664.0, 64.0, true, ms(10)));
    // SESS-076
    assert_eq!(f.node("app").unwrap().section_id.as_deref(), Some("s1"));
    assert_eq!(f.node(E).unwrap().section_id, None);
    assert_eq!(f.node("sec").unwrap().section_id, None);
    // SESS-077
    let board = f.board_json();
    assert_eq!(board["notes"]["n1"].get("sessionId"), None);
    assert_eq!(board["notes"]["n2"]["sessionId"], "app");
    assert_eq!(
        (board["notes"]["n1"]["x"].as_f64(), board["notes"]["n1"]["y"].as_f64()),
        (Some(1.0), Some(2.0))
    );
    // SESS-078
    assert_eq!(snapshot.project.opened_at, Some(NOW));
    assert_eq!(f.store.read(|s| s.projects[0].opened_at), Some(NOW));
    // SESS-080: meta only with content; archived reads as done; live as the processes say.
    let card = |id: &str| snapshot.cards.iter().find(|c| c.node.session_id == id).unwrap();
    assert_eq!(card(A).meta.as_ref().map(|m| m.title.as_str()), Some("Older"));
    assert_eq!(card(A).status, SessionStatus::Done);
    assert!(card("app").meta.is_none());
    assert_eq!((card("live").status, card("live").live), (SessionStatus::Working, true));
    assert!(f.sessions.log().contains(&format!("agents:{A}:Some({:?})", ms(10))));
    // SESS-081
    let sections: Vec<&str> = snapshot.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(sections, ["s0", "s1"]);
    assert!(snapshot.ink.is_empty());
    assert_eq!(snapshot.viewport, None);
    assert_eq!(snapshot.archive.w, nodus_store::DEFAULT_ARCHIVE_W);
}

#[tokio::test]
async fn a_missing_transcript_folder_never_wipes_the_layout() {
    let f = Fixture::new();
    f.put(node("imported", json!({ "archived": true })));
    let snapshot = f.board.load("p").unwrap();
    assert_eq!(snapshot.cards.len(), 1);
    assert!(f.node("imported").is_some());
}

#[tokio::test]
async fn an_unknown_project_fails_to_load() {
    let f = Fixture::new();
    let err = f.board.load("nope").unwrap_err();
    assert_eq!(err.to_string(), "Unknown project nope");
}

#[tokio::test]
async fn finishes_runs_that_ended_while_the_app_was_closed() {
    let f = Fixture::new();
    f.store
        .update(|s| s.projects[0].actions = Some(vec![action("deploy", "Deploy")]));
    f.talk(A, "Run Deploy", "Deployed to staging.", 1);
    f.put(node(
        A,
        json!({ "runOf": "deploy", "title": "Old name", "createdByApp": true }),
    ));
    let snapshot = f.board.load("p").unwrap();
    assert!(snapshot.cards.is_empty());
    assert!(f.node(A).is_none());
    // SESS-155: hidden, so never imported again.
    assert_eq!(f.board_json()["hidden"], json!([A]));
    assert_eq!(
        f.events.take(),
        [
            Event::RunFinished(ActionRunResult {
                project_id: "p".into(),
                session_id: A.into(),
                action_name: "Deploy".into(),
                message: Some("Deployed to staging.".into()),
            }),
            board_changed(),
        ]
    );
    assert_eq!(f.sessions.log()[0], format!("stop+forget:{A}"));
}

#[tokio::test]
async fn names_a_finished_run_by_its_title_when_the_action_is_gone() {
    let f = Fixture::new();
    f.put(node("r1", json!({ "runOf": "gone", "title": "Nightly" })));
    f.put(node("r2", json!({ "runOf": "gone" })));
    // A run that's still running isn't finished on load.
    f.put(node("r3", json!({ "runOf": "gone" })));
    f.sessions.set_live("r3", SessionStatus::Working);
    f.board.load("p").unwrap();
    let names: Vec<String> = f
        .events
        .take()
        .into_iter()
        .filter_map(|e| match e {
            Event::RunFinished(r) => Some(r.action_name),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["Nightly", "Action"]);
    assert!(f.node("r3").is_some());
    // A session that isn't a run is left alone.
    f.board.finish_run("r3-missing");
    assert!(f.events.take().is_empty());
}

#[tokio::test]
async fn saves_node_patches() {
    let f = Fixture::new();
    f.talk(A, "Say hi", "Hi", 1);
    f.put(node(A, json!({ "sectionId": "s1", "expanded": true })));
    f.put(node("term", json!({ "kind": "terminal" })));
    f.put(node("brain", json!({ "viewOf": "v1" })));
    f.store.update(|s| {
        let section = json!({ "id": "s2", "name": "", "color": "red", "x": 0, "y": 0, "createdAt": 1 });
        s.board_mut("p")
            .sections
            .insert("s2".into(), serde_json::from_value(section).unwrap());
    });
    let patch = |v: serde_json::Value| -> NodePatch { serde_json::from_value(v).unwrap() };
    f.board
        .save_nodes(
            "p",
            &[
                // SESS-090
                patch(json!({ "sessionId": "nope", "x": 1 })),
                patch(json!({ "sessionId": A, "x": 10.5, "y": -3.25, "title": "  Mine  ", "w": 100, "h": 5000.4, "sectionId": "s2" })),
            ],
        )
        .unwrap();
    let a = f.node(A).unwrap();
    // SESS-092, SESS-093, SESS-095, SESS-097
    assert_eq!((a.x, a.y, a.title.as_deref()), (10.5, -3.25, Some("Mine")));
    assert_eq!(
        (a.w, a.h, a.section_id.as_deref()),
        (Some(200.0), Some(4000.0), Some("s2"))
    );
    f.board
        .save_nodes(
            "p",
            &[patch(
                json!({ "sessionId": A, "title": "   ", "sectionId": "unknown", "expanded": false, "w": 777.5 }),
            )],
        )
        .unwrap();
    let a = f.node(A).unwrap();
    // SESS-093, SESS-095 (unknown ignored), SESS-096, SESS-097
    assert_eq!(
        (a.title, a.section_id.as_deref(), a.expanded, a.w),
        (None, Some("s2"), None, Some(778.0))
    );
    f.board
        .save_nodes(
            "p",
            &[patch(json!({ "sessionId": A, "sectionId": null, "expanded": true }))],
        )
        .unwrap();
    let a = f.node(A).unwrap();
    assert_eq!((a.section_id, a.expanded), (None, Some(true)));

    // SESS-094, SESS-098: archiving stops the process, stamps the time and folds the card.
    f.board
        .save_nodes(
            "p",
            &[patch(json!({ "sessionId": A, "archived": true, "sectionId": "s2" }))],
        )
        .unwrap();
    let a = f.node(A).unwrap();
    assert_eq!(
        (a.archived, a.archived_at, a.section_id, a.expanded),
        (true, Some(NOW), None, None)
    );
    assert_eq!(f.sessions.log(), [format!("stop:{A}")]);
    // Un-archiving keeps the time.
    f.board
        .save_nodes("p", &[patch(json!({ "sessionId": A, "archived": false }))])
        .unwrap();
    assert_eq!(f.node(A).unwrap().archived_at, Some(NOW));
    // Terminals and brains aren't archived, but still move.
    f.board
        .save_nodes(
            "p",
            &[
                patch(json!({ "sessionId": "term", "archived": true, "x": 7 })),
                patch(json!({ "sessionId": "brain", "archived": false })),
            ],
        )
        .unwrap();
    assert_eq!(f.node("term").map(|n| (n.archived, n.x)), Some((false, 7.0)));
    // SESS-099: nothing dropped, nothing told.
    assert!(f.events.take().is_empty());
}

#[tokio::test]
async fn drops_an_empty_session_instead_of_archiving_it() {
    let f = Fixture::new();
    // A slash command only: nothing said.
    f.write(
        A,
        &[json!({ "type": "user", "message": { "content": "<command-name>/init</command-name>" } })],
    );
    f.put(node(A, json!({ "x": 30, "y": 40 })));
    // A brain with no conversation goes too: the empty check comes first.
    f.put(node("brain", json!({ "viewOf": "v1" })));
    f.store.update(|s| {
        let note = json!({ "id": "n", "text": "", "color": "yellow", "x": 1, "y": 2, "sessionId": A, "createdAt": 3 });
        s.board_mut("p")
            .notes
            .insert("n".into(), serde_json::from_value(note).unwrap());
    });
    let patches: Vec<NodePatch> = [A, "brain"]
        .iter()
        .map(|id| serde_json::from_value(json!({ "sessionId": id, "archived": true, "x": 999 })).unwrap())
        .collect();
    f.board.save_nodes("p", &patches).unwrap();
    // SESS-091: dropped where it was, before it moved.
    assert!(f.node(A).is_none() && f.node("brain").is_none());
    let board = f.board_json();
    assert_eq!(
        (board["notes"]["n"]["x"].as_f64(), board["notes"]["n"].get("sessionId")),
        (Some(30.0), None)
    );
    assert_eq!(board["hidden"], json!([]));
    assert_eq!(
        f.sessions.log(),
        [format!("stop+forget:{A}"), "stop+forget:brain".into()]
    );
    assert_eq!(f.events.take(), [board_changed()]);
}

#[tokio::test]
async fn saves_sections_and_removes_them() {
    let f = Fixture::new();
    // SESS-100: input without an id or a finite position is ignored.
    for bad in [
        json!(null),
        json!({ "x": 1, "y": 1 }),
        json!({ "id": "s", "x": "1", "y": 1 }),
        json!({ "id": 5, "x": 1, "y": 1 }),
    ] {
        f.board.save_section("p", &bad);
    }
    // Ignored before the board is even looked up.
    assert!(f.store.read(|s| s.board("p").is_none()));
    f.board.save_section(
        "p",
        &json!({ "id": "s", "name": "  Auth  ", "color": "green", "x": 1.5, "y": 2, "extra": 1 }),
    );
    f.board
        .save_section("p", &json!({ "id": "s", "name": 5, "color": "teal", "x": 3, "y": 4 }));
    let section = &f.board_json()["sections"]["s"];
    assert_eq!(
        *section,
        json!({ "id": "s", "name": "", "color": "blue", "x": 3.0, "y": 4.0, "createdAt": NOW })
    );
    f.put(node("in", json!({ "sectionId": "s", "x": 9 })));
    // SESS-101
    f.board.remove_section("p", "s");
    assert!(f.board_json()["sections"].as_object().unwrap().is_empty());
    assert_eq!(f.node("in").map(|n| (n.section_id, n.x)), Some((None, 9.0)));
}

#[tokio::test]
async fn saves_notes_as_the_window_has_them() {
    let f = Fixture::new();
    f.put(node(A, json!({})));
    f.store.update(|s| {
        let note = json!({
            "id": "n", "text": "", "color": "yellow", "x": 0, "y": 0, "from": A,
            "fromProject": "q", "fromTitle": "Other", "createdAt": 42
        });
        s.board_mut("p")
            .notes
            .insert("n".into(), serde_json::from_value(note).unwrap());
    });
    let long = format!("{}😀", "x".repeat(19_999));
    f.board.save_note(
        "p",
        &json!({
            "id": "n", "text": long, "color": "purple", "x": -2.5, "y": 2.5, "w": 50, "h": 9000,
            "sessionId": A, "from": "forged", "createdAt": 1
        }),
    );
    let note = &f.board_json()["notes"]["n"];
    let text = note["text"].as_str().unwrap();
    // SESS-103: 20 000 UTF-16 units, the emoji cut in half.
    assert_eq!(nodus_domain::js::utf16_len(text), 20_000);
    assert!(text.ends_with('\u{fffd}'));
    assert_eq!((note["x"].as_f64(), note["y"].as_f64()), (Some(-2.0), Some(3.0)));
    assert_eq!((note["w"].as_f64(), note["h"].as_f64()), (Some(120.0), Some(1200.0)));
    assert_eq!(
        (note["color"].as_str(), note["sessionId"].as_str()),
        (Some("purple"), Some(A))
    );
    assert_eq!(
        (note["from"].as_str(), note["createdAt"].as_f64()),
        (Some(A), Some(42.0))
    );
    // fromProject and fromTitle are lost, as in 0.9.4.
    assert_eq!((note.get("fromProject"), note.get("fromTitle")), (None, None));
    let keys: Vec<&String> = note.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "id",
            "text",
            "color",
            "x",
            "y",
            "w",
            "h",
            "sessionId",
            "from",
            "createdAt"
        ]
    );

    f.board.save_note(
        "p",
        &json!({ "id": "m", "text": 5, "color": "teal", "x": 1, "y": 1, "sessionId": "nope", "w": "9" }),
    );
    let note = &f.board_json()["notes"]["m"];
    assert_eq!(
        *note,
        json!({ "id": "m", "text": "", "color": "yellow", "x": 1.0, "y": 1.0, "createdAt": NOW })
    );
    f.board.save_note("p", &json!({ "id": "z", "x": f64::NAN, "y": 1 }));
    assert!(f.board_json()["notes"].get("z").is_none());

    // SESS-104
    f.board.remove_notes("p", &["n".into(), "missing".into()]);
    let ids: Vec<String> = f.board_json()["notes"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(ids, ["m"]);
}

#[tokio::test]
async fn keeps_ink_the_archive_and_the_viewport_as_given() {
    let f = Fixture::new();
    let stroke = InkStroke {
        id: "i".into(),
        kind: InkKind::Pen,
        color: InkColor::Red,
        points: vec![1.0, 2.0],
        size: 1.0,
        text: None,
        extra: Map::new(),
    };
    // SESS-105, SESS-106
    f.board.save_ink("p", vec![stroke.clone()]);
    let archive = Rect {
        x: -5.0,
        y: 3.0,
        w: 100.0,
        h: 50.0,
    };
    f.board.save_archive("p", archive);
    let viewport = Viewport {
        x: 1.0,
        y: 2.0,
        zoom: 0.5,
    };
    f.board.save_viewport("p", viewport);
    f.store.read(|s| {
        let board = s.board("p").unwrap();
        assert_eq!(
            (board.ink.as_deref(), board.archive, board.viewport),
            (Some(&[stroke][..]), archive, Some(viewport))
        );
    });
    // SESS-204: an unknown project gets a board record.
    f.board.save_viewport("ghost", viewport);
    assert!(f.store.read(|s| s.board("ghost").is_some()));
}

#[tokio::test]
async fn removes_a_card_and_hides_its_session_for_good() {
    let f = Fixture::new();
    f.talk(A, "Hello", "Hi", 1);
    f.put(node(A, json!({ "x": 30, "y": 40 })));
    f.put(node("term", json!({ "kind": "terminal" })));
    f.store.update(|s| {
        let note = json!({ "id": "n", "text": "", "color": "yellow", "x": 1, "y": 2, "sessionId": A, "createdAt": 3 });
        s.board_mut("p")
            .notes
            .insert("n".into(), serde_json::from_value(note).unwrap());
    });
    // SESS-107
    f.board.remove_node("p", A).await;
    f.board.remove_node("p", "term").await;
    f.board.remove_node("p", "never-there").await;
    f.board.remove_node("p", A).await;
    let board = f.board_json();
    assert_eq!(board["hidden"], json!([A, "never-there"]));
    assert_eq!(board["notes"]["n"]["x"].as_f64(), Some(30.0));
    assert_eq!(
        f.sessions.log()[..4],
        [
            format!("kill:{A}"),
            format!("forget:{A}"),
            "kill:term".into(),
            "forget:term".into()
        ]
    );
    // SESS-108: hidden sessions are never imported again.
    let snapshot = f.board.load("p").unwrap();
    assert!(snapshot.cards.is_empty());
    assert_eq!(f.board.project_stats(&snapshot.project).total, 0);
}

#[tokio::test]
async fn names_a_section_only_after_sessions_with_something_to_say() {
    let f = Fixture::new();
    f.write(B, &[json!({ "type": "system", "timestamp": ts(1) })]);
    f.put(node("term", json!({ "kind": "terminal", "title": "Shell" })));
    f.put(node(B, json!({})));
    let env = nodus_platform::Env::from_process();
    let ids = vec!["term".to_owned(), B.to_owned(), "missing".to_owned()];
    // SESS-102
    let err = f.board.name_section("p", &ids, &env, &LocalRunner).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "These sessions have nothing to name the section after yet"
    );
    // With a title, the next thing it needs is claude, which this environment hasn't found.
    f.put(node(A, json!({ "title": "Fix login" })));
    let err = f
        .board
        .name_section("p", &[A.to_owned()], &env, &LocalRunner)
        .await
        .unwrap_err();
    assert!(matches!(err, BoardError::NoClaude(_)), "{err}");
    let err = f
        .board
        .name_section("nope", &[A.to_owned()], &env, &LocalRunner)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Unknown project nope");
}

#[tokio::test]
async fn titles_sessions_as_their_cards_do() {
    let f = Fixture::new();
    f.talk(A, "Fix the login page", "Done", 1);
    f.write(B, &[json!({ "type": "system" })]);
    f.put(node(A, json!({})));
    f.put(node("custom", json!({ "title": "Mine" })));
    f.put(node("term", json!({ "kind": "terminal" })));
    f.put(node(B, json!({})));
    f.put(node("brain", json!({ "viewOf": "v1" })));
    f.put(node("orphan", json!({ "viewOf": "gone" })));
    f.store.update(|s| {
        let view = json!({ "id": "v1", "title": "Stats", "description": "", "x": 0, "y": 0, "version": 0, "createdAt": 0, "updatedAt": 0 });
        s.board_mut("p").views.insert("v1".into(), serde_json::from_value(view).unwrap());
    });
    let title = |id: &str| f.board.session_title("p", id).unwrap();
    // SESS-158
    assert_eq!(title(A), "Fix the login page");
    assert_eq!(title("custom"), "Mine");
    assert_eq!(title("term"), "Terminal");
    assert_eq!(title(B), "Untitled session");
    assert_eq!(title("brain"), "Stats");
    assert_eq!(title("orphan"), "A view");
    assert_eq!(title("not-on-board"), "New session");
    assert_eq!(
        f.board.session_title("nope", A).unwrap_err().to_string(),
        "Unknown project nope"
    );
}

#[tokio::test]
async fn lists_sessions_for_the_board_tools() {
    let f = Fixture::new();
    f.write(
        A,
        &[
            json!({ "type": "user", "timestamp": ts(1), "gitBranch": "main", "message": { "content": "Fix login" } }),
            json!({ "type": "assistant", "timestamp": ts(2), "message": { "content": [{ "type": "text", "text": "Fixed" }] } }),
        ],
    );
    f.put(node(A, json!({ "createdByApp": true, "sectionId": "s" })));
    f.put(node("self", json!({ "createdByApp": true })));
    f.put(node(
        "term",
        json!({ "kind": "terminal", "createdByApp": true, "createdAt": 1_700_000_000_000.9_f64 }),
    ));
    f.put(node("run", json!({ "runOf": "a", "createdByApp": true })));
    f.put(node("old", json!({ "archived": true, "createdByApp": true })));
    f.put(node("brain", json!({ "viewOf": "v", "createdByApp": true })));
    f.sessions.set_live(A, SessionStatus::Waiting);
    f.store.update(|s| {
        let section = json!({ "id": "s", "name": "", "color": "red", "x": 0, "y": 0, "createdAt": 1 });
        s.board_mut("p")
            .sections
            .insert("s".into(), serde_json::from_value(section).unwrap());
    });
    // SESS-159
    let list = f.board.session_list("p", Some("self"), false).unwrap();
    let json = nodus_domain::json::to_string(&list).unwrap();
    assert_eq!(
        json,
        format!(
            concat!(
                r#"[{{"id":"{a}","title":"Fix login","kind":"claude","status":"waiting for the user","running":true,"#,
                r#""section":"Untitled section","branch":"main","lastReply":"Fixed","lastActive":"2026-11-01T10:00:02.000Z"}},"#,
                r#"{{"id":"term","title":"Terminal","kind":"terminal","status":"done","running":false,"lastActive":"2023-11-14T22:13:20.000Z"}},"#,
                r#"{{"id":"run","title":"New session","kind":"claude","status":"done","running":false,"actionRun":true,"lastActive":"1970-01-01T00:00:00.005Z"}}]"#
            ),
            a = A
        )
    );
    let all = f.board.session_list("p", None, true).unwrap();
    let ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, [A, "self", "term", "run", "old"]);
    assert_eq!(all[4].status, "archived");
}

#[tokio::test]
async fn counts_a_projects_sessions() {
    let f = Fixture::new();
    f.talk(A, "On disk only", "Reply", 1);
    f.talk(B, "Hidden", "Reply", 2);
    f.put(node("work", json!({ "createdByApp": true })));
    f.put(node("wait", json!({ "createdByApp": true })));
    f.put(node("idle", json!({ "createdByApp": true })));
    f.put(node("old", json!({ "archived": true, "createdByApp": true })));
    f.put(node("term", json!({ "kind": "terminal", "createdByApp": true })));
    f.put(node("run", json!({ "runOf": "x", "createdByApp": true })));
    f.put(node("brain", json!({ "viewOf": "v", "createdByApp": true })));
    f.store.update(|s| s.board_mut("p").hidden.push(B.into()));
    f.sessions.set_live("work", SessionStatus::Working);
    f.sessions.set_live("wait", SessionStatus::Waiting);
    f.sessions.set_live("old", SessionStatus::Working);
    f.sessions.set_live("term", SessionStatus::Done);
    f.sessions.set_live("run", SessionStatus::Working);
    f.sessions.set_live("brain", SessionStatus::Working);
    let project = f.store.read(|s| s.projects[0].clone());
    // SESS-160
    let stats = f.board.project_stats(&project);
    assert_eq!(
        (stats.total, stats.active, stats.working, stats.waiting),
        (6 + 1, 5, 2, 1)
    );
    let running = stats.running;
    assert_eq!((running.sessions, running.background, running.terminals), (3, 2, 1));
    assert_eq!(f.board.all_project_stats()["p"], stats);
    // SESS-161
    assert!(f.board.has_transcript(&project, A));
    assert!(!f.board.has_transcript(&project, "work"));
}

#[tokio::test]
async fn claims_a_run_the_user_steps_into() {
    let f = Fixture::new();
    f.put(node("run", json!({ "runOf": "a" })));
    f.put(node("plain", json!({})));
    // SESS-140
    f.board.claim_run("plain");
    assert!(f.events.take().is_empty());
    f.board.claim_run("run");
    assert_eq!(f.node("run").unwrap().run_of, None);
    assert_eq!(f.events.take(), [board_changed()]);
}

#[tokio::test]
async fn archives_a_session_that_asked_once_its_turn_ends() {
    let f = Fixture::new();
    f.put(node(A, json!({ "x": 500, "sectionId": "s", "expanded": true })));
    // SESS-157: not asked, so nothing.
    f.board.on_done(A);
    assert!(!f.node(A).unwrap().archived);
    f.board.archive_when_done(A);
    f.board.on_done(A);
    let a = f.node(A).unwrap();
    assert_eq!((a.archived, a.x, a.y, a.archived_at), (true, 40.0, 64.0, Some(NOW)));
    assert_eq!(f.events.take(), [board_changed()]);
    assert_eq!(f.sessions.log(), [format!("stop:{A}")]);
    // Forgotten once done.
    f.board
        .save_nodes(
            "p",
            &[serde_json::from_value(json!({ "sessionId": A, "archived": false })).unwrap()],
        )
        .unwrap();
    f.board.on_done(A);
    assert!(!f.node(A).unwrap().archived);
}

fn opts(v: serde_json::Value) -> CreateSessionOptions {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn creates_a_session_card_before_its_process_starts() {
    let f = Fixture::new();
    f.store.update(|s| {
        let view = json!({ "id": "v1", "title": "Stats", "description": "", "x": 0, "y": 0, "version": 0, "createdAt": 0, "updatedAt": 0 });
        s.board_mut("p").views.insert("v1".into(), serde_json::from_value(view).unwrap());
    });
    let launcher = FakeLauncher::new(&f.store, None);
    let card = f
        .board
        .create_session(
            "p",
            &opts(json!({
                "x": 1.5, "y": 2, "name": "  Fix it  ", "initialPrompt": " go ", "parents": ["a", "b"],
                "expanded": true, "autoMode": true, "fromView": "v1"
            })),
            None,
            &launcher,
        )
        .await
        .unwrap();
    let id = card.node.session_id.clone();
    // SESS-114
    assert!(uuid_v4(&id), "{id}");
    // SESS-118
    assert_eq!(launcher.log.lock().unwrap()[0], format!("start:{id}:on-board=true"));
    let stored = f.board_json()["nodes"][&id].clone();
    // SESS-116
    assert_eq!(
        stored,
        json!({
            "sessionId": id, "x": 1.5, "y": 2.0, "archived": false, "title": "Fix it", "parents": ["a", "b"],
            "createdByApp": true, "expanded": true, "autoMode": true, "fromView": "v1", "createdAt": NOW,
            "h": 960.0
        })
    );
    // SESS-119: the launcher gets what the window sent.
    assert_eq!(
        launcher.starts.lock().unwrap()[0].initial_prompt.as_deref(),
        Some(" go ")
    );
    // SESS-120
    assert!(card.meta.is_none());

    // A terminal keeps its name, and nothing Claude-only.
    let card = f
        .board
        .create_session(
            "p",
            &opts(json!({ "x": 0, "y": 0, "kind": "terminal", "name": " zsh ", "parents": ["a"], "autoMode": true, "fromView": "v1" })),
            None,
            &launcher,
        )
        .await
        .unwrap();
    let node = &card.node;
    assert_eq!(
        (node.kind, node.title.as_deref()),
        (Some(SessionKind::Terminal), Some("zsh"))
    );
    assert_eq!((&node.parents, node.auto_mode, &node.from_view), (&None, None, &None));
    assert_eq!(node.h, Some(960.0));
    // An unknown view, and empty parents, are left out.
    let card = f
        .board
        .create_session(
            "p",
            &opts(json!({ "x": 0, "y": 0, "parents": [], "fromView": "nope", "name": "  " })),
            None,
            &launcher,
        )
        .await
        .unwrap();
    assert_eq!(
        (&card.node.parents, &card.node.from_view, &card.node.title),
        (&None, &None, &None)
    );
}

fn uuid_v4(id: &str) -> bool {
    id.len() == 36
        && id.chars().nth(14) == Some('4')
        && id
            .chars()
            .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[tokio::test]
async fn runs_an_action_by_its_name() {
    let f = Fixture::new();
    f.store.update(|s| {
        s.projects[0].actions = Some(vec![action("a1", "Own")]);
        s.actions = Some(vec![action("a1", "Global twin"), action("g1", "Global")]);
    });
    let launcher = FakeLauncher::new(&f.store, None);
    // SESS-115, SESS-117: the project's own action first, then the global ones.
    for (id, name) in [("a1", "Own"), ("g1", "Global")] {
        let card = f
            .board
            .create_session(
                "p",
                &opts(json!({ "x": 0, "y": 0, "actionId": id, "name": "ignored" })),
                None,
                &launcher,
            )
            .await
            .unwrap();
        assert_eq!(
            (card.node.run_of.as_deref(), card.node.title.as_deref()),
            (Some(id), Some(name))
        );
        assert_eq!(
            launcher
                .starts
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .action
                .as_ref()
                .unwrap()
                .name,
            name
        );
    }
    for o in [
        json!({ "x": 0, "y": 0, "actionId": "gone" }),
        json!({ "x": 0, "y": 0, "actionId": "a1", "kind": "terminal" }),
    ] {
        let err = f
            .board
            .create_session("p", &opts(o), None, &launcher)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "That action no longer exists");
    }
    assert_eq!(f.board_json()["nodes"].as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn takes_the_card_off_again_when_its_process_cant_start() {
    let f = Fixture::new();
    let launcher = FakeLauncher::new(&f.store, Some("The session host stopped"));
    let err = f
        .board
        .create_session("p", &opts(json!({ "x": 0, "y": 0 })), None, &launcher)
        .await
        .unwrap_err();
    // SESS-118
    assert_eq!(err.to_string(), "The session host stopped");
    assert!(f.board_json()["nodes"].as_object().unwrap().is_empty());
    let err = f
        .board
        .create_session("nope", &opts(json!({ "x": 0, "y": 0 })), None, &launcher)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Unknown project nope");
}

#[tokio::test]
async fn sends_a_prompt_into_a_session() {
    let f = Fixture::new();
    let launcher = FakeLauncher::new(&f.store, None);
    f.talk(A, "Hello", "Hi", 1);
    f.put(node(A, json!({ "runOf": "x" })));
    f.put(node(B, json!({ "autoMode": true })));
    f.put(node("old", json!({ "archived": true })));
    f.put(node("term", json!({ "kind": "terminal" })));
    // SESS-141: blank text does nothing, even for a session that isn't there.
    f.board
        .send_to_session("p", "nowhere", "  \n ", &launcher)
        .await
        .unwrap();
    // SESS-142
    for id in ["nowhere", "old", "term"] {
        let err = f.board.send_to_session("p", id, "hi", &launcher).await.unwrap_err();
        assert_eq!(err.to_string(), "That session is no longer on the board");
    }
    // SESS-143
    f.sessions.set_live(A, SessionStatus::Waiting);
    let err = f.board.send_to_session("p", A, "hi", &launcher).await.unwrap_err();
    assert_eq!(err.to_string(), "It’s waiting for your answer. Open it to reply.");
    // SESS-144: typed in, and the run becomes the user's.
    f.sessions.set_live(A, SessionStatus::Working);
    f.board
        .send_to_session("p", A, "  more please \n", &launcher)
        .await
        .unwrap();
    assert_eq!(
        launcher.log.lock().unwrap().as_slice(),
        [format!("type:{A}:more please")]
    );
    assert_eq!(f.node(A).unwrap().run_of, None);
    // SESS-145: a stopped session resumes with the prompt.
    f.board.send_to_session("p", B, " go on ", &launcher).await.unwrap();
    let resume = launcher.resumes.lock().unwrap()[0].clone();
    assert_eq!((resume.prompt.as_str(), resume.has_transcript), ("go on", false));
    assert_eq!(resume.node.auto_mode, Some(true));
    let failing = FakeLauncher::new(&f.store, Some("Claude Code CLI not found."));
    let err = f.board.send_to_session("p", B, "again", &failing).await.unwrap_err();
    assert_eq!(err.to_string(), "Claude Code CLI not found.");
}

#[tokio::test]
async fn searches_the_sessions_drawn_on_the_board() {
    let f = Fixture::new();
    f.talk(A, "Fix the websocket", "Done", 1);
    f.talk(B, "Websocket for the run", "Done", 2);
    f.talk(C, "Websocket in a shell", "Done", 3);
    f.put(node(A, json!({ "archived": true })));
    f.put(node(B, json!({ "runOf": "x" })));
    f.put(node(C, json!({ "kind": "terminal" })));
    f.put(node(D, json!({ "title": "Websocket notes" })));
    // SESS-175
    let hits = f.board.search("p", "websocket").await.unwrap();
    let ids: Vec<&str> = hits.iter().map(|h| h.session_id.as_str()).collect();
    assert_eq!(ids, [A, D]);
    // The transcript's title stands in for a card without one.
    let hits = f.board.search("p", "fix the websocket").await.unwrap();
    assert_eq!(hits[0].session_id, A);
    assert_eq!(
        f.board.search("nope", "x").await.unwrap_err().to_string(),
        "Unknown project nope"
    );
}

#[tokio::test]
async fn summarizes_only_sessions_with_a_conversation() {
    let f = Fixture::new();
    f.talk(A, "Fix login", "Done", 1);
    f.put(node(A, json!({})));
    f.put(node("new", json!({ "title": "Fresh" })));
    let env = nodus_platform::Env::from_process();
    let ids = vec!["new".to_owned(), "missing".to_owned(), A.to_owned()];
    // SESS-165, SESS-166, then SESS-174: no claude fails the whole call.
    let err = f
        .board
        .summarize("p", &ids, "r1", None, &env, &LocalRunner)
        .await
        .unwrap_err();
    assert!(matches!(err, BoardError::NoClaude(_)));
    let error = Some("This session has no conversation yet".to_owned());
    let progress = |id: &str| {
        Event::SummarizeProgress(SummarizeProgress {
            request_id: "r1".into(),
            session_id: id.into(),
            phase: SummarizePhase::Error,
            error: error.clone(),
        })
    };
    assert_eq!(f.events.take(), [progress("new"), progress("missing")]);
}

#[tokio::test]
async fn reads_transcripts_for_the_preview_and_the_agent_viewer() {
    let f = Fixture::new();
    f.talk(A, "Hello", "Hi", 1);
    // SESS-162
    let messages = f.board.transcript("p", A).unwrap();
    assert_eq!(messages.len(), 2);
    assert!(f.board.transcript("p", B).unwrap().is_empty());
    // SESS-163: ids that could leave the folder aren't read.
    for (session, agent) in [(A, "../x"), ("../x", "a1"), (A, "")] {
        let err = f.board.agent_transcript("p", session, agent).unwrap_err();
        assert_eq!(err.to_string(), "Not an agent of this session");
    }
    let agents = f.dir().join(A).join("subagents");
    std::fs::create_dir_all(&agents).unwrap();
    let line = json!({ "type": "user", "isSidechain": true, "message": { "content": "Look around" } });
    std::fs::write(agents.join("agent-a1.jsonl"), format!("{line}\n")).unwrap();
    let steps = f.board.agent_transcript("p", A, "a1").unwrap();
    assert_eq!(steps[0].text, "Look around");
    assert_eq!(
        f.board.transcript("nope", A).unwrap_err().to_string(),
        "Unknown project nope"
    );
}
