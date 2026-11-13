//! Setting a server up, signing in, members and devices (SRV-040…045,
//! SRV-060…063, SRV-210): the HTTP routes outside the WebSocket and what
//! they take and give, and the types the server's own calls answer with.
//!
//! Every body is JSON. A route that fails answers its HTTP status with
//! [`HttpError`], `{ "message": "…" }`, in words the app can show as they are.
//!
//! ```text
//! POST /api/setup/claim           { code }                 → { token }        (SRV-060)
//! POST /api/github/setup/nonce    Bearer setup or device   → { nonce, expiresAt }
//!                                 { workflows? }
//! GET  /github/setup/start?nonce=…&owner=<org or empty>    → a page that posts the manifest to GitHub
//! GET  /github/setup/callback?code=…&state=<nonce>         → a page: the App is made, or why not
//! POST /api/auth/device           { deviceName }           → { userCode, verificationUri, handle, interval, expiresIn }
//!                                 (Bearer setup token while there are no members)
//! POST /api/auth/device/poll      { handle }               → SignInPoll
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `POST`: [`SetupClaim`] → [`SetupClaimed`].
pub const SETUP_CLAIM_PATH: &str = "/api/setup/claim";
/// `POST`, with a setup or device token: [`NonceRequest`] → [`Nonce`].
pub const GITHUB_NONCE_PATH: &str = "/api/github/setup/nonce";
/// `GET ?nonce=…&owner=…`, opened in the browser.
pub const GITHUB_START_PATH: &str = "/github/setup/start";
/// `GET ?code=…&state=…`, where GitHub sends the browser back.
pub const GITHUB_CALLBACK_PATH: &str = "/github/setup/callback";
/// `POST`: [`SignInStart`] → [`SignInStarted`].
pub const SIGN_IN_PATH: &str = "/api/auth/device";
/// `POST`: [`SignInPollRequest`] → [`SignInPoll`].
pub const SIGN_IN_POLL_PATH: &str = "/api/auth/device/poll";

/// What a route that failed answers, with its HTTP status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpError {
    pub message: String,
}

// ---- setup (SRV-060, SRV-061)

/// The code `nodus setup-code` printed, as typed: `K7QM-4RXD`, any case,
/// with or without the dash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupClaim {
    pub code: String,
}

/// A setup token: for wizard steps 1 and 2 only, until an hour passes
/// without it being used, or until the first member signs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupClaimed {
    pub token: String,
}

/// The wizard's steps, in order (SRV-061).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetupStep {
    /// 1. The server's name.
    Name,
    /// 2. The GitHub App, installing it, and the first sign-in.
    Github,
    /// 3. Inviting members; can be skipped.
    Members,
    /// 4. Claude accounts (slice 5).
    Accounts,
    /// 5. The first project (slice 4).
    Project,
    /// Every step is done.
    Done,
}

impl SetupStep {
    /// The steps a wizard goes through, in order.
    pub const ALL: [Self; 5] = [Self::Name, Self::Github, Self::Members, Self::Accounts, Self::Project];
}

/// `setup:status`: where the wizard stands, so closing the app resumes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    /// Until step 2 makes the first member.
    pub needs_setup: bool,
    /// The first step not done yet.
    pub step: SetupStep,
    /// The steps done, in order.
    pub done: Vec<SetupStep>,
    /// The server's name: the machine's until step 1 names it.
    pub name: String,
    /// The App, once step 2 has made it.
    pub github: Option<GithubApp>,
}

// ---- the GitHub App (SRV-062, SRV-063)

/// What the app asks with a nonce.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NonceRequest {
    /// "Let sessions change GitHub Actions workflows": asks for `workflows:
    /// write`. Off unless asked (SRV-062).
    #[serde(default)]
    pub workflows: bool,
}

/// A single-use nonce for [`GITHUB_START_PATH`], and GitHub's `state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nonce {
    pub nonce: String,
    /// Milliseconds since the epoch: 10 minutes from when it was given.
    pub expires_at: f64,
}

/// The server's GitHub App, without anything secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubApp {
    pub app_id: u64,
    pub slug: String,
    pub name: String,
    /// The account or organization that owns it.
    pub owner: Option<String>,
    /// Its page on GitHub.
    pub html_url: String,
    /// Public: the device flow names it.
    pub client_id: String,
    /// Whether it was given `workflows: write`.
    pub workflows: bool,
}

