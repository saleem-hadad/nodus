//! Projects from GitHub and a worktree per card (SRV-080…085, SRV-120…128,
//! SRV-143, SRV-183), as two members use them: the real binary on loopback,
//! its session host on a socket of its own as `nodus-host.service` runs it
//! (so every git command runs there, SRV-005), and a fake GitHub in this
//! process. The fake answers the App's calls, checking its JWT, mints
//! installation tokens and remembers what each was for, and serves the
//! repositories over git's smart HTTP through `git http-backend`, only to a
//! token minted for that repository. The server is set up as the wizard
//! would leave it: an App, its key (made by `openssl genrsa` as the test
//! runs), and Alice and Bob each signed in on a device. A stand-in `claude`
//! says where it started and what it was given, and a `git` in front of the
//! real one writes down which process ran it.
#![cfg(unix)]

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request as HttpRequest, Response};
use hyper_util::rt::TokioIo;
use nodus_domain::{
    BoardNote, BoardSnapshot, CreatedBy, FolderColor, GitStatus, Project, ProjectGithub, SessionCard, Worktree,
};
use nodus_wire::client::{Client, Events};
use nodus_wire::{GithubRepo, Member, Request};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(30);
const APP_ID: u64 = 4242;
const INSTALLATION: u64 = 77;
const BOT_ID: u64 = 9999;
/// GitHub's path for the App's bot user, in two pieces so the pre-push check
/// doesn't take it for a personal macOS path.
const BOT_PATH: &str = concat!("/users", "/nodus-box[bot]");
const ALICE_TOKEN: &str = "alice-device-token";
const BOB_TOKEN: &str = "bob-device-token";

// ---------------------------------------------------------------------------
// A fake GitHub, with git over HTTP.

#[derive(Clone)]
struct GitHub {
    url: String,
    fake: Arc<Mutex<Fake>>,
}

struct Fake {
    /// The App's key's public half.
    key: Vec<u8>,
    /// Where the bare repositories are: `<owner>/<name>.git`.
    repos: PathBuf,
    /// Every token minted, in order.
    minted: Vec<Minted>,
    /// Every push taken: the repository and the token it came with.
    pushes: Vec<(String, String)>,
    /// Git requests turned away for want of a token for their repository.
    refused: u32,
}

#[derive(Debug, Clone)]
struct Minted {
    token: String,
    repositories: Option<Vec<String>>,
    permissions: Value,
}

impl GitHub {
    async fn start(repos: PathBuf, pem: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fake = Arc::new(Mutex::new(Fake {
            key: public_key(pem),
            repos,
            minted: Vec::new(),
            pushes: Vec::new(),
            refused: 0,
        }));
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

    fn fake(&self) -> MutexGuard<'_, Fake> {
        self.fake.lock().unwrap()
    }

    /// The `host:port` git asks the helper about.
    fn host(&self) -> String {
        self.url.trim_start_matches("http://").to_owned()
    }
}

async fn answer(fake: Arc<Mutex<Fake>>, req: HttpRequest<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().to_string();
    let path = percent_decode(req.uri().path());
    let query = req.uri().query().unwrap_or("").to_owned();
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    let body = req.into_body().collect().await.unwrap().to_bytes();
    if let Some((repo, rest)) = git_path(&path) {
        return Ok(git(&fake, &method, &repo, &rest, &query, &headers, body.to_vec()).await);
    }
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("token ")))
        .map(str::to_owned);
    let (status, reply) = fake.lock().unwrap().api(&method, &path, bearer.as_deref(), &body);
    let mut response = Response::new(Full::new(Bytes::from(reply.to_string())));
    *response.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
    Ok(response)
}

impl Fake {
    fn api(&mut self, method: &str, path: &str, bearer: Option<&str>, body: &[u8]) -> (u16, Value) {
        let app = self.app_jwt(bearer);
        match (method, path) {
            ("GET", "/app/installations") if app => (
                200,
                json!([{
                    "id": INSTALLATION,
                    "account": { "login": "acme", "type": "Organization" },
                    "repository_selection": "selected",
                    "html_url": "https://github.com/organizations/acme/settings/installations/77"
                }]),
            ),
            ("POST", "/app/installations/77/access_tokens") if app => {
                let asked: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                let token = format!("ghs_fake{}", self.minted.len() + 1);
                self.minted.push(Minted {
                    token: token.clone(),
                    repositories: asked["repositories"]
                        .as_array()
                        .map(|names| names.iter().map(|n| n.as_str().unwrap_or("").to_owned()).collect()),
                    permissions: asked["permissions"].clone(),
                });
                (201, json!({ "token": token, "expires_at": "2099-01-01T00:00:00Z" }))
            }
            ("GET", "/installation/repositories") => {
                if !bearer.is_some_and(|b| self.minted.iter().any(|m| m.token == b)) {
                    return (401, json!({ "message": "Bad credentials" }));
                }
                (
                    200,
                    json!({
                        "total_count": 2,
                        "repositories": [
                            { "full_name": "acme/shop", "name": "shop", "private": true, "default_branch": "main" },
                            { "full_name": "acme/other", "name": "other", "private": false, "default_branch": "trunk" }
                        ]
                    }),
                )
            }
            ("GET", path) if path == BOT_PATH => (200, json!({ "id": BOT_ID, "login": "nodus-box[bot]" })),
            ("GET" | "POST", "/app/installations" | "/app/installations/77/access_tokens") => {
                (401, json!({ "message": "A JSON web token could not be decoded" }))
            }
            _ => (404, json!({ "message": "Not Found" })),
        }
    }

