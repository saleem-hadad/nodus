//! Updating a server from the app (SRV-200, SRV-201). The real binary is
//! copied into a scratch "install" folder and run from there, on loopback
//! with a dev token, and updates itself from a fake GitHub releases API in
//! this process (`NODUS_GITHUB_API_URL`). Releases are packed as
//! `package.sh` packs them and signed with a throwaway ed25519 key made as
//! the test runs, which the debug build checks instead of the release key
//! (`NODUS_DEV_UPDATE_KEY`, read only by debug builds).
#![cfg(unix)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::{Request as HttpRequest, Response};
use hyper_util::rt::TokioIo;
use nodus_domain::{Event, ServerUpdateStatus};
use nodus_wire::client::{Client, Events, Received};
use nodus_wire::{Hello, Request};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(25);
const TOKEN: &str = "a-dev-token";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// What a placeholder `signing-key.pub.pem` holds.
const PLACEHOLDER: &str = "# TODO: placeholder. Run scripts/server-signing-key.sh to make the key and write it here.\n";

// ---------------------------------------------------------------------------
// Keys and releases.

struct Key(Ed25519KeyPair);

impl Key {
    fn new() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Self(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
    }

    /// The public key as `openssl pkey -pubout` writes it.
    fn pem(&self) -> String {
        let mut der = vec![0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
        der.extend_from_slice(self.0.public_key().as_ref());
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            STANDARD.encode(der)
        )
    }

    /// 64 raw bytes, as `openssl pkeyutl -sign -rawin` writes them.
    fn sign(&self, file: &[u8]) -> Vec<u8> {
        self.0.sign(file).as_ref().to_vec()
    }
}

/// A version after this one.
fn next_version() -> String {
    let parts: Vec<u64> = VERSION.split('.').map(|p| p.parse().unwrap()).collect();
    format!("{}.{}.0", parts[0], parts[1] + 1)
}

fn asset() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    format!("nodus-server-linux-{arch}.tar.gz")
}

/// `nodus`, packed as `package.sh` packs it, by the system's tar.
fn pack(binary: &[u8]) -> Vec<u8> {
    let stage = tempfile::tempdir().unwrap();
    let file = stage.path().join("nodus");
    std::fs::write(&file, binary).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = stage.path().join("out.tar.gz");
    let status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-C")
        .arg(stage.path())
        .arg("-czf")
        .arg(&out)
        .arg("nodus")
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::read(out).unwrap()
}

/// A release as GitHub's API lists it, with `files` under `/download/<tag>/`.
fn release(base: &str, version: &str, files: &[&str]) -> Value {
    json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "assets": files.iter().map(|name| json!({
            "name": name,
            "state": "uploaded",
            "browser_download_url": format!("{base}/download/v{version}/{name}"),
        })).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// A fake GitHub with releases.

#[derive(Default)]
struct Fake {
    releases: Vec<Value>,
    /// `/download/<tag>/<name>`'s bytes.
    files: HashMap<String, Vec<u8>>,
    /// Every path asked for, in order.
    asked: Vec<String>,
}

#[derive(Clone)]
struct GitHub {
    url: String,
    fake: Arc<Mutex<Fake>>,
}

impl GitHub {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fake = Arc::new(Mutex::new(Fake::default()));
        let serving = Arc::clone(&fake);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let fake = Arc::clone(&serving);
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| answer(Arc::clone(&fake), req));
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self { url, fake }
    }

    /// Publishes `version` with `nodus` in it, its archive signed by `key`;
    /// without a signature when there's no key.
    fn publish(&self, version: &str, nodus: &[u8], key: Option<&Key>) {
        let archive = pack(nodus);
        let name = asset();
        let mut files = vec![name.clone()];
        let mut fake = self.fake.lock().unwrap();
        if let Some(key) = key {
            let sig = format!("{name}.sig");
            fake.files.insert(format!("v{version}/{sig}"), key.sign(&archive));
            files.push(sig);
        }
        fake.files.insert(format!("v{version}/{name}"), archive);
        let files: Vec<&str> = files.iter().map(String::as_str).collect();
        let listed = release(&self.url, version, &files);
        fake.releases.insert(0, listed);
    }

    fn downloads(&self) -> Vec<String> {
        let fake = self.fake.lock().unwrap();
        fake.asked
            .iter()
            .filter(|p| p.starts_with("/download/"))
            .cloned()
            .collect()
    }

    fn asked(&self) -> Vec<String> {
        self.fake.lock().unwrap().asked.clone()
    }
}

async fn answer(fake: Arc<Mutex<Fake>>, req: HttpRequest<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_owned();
    let mut fake = fake.lock().unwrap();
    fake.asked.push(path.clone());
    let (status, body) = if path == "/repos/saleem-hadad/nodus/releases" {
        (200, Value::Array(fake.releases.clone()).to_string().into_bytes())
    } else if let Some(file) = path.strip_prefix("/download/").and_then(|f| fake.files.get(f)) {
        (200, file.clone())
    } else {
        (404, b"{\"message\":\"Not Found\"}".to_vec())
    };
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
    Ok(response)
}

