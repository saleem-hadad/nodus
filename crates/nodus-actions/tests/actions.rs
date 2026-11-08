//! The actions:* commands against a real store, drafting against a stand-in
//! `claude`, and the takeover rules a run is ended or kept by.

use std::sync::Arc;

use nodus_actions::{ActionError, Actions, DraftError, draft_with, is_run};
use nodus_domain::{Project, SessionStatus};
use nodus_store::Store;
use serde_json::{Value, json};
use tempfile::TempDir;

fn project(id: &str, actions: Value) -> Project {
    let mut raw = json!({ "id": id, "name": "Demo", "repoPath": "/work/demo", "color": "blue", "createdAt": 1 });
    if !actions.is_null() {
        raw["actions"] = actions;
    }
    serde_json::from_value(raw).unwrap()
}

fn store_with(dir: &TempDir, projects: Vec<Project>) -> Arc<Store> {
    let store = Store::open(dir.path()).unwrap();
    store.update(|state| state.projects = projects);
    Arc::new(store)
}

fn ids(list: &[nodus_domain::ProjectAction]) -> Vec<&str> {
    list.iter().map(|a| a.id.as_str()).collect()
}

#[test]
fn saves_actions_in_their_scope_and_moves_them_when_it_changes() {
    let dir = TempDir::new().unwrap();
    let store = store_with(&dir, vec![project("p1", Value::Null), project("p2", Value::Null)]);
    let actions = Actions::new(store.clone());
    // ACT-013
    assert!(actions.global().is_empty());

    // ACT-014: a project action, appended.
    let lists = actions
        .save_at(
            "p1",
            &json!({ "id": "a", "name": "Tests", "prompt": "Run the tests." }),
            10.0,
        )
        .unwrap();
    assert_eq!(ids(lists.project.actions.as_deref().unwrap()), ["a"]);
    assert!(lists.global.is_empty());
    actions
        .save_at("p1", &json!({ "id": "b", "prompt": "B." }), 11.0)
        .unwrap();

    // Edited in place, keeping its creation time.
    let lists = actions
        .save_at(
            "p1",
            &json!({ "id": "a", "name": "Run Tests", "prompt": "Again." }),
            20.0,
        )
        .unwrap();
    let own = lists.project.actions.unwrap();
    assert_eq!(ids(&own), ["a", "b"]);
    assert_eq!((own[0].name.as_str(), own[0].created_at), ("Run Tests", 10.0));

    // Turned global: out of the project, into the global list.
    let lists = actions
        .save_at("p1", &json!({ "id": "a", "prompt": "Again.", "global": true }), 30.0)
        .unwrap();
    assert_eq!(ids(lists.project.actions.as_deref().unwrap()), ["b"]);
    assert_eq!(ids(&lists.global), ["a"]);
    assert_eq!(lists.global[0].created_at, 10.0);
    assert_eq!(ids(&actions.global()), ["a"]);

    // Turned off from another project: it moves into that one.
    let lists = actions
        .save_at("p2", &json!({ "id": "a", "prompt": "Again." }), 40.0)
        .unwrap();
    assert_eq!(ids(lists.project.actions.as_deref().unwrap()), ["a"]);
    // ACT-002: an empty list is left out, not written as [].
    assert_eq!(lists.global, []);
    store.read(|state| {
        assert_eq!(state.actions, None);
        assert_eq!(ids(state.project("p1").unwrap().actions.as_deref().unwrap()), ["b"]);
    });

    // ACT-015: removing takes it from both lists.
    actions
        .save_at("p1", &json!({ "id": "g", "prompt": "G.", "global": true }), 50.0)
        .unwrap();
    let lists = actions.remove("p2", "g").unwrap();
    assert_eq!(lists.global, []);
    let lists = actions.remove("p1", "b").unwrap();
    assert_eq!(lists.project.actions, None);
    store.flush();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(store.file()).unwrap()).unwrap();
    assert!(saved.get("actions").is_none());
    assert!(saved["projects"][0].get("actions").is_none());
    assert_eq!(saved["projects"][1]["actions"][0]["id"], "a");
}

#[test]
fn refuses_unknown_projects_and_actions_without_a_prompt() {
    let dir = TempDir::new().unwrap();
    let actions = Actions::new(store_with(&dir, vec![project("p1", Value::Null)]));
    assert_eq!(
        actions.save("nope", &json!({ "prompt": "x" })).unwrap_err().to_string(),
        "Unknown project nope"
    );
    assert_eq!(
        actions.remove("nope", "a").unwrap_err().to_string(),
        "Unknown project nope"
    );
    assert_eq!(
        actions.save("p1", &json!({ "name": "x" })).unwrap_err(),
        ActionError::NoPrompt
    );
    assert_eq!(actions.save("p1", &Value::Null).unwrap_err(), ActionError::NoPrompt);
}

#[test]
fn knows_which_sessions_are_runs() {
    let dir = TempDir::new().unwrap();
    let store = store_with(&dir, vec![project("p1", Value::Null)]);
    store.update(|state| {
        let board = state.board_mut("p1");
        for (id, run_of) in [("run", json!("a")), ("plain", Value::Null), ("blank", json!(""))] {
            let node = serde_json::from_value(
                json!({ "sessionId": id, "x": 0, "y": 0, "archived": false, "createdAt": 0, "runOf": run_of }),
            )
            .unwrap();
            board.nodes.insert(id.into(), node);
        }
    });
    store.read(|state| {
        assert!(is_run(state, "run"));
        assert!(!is_run(state, "plain"));
        assert!(!is_run(state, "blank"));
        assert!(!is_run(state, "missing"));
    });
}

