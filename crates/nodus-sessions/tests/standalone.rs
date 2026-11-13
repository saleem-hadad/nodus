//! The client of a host that runs on its own (SRV-004), and the runner that
//! runs every process in it (SRV-005), against the Rust host in this process
//! and a scripted one on another protocol.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nodus_domain::{Event, EventSink, SessionStatus};
use nodus_host_proto::{HostPaths, MessageReader};
use nodus_platform::env::EnvMap;
use nodus_platform::process::Exit;
use nodus_platform::runner::{Program, RunRequest, Runner};
use nodus_sessions::client::{
    BoardTools, BoxFuture, HostClient, HostLauncher, HostPrompt, HostRunner, ReplaceChoice, Wiring,
};
use nodus_sessions::hooks::{HookEndpoint, SessionInput};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

const TIMEOUT: Duration = Duration::from_secs(15);

struct Quiet;

impl SessionInput for Quiet {
    fn process_started(&self, _: &str, _: SessionStatus) {}
    fn process_exited(&self, _: &str) {}
    fn hook(&self, _: &str, _: &str, _: &Value, _: Option<f64>) {}
    fn user_input(&self, _: &str, _: &str, _: Option<f64>) {}
}

impl EventSink for Quiet {
    fn emit(&self, _: Event) {}
}

impl BoardTools for Quiet {
    fn answer(&self, _: &str, _: Value) -> BoxFuture<Result<Option<Value>, String>> {
        Box::pin(async { Ok(None) })
    }
}

/// Nobody is ever asked about another version's sessions here.
#[derive(Default)]
struct Asked(AtomicUsize);

impl HostPrompt for Asked {
    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { ReplaceChoice::Quit })
    }
}

/// Nothing starts a standalone host.
struct NeverStarts;

impl HostLauncher for NeverStarts {
    fn start(&self, _: &HostPaths) -> std::io::Result<()> {
        panic!("a standalone host is never started by its client");
    }
}

struct World {
    dir: tempfile::TempDir,
    socket: PathBuf,
    asked: Arc<Asked>,
    client: HostClient,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-standalone-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("run").join("host.sock");
        let asked = Arc::new(Asked::default());
        let client = HostClient::standalone(
            dir.path().join("ud"),
            &socket,
            Wiring {
                events: Arc::new(Quiet),
                status: Arc::new(Quiet),
                endpoint: Arc::new(HookEndpoint::new()),
                board_tools: Arc::new(Quiet),
                prompt: Arc::clone(&asked) as Arc<dyn HostPrompt>,
                launcher: Arc::new(NeverStarts),
                server: None,
            },
        );
        Self {
            dir,
            socket,
            asked,
            client,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    /// The Rust host on its own, as `nodus host --socket` runs it.
    fn start_host(&self) -> tokio::task::JoinHandle<i32> {
        let (log, _lines) = nodus_host::Logger::memory();
        let args = ["--socket".to_owned(), self.socket.to_string_lossy().into_owned()];
        tokio::spawn(async move { nodus_host::run(&args, log).await })
    }
}

fn env() -> EnvMap {
    EnvMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())])
}

/// Its process is there, or a zombie nothing will reap.
fn dead(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return true;
    }
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')))
}

