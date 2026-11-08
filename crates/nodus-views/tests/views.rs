//! views.test.ts, ported: each test a project whose folder is the data
//! folder, with card s1 at (2000, 0).

mod common;

use std::fs;

use common::{Fixture, HTML, P, node};
use nodus_board::geometry::{CARD_W, GAP};
use nodus_domain::{BoardView, ViewEvent, ViewPatch, ViewReport};
use nodus_views::{normalize_commands, normalize_refresh};
use serde_json::{Value, json};

fn remembered(allowed: &[&str]) -> BoardView {
    serde_json::from_value(json!({
        "id": "v", "title": "V", "description": "V", "x": 0, "y": 0, "version": 1, "createdAt": 0, "updatedAt": 0,
        "allowedRuns": allowed
    }))
    .unwrap()
}

fn err<T: std::fmt::Debug>(result: Result<T, nodus_views::ViewError>) -> String {
    result.unwrap_err().to_string()
}

async fn create(f: &Fixture, args: Value) -> Value {
    f.views.create_view_from_tool(P, "s1", &args).await.unwrap()
}

async fn create_id(f: &Fixture, args: Value) -> String {
    create(f, args).await["view_id"].as_str().unwrap().to_owned()
}

#[test]
fn normalize_commands_keeps_a_command_line_the_user_allowed_allowed_whatever_it_is_called_now() {
    let cmds = normalize_commands(
        &json!([{ "name": "list", "run": "vercel ls --json", "every": 10 }]),
        Some(&remembered(&["vercel ls --json"])),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&cmds[0]).unwrap(),
        json!({ "name": "list", "run": "vercel ls --json", "every": 10.0, "approved": "vercel ls --json" })
    );
}

#[test]
fn normalize_commands_asks_again_for_a_command_line_that_changed() {
    let cmds = normalize_commands(
        &json!([{ "name": "deploys", "run": "vercel ls; rm -rf ~" }]),
        Some(&remembered(&["vercel ls"])),
    )
    .unwrap();
    assert_eq!(cmds[0].approved, None);
}

#[test]
fn normalize_commands_runs_no_more_than_every_minute_nor_less_than_weekly() {
    let cmds = normalize_commands(
        &json!([
            { "name": "a", "run": "a", "every": 0.2 },
            { "name": "b", "run": "b", "every": 99999 },
            { "name": "c", "run": "c", "every": -5 }
        ]),
        None,
    )
    .unwrap();
    let every: Vec<Option<f64>> = cmds.iter().map(|c| c.every).collect();
    assert_eq!(every, [Some(1.0), Some(7.0 * 24.0 * 60.0), None]);
}

#[test]
fn normalize_commands_turns_down_commands_it_cant_use_saying_why() {
    assert_eq!(
        err(normalize_commands(&json!("ls"), None)),
        "commands must be a list of { name, run, every }"
    );
    assert_eq!(
        err(normalize_commands(&json!([{ "name": "x" }]), None)),
        "Each command needs a name and a run (its command line)"
    );
    assert_eq!(
        err(normalize_commands(
            &json!([{ "name": "x", "run": "a" }, { "name": "x", "run": "b" }]),
            None
        )),
        "There are two commands called “x”"
    );
    let many: Vec<Value> = (0..13)
        .map(|i| json!({ "name": format!("c{i}"), "run": "x" }))
        .collect();
    assert_eq!(
        err(normalize_commands(&Value::Array(many), None)),
        "A view can have at most 12 commands"
    );
}

#[test]
fn normalize_refresh_asks_the_brain_at_most_every_ten_minutes() {
    let refresh = normalize_refresh(Some(&json!({ "every": 1, "prompt": " Fetch my tickets " })))
        .unwrap()
        .unwrap();
    assert_eq!((refresh.every, refresh.prompt.as_str()), (10.0, "Fetch my tickets"));
    assert_eq!(normalize_refresh(Some(&Value::Null)).unwrap(), None);
    assert_eq!(normalize_refresh(None).unwrap(), None);
    assert_eq!(
        err(normalize_refresh(Some(&json!({ "every": 60 })))),
        "refresh needs every (minutes) and a prompt, or null to stop refreshing"
    );
}

