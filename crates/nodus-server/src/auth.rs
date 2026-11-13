//! Who may use a server, and what they did (SRV-040…045, SRV-060…063,
//! SRV-210): the tokens a request carries, members and their devices, the
//! wizard's state, the GitHub App, the sockets open now, and the notices
//! every member hears.
//!
//! All members are equal: there's no per-call check beyond being one,
//! because a member can type into any terminal and so is in effect the
//! `nodus` user (SRV-040, §13).

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use nodus_wire::{
    AuditDevice, AuditMember, Device, Hello, Installation, Member, Nonce, Notice, PROTOCOL, ServerInfo, SetupStatus,
    SetupStep, SignInPoll, SignInStarted, TAILSCALE_TAG, TailscaleIdentity, TailscaleStatus,
};
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::app_key::AppKey;
use crate::audit::Audit;
use crate::github::{Conversion, GitHub, GitHubUser, TokenPoll};
use crate::net::{same_token, tailscale};
use crate::setup;
use crate::store::{
    self, APP_KEY_FILE, AppConfig, DEVICES_FILE, DeviceRecord, MEMBERS_FILE, People, SERVER_FILE, now_ms, random_hex,
    sha256_hex,
};

/// How long a nonce for the manifest flow lasts (SRV-022).
const NONCE_MS: f64 = 10.0 * 60.0 * 1000.0;
/// How often a device's last-seen time is written down at most.
const SEEN_EVERY_MS: f64 = 60.0 * 1000.0;
/// The longest server and device names.
const NAME_CHARS: usize = 64;
/// The only host avatars load from (SRV-045).
const AVATAR_HOST: &str = "avatars.githubusercontent.com";

/// Who a request or socket is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Who {
    /// `--dev-token`, which only a server on loopback takes: everything,
    /// as no member. For development and tests.
    Dev,
    /// A setup token: the wizard's steps 1 and 2 (SRV-060).
    Setup { token: String },
    /// A member's device (SRV-042).
    Device { id: String, github_id: u64 },
}

/// A nonce given and not yet used, and who asked for it.
struct NonceEntry {
    expires_at: f64,
    workflows: bool,
    asked_by: Who,
}

/// A sign-in waiting on GitHub, by its handle.
struct PendingSignIn {
    device_code: String,
    interval: u64,
    expires_at: f64,
    next_poll_at: f64,
    device_name: String,
    /// The setup token it started with, which lets it make the first member.
    setup_token: Option<String>,
}

/// An open socket.
struct Connection {
    who: Who,
    close: watch::Sender<Option<CloseFrame>>,
}

/// Why a route failed: its HTTP status and what to show.
pub(crate) type Refusal = (u16, String);

pub(crate) struct Auth {
    data: PathBuf,
    dev_token: Option<String>,
    machine_name: String,
    people: Mutex<People>,
    pub github: GitHub,
    pub audit: Audit,
    nonces: Mutex<HashMap<String, NonceEntry>>,
    sign_ins: Mutex<HashMap<String, PendingSignIn>>,
    connections: Mutex<HashMap<u64, Connection>>,
    next_connection: AtomicU64,
    notices: broadcast::Sender<Notice>,
}

impl Auth {
    /// Reads the people files in `data`. `dev_token` is `--dev-token`.
    pub(crate) fn open(data: &Path, dev_token: Option<String>, machine_name: String) -> Result<Self, String> {
        Ok(Self {
            data: data.to_path_buf(),
            dev_token,
            machine_name,
            people: Mutex::new(People::load(data)?),
            github: GitHub::from_env(),
            audit: Audit::new(data),
            nonces: Mutex::new(HashMap::new()),
            sign_ins: Mutex::new(HashMap::new()),
            connections: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(1),
            notices: broadcast::channel(64).0,
        })
    }

    // ---- who's asking

