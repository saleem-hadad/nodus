//! Setting a server up and signing in, against a fake GitHub: the real
//! binary on loopback with a scratch data folder, and a fake GitHub in this
//! process that does the device flow, `/user`, `/users/<login>`, the
//! manifest conversion and the App's own calls, checking the App's JWT
//! against its key. `NODUS_GITHUB_API_URL` and `NODUS_GITHUB_URL` point the
//! server at it. Keys are made by `openssl genrsa` as a test runs; none is
//! kept in git. A stand-in `tailscale` says the machine is tagged.
#![cfg(unix)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{BufRead, BufReader};
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
use nodus_core::domain::ServerState;
use nodus_remote::{Remote, RemoteEvent, RemoteEvents};
use nodus_wire::client::{
    Client, ConnectError, Events, claim_setup, download, github_nonce, hello, poll_sign_in, start_sign_in, upload,
};
use nodus_wire::{
    AuditLine, DEVICE_SIGNED_IN, Device, GithubApp, Installation, Member, NonceRequest, Request, ServerInfo,
    SetupStatus, SetupStep, SignInPoll,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(20);

const APP_ID: u64 = 4242;
const CLIENT_ID: &str = "Iv1.fakeclient";
const CLIENT_SECRET: &str = "client-secret-never-shown";
const WEBHOOK_SECRET: &str = "webhook-secret-never-shown";

/// The people on the fake GitHub.
const OCTO: u64 = 1;
const ALICE: u64 = 100;
const MALLORY: u64 = 666;
/// Whoever took `alice` once Alice renamed herself.
const NEW_ALICE: u64 = 200;

// ---------------------------------------------------------------------------
// A fake GitHub.

#[derive(Clone)]
struct GitHub {
    url: String,
    fake: Arc<Mutex<Fake>>,
}

struct Fake {
    url: String,
    users: Vec<(u64, String, String)>,
    flows: HashMap<String, Flow>,
    tokens: HashMap<String, u64>,
    counter: u32,
    /// Codes GitHub's App page would send back, and the App each makes.
    manifest_codes: HashMap<String, String>,
    /// Every code the server asked to convert.
    conversions: Vec<String>,
    app: Option<App>,
    installations: Vec<Value>,
    /// Whether nobody ticked Enable Device Flow on the App's page.
    device_flow_off: bool,
}

struct Flow {
    user_code: String,
    user: Option<u64>,
}

struct App {
    /// The public halves of the keys GitHub takes for it.
    keys: Vec<Vec<u8>>,
}

impl GitHub {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fake = Arc::new(Mutex::new(Fake {
            url: url.clone(),
            users: vec![
                (OCTO, "octo".into(), "Octo Cat".into()),
                (ALICE, "alice".into(), "Alice".into()),
                (MALLORY, "mallory".into(), String::new()),
            ],
            flows: HashMap::new(),
            tokens: HashMap::new(),
            counter: 0,
            manifest_codes: HashMap::new(),
            conversions: Vec::new(),
            app: None,
            installations: Vec::new(),
            device_flow_off: false,
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

    /// The person types the code on github.com/login/device and says yes.
    fn approve(&self, user_code: &str, user: u64) {
        let mut fake = self.fake();
        let flow = fake.flows.values_mut().find(|flow| flow.user_code == user_code);
        flow.expect("a code GitHub gave").user = Some(user);
    }

    /// GitHub's App page made an App from the manifest: the code it sends
    /// the browser back with.
    fn app_page_made(&self, code: &str, pem: &str) {
        self.fake().manifest_codes.insert(code.into(), pem.into());
    }

    /// Alice renames herself, and someone else takes her old login.
    fn rename_alice(&self) {
        let mut fake = self.fake();
        for user in &mut fake.users {
            if user.0 == ALICE {
                user.1 = "alice-renamed".into();
            }
        }
        fake.users.push((NEW_ALICE, "alice".into(), "Not Alice".into()));
    }
}

async fn answer(fake: Arc<Mutex<Fake>>, req: HttpRequest<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().to_string();
    let path = req.uri().path().to_owned();
    let bearer = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(str::to_owned);
    let body = req.into_body().collect().await.unwrap().to_bytes();
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let (status, answer) = fake.lock().unwrap().answer(&method, &path, bearer.as_deref(), &form);
    let mut reply = Response::new(Full::new(Bytes::from(answer.to_string())));
    *reply.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
    reply
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    Ok(reply)
}

impl Fake {
    fn answer(
        &mut self,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        form: &HashMap<String, String>,
    ) -> (u16, Value) {
        match (method, path) {
            ("POST", "/login/device/code") => {
                if form.get("client_id").map(String::as_str) != Some(CLIENT_ID) {
                    return (400, json!({ "error": "incorrect_client_credentials" }));
                }
                if self.device_flow_off {
                    return (200, json!({ "error": "device_flow_disabled" }));
                }
                self.counter += 1;
                let device_code = format!("device-{}", self.counter);
                let user_code = format!("CODE-{:04}", self.counter);
                self.flows.insert(
                    device_code.clone(),
                    Flow {
                        user_code: user_code.clone(),
                        user: None,
                    },
                );
                (
                    200,
                    json!({
                        "device_code": device_code,
                        "user_code": user_code,
                        "verification_uri": format!("{}/login/device", self.url),
                        "expires_in": 900,
                        "interval": 1,
                    }),
                )
            }
            ("POST", "/login/oauth/access_token") => {
                assert_eq!(form["grant_type"], "urn:ietf:params:oauth:grant-type:device_code");
                assert_eq!(form["client_id"], CLIENT_ID);
                let code = &form["device_code"];
                match self.flows.get(code).map(|flow| flow.user) {
                    None => (200, json!({ "error": "expired_token" })),
                    Some(None) => (200, json!({ "error": "authorization_pending" })),
                    Some(Some(user)) => {
                        self.flows.remove(code);
                        self.counter += 1;
                        let token = format!("gho_user{user}_{}", self.counter);
                        self.tokens.insert(token.clone(), user);
                        (
                            200,
                            json!({ "access_token": token, "token_type": "bearer", "scope": "" }),
                        )
                    }
                }
            }
            ("GET", "/user") => match bearer.and_then(|token| self.tokens.get(token)) {
                Some(&id) => (200, self.user_json(id)),
                None => (401, json!({ "message": "Bad credentials" })),
            },
            ("GET", path) if path.starts_with("/users/") => {
                let login = &path["/users/".len()..];
                match self.users.iter().find(|user| user.1.eq_ignore_ascii_case(login)) {
                    Some(user) => (200, self.user_json(user.0)),
                    None => (404, json!({ "message": "Not Found" })),
                }
            }
            ("POST", path) if path.starts_with("/app-manifests/") && path.ends_with("/conversions") => {
                let code = path["/app-manifests/".len()..path.len() - "/conversions".len()].to_owned();
                self.conversions.push(code.clone());
                let Some(pem) = self.manifest_codes.remove(&code) else {
                    return (404, json!({ "message": "Not Found" }));
                };
                self.app = Some(App {
                    keys: vec![public_key(&pem)],
                });
                (
                    201,
                    json!({
                        "id": APP_ID,
                        "slug": "nodus-box",
                        "name": "Nodus · Box",
                        "owner": { "login": "octo", "type": "User" },
                        "html_url": format!("{}/apps/nodus-box", self.url),
                        "client_id": CLIENT_ID,
                        "client_secret": CLIENT_SECRET,
                        "webhook_secret": WEBHOOK_SECRET,
                        "pem": pem,
                    }),
                )
            }
            ("GET", "/app/installations") if self.app_jwt(bearer) => (200, Value::Array(self.installations.clone())),
            ("GET", "/app") if self.app_jwt(bearer) => (200, json!({ "id": APP_ID, "slug": "nodus-box" })),
            ("GET", "/app" | "/app/installations") => {
                (401, json!({ "message": "A JSON web token could not be decoded" }))
            }
            _ => (404, json!({ "message": "Not Found" })),
        }
    }

    fn user_json(&self, id: u64) -> Value {
        let user = self.users.iter().find(|user| user.0 == id).unwrap();
        // SRV-045: Mallory's avatar is somewhere else, and isn't shown.
        let avatar = if id == MALLORY {
            "https://evil.example/mallory.png".to_owned()
        } else {
            format!("https://avatars.githubusercontent.com/u/{id}?v=4")
        };
        let name = (!user.2.is_empty()).then(|| user.2.clone());
        json!({ "id": id, "login": user.1, "name": name, "avatar_url": avatar })
    }

    /// Whether `bearer` is a JWT signed by one of the App's keys, for the
    /// App, and in date, as GitHub checks it.
    fn app_jwt(&self, bearer: Option<&str>) -> bool {
        let (Some(app), Some(jwt)) = (&self.app, bearer) else {
            return false;
        };
        let parts: Vec<&str> = jwt.split('.').collect();
        let [header, claims, signature] = parts[..] else {
            return false;
        };
        let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
        let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if header["alg"] != "RS256"
            // The App's id, as a string.
            || claims["iss"].as_str().and_then(|iss| iss.parse().ok()) != Some(APP_ID)
            || claims["exp"].as_u64().is_none_or(|exp| exp <= now || exp > now + 600)
            || claims["iat"].as_u64().is_none_or(|iat| iat > now)
        {
            return false;
        }
        let signed = format!("{}.{}", parts[0], parts[1]);
        let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
        app.keys.iter().any(|key| {
            ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, key)
                .verify(signed.as_bytes(), &signature)
                .is_ok()
        })
    }
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
// The server.

struct World {
    dir: tempfile::TempDir,
    github: GitHub,
}

impl World {
    async fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-auth-")
            .tempdir_in("/tmp")
            .unwrap();
        let world = Self {
            dir,
            github: GitHub::start().await,
        };
        let root = world.root();
        for folder in ["home", "bin", "data"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        // SRV-029: a machine tagged on its tailnet.
        stand_in(
            &root.join("bin/tailscale"),
            r#"#!/bin/sh
case "$1" in
  status) echo '{"Self":{"DNSName":"box.tail1234.ts.net.","Tags":["tag:nodus"]}}' ;;
  *) exit 1 ;;
esac
"#,
        );
        world
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn data(&self) -> PathBuf {
        self.root().join("data")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nodus-server"));
        command
            .args(args)
            .env_clear()
            .env(
                "PATH",
                format!("{}/bin:/usr/bin:/bin:/usr/sbin:/sbin", self.root().display()),
            )
            .env("HOME", self.root().join("home"))
            .env("NODUS_USER_DATA", self.data())
            .env("NODUS_GITHUB_API_URL", &self.github.url)
            .env("NODUS_GITHUB_URL", &self.github.url);
        command
    }

    /// `serve` on loopback, once it's listening.
    fn serve(&self, extra: &[&str]) -> Server {
        let home = self.root().join("home");
        let home = home.to_string_lossy();
        let mut args = vec!["serve", "--bind", "127.0.0.1:0", "--run-home", &home];
        args.extend_from_slice(extra);
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
        let port = url.rsplit(':').next().unwrap().parse().unwrap();
        Server { child, url, port }
    }

    /// `nodus-server setup-code`.
    fn setup_code(&self) -> String {
        let out = self.command(&["setup-code"]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.data().join(name)).unwrap()
    }

    fn mode(&self, name: &str) -> u32 {
        std::fs::metadata(self.data().join(name)).unwrap().permissions().mode() & 0o777
    }

    /// The whole of steps 1 and 2, as the wizard runs them: claim, name,
    /// make the App, and Octo signs in first. Octo's device token.
    async fn set_up(&self, server: &Server) -> String {
        let setup = claim_setup(&server.url, &self.setup_code()).await.unwrap().token;
        let nonce = github_nonce(&server.url, &setup, &NonceRequest::default())
            .await
            .unwrap()
            .nonce;
        self.github.app_page_made("made-1", &new_key());
        let (status, _) = server
            .get(&format!("/github/setup/callback?code=made-1&state={nonce}"))
            .await;
        assert_eq!(status, 200);
        signed_in(&self.github, server, Some(&setup), "Studio", OCTO).await.0
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

struct Server {
    child: Child,
    url: String,
    port: u16,
}

impl Server {
    async fn connect(&self, token: &str) -> (Client, Events) {
        Client::connect(&self.url, token).await.unwrap()
    }

    /// `GET path`, as a browser on this machine asks: the status and body.
    async fn get(&self, path: &str) -> (u16, String) {
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            self.port
        );
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        tokio::time::timeout(WAIT, stream.read_to_end(&mut answer))
            .await
            .expect("an answer")
            .unwrap();
        let answer = String::from_utf8_lossy(&answer);
        let status = answer[9..12].parse().unwrap();
        let body = answer
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_owned())
            .unwrap_or_default();
        (status, body)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Signs in on GitHub as `user` from `device`: polls once before the person
/// says yes, then until the server says how it went.
async fn sign_in(github: &GitHub, server: &Server, setup: Option<&str>, device: &str, user: u64) -> SignInPoll {
    let started = start_sign_in(&server.url, setup, device).await.unwrap();
    assert!(started.verification_uri.ends_with("/login/device"));
    // SRV-041: the handle is the server's own, never the code.
    assert_ne!(started.handle, started.user_code);
    assert_eq!(started.handle.len(), 64);
    let first = poll_sign_in(&server.url, &started.handle).await.unwrap();
    assert!(matches!(first, SignInPoll::Pending { .. }), "{first:?}");
    github.approve(&started.user_code, user);
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let polled = poll_sign_in(&server.url, &started.handle).await.unwrap();
        if !matches!(polled, SignInPoll::Pending { .. }) {
            return polled;
        }
        assert!(tokio::time::Instant::now() < deadline, "still pending");
    }
}

/// [`sign_in`], which must work: the device token, the member and device.
async fn signed_in(
    github: &GitHub,
    server: &Server,
    setup: Option<&str>,
    device: &str,
    user: u64,
) -> (String, Member, Device) {
    match sign_in(github, server, setup, device, user).await {
        SignInPoll::SignedIn { token, member, device } => (token, member, device),
        other => panic!("not signed in: {other:?}"),
    }
}

/// The first of a remote's events that's `wanted`, skipping the rest.
async fn remote_heard(events: &mut RemoteEvents, wanted: impl Fn(&RemoteEvent) -> bool) -> RemoteEvent {
    loop {
        let event = tokio::time::timeout(WAIT, events.next())
            .await
            .expect("the remote's event")
            .expect("the remote open");
        if wanted(&event) {
            return event;
        }
    }
}

async fn call<T: serde::de::DeserializeOwned>(client: &Client, request: Request) -> T {
    client.call_as(&request).await.unwrap()
}

async fn audit(client: &Client) -> Vec<AuditLine> {
    call(client, Request::AuditList {}).await
}

fn events(lines: &[AuditLine]) -> Vec<&str> {
    lines.iter().map(|line| line.event.as_str()).collect()
}

/// Turns `&quot;` and the rest back into text.
fn unescape(html: &str) -> String {
    html.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// What follows `before` in `text`, up to `until`.
fn between<'a>(text: &'a str, before: &str, until: &str) -> &'a str {
    let start = text.find(before).unwrap_or_else(|| panic!("{before} in {text}")) + before.len();
    let end = text[start..].find(until).unwrap() + start;
    &text[start..end]
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_wizard_claims_the_code_makes_the_app_and_its_first_member() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let github = &world.github;

    // SRV-060: a fresh server needs setting up, and its code.
    assert!(hello(&server.url).await.unwrap().needs_setup);
    let code = world.setup_code();
    let wrong = claim_setup(&server.url, "AAAA-AAAA").await.unwrap_err();
    assert!(matches!(wrong, ConnectError::Answered { status: 401, .. }), "{wrong:?}");
    let setup = claim_setup(&server.url, &code.to_lowercase()).await.unwrap().token;
    // Single-use.
    let again = claim_setup(&server.url, &code).await.unwrap_err();
    assert!(matches!(again, ConnectError::Answered { status: 401, .. }), "{again:?}");

    // Steps 1 and 2 over a socket with the setup token, and nothing else.
    let (wizard, _) = server.connect(&setup).await;
    let status: SetupStatus = call(&wizard, Request::SetupStatus {}).await;
    assert_eq!((status.step, status.needs_setup), (SetupStep::Name, true));
    let status: SetupStatus = call(&wizard, Request::SetupName { name: " Box ".into() }).await;
    assert_eq!((status.step, status.name.as_str()), (SetupStep::Github, "Box"));
    assert_eq!(hello(&server.url).await.unwrap().name, "Box");
    let err = wizard.call(&Request::MembersList {}).await.unwrap_err();
    assert!(err.message.starts_with("Sign in to do that"), "{}", err.message);
    let err = wizard.call(&Request::ProjectsList {}).await.unwrap_err();
    assert!(err.message.starts_with("Sign in to do that"), "{}", err.message);

    // SRV-062: a nonce needs a token, and the start page posts the manifest.
    let refused = github_nonce(&server.url, "nope", &NonceRequest::default())
        .await
        .unwrap_err();
    assert!(
        matches!(refused, ConnectError::Answered { status: 401, .. }),
        "{refused:?}"
    );
    let nonce = github_nonce(&server.url, &setup, &NonceRequest::default())
        .await
        .unwrap();
    assert_eq!(nonce.nonce.len(), 32);
    let (status, page) = server
        .get(&format!("/github/setup/start?nonce={}&owner=", nonce.nonce))
        .await;
    assert_eq!(status, 200, "{page}");
    let action = unescape(between(&page, "action=\"", "\""));
    assert_eq!(
        action,
        format!("{}/settings/apps/new?state={}", github.url, nonce.nonce)
    );
    let manifest: Value = serde_json::from_str(&unescape(between(&page, "name=\"manifest\" value=\"", "\""))).unwrap();
    assert_eq!(manifest["name"], "Nodus · Box");
    assert_eq!(manifest["public"], false);
    assert_eq!(
        manifest["redirect_url"],
        format!("http://127.0.0.1:{}/github/setup/callback", server.port)
    );
    assert_eq!(manifest["default_permissions"]["contents"], "write");
    assert_eq!(manifest["default_permissions"]["checks"], "read");
    assert!(manifest["default_permissions"].get("workflows").is_none());
    assert!(manifest.get("hook_attributes").is_none());
    assert!(page.contains(".submit()"));
    // For an organization, and with workflows asked for.
    let asked = NonceRequest { workflows: true };
    let org = github_nonce(&server.url, &setup, &asked).await.unwrap().nonce;
    let (_, page) = server.get(&format!("/github/setup/start?nonce={org}&owner=acme")).await;
    let action = unescape(between(&page, "action=\"", "\""));
    assert_eq!(
        action,
        format!("{}/organizations/acme/settings/apps/new?state={org}", github.url)
    );
    let manifest: Value = serde_json::from_str(&unescape(between(&page, "name=\"manifest\" value=\"", "\""))).unwrap();
    assert_eq!(manifest["default_permissions"]["workflows"], "write");
    let (status, _) = server.get(&format!("/github/setup/start?nonce={org}&owner=../x")).await;
    assert_eq!(status, 400);

    // GitHub sends the browser back; the App is kept, its secrets with it.
    let pem = new_key();
    github.app_page_made("made-1", &pem);
    let (status, page) = server
        .get(&format!("/github/setup/callback?code=made-1&state={}", nonce.nonce))
        .await;
    assert_eq!(status, 200, "{page}");
    assert!(
        page.contains(&format!("{}/apps/nodus-box/installations/new", github.url)),
        "{page}"
    );
    for secret in [CLIENT_SECRET, WEBHOOK_SECRET, "PRIVATE KEY"] {
        assert!(!page.contains(secret), "{secret} in {page}");
    }
    let config: Value = serde_json::from_str(&world.file("server.json")).unwrap();
    assert_eq!(config["github"]["appId"], APP_ID);
    assert_eq!(config["github"]["clientSecret"], CLIENT_SECRET);
    assert_eq!(config["github"]["webhookSecret"], WEBHOOK_SECRET);
    assert_eq!(world.file("github-app.pem"), pem);
    assert_eq!(world.mode("github-app.pem"), 0o600);
    assert_eq!(world.mode("server.json"), 0o600);
    // What clients see of it has nothing secret.
    let answer = wizard.call(&Request::SetupStatus {}).await.unwrap();
    let status: SetupStatus = serde_json::from_value(answer.clone()).unwrap();
    let app: GithubApp = status.github.unwrap();
    assert_eq!(
        (app.app_id, app.slug.as_str(), app.workflows),
        (APP_ID, "nodus-box", false)
    );
    for secret in [CLIENT_SECRET, WEBHOOK_SECRET, "PRIVATE KEY"] {
        assert!(!answer.to_string().contains(secret));
    }

    // Installing: the wizard waits until the App's JWT finds one.
    let none: Vec<Installation> = call(&wizard, Request::GithubInstallations {}).await;
    assert!(none.is_empty());
    github.fake().installations.push(json!({
        "id": 77,
        "account": { "login": "octo", "type": "User" },
        "repository_selection": "selected",
        "html_url": "https://github.com/settings/installations/77",
    }));
    let found: Vec<Installation> = call(&wizard, Request::GithubInstallations {}).await;
    assert_eq!((found[0].id, found[0].account.as_str()), (77, "octo"));

    // SRV-061: only the setup token's holder becomes the first member.
    let refused = sign_in(github, &server, None, "Elsewhere", MALLORY).await;
    assert_eq!(
        refused,
        SignInPoll::Refused {
            message: "mallory isn't a member of this server. Ask a member to invite you.".into()
        }
    );
    let (token, octo, device) = signed_in(github, &server, Some(&setup), "Studio", OCTO).await;
    assert_eq!((octo.github_id, octo.login.as_str()), (OCTO, "octo"));
    assert_eq!(octo.added_by, None);
    assert_eq!(
        octo.avatar_url.as_deref(),
        Some("https://avatars.githubusercontent.com/u/1?v=4")
    );
    assert_eq!((device.name.as_str(), device.login.as_str()), ("Studio", "octo"));
    assert_eq!(token.len(), 64);
    assert!(!hello(&server.url).await.unwrap().needs_setup);

    // SRV-060: the setup token dies with the first member, socket and all.
    let closed = tokio::time::timeout(WAIT, wizard.closed()).await.expect("closed");
    assert_eq!(closed.code, Some(1008), "{closed:?}");
    let gone = Client::connect(&server.url, &setup).await.err();
    assert!(matches!(gone, Some(ConnectError::Refused(401))), "{gone:?}");
    let gone = github_nonce(&server.url, &setup, &NonceRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(gone, ConnectError::Answered { status: 401, .. }), "{gone:?}");
    let claimed = claim_setup(&server.url, &code).await.unwrap_err();
    assert!(
        matches!(claimed, ConnectError::Answered { status: 409, .. }),
        "{claimed:?}"
    );
    let out = world.command(&["setup-code"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("set up already"));

    // SRV-042: the device's token gets in; only its hash is kept.
    let (app, _) = server.connect(&token).await;
    let members: Vec<Member> = call(&app, Request::MembersList {}).await;
    assert_eq!(members, std::slice::from_ref(&octo));
    let status: SetupStatus = call(&app, Request::SetupStatus {}).await;
    assert_eq!(status.step, SetupStep::Members);
    assert_eq!(status.done, [SetupStep::Name, SetupStep::Github]);
    let status: SetupStatus = call(
        &app,
        Request::SetupDone {
            step: SetupStep::Members,
        },
    )
    .await;
    assert_eq!(status.step, SetupStep::Accounts);
    let devices = world.file("devices.json");
    assert!(!devices.contains(&token), "{devices}");
    assert!(devices.contains(&hex_sha256(&token)), "{devices}");
    for file in ["devices.json", "members.json", "audit.jsonl", "setup.json"] {
        assert_eq!(world.mode(file), 0o600, "{file}");
    }

    // SRV-210: what happened, in order, with who did it.
    let lines = audit(&app).await;
    assert_eq!(
        events(&lines),
        [
            "setup.claimed",
            "server.named",
            "github.app-created",
            "github.installations-changed",
            "member.refused",
            "member.added",
            "member.signed-in",
        ]
    );
    assert_eq!(lines[2].details["appId"], APP_ID);
    assert_eq!(lines[3].details["added"][0]["account"], "octo");
    assert_eq!(lines[4].details["login"], "mallory");
    let signed = &lines[6];
    assert_eq!(signed.member.as_ref().unwrap().login, "octo");
    assert_eq!(signed.device.as_ref().unwrap().name, "Studio");

    // SRV-029: the server's state, tagged on its tailnet.
    let info: ServerInfo = call(&app, Request::ServerInfo {}).await;
    assert_eq!(info.name, "Box");
    assert!(!info.needs_setup);
    assert_eq!(info.github.unwrap().slug, "nodus-box");
    assert_eq!(info.installations.len(), 1);
    assert!(info.tailscale.running && info.tailscale.tagged, "{:?}", info.tailscale);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_callback_needs_the_state_of_an_unused_nonce() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let github = &world.github;
    let setup = claim_setup(&server.url, &world.setup_code()).await.unwrap().token;
    github.app_page_made("made-1", &new_key());
    let nonce = github_nonce(&server.url, &setup, &NonceRequest::default())
        .await
        .unwrap()
        .nonce;

    // SRV-062: a missing, wrong or used state is turned away before
    // GitHub's code is exchanged, so nobody can hand the server an App.
    for path in [
        "/github/setup/callback?code=made-1".to_owned(),
        "/github/setup/callback?code=made-1&state=".to_owned(),
        "/github/setup/callback?code=made-1&state=0123456789abcdef0123456789abcdef".to_owned(),
        format!("/github/setup/callback?code=made-1&state={nonce}x"),
    ] {
        let (status, page) = server.get(&path).await;
        assert_eq!(status, 400, "{path}");
        assert!(page.contains("expired or was used already"), "{page}");
    }
    assert!(github.fake().conversions.is_empty());
    let (status, _) = server.get("/github/setup/start?nonce=nope").await;
    assert_eq!(status, 400);

    let (status, _) = server
        .get(&format!("/github/setup/callback?code=made-1&state={nonce}"))
        .await;
    assert_eq!(status, 200);
    // Used once, it's gone: as a state and for the start page.
    github.app_page_made("made-2", &new_key());
    let (status, _) = server
        .get(&format!("/github/setup/callback?code=made-2&state={nonce}"))
        .await;
    assert_eq!(status, 400);
    let (status, _) = server.get(&format!("/github/setup/start?nonce={nonce}")).await;
    assert_eq!(status, 400);
    assert_eq!(github.fake().conversions, ["made-1"]);

    // A code GitHub doesn't know makes no App, and uses the nonce up all the same.
    let nonce = github_nonce(&server.url, &setup, &NonceRequest::default())
        .await
        .unwrap()
        .nonce;
    let (status, page) = server
        .get(&format!("/github/setup/callback?code=unknown&state={nonce}"))
        .await;
    assert_eq!(status, 502, "{page}");
    let config: Value = serde_json::from_str(&world.file("server.json")).unwrap();
    assert_eq!(config["github"]["appId"], APP_ID);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn members_are_their_github_id_and_invited_by_login() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let github = &world.github;
    let token = world.set_up(&server).await;
    let (app, _) = server.connect(&token).await;

    // SRV-044: an invite resolves the login to its id.
    let alice: Member = call(&app, Request::MembersInvite { login: "@Alice".into() }).await;
    assert_eq!((alice.github_id, alice.login.as_str()), (ALICE, "alice"));
    assert_eq!(alice.added_by.as_deref(), Some("octo"));
    let err = app
        .call(&Request::MembersInvite { login: "alice".into() })
        .await
        .unwrap_err();
    assert_eq!(err.message, "alice is a member already");
    let err = app
        .call(&Request::MembersInvite { login: "nobody".into() })
        .await
        .unwrap_err();
    assert_eq!(err.message, "There's no GitHub account named nobody");
    let err = app
        .call(&Request::MembersInvite { login: "../app".into() })
        .await
        .unwrap_err();
    assert!(err.message.contains("isn't a GitHub login"), "{}", err.message);

    // SRV-062: the manifest can't turn the device flow on, and says so.
    github.fake().device_flow_off = true;
    let off = start_sign_in(&server.url, None, "Phone").await.unwrap_err();
    let ConnectError::Answered { status, message } = off else {
        panic!("{off:?}");
    };
    assert_eq!(status, 502);
    assert!(message.starts_with("Tick Enable Device Flow"), "{message}");
    github.fake().device_flow_off = false;

    // A non-member is refused, in the app's words.
    let refused = sign_in(github, &server, None, "Phone", MALLORY).await;
    assert_eq!(
        refused,
        SignInPoll::Refused {
            message: "mallory isn't a member of this server. Ask a member to invite you.".into()
        }
    );

    // SRV-040: Alice renames herself and someone else takes `alice`. The
    // login matches, the id doesn't: refused.
    github.rename_alice();
    let refused = sign_in(github, &server, None, "Laptop", NEW_ALICE).await;
    assert!(matches!(refused, SignInPoll::Refused { .. }), "{refused:?}");
    // Alice herself gets in, under her new login.
    let (_, member, _) = signed_in(github, &server, None, "Laptop", ALICE).await;
    assert_eq!((member.github_id, member.login.as_str()), (ALICE, "alice-renamed"));
    let members: Vec<Member> = call(&app, Request::MembersList {}).await;
    assert_eq!(members.len(), 2);
    assert_eq!(members[1].login, "alice-renamed");

    let lines = audit(&app).await;
    let invited = lines.iter().find(|line| line.event == "member.invited").unwrap();
    assert_eq!(invited.details["githubId"], ALICE);
    assert_eq!(invited.member.as_ref().unwrap().login, "octo");
    let refused: Vec<_> = lines.iter().filter(|line| line.event == "member.refused").collect();
    assert_eq!(refused.len(), 2);
    assert_eq!(refused[1].details["githubId"], NEW_ALICE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn devices_are_told_of_new_ones_and_close_when_revoked() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let github = &world.github;
    let token = world.set_up(&server).await;
    let (studio, mut studio_events) = server.connect(&token).await;
    // A socket answers once it's being served, so notices reach it.
    let _: Vec<Member> = call(&studio, Request::MembersList {}).await;
    // The app's remote too, which hands them out among its events.
    let (remote, mut remote_events) = Remote::start(&tokio::runtime::Handle::current(), &server.url, &token);
    remote_heard(
        &mut remote_events,
        |event| matches!(event, RemoteEvent::Status(status) if status.state == ServerState::Connected),
    )
    .await;
    remote.call(&Request::MembersList {}).await.unwrap();

    // SRV-042: a new device signing in tells every connected member.
    let (laptop_token, _, laptop) = signed_in(github, &server, None, "Laptop", OCTO).await;
    let notice = tokio::time::timeout(WAIT, studio_events.next_notice())
        .await
        .expect("a notice")
        .unwrap();
    assert_eq!(notice.kind, DEVICE_SIGNED_IN);
    assert_eq!(notice.message, "Octo Cat signed in on Laptop");
    let heard = remote_heard(&mut remote_events, |event| matches!(event, RemoteEvent::Notice(_))).await;
    assert_eq!(heard, RemoteEvent::Notice(notice));
    remote.close();
    let (laptop_app, _) = server.connect(&laptop_token).await;
    let devices: Vec<Device> = call(&studio, Request::DevicesList {}).await;
    let names: Vec<(&str, bool)> = devices.iter().map(|d| (d.name.as_str(), d.current)).collect();
    assert_eq!(names, [("Studio", true), ("Laptop", false)]);

    // Revoked, its socket closes at once and its token stops working.
    let () = call(&studio, Request::DevicesRevoke { id: laptop.id.clone() }).await;
    let closed = tokio::time::timeout(WAIT, laptop_app.closed()).await.expect("closed");
    assert_eq!(closed.code, Some(1008), "{closed:?}");
    let gone = Client::connect(&server.url, &laptop_token).await.err();
    assert!(matches!(gone, Some(ConnectError::Refused(401))), "{gone:?}");
    let err = studio
        .call(&Request::DevicesRevoke { id: laptop.id.clone() })
        .await
        .unwrap_err();
    assert_eq!(err.message, "No such device");

    // SRV-040: removing a member revokes their devices.
    let _: Member = call(&studio, Request::MembersInvite { login: "alice".into() }).await;
    let (alice_token, _, _) = signed_in(github, &server, None, "Alice's Mac", ALICE).await;
    let (alice_app, _) = server.connect(&alice_token).await;
    let () = call(&studio, Request::MembersRemove { github_id: ALICE }).await;
    let closed = tokio::time::timeout(WAIT, alice_app.closed()).await.expect("closed");
    assert_eq!(closed.code, Some(1008), "{closed:?}");
    let gone = Client::connect(&server.url, &alice_token).await.err();
    assert!(matches!(gone, Some(ConnectError::Refused(401))), "{gone:?}");
    let devices: Vec<Device> = call(&studio, Request::DevicesList {}).await;
    assert_eq!(devices.len(), 1);
    // The last member can't remove themselves.
    let err = studio
        .call(&Request::MembersRemove { github_id: OCTO })
        .await
        .unwrap_err();
    assert_eq!(err.message, "You're the last member, so you can't remove yourself");

    let lines = audit(&studio).await;
    let revoked = lines.iter().find(|line| line.event == "device.revoked").unwrap();
    assert_eq!(revoked.details["name"], "Laptop");
    assert_eq!(revoked.device.as_ref().unwrap().name, "Studio");
    let removed = lines.iter().find(|line| line.event == "member.removed").unwrap();
    assert_eq!(removed.details["login"], "alice");
    assert_eq!(removed.details["devices"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_setup_token_lasts_an_hour_unused_and_a_new_code_ends_it() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let setup = claim_setup(&server.url, &world.setup_code()).await.unwrap().token;
    let (wizard, _) = server.connect(&setup).await;
    let _: SetupStatus = call(&wizard, Request::SetupStatus {}).await;

    // SRV-060: an hour without being used, and it's done for.
    let file = world.data().join("setup.json");
    let mut kept: Value = serde_json::from_str(&world.file("setup.json")).unwrap();
    let used = kept["tokens"][0]["lastUsedAt"].as_f64().unwrap();
    kept["tokens"][0]["lastUsedAt"] = json!(used - 61.0 * 60.0 * 1000.0);
    std::fs::write(&file, kept.to_string()).unwrap();
    let err = wizard.call(&Request::SetupStatus {}).await.unwrap_err();
    assert!(
        err.message.starts_with("The setup token has expired"),
        "{}",
        err.message
    );
    let closed = tokio::time::timeout(WAIT, wizard.closed()).await.expect("closed");
    assert_eq!(closed.code, Some(1008));
    let gone = Client::connect(&server.url, &setup).await.err();
    assert!(matches!(gone, Some(ConnectError::Refused(401))), "{gone:?}");

    // A new code cancels every earlier code and token.
    let first = world.setup_code();
    let setup = claim_setup(&server.url, &first).await.unwrap().token;
    let (_wizard, _) = server.connect(&setup).await;
    let second = world.setup_code();
    let gone = Client::connect(&server.url, &setup).await.err();
    assert!(matches!(gone, Some(ConnectError::Refused(401))), "{gone:?}");

    // Ten tries an hour, right or wrong.
    for _ in 0..10 {
        let wrong = claim_setup(&server.url, "AAAA-AAAA").await.unwrap_err();
        assert!(matches!(wrong, ConnectError::Answered { status: 401, .. }), "{wrong:?}");
    }
    let limited = claim_setup(&server.url, &second).await.unwrap_err();
    assert!(
        matches!(limited, ConnectError::Answered { status: 429, .. }),
        "{limited:?}"
    );
    // A new code starts the count again.
    let third = world.setup_code();
    claim_setup(&server.url, &third).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_app_key_replaces_the_old_once_github_takes_it() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let github = &world.github;
    let token = world.set_up(&server).await;
    let (app, _) = server.connect(&token).await;
    let old = world.file("github-app.pem");

    // SRV-063: a key GitHub doesn't know for the App, or no key, changes nothing.
    let unknown = new_key();
    let err = app
        .call(&Request::GithubReplaceKey { pem: unknown.clone() })
        .await
        .unwrap_err();
    assert!(
        err.message.starts_with("GitHub didn't take that key"),
        "{}",
        err.message
    );
    let err = app
        .call(&Request::GithubReplaceKey { pem: "hello".into() })
        .await
        .unwrap_err();
    assert!(
        err.message.starts_with("That isn't a GitHub App private key"),
        "{}",
        err.message
    );
    assert_eq!(world.file("github-app.pem"), old);

    // Made on GitHub's App page, it's taken, and the App speaks with it.
    let new = new_key();
    github.fake().app.as_mut().unwrap().keys.push(public_key(&new));
    let replaced: GithubApp = call(&app, Request::GithubReplaceKey { pem: new.clone() }).await;
    assert_eq!(replaced.app_id, APP_ID);
    assert_eq!(world.file("github-app.pem"), new);
    assert_eq!(world.mode("github-app.pem"), 0o600);
    github.fake().app.as_mut().unwrap().keys.remove(0);
    let _: Vec<Installation> = call(&app, Request::GithubInstallations {}).await;
    let lines = audit(&app).await;
    let line = lines.last().unwrap();
    assert_eq!(line.event, "github.key-replaced");
    assert_eq!(line.member.as_ref().unwrap().login, "octo");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dev_token_is_only_for_loopback() {
    let world = World::new().await;
    // A server whose Tailscale address it would listen on takes none.
    stand_in(
        &world.root().join("bin/tailscale"),
        "#!/bin/sh\n[ \"$1\" = ip ] && echo 100.101.102.103 && exit 0\nexit 1\n",
    );
    let home = world.root().join("home");
    let out = world
        .command(&["serve", "--dev-token", "t", "--run-home", &home.to_string_lossy()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("--dev-token is for development on loopback only"),
        "{said}"
    );

    // On loopback it gets in, as no member.
    let server = world.serve(&["--dev-token", "a-dev-token"]);
    let (app, _) = server.connect("a-dev-token").await;
    let members: Vec<Member> = call(&app, Request::MembersList {}).await;
    assert!(members.is_empty());
    assert!(Client::connect(&server.url, "another").await.is_err());
}

/// SRV-163: the files' routes take a token as `/api/ws` does, but never a
/// setup token, and a revoked device's no more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_take_a_devices_token_but_never_a_setup_token() {
    let world = World::new().await;
    let server = world.serve(&[]);
    let github = &world.github;

    // SRV-060: a setup token that holds, for the wizard, is no good here.
    let setup = claim_setup(&server.url, &world.setup_code()).await.unwrap().token;
    let (wizard, _) = server.connect(&setup).await;
    let _: SetupStatus = call(&wizard, Request::SetupStatus {}).await;
    let refused = upload(&server.url, &setup, "a.txt", b"x").await.unwrap_err();
    assert!(matches!(refused, ConnectError::Refused(403)), "{refused:?}");
    assert_eq!(download(&server.url, &setup, "a.txt").await.unwrap().status, 403);
    drop(wizard);

    // A device's token keeps a file and gets it back.
    let token = world.set_up(&server).await;
    let kept = upload(&server.url, &token, "notes.txt", b"the bytes").await.unwrap();
    let back = download(&server.url, &token, &kept.id).await.unwrap();
    assert_eq!((back.status, back.body.as_slice()), (200, &b"the bytes"[..]));

    // Revoked, a device's token gets nothing; nor does one never given.
    let (laptop_token, _, laptop) = signed_in(github, &server, None, "Laptop", OCTO).await;
    let back = download(&server.url, &laptop_token, &kept.id).await.unwrap();
    assert_eq!(back.status, 200);
    let (studio, _) = server.connect(&token).await;
    let () = call(&studio, Request::DevicesRevoke { id: laptop.id }).await;
    let gone = download(&server.url, &laptop_token, &kept.id).await.unwrap();
    assert_eq!(gone.status, 401);
    let refused = upload(&server.url, "not-a-token", "a.txt", b"x").await.unwrap_err();
    assert!(matches!(refused, ConnectError::Refused(401)), "{refused:?}");
}

fn hex_sha256(text: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, text.as_bytes());
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
