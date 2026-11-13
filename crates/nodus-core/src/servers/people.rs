//! Setting a server up, signing in with GitHub, and a server's members,
//! devices, audit and GitHub App (SRV-040…045, SRV-060…063, SRV-165,
//! SRV-210), for the window: it never touches the network itself.
//!
//! A server is set up in two halves (SRV-060). Steps 1 and 2 of the wizard
//! go through a [`SetupSession`], made by claiming the setup code; the
//! setup token it holds dies once the first member signs in. From then on a
//! server is one this app has added, and its calls go through its
//! connection with this device's token, by the server's id.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{ServerInfo, ServerState};
use nodus_remote::wire::client::{self as http, Client, ConnectError};
use nodus_remote::wire::{
    self, AuditLine, Device, GithubApp, Hello, Installation, Member, Nonce, NonceRequest, Request, SetupStatus,
    SetupStep, SignInStarted,
};
use nodus_remote::{CheckError, DISCONNECTED, PROTOCOL, Remote};
use serde::de::DeserializeOwned;
use tokio::runtime::Handle;

use super::{ServersApi, server_url};
use crate::nodus::{OnCore, lock};
use crate::{CommandError, CommandResult};

/// How long a call to a server just added waits for its connection.
const CONNECT_WAIT: Duration = Duration::from_secs(10);

/// How long a setup call waits for its answer.
const SETUP_CALL_WAIT: Duration = Duration::from_secs(30);

/// Where avatars come from, and nowhere else (SRV-045).
const AVATAR_HOST: &str = "https://avatars.githubusercontent.com/";

/// How long fetching an avatar may take.
const AVATAR_WAIT: Duration = Duration::from_secs(15);

/// What a setup session says once its token is no good: used up by the
/// first sign-in, an hour idle, or cancelled by a newer code.
pub const SETUP_EXPIRED: &str = "This setup code is no longer good. Run nodus setup-code on the server for a new one.";

/// Where a sign-in stands, after one poll (SRV-041).
// One answer to one poll: its size doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum SignIn {
    /// Not yet: ask again after this many seconds.
    Pending { interval: u64 },
    /// Signed in: the server is added (or its token replaced) and
    /// connecting, with who signed in kept beside it.
    SignedIn { server: ServerInfo, member: Member },
    /// Refused, denied on GitHub or expired, in the server's words: ask
    /// for a new code to try again.
    Ended { message: String },
}

/// Wizard steps 1 and 2 with the setup token a claimed code gave
/// (SRV-060, SRV-061): the server's name, its GitHub App and the first
/// sign-in. Cheap to clone; every clone shares one WebSocket, opened when
/// first needed and again after it closes.
#[derive(Clone)]
pub struct SetupSession {
    inner: Arc<SetupInner>,
}

struct SetupInner {
    url: String,
    token: String,
    runtime: Handle,
    client: Mutex<Option<Arc<Client>>>,
}

impl std::fmt::Debug for SetupSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupSession")
            .field("url", &self.inner.url)
            .finish_non_exhaustive()
    }
}

impl SetupSession {
    fn new(url: String, token: String, runtime: Handle) -> Self {
        Self {
            inner: Arc::new(SetupInner {
                url,
                token,
                runtime,
                client: Mutex::new(None),
            }),
        }
    }

    /// The server's address, as it's kept: `http://host[:port]`.
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    /// The setup token: for steps 1 and 2 only, and never shown.
    pub fn token(&self) -> &str {
        &self.inner.token
    }

    /// `setup:status`: where the wizard stands, to resume it.
    pub async fn status(&self) -> CommandResult<SetupStatus> {
        self.call("setup:status", Request::SetupStatus {}).await
    }

    /// `setup:name`: step 1. The server trims it.
    pub async fn name(&self, name: &str) -> CommandResult<SetupStatus> {
        self.call("setup:name", Request::SetupName { name: name.to_owned() })
            .await
    }

    /// `github:installations`: the App's installations, as GitHub says now;
    /// step 2 waits for one.
    pub async fn installations(&self) -> CommandResult<Vec<Installation>> {
        self.call("github:installations", Request::GithubInstallations {}).await
    }

    /// A single-use nonce for making the App (SRV-062).
    pub async fn nonce(&self, workflows: bool) -> CommandResult<Nonce> {
        let (url, token) = (self.inner.url.clone(), self.inner.token.clone());
        OnCore::new(&self.inner.runtime, nonce(url, token, workflows)).await
    }

