//! Starting up: why updates are off, and the automatic checks (UPD-001…UPD-012).

mod common;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::*;
use nodus_domain::{Event, UpdateState, UpdateStatus};
use nodus_updater::{
    LATEST_URL, OFF_APPLICATIONS, OFF_DEVELOPMENT, OFF_MOVE, OFF_PLATFORM, Platform, RestartOutcome, Updater,
};

fn off(reason: &str) -> UpdateState {
    UpdateState::Off { reason: reason.into() }
}

/// A world that answers that nothing is newer.
fn quiet(platform: Platform) -> World {
    let world = World::new(platform, "0.9.4", no_tools());
    world
        .system
        .route(LATEST_URL, Route::json(&release_json("v0.9.4", &[])));
    world
}

#[tokio::test]
async fn upd_001_002_starts_idle_and_sends_every_change_whole() {
    let world = quiet(Platform::Windows);
    let updater = Updater::new(world.app.clone(), world.system.clone(), world.events.clone());
    assert_eq!(
        updater.status(),
        UpdateStatus {
            state: UpdateState::Idle,
            current: "0.9.4".into()
        }
    );
    updater.start();
    updater.check(true).await;
    assert_eq!(
        world.events.events(),
        [
            Event::UpdateStatus(UpdateStatus {
                state: UpdateState::Checking,
                current: "0.9.4".into()
            }),
            Event::UpdateStatus(UpdateStatus {
                state: UpdateState::Latest,
                current: "0.9.4".into()
            }),
        ]
    );
    let json = nodus_domain::json::to_string(&updater.status()).unwrap();
    assert_eq!(json, r#"{"state":"latest","current":"0.9.4"}"#);
}

#[tokio::test]
async fn upd_003_004_other_systems_are_off_and_do_nothing_else() {
    let world = quiet(Platform::Other);
    std::fs::create_dir_all(world.updates()).unwrap();
    std::fs::write(world.updates().join("stray.tmp"), "").unwrap();
    let updater = world.launch();
    assert_eq!(state(&updater), off(OFF_PLATFORM));
    updater.check(true).await;
    assert_eq!(state(&updater), off(OFF_PLATFORM));
    assert_eq!(world.system.hits(LATEST_URL), 0);
    assert_eq!(world.update_files(), ["stray.tmp"], "no clean-up");
    updater.on_will_quit();
    assert_eq!(updater.restart_to_update().await, RestartOutcome::Nothing);
}

#[tokio::test]
async fn upd_005_development_builds_are_off_after_the_platform_check() {
    for platform in [Platform::Mac, Platform::Windows] {
        let world = quiet(platform).with_system(|system| system.packaged = false);
        assert_eq!(state(&world.launch()), off(OFF_DEVELOPMENT));
    }
    let world = quiet(Platform::Other).with_system(|system| system.packaged = false);
    assert_eq!(state(&world.launch()), off(OFF_PLATFORM));
}

#[tokio::test]
async fn upd_006_a_packaged_windows_build_is_never_off() {
    let world = quiet(Platform::Windows).with_system(|system| {
        system.writable = Some(Box::new(|_: &Path| false));
        system.in_applications = false;
    });
    assert_eq!(state(&world.launch()), UpdateState::Idle);
}

#[tokio::test]
async fn upd_007_to_009_a_mac_copy_needs_to_write_its_bundle_and_folder() {
    let bundle_only_read_only = |world: World| {
        let bundle = world.system.bundle.clone();
        world.with_system(move |system| {
            system.writable = Some(Box::new(move |path: &Path| path != bundle));
        })
    };
    let world = bundle_only_read_only(quiet(Platform::Mac));
    assert_eq!(state(&world.launch()), off(OFF_APPLICATIONS));
    let world = bundle_only_read_only(quiet(Platform::Mac)).with_system(|system| system.in_applications = false);
    assert_eq!(state(&world.launch()), off(OFF_MOVE));

    let world = quiet(Platform::Mac);
    let folder = world.system.bundle.parent().unwrap().to_owned();
    let world = world.with_system(move |system| {
        system.writable = Some(Box::new(move |path: &Path| path != folder));
    });
    assert_eq!(state(&world.launch()), off(OFF_APPLICATIONS));

    // Both writable, for real: on.
    let world = quiet(Platform::Mac);
    assert_eq!(state(&world.launch()), UpdateState::Idle);
}

#[tokio::test(start_paused = true)]
async fn upd_012_checks_30_seconds_in_then_every_4_hours_while_allowed() {
    let world = quiet(Platform::Windows);
    world.app.auto_update.store(true, Ordering::SeqCst);
    let _updater = world.launch();
    tokio::time::sleep(Duration::from_secs(29)).await;
    assert_eq!(world.system.hits(LATEST_URL), 0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(world.system.hits(LATEST_URL), 1);
    // The 4 hours count from the start, not from the first check.
    tokio::time::sleep(Duration::from_secs(4 * 3600 - 32)).await;
    assert_eq!(world.system.hits(LATEST_URL), 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(world.system.hits(LATEST_URL), 2);
    // Each tick reads the setting as it is then.
    world.app.auto_update.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(4 * 3600)).await;
    assert_eq!(world.system.hits(LATEST_URL), 2);
    world.app.auto_update.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(4 * 3600)).await;
    assert_eq!(world.system.hits(LATEST_URL), 3);
}

#[tokio::test(start_paused = true)]
async fn upd_012_no_automatic_check_while_switched_off() {
    let world = quiet(Platform::Windows);
    let _updater = world.launch();
    tokio::time::sleep(Duration::from_secs(5 * 3600)).await;
    assert_eq!(world.system.hits(LATEST_URL), 0);
}

#[tokio::test]
async fn upd_010_075_a_missing_updates_folder_is_fine() {
    let world = quiet(Platform::Mac);
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(state(&updater), UpdateState::Latest);
    assert!(world.system.runs().is_empty());
}
