//! The server's own files beside state.json (SRV-003), each written whole
//! to a file beside it and renamed into place, readable by `nodus` alone
//! (0600), since `nodus-run` must read none of them (SRV-005):
//!
//! - `server.json`: the name, the wizard's steps, and the GitHub App's ids
//!   and secrets (its client secret and webhook secret);
//! - `members.json`, `devices.json` (token hashes only);
//! - `github-app.pem`, the App's private key;
//! - `setup.json`, the setup code's hash and the setup tokens' (setup.rs);
//! - `audit.jsonl`, appended to and never rewritten (audit.rs).

use std::path::Path;

use nodus_wire::{GithubApp, Member, SetupStep, TailscaleIdentity};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SERVER_FILE: &str = "server.json";
pub(crate) const MEMBERS_FILE: &str = "members.json";
pub(crate) const DEVICES_FILE: &str = "devices.json";
pub(crate) const APP_KEY_FILE: &str = "github-app.pem";

/// `server.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerConfig {
    /// Step 1's name; the machine's until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The wizard's steps finished, other than step 2, which is done once
    /// there's a member (SRV-061).
    #[serde(default)]
    pub setup: Vec<SetupStep>,
    /// The GitHub App the manifest flow made (SRV-062).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<AppConfig>,
}

/// The App, as GitHub's manifest conversion gave it. Its private key is in
/// `github-app.pem`; the secrets here never reach a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppConfig {
    pub app_id: u64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub html_url: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub workflows: bool,
    pub created_at: f64,
    /// The installations last seen, to tell when they change (SRV-210).
    #[serde(default)]
    pub installations: Vec<u64>,
}

impl AppConfig {
    /// What clients may see of it.
    pub(crate) fn public(&self) -> GithubApp {
        GithubApp {
            app_id: self.app_id,
            slug: self.slug.clone(),
            name: self.name.clone(),
            owner: self.owner.clone(),
            html_url: self.html_url.clone(),
            client_id: self.client_id.clone(),
            workflows: self.workflows,
        }
    }
}

/// One device in `devices.json` (SRV-042): the SHA-256 of its token, never
/// the token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceRecord {
    pub id: String,
    pub token_hash: String,
    pub github_id: u64,
    pub name: String,
    #[serde(default)]
    pub tailscale: Option<TailscaleIdentity>,
    pub created_at: f64,
    pub last_seen_at: f64,
}

/// The people files, as read at startup.
#[derive(Debug, Clone, Default)]
pub(crate) struct People {
    pub config: ServerConfig,
    pub members: Vec<Member>,
    pub devices: Vec<DeviceRecord>,
}

impl People {
    /// Reads the three files. One that's there but can't be read stops the
    /// server, rather than starting empty and writing over it.
    pub(crate) fn load(data: &Path) -> Result<Self, String> {
        Ok(Self {
            config: read_json(&data.join(SERVER_FILE))?,
            members: read_json(&data.join(MEMBERS_FILE))?,
            devices: read_json(&data.join(DEVICES_FILE))?,
        })
    }
}

/// A JSON file, or its type's default when there's none.
pub(crate) fn read_json<T: DeserializeOwned + Default>(file: &Path) -> Result<T, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => nodus_domain::json::from_str(&text).map_err(|err| format!("Can't read {}: {err}", file.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(format!("Can't read {}: {err}", file.display())),
    }
}

/// Writes a JSON file only this user can read, renamed into place.
pub(crate) fn write_json<T: Serialize>(file: &Path, value: &T) -> Result<(), String> {
    let text = nodus_domain::json::to_string_pretty(value).map_err(|err| err.to_string())?;
    write_private(file, &text)
}

/// Writes a file only this user can read (0600), renamed into place.
pub(crate) fn write_private(file: &Path, text: &str) -> Result<(), String> {
    nodus_host_proto::write_private_file(file, text).map_err(|err| format!("Can't write {}: {err}", file.display()))
}

/// Milliseconds since the epoch, as JavaScript's `Date.now()`.
pub(crate) fn now_ms() -> f64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    since.as_millis() as f64
}

/// `len` random bytes as hex.
pub(crate) fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    hex(&bytes)
}

/// SHA-256, as hex: how tokens and codes are kept.
pub(crate) fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_its_files_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SERVER_FILE);
        assert_eq!(read_json::<ServerConfig>(&file).unwrap(), ServerConfig::default());
        let config = ServerConfig {
            name: Some("box".into()),
            setup: vec![SetupStep::Name],
            github: None,
        };
        write_json(&file, &config).unwrap();
        assert_eq!(read_json::<ServerConfig>(&file).unwrap(), config);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(&file, "{ nope").unwrap();
        assert!(read_json::<ServerConfig>(&file).unwrap_err().contains("Can't read"));
    }

    #[test]
    fn hashes_and_draws_tokens() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let token = random_hex(32);
        assert_eq!(token.len(), 64);
        assert_ne!(token, random_hex(32));
    }
}
