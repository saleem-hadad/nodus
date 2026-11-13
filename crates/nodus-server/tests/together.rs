//! Working together on one server board (slice 6): presence (SRV-180), who
//! sent each prompt (SRV-142), the terminal following whoever typed last
//! (SRV-144), who allowed a view's command (SRV-145, SRV-210) and ink saved
//! a stroke at a time (SRV-184).
//!
//! The real binary on loopback, as `serve.rs` runs it, with two members
//! signed in already: Octo and Alice, each with a device token of their
//! own. The session host is this binary too, and `claude` a stand-in that
//! echoes each line it reads with its terminal's size.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nodus_domain::{
    BoardPoint, BoardSnapshot, CreatedBy, Event, InkStroke, Presence, Project, SessionCard, TranscriptMessage,
    Viewport, presence_color,
};
use nodus_wire::client::{Client, Events};
use nodus_wire::{AuditLine, Member, PresenceFrame, Request};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const DEV_TOKEN: &str = "a-dev-token";
const OCTO_TOKEN: &str = "octo-device-token";
const ALICE_TOKEN: &str = "alice-device-token";

// ---------------------------------------------------------------------------
// The world a test runs in.

struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nodus-tog-")
            .tempdir_in("/tmp")
            .unwrap();
        let world = Self { dir };
        let root = world.root();
        for folder in ["home", "bin", "log", "data", "shop", "sock"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        stand_in(&root.join("bin/claude"), &root.join("log"));
        std::fs::write(
            world.data().join("env"),
            format!(
                "PATH={}/bin:/usr/bin:/bin\nHOME=/nowhere\nSHELL=/bin/sh\nLANG=en_US.UTF-8\n",
                root.display()
            ),
        )
        .unwrap();
        world.people();
        world
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn data(&self) -> PathBuf {
        self.root().join("data")
    }

    /// Octo and Alice, signed in already, each on a device of their own.
    fn people(&self) {
        let private = |name: &str, text: &str| {
            use std::os::unix::fs::PermissionsExt;
            let file = self.data().join(name);
            std::fs::write(&file, text).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        };
        private("server.json", r#"{"name":"Box","setup":[]}"#);
        let members = [(1, "octo", "Octo Cat"), (100, "alice", "Alice")].map(|(github_id, login, name)| Member {
            github_id,
            login: login.into(),
            name: Some(name.into()),
            avatar_url: None,
            added_by: None,
            added_at: 1.0,
        });
        private("members.json", &serde_json::to_string(&members).unwrap());
        let device = |id: &str, token: &str, github_id: u64| {
            json!({
                "id": id, "tokenHash": sha256_hex(token), "githubId": github_id, "name": id,
                "createdAt": 1, "lastSeenAt": 1
            })
        };
        private(
            "devices.json",
            &json!([device("d-octo", OCTO_TOKEN, 1), device("d-alice", ALICE_TOKEN, 100)]).to_string(),
        );
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nodus-server"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.root().join("home"))
            .env("NODUS_USER_DATA", self.data());
        command
    }

    /// The session host on its own, as `nodus-host.service` runs it, then
    /// `serve` on loopback connected to it, with a dev token for adding the
    /// project.
    fn serve(&self) -> Server {
        let socket = self.root().join("sock/host.sock");
        let host = self
            .command()
            .args(["host", "--socket"])
            .arg(&socket)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = self
            .command()
            .args(["serve", "--bind", "127.0.0.1:0", "--dev-token", DEV_TOKEN, "--run-home"])
            .arg(self.root().join("home"))
            .arg("--host-socket")
            .arg(&socket)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let (listening, url) = std::sync::mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        let kept = Arc::clone(&output);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(at) = line.find("listening http://") {
                    let _ = listening.send(line[at + "listening ".len()..].to_owned());
                }
                kept.lock().unwrap().push_str(&line);
                kept.lock().unwrap().push('\n');
            }
        });
        let url = url
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("the server didn't listen: {}", output.lock().unwrap()));
        Server { child, host, url }
    }

    /// The project everyone works on: a folder on the server.
    async fn shop(&self, server: &Server) -> Project {
        let folder = self.root().join("shop");
        git(&folder, &["init", "-q"]);
        let (client, _events) = server.connect(DEV_TOKEN).await;
        client
            .call_as(&Request::ProjectsAdd {
                repo_path: folder.to_string_lossy().into_owned(),
                name: Some("shop".into()),
            })
            .await
            .unwrap()
    }

    /// The environment a session's stand-in found: its hook port and token.
    async fn session_env(&self, session_id: &str) -> BTreeMap<String, String> {
        let file = self.root().join(format!("log/env.{session_id}"));
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(text) = std::fs::read_to_string(&file)
                && text.contains("NODUS_TOKEN")
            {
                return text
                    .lines()
                    .filter_map(|line| line.split_once('='))
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect();
            }
            assert!(Instant::now() < deadline, "the session never started");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for World {
    /// The session host outlives its server, as it should; here it goes too.
    fn drop(&mut self) {
        let _ = Command::new("pkill")
            .args(["-f", &self.root().to_string_lossy()])
            .status();
    }
}

/// The stand-in `claude`: echoes each line it reads with its terminal's
/// size, as `stty size` says it (rows, then columns).
fn stand_in(file: &Path, log: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "9.9.9 (Claude Code)"; exit 0; fi
env > "{log}/env.$NODUS_SESSION"
printf '\033[?2004h'
echo "stand-in ready"
while IFS= read -r line; do echo "got: $line size: $(stty size)"; done
"#,
        log = log.display()
    );
    std::fs::write(file, script).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

fn sha256_hex(text: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Server {
    child: Child,
    host: Child,
    url: String,
}

impl Server {
    async fn connect(&self, token: &str) -> (Client, Events) {
        Client::connect(&self.url, token).await.unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        for child in [&mut self.child, &mut self.host] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers.

/// The first event from now that `wanted` picks.
async fn until(events: &mut Events, wanted: impl Fn(&Event) -> bool) -> Event {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next())
            .await
            .expect("the event in time")
            .expect("the connection open");
        if wanted(&event) {
            return event;
        }
    }
}

/// The next `ev:presence` for `project` that `wanted` picks.
async fn presence(events: &mut Events, project: &str, wanted: impl Fn(&Presence) -> bool) -> Presence {
    let Event::Presence(presence) = until(
        events,
        |e| matches!(e, Event::Presence(p) if p.project == project && wanted(p)),
    )
    .await
    else {
        unreachable!()
    };
    presence
}

fn logins(presence: &Presence) -> Vec<&str> {
    presence.peers.iter().map(|p| p.login.as_str()).collect()
}

/// Says where on the board `client` is.
fn on_board(client: &Client, project: &str, x: f64) {
    let frame = PresenceFrame {
        project: Some(project.into()),
        cursor: Some(BoardPoint { x, y: 10.0 }),
        viewport: Some(Viewport {
            x: -x,
            y: 0.0,
            zoom: 1.0,
        }),
    };
    assert!(client.send_text(frame.to_text()));
}

async fn call<T: serde::de::DeserializeOwned>(client: &Client, request: Request) -> T {
    client.call_as(&request).await.unwrap()
}

async fn board(client: &Client, project: &Project) -> BoardSnapshot {
    call(
        client,
        Request::BoardLoad {
            project_id: project.id.clone(),
        },
    )
    .await
}

/// A card with the stand-in running in it, started by `client`.
async fn new_card(client: &Client, project: &Project) -> String {
    let card: SessionCard = call(
        client,
        Request::SessionCreate {
            project_id: project.id.clone(),
            opts: serde_json::from_value(json!({ "x": 0, "y": 0, "name": "Shared" })).unwrap(),
        },
    )
    .await;
    card.node.session_id
}

async fn open(client: &Client, project: &Project, session_id: &str, (cols, rows): (f64, f64)) {
    client
        .call(&Request::SessionOpen {
            project_id: project.id.clone(),
            session_id: session_id.into(),
            cols,
            rows,
        })
        .await
        .unwrap();
}

async fn write(client: &Client, session_id: &str, data: &str) {
    client
        .call(&Request::SessionWrite {
            session_id: session_id.into(),
            data: data.into(),
        })
        .await
        .unwrap();
}

/// The next size the session's terminal is said to have.
async fn sized(events: &mut Events, session_id: &str) -> (f64, f64) {
    let Event::PtySize { cols, rows, .. } = until(
        events,
        |e| matches!(e, Event::PtySize { session_id: s, .. } if s == session_id),
    )
    .await
    else {
        unreachable!()
    };
    (cols, rows)
}

/// Waits until the session has printed `text`.
async fn printed(events: &mut Events, session_id: &str, text: &str) {
    let mut out = String::new();
    while !out.contains(text) {
        if let Event::PtyData { data, .. } = until(
            events,
            |e| matches!(e, Event::PtyData { session_id: s, .. } if s == session_id),
        )
        .await
        {
            out.push_str(&data);
        }
    }
}

async fn add_ink(client: &Client, project: &Project, strokes: Vec<InkStroke>) {
    let add = Request::BoardAddInk {
        project_id: project.id.clone(),
        strokes,
    };
    client.call(&add).await.unwrap();
}

async fn remove_ink(client: &Client, project: &Project, ids: Vec<String>) {
    let remove = Request::BoardRemoveInk {
        project_id: project.id.clone(),
        ids,
    };
    client.call(&remove).await.unwrap();
}

/// POSTs to the session host as the session's own hooks do.
async fn post(port: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut request = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header("Content-Type", "application/json")
        .body(body.to_owned());
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status().as_u16();
    (status, answer.text().await.unwrap())
}

/// Claude's `UserPromptSubmit` hook for the session.
async fn prompt_hook(env: &BTreeMap<String, String>, session_id: &str) {
    let bearer = format!("Bearer {}", env["NODUS_TOKEN"]);
    let (status, _) = post(
        &env["NODUS_PORT"],
        &format!("/hook?evt=UserPromptSubmit&sid={session_id}"),
        &[("Authorization", &bearer)],
        &json!({ "session_id": session_id, "hook_event_name": "UserPromptSubmit", "prompt": "x" }).to_string(),
    )
    .await;
    assert_eq!(status, 204);
}

// ---------------------------------------------------------------------------

/// SRV-180, SRV-181
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn members_on_a_board_see_each_other_named_by_their_tokens() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, mut octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, mut alice_hears) = server.connect(ALICE_TOKEN).await;
    let (elsewhere, mut elsewhere_hears) = server.connect(OCTO_TOKEN).await;

    on_board(&octo, &project.id, 1.0);
    // Alice says she's someone else; she isn't.
    let spoofed = json!({ "presence": {
        "project": project.id, "login": "mallory", "device": "d-mallory", "color": "#000000",
        "cursor": { "x": 3, "y": 4 }, "viewport": { "x": 0, "y": 0, "zoom": 2 }
    }});
    assert!(alice.send_text(spoofed.to_string()));
    // Octo's other device is on another board.
    on_board(&elsewhere, "another-board", 9.0);

    let heard = presence(&mut octo_hears, &project.id, |p| !p.peers.is_empty()).await;
    assert_eq!(logins(&heard), ["alice"]);
    let peer = &heard.peers[0];
    assert_eq!(peer.device, "d-alice");
    assert_eq!(peer.color, presence_color("alice"));
    assert_eq!(peer.cursor, Some(BoardPoint { x: 3.0, y: 4.0 }));
    assert_eq!(peer.viewport.map(|v| v.zoom), Some(2.0));

    let heard = presence(&mut alice_hears, &project.id, |p| !p.peers.is_empty()).await;
    assert_eq!(logins(&heard), ["octo"]);
    assert_eq!(heard.peers[0].device, "d-octo");
    assert_eq!(heard.peers[0].cursor, Some(BoardPoint { x: 1.0, y: 10.0 }));

    // A dev token is `dev`, and Octo moving reaches Alice, never Octo.
    let (dev, _dev_hears) = server.connect(DEV_TOKEN).await;
    on_board(&dev, &project.id, 5.0);
    on_board(&octo, &project.id, 2.0);
    let heard = presence(&mut alice_hears, &project.id, |p| {
        p.peers.len() == 2 && p.peers[0].cursor.is_some_and(|c| c.x == 2.0)
    })
    .await;
    assert_eq!(logins(&heard), ["octo", "dev"]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Some(event) = octo_hears.try_next() {
        if let Event::Presence(p) = event {
            assert!(!logins(&p).contains(&"octo"), "Octo heard of himself: {p:?}");
        }
    }
    // The other board heard only of itself: nobody.
    let lonely = presence(&mut elsewhere_hears, "another-board", |_| true).await;
    assert!(lonely.peers.is_empty());
}

/// SRV-180: 20 times a second at most, the latest of each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_board_hears_at_most_20_relays_a_second() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, mut octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, _alice_hears) = server.connect(ALICE_TOKEN).await;
    on_board(&octo, &project.id, 0.0);
    on_board(&alice, &project.id, 0.0);
    presence(&mut octo_hears, &project.id, |p| !p.peers.is_empty()).await;
    while octo_hears.try_next().is_some() {}

    let started = Instant::now();
    for i in 1..=200 {
        on_board(&alice, &project.id, f64::from(i));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Long enough for the last relay to arrive.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let elapsed = started.elapsed();
    let mut relays = Vec::new();
    while let Some(event) = octo_hears.try_next() {
        if let Event::Presence(p) = event {
            relays.push(p);
        }
    }
    let most = (elapsed.as_millis() / 50) as usize + 2;
    assert!(
        !relays.is_empty() && relays.len() <= most,
        "{} relays in {elapsed:?}",
        relays.len()
    );
    let last = relays.last().unwrap();
    assert_eq!(last.peers[0].cursor, Some(BoardPoint { x: 200.0, y: 10.0 }));
}

/// SRV-180: 10 s after its last frame, or at once when it goes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_drops_out_after_10_seconds_or_at_once_when_it_goes() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, mut octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, _alice_hears) = server.connect(ALICE_TOKEN).await;
    on_board(&octo, &project.id, 0.0);
    on_board(&alice, &project.id, 0.0);
    presence(&mut octo_hears, &project.id, |p| logins(p) == ["alice"]).await;
    let quiet_since = Instant::now();

    // Octo stays, saying the same again; Alice says nothing more.
    let keep_up = async {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_secs(3)).await;
            on_board(&octo, &project.id, 0.0);
        }
    };
    let gone = async {
        presence(&mut octo_hears, &project.id, |p| p.peers.is_empty()).await;
        quiet_since.elapsed()
    };
    let ((), after) = tokio::join!(keep_up, gone);
    assert!(
        after >= Duration::from_millis(9_900) && after < Duration::from_secs(12),
        "dropped after {after:?}"
    );

    // Back, then gone at once when its connection closes.
    on_board(&alice, &project.id, 1.0);
    presence(&mut octo_hears, &project.id, |p| logins(p) == ["alice"]).await;
    let closed = Instant::now();
    alice.close();
    alice.closed().await;
    presence(&mut octo_hears, &project.id, |p| p.peers.is_empty()).await;
    assert!(closed.elapsed() < Duration::from_secs(2), "{:?}", closed.elapsed());
}

