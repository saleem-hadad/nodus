//! The whole app, headless, through `nodus-core`: a scratch data folder, a
//! fake home with synthetic transcripts, the real Rust session host (run in
//! this process, as `Nodus host` would run it), and a stand-in `claude` on
//! the PATH that answers `--version`, logs its arguments and environment,
//! turns on bracketed paste and echoes what it reads. The real claude is
//! never run. Every host and process a test starts is stopped before it ends.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_core::domain::{
    CreateSessionOptions, CreateViewOptions, Event, Project, SessionKind, SessionStatus, ViewEvent,
};
use nodus_core::{AppShell, BoxFuture, EnvSource, HostLauncher, Nodus, NodusConfig, ReplaceChoice};
use nodus_host_proto::HostPaths;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

const WAIT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// The world a test runs in.

/// Runs the Rust session host in this process, as `Nodus host` would run
/// it, so the test can wait for it to exit.
#[derive(Default)]
struct InProcessHost {
    started: Mutex<Vec<JoinHandle<i32>>>,
}

impl HostLauncher for InProcessHost {
    fn start(&self, paths: &HostPaths) -> std::io::Result<()> {
        nodus_host_proto::ensure_private_dir(&paths.dir)?;
        let (log, _lines) = nodus_host::Logger::memory();
        let args = [
            paths.socket.to_string_lossy().into_owned(),
            paths.key.to_string_lossy().into_owned(),
        ];
        let host = tokio::spawn(async move { nodus_host::run(&args, log).await });
        self.started.lock().unwrap().push(host);
        Ok(())
    }
}

impl InProcessHost {
    fn count(&self) -> usize {
        self.started.lock().unwrap().len()
    }

    /// Waits for every host started to exit, and gives their exit codes.
    async fn exited(&self) -> Vec<i32> {
        let hosts: Vec<JoinHandle<i32>> = std::mem::take(&mut *self.started.lock().unwrap());
        let mut codes = Vec::new();
        for host in hosts {
            codes.push(tokio::time::timeout(WAIT, host).await.expect("the host exits").unwrap());
        }
        codes
    }
}

/// A window that's closed, counting quits.
#[derive(Default)]
struct Shell {
    quits: AtomicUsize,
}

impl AppShell for Shell {
    fn is_open(&self) -> bool {
        false
    }
    fn is_focused(&self) -> bool {
        false
    }
    fn is_loading(&self) -> bool {
        false
    }
    fn show(&self) {}
    fn quit(&self) {
        self.quits.fetch_add(1, Ordering::SeqCst);
    }
    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        Box::pin(async { ReplaceChoice::Quit })
    }
    fn exit_now(&self) {
        panic!("a test never quits at once");
    }
}

struct World {
    dir: tempfile::TempDir,
    home: PathBuf,
    user_data: PathBuf,
    host: Arc<InProcessHost>,
    shell: Arc<Shell>,
    env: BTreeMap<String, String>,
}

