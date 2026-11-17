//! A view's schedules as its caption shows them, and changing how often
//! they run from its picker (SCHED-061 to SCHED-064), on a clock the test
//! moves by hand.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Fixture, HTML, P};
use nodus_domain::{Event, ViewSchedule};
use serde_json::{Value, json};

const NOW: f64 = 1_800_000_000_000.0;
const MIN: f64 = 60_000.0;

/// A fixture whose clock says what the test last set it to.
fn by_hand() -> (Fixture, Arc<Mutex<f64>>) {
    let time = Arc::new(Mutex::new(NOW));
    let read = Arc::clone(&time);
    (Fixture::with_clock(Arc::new(move || *read.lock().unwrap())), time)
}

fn set(time: &Mutex<f64>, at: f64) {
    *time.lock().unwrap() = at;
}

async fn create(f: &Fixture, args: Value) -> String {
    f.views.create_view_from_tool(P, "s1", &args).await.unwrap()["view_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn err<T: std::fmt::Debug>(result: Result<T, nodus_views::ViewError>) -> String {
    result.unwrap_err().to_string()
}

/// Waits for something to have happened, for up to 5 s.
async fn until(mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("it didn't happen");
}

/// The command results the window was sent (the tests that run commands are
/// Unix only).
#[cfg(unix)]
fn results(f: &Fixture) -> usize {
    f.window
        .view_events()
        .iter()
        .filter(|(_, e, _)| *e == nodus_domain::ViewEvent::Result)
        .count()
}

#[tokio::test]
async fn lists_each_schedule_with_when_it_runs_next() {
    // SCHED-061, SCHED-062
    let (f, time) = by_hand();
    let id = create(
        &f,
        json!({ "title": "Deploys", "html": HTML,
            "commands": [
                { "name": "deploys", "run": "echo deploys", "every": 15 },
                { "name": "on click", "run": "echo clicked" }
            ],
            "refresh": { "every": 30, "prompt": "Fetch my tickets" } }),
    )
    .await;
    set(&time, NOW + 2.0 * MIN);
    let schedules = f.views.schedules(P, &id).unwrap();
    assert_eq!(
        schedules,
        [
            // Not run yet, and waiting for the user: it counts from now, and doesn't run.
            ViewSchedule {
                key: "command:deploys".into(),
                label: "deploys".into(),
                every: 15.0,
                paused: false,
                allowed: false,
                next_at: NOW + 17.0 * MIN,
            },
            // From when Claude set it.
            ViewSchedule {
                key: "refresh".into(),
                label: "Claude’s refresh".into(),
                every: 30.0,
                paused: false,
                allowed: true,
                next_at: NOW + 30.0 * MIN,
            },
        ]
    );
    assert_eq!(schedules[0].last_at(), NOW + 2.0 * MIN);
    assert_eq!(
        serde_json::to_value(&schedules[1]).unwrap(),
        json!({ "key": "refresh", "label": "Claude’s refresh", "every": 30.0, "paused": false, "allowed": true,
            "nextAt": NOW + 30.0 * MIN })
    );
    assert_eq!(
        err(f.views.schedules(P, "gone")),
        "There’s no view with that id on this board"
    );
    // A view with neither has none.
    let plain = create(&f, json!({ "title": "Plain", "html": HTML })).await;
    assert_eq!(f.views.schedules(P, &plain).unwrap(), []);
}

#[cfg(unix)]
#[tokio::test]
async fn a_command_that_ran_counts_from_its_last_run() {
    // SCHED-062
    let (f, time) = by_hand();
    let id = create(
        &f,
        json!({ "title": "A", "html": HTML, "commands": [{ "name": "deploys", "run": "echo hi", "every": 15 }] }),
    )
    .await;
    set(&time, NOW + MIN);
    f.views.approve(P, &id, "deploys", true).unwrap();
    until(|| results(&f) == 1).await;
    set(&time, NOW + 5.0 * MIN);
    let schedule = f.views.schedules(P, &id).unwrap().remove(0);
    assert!(schedule.runs());
    assert_eq!(schedule.next_at, NOW + 16.0 * MIN);
}

#[tokio::test]
async fn sets_how_often_a_command_runs_in_place_keeping_its_approval() {
    // SCHED-063, SCHED-064
    let (f, _) = by_hand();
    let id = create(
        &f,
        json!({ "title": "A", "html": HTML, "commands": [{ "name": "deploys", "run": "echo hi", "every": 15 }] }),
    )
    .await;
    f.views.approve(P, &id, "deploys", true).unwrap();
    // What a server adds (SRV-145), which Claude's commands would lose.
    f.store.update(|s| {
        let cmd = &mut s.board_mut(P).views[&id].commands.as_mut().unwrap()[0];
        cmd.extra.insert("approvedBy".into(), json!("alice"));
    });
    f.window.take();
    let command = || serde_json::to_value(&f.view(&id).unwrap().commands.unwrap()[0]).unwrap();

    f.views.set_every(P, &id, "command:deploys", Some(5.0)).unwrap();
    assert_eq!(
        command(),
        json!({ "name": "deploys", "run": "echo hi", "every": 5.0, "approved": "echo hi", "approvedBy": "alice" })
    );
    assert!(
        f.window
            .take()
            .iter()
            .any(|e| matches!(e, Event::BoardChanged { project_id } if project_id == P))
    );

    // Off keeps the interval for later; it waits to be asked meanwhile.
    f.views.set_every(P, &id, "command:deploys", None).unwrap();
    assert_eq!(
        command(),
        json!({ "name": "deploys", "run": "echo hi", "every": 5.0, "paused": true, "approved": "echo hi",
            "approvedBy": "alice" })
    );
    let schedule = f.views.schedules(P, &id).unwrap().remove(0);
    assert!(schedule.paused && schedule.allowed && !schedule.runs());
    assert_eq!(
        f.views.get_view_for_tool(P, "s1", Some(&json!(id))).unwrap()["commands"][0]["paused"],
        true
    );

    // An interval turns it on again, within the views' limits.
    f.views.set_every(P, &id, "command:deploys", Some(0.2)).unwrap();
    assert_eq!(
        command(),
        json!({ "name": "deploys", "run": "echo hi", "every": 1.0, "approved": "echo hi", "approvedBy": "alice" })
    );
    f.views.set_every(P, &id, "command:deploys", Some(99_999.0)).unwrap();
    assert_eq!(command()["every"], 10_080.0);

    assert_eq!(
        err(f.views.set_every(P, &id, "command:deploys", Some(-3.0))),
        "How often must be a number of minutes"
    );
    assert_eq!(
        err(f.views.set_every(P, &id, "command:deploys", Some(f64::NAN))),
        "How often must be a number of minutes"
    );
    assert_eq!(
        err(f.views.set_every(P, &id, "command:gone", Some(5.0))),
        "That command is gone"
    );
    assert_eq!(
        err(f.views.set_every(P, &id, "deploys", Some(5.0))),
        "This view has no schedule called “deploys”"
    );
    assert_eq!(
        err(f.views.set_every(P, &id, "refresh", None)),
        "This view doesn’t refresh"
    );
    assert_eq!(
        err(f.views.set_every(P, "gone", "refresh", None)),
        "There’s no view with that id on this board"
    );
}

#[tokio::test]
async fn pausing_a_refresh_keeps_its_prompt_until_its_turned_on_again() {
    // SCHED-064
    let (f, _) = by_hand();
    let id = create(
        &f,
        json!({ "title": "T", "html": HTML, "refresh": { "every": 30, "prompt": "Fetch my tickets" } }),
    )
    .await;
    let refresh = || serde_json::to_value(f.view(&id).unwrap().refresh.unwrap()).unwrap();
    f.views.set_every(P, &id, "refresh", None).unwrap();
    assert_eq!(
        refresh(),
        json!({ "every": 30.0, "prompt": "Fetch my tickets", "paused": true })
    );
    let schedule = f.views.schedules(P, &id).unwrap().remove(0);
    assert!(schedule.paused && !schedule.runs());
    // Picking an interval turns it on, asking the brain at most every ten minutes.
    f.views.set_every(P, &id, "refresh", Some(3.0)).unwrap();
    assert_eq!(refresh(), json!({ "every": 10.0, "prompt": "Fetch my tickets" }));
    // A refresh Claude sets runs.
    f.views.set_every(P, &id, "refresh", None).unwrap();
    f.views
        .update_view_from_tool(
            P,
            "s1",
            &json!({ "view_id": id, "refresh": { "every": 60, "prompt": "Fetch again" } }),
        )
        .await
        .unwrap();
    assert_eq!(refresh(), json!({ "every": 60.0, "prompt": "Fetch again" }));
}

#[tokio::test]
async fn the_ticker_skips_a_refresh_the_user_turned_off() {
    // SCHED-064
    let (f, time) = by_hand();
    let id = create(
        &f,
        json!({ "title": "T", "html": HTML, "refresh": { "every": 10, "prompt": "Fetch my tickets" } }),
    )
    .await;
    f.views.set_every(P, &id, "refresh", None).unwrap();
    set(&time, NOW + 11.0 * MIN);
    f.views.tick();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(f.brains.spawns().is_empty());
    // Turned on again, it's due: it's been eleven minutes.
    f.views.set_every(P, &id, "refresh", Some(10.0)).unwrap();
    f.views.tick();
    until(|| f.brains.spawns().len() == 1).await;
    assert_eq!(f.brains.spawns()[0].initial_prompt.as_deref(), Some("Fetch my tickets"));
}

#[cfg(unix)]
#[tokio::test]
async fn the_ticker_skips_a_command_the_user_turned_off() {
    // SCHED-064
    let (f, time) = by_hand();
    let id = create(
        &f,
        json!({ "title": "A", "html": HTML, "commands": [{ "name": "count", "run": "echo 1", "every": 1 }] }),
    )
    .await;
    f.views.approve(P, &id, "count", true).unwrap();
    until(|| results(&f) == 1).await;
    f.views.set_every(P, &id, "command:count", None).unwrap();
    set(&time, NOW + 5.0 * MIN);
    f.views.tick();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(results(&f), 1);
    // The page can still run it.
    f.views.run_command(P, &id, "count").await.unwrap();
    assert_eq!(results(&f), 2);
    // Turned on again, it runs a minute after that.
    f.views.set_every(P, &id, "command:count", Some(1.0)).unwrap();
    set(&time, NOW + 6.0 * MIN);
    f.views.tick();
    until(|| results(&f) == 3).await;
}
