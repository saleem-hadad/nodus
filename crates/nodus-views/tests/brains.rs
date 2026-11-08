//! A view's brain: started when first needed, asked, queued, typed into,
//! stopped when idle, and its schedules (VIEW-084 to VIEW-100). On a paused
//! clock, with fake processes.

mod common;

use common::{Fixture, HTML, P, advance, node};
use nodus_domain::{CreateViewOptions, Event, SessionStatus};
use nodus_sessions::launch::PermissionMode;
use nodus_views::{AskOrigin, VIEW_GUIDE, brain_args, brain_prompt};
use serde_json::json;

const NOW: f64 = 1_800_000_000_000.0;

async fn view(f: &Fixture, args: serde_json::Value) -> String {
    f.views.create_view_from_tool(P, "s1", &args).await.unwrap()["view_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// A view whose brain, `brain`, is on the board.
async fn with_brain(f: &Fixture) -> String {
    let id = view(f, json!({ "title": "Tickets", "html": HTML })).await;
    let mut brain = node("brain", 0.0);
    brain.view_of = Some(id.clone());
    f.add_node(brain);
    f.store
        .update(|s| s.board_mut(P).views[&id].session_id = Some("brain".into()));
    id
}

#[tokio::test(start_paused = true)]
async fn starts_a_brain_on_the_first_ask() {
    let f = Fixture::paused(NOW);
    let id = view(
        &f,
        json!({ "title": "Tickets", "html": HTML, "description": "My open tickets" }),
    )
    .await;
    f.window.take();
    f.views
        .ask_brain(P, &id, "  Fetch them  ", AskOrigin::User)
        .await
        .unwrap();
    let spawns = f.brains.spawns();
    assert_eq!(spawns.len(), 1);
    let spawn = &spawns[0];
    let brain = f.view(&id).unwrap().session_id.unwrap();
    assert_eq!(spawn.session_id, brain);
    assert_eq!((spawn.project_id.as_str(), spawn.cwd.as_str()), (P, f.repo.as_str()));
    assert!(!spawn.resume);
    assert_eq!(spawn.initial_prompt.as_deref(), Some("Fetch them"));
    assert_eq!(spawn.permission_mode, Some(PermissionMode::Auto));
    let args = spawn.extra_args.clone().unwrap();
    assert_eq!(args[0], "--append-system-prompt");
    assert!(args[1].contains(&format!(
        "The view is \"Tickets\" (id {id}). The user asked for:\n\nMy open tickets\n\n"
    )));
    assert!(args[1].contains(VIEW_GUIDE));
    // VIEW-088
    let node = f.node(&brain).unwrap();
    assert_eq!(node.title.as_deref(), Some("View: Tickets"));
    assert_eq!(node.view_of.as_deref(), Some(id.as_str()));
    assert_eq!(
        (node.auto_mode, node.created_by_app, node.archived),
        (Some(true), Some(true), false)
    );
    assert_eq!((node.x, node.y), (f.view(&id).unwrap().x, f.view(&id).unwrap().y));
    assert_eq!(f.window.take(), [Event::BoardChanged { project_id: P.into() }]);
}

#[tokio::test(start_paused = true)]
async fn takes_a_brain_off_again_when_it_cant_start() {
    let f = Fixture::paused(NOW);
    let id = view(&f, json!({ "title": "A", "html": HTML })).await;
    *f.brains.fail.lock().unwrap() = Some("Claude Code CLI not found".into());
    let failed = f.views.ensure_brain(P, &id).await.unwrap_err();
    assert_eq!(failed.to_string(), "Claude Code CLI not found");
    assert_eq!(f.view(&id).unwrap().session_id, None);
    assert_eq!(f.store.read(|s| s.board(P).unwrap().nodes.len()), 1);
}

#[tokio::test(start_paused = true)]
async fn starts_an_idle_brain_only_when_there_is_none() {
    let f = Fixture::paused(NOW);
    let id = with_brain(&f).await;
    assert_eq!(f.views.brain(P, &id).await.unwrap(), "brain");
    let other = view(&f, json!({ "title": "B", "html": HTML })).await;
    let started = f.views.brain(P, &other).await.unwrap();
    assert_eq!(f.brains.spawns()[0].initial_prompt, None);
    assert_eq!(f.view(&other).unwrap().session_id, Some(started));
}

#[tokio::test(start_paused = true)]
async fn creates_a_view_from_the_board_and_has_its_brain_build_it() {
    let f = Fixture::paused(NOW);
    let opts = |description: &str, x: f64| CreateViewOptions {
        description: description.into(),
        x,
        y: 20.6,
    };
    let view = f.views.create(P, &opts(" Deploys \n", 10.5)).await.unwrap();
    assert_eq!(
        (view.title.as_str(), view.description.as_str(), view.version),
        ("New view", "Deploys", 0)
    );
    assert_eq!((view.x, view.y, view.created_at), (11.0, 21.0, NOW));
    assert!(view.session_id.is_some());
    assert_eq!(
        f.brains.spawns()[0].initial_prompt.as_deref(),
        Some("Build this view:\n\nDeploys")
    );
    let changes = f
        .window
        .take()
        .into_iter()
        .filter(|e| matches!(e, Event::BoardChanged { .. }))
        .count();
    assert_eq!(changes, 2);

    let err = |r: Result<nodus_domain::BoardView, nodus_views::ViewError>| r.unwrap_err().to_string();
    assert_eq!(
        err(f.views.create(P, &opts("  ", 0.0)).await),
        "Describe what the view should show"
    );
    assert_eq!(
        err(f.views.create(P, &opts("x", f64::NAN)).await),
        "A view needs a place on the board"
    );
    assert_eq!(
        err(f.views.create("gone", &opts("x", 0.0)).await),
        "Unknown project gone"
    );
    *f.brains.fail.lock().unwrap() = Some("no claude".into());
    assert_eq!(err(f.views.create(P, &opts("x", 0.0)).await), "no claude");
    assert_eq!(f.store.read(|s| s.board(P).unwrap().views.len()), 1);
}

#[tokio::test(start_paused = true)]
async fn types_into_a_brain_waiting_at_its_prompt() {
    let f = Fixture::paused(NOW);
    let id = with_brain(&f).await;
    f.sessions.set("brain", true, SessionStatus::Done);
    f.views.ask(P, &id, "Fetch\nthem").await.unwrap();
    assert_eq!(
        f.brains.writes(),
        [("brain".to_owned(), "\x1b[200~Fetch\nthem\x1b[201~".to_owned())]
    );
    advance(199).await;
    assert_eq!(f.brains.writes().len(), 1);
    advance(1).await;
    assert_eq!(f.brains.writes()[1], ("brain".to_owned(), "\r".to_owned()));
    assert!(f.brains.spawns().is_empty());
}

#[tokio::test(start_paused = true)]
async fn queues_asks_for_a_busy_brain_until_its_turn_ends() {
    let f = Fixture::paused(NOW);
    let id = with_brain(&f).await;
    f.sessions.set("brain", true, SessionStatus::Working);
    f.views.ask(P, &id, "one").await.unwrap();
    f.views.ask(P, &id, "two").await.unwrap();
    // A scheduled refresh skips a busy brain.
    f.views
        .ask_brain(P, &id, "scheduled", AskOrigin::Schedule)
        .await
        .unwrap();
    assert!(f.brains.writes().is_empty());
    f.sessions.set("brain", true, SessionStatus::Done);
    f.views.brain_status("brain", SessionStatus::Done, true);
    advance(499).await;
    assert!(f.brains.writes().is_empty());
    advance(1).await;
    assert_eq!(
        f.brains.writes(),
        [("brain".to_owned(), "\x1b[200~one\n\ntwo\x1b[201~".to_owned())]
    );
}

#[tokio::test(start_paused = true)]
async fn keeps_only_the_last_4000_characters_of_what_is_queued() {
    let f = Fixture::paused(NOW);
    let id = with_brain(&f).await;
    f.sessions.set("brain", true, SessionStatus::Waiting);
    f.views.ask(P, &id, &"a".repeat(3000)).await.unwrap();
    f.views.ask(P, &id, &"b".repeat(3000)).await.unwrap();
    f.views.brain_status("brain", SessionStatus::Done, true);
    advance(500).await;
    let typed = f.brains.writes()[0].1.clone();
    let text = typed.trim_start_matches("\x1b[200~").trim_end_matches("\x1b[201~");
    assert_eq!(text, format!("{}\n\n{}", "a".repeat(998), "b".repeat(3000)));
}

#[tokio::test(start_paused = true)]
async fn resumes_a_stopped_brain_with_the_prompt() {
    let f = Fixture::paused(NOW);
    let id = with_brain(&f).await;
    f.views.ask(P, &id, "Fetch").await.unwrap();
    let spawn = &f.brains.spawns()[0];
    assert_eq!((spawn.session_id.as_str(), spawn.resume), ("brain", false));
    assert_eq!(spawn.initial_prompt.as_deref(), Some("Fetch"));
    assert_eq!(spawn.extra_args.as_ref().unwrap()[0], "--append-system-prompt");
    assert_eq!(
        f.views.ask(P, &id, " \n ").await.unwrap_err().to_string(),
        "Say what the view’s brain should do"
    );
}

#[tokio::test(start_paused = true)]
async fn stops_a_brain_idle_for_five_minutes() {
    let f = Fixture::paused(NOW);
    with_brain(&f).await;
    f.sessions.set("brain", true, SessionStatus::Done);
    f.views.brain_status("brain", SessionStatus::Done, true);
    advance(5 * 60 * 1000 - 1).await;
    assert!(f.sessions.log().is_empty());
    advance(1).await;
    assert_eq!(f.sessions.log(), ["kill:brain"]);
}

#[tokio::test(start_paused = true)]
async fn waits_another_five_minutes_when_the_user_typed_into_it() {
    let f = Fixture::paused(NOW);
    with_brain(&f).await;
    f.sessions.set("brain", true, SessionStatus::Done);
    f.views.brain_status("brain", SessionStatus::Done, true);
    advance(60 * 1000).await;
    f.views.brain_input("brain");
    advance(4 * 60 * 1000).await;
    // Typed into a minute in: not quiet for five minutes, so it waits again.
    assert!(f.sessions.log().is_empty());
    advance(5 * 60 * 1000).await;
    assert_eq!(f.sessions.log(), ["kill:brain"]);
}

#[tokio::test(start_paused = true)]
async fn leaves_brains_that_are_working_and_sessions_that_arent_brains() {
    let f = Fixture::paused(NOW);
    with_brain(&f).await;
    f.sessions.set("brain", true, SessionStatus::Done);
    f.views.brain_status("brain", SessionStatus::Done, true);
    f.views.brain_status("brain", SessionStatus::Working, true);
    f.views.brain_status("s1", SessionStatus::Done, true);
    // Input to a session with no idle timer isn't noted.
    f.views.brain_input("s1");
    advance(10 * 60 * 1000).await;
    assert!(f.sessions.log().is_empty());
}

#[tokio::test(start_paused = true)]
async fn refreshes_on_schedule_while_the_app_is_open() {
    let f = Fixture::paused(NOW);
    let id = view(
        &f,
        json!({ "title": "Tickets", "html": HTML, "refresh": { "every": 10, "prompt": "Fetch my tickets" } }),
    )
    .await;
    f.views.start_schedules();
    f.views.start_schedules();
    advance(9 * 60 * 1000 + 30 * 1000).await;
    assert!(f.brains.spawns().is_empty());
    advance(30 * 1000).await;
    let spawns = f.brains.spawns();
    assert_eq!(spawns.len(), 1);
    assert_eq!(spawns[0].initial_prompt.as_deref(), Some("Fetch my tickets"));
    assert_eq!(
        f.view(&id).unwrap().session_id.as_deref(),
        Some(spawns[0].session_id.as_str())
    );
    // Its runtime says when, so the next is ten minutes on.
    f.sessions.set(&spawns[0].session_id, true, SessionStatus::Working);
    advance(9 * 60 * 1000).await;
    assert_eq!(f.brains.spawns().len(), 1);
    f.views.flush();
    advance(20 * 60 * 1000).await;
    // Quitting stopped the schedules.
    assert!(f.brains.writes().is_empty());
}

#[tokio::test(start_paused = true)]
async fn refreshing_asks_the_brain_and_runs_the_allowed_commands() {
    let f = Fixture::paused(NOW);
    let id = view(
        &f,
        json!({ "title": "T", "html": HTML, "refresh": { "every": 10, "prompt": "Fetch" } }),
    )
    .await;
    assert_eq!(f.views.refresh(P, &id).await.unwrap(), []);
    assert_eq!(f.brains.spawns()[0].initial_prompt.as_deref(), Some("Fetch"));
    let plain = view(&f, json!({ "title": "U", "html": HTML })).await;
    assert_eq!(f.views.refresh(P, &plain).await.unwrap(), []);
    assert_eq!(f.brains.spawns().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn gives_brains_their_arguments_and_nobody_else() {
    let f = Fixture::paused(NOW);
    let id = with_brain(&f).await;
    let board = f.store.read(|s| s.board(P).cloned().unwrap());
    let args = brain_args(&board, &f.node("brain").unwrap()).unwrap();
    assert_eq!(
        args,
        ["--append-system-prompt".to_owned(), brain_prompt(&board.views[&id])]
    );
    assert!(brain_args(&board, &f.node("s1").unwrap()).is_none());
    let mut orphan = node("orphan", 0.0);
    orphan.view_of = Some("gone".into());
    assert!(brain_args(&board, &orphan).is_none());
    // A rename names its brain's card too (VIEW-091).
    f.views
        .update_view_from_tool(P, "brain", &json!({ "title": "Renamed" }))
        .await
        .unwrap();
    assert_eq!(f.node("brain").unwrap().title.as_deref(), Some("View: Renamed"));
}

#[tokio::test(start_paused = true)]
async fn writes_a_views_runtime_half_a_second_after_the_last_change() {
    // VIEW-006
    let f = Fixture::paused(NOW);
    let id = view(&f, json!({ "title": "A", "html": HTML })).await;
    advance(1000).await;
    let file = f.views_dir(&id).join("runtime.json");
    std::fs::remove_file(&file).unwrap();
    let report = |error: &str| nodus_domain::ViewReport {
        error: Some(error.into()),
        loaded: None,
    };
    f.views.report(P, &id, &report("one"));
    advance(400).await;
    f.views.report(P, &id, &report("two"));
    advance(499).await;
    assert!(!file.exists());
    advance(1).await;
    let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(written["errors"][1]["message"], "two");
    assert_eq!(written["errors"][1]["at"], NOW + 1000.0 + 500.0 + 400.0);
}