#[tokio::test]
async fn create_view_puts_a_view_up_beside_the_card_of_the_session_that_made_it() {
    let f = Fixture::new();
    let reply = create(&f, json!({ "title": "Deploys", "html": HTML, "data": { "rows": [] } })).await;
    let view = f.view(reply["view_id"].as_str().unwrap()).unwrap();
    assert_eq!((view.title.as_str(), view.description.as_str()), ("Deploys", "Deploys"));
    assert_eq!((view.from.as_deref(), view.version), (Some("s1"), 1));
    assert_eq!((view.x, view.y), (2000.0 + CARD_W + GAP, 0.0));
    assert_eq!(reply["waiting_for_approval"], json!([]));
    assert_eq!(reply["errors"], json!([]));
    assert_eq!(
        fs::read_to_string(f.views_dir(&view.id).join("index.html")).unwrap(),
        HTML
    );
    // VIEW-107: no brain until it needs one.
    assert!(f.brains.spawns().is_empty() && view.session_id.is_none());
}

#[tokio::test]
async fn create_view_reports_the_errors_the_page_hit_as_it_loaded() {
    let f = Fixture::new();
    let views = f.views.clone();
    let store = std::sync::Arc::clone(&f.store);
    // The page fails while the tool waits on it.
    let failing = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let id = store.read(|s| s.board(P).unwrap().views.keys().next().cloned().unwrap());
        views.report(
            P,
            &id,
            &ViewReport {
                error: Some("TypeError: rows is undefined".into()),
                loaded: None,
            },
        );
    });
    let reply = create(&f, json!({ "title": "Broken", "html": HTML })).await;
    failing.await.unwrap();
    assert_eq!(reply["errors"], json!(["TypeError: rows is undefined"]));
}

#[tokio::test]
async fn update_view_sends_new_data_to_the_page_in_place_without_reloading_it() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Users", "html": HTML })).await;
    let reply = f
        .views
        .update_view_from_tool(P, "s1", &json!({ "view_id": id, "data": { "users": 42 } }))
        .await
        .unwrap();
    assert_eq!(
        reply,
        json!({ "view_id": id, "version": 1, "waiting_for_approval": [] })
    );
    assert!(
        f.window
            .view_events()
            .contains(&(id.clone(), ViewEvent::Data, json!({ "users": 42 })))
    );
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["data"],
        json!({ "users": 42 })
    );
}

#[tokio::test]
async fn update_view_reloads_the_page_when_its_html_changes() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Users", "html": HTML })).await;
    let reply = f
        .views
        .update_view_from_tool(P, "s1", &json!({ "view_id": id, "html": "<p>v2</p>" }))
        .await
        .unwrap();
    assert_eq!(reply["version"], 2);
    assert_eq!(reply["errors"], json!([]));
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["html"],
        "<p>v2</p>"
    );
}

#[tokio::test]
async fn update_view_lets_a_brain_leave_out_its_own_views_id_and_nobody_else() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Mine", "html": HTML })).await;
    let mut brain = node("brain", 0.0);
    brain.view_of = Some(id.clone());
    f.add_node(brain);
    f.views
        .update_view_from_tool(P, "brain", &json!({ "title": "Renamed" }))
        .await
        .unwrap();
    assert_eq!(f.view(&id).unwrap().title, "Renamed");
    assert_eq!(
        err(f
            .views
            .update_view_from_tool(P, "s1", &json!({ "title": "Nope" }))
            .await),
        "view_id is required: this session isn’t a view’s brain"
    );
    assert_eq!(
        err(f
            .views
            .update_view_from_tool(P, "s1", &json!({ "view_id": "__proto__", "title": "x" }))
            .await),
        "There’s no view with that id on this board"
    );
}

#[tokio::test]
async fn create_view_needs_a_title_and_a_page() {
    let f = Fixture::new();
    assert_eq!(
        err(f.views.create_view_from_tool(P, "s1", &json!({ "html": HTML })).await),
        "A view needs a title"
    );
    assert_eq!(
        err(f
            .views
            .create_view_from_tool(P, "s1", &json!({ "title": "x", "html": "  " }))
            .await),
        "html must be the page, as one HTML document"
    );
}

#[tokio::test]
async fn starting_sessions_stays_off_unless_claude_turns_it_on() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Tests", "html": HTML })).await;
    assert_eq!(f.view(&id).unwrap().starts_sessions, None);
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["start_sessions"],
        false
    );
    let page = f.views.serve_page(&format!("nodus-view://{id}/"));
    assert!(!page.body.contains("\"startsSessions\":true"));
}

