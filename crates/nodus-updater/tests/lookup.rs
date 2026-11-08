//! Checking GitHub and downloading (UPD-015…UPD-036), on the Windows path
//! so no tools are needed. Not ports: 0.9.4 tests only the installer flow.

mod common;

use std::time::Duration;

use common::*;
use nodus_domain::UpdateState;
use nodus_updater::{DAMAGED, INTERRUPTED, LATEST_URL, Platform, RATE_LIMITED, UNREACHABLE};

const URL: &str = "https://github.com/saleem-hadad/nodus/releases/download/v0.9.0/Nodus-Setup-x64.exe";
const INSTALLER: &[u8] = b"0123456789abcdefghij";
/// Long enough for the paused clock to pass the 60 s stall.
const MINUTES: Duration = Duration::from_secs(120);

fn world() -> World {
    let world = World::new(Platform::Windows, "0.8.9", no_tools());
    serve(&world, "v0.9.0", INSTALLER.len());
    world.system.route(URL, Route::bytes(INSTALLER));
    world
}

fn serve(world: &World, tag: &str, size: usize) {
    let asset = asset_json("Nodus-Setup-x64.exe", size, &digest_of(INSTALLER), URL);
    world
        .system
        .route(LATEST_URL, Route::json(&release_json(tag, &[asset])));
}

async fn checked(world: &World) -> UpdateState {
    let updater = world.launch();
    updater.check(false).await;
    state(&updater)
}

#[tokio::test]
async fn upd_016_asks_github_as_nodus_without_a_token() {
    let world = world();
    let updater = world.launch();
    updater.check(true).await;
    prepared(&updater).await;
    let requests = world.system.requests.lock().unwrap().clone();
    assert_eq!(
        requests,
        [
            (
                LATEST_URL.to_owned(),
                vec![
                    ("Accept", "application/vnd.github+json".to_owned()),
                    ("User-Agent", "Nodus/0.8.9".to_owned())
                ]
            ),
            // UPD-030: the download says who it is, and nothing else.
            (URL.to_owned(), vec![("User-Agent", "Nodus/0.8.9".to_owned())]),
        ]
    );
    assert_eq!(world.states()[0], UpdateState::Checking);
}

#[tokio::test]
async fn upd_017_says_when_github_cant_be_reached() {
    let world = world();
    world.system.route(LATEST_URL, Route::Unreachable);
    assert_eq!(checked(&world).await, error(UNREACHABLE));
}

#[tokio::test(start_paused = true)]
async fn upd_017_gives_up_on_github_after_20_seconds() {
    let world = world();
    world.system.route(LATEST_URL, Route::Hang);
    let updater = world.launch();
    let started = tokio::time::Instant::now();
    updater.check(false).await;
    assert_eq!(state(&updater), error(UNREACHABLE));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(20) && waited < Duration::from_secs(21),
        "{waited:?}"
    );
    // The body counts too.
    world.system.route(LATEST_URL, Route::Stall(vec![b"{\"tag".to_vec()]));
    updater.check(false).await;
    assert_eq!(state(&updater), error(UNREACHABLE));
}

#[tokio::test]
async fn upd_018_says_when_github_is_limiting_checks() {
    for status in [403, 429] {
        let world = world();
        world.system.route(
            LATEST_URL,
            Route::Answer {
                status,
                chunks: vec![b"{}".to_vec()],
            },
        );
        assert_eq!(checked(&world).await, error(RATE_LIMITED));
    }
}

#[tokio::test]
async fn upd_019_says_which_error_github_answered() {
    for status in [404, 500, 502] {
        let world = world();
        world.system.route(LATEST_URL, Route::Answer { status, chunks: vec![] });
        let expected = format!("GitHub answered the update check with an error ({status}).");
        assert_eq!(checked(&world).await, error(&expected));
    }
}

#[tokio::test]
async fn an_answer_that_isnt_json_shows_the_parsers_words() {
    let world = world();
    world.system.route(LATEST_URL, Route::json("<!DOCTYPE html>"));
    let UpdateState::Error { message } = checked(&world).await else {
        panic!("an error");
    };
    assert!(message.starts_with("expected value"), "{message}");
}

#[tokio::test]
async fn upd_023_026_nothing_newer_or_nothing_for_this_machine_is_latest() {
    for tag in ["v0.8.9", "v0.8.0", "0.8.9", "v1.0.0-beta.1"] {
        let world = world();
        serve(&world, tag, INSTALLER.len());
        assert_eq!(checked(&world).await, UpdateState::Latest, "{tag}");
        assert_eq!(world.system.hits(URL), 0);
    }
    let world = world();
    world
        .system
        .route(LATEST_URL, Route::json(&release_json("v0.9.0", &[])));
    assert_eq!(checked(&world).await, UpdateState::Latest);
}