async fn pid_in(file: &Path) -> i32 {
    for _ in 0..250 {
        if let Ok(text) = std::fs::read_to_string(file)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no pid in {}", file.display());
}

/// At launch, a host that isn't up just means no sessions.
#[tokio::test]
async fn launch_carries_on_without_the_host() {
    let world = World::new();
    let started = Instant::now();
    world.client.connect_host(false).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// A run waits for the host, trying every second, then runs in it with what
/// it was asked: arguments, folder, environment, stdin.
#[tokio::test]
async fn runs_in_the_host_once_its_up() {
    let world = World::new();
    let runner = HostRunner::new(world.client.clone());
    let request = RunRequest::new("sh")
        .args(["-c", "pwd; echo \"$WHO\"; cat; exit 4"])
        .cwd(world.root())
        .env(EnvMap::from([
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("WHO".to_owned(), "nodus-run".to_owned()),
        ]))
        .stdin("fed\n");
    let run = tokio::spawn(runner.run(request));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!run.is_finished());
    world.start_host();
    let output = tokio::time::timeout(TIMEOUT, run).await.unwrap().unwrap().unwrap();
    assert_eq!(
        output.stdout_text(),
        format!("{}\nnodus-run\nfed\n", world.root().display())
    );
    assert_eq!(output.exit, Exit::Code(4));
    assert_eq!(output.command, "sh -c pwd; echo \"$WHO\"; cat; exit 4");

    // A program that isn't there, in the host's words.
    let err = runner
        .run(RunRequest::new("nodus-no-such-program").env(env()))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "spawn nodus-no-such-program ENOENT");
    assert_eq!(err.code, "ENOENT");
    // `claude` is the host's to find.
    assert_eq!(
        runner.claude(&nodus_platform::Env::given(env(), None)),
        Ok(Program::Claude)
    );
    // A folder to trust that isn't one.
    assert_eq!(world.client.trust("").await, Err("There’s no folder to trust".into()));
}

/// Giving up on a run stops it in the host.
#[tokio::test]
async fn a_dropped_run_stops() {
    let world = World::new();
    world.start_host();
    let runner = HostRunner::new(world.client.clone());
    let pidfile = world.root().join("pid");
    let mut run = runner.run(
        RunRequest::new("/bin/sh")
            .args(["-c", &format!("echo $$ > '{}'; exec sleep 30", pidfile.display())])
            .env(env()),
    );
    let pid = tokio::select! {
        _ = &mut run => panic!("it ended"),
        pid = pid_in(&pidfile) => pid,
    };
    drop(run);
    let deadline = Instant::now() + TIMEOUT;
    while !dead(pid) {
        assert!(Instant::now() < deadline, "the run outlived its caller");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A host on another protocol is asked to stop, nobody is asked about its
/// sessions, and the new one is waited for.
#[tokio::test]
async fn waits_through_a_host_on_another_protocol() {
    let world = World::new();
    std::fs::create_dir_all(world.socket.parent().unwrap()).unwrap();
    let old = UnixListener::bind(&world.socket).unwrap();
    let socket = world.socket.clone();
    let old_host = tokio::spawn(async move {
        let (mut stream, _) = old.accept().await.unwrap();
        let mut reader = MessageReader::new();
        let mut heard = Vec::new();
        let mut buffer = vec![0u8; 4096];
        let mut welcomed = false;
        loop {
            let n = stream.read(&mut buffer).await.unwrap();
            assert_ne!(n, 0, "the client went without asking it to stop");
            for msg in reader.feed(&buffer[..n]) {
                heard.push(msg["type"].as_str().unwrap_or_default().to_owned());
                if !welcomed {
                    welcomed = true;
                    let welcome =
                        json!({"type": "welcome", "protocol": 1, "hookPort": 1, "hookToken": "t", "sessions": []});
                    stream.write_all((welcome.to_string() + "\n").as_bytes()).await.unwrap();
                } else if msg["type"] == "shutdown" {
                    // It goes, and its socket with it, as systemd brings up the new one.
                    drop(old);
                    std::fs::remove_file(&socket).unwrap();
                    return heard;
                }
            }
        }
    });
    let runner = HostRunner::new(world.client.clone());
    let run = tokio::spawn(runner.run(RunRequest::new("/bin/echo").arg("new").env(env())));
    let heard = tokio::time::timeout(TIMEOUT, old_host).await.unwrap().unwrap();
    assert_eq!(heard, ["hello", "shutdown"]);
    world.start_host();
    let output = tokio::time::timeout(TIMEOUT, run).await.unwrap().unwrap().unwrap();
    assert_eq!(output.stdout_text(), "new\n");
    assert_eq!(world.asked.0.load(Ordering::SeqCst), 0);
}
