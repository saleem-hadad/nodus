//! The Windows updater: an installer is downloaded, kept until the user
//! restarts into it, and run. A port of `src/main/updater.test.ts`, with the
//! same release (0.9.0 over a running 0.8.9) and installer bytes.

mod common;

use std::sync::atomic::Ordering;

use common::*;
use nodus_domain::UpdateState;
use nodus_updater::{INSTALLER_FAILED, NO_CHECKSUM, Platform, RestartOutcome, StopSessionsQuestion, Updater};

const ASSET: &str = "Nodus-Setup-x64.exe";
const DOWNLOAD_URL: &str = "https://github.com/saleem-hadad/nodus/releases/download/v0.9.0/Nodus-Setup-x64.exe";
const INSTALLER: &[u8] = b"the 0.9.0 installer";

/// GitHub: the latest release is 0.9.0, with an installer for this machine
/// unless `assets` is false.
fn serve_release(world: &World, assets: bool) {
    let asset = asset_json(ASSET, INSTALLER.len(), &digest_of(INSTALLER), DOWNLOAD_URL);
    let assets = if assets { vec![asset] } else { vec![] };
    world
        .system
        .route(latest(), Route::json(&release_json("v0.9.0", &assets)));
    world.system.route(DOWNLOAD_URL, Route::bytes(INSTALLER));
}

fn world() -> World {
    let world = World::new(Platform::Windows, "0.8.9", no_tools());
    serve_release(&world, true);
    world
}

fn downloads(world: &World) -> usize {
    world.system.hits(DOWNLOAD_URL)
}

fn installer_path(world: &World) -> std::path::PathBuf {
    world.updates().join("Nodus-Setup-0.9.0.exe")
}

/// A fresh launch, an asked check, and the update ready.
async fn staged(world: &World) -> std::sync::Arc<Updater> {
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, ready("0.9.0"));
    updater
}

/// What the tests' "running" session makes Windows ask.
fn live_session(world: &World) {
    world.app.live.store(1, Ordering::SeqCst);
}

mod the_installer_between_launches {
    use super::*;

    #[tokio::test]
    async fn is_downloaded_once_and_kept_for_the_next_launch() {
        let world = world();
        staged(&world).await;
        assert_eq!(downloads(&world), 1);
        staged(&world).await;
        assert_eq!(downloads(&world), 1);
        assert_eq!(std::fs::read(installer_path(&world)).unwrap(), INSTALLER);
    }

    #[tokio::test]
    async fn is_downloaded_again_when_the_kept_one_isnt_the_releases() {
        let world = world();
        std::fs::create_dir_all(world.updates()).unwrap();
        std::fs::write(installer_path(&world), b"half a download").unwrap();
        staged(&world).await;
        assert_eq!(downloads(&world), 1);
        assert_eq!(std::fs::read(installer_path(&world)).unwrap(), INSTALLER);
    }

    #[tokio::test]
    async fn only_the_newest_newer_one_is_kept() {
        let world = world();
        serve_release(&world, false);
        std::fs::create_dir_all(world.updates()).unwrap();
        for name in [
            "Nodus-Setup-0.8.0.exe",
            "Nodus-Setup-0.8.9.exe",
            "Nodus-Setup-0.9.0.exe",
            "Nodus-Setup-0.9.1.exe",
            "stray.tmp",
        ] {
            std::fs::write(world.updates().join(name), b"").unwrap();
        }
        let updater = world.launch();
        updater.check(true).await;
        assert_eq!(world.update_files(), ["Nodus-Setup-0.9.1.exe"]);
    }
}

mod restart_to_update_on_windows {
    use super::*;

    /// The tests' dialog: one session running, and the user's answer.
    async fn restart_and_answer(updater: &Updater, stop: bool) -> RestartOutcome {
        match updater.restart_to_update().await {
            RestartOutcome::Ask(question) => {
                assert_eq!(question, StopSessionsQuestion::new(1));
                updater.answer_stop_sessions(stop).await
            }
            other => other,
        }
    }

    #[tokio::test]
    async fn stops_the_sessions_and_quits_once_windows_has_started_the_installer() {
        let world = world();
        live_session(&world);
        let updater = staged(&world).await;
        assert_eq!(restart_and_answer(&updater, true).await, RestartOutcome::Quitting);
        assert_eq!(world.app.count("stop_all"), 1);
        // spawn(installer, ['--updated', '/S', '--force-run'], { detached: true, stdio: 'ignore' })
        assert_eq!(
            world.system.spawns(),
            [(
                installer_path(&world),
                vec!["--updated".to_owned(), "/S".to_owned(), "--force-run".to_owned()]
            )]
        );
        assert_eq!(world.app.count("quit"), 1);
        assert_eq!(*world.app.log.lock().unwrap(), ["stop_all", "spawn", "quit"]);
    }

    #[tokio::test]
    async fn stays_open_and_says_so_when_the_installer_doesnt_start() {
        let world = world();
        live_session(&world);
        let updater = staged(&world).await;
        world.system.spawn_fails.store(true, Ordering::SeqCst);
        assert_eq!(restart_and_answer(&updater, true).await, RestartOutcome::Failed);
        assert_eq!(world.app.count("quit"), 0);
        assert_eq!(state(&updater), error(INSTALLER_FAILED));
        assert!(!installer_path(&world).exists());
        // Check for Updates gets it again.
        updater.check(true).await;
        assert_eq!(prepared(&updater).await, ready("0.9.0"));
        assert_eq!(downloads(&world), 2);
    }