#[tokio::test]
async fn upd_027_028_a_version_that_failed_waits_to_be_asked_for() {
    let world = world();
    std::fs::create_dir_all(world.updates()).unwrap();
    std::fs::write(world.updates().join("failed"), "0.9.0\n").unwrap();
    let updater = world.launch();
    updater.check(false).await;
    assert_eq!(
        state(&updater),
        error("Nodus 0.9.0 downloaded but couldn’t install itself.")
    );
    assert_eq!(world.system.hits(URL), 0);
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, ready("0.9.0"));
}

#[tokio::test]
async fn upd_015_029_a_check_ends_with_the_lookup_and_none_runs_alongside() {
    let world = world();
    world.system.route(URL, Route::Stall(vec![INSTALLER[..5].to_vec()]));
    let updater = world.launch();
    updater.check(true).await;
    let downloading = settle(&updater, |state| {
        matches!(state, UpdateState::Downloading { percent: 25, .. })
    })
    .await;
    assert_eq!(
        downloading,
        UpdateState::Downloading {
            version: "0.9.0".into(),
            percent: 25
        }
    );
    // Busy: another check does nothing.
    updater.check(true).await;
    assert_eq!(world.system.hits(LATEST_URL), 1);
}

#[tokio::test]
async fn upd_015_a_staged_update_stops_further_checks() {
    let world = world();
    let updater = world.launch();
    updater.check(true).await;
    prepared(&updater).await;
    updater.check(true).await;
    updater.check(false).await;
    assert_eq!(world.system.hits(LATEST_URL), 1);
    assert_eq!(state(&updater), ready("0.9.0"));
}

#[tokio::test]
async fn upd_032_reports_whole_percents_as_they_change() {
    let world = world();
    let chunks: Vec<Vec<u8>> = INSTALLER.chunks(5).map(<[u8]>::to_vec).collect();
    world.system.route(URL, Route::Answer { status: 200, chunks });
    let updater = world.launch();
    updater.check(true).await;
    prepared(&updater).await;
    let percents: Vec<u8> = world
        .states()
        .into_iter()
        .filter_map(|state| match state {
            UpdateState::Downloading { percent, .. } => Some(percent),
            _ => None,
        })
        .collect();
    assert_eq!(percents, [0, 25, 50, 75, 100]);

    // A chunk that doesn't move the percent says nothing.
    let world = World::new(Platform::Windows, "0.8.9", no_tools());
    let big = vec![7u8; 1000];
    let asset = asset_json("Nodus-Setup-x64.exe", big.len(), &digest_of(&big), URL);
    world
        .system
        .route(LATEST_URL, Route::json(&release_json("v0.9.0", &[asset])));
    let chunks = vec![
        big[..1].to_vec(),
        big[1..2].to_vec(),
        big[2..600].to_vec(),
        big[600..].to_vec(),
    ];
    world.system.route(URL, Route::Answer { status: 200, chunks });
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, ready("0.9.0"));
    let percents: Vec<u8> = world
        .states()
        .into_iter()
        .filter_map(|state| match state {
            UpdateState::Downloading { percent, .. } => Some(percent),
            _ => None,
        })
        .collect();
    assert_eq!(percents, [0, 60, 100]);
}

#[tokio::test]
async fn upd_033_says_which_error_the_download_got() {
    let world = world();
    world.system.route(
        URL,
        Route::Answer {
            status: 500,
            chunks: vec![],
        },
    );
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(
        prepared(&updater).await,
        error("GitHub answered the download with an error (500).")
    );
    let world = self::world();
    world.system.route(URL, Route::NoBody(200));
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(
        prepared(&updater).await,
        error("GitHub answered the download with an error (200).")
    );
}

#[tokio::test]
async fn upd_034_a_download_of_the_wrong_size_or_digest_is_damaged() {
    let world = world();
    world.system.route(URL, Route::bytes(&INSTALLER[..19]));
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, error(DAMAGED));
    let world = self::world();
    world.system.route(URL, Route::bytes(b"0123456789abcdefghiJ"));
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, error(DAMAGED));
}

#[tokio::test]
async fn upd_035_a_broken_connection_didnt_finish() {
    let world = world();
    world.system.route(URL, Route::Unreachable);
    let updater = world.launch();
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, error(INTERRUPTED));
}

#[tokio::test(start_paused = true)]
async fn upd_031_a_download_that_stalls_for_60_seconds_is_given_up() {
    let world = world();
    world.system.route(URL, Route::Stall(vec![INSTALLER[..5].to_vec()]));
    let updater = world.launch();
    updater.check(true).await;
    let started = tokio::time::Instant::now();
    assert_eq!(prepared_within(&updater, MINUTES).await, error(INTERRUPTED));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(60) && waited < Duration::from_secs(61),
        "{waited:?}"
    );
    // Before any answer at all, too.
    world.system.route(URL, Route::Hang);
    updater.check(true).await;
    assert_eq!(prepared_within(&updater, MINUTES).await, error(INTERRUPTED));
}