    /// Who a Bearer token is: the dev token, a device's, or a setup token
    /// while there's no member yet. A device is seen now.
    pub(crate) fn authenticate(&self, token: &str) -> Option<Who> {
        if self.dev_token.as_deref().is_some_and(|dev| same_token(token, dev)) {
            return Some(Who::Dev);
        }
        let hash = sha256_hex(token);
        let now = now_ms();
        {
            let mut people = self.lock();
            let found = people
                .devices
                .iter_mut()
                .find(|device| same_token(&device.token_hash, &hash));
            if let Some(device) = found {
                let who = Who::Device {
                    id: device.id.clone(),
                    github_id: device.github_id,
                };
                if now - device.last_seen_at >= SEEN_EVERY_MS {
                    device.last_seen_at = now;
                    self.save_devices(&people);
                }
                return Some(who);
            }
        }
        let setup = Who::Setup {
            token: token.to_owned(),
        };
        self.still_good(&setup).then_some(setup)
    }

    /// Whether what a socket signed in with still holds: a setup token
    /// dies an hour after it was last used and once there's a member
    /// (SRV-060); using it now keeps it another hour.
    pub(crate) fn still_good(&self, who: &Who) -> bool {
        match who {
            Who::Dev => true,
            Who::Setup { token } => {
                self.needs_setup() && setup::use_token(&self.data, token, now_ms()).unwrap_or(false)
            }
            Who::Device { id, .. } => self.lock().devices.iter().any(|device| device.id == *id),
        }
    }

    /// The GitHub login of the member a device belongs to: who made what
    /// they add (SRV-183). None for the dev and setup tokens.
    pub(crate) fn login_of(&self, who: &Who) -> Option<String> {
        let Who::Device { github_id, .. } = who else {
            return None;
        };
        self.member(*github_id).map(|member| member.login)
    }

    /// A member by their GitHub id.
    pub(crate) fn member(&self, github_id: u64) -> Option<Member> {
        self.lock()
            .members
            .iter()
            .find(|member| member.github_id == github_id)
            .cloned()
    }

    /// A member by their login, as a card records its starter (SRV-082).
    pub(crate) fn member_by_login(&self, login: &str) -> Option<Member> {
        self.lock()
            .members
            .iter()
            .find(|member| member.login.eq_ignore_ascii_case(login))
            .cloned()
    }

    /// Who did it, for the audit log.
    fn actor(&self, who: &Who) -> (Option<AuditMember>, Option<AuditDevice>) {
        let Who::Device { id, github_id } = who else {
            return (None, None);
        };
        let people = self.lock();
        let member = people
            .members
            .iter()
            .find(|member| member.github_id == *github_id)
            .map(|member| AuditMember {
                github_id: member.github_id,
                login: member.login.clone(),
            });
        let device = people
            .devices
            .iter()
            .find(|device| device.id == *id)
            .map(|device| AuditDevice {
                id: device.id.clone(),
                name: device.name.clone(),
            });
        (member, device)
    }

    /// Appends to the audit log as `who` (SRV-210).
    pub(crate) fn record(&self, who: &Who, event: &str, details: Value) {
        let (member, device) = self.actor(who);
        self.audit.record(event, member, device, details);
    }

    // ---- the server and its setup (SRV-060, SRV-061)

    /// The server's name: step 1's, else the machine's.
    pub(crate) fn name(&self) -> String {
        let name = self.lock().config.name.clone();
        name.unwrap_or_else(|| self.machine_name.clone())
    }

    /// Until step 2 has made the first member.
    pub(crate) fn needs_setup(&self) -> bool {
        self.lock().members.is_empty()
    }

    /// `GET /api/hello`.
    pub(crate) fn hello(&self) -> Hello {
        Hello {
            name: self.name(),
            version: nodus_core::VERSION.to_owned(),
            protocol: PROTOCOL,
            needs_setup: self.needs_setup(),
        }
    }

    /// `setup:status`.
    pub(crate) fn setup_status(&self) -> SetupStatus {
        let name = self.name();
        let people = self.lock();
        let done: Vec<SetupStep> = SetupStep::ALL
            .into_iter()
            .filter(|step| match step {
                SetupStep::Github => !people.members.is_empty(),
                step => people.config.setup.contains(step),
            })
            .collect();
        let step = SetupStep::ALL
            .into_iter()
            .find(|step| !done.contains(step))
            .unwrap_or(SetupStep::Done);
        SetupStatus {
            needs_setup: people.members.is_empty(),
            step,
            done,
            name,
            github: people.config.github.as_ref().map(AppConfig::public),
        }
    }

