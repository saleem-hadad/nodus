//! state.json read and written as the Electron app does. The fixtures were
//! written by JSON.stringify, so a byte-for-byte match is a match with 0.9.4.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use nodus_domain::{AppSettings, ProjectGroup, ThemePreference};
use nodus_store::{BoardState, DEFAULT_ARCHIVE_W, State, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

const P1: &str = "1f0e5a3c-6b2d-4c8e-9a7f-0d1e2f3a4b5c";

/// A fixture's text. Git may have checked it out with Windows line endings.
fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// A data folder holding `text` as its state.json.
fn data_folder(text: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("state.json"), text).unwrap();
    dir
}

fn state_file(dir: &TempDir) -> PathBuf {
    dir.path().join("state.json")
}

fn saved(dir: &TempDir) -> String {
    fs::read_to_string(state_file(dir)).unwrap()
}

fn group(id: &str) -> ProjectGroup {
    ProjectGroup {
        id: id.into(),
        name: "Work".into(),
        created_at: 1.0,
        collapsed: None,
        extra: Default::default(),
    }
}

#[test]
fn writes_the_file_back_exactly_as_it_read_it() {
    let dir = data_folder(&fixture("current.json"));
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| {
        assert_eq!(state.projects.len(), 3);
        assert_eq!(state.projects[1].opened_at, Some(1_790_050_000_000.5));
        assert_eq!(state.settings.theme, ThemePreference::Dark);
        assert_eq!(state.voice_key.as_deref(), Some("djEwAAAAAAAAAAAAAAAAAAAAAAAA"));
        let board = state.board(P1).unwrap();
        // In the order they were saved, which is the order cards are drawn in.
        let order: Vec<&str> = board.nodes.keys().map(|id| &id[..8]).collect();
        assert_eq!(
            order,
            ["00000001", "00000002", "00000003", "00000004", "00000005", "00000006"]
        );
        assert_eq!(board.nodes[2].created_at, 1_789_990_000_000.781_2);
        assert_eq!(board.ink.as_ref().unwrap().len(), 3);
    });
    store.update(|_| {});
    store.flush();
    assert_eq!(saved(&dir), fixture("current.json"));
}

#[test]
fn keeps_what_a_newer_version_wrote() {
    let dir = data_folder(&fixture("newer.json"));
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| {
        assert_eq!(state.extra["schema"], 7);
        assert_eq!(state.settings.extra["sound"], "chime");
        assert_eq!(state.projects[0].extra["emoji"], "📝");
        let board = state.boards.values().next().unwrap();
        assert_eq!(board.extra["layers"][0]["name"], "Sketches");
        assert_eq!(board.nodes[0].extra["model"]["effort"], "max");
        assert_eq!(board.notes[0].extra["reminder"], 1_800_003_600_000_u64);
    });
    store.update(|_| {});
    store.flush();
    assert_eq!(saved(&dir), fixture("newer.json"));
}

#[test]
fn reads_an_older_state() {
    let dir = data_folder(&fixture("older.json"));
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| {
        assert!(state.groups.is_empty());
        assert_eq!(state.actions, None);
        assert_eq!(state.captures, None);
        // An empty key is no key.
        assert_eq!(state.voice_key, None);
        // A theme it doesn't know is the default; switches that start on stay
        // on unless they're false, and ones that start off stay off unless true.
        let settings = &state.settings;
        assert_eq!(settings.theme, ThemePreference::System);
        assert!(settings.auto_update && !settings.notify && settings.dock_badge && settings.capture);
        assert!(!settings.pin_project_rail && !settings.focus_feature && !settings.focus_mode);
        assert_eq!(settings.extra["expandNewSessions"], false);
        let action = &state.projects[0].actions.as_ref().unwrap()[0];
        assert_eq!(action.extra["allowedTools"][0], "Bash(pnpm lint)");
        let board = state.board(P1).unwrap();
        assert!(board.hidden.is_empty() && board.sections.is_empty() && board.notes.is_empty());
        assert_eq!(board.nodes[0].kind, None);
    });

    store.update(|_| {});
    store.flush();
    let saved: Value = serde_json::from_str(&saved(&dir)).unwrap();
    let keys = |value: &Value| value.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&saved), ["projects", "groups", "boards", "settings"]);
    // getBoard's additions, in the order it makes them.
    assert_eq!(
        keys(&saved["boards"][P1]),
        ["nodes", "archive", "hidden", "sections", "notes", "views"]
    );
    assert_eq!(
        saved["settings"],
        json!({
            "theme": "system", "autoUpdate": true, "notify": false, "dockBadge": true, "pinProjectRail": false,
            "focusFeature": false, "focusMode": false, "capture": true, "expandNewSessions": false
        })
    );
}

#[test]
fn reads_lists_that_are_not_lists_as_empty() {
    let dir = data_folder(
        r#"{"projects":{},"groups":"none","boards":[],"settings":[],"actions":{},"voiceKey":5,"captures":3}"#,
    );
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| assert_eq!(*state, State::default()));
}

#[test]
fn starts_empty_without_a_file() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| {
        assert_eq!(*state, State::default());
        assert_eq!(state.settings, AppSettings::default());
    });
    assert!(!state_file(&dir).exists());
}

