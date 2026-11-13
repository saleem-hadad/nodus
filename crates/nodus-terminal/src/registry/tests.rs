//! The registry against registry.ts itself: `tests/fixtures/registry.json`
//! holds scenarios generate.mjs ran against the real registry.ts with a
//! stand-in terminal and session API, and every call it made, in order. Each
//! is replayed here with a fake `Sessions` writing to the same log.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nodus_domain::SessionKind;
use serde_json::Value;
use tokio::sync::oneshot;

use super::*;

type Log = Arc<Mutex<Vec<Vec<String>>>>;

type Reply = oneshot::Sender<Result<String, String>>;

struct FakeSessions {
    log: Log,
    opens: Mutex<Vec<Option<Reply>>>,
}

impl FakeSessions {
    fn push(&self, entry: &[&str]) {
        self.log
            .lock()
            .unwrap()
            .push(entry.iter().map(|s| (*s).to_owned()).collect());
    }
}

impl Sessions for FakeSessions {
    fn open(&self, project_id: &str, session_id: &str, cols: usize, rows: usize) -> OpenFuture {
        self.push(&["open", project_id, session_id, &cols.to_string(), &rows.to_string()]);
        let (tx, rx) = oneshot::channel();
        self.opens.lock().unwrap().push(Some(tx));
        Box::pin(async move { rx.await.unwrap_or_else(|_| Err("never answered".to_owned())) })
    }

    fn write(&self, session_id: &str, data: &str) {
        self.push(&["write", session_id, data]);
    }