    /// `setup:name`: step 1 (SRV-061).
    pub(crate) fn set_name(&self, who: &Who, name: &str) -> Result<SetupStatus, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > NAME_CHARS || name.chars().any(char::is_control) {
            return Err(format!("A server's name is 1 to {NAME_CHARS} characters"));
        }
        {
            let mut people = self.lock();
            people.config.name = Some(name.to_owned());
            if !people.config.setup.contains(&SetupStep::Name) {
                people.config.setup.push(SetupStep::Name);
            }
            self.save_config(&people)?;
        }
        self.record(who, "server.named", json!({ "name": name }));
        Ok(self.setup_status())
    }

    /// `setup:done`: a step after step 2, finished or skipped. Steps 4 and
    /// 5 come with the slices that set them up.
    pub(crate) fn setup_done(&self, step: SetupStep) -> Result<SetupStatus, String> {
        if step != SetupStep::Members {
            return Err("Only the members step can be marked done for now".to_owned());
        }
        {
            let mut people = self.lock();
            if people.members.is_empty() {
                return Err("Set up GitHub and sign in first".to_owned());
            }
            if !people.config.setup.contains(&step) {
                people.config.setup.push(step);
                self.save_config(&people)?;
            }
        }
        Ok(self.setup_status())
    }

    /// `server:info`.
    pub(crate) async fn info(&self) -> ServerInfo {
        let github = self.lock().config.github.as_ref().map(AppConfig::public);
        let (installations, installations_error) = match &github {
            Some(_) => match self.installations().await {
                Ok(found) => (found, None),
                Err(err) => (Vec::new(), Some(err)),
            },
            None => (Vec::new(), None),
        };
        ServerInfo {
            name: self.name(),
            version: nodus_core::VERSION.to_owned(),
            protocol: PROTOCOL,
            needs_setup: self.needs_setup(),
            github,
            installations,
            installations_error,
            tailscale: tailscale_status().await,
        }
    }

    // ---- the GitHub App (SRV-062, SRV-063)

    /// A single-use nonce, good for 10 minutes, for `who`'s manifest flow.
    pub(crate) fn new_nonce(&self, who: &Who, workflows: bool) -> Nonce {
        let now = now_ms();
        let nonce = random_hex(16);
        let mut nonces = lock(&self.nonces);
        nonces.retain(|_, entry| entry.expires_at > now);
        nonces.insert(
            nonce.clone(),
            NonceEntry {
                expires_at: now + NONCE_MS,
                workflows,
                asked_by: who.clone(),
            },
        );
        Nonce {
            nonce,
            expires_at: now + NONCE_MS,
        }
    }

    /// Whether a nonce is good and unused, and whether it asked for
    /// `workflows`.
    pub(crate) fn peek_nonce(&self, nonce: &str) -> Option<bool> {
        let now = now_ms();
        let nonces = lock(&self.nonces);
        let entry = nonces.get(nonce).filter(|entry| entry.expires_at > now)?;
        Some(entry.workflows)
    }

    /// Uses a nonce up: the callback's `state` (SRV-062). Whether it asked
    /// for `workflows`, and who asked for it.
    pub(crate) fn take_nonce(&self, nonce: &str) -> Option<(bool, Who)> {
        let now = now_ms();
        let entry = lock(&self.nonces).remove(nonce)?;
        (entry.expires_at > now).then_some((entry.workflows, entry.asked_by))
    }

    /// Keeps the App GitHub just made: its key in `github-app.pem` (0600),
    /// the rest in `server.json`. Nothing of it goes back to a client but
    /// what [`AppConfig::public`] shows.
    pub(crate) fn keep_app(&self, who: &Who, made: Conversion, workflows: bool) -> Result<AppConfig, String> {
        AppKey::from_pem(&made.pem)?;
        let app = AppConfig {
            app_id: made.id,
            slug: made.slug,
            name: made.name,
            owner: made.owner.map(|owner| owner.login),
            html_url: made.html_url,
            client_id: made.client_id,
            client_secret: made.client_secret,
            webhook_secret: made.webhook_secret,
            workflows,
            created_at: now_ms(),
            installations: Vec::new(),
        };
        let replaced = {
            let mut people = self.lock();
            store::write_private(&self.data.join(APP_KEY_FILE), &made.pem)?;
            let replaced = people.config.github.replace(app.clone());
            self.save_config(&people)?;
            replaced
        };
        self.record(
            who,
            "github.app-created",
            json!({
                "appId": app.app_id,
                "slug": app.slug,
                "owner": app.owner,
                "workflows": workflows,
                "replaced": replaced.map(|old| old.app_id),
            }),
        );
        Ok(app)
    }

    /// The App's key and id, to speak as it.
    fn app_key(&self) -> Result<(AppKey, u64), String> {
        let app_id = self
            .lock()
            .config
            .github
            .as_ref()
            .map(|app| app.app_id)
            .ok_or("This server has no GitHub App yet")?;
        let pem = std::fs::read_to_string(self.data.join(APP_KEY_FILE))
            .map_err(|err| format!("Can't read the GitHub App's key: {err}"))?;
        Ok((AppKey::from_pem(&pem)?, app_id))
    }

    /// The App's installations, as GitHub says now. A change from the last
    /// look goes in the audit log (SRV-210).
    pub(crate) async fn installations(&self) -> Result<Vec<Installation>, String> {
        let (key, app_id) = self.app_key()?;
        let found = self.github.installations(&key, app_id).await?;
        let now: BTreeSet<u64> = found.iter().map(|installation| installation.id).collect();
        let changed = {
            let mut people = self.lock();
            let Some(app) = people.config.github.as_mut().filter(|app| app.app_id == app_id) else {
                return Ok(found);
            };
            let before: BTreeSet<u64> = app.installations.iter().copied().collect();
            if before == now {
                None
            } else {
                app.installations = now.iter().copied().collect();
                self.save_config(&people)?;
                Some(before)
            }
        };
        if let Some(before) = changed {
            let accounts = |ids: Vec<&u64>| -> Vec<Value> {
                ids.into_iter()
                    .map(|id| {
                        let account = found.iter().find(|i| i.id == *id).map(|i| i.account.clone());
                        json!({ "id": id, "account": account })
                    })
                    .collect()
            };
            self.audit.record(
                "github.installations-changed",
                None,
                None,
                json!({
                    "added": accounts(now.difference(&before).collect()),
                    "removed": accounts(before.difference(&now).collect()),
                }),
            );
        }
        Ok(found)
    }

    /// `github:replace-key` (SRV-063): the new key, once GitHub has taken a
    /// JWT it signed as this App, in place of the old one.
    pub(crate) async fn replace_key(&self, who: &Who, pem: &str) -> Result<nodus_wire::GithubApp, String> {
        let key = AppKey::from_pem(pem)?;
        let app_id = self
            .lock()
            .config
            .github
            .as_ref()
            .map(|app| app.app_id)
            .ok_or("This server has no GitHub App yet")?;
        let identity = self
            .github
            .app(&key, app_id)
            .await
            .map_err(|err| format!("GitHub didn't take that key: {err}"))?;
        if identity.id != app_id {
            let other = identity.slug.map(|slug| format!(" ({slug})")).unwrap_or_default();
            return Err(format!("That key belongs to another GitHub App{other}"));
        }
        let app = {
            let people = self.lock();
            store::write_private(&self.data.join(APP_KEY_FILE), pem)?;
            people.config.github.as_ref().map(AppConfig::public)
        };
        self.record(who, "github.key-replaced", json!({ "appId": app_id }));
        app.ok_or_else(|| "This server has no GitHub App yet".to_owned())
    }

    // ---- signing in (SRV-041…043)

    /// `POST /api/auth/device`: GitHub's device flow with the App's client
    /// id. The app polls with a random handle, never the user code.
    pub(crate) async fn start_sign_in(
        &self,
        setup_token: Option<&str>,
        device_name: &str,
    ) -> Result<SignInStarted, Refusal> {
        let device_name = clean_name(device_name).ok_or((400, "Give the device a name".to_owned()))?;
        let client_id = self
            .lock()
            .config
            .github
            .as_ref()
            .map(|app| app.client_id.clone())
            .ok_or((409, "Set up this server's GitHub App first".to_owned()))?;
        // While there's no member, only the setup token's holder can become
        // the first one (SRV-061).
        let setup_token = setup_token
            .filter(|token| {
                self.still_good(&Who::Setup {
                    token: (*token).to_owned(),
                })
            })
            .map(str::to_owned);
        let code = self.github.device_code(&client_id).await.map_err(|err| (502, err))?;
        let now = now_ms();
        let handle = random_hex(32);
        let mut sign_ins = lock(&self.sign_ins);
        sign_ins.retain(|_, pending| pending.expires_at > now);
        sign_ins.insert(
            handle.clone(),
            PendingSignIn {
                device_code: code.device_code,
                interval: code.interval.max(1),
                expires_at: now + code.expires_in as f64 * 1000.0,
                next_poll_at: now,
                device_name,
                setup_token,
            },
        );
        Ok(SignInStarted {
            user_code: code.user_code,
            verification_uri: code.verification_uri,
            handle,
            interval: code.interval.max(1),
            expires_in: code.expires_in,
        })
    }

    /// `POST /api/auth/device/poll`: asks GitHub once, no more often than
    /// it said. Signed in, the person's GitHub token is dropped (SRV-043).
    pub(crate) async fn poll_sign_in(&self, handle: &str, peer: IpAddr) -> Result<SignInPoll, Refusal> {
        let now = now_ms();
        let (device_code, interval) = {
            let mut sign_ins = lock(&self.sign_ins);
            let Some(pending) = sign_ins.get_mut(handle).filter(|pending| pending.expires_at > now) else {
                sign_ins.remove(handle);
                return Ok(expired());
            };
            if now < pending.next_poll_at {
                return Ok(SignInPoll::Pending {
                    interval: pending.interval,
                });
            }
            pending.next_poll_at = now + pending.interval as f64 * 1000.0;
            (pending.device_code.clone(), pending.interval)
        };
        let client_id = self
            .lock()
            .config
            .github
            .as_ref()
            .map(|app| app.client_id.clone())
            .ok_or((409, "Set up this server's GitHub App first".to_owned()))?;
        let token = match self.github.poll_token(&client_id, &device_code).await {
            Ok(TokenPoll::Token(token)) => token,
            Ok(TokenPoll::Pending) => return Ok(SignInPoll::Pending { interval }),
            Ok(TokenPoll::SlowDown(slower)) => {
                if let Some(pending) = lock(&self.sign_ins).get_mut(handle) {
                    pending.interval = slower.max(interval);
                    pending.next_poll_at = now + pending.interval as f64 * 1000.0;
                }
                return Ok(SignInPoll::Pending {
                    interval: slower.max(interval),
                });
            }
            Ok(TokenPoll::Expired) => {
                lock(&self.sign_ins).remove(handle);
                return Ok(expired());
            }
            Ok(TokenPoll::Denied) => {
                lock(&self.sign_ins).remove(handle);
                return Ok(SignInPoll::Denied {
                    message: "The sign-in was turned down on GitHub".to_owned(),
                });
            }
            Err(err) => return Err((502, err)),
        };
        // The handle is used up, whatever comes of it.
        let Some(pending) = lock(&self.sign_ins).remove(handle) else {
            return Ok(expired());
        };
        let user = self.github.user(&token).await.map_err(|err| (502, err));
        drop(token);
        let user = user?;
        let tailscale = whois(peer).await;
        self.signed_in(&user, &pending, tailscale)
    }

    /// The person GitHub vouched for: a member gets a new device, the
    /// setup token's holder becomes the first member, anyone else is
    /// refused (SRV-040, SRV-041).
    fn signed_in(
        &self,
        user: &GitHubUser,
        pending: &PendingSignIn,
        tailscale: Option<TailscaleIdentity>,
    ) -> Result<SignInPoll, Refusal> {
        let now = now_ms();
        let first_allowed = pending
            .setup_token
            .as_ref()
            .is_some_and(|token| self.still_good(&Who::Setup { token: token.clone() }));
        let token = random_hex(32);
        let (member, device, first) = {
            let mut people = self.lock();
            let first = people.members.is_empty();
            let at = people.members.iter().position(|member| member.github_id == user.id);
            let member = match at {
                // SRV-040: the id decides; the login is refreshed.
                Some(at) => {
                    let member = &mut people.members[at];
                    member.login = user.login.clone();
                    member.name = user.name.clone();
                    member.avatar_url = avatar(user.avatar_url.as_deref());
                    member.clone()
                }
                None if first && first_allowed => {
                    let member = Member {
                        github_id: user.id,
                        login: user.login.clone(),
                        name: user.name.clone(),
                        avatar_url: avatar(user.avatar_url.as_deref()),
                        added_by: None,
                        added_at: now,
                    };
                    people.members.push(member.clone());
                    member
                }
                None => {
                    drop(people);
                    self.audit.record(
                        "member.refused",
                        None,
                        None,
                        json!({ "githubId": user.id, "login": user.login, "device": pending.device_name }),
                    );
                    return Ok(SignInPoll::Refused {
                        message: format!(
                            "{} isn't a member of this server. Ask a member to invite you.",
                            user.login
                        ),
                    });
                }
            };
            let record = DeviceRecord {
                id: random_hex(8),
                token_hash: sha256_hex(&token),
                github_id: member.github_id,
                name: pending.device_name.clone(),
                tailscale,
                created_at: now,
                last_seen_at: now,
            };
            people.devices.push(record.clone());
            self.save_members(&people).map_err(|err| (500, err))?;
            self.save_devices(&people);
            let device = public_device(&record, &member.login, None);
            (member, device, first)
        };
        let actor = Who::Device {
            id: device.id.clone(),
            github_id: member.github_id,
        };
        if first {
            // SRV-060: the setup code and tokens end with the first member,
            // and so do sockets that came in with one.
            let _ = setup::finish(&self.data);
            self.close_where(
                |who| matches!(who, Who::Setup { .. }),
                "Setup is done; sign in to carry on",
            );
            self.record(
                &actor,
                "member.added",
                json!({ "githubId": member.github_id, "login": member.login, "first": true }),
            );
        }
        self.record(
            &actor,
            "member.signed-in",
            json!({ "deviceId": device.id, "device": device.name, "tailscale": device.tailscale }),
        );
        // SRV-042: how a phished device code would be noticed.
        let shown = member.name.clone().filter(|name| !name.is_empty());
        let _ = self.notices.send(Notice {
            kind: nodus_wire::DEVICE_SIGNED_IN.to_owned(),
            message: format!(
                "{} signed in on {}",
                shown.as_deref().unwrap_or(&member.login),
                device.name
            ),
        });
        Ok(SignInPoll::SignedIn { token, member, device })
    }

    // ---- members (SRV-040, SRV-044)

    pub(crate) fn members(&self) -> Vec<Member> {
        self.lock().members.clone()
    }

    /// `members:invite`: the login resolved to its numeric id.
    pub(crate) async fn invite(&self, who: &Who, login: &str) -> Result<Member, String> {
        let login = login.trim().trim_start_matches('@');
        if !is_login(login) {
            return Err(format!("{login:?} isn't a GitHub login"));
        }
        let user = self
            .github
            .user_by_login(login)
            .await?
            .ok_or_else(|| format!("There's no GitHub account named {login}"))?;
        let (inviter, _) = self.actor(who);
        let member = {
            let mut people = self.lock();
            if let Some(member) = people.members.iter().find(|member| member.github_id == user.id) {
                return Err(format!("{} is a member already", member.login));
            }
            if people.members.is_empty() {
                return Err("Sign in first: the first member is whoever finishes setup".to_owned());
            }
            let member = Member {
                github_id: user.id,
                login: user.login,
                name: user.name,
                avatar_url: avatar(user.avatar_url.as_deref()),
                added_by: inviter.map(|inviter| inviter.login),
                added_at: now_ms(),
            };
            people.members.push(member.clone());
            self.save_members(&people)?;
            member
        };
        self.record(
            who,
            "member.invited",
            json!({ "githubId": member.github_id, "login": member.login }),
        );
        Ok(member)
    }

    /// `members:remove`: the member and all their devices, whose sockets
    /// close at once. The last member stays (SRV-040).
    pub(crate) fn remove_member(&self, who: &Who, github_id: u64) -> Result<(), String> {
        let (member, devices) = {
            let mut people = self.lock();
            let at = people
                .members
                .iter()
                .position(|member| member.github_id == github_id)
                .ok_or("No such member")?;
            if people.members.len() == 1 {
                return Err("You're the last member, so you can't remove yourself".to_owned());
            }
            let member = people.members.remove(at);
            let devices: Vec<String> = people
                .devices
                .iter()
                .filter(|device| device.github_id == github_id)
                .map(|device| device.id.clone())
                .collect();
            people.devices.retain(|device| device.github_id != github_id);
            self.save_members(&people)?;
            self.save_devices(&people);
            (member, devices)
        };
        // Recorded before the sockets go, in case it's the caller's own.
        self.record(
            who,
            "member.removed",
            json!({ "githubId": member.github_id, "login": member.login, "devices": devices }),
        );
        self.close_where(
            |who| matches!(who, Who::Device { github_id: of, .. } if *of == github_id),
            "You were removed from this server",
        );
        Ok(())
    }

    // ---- devices (SRV-042)

    /// `devices:list`.
    pub(crate) fn devices(&self, who: &Who) -> Vec<Device> {
        let current = match who {
            Who::Device { id, .. } => Some(id.as_str()),
            _ => None,
        };
        let people = self.lock();
        people
            .devices
            .iter()
            .map(|device| {
                let login = people
                    .members
                    .iter()
                    .find(|member| member.github_id == device.github_id)
                    .map_or("", |member| member.login.as_str());
                public_device(device, login, current)
            })
            .collect()
    }

    /// `devices:revoke`: its token stops working, and its socket closes now.
    pub(crate) fn revoke(&self, who: &Who, id: &str) -> Result<(), String> {
        let device = {
            let mut people = self.lock();
            let at = people
                .devices
                .iter()
                .position(|device| device.id == id)
                .ok_or("No such device")?;
            let device = people.devices.remove(at);
            self.save_devices(&people);
            let login = people
                .members
                .iter()
                .find(|member| member.github_id == device.github_id)
                .map(|member| member.login.clone());
            json!({ "id": device.id, "name": device.name, "githubId": device.github_id, "login": login })
        };
        self.record(who, "device.revoked", device);
        self.close_where(
            |who| matches!(who, Who::Device { id: of, .. } if of == id),
            "This device was signed out of the server",
        );
        Ok(())
    }

    // ---- open sockets and notices

    /// Keeps track of a socket, so revoking its device can close it.
    pub(crate) fn connected(&self, who: Who, close: watch::Sender<Option<CloseFrame>>) -> u64 {
        let id = self.next_connection.fetch_add(1, Ordering::Relaxed);
        lock(&self.connections).insert(id, Connection { who, close });
        id
    }

    pub(crate) fn disconnected(&self, id: u64) {
        lock(&self.connections).remove(&id);
    }

    /// Closes every socket `which` picks, saying why.
    pub(crate) fn close_where(&self, which: impl Fn(&Who) -> bool, why: &str) {
        for connection in lock(&self.connections).values() {
            if which(&connection.who) {
                connection.close.send_replace(Some(CloseFrame {
                    code: CloseCode::Policy,
                    reason: why.to_owned().into(),
                }));
            }
        }
    }

    /// What every member's socket is told.
    pub(crate) fn notices(&self) -> broadcast::Receiver<Notice> {
        self.notices.subscribe()
    }

    // ---- the files

    fn lock(&self) -> MutexGuard<'_, People> {
        lock(&self.people)
    }

    fn save_config(&self, people: &People) -> Result<(), String> {
        store::write_json(&self.data.join(SERVER_FILE), &people.config)
    }

    fn save_members(&self, people: &People) -> Result<(), String> {
        store::write_json(&self.data.join(MEMBERS_FILE), &people.members)
    }

    /// A device list that can't be written leaves the one in memory, which
    /// is what counts while the server runs; it's said on standard error.
    fn save_devices(&self, people: &People) {
        if let Err(err) = store::write_json(&self.data.join(DEVICES_FILE), &people.devices) {
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), "nodus-server: {err}");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn expired() -> SignInPoll {
    SignInPoll::Expired {
        message: "The sign-in code expired. Start again.".to_owned(),
    }
}

