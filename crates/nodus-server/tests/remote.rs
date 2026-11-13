//! A server project in the app, end to end (SRV-160…164, SRV-027, SRV-028):
//! the real `nodus-server serve` on a fixed loopback port, set up as
//! `serve.rs` sets it up (a scratch data folder, an env file, a dev token,
//! a stand-in `claude`), and a headless app core with a data folder of its
//! own, which adds the server and links its project. Everything the app
//! does goes through the core's ordinary API, as the window's calls do.
//! The real claude is never run, and every server and host a test starts
//! is stopped before it ends.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_core::domain::{BoardSnapshot, CreateSessionOptions, Event, NodePatch, ProjectPatch, ServerState, Viewport};
use nodus_core::{EnvSource, HostLauncher, Nodus, NodusConfig, SafeStorage};
use nodus_host_proto::HostPaths;
use nodus_wire::Request;
use nodus_wire::client::{Client, hello};
use serde_json::json;
use tokio::sync::broadcast;

const WAIT: Duration = Duration::from_secs(20);
/// Long enough for the backoff to come round after a restart (1, 2, 4… s).
const RECONNECT: Duration = Duration::from_secs(45);
const TOKEN: &str = "a-dev-token";

// ---------------------------------------------------------------------------
// The world a test runs in.

struct World {
    dir: tempfile::TempDir,
    port: u16,
}