    fn resize(&self, session_id: &str, cols: usize, rows: usize) {
        self.push(&["resize", session_id, &cols.to_string(), &rows.to_string()]);
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

async fn run(scenario: &Value) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let platform = match scenario["platform"].as_str().unwrap() {
        "mac" => Platform::Mac,
        _ => Platform::Windows,
    };
    let log: Log = Arc::default();
    let sessions = Arc::new(FakeSessions {
        log: log.clone(),
        opens: Mutex::new(Vec::new()),
    });
    let registry = Registry::new(sessions.clone(), platform, tokio::runtime::Handle::current());
    *registry.trace.lock().unwrap() = Some(log.clone());
    let mut containers: HashMap<String, (ContainerId, Fit)> = HashMap::new();
    let mut shown: HashMap<String, String> = HashMap::new();
    let mut listeners: HashMap<String, (String, ListenerId)> = HashMap::new();
    let push = |entry: Vec<String>| log.lock().unwrap().push(entry);

    for step in scenario["steps"].as_array().unwrap() {
        let op = step["op"].as_str().unwrap();
        push(vec!["step".into(), op.into()]);
        let session = step["session"].as_str().unwrap_or_default().to_owned();
        let container_named = |name: &str, containers: &mut HashMap<String, (ContainerId, Fit)>| {
            let n = containers.len() as u64 + 1;
            *containers.entry(name.to_owned()).or_insert((ContainerId(n), None))
        };
        match op {
            "container" => {
                let name = step["name"].as_str().unwrap();
                let (id, _) = container_named(name, &mut containers);
                let size = step["size"]
                    .as_array()
                    .map(|s| (s[0].as_u64().unwrap() as usize, s[1].as_u64().unwrap() as usize));
                containers.insert(name.to_owned(), (id, size));
            }
            "attach" => {
                let name = step["container"].as_str().unwrap();
                let (id, fit) = container_named(name, &mut containers);
                shown.insert(session.clone(), name.to_owned());
                let kind = match step["kind"].as_str() {
                    Some("terminal") => SessionKind::Terminal,
                    _ => SessionKind::Claude,
                };
                let project = step["project"].as_str().unwrap_or("p1").to_owned();
                let attach_id = text(&step["id"]);
                let (registry, log) = (registry.clone(), log.clone());
                tokio::spawn(async move {
                    let entry = match registry.attach(&session, &project, id, kind, fit).await {
                        Ok(()) => vec!["attached".to_owned(), attach_id],
                        Err(message) => vec!["attach-failed".to_owned(), attach_id, message],
                    };
                    log.lock().unwrap().push(entry);
                });
            }
            "detach" => {
                let container = step["container"]
                    .as_str()
                    .map(|n| container_named(n, &mut containers).0);
                if container.is_none() || shown.get(&session).map(|n| containers[n].0) == container {
                    shown.remove(&session);
                }
                registry.detach(&session, container);
            }
            "dispose" => {
                shown.remove(&session);
                registry.dispose(&session);
            }
            "refit" => {
                if let Some(name) = shown.get(&session) {
                    let (id, fit) = containers[name];
                    registry.refit(&session, id, fit);
                }
            }
            "focus" => registry.focus(&session),
            "pty-data" => registry.pty_data(&session, step["data"].as_str().unwrap()),
            "pty-exit" => registry.pty_exit(&session),
            "resolve-open" | "reject-open" => {
                let n = step["n"].as_u64().unwrap() as usize;
                let tx = sessions.opens.lock().unwrap()[n].take().unwrap();
                let reply = if op == "resolve-open" {
                    Ok(step["replay"].as_str().unwrap().to_owned())
                } else {
                    Err(step["message"].as_str().unwrap().to_owned())
                };
                let _ = tx.send(reply);
            }
            "type-in" => registry.type_in(
                &session,
                step["text"].as_str().unwrap(),
                step["starting"].as_bool().unwrap_or(false),
            ),
            "input" => registry.input(&session, step["data"].as_str().unwrap()),
            "drop" => {
                let paths: Vec<&str> = step["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap())
                    .collect();
                registry.drop_paths(&session, &paths);
            }
            "advance" => {
                settle().await;
                tokio::time::advance(Duration::from_millis(step["ms"].as_u64().unwrap())).await;
            }
            "check" => push(vec![
                "state".into(),
                session.clone(),
                registry.needs_open(&session).to_string(),
                registry.has_output(&session).to_string(),
            ]),
            "listen" => {
                let id = text(&step["id"]);
                let (log, label) = (log.clone(), id.clone());
                if let Some(l) = registry.on_output(&session, move || {
                    log.lock().unwrap().push(vec!["output".into(), label.clone()]);
                }) {
                    listeners.insert(id, (session.clone(), l));
                }
            }
            "unlisten" => {
                if let Some((s, l)) = listeners.remove(&text(&step["id"])) {
                    registry.off_output(&s, l);
                }
            }
            other => panic!("unknown step {other}"),
        }
        settle().await;
    }
    let expected = scenario["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_array().unwrap().iter().map(text).collect())
        .collect();
    let got = log.lock().unwrap().clone();
    (got, expected)
}

#[tokio::test(start_paused = true)]
async fn registry_scenarios_match_registry_ts() {
    let path = format!("{}/tests/fixtures/registry.json", env!("CARGO_MANIFEST_DIR"));
    let scenarios: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut failures = Vec::new();
    for scenario in scenarios.as_array().unwrap() {
        let (got, expected) = run(scenario).await;
        if got != expected {
            let name = scenario["name"].as_str().unwrap();
            let show = |log: &[Vec<String>]| log.iter().map(|e| format!("    {e:?}")).collect::<Vec<_>>().join("\n");
            failures.push(format!(
                "{name}\n  got:\n{}\n  registry.ts:\n{}",
                show(&got),
                show(&expected)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn term_015_exit_lines() {
    assert_eq!(
        exit_line(SessionKind::Claude),
        "\r\n\x1b[2m[process exited · press Resume or reopen the card to continue]\x1b[0m\r\n"
    );
    assert_eq!(
        exit_line(SessionKind::Terminal),
        "\r\n\x1b[2m[shell exited · press Restart or reopen the card for a new shell]\x1b[0m\r\n"
    );
}

/// SRV-028: a server session whose output stopped while its server was away
/// is opened again, and its terminal starts over from the replay; one that
/// isn't ready is left for its next attach.
#[tokio::test]
async fn reopens_a_ready_session_from_its_replay() {
    let log: Log = Arc::default();
    let sessions = Arc::new(FakeSessions {
        log: log.clone(),
        opens: Mutex::new(Vec::new()),
    });
    let registry = Registry::new(sessions.clone(), Platform::Mac, tokio::runtime::Handle::current());
    let answer = |n: usize, replay: &str| {
        let reply = sessions.opens.lock().unwrap()[n].take().unwrap();
        reply.send(Ok(replay.to_owned())).unwrap();
    };
    let screen = |registry: &Registry| {
        registry
            .with_terminal("s1", |t| t.line_text(0, true, 0, None))
            .unwrap_or_default()
    };

    // Nothing to open again before there's a terminal.
    registry.reopen("s1", "p1").await.unwrap();
    assert!(sessions.opens.lock().unwrap().is_empty());

    let attach = {
        let registry = registry.clone();
        tokio::spawn(async move {
            registry
                .attach("s1", "p1", ContainerId(1), SessionKind::Claude, Some((80, 24)))
                .await
        })
    };
    settle().await;
    answer(0, "before");
    attach.await.unwrap().unwrap();
    registry.pty_data("s1", " more");
    assert!(screen(&registry).contains("before more"));

    let reopen = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.reopen("s1", "p1").await })
    };
    settle().await;
    // While it's opening, live output is covered by the replay (TERM-009).
    registry.pty_data("s1", " lost");
    answer(1, "after");
    reopen.await.unwrap().unwrap();
    let text = screen(&registry);
    assert!(
        text.contains("after") && !text.contains("before") && !text.contains("lost"),
        "{text}"
    );
    let opens: Vec<_> = log.lock().unwrap().iter().filter(|e| e[0] == "open").cloned().collect();
    assert_eq!(opens.len(), 2);
    assert_eq!(opens[1][1..3], ["p1".to_owned(), "s1".to_owned()]);

    // An exited one waits for its next attach.
    registry.pty_exit("s1");
    registry.reopen("s1", "p1").await.unwrap();
    assert_eq!(sessions.opens.lock().unwrap().len(), 2);
}