fn public_device(device: &DeviceRecord, login: &str, current: Option<&str>) -> Device {
    Device {
        id: device.id.clone(),
        name: device.name.clone(),
        github_id: device.github_id,
        login: login.to_owned(),
        tailscale: device.tailscale.clone(),
        created_at: device.created_at,
        last_seen_at: device.last_seen_at,
        current: current == Some(device.id.as_str()),
    }
}

/// A device's name, trimmed and cut to [`NAME_CHARS`]; none when empty.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(NAME_CHARS)
        .collect();
    (!name.is_empty()).then_some(name)
}

/// SRV-045: an avatar only from GitHub's avatar host, over HTTPS.
fn avatar(url: Option<&str>) -> Option<String> {
    let url = url::Url::parse(url?).ok()?;
    (url.scheme() == "https" && url.host_str() == Some(AVATAR_HOST)).then(|| url.to_string())
}

/// A GitHub login: letters, digits and single hyphens inside, at most 39.
pub(crate) fn is_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

/// `tailscale whois` of the address a device signed in from: its tailnet
/// user and machine, when Tailscale knows it (SRV-042).
async fn whois(peer: IpAddr) -> Option<TailscaleIdentity> {
    if peer.is_loopback() {
        return None;
    }
    let out = tailscale(&["whois", "--json", &peer.to_string()]).await?;
    parse_whois(&out)
}