    /// Whether `bearer` is a JWT for the App, in date and signed by its key.
    fn app_jwt(&self, bearer: Option<&str>) -> bool {
        let Some(jwt) = bearer else { return false };
        let parts: Vec<&str> = jwt.split('.').collect();
        let [_, claims, signature] = parts[..] else {
            return false;
        };
        let Ok(claims) = URL_SAFE_NO_PAD.decode(claims) else {
            return false;
        };
        let claims: Value = serde_json::from_slice(&claims).unwrap_or(Value::Null);
        if claims["iss"].as_str().and_then(|iss| iss.parse().ok()) != Some(APP_ID) {
            return false;
        }
        let signed = format!("{}.{}", parts[0], parts[1]);
        let signature = URL_SAFE_NO_PAD.decode(signature).unwrap_or_default();
        ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, &self.key)
            .verify(signed.as_bytes(), &signature)
            .is_ok()
    }

    /// The token a git request carries, when it was minted for `repo` with
    /// `contents: write`.
    fn git_token(&self, repo: &str, headers: &HashMap<String, String>) -> Option<String> {
        let basic = headers.get("authorization")?.strip_prefix("Basic ")?;
        let decoded = String::from_utf8(STANDARD.decode(basic).ok()?).ok()?;
        let (user, token) = decoded.split_once(':')?;
        let name = repo.rsplit('/').next()?;
        let minted = self.minted.iter().find(|m| m.token == token)?;
        let fits = minted
            .repositories
            .as_ref()
            .is_some_and(|names| names.iter().any(|n| n == name))
            && minted.permissions["contents"] == "write";
        (user == "x-access-token" && fits).then(|| token.to_owned())
    }
}

/// `/<owner>/<name>.git/<rest>`: the repository and the rest.
fn git_path(path: &str) -> Option<(String, String)> {
    let at = path.find(".git/")?;
    let repo = path[1..at].to_owned();
    (repo.matches('/').count() == 1).then(|| (repo, path[at + ".git".len()..].to_owned()))
}