#[tokio::test]
async fn starting_sessions_lets_the_page_know_when_it_may_start_sessions() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Tests", "html": HTML, "start_sessions": true })).await;
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["start_sessions"],
        true
    );
    assert!(
        f.views
            .serve_page(&format!("nodus-view://{id}/"))
            .body
            .contains("\"startsSessions\":true")
    );
    f.views
        .update_view_from_tool(P, "s1", &json!({ "view_id": id, "start_sessions": false }))
        .await
        .unwrap();
    assert_eq!(f.view(&id).unwrap().starts_sessions, None);
}

#[tokio::test]
async fn starting_sessions_takes_only_yes_for_an_answer_and_keeps_it_across_other_changes() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Tests", "html": HTML, "start_sessions": "yes" })).await;
    assert_eq!(f.view(&id).unwrap().starts_sessions, None);
    f.views
        .update_view_from_tool(P, "s1", &json!({ "view_id": id, "start_sessions": true }))
        .await
        .unwrap();
    f.views
        .update_view_from_tool(P, "s1", &json!({ "view_id": id, "title": "Renamed" }))
        .await
        .unwrap();
    assert_eq!(f.view(&id).unwrap().starts_sessions, Some(true));
}

#[tokio::test]
async fn starting_sessions_lets_the_user_stop_it_from_the_board_but_not_turn_it_on() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "Tests", "html": HTML })).await;
    let patch = |starts| ViewPatch {
        id: id.clone(),
        starts_sessions: Some(starts),
        ..ViewPatch::default()
    };
    f.views.save(P, &patch(true)).unwrap();
    assert_eq!(f.view(&id).unwrap().starts_sessions, None);
    f.views
        .update_view_from_tool(P, "s1", &json!({ "view_id": id, "start_sessions": true }))
        .await
        .unwrap();
    f.views.save(P, &patch(false)).unwrap();
    assert_eq!(f.view(&id).unwrap().starts_sessions, None);
}

async fn with_command(f: &Fixture, run: &str) -> String {
    create_id(
        f,
        json!({ "title": "Count", "html": HTML, "commands": [{ "name": "count", "run": run }] }),
    )
    .await
}