/// ACT-040, ACT-041: what the terminal sends into a run, and whether it's the
/// user stepping in (nodus-sessions' `launch::is_takeover`, which
/// `session:write` checks before claiming the run).
#[test]
fn tells_a_takeover_from_the_terminal_talking() {
    use SessionStatus::{Done, Waiting, Working};
    use nodus_sessions::launch::is_takeover;
    let cases = [
        // Interrupts, always.
        ("\x1b", Waiting, true),
        ("\x03", Waiting, true),
        // Typing and Return, unless the run waits on them.
        ("y", Working, true),
        ("\r", Done, true),
        ("y", Waiting, false),
        // Kept quirks: an SS3 arrow leaves its letter, an X10 mouse report
        // its bytes, and a bracketed paste its text.
        ("\x1bOA", Working, true),
        ("\x1b[M !!", Working, true),
        ("\x1b[200~hi\x1b[201~", Working, true),
        // Focus reports, SGR mouse, colour and cursor replies, Tab and
        // Backspace alone aren't.
        ("\x1b[I", Working, false),
        ("\x1b[O", Working, false),
        ("\x1b[<0;10;5M", Working, false),
        ("\x1b]11;rgb:1e1e/1e1e/1e1e\x07", Working, false),
        ("\x1b]10;rgb:ffff/ffff/ffff\x1b\\", Working, false),
        ("\x1b[12;40R", Working, false),
        ("\t", Working, false),
        ("\x7f", Working, false),
    ];
    for (data, status, takeover) in cases {
        assert_eq!(is_takeover(data, status), takeover, "{data:?} while {status}");
    }
}

#[tokio::test]
async fn asks_for_a_description_before_looking_for_claude() {
    let dir = TempDir::new().unwrap();
    let err = draft_with(None, Default::default(), dir.path(), " \n ", false)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Describe what the button should do");
    let err = draft_with(None, Default::default(), dir.path(), "x", false)
        .await
        .unwrap_err();
    assert!(matches!(err, DraftError::NoClaude(_)));
    assert!(err.to_string().starts_with("Claude Code CLI not found. Install it ("));
}

#[tokio::test]
async fn says_when_claude_cant_start() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("claude");
    let err = draft_with(Some(&missing), Default::default(), dir.path(), "x", false)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), format!("spawn {} ENOENT", missing.display()));
}

#[cfg(unix)]
mod stand_in {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// A `claude` that records how it was run and answers as told.
    fn claude(dir: &Path, answer: &str) -> std::path::PathBuf {
        let path = dir.join("claude");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$LOG.args\"\npwd > \"$LOG.cwd\"\nif read -r line; then echo open; else echo closed; fi > \"$LOG.stdin\"\n{answer}\n"
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn env(dir: &Path) -> nodus_platform::env::EnvMap {
        [
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("LOG".to_owned(), dir.join("log").display().to_string()),
        ]
        .into()
    }

    #[tokio::test]
    async fn drafts_with_sonnet_reading_the_repository() {
        let dir = TempDir::new().unwrap();
        let repo = TempDir::new().unwrap();
        let answer = r#"echo '{"type":"result","is_error":false,"result":"","structured_output":{"name":"Run Tests","icon":"test-tube","prompt":"Run the tests."}}'"#;
        let claude = claude(dir.path(), answer);
        let draft = draft_with(Some(&claude), env(dir.path()), repo.path(), "  run the tests  ", true)
            .await
            .unwrap();
        assert_eq!(
            (draft.name.as_str(), draft.icon.as_str(), draft.prompt.as_str()),
            ("Run Tests", "test-tube", "Run the tests.")
        );
        let args = std::fs::read_to_string(dir.path().join("log.args")).unwrap();
        let args: Vec<&str> = args.trim_end_matches('\0').split('\0').collect();
        assert_eq!(args, nodus_actions::draft_args("run the tests", true));
        let cwd = std::fs::read_to_string(dir.path().join("log.cwd")).unwrap();
        assert_eq!(
            std::fs::canonicalize(cwd.trim()).unwrap(),
            std::fs::canonicalize(repo.path()).unwrap()
        );
        // ACT-023: stdin is closed, so `claude -p` doesn't wait for it.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("log.stdin")).unwrap(),
            "closed\n"
        );
    }

    #[tokio::test]
    async fn passes_claudes_reason_on() {
        let dir = TempDir::new().unwrap();
        let noisy = claude(
            dir.path(),
            "echo 'Error: one' >&2; echo two >&2; echo three >&2; echo four >&2; exit 1",
        );
        let err = draft_with(Some(&noisy), env(dir.path()), dir.path(), "x", false)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "two three four");
        let quiet = claude(dir.path(), "exit 3");
        let err = draft_with(Some(&quiet), env(dir.path()), dir.path(), "x", false)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "claude exited with code 3");
    }

    #[tokio::test(start_paused = true)]
    async fn stops_a_draft_after_two_minutes() {
        // ACT-024: the paused clock runs ahead to the timeout.
        let dir = TempDir::new().unwrap();
        let claude = claude(dir.path(), "sleep 30");
        let err = draft_with(Some(&claude), env(dir.path()), dir.path(), "x", false)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Drafting the button took too long");
    }
}