    /// The page that makes the App, for the browser: the server posts the
    /// manifest to GitHub from it, under `owner` (an organization) or the
    /// person's own account when it's blank.
    pub fn start_url(&self, nonce: &str, owner: &str) -> String {
        github_start_url(&self.inner.url, nonce, owner)
    }

    /// Starts the first sign-in (SRV-041), which this session's token lets
    /// make the first member.
    pub async fn start_sign_in(&self, device_name: &str) -> CommandResult<SignInStarted> {
        let (url, token, name) = (self.inner.url.clone(), self.inner.token.clone(), device_name.to_owned());
        OnCore::new(&self.inner.runtime, start(url, Some(token), name)).await
    }

    /// Closes its WebSocket.
    fn close(&self) {
        if let Some(client) = lock(&self.inner.client).take() {
            client.close();
        }
    }

    /// A call over the session's WebSocket, opened again once if it had
    /// closed. A token that's no good says [`SETUP_EXPIRED`].
    async fn call<T: DeserializeOwned>(&self, channel: &'static str, request: Request) -> CommandResult<T> {
        let inner = Arc::clone(&self.inner);
        OnCore::new(&self.inner.runtime, async move {
            for attempt in 0..2 {
                let open = lock(&inner.client).clone();
                let client = match open {
                    Some(client) => client,
                    None => {
                        let opened = Client::connect(&inner.url, &inner.token).await;
                        let client = Arc::new(opened.map_err(|err| CommandError::new(channel, connect_error(err)))?.0);
                        *lock(&inner.client) = Some(Arc::clone(&client));
                        client
                    }
                };
                let answer = tokio::time::timeout(SETUP_CALL_WAIT, client.call_as::<T>(&request)).await;
                match answer {
                    Ok(Ok(answer)) => return Ok(answer),
                    Ok(Err(err)) if err.message == DISCONNECTED && attempt == 0 => {
                        lock(&inner.client).take();
                    }
                    Ok(Err(err)) => return Err(CommandError::new(channel, err.message)),
                    Err(_) => return Err(CommandError::new(channel, nodus_remote::TIMED_OUT)),
                }
            }
            Err(CommandError::new(channel, DISCONNECTED))
        })
        .await
    }
}

/// Why connecting failed, as the window shows it.
fn connect_error(err: ConnectError) -> String {
    match err {
        ConnectError::Refused(401 | 403) => SETUP_EXPIRED.to_owned(),
        err => err.to_string(),
    }
}

async fn nonce(url: String, token: String, workflows: bool) -> CommandResult<Nonce> {
    http::github_nonce(&url, &token, &NonceRequest { workflows })
        .await
        .map_err(|err| CommandError::new("github:nonce", answered(err)))
}

async fn start(url: String, setup_token: Option<String>, device_name: String) -> CommandResult<SignInStarted> {
    http::start_sign_in(&url, setup_token.as_deref(), &device_name)
        .await
        .map_err(|err| CommandError::new("auth:start", answered(err)))
}

/// A route's failure in the server's words, or why it couldn't be reached.
fn answered(err: ConnectError) -> String {
    match err {
        ConnectError::Answered { message, .. } => message,
        ConnectError::Io(err) => format!("The server can’t be reached: {err}"),
        err => err.to_string(),
    }
}