#[tokio::test]
async fn commands_wait_for_the_user_to_allow_a_command_before_it_runs() {
    let f = Fixture::new();
    let id = with_command(&f, "echo 42").await;
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["commands"][0]["allowed"],
        "waiting for the user"
    );
    assert_eq!(
        err(f.views.run_command(P, &id, "count").await),
        "“count” is waiting for the user to allow it on the view"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn commands_run_an_allowed_command_in_the_project_folder_and_send_the_page_its_output() {
    let f = Fixture::new();
    let id = with_command(&f, r#"printf "%s" "$(pwd)""#).await;
    f.views.approve(P, &id, "count", true).unwrap();
    let result = f.views.run_command(P, &id, "count").await.unwrap();
    assert_eq!((result.name.as_str(), result.code), ("count", Some(0)));
    let real = fs::canonicalize(&f.repo).unwrap();
    assert_eq!(result.stdout, real.to_string_lossy());
    let sent = serde_json::to_value(&result).unwrap();
    assert!(f.window.view_events().contains(&(id, ViewEvent::Result, sent)));
}

#[tokio::test]
async fn commands_remember_a_command_line_the_user_allowed_when_claude_puts_it_back() {
    let f = Fixture::new();
    let id = with_command(&f, "echo one").await;
    f.views.approve(P, &id, "count", true).unwrap();
    let put = |run: &str| json!({ "view_id": id, "commands": [{ "name": "count", "run": run }] });
    f.views.update_view_from_tool(P, "s1", &put("echo two")).await.unwrap();
    let allowed =
        |f: &Fixture| f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["commands"][0]["allowed"].clone();
    assert_eq!(allowed(&f), "waiting for the user");
    f.views.update_view_from_tool(P, "s1", &put("echo one")).await.unwrap();
    assert_eq!(allowed(&f), "yes");
}

#[tokio::test]
async fn commands_keep_a_command_the_user_turned_down_from_running() {
    let f = Fixture::new();
    let id = with_command(&f, "echo no").await;
    f.views.approve(P, &id, "count", false).unwrap();
    assert_eq!(
        err(f.views.run_command(P, &id, "count").await),
        "The user didn’t allow “count” to run"
    );
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["commands"][0]["allowed"],
        "no, the user turned it down"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn commands_report_a_failing_commands_exit_code_and_what_it_printed() {
    let f = Fixture::new();
    let id = with_command(&f, "echo oops >&2; exit 3").await;
    f.views.approve(P, &id, "count", true).unwrap();
    let result = f.views.run_command(P, &id, "count").await.unwrap();
    assert_eq!(
        (result.code, result.stderr.as_str(), result.error),
        (Some(3), "oops\n", None)
    );
}

#[tokio::test]
async fn the_pages_window_nodus_shows_a_page_the_other_views_with_their_data_and_not_itself() {
    let f = Fixture::new();
    let a = create_id(&f, json!({ "title": "A", "html": HTML, "data": { "a": 1 } })).await;
    let b = create_id(&f, json!({ "title": "B", "html": HTML, "data": { "b": 2 } })).await;
    assert_eq!(
        f.views.call(P, &a, "views", &json!({})).await.unwrap(),
        json!([{ "id": b, "title": "B", "description": "B", "data": { "b": 2 } }])
    );
    let sessions = f.views.call(P, &a, "sessions", &json!({})).await.unwrap();
    assert_eq!(sessions[0]["id"], "s1");
    assert_eq!(sessions[0]["title"], "New session");
}

#[tokio::test]
async fn the_pages_window_nodus_keeps_a_pages_state_within_bounds() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    assert_eq!(
        f.views
            .call(P, &id, "setState", &json!({ "state": { "filter": "open" } }))
            .await
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["state"],
        json!({ "filter": "open" })
    );
    assert!(
        f.window
            .view_events()
            .contains(&(id.clone(), ViewEvent::State, json!({ "filter": "open" })))
    );
    assert_eq!(
        err(f
            .views
            .call(P, &id, "setState", &json!({ "state": "x".repeat(300_000) }))
            .await),
        "The state is too big (over 256 KB)"
    );
}

#[tokio::test]
async fn the_pages_window_nodus_opens_web_links_only_and_refuses_anything_else() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    assert_eq!(
        err(f
            .views
            .call(P, &id, "openUrl", &json!({ "url": "file:///etc/passwd" }))
            .await),
        "Only http and https links open"
    );
    assert_eq!(
        err(f.views.call(P, &id, "readFile", &json!({})).await),
        "nodus.readFile isn’t something a view can do"
    );
    f.views
        .call(P, &id, "openUrl", &json!({ "url": "HTTPS://example.com/x" }))
        .await
        .unwrap();
    assert_eq!(*f.host.opened.lock().unwrap(), ["HTTPS://example.com/x"]);
    // VIEW-072: the board answers these, never the core.
    assert!(f.views.call(P, &id, "startSession", &json!({})).await.is_err());
}

#[tokio::test]
async fn the_pages_window_nodus_asks_the_brain_no_more_than_every_15_seconds() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    f.views
        .call(P, &id, "ask", &json!({ "prompt": "Fetch fresh data" }))
        .await
        .unwrap();
    assert_eq!(
        err(f.views.call(P, &id, "ask", &json!({ "prompt": "Again" })).await),
        "Asked too soon: a view can ask its brain once every 15 seconds"
    );
}

#[tokio::test]
async fn serving_serves_a_views_page_with_the_policy_and_nothing_else() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    let res = f.views.serve_page(&format!("nodus-view://{id}/?v=1"));
    assert_eq!(res.status, 200);
    assert!(
        res.header("content-security-policy")
            .unwrap()
            .contains("connect-src 'none'")
    );
    assert!(res.body.contains(r#"<p id="n"></p>"#));
    assert_eq!(
        res.headers,
        [
            ("Content-Type", "text/html; charset=utf-8".to_owned()),
            ("Content-Security-Policy", nodus_views::VIEW_CSP.to_owned()),
            ("Cache-Control", "no-store".to_owned()),
            ("X-Content-Type-Options", "nosniff".to_owned()),
        ]
    );
    let missing = f.views.serve_page("nodus-view://00000000-0000-0000-0000-000000000000/");
    assert_eq!((missing.status, missing.body.as_str()), (404, "Not found"));
    assert_eq!(
        f.views.serve_page(&format!("nodus-view://{id}/../state.json")).status,
        404
    );
}

#[tokio::test]
async fn serving_and_removing_take_a_view_off_the_board_with_its_files_and_its_brain() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    let mut brain = node("brain", 0.0);
    brain.view_of = Some(id.clone());
    f.add_node(brain);
    f.store
        .update(|s| s.board_mut(P).views[&id].session_id = Some("brain".into()));
    f.views.remove(P, &id).unwrap();
    assert!(f.view(&id).is_none());
    assert!(f.node("brain").is_none());
    assert!(
        f.store
            .read(|s| s.board(P).unwrap().hidden.contains(&"brain".to_owned()))
    );
    assert!(!f.views_dir(&id).exists());
    tokio::task::yield_now().await;
    assert_eq!(f.sessions.log(), ["kill:brain", "forget:brain"]);
    assert_eq!(
        err(f.views.remove(P, &id)),
        "There’s no view with that id on this board"
    );
}

