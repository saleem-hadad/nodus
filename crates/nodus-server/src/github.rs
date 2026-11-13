//! GitHub, as a server asks it: the device flow (SRV-041), people by id and
//! login (SRV-040, SRV-044), the manifest conversion (SRV-062), and the
//! App's own calls with its JWT (SRV-063).
//!
//! Its two addresses come from `NODUS_GITHUB_API_URL` (`https://api.github.com`)
//! and `NODUS_GITHUB_URL` (`https://github.com`), so tests can stand a fake
//! GitHub in for the real one.

use std::time::Duration;

use nodus_wire::Installation;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::app_key::{AppKey, now_secs};

const API_URL: &str = "https://api.github.com";
const WEB_URL: &str = "https://github.com";
const WAIT: Duration = Duration::from_secs(30);

pub(crate) struct GitHub {
    api: String,
    web: String,
    http: reqwest::Client,
}

/// The device flow's start (SRV-041).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// What polling for the person's token came to.
pub(crate) enum TokenPoll {
    /// The person's token, to ask who they are with and then drop.
    Token(String),
    Pending,
    /// Poll less often: every this many seconds.
    SlowDown(u64),
    Expired,
    Denied,
}

/// A GitHub account, by its numeric id.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubUser {
    pub id: u64,
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// What the manifest conversion gives: the App and its secrets.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Conversion {
    pub id: u64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub owner: Option<Account>,
    pub html_url: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub webhook_secret: Option<String>,
    pub pem: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Account {
    pub login: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Deserialize)]
struct InstallationJson {
    id: u64,
    account: Option<Account>,
    #[serde(default)]
    repository_selection: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

/// The App as `GET /app` says it is.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AppIdentity {
    pub id: u64,
    #[serde(default)]
    pub slug: Option<String>,
}

impl GitHub {
    pub(crate) fn from_env() -> Self {
        let base = |var: &str, default: &str| {
            let url = std::env::var(var).ok().filter(|url| !url.is_empty());
            url.as_deref().unwrap_or(default).trim_end_matches('/').to_owned()
        };
        let http = reqwest::Client::builder()
            .user_agent(format!("nodus-server/{}", nodus_core::VERSION))
            .timeout(WAIT)
            .build()
            .unwrap_or_default();
        Self {
            api: base("NODUS_GITHUB_API_URL", API_URL),
            web: base("NODUS_GITHUB_URL", WEB_URL),
            http,
        }
    }

    /// GitHub's own pages: `https://github.com`.
    pub(crate) fn web(&self) -> &str {
        &self.web
    }

    /// `POST /login/device/code` with the App's client id.
    pub(crate) async fn device_code(&self, client_id: &str) -> Result<DeviceCode, String> {
        let request = self
            .http
            .post(format!("{}/login/device/code", self.web))
            .header("Accept", "application/json")
            .form(&[("client_id", client_id)]);
        let (status, body) = send(request).await?;
        if status != 200 || body.get("error").is_some() {
            return Err(failed(status, &body));
        }
        parse(body)
    }

    /// `POST /login/oauth/access_token`: one poll of the device flow.
    pub(crate) async fn poll_token(&self, client_id: &str, device_code: &str) -> Result<TokenPoll, String> {
        let request = self
            .http
            .post(format!("{}/login/oauth/access_token", self.web))
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]);
        let (status, body) = send(request).await?;
        if let Some(token) = body.get("access_token").and_then(Value::as_str) {
            return Ok(TokenPoll::Token(token.to_owned()));
        }
        Ok(match body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => TokenPoll::Pending,
            Some("slow_down") => TokenPoll::SlowDown(body.get("interval").and_then(Value::as_u64).unwrap_or(10)),
            Some("expired_token" | "token_expired") => TokenPoll::Expired,
            Some("access_denied") => TokenPoll::Denied,
            _ => return Err(failed(status, &body)),
        })
    }

    /// `GET /user` with the person's token: who they are.
    pub(crate) async fn user(&self, token: &str) -> Result<GitHubUser, String> {
        let request = self.api_get("/user").bearer_auth(token);
        let (status, body) = send(request).await?;
        if status != 200 {
            return Err(failed(status, &body));
        }
        parse(body)
    }

    /// `GET /users/<login>`: none when there's no such account.
    pub(crate) async fn user_by_login(&self, login: &str) -> Result<Option<GitHubUser>, String> {
        let (status, body) = send(self.api_get(&format!("/users/{login}"))).await?;
        match status {
            200 => parse(body).map(Some),
            404 => Ok(None),
            _ => Err(failed(status, &body)),
        }
    }

    /// `POST /app-manifests/<code>/conversions`: the App the manifest made.
    pub(crate) async fn convert_manifest(&self, code: &str) -> Result<Conversion, String> {
        let request = self
            .http
            .post(format!("{}/app-manifests/{code}/conversions", self.api))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        let (status, body) = send(request).await?;
        if status != 201 && status != 200 {
            return Err(failed(status, &body));
        }
        parse(body)
    }

    /// `GET /app/installations` as the App.
    pub(crate) async fn installations(&self, key: &AppKey, app_id: u64) -> Result<Vec<Installation>, String> {
        let jwt = key.jwt(app_id, now_secs())?;
        let request = self.api_get("/app/installations?per_page=100").bearer_auth(jwt);
        let (status, body) = send(request).await?;
        if status != 200 {
            return Err(failed(status, &body));
        }
        let found: Vec<InstallationJson> = parse(body)?;
        Ok(found
            .into_iter()
            .map(|found| {
                let account = found.account;
                Installation {
                    id: found.id,
                    account: account.as_ref().map(|a| a.login.clone()).unwrap_or_default(),
                    account_type: account.and_then(|a| a.kind).unwrap_or_default(),
                    repository_selection: found.repository_selection.unwrap_or_default(),
                    html_url: found.html_url.unwrap_or_default(),
                }
            })
            .collect())
    }

    /// `GET /app` as the App: which App a key speaks for.
    pub(crate) async fn app(&self, key: &AppKey, app_id: u64) -> Result<AppIdentity, String> {
        let jwt = key.jwt(app_id, now_secs())?;
        let (status, body) = send(self.api_get("/app").bearer_auth(jwt)).await?;
        if status != 200 {
            return Err(failed(status, &body));
        }
        parse(body)
    }

    fn api_get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}{path}", self.api))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }
}

/// Sends a request: its status and its body as JSON (null when it isn't).
async fn send(request: reqwest::RequestBuilder) -> Result<(u16, Value), String> {
    let response = request
        .send()
        .await
        .map_err(|err| format!("Can't reach GitHub: {err}"))?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("Can't reach GitHub: {err}"))?;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, body))
}

fn parse<T: DeserializeOwned>(body: Value) -> Result<T, String> {
    serde_json::from_value(body).map_err(|err| format!("GitHub's answer didn't fit: {err}"))
}

/// GitHub's own reason, when it gave one.
fn failed(status: u16, body: &Value) -> String {
    let reason = ["message", "error_description", "error"]
        .iter()
        .find_map(|key| body.get(*key).and_then(Value::as_str));
    match reason {
        Some(reason) => format!("GitHub answered {status}: {reason}"),
        None => format!("GitHub answered {status}"),
    }
}