/// `/github/setup/start?nonce=…&owner=…` on the server at `url`.
fn github_start_url(url: &str, nonce: &str, owner: &str) -> String {
    let encode = |text: &str| -> String {
        text.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    format!(
        "{url}{}?nonce={}&owner={}",
        wire::GITHUB_START_PATH,
        encode(nonce),
        encode(owner.trim())
    )
}

/// The page on GitHub that installs an App on repositories.
pub fn install_url(app: &GithubApp) -> String {
    format!("{}/installations/new", app.html_url.trim_end_matches('/'))
}

impl ServersApi<'_> {
    /// `GET /api/hello` at an address as typed (SRV-060): the server's
    /// name, version and whether it waits to be set up. Fails with `Not a
    /// server address: …`, `Can't reach a Nodus server at …`, or `This
    /// server runs Nodus <v>. Update it in Settings → Servers.`
    pub async fn hello(&self, url: &str) -> CommandResult<Hello> {
        const CHANNEL: &str = "servers:hello";
        let url = server_url(url).ok_or_else(|| CommandError::new(CHANNEL, format!("Not a server address: {url}")))?;
        let hello = self.nodus.on_core(http::hello(&url)).await.map_err(|err| {
            let reason = err.to_string();
            CommandError::new(
                CHANNEL,
                CheckError::Unreachable {
                    url: url.clone(),
                    reason,
                }
                .to_string(),
            )
        })?;
        if hello.protocol != PROTOCOL {
            return Err(CommandError::new(
                CHANNEL,
                CheckError::ProtocolMismatch(hello).to_string(),
            ));
        }
        Ok(hello)
    }

    /// `POST /api/setup/claim` (SRV-060): the setup code `nodus setup-code`
    /// printed, for a session that runs the wizard's first two steps. The
    /// core keeps it for this run, so [`ServersApi::setup_session`] gives
    /// it back when the wizard opens again. Fails in the server's words
    /// (a wrong or used code, too many tries, a server set up already).
    pub async fn claim(&self, url: &str, code: &str) -> CommandResult<SetupSession> {
        const CHANNEL: &str = "servers:claim";
        let url = server_url(url).ok_or_else(|| CommandError::new(CHANNEL, format!("Not a server address: {url}")))?;
        let code = code.trim();
        if code.is_empty() {
            return Err(CommandError::new(CHANNEL, "The setup code is needed"));
        }
        let claimed = self
            .nodus
            .on_core(http::claim_setup(&url, code))
            .await
            .map_err(|err| CommandError::new(CHANNEL, answered(err)))?;
        let session = SetupSession::new(url.clone(), claimed.token, self.nodus.inner.runtime.clone());
        if let Some(old) = lock(&self.nodus.inner.remotes.setups).insert(url, session.clone()) {
            old.close();
        }
        Ok(session)
    }

    /// The setup session claimed for this address earlier in this run, if
    /// there is one; its token may have died since.
    pub fn setup_session(&self, url: &str) -> Option<SetupSession> {
        let url = server_url(url)?;
        lock(&self.nodus.inner.remotes.setups).get(&url).cloned()
    }

    /// Forgets a setup session, when its token turned out to be no good.
    pub fn forget_setup(&self, url: &str) {
        if let Some(url) = server_url(url)
            && let Some(session) = lock(&self.nodus.inner.remotes.setups).remove(&url)
        {
            session.close();
        }
    }

    /// `POST /api/github/setup/nonce` (SRV-062) with a setup or device
    /// token: a single-use nonce for making the App, asking for `workflows:
    /// write` only when `workflows` is set.
    pub async fn github_nonce(&self, url: &str, token: &str, workflows: bool) -> CommandResult<Nonce> {
        let url =
            server_url(url).ok_or_else(|| CommandError::new("github:nonce", format!("Not a server address: {url}")))?;
        self.nodus.on_core(nonce(url, token.to_owned(), workflows)).await
    }

    /// `POST /api/auth/device` (SRV-041): a code for the person to type on
    /// GitHub, and the handle to poll with. While the server has no
    /// members, only the setup token lets this make the first one.
    pub async fn start_sign_in(
        &self,
        url: &str,
        device_name: &str,
        setup_token: Option<&str>,
    ) -> CommandResult<SignInStarted> {
        let url =
            server_url(url).ok_or_else(|| CommandError::new("auth:start", format!("Not a server address: {url}")))?;
        self.nodus
            .on_core(start(url, setup_token.map(str::to_owned), device_name.to_owned()))
            .await
    }

    /// `POST /api/auth/device/poll` (SRV-041), once. Signed in, the
    /// device's token is sealed and the server added with it, as
    /// [`ServersApi::add`] does, keeping who signed in (SRV-042); the token
    /// itself never leaves the core. A setup session for the address ends,
    /// since the first sign-in ends its token.
    pub async fn poll_sign_in(&self, url: &str, handle: &str) -> CommandResult<SignIn> {
        const CHANNEL: &str = "auth:poll";
        let url = server_url(url).ok_or_else(|| CommandError::new(CHANNEL, format!("Not a server address: {url}")))?;
        let polled = self
            .nodus
            .on_core(http::poll_sign_in(&url, handle))
            .await
            .map_err(|err| CommandError::new(CHANNEL, answered(err)))?;
        Ok(match polled {
            wire::SignInPoll::Pending { interval } => SignIn::Pending { interval },
            wire::SignInPoll::SignedIn { token, member, .. } => {
                self.forget_setup(&url);
                let server = self
                    .add_as(&url, &token, Some((&member.login, member.github_id)))
                    .await?;
                SignIn::SignedIn { server, member }
            }
            wire::SignInPoll::Refused { message }
            | wire::SignInPoll::Denied { message }
            | wire::SignInPoll::Expired { message } => SignIn::Ended { message },
        })
    }

    /// This computer's name, which a sign-in sends as the device's (SRV-042):
    /// macOS's ComputerName, Windows's `COMPUTERNAME`, or the host name.
    pub async fn device_name(&self) -> String {
        self.nodus
            .inner
            .runtime
            .spawn_blocking(device_name)
            .await
            .unwrap_or_else(|_| "This computer".to_owned())
    }

    /// A member's avatar (SRV-045), kept here once fetched: only from
    /// `avatars.githubusercontent.com`, never another address a server
    /// passes on. None for any other address, or until it's been fetched.
    pub async fn avatar(&self, url: &str) -> Option<PathBuf> {
        let file = self.nodus.inner.remotes.files.join("avatars").join(avatar_file(url)?);
        if file.exists() {
            return Some(file);
        }
        let url = url.to_owned();
        let kept = file.clone();
        // Fetched and written on the core's runtime: tokio's files need it.
        self.nodus
            .on_core(async move {
                let client = reqwest::Client::builder()
                    .timeout(AVATAR_WAIT)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .ok()?;
                let response = client.get(&url).send().await.ok()?;
                if !response.status().is_success() {
                    return None;
                }
                let bytes = response.bytes().await.ok()?;
                super::write_file(&kept, &bytes).await.ok()
            })
            .await?;
        Some(file)
    }

    // ---- an added server's own calls (SRV-165), with this device's token

    /// `setup:status` of an added server: the wizard resumes at step 3.
    pub async fn setup_status(&self, server_id: &str) -> CommandResult<SetupStatus> {
        self.server_call(server_id, "setup:status", Request::SetupStatus {})
            .await
    }

    /// `setup:done`: a step after step 2 finished or skipped (step 3,
    /// `members`, for now).
    pub async fn finish_step(&self, server_id: &str, step: SetupStep) -> CommandResult<SetupStatus> {
        self.server_call(server_id, "setup:done", Request::SetupDone { step })
            .await
    }

    /// `members:list`, in the order they were added (SRV-040).
    pub async fn members(&self, server_id: &str) -> CommandResult<Vec<Member>> {
        self.server_call(server_id, "members:list", Request::MembersList {})
            .await
    }

    /// `members:invite` (SRV-044): a GitHub login, which the server
    /// resolves to its id. Fails in the server's words for a login GitHub
    /// doesn't know or a member already there.
    pub async fn invite(&self, server_id: &str, login: &str) -> CommandResult<Member> {
        let login = login.trim().trim_start_matches('@').to_owned();
        if login.is_empty() {
            return Err(CommandError::new("members:invite", "A GitHub login is needed"));
        }
        self.server_call(server_id, "members:invite", Request::MembersInvite { login })
            .await
    }

    /// `members:remove` (SRV-040): the member and every device of theirs.
    /// Removing yourself signs this device out: the server is forgotten
    /// here, as Disconnect does.
    pub async fn remove_member(&self, server_id: &str, github_id: u64) -> CommandResult<()> {
        let yours = self.status(server_id).and_then(|s| s.github_id) == Some(github_id);
        self.server_call::<()>(server_id, "members:remove", Request::MembersRemove { github_id })
            .await?;
        if yours {
            self.remove(server_id).await;
        }
        Ok(())
    }

    /// `devices:list`: every member's, oldest first, this one marked
    /// `current` (SRV-042).
    pub async fn devices(&self, server_id: &str) -> CommandResult<Vec<Device>> {
        self.server_call(server_id, "devices:list", Request::DevicesList {})
            .await
    }

    /// `devices:revoke` (SRV-042): its socket closes at once. Revoking this
    /// device signs it out: the server is forgotten here, as Disconnect does.
    pub async fn revoke_device(&self, server_id: &str, device_id: &str) -> CommandResult<()> {
        let devices = self.devices(server_id).await?;
        let this = devices.iter().any(|d| d.id == device_id && d.current);
        self.server_call::<()>(
            server_id,
            "devices:revoke",
            Request::DevicesRevoke {
                id: device_id.to_owned(),
            },
        )
        .await?;
        if this {
            self.remove(server_id).await;
        }
        Ok(())
    }

    /// `audit:list` (SRV-210): the last 200 lines, newest first.
    pub async fn audit(&self, server_id: &str) -> CommandResult<Vec<AuditLine>> {
        let mut lines: Vec<AuditLine> = self.server_call(server_id, "audit:list", Request::AuditList {}).await?;
        lines.reverse();
        Ok(lines)
    }

    /// `server:info`: its name and version, its App and installations, and
    /// whether its machine is tagged on the tailnet (SRV-029).
    pub async fn info(&self, server_id: &str) -> CommandResult<wire::ServerInfo> {
        self.server_call(server_id, "server:info", Request::ServerInfo {}).await
    }

    /// `github:installations`: the App's installations, as GitHub says now.
    pub async fn installations(&self, server_id: &str) -> CommandResult<Vec<Installation>> {
        self.server_call(server_id, "github:installations", Request::GithubInstallations {})
            .await
    }

    /// `github:replace-key` (SRV-063): a new private key for the App, which
    /// the server checks with GitHub before it replaces the old one.
    pub async fn replace_key(&self, server_id: &str, pem: &str) -> CommandResult<GithubApp> {
        self.server_call(
            server_id,
            "github:replace-key",
            Request::GithubReplaceKey { pem: pem.to_owned() },
        )
        .await
    }

    /// A call to an added server, waiting a moment for its connection when
    /// it was only just added.
    async fn server_call<T: DeserializeOwned>(
        &self,
        server_id: &str,
        channel: &'static str,
        request: Request,
    ) -> CommandResult<T> {
        let remote = self
            .nodus
            .inner
            .remotes
            .get(server_id)
            .ok_or_else(|| CommandError::new(channel, format!("Unknown server {server_id}")))?;
        self.nodus
            .on_core(async move {
                connected(&remote).await;
                remote.call_as::<T>(&request).await
            })
            .await
            .map_err(|err| CommandError::new(channel, err.message))
    }
}