#[test]
fn starts_empty_when_the_file_is_not_json() {
    let dir = data_folder("{\"projects\": [");
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| assert_eq!(*state, State::default()));
    // Written over on the next save, as the TypeScript does.
    assert_eq!(saved(&dir), "{\"projects\": [");
}

#[test]
fn moves_aside_a_file_it_cannot_read() {
    let text = r#"{"projects":[{"id":"p","name":"p","repoPath":"/tmp/p","color":"teal","createdAt":1}]}"#;
    let dir = data_folder(text);
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| assert_eq!(*state, State::default()));
    assert!(!state_file(&dir).exists());
    let aside: Vec<PathBuf> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(aside.len(), 1);
    assert!(
        aside[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("state.json.unreadable-")
    );
    assert_eq!(fs::read_to_string(&aside[0]).unwrap(), text);
}

#[test]
fn reads_a_title_cut_mid_emoji() {
    // JavaScript writes the half of an emoji that a slice left behind.
    let dir = data_folder(
        r#"{"captures":[{"id":"c","kind":"note","projectId":"p","title":"Fix the \ud83d","createdAt":1}]}"#,
    );
    let store = Store::open(dir.path()).unwrap();
    store.read(|state| assert_eq!(state.captures.as_ref().unwrap()[0].title, "Fix the \u{fffd}"));
}

#[test]
fn saves_a_moment_after_the_last_change() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.update(|state| state.groups.push(group("g")));
    assert!(!state_file(&dir).exists(), "saved before the delay");

    let deadline = Instant::now() + Duration::from_secs(5);
    while !state_file(&dir).exists() {
        assert!(Instant::now() < deadline, "never saved");
        thread::sleep(Duration::from_millis(20));
    }
    let saved: Value = serde_json::from_str(&saved(&dir)).unwrap();
    assert_eq!(saved["groups"][0]["id"], "g");
}

#[test]
fn flush_writes_what_is_waiting() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.update(|state| state.settings.theme = ThemePreference::Light);
    store.flush();
    let saved: Value = serde_json::from_str(&saved(&dir)).unwrap();
    assert_eq!(saved["settings"]["theme"], "light");
}

#[test]
fn flush_does_nothing_when_nothing_is_waiting() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.flush();
    assert!(!state_file(&dir).exists());
}

#[test]
fn dropping_the_store_writes_what_is_waiting() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.update(|state| state.groups.push(group("g")));
    drop(store);
    let saved: Value = serde_json::from_str(&saved(&dir)).unwrap();
    assert_eq!(saved["groups"][0]["id"], "g");
}

#[test]
fn writes_through_a_temporary_file() {
    let original = fixture("current.json");
    let dir = data_folder(&original);
    let store = Store::open(dir.path()).unwrap();

    store.update(|state| state.groups.clear());
    store.flush();
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["state.json"]);

    // When the temporary file can't be written, state.json is left as it was.
    let before = saved(&dir);
    fs::create_dir(dir.path().join(format!("state.json.{}.tmp", std::process::id()))).unwrap();
    store.update(|state| state.projects.clear());
    store.flush();
    assert_eq!(saved(&dir), before);
}

#[test]
fn takes_changes_from_many_threads() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let writers: Vec<_> = (0..8)
        .map(|t| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for i in 0..50 {
                    store.update(|state| state.groups.push(group(&format!("{t}-{i}"))));
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    store.flush();
    store.read(|state| assert_eq!(state.groups.len(), 400));
    let saved: Value = serde_json::from_str(&saved(&dir)).unwrap();
    assert_eq!(saved["groups"].as_array().unwrap().len(), 400);
}

#[test]
fn makes_a_board_as_get_board_does() {
    let mut state = State::default();
    let board = state.board_mut("unknown-project");
    assert_eq!(board.archive.w, DEFAULT_ARCHIVE_W);
    assert_eq!(
        nodus_domain::json::to_string(board).unwrap(),
        r#"{"nodes":{},"archive":{"x":0,"y":0,"w":1928,"h":320},"hidden":[],"sections":{},"notes":{},"views":{}}"#
    );
    // It's kept, to be saved with the rest.
    assert!(state.boards.contains_key("unknown-project"));
    assert_eq!(*state.board_mut("unknown-project"), BoardState::default());
}

#[test]
fn finds_projects_groups_sessions_and_views() {
    let state = State::from_json(serde_json::from_str(&fixture("current.json")).unwrap()).unwrap();
    assert_eq!(state.project(P1).unwrap().name, "shop");
    assert_eq!(state.project("nope").unwrap_err().to_string(), "Unknown project nope");
    assert_eq!(
        state.group("9c8b7a6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d").unwrap().name,
        "Work"
    );
    assert_eq!(state.group("nope").unwrap_err().to_string(), "Unknown group nope");
    assert_eq!(
        state.project_id_for_session("00000002-aaaa-4bbb-8ccc-dddddddddddd"),
        Some(P1)
    );
    assert_eq!(
        state.project_id_for_session("00000007-aaaa-4bbb-8ccc-dddddddddddd"),
        None
    );
    assert_eq!(
        state.project_id_for_view("708192a3-b4c5-4d6e-8f70-8192a3b4c5d6"),
        Some(P1)
    );
    assert_eq!(state.project_id_for_view("nope"), None);
}
