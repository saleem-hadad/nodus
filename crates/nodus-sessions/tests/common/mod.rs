//! What the fixture replays share: the steps fixtures/generate.mjs runs
//! through the TypeScript, run here through the Rust.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{Event, EventSink, SessionStatus};
use nodus_sessions::hooks::StatusMachine;
use serde_json::{Value, json};

pub const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
}

/// Every event and answer in the order they came, as the generator writes them.
#[derive(Default)]
pub struct Log(Mutex<Vec<Value>>);

impl Log {
    pub fn push(&self, entry: Value) {
        self.0.lock().unwrap().push(entry);
    }

    pub fn take(&self) -> Vec<Value> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl EventSink for Log {
    fn emit(&self, event: Event) {
        self.push(match event {
            Event::Status {
                session_id,
                status,
                live,
            } => json!(["status", session_id, status, live]),
            Event::AgentStatus {
                session_id,
                agent_id,
                status,
            } => json!(["agent", session_id, agent_id, status]),
            Event::Usage { session_id, usage } => json!(["usage", session_id, usage]),
            other => panic!("unexpected event {other:?}"),
        });
    }
}

/// Lets timers that are due run, as the paused clock reaches them.
pub async fn advance(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// Runs one of the status machine's steps; false when it isn't one.
pub async fn status_step(machine: &StatusMachine, log: &Log, pending: &AtomicBool, step: &Value) -> bool {
    let at = step.get("at").and_then(Value::as_f64);
    let text = |value: &Value| value.as_str().unwrap().to_owned();
    if let Some(id) = step.get("start") {
        let initial = step
            .get("initial")
            .map_or(SessionStatus::Done, |s| serde_json::from_value(s.clone()).unwrap());
        machine.process_started(&text(id), initial);
    } else if let Some(id) = step.get("exit") {
        machine.process_exited(&text(id));
    } else if let Some(event) = step.get("hook") {
        machine.receive_hook(&text(event), &text(&step["session"]), &step["payload"], at);
    } else if let Some(data) = step.get("input") {
        machine.note_user_input(&text(&step["session"]), &text(data), at);
    } else if let Some(ms) = step.get("advance") {
        advance(ms.as_u64().unwrap()).await;
    } else if let Some(id) = step.get("forget") {
        machine.forget(&text(id));
    } else if let Some(ids) = step.get("settle") {
        machine.settle_agent(&text(&ids[0]), &text(&ids[1]));
    } else if let Some(ids) = step.get("hold") {
        machine.hold_agent(&text(&ids[0]), &text(&ids[1]));
    } else if let Some(value) = step.get("pending") {
        pending.store(value.as_bool().unwrap(), Ordering::SeqCst);
    } else if let Some(id) = step.get("status") {
        log.push(json!(["getStatus", id, machine.status(&text(id))]));
    } else if let Some(ids) = step.get("agent") {
        let hook = machine
            .agent_hook(&text(&ids[0]), &text(&ids[1]))
            .map(|hook| json!({"status": hook.status, "at": hook.at, "since": hook.since}));
        log.push(json!(["getAgentHook", ids[0], ids[1], hook]));
    } else if let Some(id) = step.get("usage") {
        log.push(json!(["getUsage", id, machine.usage(&text(id))]));
    } else {
        return false;
    }
    true
}

/// Each entry as JavaScript would write it, to compare byte for byte.
pub fn lines(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| nodus_domain::json::to_string(entry).unwrap())
        .collect()
}

/// A paused runtime, as vi.useFakeTimers gives the TypeScript's tests.
pub fn paused_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap()
}

/// A shared flag for `hasPendingWork` to answer with.
pub fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
