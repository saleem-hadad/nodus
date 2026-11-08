//! Subagents read from their transcripts and meta files, replaying what the
//! real agents.ts did with the same files (fixtures/generate.mjs).

mod common;

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, UNIX_EPOCH};

use nodus_domain::{SessionCard, SessionStatus};
use nodus_sessions::agents::{Agents, list_agent_files};
use nodus_sessions::hooks::{StatusMachine, TokioClock};
use serde_json::{Map, Value, json};

fn subagents_dir(home: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join("-repo")
        .join(session_id)
        .join("subagents")
}

fn set_mtime(file: &Path, ms: f64) {
    let handle = File::options().write(true).open(file).unwrap();
    handle
        .set_modified(UNIX_EPOCH + Duration::from_millis(ms as u64))
        .unwrap();
}

async fn replay(home: &Path, start: f64, steps: &[Value]) -> Vec<Value> {
    let dir = subagents_dir(home, "S");
    fs::create_dir_all(&dir).unwrap();
    let dir_text = dir.to_str().unwrap().to_owned();
    let with_dir = |text: &str| text.replace("$DIR", &dir_text);

    let clock = Arc::new(TokioClock::starting_at(start));
    let agents = Agents::new(home, clock.clone());
    let log = Arc::new(common::Log::default());
    let pending = common::flag();
    let machine = StatusMachine::new(log.clone(), clock, {
        let agents = agents.clone();
        move |file: &str| agents.has_pending_work(file)
    });

    for step in steps {
        if let Some(files) = step.get("files") {
            for (name, file) in files.as_object().unwrap() {
                let target = dir.join(name);
                if file.get("append").is_some() {
                    let mut handle = File::options().append(true).open(&target).unwrap();
                    handle.write_all(file["content"].as_str().unwrap().as_bytes()).unwrap();
                } else if file.get("same").is_none() {
                    fs::write(&target, file["content"].as_str().unwrap()).unwrap();
                }
                set_mtime(&target, file["mtime"].as_f64().unwrap());
            }
        } else if let Some(session) = step.get("read") {
            let session = session.as_str().unwrap();
            let found = agents.read_agents(&machine, session, &list_agent_files(&subagents_dir(home, session)));
            log.push(json!(["agents", found]));
        } else if let Some(path) = step.get("pendingWork") {
            let found = agents.has_pending_work(&with_dir(path.as_str().unwrap()));
            log.push(json!(["pendingWork", path, found]));
        } else if let Some(card) = step.get("attach") {
            let mut card: SessionCard = serde_json::from_value(card.clone()).unwrap();
            let files = list_agent_files(&subagents_dir(home, &card.node.session_id));
            agents.attach_agents(
                &machine,
                &mut card,
                &files,
                step.get("lastPromptAt").and_then(Value::as_f64),
            );
            // JSON.stringify leaves out what's undefined.
            let mut written = Map::new();
            if let Some(count) = card.agent_count {
                written.insert("agentCount".into(), json!(count));
            }
            if let Some(found) = card.agents {
                written.insert("agents".into(), json!(found));
            }
            log.push(json!(["card", written]));
        } else {
            let mut step = step.clone();
            if let Some(path) = step.pointer_mut("/payload/agent_transcript_path") {
                *path = json!(with_dir(path.as_str().unwrap()));
            }
            assert!(
                common::status_step(&machine, &log, &pending, &step).await,
                "unknown step {step}"
            );
        }
    }
    assert!(!pending.load(Ordering::SeqCst));
    log.take()
}

#[test]
fn agents_read_their_files_as_agents_ts_did() {
    // HOST-183 to HOST-199
    let fixture: Value = serde_json::from_str(&common::fixture("agents.json")).unwrap();
    let home = tempfile::tempdir().unwrap();
    let got = common::paused_runtime().block_on(replay(
        home.path(),
        fixture["start"].as_f64().unwrap(),
        fixture["steps"].as_array().unwrap(),
    ));
    let expected = fixture["results"].as_array().unwrap();
    let (got, expected) = (common::lines(&got), common::lines(expected));
    for (i, (got, expected)) in got.iter().zip(&expected).enumerate() {
        assert_eq!(got, expected, "entry {i}");
    }
    assert_eq!(got.len(), expected.len());
}

#[tokio::test(start_paused = true)]
async fn an_agent_of_a_session_that_isnt_running_is_done() {
    // HOST-193, HOST-194
    let home = tempfile::tempdir().unwrap();
    let dir = subagents_dir(home.path(), "S");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("agent-a1.jsonl"),
        "{\"type\":\"user\",\"timestamp\":\"2025-06-01T10:00:00.000Z\",\"message\":{\"content\":\"go\"}}\n",
    )
    .unwrap();
    let clock = Arc::new(TokioClock::starting_at(1_700_000_000_000.0));
    let agents = Agents::new(home.path(), clock.clone());
    let machine = StatusMachine::new(Arc::new(common::Log::default()), clock, |_: &str| false);
    let files = list_agent_files(&dir);
    assert_eq!(agents.read_agents(&machine, "S", &files)[0].status, SessionStatus::Done);
    // Live, but no hook has spoken of it in this process.
    machine.process_started("S", SessionStatus::Done);
    assert_eq!(agents.read_agents(&machine, "S", &files)[0].status, SessionStatus::Done);
    machine.receive_hook("SubagentStart", "S", &json!({"agent_id": "a1"}), None);
    assert_eq!(
        agents.read_agents(&machine, "S", &files)[0].status,
        SessionStatus::Working
    );
}

#[tokio::test(start_paused = true)]
async fn a_line_longer_than_a_chunk_is_read_whole() {
    // HOST-185: a single line over 4 MiB widens the read until it fits.
    let home = tempfile::tempdir().unwrap();
    let dir = subagents_dir(home.path(), "S");
    fs::create_dir_all(&dir).unwrap();
    let long = "x".repeat(9 * 1024 * 1024);
    let line = json!({"type": "assistant", "timestamp": "2025-06-01T10:00:00.000Z", "message": {"content": [{"type": "text", "text": long}]}});
    let tool = json!({"type": "assistant", "timestamp": "2025-06-01T10:00:01.000Z", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "ls"}}]}});
    fs::write(dir.join("agent-a1.jsonl"), format!("{line}\n{tool}\n{{\"partial\":")).unwrap();
    let clock = Arc::new(TokioClock::starting_at(0.0));
    let agents = Agents::new(home.path(), clock.clone());
    let machine = StatusMachine::new(Arc::new(common::Log::default()), clock, |_: &str| false);
    let found = agents.read_agents(&machine, "S", &list_agent_files(&dir));
    assert_eq!(found[0].tool.as_deref(), Some("Bash"));
    assert_eq!(found[0].tool_count, 1);
    assert_eq!(found[0].started_at, 1_748_772_000_000.0);
    assert_eq!(found[0].updated_at, 1_748_772_001_000.0);
}