/// SRV-144
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_terminal_follows_whoever_typed_last() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, mut octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, mut alice_hears) = server.connect(ALICE_TOKEN).await;
    let id = new_card(&octo, &project).await;

    // The first to open sizes it, and is told so.
    open(&octo, &project, &id, (100.0, 30.0)).await;
    assert_eq!(sized(&mut octo_hears, &id).await, (100.0, 30.0));
    printed(&mut octo_hears, &id, "stand-in ready").await;
    // Alice opens it smaller: it keeps Octo's size, and she's told it.
    open(&alice, &project, &id, (80.0, 24.0)).await;
    assert_eq!(sized(&mut alice_hears, &id).await, (100.0, 30.0));
    // Her terminal resizing doesn't move it.
    alice
        .call(&Request::SessionResize {
            session_id: id.clone(),
            cols: 82.0,
            rows: 24.0,
        })
        .await
        .unwrap();
    write(&octo, &id, "one\r").await;
    printed(&mut octo_hears, &id, "got: one size: 30 100").await;

    // She types: her size first, for both of them.
    write(&alice, &id, "two\r").await;
    assert_eq!(sized(&mut alice_hears, &id).await, (82.0, 24.0));
    assert_eq!(sized(&mut octo_hears, &id).await, (82.0, 24.0));
    printed(&mut octo_hears, &id, "got: two size: 24 82").await;
    // Now Octo's resizes wait until he types again.
    octo.call(&Request::SessionResize {
        session_id: id.clone(),
        cols: 120.0,
        rows: 40.0,
    })
    .await
    .unwrap();
    write(&alice, &id, "three\r").await;
    printed(&mut alice_hears, &id, "got: three size: 24 82").await;
    write(&octo, &id, "four\r").await;
    assert_eq!(sized(&mut alice_hears, &id).await, (120.0, 40.0));
    printed(&mut alice_hears, &id, "got: four size: 40 120").await;
}