impl World {
    async fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nodus-core-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        let bin = root.join("bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        write_stand_in(&bin.join("claude"), &root.join("log"));
        std::fs::create_dir_all(root.join("log")).unwrap();
        let env: BTreeMap<String, String> = [
            ("PATH", format!("{}:/usr/bin:/bin", bin.display())),
            ("HOME", home.display().to_string()),
            ("SHELL", "/bin/sh".to_owned()),
            ("LANG", "en_US.UTF-8".to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        Self {
            user_data: root.join("userData"),
            home,
            dir,
            host: Arc::default(),
            shell: Arc::default(),
            env,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    /// The app, started and launched as index.ts does it.
    async fn start(&self) -> Nodus {
        let env = nodus_platform::Env::with_vars(self.env.clone(), Some(&self.home)).await;
        let nodus = Nodus::start(NodusConfig {
            env: EnvSource::Given(env),
            home: self.home.clone(),
            shell: Some(Arc::clone(&self.shell) as _),
            host_launcher: Some(Arc::clone(&self.host) as _),
            ..NodusConfig::new(&self.user_data, tokio::runtime::Handle::current())
        })
        .unwrap();
        nodus.launch().await.unwrap();
        nodus
    }

    /// A git repository to add as a project.
    async fn repo(&self, name: &str) -> PathBuf {
        let repo = self.root().join(name);
        std::fs::create_dir_all(&repo).unwrap();
        let _ = tokio::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .await;
        repo
    }

    /// Each time the stand-in ran for a session, its arguments.
    fn argvs(&self, session_id: &str) -> Vec<Vec<String>> {
        let mut runs = Vec::new();
        for entry in std::fs::read_dir(self.root().join("log")).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with(&format!("argv.{session_id}.")) {
                let text = std::fs::read_to_string(&path).unwrap();
                let mut args: Vec<String> = text.split('\0').map(str::to_owned).collect();
                args.pop();
                runs.push(args);
            }
        }
        runs
    }

    /// The environment the stand-in ran in for a session, as `NAME=value` lines.
    fn env_of(&self, session_id: &str) -> BTreeMap<String, String> {
        let text = std::fs::read_to_string(self.root().join("log").join(format!("env.{session_id}"))).unwrap();
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    /// Stops everything: Quit and Stop Sessions, then waits for the host to go.
    async fn stop(&self, nodus: Nodus) {
        nodus.menu().quit_and_stop_sessions().await;
        nodus.shutdown();
        for code in self.host.exited().await {
            assert_eq!(code, 0);
        }
    }
}

/// The stand-in `claude`: `--version` answers at once; otherwise it logs its
/// arguments (each ended by NUL) and environment by session, turns on
/// bracketed paste, says it's ready and echoes every line it reads.
fn write_stand_in(file: &Path, log: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "9.9.9 (Claude Code)"; exit 0; fi
for a in "$@"; do printf '%s\000' "$a"; done > "{log}/argv.$NODUS_SESSION.$$"
env > "{log}/env.$NODUS_SESSION"
printf '\033[?2004h'
echo "stand-in ready"
while IFS= read -r line; do echo "got: $line"; done
"#,
        log = log.display()
    );
    std::fs::write(file, script).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// ---------------------------------------------------------------------------
// Events.

struct Events {
    rx: broadcast::Receiver<Event>,
    /// Output so far, by session.
    output: BTreeMap<String, String>,
    seen: Vec<Event>,
}

impl Events {
    fn new(nodus: &Nodus) -> Self {
        Self {
            rx: nodus.subscribe(),
            output: BTreeMap::new(),
            seen: Vec::new(),
        }
    }

    async fn next(&mut self) -> Event {
        let event = tokio::time::timeout(WAIT, self.rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no event in time; seen {:#?}", self.seen))
            .unwrap();
        if let Event::PtyData { session_id, data } = &event {
            self.output.entry(session_id.clone()).or_default().push_str(data);
        }
        self.seen.push(event.clone());
        event
    }

    /// The first event, from now, that `wanted` picks.
    async fn until(&mut self, wanted: impl Fn(&Event) -> bool) -> Event {
        loop {
            let event = self.next().await;
            if wanted(&event) {
                return event;
            }
        }
    }

    /// Waits until a session has printed `text`.
    async fn printed(&mut self, session_id: &str, text: &str) {
        while !self.output.get(session_id).is_some_and(|out| out.contains(text)) {
            self.next().await;
        }
    }

    async fn status(&mut self, session_id: &str, status: SessionStatus, live: bool) {
        let want = Event::Status {
            session_id: session_id.to_owned(),
            status,
            live,
        };
        self.until(|e| *e == want).await;
    }
}

// ---------------------------------------------------------------------------
// The session host's HTTP side, as claude's hooks and MCP client reach it.

async fn post(port: u16, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = String::new();
    tokio::time::timeout(WAIT, stream.read_to_string(&mut answer))
        .await
        .expect("an answer")
        .unwrap();
    let status = answer[9..12].parse().unwrap();
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

/// Where a session's hooks post, and their token, from its environment.
fn hook_target(env: &BTreeMap<String, String>) -> (u16, String) {
    (env["NODUS_PORT"].parse().unwrap(), env["NODUS_TOKEN"].clone())
}

async fn hook(port: u16, token: &str, session_id: &str, event: &str, payload: &Value) {
    let (status, _) = post(
        port,
        &format!("/hook?evt={event}&sid={session_id}"),
        &[("Authorization", &format!("Bearer {token}"))],
        &payload.to_string(),
    )
    .await;
    assert_eq!(status, 204);
}

async fn mcp(port: u16, token: &str, session_id: &str, message: &Value) -> Value {
    let (status, body) = post(
        port,
        "/mcp",
        &[
            ("Authorization", &format!("Bearer {token}")),
            ("X-Nodus-Session", session_id),
        ],
        &message.to_string(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

// ---------------------------------------------------------------------------
// Transcripts on disk.

fn transcript_line(kind: &str, text: &str, second: u32) -> String {
    let at = format!("2026-11-01T10:00:{second:02}.000Z");
    let line = if kind == "user" {
        json!({ "type": "user", "timestamp": at, "gitBranch": "main", "message": { "role": "user", "content": text } })
    } else {
        json!({ "type": "assistant", "timestamp": at, "message": { "id": format!("m{second}"), "content": [{ "type": "text", "text": text }] } })
    };
    format!("{line}\n")
}

fn write_transcript(home: &Path, project: &Project, session_id: &str, lines: &[String]) -> PathBuf {
    let dir = nodus_transcripts::transcript_dir(home, &project.repo_path);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{session_id}.jsonl"));
    std::fs::write(&file, lines.concat()).unwrap();
    file
}

fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2).any(|w| w[0] == flag && w[1] == value)
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_run_in_the_host_and_hooks_drive_their_status() {
    let world = World::new().await;
    let nodus = world.start().await;
    let info = nodus.app().info().await;
    assert_eq!(info.claude_version.as_deref(), Some("9.9.9 (Claude Code)"));
    assert!(info.claude_path.unwrap().ends_with("/bin/claude"));
    // No host was running, and the launch starts none (PLAT-010 step 8).
    assert_eq!(world.host.count(), 0);

    let repo = world.repo("shop").await;
    let project = nodus
        .projects()
        .add(Some(repo.to_string_lossy().into_owned()))
        .await
        .unwrap()
        .unwrap();
    let mut events = Events::new(&nodus);

    // A terminal card: a login shell in the project folder, through a host started for it (SESS-131).
    let terminal = nodus
        .sessions()
        .create(
            &project.id,
            &CreateSessionOptions {
                x: 0.0,
                y: 0.0,
                kind: Some(SessionKind::Terminal),
                ..CreateSessionOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(world.host.count(), 1);
    assert!(terminal.live);
    assert_eq!(terminal.node.kind, Some(SessionKind::Terminal));
    let terminal_id = terminal.node.session_id.clone();

    // A Claude card with a first prompt: working at once (SESS-129).
    let card = nodus
        .sessions()
        .create(
            &project.id,
            &CreateSessionOptions {
                x: 320.0,
                y: 0.0,
                name: Some("  Fix login  ".into()),
                initial_prompt: Some(" -look at the login form ".into()),
                ..CreateSessionOptions::default()
            },
        )
        .await
        .unwrap();
    let id = card.node.session_id.clone();
    assert_eq!(card.node.title.as_deref(), Some("Fix login"));
    assert!(card.live && card.status == SessionStatus::Working);
    events.status(&id, SessionStatus::Working, true).await;
    events.printed(&id, "stand-in ready").await;

    // SESS-121…124: its command line, with the hooks and the board tools the host relays.
    let argv = &world.argvs(&id)[0];
    assert!(has_pair(argv, "--session-id", &id), "{argv:?}");
    assert!(has_pair(argv, "--name", "Fix login"));
    assert!(has_pair(argv, "--permission-mode", "acceptEdits"));
    assert!(has_pair(argv, "--mcp-config", nodus_tools::BOARD_MCP_CONFIG));
    assert_eq!(
        argv[argv.len() - 2..],
        ["--".to_owned(), "-look at the login form".to_owned()]
    );
    let settings = &argv[argv.iter().position(|a| a == "--settings").unwrap() + 1];
    assert!(settings.contains("\"statusLine\""));
    let env = world.env_of(&id);
    assert_eq!(env["NODUS_SESSION"], id);
    assert!(env.contains_key("NODUS_HOOK_CONFIG"));
    assert_eq!(env["TERM"], "xterm-256color");
    let (port, token) = hook_target(&env);

    // HOST-216…219: what's running, by project.
    let stats = nodus.projects().stats().await;
    assert_eq!(stats[&project.id].running.sessions, 1);
    assert_eq!(stats[&project.id].running.terminals, 1);
    assert_eq!(stats[&project.id].working, 1);

    // Typing goes to the process (HOST-223, SESS-138).
    nodus.sessions().write(&id, "typed\r");
    events.printed(&id, "got: typed").await;

    // A live session opens resized, with its output to replay (HOST-222, SESS-135).
    let opened = nodus.sessions().open(&project.id, &id, 100.0, 30.0).await.unwrap();
    assert!(opened.live);
    assert!(opened.replay.contains("stand-in ready"), "{:?}", opened.replay);
    nodus.sessions().resize(&id, 90.0, 25.0);

    // Hooks posted to the host move its status (HOST-132…159, HOST-068…078).
    hook(port, &token, &id, "Stop", &json!({})).await;
    events.status(&id, SessionStatus::Done, true).await;
    hook(port, &token, &id, "UserPromptSubmit", &json!({ "prompt": "go" })).await;
    events.status(&id, SessionStatus::Working, true).await;
    hook(port, &token, &id, "PermissionRequest", &json!({})).await;
    events.status(&id, SessionStatus::Waiting, true).await;
    // SESS-143: waiting on a question, a prompt sent from the board is refused.
    let err = nodus.sessions().send(&project.id, &id, "more").await.unwrap_err();
    assert_eq!(
        err.ipc_message(),
        "Error invoking remote method 'session:send': Error: It’s waiting for your answer. Open it to reply."
    );
    hook(port, &token, &id, "Stop", &json!({})).await;
    events.status(&id, SessionStatus::Done, true).await;
    // The status line, as usage (HOST-166…173).
    hook(
        port,
        &token,
        &id,
        "StatusLine",
        &json!({ "model": { "display_name": "Opus 5.5" }, "cost": { "total_cost_usd": 0.25 } }),
    )
    .await;
    let usage = events.until(|e| matches!(e, Event::Usage { .. })).await;
    let Event::Usage { session_id, usage } = usage else {
        unreachable!()
    };
    assert_eq!(session_id, id);
    assert_eq!(usage.model.as_deref(), Some("Opus 5.5"));
    assert_eq!(usage.cost_usd, Some(0.25));

    // A prompt sent to a live session is pasted, then sent (SESS-144).
    nodus
        .sessions()
        .send(&project.id, &id, "  second prompt ")
        .await
        .unwrap();
    events.printed(&id, "second prompt\x1b[201~").await;

    // The board tools, through the host's MCP endpoint (TOOL-014, NOTE-001…).
    let note = mcp(
        port,
        &token,
        &id,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                 "params": { "name": "create_note", "arguments": { "text": "Remember the milk" } } }),
    )
    .await;
    assert_eq!(note["id"], 1);
    assert!(note["result"]["isError"].is_null(), "{note}");
    events
        .until(|e| {
            *e == Event::BoardChanged {
                project_id: project.id.clone(),
            }
        })
        .await;
    let listed = mcp(
        port,
        &token,
        &id,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "list_notes", "arguments": {} } }),
    )
    .await;
    assert!(listed.to_string().contains("Remember the milk"), "{listed}");
    let board = nodus.board().load(&project.id).await.unwrap();
    assert_eq!(board.notes.len(), 1);
    assert_eq!(board.notes[0].text, "Remember the milk");
    assert_eq!(board.notes[0].from.as_deref(), Some(id.as_str()));

    // Killing it waits for the exit, then it reads as done and gone (HOST-224, SESS-148).
    // The status machine hears of the exit before the window does (pty.ts's `exited`).
    nodus.sessions().kill(&id).await;
    events.status(&id, SessionStatus::Done, false).await;
    events
        .until(|e| matches!(e, Event::PtyExit { session_id, .. } if *session_id == id))
        .await;
    let stats = nodus.projects().stats().await;
    assert_eq!(stats[&project.id].running.sessions, 0);
    assert_eq!(stats[&project.id].running.terminals, 1);

    // Sent a prompt while stopped, it starts again with it (SESS-145): no
    // transcript yet, so as a new session with the same id.
    events.output.remove(&id);
    nodus.sessions().send(&project.id, &id, "again").await.unwrap();
    events.printed(&id, "stand-in ready").await;
    let runs = world.argvs(&id);
    assert_eq!(runs.len(), 2);
    let again = runs
        .iter()
        .find(|args| args.last().map(String::as_str) == Some("again"))
        .unwrap();
    assert!(has_pair(again, "--session-id", &id));
    assert!(!again.contains(&"--name".to_owned()));

    // SESS-153: stopping waits for every process to exit.
    nodus.sessions().stop(&[id.clone(), terminal_id.clone()]).await;
    let stats = nodus.projects().stats().await;
    assert_eq!(stats[&project.id].running, Default::default());

    // Opening a stopped terminal starts a fresh shell at its size (SESS-136).
    let reopened = nodus
        .sessions()
        .open(&project.id, &terminal_id, 80.0, 24.0)
        .await
        .unwrap();
    assert_eq!((reopened.replay.as_str(), reopened.live), ("", true));
    assert_eq!(nodus.projects().stats().await[&project.id].running.terminals, 1);

    world.stop(nodus).await;
    assert_eq!(world.shell.quits.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_board_merges_the_transcripts_on_disk_and_follows_them() {
    let world = World::new().await;
    let nodus = world.start().await;
    let repo = world.repo("docs").await;
    let project = nodus
        .projects()
        .add(Some(repo.to_string_lossy().into_owned()))
        .await
        .unwrap()
        .unwrap();
    let old = "00000000-0000-4000-8000-0000000000a1";
    let file = write_transcript(
        &world.home,
        &project,
        old,
        &[
            transcript_line("user", "Fix the login", 1),
            transcript_line("assistant", "Done, the form validates now", 2),
        ],
    );
    let mut events = Events::new(&nodus);

    // SESS-072…081: the session on disk is imported into the archive.
    let board = nodus.board().load(&project.id).await.unwrap();
    assert_eq!(board.cards.len(), 1);
    let card = &board.cards[0];
    assert_eq!(card.node.session_id, old);
    assert!(card.node.archived && !card.live && card.status == SessionStatus::Done);
    let meta = card.meta.as_ref().unwrap();
    assert_eq!(meta.title, "Fix the login");
    assert_eq!(
        meta.last_assistant_text.as_deref(),
        Some("Done, the form validates now")
    );
    assert!(nodus.projects().list().await[0].opened_at.is_some());
    assert_eq!(nodus.projects().stats().await[&project.id].total, 1);

    // SESS-162, SESS-175: its conversation, and search.
    let said = nodus.sessions().transcript(&project.id, old).await.unwrap();
    assert_eq!(said.len(), 2);
    let hits = nodus.sessions().search(&project.id, "validates").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].session_id, old);
    // SESS-163
    let err = nodus
        .sessions()
        .agent_transcript(&project.id, old, "../x")
        .await
        .unwrap_err();
    assert_eq!(err.message, "Not an agent of this session");
    assert!(nodus.sessions().agents(&project.id, old).await.unwrap().is_empty());

    // The folder is watched: the transcript growing is a board change
    // (SESS-063…069). The OS's file events start a moment after the load.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let mut grown = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
    std::io::Write::write_all(&mut grown, transcript_line("user", "And the signup form", 3).as_bytes()).unwrap();
    drop(grown);
    events
        .until(|e| {
            *e == Event::BoardChanged {
                project_id: project.id.clone(),
            }
        })
        .await;

    // The board's own changes are kept (SESS-090…106).
    nodus
        .board()
        .save_viewport(
            &project.id,
            nodus_core::domain::Viewport {
                x: 1.5,
                y: 2.0,
                zoom: 0.75,
            },
        )
        .await;
    let board = nodus.board().load(&project.id).await.unwrap();
    assert_eq!(board.viewport.unwrap().zoom, 0.75);
    // SESS-107: a removed card's session is hidden for good.
    nodus.board().remove_node(&project.id, old).await;
    assert!(nodus.board().load(&project.id).await.unwrap().cards.is_empty());

    // Unknown projects, in 0.9.4's words.
    let err = nodus.board().load("nope").await.unwrap_err();
    assert_eq!(
        err.ipc_message(),
        "Error invoking remote method 'board:load': Error: Unknown project nope"
    );

    // Removing the project stops watching it (PLAT-075).
    nodus.projects().remove(&project.id).await;
    world.stop(nodus).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_view_starts_its_brain_and_a_relaunch_adopts_the_host() {
    let world = World::new().await;
    let nodus = world.start().await;
    let repo = world.repo("stats").await;
    let project = nodus
        .projects()
        .add(Some(repo.to_string_lossy().into_owned()))
        .await
        .unwrap()
        .unwrap();
    let mut events = Events::new(&nodus);

    // VIEW-100, SESS-134: a view, and its brain on building it.
    let view = nodus
        .views()
        .create(
            &project.id,
            &CreateViewOptions {
                description: "Show the open issues".into(),
                x: 100.4,
                y: 200.0,
            },
        )
        .await
        .unwrap();
    assert_eq!((view.title.as_str(), view.x, view.version), ("New view", 100.0, 0));
    let brain = view.session_id.clone().expect("a brain");
    events
        .until(|e| {
            *e == Event::BoardChanged {
                project_id: project.id.clone(),
            }
        })
        .await;
    events.printed(&brain, "stand-in ready").await;
    let argv = &world.argvs(&brain)[0];
    assert!(has_pair(argv, "--session-id", &brain), "{argv:?}");
    assert!(has_pair(argv, "--permission-mode", "auto"));
    let system = &argv[argv.iter().position(|a| a == "--append-system-prompt").unwrap() + 1];
    assert!(system.contains("Show the open issues") && system.contains(&view.id));
    assert_eq!(argv[argv.len() - 1], "Build this view:\n\nShow the open issues");
    assert!(!argv.contains(&"--name".to_owned()));
    // views:brain gives the brain it has.
    assert_eq!(nodus.views().brain(&project.id, &view.id).await.unwrap(), brain);
    // A brain is running in the background, not as a card (SESS-160).
    let stats = nodus.projects().stats().await;
    assert_eq!(stats[&project.id].running.background, 1);
    assert_eq!(stats[&project.id].active, 0);

    // The page its frame loads, and what the page asks for.
    let page = nodus.views().serve_page(&format!("nodus-view://{}/", view.id));
    assert_eq!(page.status, 200);
    assert!(page.body.contains("window.nodus"));
    let saved = nodus
        .views()
        .call(&project.id, &view.id, "setState", &json!({ "state": { "tab": 2 } }))
        .await
        .unwrap();
    assert_eq!(saved, Value::Null);
    let told = events
        .until(|e| {
            matches!(
                e,
                Event::View {
                    event: ViewEvent::State,
                    ..
                }
            )
        })
        .await;
    assert_eq!(
        told,
        Event::View {
            project_id: project.id.clone(),
            view_id: view.id.clone(),
            event: ViewEvent::State,
            payload: json!({ "tab": 2 }),
        }
    );
    let err = nodus
        .views()
        .call(&project.id, "missing", "sessions", &Value::Null)
        .await
        .unwrap_err();
    assert_eq!(err.channel, "views:call");

    // Quitting lets go of the host; its sessions keep running (PLAT-033, HOST-111).
    nodus.shutdown();
    drop(nodus);
    drop(events);

    // The next launch adopts them, with what happened in them (HOST-105…108).
    let nodus = world.start().await;
    assert_eq!(world.host.count(), 1, "adopted, not started again");
    let opened = nodus.sessions().open(&project.id, &brain, 80.0, 24.0).await.unwrap();
    assert!(opened.live);
    assert!(opened.replay.contains("stand-in ready"), "{:?}", opened.replay);
    let board = nodus.board().load(&project.id).await.unwrap();
    let card = board.cards.iter().find(|c| c.node.session_id == brain).unwrap();
    assert!(card.live && card.status == SessionStatus::Working);
    assert_eq!(nodus.projects().stats().await[&project.id].running.background, 1);

    // Removing the view takes its brain with it (VIEW-117).
    let mut events = Events::new(&nodus);
    nodus.views().remove(&project.id, &view.id).await.unwrap();
    events
        .until(|e| matches!(e, Event::PtyExit { session_id, .. } if *session_id == brain))
        .await;
    let board = nodus.board().load(&project.id).await.unwrap();
    assert!(board.views.is_empty() && board.cards.is_empty());

    world.stop(nodus).await;
}