// ---------------------------------------------------------------------------
// More than views.test.ts covers

#[tokio::test]
async fn says_so_when_the_board_isnt_showing_the_view() {
    // VIEW-108: after 3 s without a load.
    let f = Fixture::new();
    *f.window.hidden.lock().unwrap() = true;
    tokio::time::pause();
    let reply = create(&f, json!({ "title": "A", "html": HTML })).await;
    assert_eq!(
        reply["checked"],
        "The board isn’t showing this view right now, so the page couldn’t be checked for errors."
    );
    assert!(reply.get("errors").is_none());
}

#[tokio::test]
async fn places_size_and_describe_a_tools_view() {
    let f = Fixture::new();
    let id = create_id(
        &f,
        json!({
            "title": "  Big  ", "html": HTML, "width": "500", "height": null, "description": " For the team ",
            "commands": [], "refresh": { "every": 30, "prompt": "Fetch" }
        }),
    )
    .await;
    let view = f.view(&id).unwrap();
    assert_eq!(
        (view.title.as_str(), view.description.as_str()),
        ("Big", "For the team")
    );
    // VIEW-104: Number("500") and Number(null).
    assert_eq!((view.w, view.h), (Some(500.0), Some(160.0)));
    assert_eq!(view.commands, None);
    assert_eq!(view.refresh.as_ref().map(|r| r.every), Some(30.0));
    f.views.flush();
    let runtime: Value =
        serde_json::from_str(&fs::read_to_string(f.views_dir(&id).join("runtime.json")).unwrap()).unwrap();
    assert!(runtime["refreshedAt"].is_number());
}

#[tokio::test]
async fn reads_a_view_with_long_output_cut_and_errors_dated() {
    let f = Fixture::new();
    let id = with_command(&f, "echo x").await;
    let long = "y".repeat(4100);
    let result = json!({ "name": "count", "code": 0, "stdout": long, "stderr": "", "at": 1, "durationMs": 2 });
    let runtime =
        json!({ "results": { "count": result }, "errors": [{ "message": "boom", "at": 1_700_000_000_000_i64 }] });
    // A view first read from its files.
    let other = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    fs::create_dir_all(f.views_dir(other)).unwrap();
    fs::write(f.views_dir(other).join("runtime.json"), runtime.to_string()).unwrap();
    let mut view = f.view(&id).unwrap();
    view.id = other.into();
    f.store.update(|s| s.board_mut(P).views.insert(other.into(), view));
    let got = f.views.get_view_for_tool(P, "s1", Some(&json!(other))).unwrap();
    let stdout = got["commands"][0]["last"]["stdout"].as_str().unwrap();
    assert!(stdout.ends_with("y… (100 more characters)") && stdout.starts_with(&"y".repeat(4000)));
    assert_eq!(got["commands"][0]["last"]["durationMs"], 2);
    assert_eq!(
        got["errors"],
        json!([{ "message": "boom", "at": "2023-11-14T22:13:20.000Z" }])
    );
    assert_eq!(got["html"], "");
    assert!(got.get("data").is_none() && got.get("refresh").is_none());
}