/// SRV-142
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_prompt_records_who_sent_it() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, _octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, _alice_hears) = server.connect(ALICE_TOKEN).await;
    let id = new_card(&octo, &project).await;
    let env = world.session_env(&id).await;
    open(&octo, &project, &id, (100.0, 30.0)).await;

    // Nobody has typed yet: the card's starter sent it.
    prompt_hook(&env, &id).await;
    // Alice types the next one, from the prompt bar's road too.
    write(&alice, &id, "hello\r").await;
    prompt_hook(&env, &id).await;
    alice
        .call(&Request::SessionSend {
            project_id: project.id.clone(),
            session_id: id.clone(),
            text: "again".into(),
        })
        .await
        .unwrap();
    prompt_hook(&env, &id).await;
    // Then Octo.
    write(&octo, &id, "mine\r").await;
    prompt_hook(&env, &id).await;

    let deadline = Instant::now() + WAIT;
    let prompts = loop {
        let card = board(&octo, &project)
            .await
            .cards
            .into_iter()
            .find(|c| c.node.session_id == id)
            .unwrap();
        let prompts = card.node.prompts();
        if prompts.len() == 4 || Instant::now() > deadline {
            break prompts;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let by: Vec<&str> = prompts.iter().map(|p| p.by.as_str()).collect();
    assert_eq!(by, ["octo", "alice", "alice", "octo"]);

    // The transcript names each user turn's sender.
    let iso = |at: u64| nodus_transcripts::date::to_iso_string(at as f64);
    let user = |at: u64, text: &str| {
        json!({ "type": "user", "timestamp": iso(at), "message": { "role": "user", "content": text } }).to_string()
    };
    let reply = |at: u64| {
        json!({ "type": "assistant", "timestamp": iso(at + 500), "message": { "id": format!("m{at}"), "content": [{ "type": "text", "text": "ok" }] } })
            .to_string()
    };
    let lines: Vec<String> = prompts.iter().flat_map(|p| [user(p.at, "x"), reply(p.at)]).collect();
    let dir = nodus_transcripts::transcript_dir(&world.root().join("home"), &project.repo_path);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.jsonl")), lines.join("\n") + "\n").unwrap();
    let transcript: Vec<TranscriptMessage> = call(
        &alice,
        Request::SessionTranscript {
            project_id: project.id.clone(),
            session_id: id.clone(),
        },
    )
    .await;
    let senders: Vec<Option<&str>> = transcript.iter().map(|m| m.by.as_deref()).collect();
    assert_eq!(
        senders,
        [
            Some("octo"),
            None,
            Some("alice"),
            None,
            Some("alice"),
            None,
            Some("octo"),
            None
        ]
    );
}

/// SRV-145, SRV-210
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn any_member_allows_a_views_command_and_the_log_says_who() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, _octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, _alice_hears) = server.connect(ALICE_TOKEN).await;
    let id = new_card(&octo, &project).await;
    let env = world.session_env(&id).await;

    // The session puts up a view with a command, through its board tools.
    let call_tool = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "create_view", "arguments": {
            "title": "Hello", "html": "<p>hi</p>", "commands": [{ "name": "hello", "run": "echo hi" }]
        } }
    });
    let bearer = format!("Bearer {}", env["NODUS_TOKEN"]);
    let (status, body) = post(
        &env["NODUS_PORT"],
        "/mcp",
        &[("Authorization", &bearer), ("X-Nodus-Session", &id)],
        &call_tool.to_string(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let view = board(&alice, &project)
        .await
        .views
        .into_iter()
        .next()
        .expect("the view");
    let command = |views: &[nodus_domain::BoardView]| views[0].commands.clone().unwrap().remove(0);
    assert_eq!(command(std::slice::from_ref(&view)).approved_by(), None);

    // Alice didn't put it up, and allows it all the same.
    alice
        .call(&Request::ViewsApprove {
            project_id: project.id.clone(),
            view_id: view.id.clone(),
            name: "hello".into(),
            allow: true,
        })
        .await
        .unwrap();
    let allowed = command(&board(&octo, &project).await.views);
    assert_eq!(allowed.approved.as_deref(), Some("echo hi"));
    assert_eq!(allowed.approved_by(), Some("alice"));

    // Octo turns it down: nobody's allowing it now.
    octo.call(&Request::ViewsApprove {
        project_id: project.id.clone(),
        view_id: view.id.clone(),
        name: "hello".into(),
        allow: false,
    })
    .await
    .unwrap();
    let denied = command(&board(&octo, &project).await.views);
    assert_eq!((denied.approved.as_deref(), denied.approved_by()), (None, None));

    let audit: Vec<AuditLine> = call(&octo, Request::AuditList {}).await;
    let approvals: Vec<(&str, &str, &Value)> = audit
        .iter()
        .filter(|line| line.event.starts_with("view."))
        .map(|line| {
            (
                line.event.as_str(),
                line.member.as_ref().unwrap().login.as_str(),
                &line.details,
            )
        })
        .collect();
    assert_eq!(approvals.len(), 2, "{audit:#?}");
    assert_eq!(approvals[0].0, "view.command-approved");
    assert_eq!(approvals[0].1, "alice");
    assert_eq!(
        *approvals[0].2,
        json!({ "projectId": project.id, "viewId": view.id, "name": "hello", "run": "echo hi" })
    );
    assert_eq!((approvals[1].0, approvals[1].1), ("view.command-denied", "octo"));
}

/// SRV-184, SRV-183
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_members_drawing_at_once_keep_both() {
    let world = World::new();
    let server = world.serve();
    let project = world.shop(&server).await;
    let (octo, _octo_hears) = server.connect(OCTO_TOKEN).await;
    let (alice, mut alice_hears) = server.connect(ALICE_TOKEN).await;
    alice
        .call(&Request::Watch {
            projects: vec![project.id.clone()],
        })
        .await
        .unwrap();
    let stroke = |id: &str, x: f64| -> InkStroke {
        serde_json::from_value(
            json!({ "id": id, "kind": "pen", "color": "red", "points": [x, 0, x + 5.0, 5], "size": 1 }),
        )
        .unwrap()
    };
    // Each draws two strokes, at the same moment.
    tokio::join!(
        add_ink(&octo, &project, vec![stroke("o1", 0.0)]),
        add_ink(&alice, &project, vec![stroke("a1", 100.0)]),
        add_ink(&octo, &project, vec![stroke("o2", 10.0)]),
        add_ink(&alice, &project, vec![stroke("a2", 110.0)]),
    );
    // Alice hears the board changed, Octo's strokes too.
    until(
        &mut alice_hears,
        |e| matches!(e, Event::BoardChanged { project_id } if *project_id == project.id),
    )
    .await;
    let ink = board(&alice, &project).await.ink;
    let mut ids: Vec<&str> = ink.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["a1", "a2", "o1", "o2"]);
    let by = |id: &str| {
        ink.iter()
            .find(|s| s.id == id)
            .and_then(|s| s.created_by().map(str::to_owned))
    };
    assert_eq!(by("o1").as_deref(), Some("octo"));
    assert_eq!(by("a2").as_deref(), Some("alice"));

    // Octo erases one of his while Alice moves one of hers.
    tokio::join!(
        remove_ink(&octo, &project, vec!["o1".into()]),
        add_ink(&alice, &project, vec![stroke("a1", 200.0)]),
    );
    let ink = board(&octo, &project).await.ink;
    let ids: Vec<&str> = ink.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(!ids.contains(&"o1"));
    let a1 = ink.iter().find(|s| s.id == "a1").unwrap();
    assert_eq!(a1.points[0], 200.0);
    assert_eq!(a1.created_by(), Some("alice"));
}