// ---------------------------------------------------------------------------
// The server, installed in a scratch folder.

struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nodus-upd-")
            .tempdir_in("/tmp")
            .unwrap();
        let world = Self { dir };
        for folder in ["install", "home", "data"] {
            std::fs::create_dir_all(world.root().join(folder)).unwrap();
        }
        std::fs::copy(env!("CARGO_BIN_EXE_nodus-server"), world.binary()).unwrap();
        std::fs::set_permissions(world.binary(), std::fs::Permissions::from_mode(0o755)).unwrap();
        world
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    /// The installed binary: `/opt/nodus/bin/nodus` on a server.
    fn binary(&self) -> PathBuf {
        self.root().join("install/nodus")
    }

    fn installed(&self) -> Vec<u8> {
        std::fs::read(self.binary()).unwrap()
    }

    /// The audit log's lines with this event.
    fn audit(&self, event: &str) -> Vec<Value> {
        let text = std::fs::read_to_string(self.root().join("data/audit.jsonl")).unwrap_or_default();
        text.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|line| line["event"] == event)
            .collect()
    }

    /// Starts the installed binary, checking updates from `github` against `key`.
    fn serve(&self, github: &GitHub, key: &str) -> Server {
        let home = self.root().join("home");
        let mut child = Command::new(self.binary())
            .args(["serve", "--bind", "127.0.0.1:0", "--dev-token", TOKEN, "--run-home"])
            .arg(&home)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", &home)
            .env("NODUS_USER_DATA", self.root().join("data"))
            .env("NODUS_GITHUB_API_URL", &github.url)
            .env("NODUS_DEV_UPDATE_KEY", key)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let (listening, url) = std::sync::mpsc::channel();
        for stream in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let kept = Arc::clone(&output);
            let listening = listening.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Some(at) = line.find("listening http://") {
                        let _ = listening.send(line[at + "listening ".len()..].to_owned());
                    }
                    let mut kept = kept.lock().unwrap();
                    kept.push_str(&line);
                    kept.push('\n');
                }
            });
        }
        let url = url
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("the server didn't listen: {}", output.lock().unwrap()));
        Server { child, url, output }
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

struct Server {
    child: Child,
    url: String,
    output: Arc<Mutex<String>>,
}

impl Server {
    async fn connect(&self) -> (Client, Events) {
        Client::connect(&self.url, TOKEN).await.unwrap()
    }

    /// Waits for the process to end by itself.
    async fn exited(&mut self) -> std::process::ExitStatus {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the server didn't exit: {}",
                self.output.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `server:update`, as an app makes it.
async fn update(client: &Client) -> Result<ServerUpdateStatus, String> {
    let answer = tokio::time::timeout(Duration::from_secs(120), client.call(&Request::ServerUpdate {}))
        .await
        .expect("an answer in time");
    answer
        .map(|value| serde_json::from_value(value).unwrap())
        .map_err(|err| err.message)
}

/// The `server-update` events that have arrived.
fn told_since(events: &mut Events) -> Vec<ServerUpdateStatus> {
    let mut told = Vec::new();
    while let Some(event) = events.try_next() {
        if let Event::ServerUpdate { status, .. } = event {
            told.push(status);
        }
    }
    told
}

/// Waits for a `server-update` event that `wanted` takes, keeping all of them.
async fn told_until(events: &mut Events, wanted: impl Fn(&ServerUpdateStatus) -> bool) -> Vec<ServerUpdateStatus> {
    let mut told = Vec::new();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let next = tokio::time::timeout_at(deadline, events.next_received())
            .await
            .expect("the update's events");
        if let Some(Received::Event(Event::ServerUpdate { status, .. })) = next {
            let done = wanted(&status);
            told.push(status);
            if done {
                return told;
            }
        }
    }
}

/// What the new version's binary holds: this one, marked, so it still runs
/// on this machine and the swap shows.
fn new_binary() -> Vec<u8> {
    let mut binary = std::fs::read(env!("CARGO_BIN_EXE_nodus-server")).unwrap();
    binary.extend_from_slice(b"\n# the new version\n");
    binary
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "nodus")
        .collect()
}

// ---------------------------------------------------------------------------

/// SRV-201: a build with the placeholder for a key refuses before it asks
/// GitHub anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refuses_without_a_signing_key() {
    let world = World::new();
    let github = GitHub::start().await;
    let key = Key::new();
    github.publish(&next_version(), &new_binary(), Some(&key));
    let mut server = world.serve(&github, PLACEHOLDER);
    let before = world.installed();
    let (client, mut events) = server.connect().await;

    let err = update(&client).await.unwrap_err();
    assert_eq!(
        err,
        "This build has no signing key yet, so it can’t check an update. Install a release with install-server.sh."
    );
    assert!(github.asked().is_empty(), "{:?}", github.asked());
    assert_eq!(world.installed(), before);
    let told = told_until(&mut events, |s| matches!(s, ServerUpdateStatus::Error { .. })).await;
    assert_eq!(told, [ServerUpdateStatus::Error { message: err.clone() }]);
    let audit = world.audit("server.update-failed");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["details"]["error"], err.as_str());
    assert!(server.running());
}