/// One installation of the App.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub id: u64,
    /// The account or organization it's installed on.
    pub account: String,
    /// `User` or `Organization`.
    pub account_type: String,
    /// `all` or `selected`.
    pub repository_selection: String,
    pub html_url: String,
}

// ---- signing in (SRV-041)

/// Starts a sign-in. While the server has no members, only a request with
/// the setup token can make the first one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInStart {
    /// The computer's name, shown in Devices.
    pub device_name: String,
}

/// The code to show, and the handle to poll with: random, never the code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInStarted {
    pub user_code: String,
    /// `https://github.com/login/device`.
    pub verification_uri: String,
    pub handle: String,
    /// Seconds to wait between polls.
    pub interval: u64,
    /// Seconds until the code expires.
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignInPollRequest {
    pub handle: String,
}

/// Where a sign-in stands. `pending` and `signed-in` are HTTP 200,
/// `refused` and `denied` 403, `expired` 410.
// One answer to one poll: its size doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum SignInPoll {
    /// Not yet: poll again after `interval` seconds.
    Pending { interval: u64 },
    /// The device's token: `Authorization: Bearer` from now on. Shown once.
    SignedIn {
        token: String,
        member: Member,
        device: Device,
    },
    /// `<login> isn't a member of this server. Ask a member to invite you.`
    Refused { message: String },
    /// The person said no on GitHub.
    Denied { message: String },
    /// The code expired, or the handle isn't one the server gave.
    Expired { message: String },
}

// ---- people (SRV-040, SRV-042)

/// A member, matched by `githubId`; `login` is for showing, refreshed at
/// each sign-in (SRV-040).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub github_id: u64,
    pub login: String,
    pub name: Option<String>,
    /// Only on `avatars.githubusercontent.com` (SRV-045).
    pub avatar_url: Option<String>,
    /// Who invited them; none for the first member.
    pub added_by: Option<String>,
    pub added_at: f64,
}

/// A device's Tailscale identity, from `tailscale whois` when it signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleIdentity {
    /// The tailnet user, `someone@example.com`.
    pub user: Option<String>,
    /// The machine's name on the tailnet.
    pub node: Option<String>,
}

/// A signed-in device (SRV-042). Its token is never shown again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub github_id: u64,
    /// Its member's login.
    pub login: String,
    pub tailscale: Option<TailscaleIdentity>,
    pub created_at: f64,
    pub last_seen_at: f64,
    /// Whether it's the device asking.
    #[serde(default)]
    pub current: bool,
}

/// One line of `audit.jsonl` (SRV-210).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLine {
    /// Milliseconds since the epoch.
    pub time: f64,
    /// `member.signed-in`, `device.revoked`, `github.key-replaced`, …
    pub event: String,
    /// Who did it, when a member did.
    pub member: Option<AuditMember>,
    /// From which device.
    pub device: Option<AuditDevice>,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditMember {
    pub github_id: u64,
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditDevice {
    pub id: String,
    pub name: String,
}

// ---- the server

/// `server:info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub needs_setup: bool,
    pub github: Option<GithubApp>,
    /// The App's installations, as GitHub says now.
    pub installations: Vec<Installation>,
    /// Why they couldn't be asked, when they couldn't.
    pub installations_error: Option<String>,
    pub tailscale: TailscaleStatus,
}

/// This machine on the tailnet (SRV-029).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleStatus {
    /// Whether `tailscale status` answered.
    pub running: bool,
    /// Whether the machine carries `tag:nodus`. Settings warn when not.
    pub tagged: bool,
    /// Its tags.
    pub tags: Vec<String>,
}

/// The tag a server's machine carries on the tailnet (SRV-029).
pub const TAILSCALE_TAG: &str = "tag:nodus";

/// `{ "ev": "server-notice", "p": { "kind", "message" } }`: something every
/// connected member is told, such as a new device signing in (SRV-042).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// `device-signed-in`.
    pub kind: String,
    /// `Saleem signed in on Studio`.
    pub message: String,
}

/// The event name a [`Notice`] goes by.
pub const NOTICE_EVENT: &str = "server-notice";

/// A notice's kind: a device signed in for the first time.
pub const DEVICE_SIGNED_IN: &str = "device-signed-in";