/// The user's login and the machine's name in `tailscale whois --json`.
fn parse_whois(out: &str) -> Option<TailscaleIdentity> {
    let found: Value = serde_json::from_str(out).ok()?;
    let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let identity = TailscaleIdentity {
        user: text(&found["UserProfile"]["LoginName"]),
        node: text(&found["Node"]["ComputedName"]).or_else(|| text(&found["Node"]["Name"])),
    };
    (identity.user.is_some() || identity.node.is_some()).then_some(identity)
}

/// `tailscale status --json`: whether it's up, and this machine's tags (SRV-029).
pub(crate) async fn tailscale_status() -> TailscaleStatus {
    let Some(out) = tailscale(&["status", "--json"]).await else {
        return TailscaleStatus::default();
    };
    let found: Value = serde_json::from_str(&out).unwrap_or_default();
    let tags: Vec<String> = found["Self"]["Tags"]
        .as_array()
        .map(|tags| tags.iter().filter_map(|tag| tag.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    TailscaleStatus {
        running: true,
        tagged: tags.iter().any(|tag| tag == TAILSCALE_TAG),
        tags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_avatars_only_from_githubs_avatar_host() {
        assert_eq!(
            avatar(Some("https://avatars.githubusercontent.com/u/1?v=4")).as_deref(),
            Some("https://avatars.githubusercontent.com/u/1?v=4")
        );
        for elsewhere in [
            "http://avatars.githubusercontent.com/u/1",
            "https://evil.example/u/1",
            "https://avatars.githubusercontent.com.evil.example/u/1",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(avatar(Some(elsewhere)), None, "{elsewhere}");
        }
    }

    #[test]
    fn knows_a_github_login() {
        for login in ["octocat", "a", "saleem-hadad", "A1-b2"] {
            assert!(is_login(login), "{login}");
        }
        for wrong in ["", "-a", "a-", "a--b", "a/b", "../x", "a b", &"x".repeat(40)] {
            assert!(!is_login(wrong), "{wrong}");
        }
    }

    #[test]
    fn reads_who_tailscale_says_a_device_is() {
        let out = r#"{"Node":{"Name":"studio.tail1234.ts.net.","ComputedName":"studio"},"UserProfile":{"LoginName":"octo@example.com","DisplayName":"Octo"}}"#;
        assert_eq!(
            parse_whois(out),
            Some(TailscaleIdentity {
                user: Some("octo@example.com".into()),
                node: Some("studio".into()),
            })
        );
        assert_eq!(parse_whois("{}"), None);
        assert_eq!(parse_whois("no such peer"), None);
    }

    #[test]
    fn names_a_device_plainly() {
        assert_eq!(clean_name("  Studio\n").as_deref(), Some("Studio"));
        assert_eq!(clean_name(" \t "), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().len(), NAME_CHARS);
    }
}