/// SRV-200: an archive whose signature isn't the release key's is never
/// installed, and the server carries on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refuses_a_bad_signature() {
    let world = World::new();
    let github = GitHub::start().await;
    let (trusted, other) = (Key::new(), Key::new());
    let version = next_version();
    github.publish(&version, &new_binary(), Some(&other));
    let mut server = world.serve(&github, &trusted.pem());
    let before = world.installed();
    let (client, mut events) = server.connect().await;

    let err = update(&client).await.unwrap_err();
    assert_eq!(
        err,
        "The update’s signature doesn’t match Nodus’s signing key. Nothing was installed."
    );
    let asset = asset();
    assert_eq!(
        github.downloads(),
        [
            format!("/download/v{version}/{asset}"),
            format!("/download/v{version}/{asset}.sig")
        ]
    );
    assert_eq!(world.installed(), before);
    assert!(leftovers(&world.root().join("install")).is_empty());
    let told = told_until(&mut events, |s| matches!(s, ServerUpdateStatus::Error { .. })).await;
    assert_eq!(told.first(), Some(&ServerUpdateStatus::Checking));
    assert!(told.iter().any(|s| matches!(s, ServerUpdateStatus::Downloading { .. })));

    // Still up, and a second try is allowed.
    let hello: Hello = nodus_wire::client::hello(&server.url).await.unwrap();
    assert_eq!(hello.version, VERSION);
    assert_eq!(update(&client).await.unwrap_err(), err);
    assert!(server.running());
    assert_eq!(world.audit("server.update-failed").len(), 2);
}

/// SRV-200: only a release newer than the one running is taken. One that
/// isn't changes nothing and downloads nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn never_downgrades() {
    let world = World::new();
    let github = GitHub::start().await;
    let key = Key::new();
    github.publish("0.0.1", &new_binary(), Some(&key));
    github.publish(VERSION, &new_binary(), Some(&key));
    let mut server = world.serve(&github, &key.pem());
    let before = world.installed();
    let (client, mut events) = server.connect().await;

    let answer = update(&client).await.unwrap();
    assert_eq!(
        answer,
        ServerUpdateStatus::Latest {
            version: VERSION.into()
        }
    );
    assert!(github.downloads().is_empty(), "{:?}", github.downloads());
    assert_eq!(world.installed(), before);
    let told = told_until(&mut events, |s| matches!(s, ServerUpdateStatus::Latest { .. })).await;
    assert_eq!(told, [ServerUpdateStatus::Checking, answer]);
    assert_eq!(world.audit("server.up-to-date").len(), 1);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(server.running());
}

/// SRV-200: the newest signed release is downloaded, checked and swapped in
/// beside the old binary, and the server answers and exits for systemd to
/// start the new one. Newer releases without a signature or marked as a
/// prerelease are passed over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installs_the_newest_signed_release_and_exits() {
    let world = World::new();
    let github = GitHub::start().await;
    let key = Key::new();
    let version = next_version();
    let binary = new_binary();
    github.publish("0.0.1", b"an old one", Some(&key));
    github.publish(&version, &binary, Some(&key));
    github.publish("998.0.0", b"unsigned", None);
    {
        github.publish("999.0.0", b"a prerelease", Some(&key));
        let mut fake = github.fake.lock().unwrap();
        fake.releases[0]["prerelease"] = json!(true);
    }
    let mut server = world.serve(&github, &key.pem());
    let (client, mut events) = server.connect().await;

    let answer = update(&client).await.unwrap();
    assert_eq!(
        answer,
        ServerUpdateStatus::Restarting {
            version: version.clone()
        }
    );
    // In place already when it answers: the whole new binary, runnable.
    assert!(world.installed() == binary, "the binary wasn't swapped");
    let mode = std::fs::metadata(world.binary()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    assert!(leftovers(&world.root().join("install")).is_empty());
    let asset = asset();
    assert_eq!(
        github.downloads(),
        [
            format!("/download/v{version}/{asset}"),
            format!("/download/v{version}/{asset}.sig")
        ]
    );

    let told = told_until(&mut events, |s| matches!(s, ServerUpdateStatus::Restarting { .. })).await;
    assert_eq!(told.first(), Some(&ServerUpdateStatus::Checking));
    let percents: Vec<u8> = told
        .iter()
        .filter_map(|s| match s {
            ServerUpdateStatus::Downloading { version: v, percent } if *v == version => Some(*percent),
            _ => None,
        })
        .collect();
    assert_eq!(percents.first(), Some(&0));
    assert_eq!(percents.last(), Some(&100));
    assert!(percents.windows(2).all(|w| w[0] < w[1]), "{percents:?}");

    // It exits by itself, cleanly, and says why.
    let status = server.exited().await;
    assert!(status.success(), "{status:?}");
    let output = server.output.lock().unwrap().clone();
    assert!(output.contains("restarting"), "{output}");
    let audit = world.audit("server.updated");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["details"], json!({ "from": VERSION, "to": version }));
    assert!(
        told_since(&mut events)
            .iter()
            .all(|s| !matches!(s, ServerUpdateStatus::Error { .. }))
    );
}