    #[tokio::test]
    async fn stops_nothing_when_the_installer_has_gone() {
        let world = world();
        live_session(&world);
        let updater = staged(&world).await;
        std::fs::remove_file(installer_path(&world)).unwrap();
        // No question: it fails before asking.
        assert_eq!(updater.restart_to_update().await, RestartOutcome::Failed);
        assert_eq!(world.app.count("stop_all"), 0);
        assert_eq!(world.app.count("quit"), 0);
        assert!(matches!(state(&updater), UpdateState::Error { .. }));
    }

    #[tokio::test]
    async fn leaves_the_sessions_running_when_the_user_says_not_now() {
        let world = world();
        live_session(&world);
        let updater = staged(&world).await;
        assert_eq!(restart_and_answer(&updater, false).await, RestartOutcome::NotNow);
        assert_eq!(world.app.count("stop_all"), 0);
        assert!(world.system.spawns().is_empty());
        assert_eq!(state(&updater), ready("0.9.0"));
    }

    #[tokio::test]
    async fn asks_once_when_the_button_is_clicked_twice() {
        let world = world();
        live_session(&world);
        let updater = staged(&world).await;
        let (first, second) = tokio::join!(updater.restart_to_update(), updater.restart_to_update());
        let asked = [&first, &second]
            .iter()
            .filter(|outcome| matches!(outcome, RestartOutcome::Ask(_)))
            .count();
        assert_eq!(asked, 1);
        assert!([&first, &second].contains(&&RestartOutcome::Nothing));
        assert_eq!(updater.answer_stop_sessions(true).await, RestartOutcome::Quitting);
        assert_eq!(world.system.spawns().len(), 1);
    }
}

// Beyond updater.test.ts: the rest of spec 06's Windows behavior.

#[tokio::test]
async fn stops_the_host_without_asking_when_no_session_runs() {
    let world = world();
    let updater = staged(&world).await;
    assert_eq!(updater.restart_to_update().await, RestartOutcome::Quitting);
    assert_eq!(*world.app.log.lock().unwrap(), ["stop_all", "spawn", "quit"]);
}

#[tokio::test]
async fn upd_069_a_host_that_wont_stop_fails_the_install() {
    let world = world();
    let updater = staged(&world).await;
    world.app.stop_fails.store(true, Ordering::SeqCst);
    assert_eq!(updater.restart_to_update().await, RestartOutcome::Failed);
    assert!(world.system.spawns().is_empty());
    assert_eq!(state(&updater), error(INSTALLER_FAILED));
    assert!(!installer_path(&world).exists());
    assert_eq!(updater.staged(), None);
}

#[tokio::test]
async fn upd_049_refuses_an_installer_without_a_checksum() {
    let world = world();
    let asset = asset_json(ASSET, INSTALLER.len(), "null", DOWNLOAD_URL);
    world
        .system
        .route(latest(), Route::json(&release_json("v0.9.0", &[asset])));
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, error(NO_CHECKSUM));
    assert_eq!(downloads(&world), 0);
    assert!(!installer_path(&world).exists());
}

#[tokio::test]
async fn upd_053_a_damaged_installer_is_deleted() {
    let world = world();
    world.system.route(DOWNLOAD_URL, Route::bytes(b"the 0.9.0 installeR"));
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, error(nodus_updater::DAMAGED));
    assert!(!installer_path(&world).exists());
}

#[tokio::test]
async fn upd_050_a_kept_installer_goes_straight_to_ready() {
    let world = world();
    std::fs::create_dir_all(world.updates()).unwrap();
    std::fs::write(installer_path(&world), INSTALLER).unwrap();
    staged(&world).await;
    assert_eq!(downloads(&world), 0);
    assert_eq!(
        world.states(),
        [
            UpdateState::Checking,
            UpdateState::Downloading {
                version: "0.9.0".into(),
                percent: 0
            },
            ready("0.9.0"),
        ]
    );
}

#[tokio::test]
async fn upd_070_never_installs_on_quit() {
    let world = world();
    let updater = staged(&world).await;
    updater.on_will_quit();
    assert!(world.system.spawns().is_empty());
}

#[tokio::test]
async fn an_answer_without_a_question_does_nothing() {
    let world = world();
    let updater = staged(&world).await;
    assert_eq!(updater.answer_stop_sessions(true).await, RestartOutcome::Nothing);
    assert!(world.system.spawns().is_empty());
}

#[tokio::test]
async fn upd_054_nothing_staged_nothing_happens() {
    let world = world();
    let updater = world.launch();
    assert_eq!(updater.restart_to_update().await, RestartOutcome::Nothing);
    assert!(world.app.log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upd_021_an_arm_machine_takes_the_arm64_installer() {
    let world = World::new(Platform::Windows, "0.8.9", no_tools()).with_system(|system| system.arm64 = true);
    let url = "https://github.com/saleem-hadad/nodus/releases/download/v0.9.0/Nodus-Setup-arm64.exe";
    let assets = [
        asset_json(ASSET, INSTALLER.len(), &digest_of(INSTALLER), DOWNLOAD_URL),
        asset_json("Nodus-Setup-arm64.exe", INSTALLER.len(), &digest_of(INSTALLER), url),
    ];
    world
        .system
        .route(latest(), Route::json(&release_json("v0.9.0", &assets)));
    world.system.route(url, Route::bytes(INSTALLER));
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, ready("0.9.0"));
    assert_eq!(world.system.hits(url), 1);
    assert_eq!(downloads(&world), 0);
}