#[tokio::test]
async fn lists_views_oldest_first_saying_which_are_yours() {
    let f = Fixture::new();
    let a = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    let b = f
        .views
        .create_view_from_tool(P, "other", &json!({ "title": "B", "html": HTML }))
        .await
        .unwrap()["view_id"]
        .clone();
    f.store.update(|s| {
        s.board_mut(P).views[&a].created_at = 2.0;
        s.board_mut(P).views[b.as_str().unwrap()].created_at = 1.0;
    });
    assert_eq!(
        f.views.list_views_for_tool(P, "s1"),
        json!([
            { "id": b, "title": "B", "description": "B" },
            { "id": a, "title": "A", "description": "A", "yours": true }
        ])
    );
}

#[tokio::test]
async fn keeps_the_last_twenty_errors_cut_to_two_thousand_characters() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    for i in 0..25 {
        f.views.report(
            P,
            &id,
            &ViewReport {
                error: Some(format!("{i}{}", "e".repeat(2100))),
                loaded: None,
            },
        );
    }
    f.views.report(
        P,
        "gone",
        &ViewReport {
            error: Some("x".into()),
            loaded: None,
        },
    );
    let errors = f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["errors"].clone();
    // get_view shows the last 10 of the 20 kept.
    assert_eq!(errors.as_array().unwrap().len(), 10);
    assert!(errors[0]["message"].as_str().unwrap().starts_with("15e"));
    assert_eq!(errors[0]["message"].as_str().unwrap().len(), 2000);
    f.views.flush();
    let runtime: Value =
        serde_json::from_str(&fs::read_to_string(f.views_dir(&id).join("runtime.json")).unwrap()).unwrap();
    assert_eq!(runtime["errors"].as_array().unwrap().len(), 20);
}

#[tokio::test]
async fn approving_remembers_the_answers_and_runs_what_was_allowed() {
    let f = Fixture::new();
    let id = with_command(&f, "echo one").await;
    assert_eq!(err(f.views.approve(P, &id, "nope", true)), "That command is gone");
    f.views.approve(P, &id, "count", false).unwrap();
    let view = f.view(&id).unwrap();
    assert_eq!(
        (view.allowed_runs.clone(), view.denied_runs.clone()),
        (Some(vec![]), Some(vec!["echo one".to_owned()]))
    );
    f.views.approve(P, &id, "count", true).unwrap();
    let view = f.view(&id).unwrap();
    assert_eq!(
        (view.allowed_runs.clone(), view.denied_runs.clone()),
        (Some(vec!["echo one".to_owned()]), Some(vec![]))
    );
    let cmd = &view.commands.unwrap()[0];
    assert_eq!(
        (cmd.approved.as_deref(), cmd.denied.as_deref()),
        (Some("echo one"), None)
    );
    assert!(
        f.window
            .take()
            .iter()
            .any(|e| matches!(e, nodus_domain::Event::BoardChanged { .. }))
    );
}

#[tokio::test]
async fn saves_where_the_user_put_a_view_and_renames_its_brain_too() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    let mut brain = node("brain", 0.0);
    brain.view_of = Some(id.clone());
    f.add_node(brain);
    f.store
        .update(|s| s.board_mut(P).views[&id].session_id = Some("brain".into()));
    f.views
        .save(
            P,
            &ViewPatch {
                id: id.clone(),
                x: Some(10.4),
                y: Some(-2.5),
                w: Some(5000.0),
                h: Some(100.0),
                title: Some("  Deploys ".into()),
                starts_sessions: None,
            },
        )
        .unwrap();
    let view = f.view(&id).unwrap();
    assert_eq!(
        (view.x, view.y, view.w, view.h),
        (10.0, -2.0, Some(2400.0), Some(160.0))
    );
    assert_eq!(view.title, "Deploys");
    assert_eq!(f.node("brain").unwrap().title.as_deref(), Some("View: Deploys"));
    assert_eq!(
        err(f.views.save(
            P,
            &ViewPatch {
                id: "__proto__".into(),
                ..ViewPatch::default()
            }
        )),
        "There’s no view with that id on this board"
    );
}