/// git's smart HTTP, through `git http-backend`, for a token minted for
/// the repository; anything else gets a 401, as GitHub asks for one.
async fn git(
    fake: &Arc<Mutex<Fake>>,
    method: &str,
    repo: &str,
    rest: &str,
    query: &str,
    headers: &HashMap<String, String>,
    body: Vec<u8>,
) -> Response<Full<Bytes>> {
    let (token, root) = {
        let mut fake = fake.lock().unwrap();
        let token = fake.git_token(repo, headers);
        if token.is_none() {
            fake.refused += 1;
        }
        (token, fake.repos.clone())
    };
    let Some(token) = token else {
        let mut response = Response::new(Full::new(Bytes::from("Authentication required")));
        *response.status_mut() = hyper::StatusCode::UNAUTHORIZED;
        response
            .headers_mut()
            .insert("WWW-Authenticate", "Basic realm=\"GitHub\"".parse().unwrap());
        return response;
    };
    if method == "POST" && rest == "/git-receive-pack" {
        fake.lock().unwrap().pushes.push((repo.to_owned(), token));
    }
    let mut command = Command::new(real_git());
    command
        .arg("http-backend")
        .env_clear()
        .env("GIT_PROJECT_ROOT", &root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", format!("/{repo}.git{rest}"))
        .env("QUERY_STRING", query)
        .env("REQUEST_METHOD", method)
        .env("REMOTE_USER", "x-access-token")
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (header, var) in [
        ("content-type", "CONTENT_TYPE"),
        ("content-encoding", "HTTP_CONTENT_ENCODING"),
        ("git-protocol", "GIT_PROTOCOL"),
    ] {
        if let Some(value) = headers.get(header) {
            command.env(var, value);
        }
    }
    let output = tokio::task::spawn_blocking(move || {
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(&body).unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    // A CGI answer: headers, a blank line, the body.
    let out = output.stdout;
    let split = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| (at, 4))
        .or_else(|| out.windows(2).position(|w| w == b"\n\n").map(|at| (at, 2)))
        .unwrap_or((out.len(), 0));
    let head = String::from_utf8_lossy(&out[..split.0]).into_owned();
    let mut response = Response::new(Full::new(Bytes::from(out[split.0 + split.1..].to_vec())));
    for line in head.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            let code = value.split(' ').next().and_then(|c| c.parse().ok()).unwrap_or(200);
            *response.status_mut() = hyper::StatusCode::from_u16(code).unwrap();
        } else if let (Ok(name), Ok(value)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            hyper::header::HeaderValue::from_str(value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

fn percent_decode(path: &str) -> String {
    url::form_urlencoded::parse(format!("p={}", path.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// The git this machine has, not the stand-in in front of it.
fn real_git() -> &'static str {
    ["/usr/bin/git", "/usr/local/bin/git", "/opt/homebrew/bin/git"]
        .into_iter()
        .find(|git| Path::new(git).exists())
        .expect("git is installed")
}

/// A fresh App key, as GitHub hands one out.
fn new_key() -> String {
    let out = Command::new("openssl")
        .args(["genrsa", "2048"])
        .stderr(Stdio::null())
        .output()
        .expect("openssl is installed");
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

/// A PEM key's public half, as GitHub keeps it.
fn public_key(pem: &str) -> Vec<u8> {
    let begin = pem.find("-----BEGIN ").unwrap();
    let rest = &pem[begin + "-----BEGIN ".len()..];
    let (label, rest) = rest.split_once("-----").unwrap();
    let (body, _) = rest.split_once("-----END").unwrap();
    let body: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let der = STANDARD.decode(body).unwrap();
    let pair = match label {
        "RSA PRIVATE KEY" => ring::signature::RsaKeyPair::from_der(&der),
        _ => ring::signature::RsaKeyPair::from_pkcs8(&der),
    };
    pair.unwrap().public().as_ref().to_vec()
}

// ---------------------------------------------------------------------------
// The world a test runs in.

struct World {
    dir: tempfile::TempDir,
    github: GitHub,
}

impl World {
    async fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new().prefix("nodus-gh-").tempdir_in("/tmp").unwrap();
        let root = dir.path().canonicalize().unwrap();
        for folder in ["home", "bin", "log", "data", "run", "sock", "github/acme", "shims"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let pem = new_key();
        let github = GitHub::start(root.join("github"), &pem).await;
        let world = Self { dir, github };
        world.stand_ins();
        world.set_up(&pem);
        world.repositories();
        world
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn data(&self) -> PathBuf {
        self.root().join("data")
    }

    fn log(&self) -> PathBuf {
        self.root().join("log")
    }

    /// The stand-in `claude`, which says where it started and with what,
    /// and a `git` that writes down its parent before running the real one.
    fn stand_ins(&self) {
        let log = self.log();
        let log = log.display();
        stand_in(
            &self.root().join("bin/claude"),
            &format!(
                r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "9.9.9 (Claude Code)"; exit 0; fi
{{ pwd; [ -f .nodus/ran ] && echo setup-ran; }} > "{log}/cwd.$NODUS_SESSION"
env > "{log}/env.$NODUS_SESSION"
echo "stand-in ready"
while IFS= read -r line; do echo "got: $line"; done
"#
            ),
        );
        stand_in(
            &self.root().join("bin/git"),
            &format!(
                "#!/bin/sh\nps -o command= -p $PPID >> \"{log}/git-parents\"\nexec {} \"$@\"\n",
                real_git()
            ),
        );
        // SRV-141: the sessions' environment.
        std::fs::write(
            self.data().join("env"),
            format!(
                "PATH={}/bin:/usr/bin:/bin\nSHELL=/bin/sh\nLANG=en_US.UTF-8\n",
                self.root().display()
            ),
        )
        .unwrap();
        // SRV-101: one Claude account in the pool, so sessions can start.
        std::fs::write(
            self.data().join("claude-accounts.json"),
            r#"[{"id":"seed","label":"Test","token":"test-token","addedAt":1,"state":"ok"}]"#,
        )
        .unwrap();
    }

    /// The server as the wizard leaves it: the App and its key, and Alice
    /// and Bob signed in, each on a device.
    fn set_up(&self, pem: &str) {
        let data = self.data();
        let private = |name: &str, text: &str| {
            std::fs::write(data.join(name), text).unwrap();
            std::fs::set_permissions(data.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
        };
        private(
            "server.json",
            &json!({
                "name": "Box",
                "setup": [],
                "github": {
                    "appId": APP_ID, "slug": "nodus-box", "name": "Nodus · Box", "owner": "acme",
                    "htmlUrl": format!("{}/apps/nodus-box", self.github.url), "clientId": "Iv1.fake",
                    "clientSecret": "secret", "workflows": false, "createdAt": 1, "installations": [INSTALLATION]
                }
            })
            .to_string(),
        );
        private("github-app.pem", pem);
        let members = [(100, "alice", "Alice Liddell"), (200, "bob", "Bob")].map(|(github_id, login, name)| Member {
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
            &json!([device("d-alice", ALICE_TOKEN, 100), device("d-bob", BOB_TOKEN, 200)]).to_string(),
        );
    }

    /// `acme/shop`, with a `.nodus/setup` that leaves a mark and says what
    /// it did, and `acme/other`, on the fake GitHub.
    fn repositories(&self) {
        let work = self.root().join("shop-work");
        std::fs::create_dir_all(work.join(".nodus")).unwrap();
        std::fs::write(work.join("README.md"), "# Shop\n").unwrap();
        std::fs::write(work.join(".gitignore"), ".nodus/ran\n").unwrap();
        stand_in(
            &work.join(".nodus/setup"),
            "#!/bin/sh\necho installing\nsleep 1\ntouch .nodus/ran\necho \"set up on port $PORT\"\n",
        );
        plain_git(&work, &["init", "-q", "-b", "main"]);
        plain_git(&work, &["add", "-A"]);
        plain_git(
            &work,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "First",
            ],
        );
        for name in ["shop", "other"] {
            let bare = self.root().join(format!("github/acme/{name}.git"));
            plain_git(
                &self.root(),
                &[
                    "clone",
                    "-q",
                    "--bare",
                    &work.to_string_lossy(),
                    &bare.to_string_lossy(),
                ],
            );
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nodus-server"));
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.root().join("home"))
            .env("NODUS_USER_DATA", self.data())
            .env("NODUS_GITHUB_API_URL", &self.github.url)
            .env("NODUS_GITHUB_URL", &self.github.url)
            .env("NODUS_SHIMS", self.root().join("shims"));
        command
    }

    fn socket(&self) -> String {
        self.root().join("sock/host.sock").to_string_lossy().into_owned()
    }

    /// The session host, on its own, as `nodus-host.service` runs it.
    fn host(&self) -> Host {
        Host(
            self.command(&["host", "--socket", &self.socket()])
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    /// `serve`, connected to that host, once it's listening.
    fn serve(&self) -> Server {
        let run = self.root().join("run");
        let run = run.to_string_lossy();
        let socket = self.socket();
        let args = [
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--run-home",
            &run,
            "--host-socket",
            &socket,
        ];
        let mut child = self
            .command(&args)
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
        Server { child, url }
    }

    /// What a session's process was started with.
    async fn session_env(&self, session_id: &str) -> BTreeMap<String, String> {
        let file = self.log().join(format!("env.{session_id}"));
        let text = eventually(|| {
            std::fs::read_to_string(&file)
                .ok()
                .filter(|t| t.contains("NODUS_TOKEN"))
        })
        .await;
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    /// Adds `acme/shop` as Alice.
    async fn shop(&self, client: &Client) -> Project {
        call(
            client,
            Request::ProjectsAddGithub {
                full_name: "acme/shop".into(),
                name: None,
                color: None,
            },
        )
        .await
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

fn stand_in(file: &Path, script: &str) {
    std::fs::write(file, script).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn plain_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(real_git()).args(args).current_dir(dir).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn sha256_hex(text: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The session host, stopped when the test ends.
struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Server {
    child: Child,
    url: String,
}

impl Server {
    async fn connect(&self, token: &str) -> (Client, Events) {
        Client::connect(&self.url, token).await.unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn call<T: serde::de::DeserializeOwned>(client: &Client, request: Request) -> T {
    let name = request.name();
    client
        .call_as(&request)
        .await
        .unwrap_or_else(|err| panic!("{name}: {}", err.message))
}

/// Waits for `check` to give something, a few times a second.
async fn eventually<T>(mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(tokio::time::Instant::now() < deadline, "waited too long");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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

async fn new_card(client: &Client, project: &Project, name: &str) -> SessionCard {
    call(
        client,
        Request::SessionCreate {
            project_id: project.id.clone(),
            opts: serde_json::from_value(json!({ "x": 0, "y": 0, "name": name, "initialPrompt": "look" })).unwrap(),
        },
    )
    .await
}

/// Runs git as the card's terminal would: in its worktree, with exactly
/// its session's environment.
fn session_git(env: &BTreeMap<String, String>, dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(real_git())
        .args(args)
        .current_dir(dir)
        .env_clear()
        .envs(env)
        .output()
        .unwrap()
}

/// `nodus git-credential get` with a session's environment, as git runs it.
fn helper(env: &BTreeMap<String, String>, request: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nodus-server"))
        .arg("git-credential")
        .arg("get")
        .env_clear()
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(request.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// POSTs to the session host's hook port.
async fn post(port: &str, path: &str, headers: &[(&str, &str)], body: &str) -> u16 {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut request = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header("Content-Type", "application/json")
        .body(body.to_owned());
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.unwrap().status().as_u16()
}

fn worktree(card: &SessionCard) -> Worktree {
    card.node.worktree().expect("the card's worktree")
}

// ---------------------------------------------------------------------------

/// SRV-120, SRV-080, SRV-081, SRV-005
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn projects_come_from_the_apps_repositories_and_clone_through_the_host() {
    let world = World::new().await;
    let _host = world.host();
    let server = world.serve();
    let (alice, _events) = server.connect(ALICE_TOKEN).await;

    let repos: Vec<GithubRepo> = call(&alice, Request::GithubRepos {}).await;
    assert_eq!(
        repos,
        [
            GithubRepo {
                full_name: "acme/shop".into(),
                private: true,
                default_branch: "main".into(),
                installation_id: INSTALLATION,
            },
            GithubRepo {
                full_name: "acme/other".into(),
                private: false,
                default_branch: "trunk".into(),
                installation_id: INSTALLATION,
            },
        ]
    );

    let project = world.shop(&alice).await;
    let main = world.root().join("run/projects/shop/main");
    assert_eq!(project.name, "shop");
    assert_eq!(Path::new(&project.repo_path).canonicalize().unwrap(), main);
    assert!(main.join("README.md").exists());
    assert_eq!(
        project.github(),
        Some(ProjectGithub {
            full_name: "acme/shop".into(),
            default_branch: "main".into(),
            installation_id: INSTALLATION,
        })
    );
    assert!(FolderColor::ALL.contains(&project.color));

    // SRV-005: git ran in the session host, never in `serve`.
    let parents = std::fs::read_to_string(world.log().join("git-parents")).unwrap();
    assert!(parents.lines().all(|p| p.contains("host --socket")), "{parents}");
    // SRV-080: through a token for that repository, to read and write its
    // contents and nothing else.
    let minted = world.github.fake().minted.clone();
    assert!(
        minted
            .iter()
            .any(|m| m.repositories.as_deref() == Some(&["shop".to_owned()][..])
                && m.permissions == json!({ "contents": "write" })),
        "{minted:#?}"
    );
    // SRV-081: and never in the clone's config or its remote's address.
    let config = std::fs::read_to_string(main.join(".git/config")).unwrap();
    assert!(!config.contains("ghs_"), "{config}");
    assert!(
        config.contains(&format!("url = {}/acme/shop.git", world.github.url)),
        "{config}"
    );

    // The same repository again is the same project.
    let again: Project = call(
        &alice,
        Request::ProjectsAddGithub {
            full_name: "ACME/Shop".into(),
            name: Some("Again".into()),
            color: None,
        },
    )
    .await;
    assert_eq!(again.id, project.id);
    // One the App can't reach isn't cloned.
    let err = alice
        .call(&Request::ProjectsAddGithub {
            full_name: "acme/secret".into(),
            name: None,
            color: None,
        })
        .await
        .unwrap_err();
    assert!(err.message.contains("can't reach acme/secret"), "{}", err.message);
    let projects: Vec<Project> = call(&alice, Request::ProjectsList {}).await;
    assert_eq!(projects.len(), 1);
}

/// SRV-122, SRV-123, SRV-126, SRV-127, SRV-128, SRV-082, SRV-084, SRV-183
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_members_cards_get_their_own_worktrees_and_push_as_their_starters() {
    let world = World::new().await;
    let _host = world.host();
    let server = world.serve();
    let (alice, _a) = server.connect(ALICE_TOKEN).await;
    let (bob, _b) = server.connect(BOB_TOKEN).await;
    let project = world.shop(&alice).await;

    let a = new_card(&alice, &project, "Fix the login page").await;
    let b = new_card(&bob, &project, "Add search").await;
    let (id_a, id_b) = (a.node.session_id.clone(), b.node.session_id.clone());
    // SRV-183
    assert_eq!(a.node.created_by(), Some("alice"));
    assert_eq!(b.node.created_by(), Some("bob"));
    // SRV-122: a worktree and a branch each, cut from the default branch.
    let (wa, wb) = (worktree(&a), worktree(&b));
    assert_eq!(
        Path::new(&wa.path),
        world.root().join(format!("run/projects/shop/worktrees/{}", &id_a[..8]))
    );
    assert_eq!(wa.branch, format!("nodus/fix-the-login-page-{}", &id_a[..4]));
    assert_eq!(wb.branch, format!("nodus/add-search-{}", &id_b[..4]));
    assert_ne!(wa.path, wb.path);
    let head = |dir: &str| plain_git(Path::new(dir), &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head(&wa.path).trim(), wa.branch);
    assert_eq!(head(&wb.path).trim(), wb.branch);
    // SRV-127: a preview port each.
    let (pa, pb) = (a.node.preview_port().unwrap(), b.node.preview_port().unwrap());
    assert!((8100..=8999).contains(&pa) && (8100..=8999).contains(&pb) && pa != pb);

    // SRV-123: each started once the repo's setup had run in its worktree.
    let started = std::fs::read_to_string(world.log().join(format!("cwd.{id_a}")));
    let cwd = match started {
        Ok(text) => text,
        Err(_) => eventually(|| std::fs::read_to_string(world.log().join(format!("cwd.{id_a}"))).ok()).await,
    };
    assert_eq!(cwd.lines().next(), Some(wa.path.as_str()));
    assert!(cwd.contains("setup-ran"), "{cwd}");
    let setup_log = world
        .root()
        .join(format!("run/projects/shop/setup-logs/{}.log", &id_a[..8]));
    assert!(
        std::fs::read_to_string(setup_log)
            .unwrap()
            .contains(&format!("set up on port {pa}"))
    );
    let loaded = board(&alice, &project).await;
    let card = loaded.cards.iter().find(|c| c.node.session_id == id_a).unwrap();
    assert_eq!(card.node.setup(), None);
    // SRV-126: both folders trusted for Claude Code, in the host user's file.
    let trusted = std::fs::read_to_string(world.root().join("home/.claude.json")).unwrap();
    assert!(trusted.contains(&wa.path) && trusted.contains(&wb.path), "{trusted}");

    // SRV-141, SRV-143, SRV-082: each session's own environment.
    let env_a = world.session_env(&id_a).await;
    let env_b = world.session_env(&id_b).await;
    assert_ne!(env_a["NODUS_TOKEN"], env_b["NODUS_TOKEN"]);
    assert_eq!(env_a["GIT_AUTHOR_NAME"], "Alice Liddell");
    assert_eq!(env_a["GIT_AUTHOR_EMAIL"], "100+alice@users.noreply.github.com");
    assert_eq!(env_b["GIT_AUTHOR_EMAIL"], "200+bob@users.noreply.github.com");
    assert_eq!(env_a["GIT_COMMITTER_NAME"], "nodus-box[bot]");
    assert_eq!(
        env_a["GIT_COMMITTER_EMAIL"],
        "9999+nodus-box[bot]@users.noreply.github.com"
    );
    assert_eq!(env_a["PORT"], pa.to_string());
    assert_eq!(env_a["NODUS_PREVIEW_PORT"], pa.to_string());
    assert!(env_a["PATH"].starts_with(&world.root().join("shims").display().to_string()));
    // SRV-081: no token anywhere in it.
    assert!(env_a.values().all(|v| !v.contains("ghs_")));

    // SRV-084: a push from Alice's card goes through the helper, with her
    // as its author and the App's bot as its committer.
    std::fs::write(Path::new(&wa.path).join("login.txt"), "fixed\n").unwrap();
    let dir = Path::new(&wa.path);
    assert!(session_git(&env_a, dir, &["add", "login.txt"]).status.success());
    let committed = session_git(&env_a, dir, &["commit", "-q", "-m", "Fix the login page"]);
    assert!(
        committed.status.success(),
        "{}",
        String::from_utf8_lossy(&committed.stderr)
    );
    let pushed = session_git(&env_a, dir, &["push", "-q", "origin", "HEAD"]);
    assert!(pushed.status.success(), "{}", String::from_utf8_lossy(&pushed.stderr));
    assert_eq!(world.github.fake().pushes.len(), 1);
    assert_eq!(world.github.fake().pushes[0].0, "acme/shop");
    let bare = world.root().join("github/acme/shop.git");
    let who = plain_git(
        &bare,
        &[
            "log",
            "-1",
            "--format=%an <%ae>|%cn <%ce>",
            &format!("refs/heads/{}", wa.branch),
        ],
    );
    assert_eq!(
        who.trim(),
        "Alice Liddell <100+alice@users.noreply.github.com>|nodus-box[bot] <9999+nodus-box[bot]@users.noreply.github.com>"
    );

    // SRV-128: git panels for a card are its worktree's.
    let status: Option<GitStatus> = call(
        &bob,
        Request::GitStatus {
            project_id: project.id.clone(),
            session_id: Some(id_a.clone()),
        },
    )
    .await;
    assert_eq!(status.unwrap().branch.as_deref(), Some(wa.branch.as_str()));
    let status: Option<GitStatus> = call(
        &bob,
        Request::GitStatus {
            project_id: project.id.clone(),
            session_id: None,
        },
    )
    .await;
    assert_eq!(status.unwrap().branch.as_deref(), Some("main"));

    // SRV-183: what a member adds is theirs, whoever changes it after.
    let note: BoardNote = serde_json::from_value(json!({
        "id": "n1", "text": "Ship it", "color": "yellow", "x": 0, "y": 0, "createdAt": 1
    }))
    .unwrap();
    let _: Value = call(
        &alice,
        Request::BoardSaveNote {
            project_id: project.id.clone(),
            note: note.clone(),
        },
    )
    .await;
    let mut edited = note.clone();
    edited.text = "Ship it today".into();
    let _: Value = call(
        &bob,
        Request::BoardSaveNote {
            project_id: project.id.clone(),
            note: edited,
        },
    )
    .await;
    let loaded = board(&bob, &project).await;
    let saved = loaded.notes.iter().find(|n| n.id == "n1").unwrap();
    assert_eq!(saved.text, "Ship it today");
    assert_eq!(saved.created_by(), Some("alice"));
    // A session's board tools work for its starter.
    let port = &env_b["NODUS_PORT"];
    let token = format!("Bearer {}", env_b["NODUS_TOKEN"]);
    let created = post(
        port,
        "/mcp",
        &[("Authorization", &token), ("X-Nodus-Session", &id_b)],
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                 "params": { "name": "create_note", "arguments": { "text": "From Bob's session" } } })
        .to_string(),
    )
    .await;
    assert_eq!(created, 200);
    let loaded = board(&alice, &project).await;
    let from_tools = loaded.notes.iter().find(|n| n.text == "From Bob's session").unwrap();
    assert_eq!(from_tools.created_by(), Some("bob"));
}

/// SRV-084, SRV-143
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_sessions_own_token_gets_in_and_only_for_its_own_repository() {
    let world = World::new().await;
    let _host = world.host();
    let server = world.serve();
    let (alice, _a) = server.connect(ALICE_TOKEN).await;
    let project = world.shop(&alice).await;
    let a = new_card(&alice, &project, "One").await;
    let b = new_card(&alice, &project, "Two").await;
    let (id_a, id_b) = (a.node.session_id.clone(), b.node.session_id.clone());
    let env_a = world.session_env(&id_a).await;
    let env_b = world.session_env(&id_b).await;
    let host = world.github.host();

    // SRV-084: the helper answers for the session's own repository.
    let asked = |path: &str| format!("protocol=http\nhost={host}\npath={path}\n\n");
    let (out, err) = helper(&env_a, &asked("acme/shop.git"));
    assert!(out.starts_with("username=x-access-token\npassword=ghs_"), "{out}{err}");
    // Not another, though the App reaches that one too.
    let (out, err) = helper(&env_a, &asked("acme/other.git"));
    assert!(!out.contains("password=ghs"), "{out}");
    assert!(err.contains("Only acme/shop is answered here"), "{err}");
    // Only GitHub.
    let (out, err) = helper(&env_a, "protocol=https\nhost=gitlab.com\npath=acme/shop.git\n\n");
    assert!(
        !out.contains("password=ghs") && err.contains("is answered"),
        "{out}{err}"
    );
    // SRV-143: not with another session's token, nor with none.
    let mut swapped = env_a.clone();
    swapped.insert("NODUS_TOKEN".into(), env_b["NODUS_TOKEN"].clone());
    let (out, err) = helper(&swapped, &asked("acme/shop.git"));
    assert!(!out.contains("password=ghs"), "{out}{err}");
    let mut sessionless = env_a.clone();
    sessionless.remove("NODUS_SESSION");
    let (out, err) = helper(&sessionless, &asked("acme/shop.git"));
    assert!(
        !out.contains("password=ghs") && err.contains("isn't one this server gave"),
        "{out}{err}"
    );

    // SRV-083: `gh` in the session is the shim, which runs the real one
    // with a token for the session's repository, pull requests and issues too.
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_nodus-server"), world.root().join("shims/gh")).unwrap();
    stand_in(
        &world.root().join("bin/gh"),
        "#!/bin/sh\necho \"gh $* with $GH_TOKEN\"\n",
    );
    let shimmed = Command::new("gh")
        .args(["pr", "list"])
        .current_dir(&worktree(&a).path)
        .env_clear()
        .envs(&env_a)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&shimmed.stdout);
    assert!(
        said.starts_with("gh pr list with ghs_"),
        "{said}{}",
        String::from_utf8_lossy(&shimmed.stderr)
    );
    let token = said.trim().rsplit(' ').next().unwrap().to_owned();
    let minted = world.github.fake().minted.clone();
    let for_gh = minted.iter().find(|m| m.token == token).unwrap();
    assert_eq!(for_gh.repositories.as_deref(), Some(&["shop".to_owned()][..]));
    assert_eq!(
        for_gh.permissions,
        json!({ "contents": "write", "pull_requests": "write", "issues": "write" })
    );

    // SRV-143: hooks, the status line and the board tools take only the
    // session's own token. The host's shared one is for the app's sessions.
    let shared = walk(&world.data())
        .into_iter()
        .find(|f| f.file_name().is_some_and(|n| n == "hook.curlrc"))
        .map(|f| std::fs::read_to_string(f).unwrap())
        .and_then(|text| {
            text.split("Bearer ")
                .nth(1)
                .map(|t| t.trim().trim_end_matches('"').to_owned())
        })
        .expect("the host's shared token");
    let port = &env_a["NODUS_PORT"];
    let hook = format!("/hook?evt=Stop&sid={id_a}");
    for token in [shared.as_str(), env_b["NODUS_TOKEN"].as_str(), "nope"] {
        let bearer = format!("Bearer {token}");
        assert_eq!(post(port, &hook, &[("Authorization", &bearer)], "{}").await, 404);
        let status_line = format!("/hook?evt=StatusLine&sid={id_a}");
        assert_eq!(post(port, &status_line, &[("Authorization", &bearer)], "{}").await, 404);
        let mcp = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();
        let headers = [("Authorization", bearer.as_str()), ("X-Nodus-Session", id_a.as_str())];
        assert_eq!(post(port, "/mcp", &headers, &mcp).await, 401);
    }
    let own = format!("Bearer {}", env_a["NODUS_TOKEN"]);
    assert_eq!(post(port, &hook, &[("Authorization", &own)], "{}").await, 204);
    // The hook commands find the session's own token in a file of its own.
    let config = std::fs::read_to_string(&env_a["NODUS_HOOK_CONFIG"]).unwrap();
    assert!(
        config.contains(&env_a["NODUS_TOKEN"]) && !config.contains(&shared),
        "{config}"
    );
}

/// SRV-124
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_removed_cards_unsaved_work_is_kept_as_a_patch_and_a_bundle() {
    let world = World::new().await;
    let _host = world.host();
    let server = world.serve();
    let (alice, _a) = server.connect(ALICE_TOKEN).await;
    let project = world.shop(&alice).await;
    let dirty = new_card(&alice, &project, "Dirty").await;
    let clean = new_card(&alice, &project, "Clean").await;
    let env = world.session_env(&dirty.node.session_id).await;
    world.session_env(&clean.node.session_id).await;
    let wd = worktree(&dirty);
    let dir = Path::new(&wd.path);
    // A commit that was never pushed, a change and a new file.
    std::fs::write(dir.join("one.txt"), "committed\n").unwrap();
    assert!(session_git(&env, dir, &["add", "one.txt"]).status.success());
    assert!(session_git(&env, dir, &["commit", "-q", "-m", "One"]).status.success());
    std::fs::write(dir.join("README.md"), "# Shop, changed\n").unwrap();
    std::fs::write(dir.join("new.txt"), "untracked\n").unwrap();

    for card in [&dirty, &clean] {
        let _: Value = call(
            &alice,
            Request::BoardRemoveNode {
                project_id: project.id.clone(),
                session_id: card.node.session_id.clone(),
            },
        )
        .await;
    }
    let saved_root = world.root().join("run/projects/shop/saved");
    let saved: Vec<PathBuf> = std::fs::read_dir(&saved_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    let name = saved[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(&format!("{}-", dirty.node.session_id)), "{name}");
    let patch = std::fs::read_to_string(saved[0].join("changes.patch")).unwrap();
    assert!(
        patch.contains("Shop, changed") && patch.contains("untracked"),
        "{patch}"
    );
    let main = PathBuf::from(&project.repo_path);
    let bundle = saved[0].join("branch.bundle");
    let heads = plain_git(&main, &["bundle", "list-heads", &bundle.to_string_lossy()]);
    assert!(heads.contains(&wd.branch), "{heads}");
    plain_git(&main, &["bundle", "verify", "-q", &bundle.to_string_lossy()]);
    // The worktrees and their branches are gone.
    assert!(!dir.exists());
    assert!(!Path::new(&worktree(&clean).path).exists());
    let branches = plain_git(&main, &["branch", "--list", "nodus/*"]);
    assert!(branches.trim().is_empty(), "{branches}");
}

/// Every file under a folder.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