impl World {
    fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nodus-rem-")
            .tempdir_in("/tmp")
            .unwrap();
        // A port of its own, so the server comes back where the app expects it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let world = Self { dir, port };
        let root = world.root();
        for folder in ["home", "bin", "log", "data", "app", "apphome"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        write_stand_in(&root.join("bin/claude"), &root.join("log"));
        std::fs::write(
            world.data().join("env"),
            format!(
                "PATH={}/bin:/usr/bin:/bin\nHOME=/nowhere\nSHELL=/bin/sh\nLANG=en_US.UTF-8\n",
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

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
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

    /// The server on its port, once it's listening.
    fn serve(&self) -> Server {
        let home = self.root().join("home");
        let bind = format!("127.0.0.1:{}", self.port);
        let mut child = self
            .command(&[
                "serve",
                "--bind",
                &bind,
                "--dev-token",
                TOKEN,
                "--run-home",
                &home.to_string_lossy(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let (listening, heard) = std::sync::mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        let kept = Arc::clone(&output);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.contains("listening http://") {
                    let _ = listening.send(());
                }
                kept.lock().unwrap().push_str(&line);
                kept.lock().unwrap().push('\n');
            }
        });
        heard
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("the server didn't listen: {}", output.lock().unwrap()));
        Server { child }
    }

    /// A repository with one commit, as a bare clone to add by URL.
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

    /// `project add` against the running server.
    fn add_project(&self, target: &str) {
        let added = self
            .command(&["project", "add", target, "--server", &self.url(), "--token", TOKEN])
            .output()
            .unwrap();
        assert!(
            added.status.success(),
            "{}{}",
            String::from_utf8_lossy(&added.stdout),
            String::from_utf8_lossy(&added.stderr)
        );
    }

    /// The app's core, headless, started and launched as the app does it,
    /// with a data folder and home of its own and a synthetic Keychain.
    async fn app(&self) -> Nodus {
        let home = self.root().join("apphome");
        let vars: BTreeMap<String, String> = [
            ("PATH", "/usr/bin:/bin".to_owned()),
            ("HOME", home.display().to_string()),
            ("SHELL", "/bin/sh".to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let env = nodus_platform::Env::with_vars(vars, Some(&home)).await;
        let nodus = Nodus::start(NodusConfig {
            env: EnvSource::Given(env),
            home,
            host_launcher: Some(Arc::new(NoHost)),
            secrets: Some(Arc::new(SafeStorage::with_mac_password(b"a test password"))),
            ..NodusConfig::new(self.root().join("app"), tokio::runtime::Handle::current())
        })
        .unwrap();
        nodus.launch().await.unwrap();
        nodus
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

/// The app runs no sessions of its own here: its host is never started.
struct NoHost;

impl HostLauncher for NoHost {
    fn start(&self, _paths: &HostPaths) -> std::io::Result<()> {
        Err(std::io::Error::other("no local sessions in this test"))
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
}

impl Server {
    /// SIGTERM, as systemd stops it for a restart, then waits for it to go.
    fn stop(mut self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// The core's events, as the window receives them.

struct Heard {
    events: broadcast::Receiver<Event>,
    /// Output so far, by session, starting with what an open replayed.
    output: BTreeMap<String, String>,
    seen: Vec<Event>,
}

impl Heard {
    fn new(events: broadcast::Receiver<Event>) -> Self {
        Self {
            events,
            output: BTreeMap::new(),
            seen: Vec::new(),
        }
    }

    async fn next_within(&mut self, within: Duration) -> Event {
        loop {
            let event = tokio::time::timeout(within, self.events.recv())
                .await
                .unwrap_or_else(|_| panic!("no event in time; seen {:#?}", self.seen));
            match event {
                Ok(event) => {
                    if let Event::PtyData { session_id, data } = &event {
                        self.output.entry(session_id.clone()).or_default().push_str(data);
                    }
                    self.seen.push(event.clone());
                    return event;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("the core went"),
            }
        }
    }

    /// The first event, from now, that `wanted` picks.
    async fn until(&mut self, within: Duration, wanted: impl Fn(&Event) -> bool) -> Event {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let event = self.next_within(left.max(Duration::from_millis(1))).await;
            if wanted(&event) {
                return event;
            }
        }
    }

    async fn server_went(&mut self, server_id: &str, state: ServerState, within: Duration) {
        let want = Event::ServerStatus {
            server_id: server_id.to_owned(),
            state,
        };
        self.until(within, |event| *event == want).await;
    }

    /// Waits until a session has printed `text`, from what reached the core.
    async fn printed(&mut self, session_id: &str, text: &str) {
        while !self.output.get(session_id).is_some_and(|out| out.contains(text)) {
            self.next_within(WAIT).await;
        }
    }
}

/// What the server itself holds of a board, asked over its own WebSocket.
async fn server_board(world: &World, project_id: &str) -> BoardSnapshot {
    let (client, _events) = Client::connect(&world.url(), TOKEN).await.unwrap();
    client
        .call_as(&Request::BoardLoad {
            project_id: project_id.to_owned(),
        })
        .await
        .unwrap()
}

async fn within<T>(work: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, work).await.expect("in time")
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_project_opens_and_runs_like_a_local_one() {
    let world = World::new();
    let server = world.serve();
    let bare = world.bare_repo("shop");
    world.add_project(&format!("file://{}", bare.display()));

    let app = world.app().await;
    let mut heard = Heard::new(app.subscribe());

    // SRV-027: a wrong token is turned down before anything is kept.
    let err = within(app.servers().add(&world.url(), "not-the-token"))
        .await
        .unwrap_err();
    assert_eq!(err.message, nodus_remote::TOKEN_REFUSED);
    assert!(within(app.servers().list()).await.is_empty());

    // SRV-060: a fresh server still needs setting up, which the dev token
    // gets past on loopback.
    assert!(within(hello(&world.url())).await.unwrap().needs_setup);

    // Added, it's kept with its token sealed, and connects.
    let added = within(app.servers().add(&format!("127.0.0.1:{}/", world.port), TOKEN))
        .await
        .unwrap();
    assert_eq!(added.url, world.url());
    heard.server_went(&added.id, ServerState::Connected, WAIT).await;
    let status = app.servers().status(&added.id).unwrap();
    assert_eq!(status.state, ServerState::Connected);
    assert_eq!(status.version.as_deref(), Some(nodus_core::VERSION));
    app.shutdown();
    drop((app, heard));
    let state = std::fs::read_to_string(world.root().join("app/state.json")).unwrap();
    assert!(state.contains("\"servers\"") && !state.contains(TOKEN), "{state}");
    let app = world.app().await;
    let mut heard = Heard::new(app.subscribe());
    // Launched again, it connects from what it kept, token and all.
    if app.servers().status(&added.id).unwrap().state != ServerState::Connected {
        heard.server_went(&added.id, ServerState::Connected, WAIT).await;
    }

    // SRV-161: the server's projects, for the picker, and one linked.
    let theirs = within(app.servers().projects(&added.id)).await.unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].name, "shop");
    let project = within(app.servers().link(&added.id, &theirs[0].id)).await.unwrap();
    assert_eq!(project.id, theirs[0].id);
    assert_eq!(project.server.as_deref(), Some(added.id.as_str()));
    assert!(project.repo_path.is_empty());

    // SRV-160: in the grid with the local projects, named as the server
    // names it, even after someone renames it there.
    let listed = within(app.projects().list()).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].id.as_str(), listed[0].name.as_str()),
        (project.id.as_str(), "shop")
    );
    let (someone, _events) = Client::connect(&world.url(), TOKEN).await.unwrap();
    someone
        .call(&Request::ProjectsUpdate {
            id: project.id.clone(),
            patch: ProjectPatch {
                name: Some("Shop".into()),
                ..ProjectPatch::default()
            },
        })
        .await
        .unwrap();
    drop(someone);
    assert_eq!(within(app.projects().list()).await[0].name, "Shop");
    // Its group is this app's own; its name goes to the server.
    let group = within(app.groups().create("Clients")).await;
    let filed = within(app.projects().update(
        &project.id,
        ProjectPatch {
            group_id: Some(Some(group.id.clone())),
            name: Some("The shop".into()),
            ..ProjectPatch::default()
        },
    ))
    .await
    .unwrap();
    assert_eq!(filed.group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(filed.name, "The shop");
    let on_server = server_board(&world, &project.id).await;
    assert_eq!(on_server.project.name, "The shop");
    assert_eq!(on_server.project.group_id, None);

    // board:load, from the server.
    let board = within(app.board().load(&project.id)).await.unwrap();
    assert!(board.cards.is_empty() && !board.offline);
    assert_eq!(board.project.server.as_deref(), Some(added.id.as_str()));
    assert!(board.project.repo_path.is_empty());
    let git = within(app.git().status(&project.id)).await.unwrap().unwrap();
    assert!(git.oid.is_some(), "{git:?}");

    // session:create runs it on the server, expanded only here (SRV-164).
    let opts: CreateSessionOptions = serde_json::from_value(
        json!({ "x": 0, "y": 0, "name": "Fix login", "initialPrompt": "look", "expanded": true }),
    )
    .unwrap();
    let card = within(app.sessions().create(&project.id, &opts)).await.unwrap();
    let id = card.node.session_id.clone();
    assert!(card.live);
    assert_eq!(card.node.expanded, Some(true));
    assert_eq!(card.node.title.as_deref(), Some("Fix login"));

    // Its output arrives as the core's own PtyData, after what open replays.
    let opened = within(app.sessions().open(&project.id, &id, 100.0, 30.0))
        .await
        .unwrap();
    assert!(opened.live);
    heard.output.entry(id.clone()).or_default().push_str(&opened.replay);
    heard.printed(&id, "stand-in ready").await;
    assert_eq!(app.board().session_project(&id).as_deref(), Some(project.id.as_str()));

    // session:write, key by key, in order.
    for key in ["t", "y", "p", "e", "d", "\r"] {
        app.sessions().write(&id, key);
    }
    heard.printed(&id, "got: typed").await;
    app.sessions().resize(&id, 90.0, 25.0);

    // SRV-164: the viewport and expanded cards are kept here, never there.
    let viewport = Viewport {
        x: 12.0,
        y: -34.5,
        zoom: 0.75,
    };
    within(app.board().save_viewport(&project.id, viewport)).await;
    within(app.board().save_nodes(
        &project.id,
        &[NodePatch {
            session_id: id.clone(),
            x: Some(400.0),
            ..NodePatch::default()
        }],
    ))
    .await
    .unwrap();
    let mine = within(app.board().load(&project.id)).await.unwrap();
    assert_eq!(mine.viewport, Some(viewport));
    assert_eq!(mine.cards[0].node.expanded, Some(true));
    assert_eq!(mine.cards[0].node.x, 400.0);
    let theirs = server_board(&world, &project.id).await;
    assert_eq!(theirs.viewport, None);
    assert_eq!(theirs.cards[0].node.expanded, None);
    assert_eq!(theirs.cards[0].node.x, 400.0);

    // SRV-028: the server goes, and the board still opens, read-only, as
    // it was last seen.
    server.stop();
    heard.server_went(&added.id, ServerState::Offline, WAIT).await;
    heard
        .until(
            WAIT,
            |e| matches!(e, Event::BoardChanged { project_id } if *project_id == project.id),
        )
        .await;
    let cached = within(app.board().load(&project.id)).await.unwrap();
    assert!(cached.offline);
    assert_eq!(cached.cards.len(), 1);
    assert_eq!(cached.cards[0].node.session_id, id);
    assert_eq!(cached.viewport, Some(viewport));
    assert_eq!(cached.cards[0].node.expanded, Some(true));
    let err = within(app.git().status(&project.id)).await.unwrap_err();
    assert_eq!(
        (err.channel, err.message.as_str()),
        ("git:status", nodus_remote::OFFLINE)
    );
    assert_eq!(app.servers().status(&added.id).unwrap().state, ServerState::Offline);
    // Typing into it now goes nowhere, and nothing breaks.
    app.sessions().write(&id, "lost\r");

    // Back where it was, the app reconnects by itself and watches again.
    let server = world.serve();
    heard.server_went(&added.id, ServerState::Connected, RECONNECT).await;
    let fresh = within(app.board().load(&project.id)).await.unwrap();
    assert!(!fresh.offline);
    assert_eq!(fresh.cards[0].node.session_id, id);
    // The session ran on in the host; opened again, it replays and streams.
    let reopened = within(app.sessions().open(&project.id, &id, 100.0, 30.0))
        .await
        .unwrap();
    assert!(reopened.replay.contains("got: typed"), "{:?}", reopened.replay);
    app.sessions().write(&id, "again\r");
    heard.printed(&id, "got: again").await;
    // Its status reaches the app as the project is watched again.
    within(app.sessions().kill(&id)).await;
    heard
        .until(
            WAIT,
            |e| matches!(e, Event::PtyExit { session_id, .. } if *session_id == id),
        )
        .await;

    // Unlinked, it leaves the grid, and stays on the server.
    within(app.servers().unlink(&project.id)).await.unwrap();
    assert!(within(app.projects().list()).await.is_empty());
    assert_eq!(within(app.servers().projects(&added.id)).await.unwrap().len(), 1);
    within(app.servers().remove(&added.id)).await;
    assert!(within(app.servers().list()).await.is_empty());

    app.shutdown();
    server.stop();
}

/// SRV-163: a server board's pictures, its views' pages and the files
/// dropped on its terminals go through its server, never a local path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_projects_files_go_through_its_server() {
    let world = World::new();
    let server = world.serve();
    let bare = world.bare_repo("shop");
    world.add_project(&format!("file://{}", bare.display()));
    let app = world.app().await;
    let mut heard = Heard::new(app.subscribe());
    let added = within(app.servers().add(&world.url(), TOKEN)).await.unwrap();
    heard.server_went(&added.id, ServerState::Connected, WAIT).await;
    let theirs = within(app.servers().projects(&added.id)).await.unwrap();
    let project = within(app.servers().link(&added.id, &theirs[0].id)).await.unwrap();

    // A picture goes up to the server, under the name it picks, and is
    // kept here, so it isn't fetched back.
    let name = within(app.board().save_image(&project.id, b"\x89PNG one".to_vec(), "PNG"))
        .await
        .unwrap();
    assert!(name.ends_with(".png"), "{name}");
    let uploads = world.data().join("uploads");
    assert_eq!(std::fs::read(uploads.join(&name)).unwrap(), b"\x89PNG one");
    let here = app.board().image_path(&project.id, &name).unwrap();
    assert!(
        here.starts_with(world.root().join("app/server-files")),
        "{}",
        here.display()
    );
    assert_eq!(std::fs::read(&here).unwrap(), b"\x89PNG one");
    assert!(!world.root().join("app/board-images").exists());

    // One only the server has (a session's board tools put it there) is
    // fetched once asked for, and the board changes when it's in.
    let theirs_only = "6f1c2d3e-4a5b-4c6d-8e7f-901a2b3c4d5e.png";
    std::fs::create_dir_all(world.data().join("board-images")).unwrap();
    std::fs::write(world.data().join("board-images").join(theirs_only), b"\x89PNG two").unwrap();
    assert_eq!(app.board().image_path(&project.id, theirs_only), None);
    heard
        .until(
            WAIT,
            |e| matches!(e, Event::BoardChanged { project_id } if *project_id == project.id),
        )
        .await;
    let fetched = app.board().image_path(&project.id, theirs_only).unwrap();
    assert_eq!(std::fs::read(fetched).unwrap(), b"\x89PNG two");

    // A view's page comes from the server, not the local core, and is
    // shown as last seen while the server is away.
    let view = within(app.views().create(
        &project.id,
        &serde_json::from_value(json!({ "description": "orders today", "x": 0, "y": 0 })).unwrap(),
    ))
    .await
    .unwrap();
    let page_dir = world.data().join("views").join(&view.id);
    std::fs::create_dir_all(&page_dir).unwrap();
    std::fs::write(page_dir.join("index.html"), "<h1>From the server</h1>").unwrap();
    within(app.board().load(&project.id)).await.unwrap();
    let url = format!("nodus-view://{}/", view.id);
    let serve = |app: &Nodus| {
        let (app, url) = (app.clone(), url.clone());
        tokio::task::spawn_blocking(move || app.views().serve_page(&url))
    };
    let page = within(serve(&app)).await.unwrap();
    assert_eq!(page.status, 200);
    assert!(page.body.contains("From the server"), "{}", page.body);
    assert!(page.header("Content-Security-Policy").is_some());
    assert!(!world.root().join("app/views").join(&view.id).exists());

    // Files dropped on a server terminal are uploaded; their server paths
    // are what's typed.
    let opts: CreateSessionOptions = serde_json::from_value(json!({ "x": 0, "y": 0, "name": "Drop" })).unwrap();
    let card = within(app.sessions().create(&project.id, &opts)).await.unwrap();
    let id = card.node.session_id;
    assert!(app.sessions().on_server(&id));
    let dropped = world.root().join("apphome/notes.txt");
    std::fs::write(&dropped, "local notes").unwrap();
    let paths = within(
        app.sessions()
            .upload_paths(&id, &[dropped.clone(), world.root().join("apphome")]),
    )
    .await
    .unwrap();
    assert_eq!(paths.len(), 1, "a folder is left out: {paths:?}");
    assert!(
        paths[0].starts_with(&uploads.to_string_lossy().into_owned()),
        "{paths:?}"
    );
    assert!(paths[0].ends_with(".txt"));
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "local notes");

    server.stop();
    heard.server_went(&added.id, ServerState::Offline, WAIT).await;
    let away = within(serve(&app)).await.unwrap();
    assert_eq!((away.status, away.body.contains("From the server")), (200, true));
    let err = within(app.board().save_image(&project.id, b"x".to_vec(), "png"))
        .await
        .unwrap_err();
    assert_eq!(err.message, nodus_remote::OFFLINE);
    app.shutdown();
}