/// Waits, at most [`CONNECT_WAIT`], for a remote that's still connecting.
async fn connected(remote: &Remote) {
    if remote.is_connected() {
        return;
    }
    let mut status = remote.subscribe();
    let up = status.wait_for(|s| s.state != ServerState::Connecting);
    let _ = tokio::time::timeout(CONNECT_WAIT, up).await;
}

/// The file an avatar is kept in: its path on GitHub's avatar host, made
/// safe as a name. None for an address on any other host (SRV-045).
fn avatar_file(url: &str) -> Option<String> {
    let rest = url.strip_prefix(AVATAR_HOST)?;
    let name: String = rest
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    (!name.is_empty() && name.len() <= 128).then_some(name)
}

/// This computer's name (see [`ServersApi::device_name`]).
fn device_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        && out.status.success()
    {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !name.is_empty() {
            return name;
        }
    }
    if let Ok(name) = std::env::var("COMPUTERNAME")
        && !name.trim().is_empty()
    {
        return name.trim().to_owned();
    }
    if let Ok(out) = std::process::Command::new("hostname").output()
        && out.status.success()
    {
        let host = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let host = host.trim_end_matches(".local");
        if !host.is_empty() {
            return host.to_owned();
        }
    }
    "This computer".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_githubs_app_page_through_the_server() {
        assert_eq!(
            github_start_url("http://box:7420", "abc123", ""),
            "http://box:7420/github/setup/start?nonce=abc123&owner="
        );
        assert_eq!(
            github_start_url("http://box:7420", "n", " acme corp "),
            "http://box:7420/github/setup/start?nonce=n&owner=acme%20corp"
        );
    }

    #[test]
    fn fetches_avatars_only_from_githubs_avatar_host() {
        assert_eq!(
            avatar_file("https://avatars.githubusercontent.com/u/583231?v=4").as_deref(),
            Some("u-583231-v-4")
        );
        for elsewhere in [
            "https://evil.example/mallory.png",
            "http://avatars.githubusercontent.com/u/1",
            "https://avatars.githubusercontent.com.evil.example/u/1",
            "https://avatars.githubusercontent.com/",
        ] {
            assert_eq!(avatar_file(elsewhere), None, "{elsewhere}");
        }
    }

    #[test]
    fn installs_on_the_apps_own_page() {
        let app = GithubApp {
            app_id: 1,
            slug: "nodus-box".into(),
            name: "Nodus · Box".into(),
            owner: Some("octo".into()),
            html_url: "https://github.com/apps/nodus-box".into(),
            client_id: "Iv1.x".into(),
            workflows: false,
        };
        assert_eq!(install_url(&app), "https://github.com/apps/nodus-box/installations/new");
    }
}
