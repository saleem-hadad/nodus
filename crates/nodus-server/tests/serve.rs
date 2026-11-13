//! `nodus-server serve` as a service runs it, over its WebSocket: the real
//! binary on loopback with a scratch data folder, an env file, a dev token,
//! and a stand-in `claude` on the env file's PATH that answers `--version`,
//! turns on bracketed paste and echoes what it reads (as in nodus-core's
//! headless tests). The session host is this binary too, started by the
//! server. The real claude is never run, and every server and host a test
//! starts is stopped before it ends.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{BoardSnapshot, Event, OpenSessionResult, Project, SessionCard, SessionStatus};
use nodus_wire::client::{Client, ConnectError, Events, hello};
use nodus_wire::{MAX_FRAME, NOT_AVAILABLE, PROTOCOL, Request};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(20);
const TOKEN: &str = "a-dev-token";

// ---------------------------------------------------------------------------
// The world a test runs in.

struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nodus-srv-")
            .tempdir_in("/tmp")
            .unwrap();
        let world = Self { dir };
        let root = world.root();
        for folder in ["home", "bin", "log", "data"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        write_stand_in(&root.join("bin/claude"), &root.join("log"));
        // SRV-141: the sessions' whole environment, but for HOME, which
        // `--run-home` gives.
        std::fs::write(
            world.data().join("env"),
            format!(
                "# The stand-in first.\nPATH={}/bin:/usr/bin:/bin\nHOME=/nowhere\nSHELL=/bin/sh\nLANG=en_US.UTF-8\n",
                root.display()
            ),
        )
        .unwrap();
        world
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn data(&self) -> PathBuf {
        self.root().join("data")
    }

    /// The binary, with nothing of this process's environment but a PATH.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nodus-server"));
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.root().join("home"))
            .env("NODUS_USER_DATA", self.data());
        command
    }

    /// `serve --bind 127.0.0.1:0 --dev-token … --run-home …`, then `extra`.
    fn serve_command(&self, extra: &[&str]) -> Command {
        let home = self.root().join("home");
        let home = home.to_string_lossy();
        let mut args = vec![
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--dev-token",
            TOKEN,
            "--run-home",
            &home,
        ];
        args.extend_from_slice(extra);
        self.command(&args)
    }

    /// The server, once it's listening.
    fn serve(&self) -> Server {
        self.serve_with(&[])
    }

    /// The server with more flags, once it's listening.
    fn serve_with(&self, extra: &[&str]) -> Server {
        let mut child = self
            .serve_command(extra)
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
        Server { child, url, output }
    }

    /// A repository with one commit, and a bare clone of it to add by URL.
    fn bare_repo(&self, name: &str) -> PathBuf {
        let work = self.root().join(format!("{name}-work"));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("README.md"), "# Shop\n").unwrap();
        git(&work, &["init", "-q"]);
        git(&work, &["add", "README.md"]);
        git(
            &work,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "First",
            ],
        );
        let bare = self.root().join(format!("{name}.git"));
        git(
            &self.root(),
            &[
                "clone",
                "-q",
                "--bare",
                &work.to_string_lossy(),
                &bare.to_string_lossy(),
            ],
        );
        bare
    }

    /// `project add` against a running server, and what it printed.
    fn add_project(&self, server: &Server, target: &str) -> String {
        let added = self
            .command(&["project", "add", target, "--server", &server.url, "--token", TOKEN])
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&added.stdout).into_owned();
        assert!(
            added.status.success(),
            "{out}{}",
            String::from_utf8_lossy(&added.stderr)
        );
        out
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

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The stand-in `claude`: `--version` answers at once; otherwise it turns
/// on bracketed paste, says it's ready and echoes every line it reads.
fn write_stand_in(file: &Path, log: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "9.9.9 (Claude Code)"; exit 0; fi
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

struct Server {
    child: Child,
    url: String,
    output: Arc<Mutex<String>>,
}

impl Server {
    fn port(&self) -> u16 {
        self.url.rsplit(':').next().unwrap().parse().unwrap()
    }

    async fn connect(&self) -> (Client, Watcher) {
        let (client, events) = Client::connect(&self.url, TOKEN).await.unwrap();
        (client, Watcher::new(events))
    }

    /// SIGTERM, as systemd stops it, then waits for it to go.
    fn stop(mut self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let exited = self.child.wait().unwrap();
        assert!(exited.success(), "{}", self.output.lock().unwrap());
        assert!(self.output.lock().unwrap().contains("stopping"));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Events, as a connection receives them.

struct Watcher {
    events: Events,
    /// Output so far, by session, starting with what `session:open` replayed.
    output: BTreeMap<String, String>,
    seen: Vec<Event>,
}

impl Watcher {
    fn new(events: Events) -> Self {
        Self {
            events,
            output: BTreeMap::new(),
            seen: Vec::new(),
        }
    }

    fn keep(&mut self, event: Event) -> Event {
        if let Event::PtyData { session_id, data } = &event {
            self.output.entry(session_id.clone()).or_default().push_str(data);
        }
        self.seen.push(event.clone());
        event
    }

    async fn next(&mut self) -> Event {
        let event = tokio::time::timeout(WAIT, self.events.next())
            .await
            .unwrap_or_else(|_| panic!("no event in time; seen {:#?}", self.seen))
            .expect("the connection is open");
        self.keep(event)
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

    /// Every event that has arrived and not been taken yet.
    fn arrived(&mut self) -> Vec<Event> {
        let mut arrived = Vec::new();
        while let Some(event) = self.events.try_next() {
            arrived.push(self.keep(event));
        }
        arrived
    }
}

fn status(session_id: &str, status: SessionStatus, live: bool) -> impl Fn(&Event) -> bool {
    let want = Event::Status {
        session_id: session_id.to_owned(),
        status,
        live,
    };
    move |event| *event == want
}

fn is_output_of(event: &Event, id: &str) -> bool {
    matches!(event, Event::PtyData { session_id, .. } if session_id == id)
}

/// `GET path` with these headers, raw, and the status and body.
async fn get(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let mut request = format!("GET {path} HTTP/1.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    // To the end, or to the end of a 101's head: the socket stays open then.
    let mut answer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = tokio::time::timeout(WAIT, stream.read(&mut chunk))
            .await
            .expect("an answer")
            .unwrap();
        answer.extend_from_slice(&chunk[..read]);
        let switched = answer.starts_with(b"HTTP/1.1 101") && answer.windows(4).any(|w| w == b"\r\n\r\n");
        if read == 0 || switched {
            break;
        }
    }
    let answer = String::from_utf8_lossy(&answer);
    let status = answer[9..12].parse().unwrap();
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_a_host_of_its_own_the_server_runs_everything_there() {
    let world = World::new();
    // SRV-004: the host `nodus-host.service` runs, on its own socket, which
    // the server connects to instead of starting one.
    let socket = world.root().join("run/host.sock");
    let socket = socket.to_string_lossy();
    let mut host = world
        .command(&["host", "--socket", &socket])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let server = world.serve_with(&["--host-socket", &socket]);
    let bare = world.bare_repo("shop");
    world.add_project(&server, &format!("file://{}", bare.display()));

    let (app, mut events) = server.connect().await;
    let projects: Vec<Project> = app.call_as(&Request::ProjectsList {}).await.unwrap();
    let project = projects[0].clone();
    app.call(&Request::Watch {
        projects: vec![project.id.clone()],
    })
    .await
    .unwrap();

    // SRV-005: git runs in the host, through its `exec`, and answers as if
    // it ran here.
    let git: Value = app
        .call_as(&Request::GitStatus {
            project_id: project.id.clone(),
        })
        .await
        .unwrap();
    assert!(git["branch"].is_string(), "{git}");

    // And sessions run there, as ever.
    let card: SessionCard = app
        .call_as(&Request::SessionCreate {
            project_id: project.id.clone(),
            opts: serde_json::from_value(json!({ "x": 0, "y": 0, "name": "Look", "initialPrompt": "look" })).unwrap(),
        })
        .await
        .unwrap();
    let id = card.node.session_id.clone();
    let opened: OpenSessionResult = app
        .call_as(&Request::SessionOpen {
            project_id: project.id.clone(),
            session_id: id.clone(),
            cols: 100.0,
            rows: 30.0,
        })
        .await
        .unwrap();
    events.output.insert(id.clone(), opened.replay);
    events.printed(&id, "stand-in ready").await;

    let _ = host.kill();
    let _ = host.wait();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_runs_on_the_server_and_streams_to_the_app() {
    let world = World::new();
    let server = world.serve();

    // SRV-022, SRV-027
    let said = hello(&server.url).await.unwrap();
    assert_eq!(said.protocol, PROTOCOL);
    assert_eq!(said.version, env!("CARGO_PKG_VERSION"));
    // No member has signed in: a dev token gets in all the same (SRV-060).
    assert!(said.needs_setup && !said.name.is_empty());

    // A repository added by its URL is cloned into the data folder.
    let bare = world.bare_repo("shop");
    let printed = world.add_project(&server, &format!("file://{}", bare.display()));
    assert!(printed.starts_with("Added shop ("), "{printed}");
    let clone = world.data().join("projects/shop/main");
    assert!(clone.join("README.md").exists());

    let (app, mut events) = server.connect().await;
    // Another app, watching nothing.
    let (_other, mut elsewhere) = server.connect().await;
    let projects: Vec<Project> = app.call_as(&Request::ProjectsList {}).await.unwrap();
    assert_eq!(projects.len(), 1);
    let project = projects[0].clone();
    assert_eq!(project.name, "shop");
    assert_eq!(Path::new(&project.repo_path), clone.canonicalize().unwrap());

    // SRV-026: the project's events from now on.
    app.call(&Request::Watch {
        projects: vec![project.id.clone()],
    })
    .await
    .unwrap();
    let board: BoardSnapshot = app
        .call_as(&Request::BoardLoad {
            project_id: project.id.clone(),
        })
        .await
        .unwrap();
    assert!(board.cards.is_empty());

    // A card with a first prompt, working at once, in the host.
    let card: SessionCard = app
        .call_as(&Request::SessionCreate {
            project_id: project.id.clone(),
            opts: serde_json::from_value(json!({ "x": 0, "y": 0, "name": "Fix login", "initialPrompt": "look" }))
                .unwrap(),
        })
        .await
        .unwrap();
    let id = card.node.session_id.clone();
    assert!(card.live && card.status == SessionStatus::Working);
    assert_eq!(card.node.title.as_deref(), Some("Fix login"));
    events.until(status(&id, SessionStatus::Working, true)).await;

    // SRV-026: its output only once it's open, after what the open replays.
    let opened: OpenSessionResult = app
        .call_as(&Request::SessionOpen {
            project_id: project.id.clone(),
            session_id: id.clone(),
            cols: 100.0,
            rows: 30.0,
        })
        .await
        .unwrap();
    assert!(opened.live);
    assert!(
        events.arrived().iter().all(|e| !is_output_of(e, &id)),
        "{:#?}",
        events.seen
    );
    events.output.insert(id.clone(), opened.replay);
    events.printed(&id, "stand-in ready").await;
    // In the env file's environment, and only that (SRV-141).
    let env = std::fs::read_to_string(world.root().join(format!("log/env.{id}"))).unwrap();
    assert!(env.contains(&format!("HOME={}/home", world.root().display())), "{env}");
    assert!(env.contains(&format!("PATH={}/bin:", world.root().display())), "{env}");
    assert!(!env.contains("NODUS_USER_DATA"), "{env}");

    // Typing reaches the process, and its echo comes back.
    for key in ["t", "y", "p", "e", "d", "\r"] {
        app.call(&Request::SessionWrite {
            session_id: id.clone(),
            data: key.into(),
        })
        .await
        .unwrap();
    }
    events.printed(&id, "got: typed").await;
    app.call(&Request::SessionResize {
        session_id: id.clone(),
        cols: 90.0,
        rows: 25.0,
    })
    .await
    .unwrap();

    // Closed, it sends no more output here, though the session runs on.
    app.call(&Request::SessionClose { session_id: id.clone() })
        .await
        .unwrap();
    app.call(&Request::SessionWrite {
        session_id: id.clone(),
        data: "quiet\r".into(),
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let reopened: OpenSessionResult = app
        .call_as(&Request::SessionOpen {
            project_id: project.id.clone(),
            session_id: id.clone(),
            cols: 100.0,
            rows: 30.0,
        })
        .await
        .unwrap();
    assert!(reopened.replay.contains("got: quiet"), "{:?}", reopened.replay);
    let between = events.arrived();
    assert!(between.iter().all(|e| !is_output_of(e, &id)), "{between:#?}");

    // SRV-025: what stays on the app, and arguments that don't fit.
    let err = app.call_named("settings:get", Value::Null).await.unwrap_err();
    assert_eq!(
        (err.channel.as_str(), err.message.as_str()),
        ("settings:get", NOT_AVAILABLE)
    );
    let err = app
        .call_named("projects:reveal", json!({ "id": project.id }))
        .await
        .unwrap_err();
    assert_eq!(err.message, NOT_AVAILABLE);
    let err = app.call_named("board:load", json!({})).await.unwrap_err();
    assert!(err.message.starts_with("Bad arguments: "), "{}", err.message);
    // The core's own errors, in its words.
    let err = app
        .call(&Request::BoardLoad {
            project_id: "nope".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        (err.channel.as_str(), err.message.as_str()),
        ("board:load", "Unknown project nope")
    );

    // Killing it waits for the exit, then it reads as done and gone.
    app.call(&Request::SessionKill { session_id: id.clone() })
        .await
        .unwrap();
    events.until(status(&id, SessionStatus::Done, false)).await;
    events
        .until(|e| matches!(e, Event::PtyExit { session_id, .. } if *session_id == id))
        .await;

    // The other app watched nothing, so it heard nothing of the session.
    let heard = elsewhere.arrived();
    assert!(heard.iter().all(|e| *e == Event::ProjectsChanged), "{heard:#?}");

    // Disconnected and back, the board is as it was left.
    app.close();
    app.closed().await;
    drop(app);
    let (app, _events) = server.connect().await;
    let board: BoardSnapshot = app
        .call_as(&Request::BoardLoad {
            project_id: project.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(board.cards.len(), 1);
    assert_eq!(board.cards[0].node.session_id, id);
    assert!(!board.cards[0].live);
    drop(app);

    // So it is after the server restarts: its state was saved on the way out.
    server.stop();
    let server = world.serve();
    let (app, _events) = server.connect().await;
    let board: BoardSnapshot = app
        .call_as(&Request::BoardLoad {
            project_id: project.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(board.cards[0].node.title.as_deref(), Some("Fix login"));
    drop(app);
    server.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_on_the_server_is_added_as_it_is() {
    let world = World::new();
    let server = world.serve();
    let folder = world.root().join("docs");
    std::fs::create_dir_all(&folder).unwrap();
    let printed = world.add_project(&server, &folder.to_string_lossy());
    assert!(printed.starts_with("Added docs ("), "{printed}");
    // One server to a data folder.
    let second = world.serve_command(&[]).output().unwrap();
    assert_eq!(second.status.code(), Some(1));
    let said = String::from_utf8_lossy(&second.stdout);
    assert!(said.contains("Another nodus-server serve is using"), "{said}");
    // Every connected app hears the projects changed.
    let (app, mut events) = server.connect().await;
    app.call(&Request::GroupsCreate { name: "Ops".into() }).await.unwrap();
    assert_eq!(events.next().await, Event::ProjectsChanged);
    let projects: Vec<Project> = app.call_as(&Request::ProjectsList {}).await.unwrap();
    assert_eq!(Path::new(&projects[0].repo_path), folder);
    assert!(!world.data().join("projects").exists());
    drop(app);
    server.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_token_in_its_header_and_the_servers_own_names_get_in() {
    let world = World::new();
    let server = world.serve();
    let port = server.port();
    let host = format!("127.0.0.1:{port}");

    // SRV-022: the token in the Authorization header, and only there.
    let wrong = Client::connect(&server.url, "not-the-token").await;
    assert!(matches!(wrong, Err(ConnectError::Refused(401))), "{:?}", wrong.err());
    let upgrade = [
        ("Host", host.as_str()),
        ("Connection", "Upgrade"),
        ("Upgrade", "websocket"),
        ("Sec-WebSocket-Version", "13"),
        ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="), // RFC 6455's sample key. gitleaks:allow
    ];
    assert_eq!(get(port, "/api/ws", &upgrade).await.0, 401);
    assert_eq!(get(port, &format!("/api/ws?token={TOKEN}"), &upgrade).await.0, 401);
    let token_cookie = format!("token={TOKEN}");
    let cookie = [upgrade.as_slice(), &[("Cookie", token_cookie.as_str())]].concat();
    assert_eq!(get(port, "/api/ws", &cookie).await.0, 401);
    let bearer = format!("Bearer {TOKEN}");
    let signed = [upgrade.as_slice(), &[("Authorization", bearer.as_str())]].concat();
    assert_eq!(get(port, "/api/ws", &signed).await.0, 101);

    // SRV-021: a name the server doesn't go by is turned away, hello too.
    let (status, body) = get(port, "/api/hello", &[("Host", &host)]).await;
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["protocol"], PROTOCOL);
    assert_eq!(
        get(port, "/api/hello", &[("Host", &format!("localhost:{port}"))])
            .await
            .0,
        200
    );
    for name in [format!("evil.example:{port}"), "rebound.example".to_owned()] {
        assert_eq!(get(port, "/api/hello", &[("Host", &name)]).await.0, 403, "{name}");
    }
    let rebound = [&[("Host", "evil.example")], &signed[1..]].concat();
    assert_eq!(get(port, "/api/ws", &rebound).await.0, 403);
    assert_eq!(get(port, "/api/elsewhere", &[("Host", &host)]).await.0, 404);
    server.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_frame_over_4_mb_closes_the_connection() {
    let world = World::new();
    let server = world.serve();

    // Just under the cap is a frame like any other.
    let (app, _events) = server.connect().await;
    let big = "x".repeat(MAX_FRAME - 100);
    app.call(&Request::SessionWrite {
        session_id: "not-running".into(),
        data: big,
    })
    .await
    .unwrap();

    // SRV-023: one byte over, and the server closes with 1009.
    let over = format!(
        "{{\"id\":1,\"call\":\"session:write\",\"args\":{{\"data\":\"{}\"}}}}",
        "x".repeat(MAX_FRAME)
    );
    assert!(app.send_text(over));
    let closed = tokio::time::timeout(WAIT, app.closed()).await.expect("closed");
    assert_eq!(closed.code, Some(1009), "{closed:?}");
    assert!(app.call(&Request::ProjectsList {}).await.is_err());

    // The server carries on for everyone else.
    let (other, _events) = server.connect().await;
    let projects: Vec<Project> = other.call_as(&Request::ProjectsList {}).await.unwrap();
    assert!(projects.is_empty());
    drop(other);
    server.stop();
}