#[tokio::test]
async fn reads_a_pages_notes_and_one_view() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML, "data": [1] })).await;
    f.store.update(|s| {
        let notes = &mut s.board_mut(P).notes;
        for (nid, at) in [("late", 2), ("early", 1)] {
            let note = json!({ "id": nid, "text": nid, "color": "red", "x": 0, "y": 0, "createdAt": at, "from": "s1" });
            notes.insert(nid.into(), serde_json::from_value(note).unwrap());
        }
    });
    assert_eq!(
        f.views.call(P, &id, "notes", &Value::Null).await.unwrap(),
        json!([{ "id": "early", "text": "early", "color": "red" }, { "id": "late", "text": "late", "color": "red" }])
    );
    assert_eq!(
        f.views.call(P, &id, "view", &json!({ "id": id })).await.unwrap(),
        json!({ "id": id, "title": "A", "description": "A", "data": [1] })
    );
    assert_eq!(f.views.call(P, &id, "git", &json!({})).await.unwrap(), Value::Null);
    assert_eq!(
        err(f.views.call(P, "gone", "notes", &json!({})).await),
        "There’s no view with that id on this board"
    );
}

#[tokio::test]
async fn forgets_a_removed_projects_view_files() {
    let f = Fixture::new();
    let id = create_id(&f, json!({ "title": "A", "html": HTML })).await;
    let board = f.store.read(|s| s.board(P).cloned());
    f.views.remove_project_views(board.as_ref());
    assert!(!f.views_dir(&id).exists());
}

/// Waits for a command's result to reach the page.
#[cfg(unix)]
async fn result_event(f: &Fixture, id: &str) -> Value {
    for _ in 0..500 {
        if let Some((_, _, payload)) = f
            .window
            .view_events()
            .into_iter()
            .find(|(v, e, _)| v == id && *e == ViewEvent::Result)
        {
            return payload;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no result for {id}");
}

#[cfg(unix)]
#[tokio::test]
async fn runs_scheduled_commands_that_are_due_and_allowed() {
    // VIEW-086: a scheduled command that never ran runs on the next tick.
    let f = Fixture::new();
    let id = create_id(
        &f,
        json!({ "title": "A", "html": HTML, "commands": [
            { "name": "due", "run": "echo due", "every": 1 },
            { "name": "asked", "run": "echo asked" }
        ] }),
    )
    .await;
    f.views.tick();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(f.window.view_events().iter().all(|(_, e, _)| *e != ViewEvent::Result));
    f.views.approve(P, &id, "asked", true).unwrap();
    let asked = result_event(&f, &id).await;
    assert_eq!(asked["stdout"], "asked\n");
    f.window.take();
    f.views.approve(P, &id, "due", true).unwrap();
    let due = result_event(&f, &id).await;
    assert_eq!(due["stdout"], "due\n");
    f.window.take();
    // It ran just now, so it isn't due again for a minute.
    f.views.tick();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(f.window.view_events().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn shares_a_run_already_going() {
    // VIEW-074: no second process.
    let f = Fixture::new();
    let id = with_command(&f, "sleep 0.2; echo $$").await;
    f.views.approve(P, &id, "count", true).unwrap();
    let (a, b) = tokio::join!(
        f.views.run_command(P, &id, "count"),
        f.views.run_command(P, &id, "count")
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a, b);
    assert!(!a.stdout.is_empty() && a.duration_ms >= 0.0 && a.at > 0.0);
}

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn stops_a_command_after_two_minutes() {
    // VIEW-078: with the clock paused it reaches the timeout while the command sleeps.
    let f = Fixture::paused(1_800_000_000_000.0);
    let id = with_command(&f, "sleep 30").await;
    f.views.approve(P, &id, "count", true).unwrap();
    let result = f.views.run_command(P, &id, "count").await.unwrap();
    assert_eq!(result.code, None);
    assert_eq!(
        result.error.as_deref(),
        Some("It took longer than 2 minutes, so it was stopped")
    );
    assert!(result.duration_ms >= 120_000.0);
}

#[tokio::test]
async fn says_why_a_command_couldnt_start() {
    // VIEW-079: a project folder that's gone.
    let f = Fixture::new();
    let id = with_command(&f, "echo x").await;
    f.views.approve(P, &id, "count", false).unwrap();
    f.store
        .update(|s| s.projects[0].repo_path = f.user_data.path().join("gone").to_string_lossy().into_owned());
    f.views.approve(P, &id, "count", true).unwrap();
    let result = f.views.run_command(P, &id, "count").await.unwrap();
    assert_eq!(result.code, None);
    assert!(result.error.unwrap().ends_with(" ENOENT"));
    assert_eq!(
        f.views.run_command(P, &id, "nope").await.unwrap_err().to_string(),
        "This view has no command called “nope”"
    );
}
