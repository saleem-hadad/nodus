//! `server:update` (SRV-200, SRV-201): a member updates the server from the
//! app.
//!
//! 1. The server picks the version itself: the newest release GitHub lists
//!    with this machine's `nodus-server-linux-<arch>.tar.gz` and its `.sig`,
//!    leaving out drafts and prereleases. One that isn't newer than what it
//!    runs changes nothing, so it never downgrades.
//! 2. It downloads both and checks the archive's ed25519 signature against
//!    the release key built into this binary
//!    (`packaging/server/signing-key.pub.pem`, which `install-server.sh`
//!    carries too). A build without one, as with the placeholder, refuses.
//! 3. It takes `nodus` out of the archive, writes it beside the running
//!    binary (`/opt/nodus/bin/nodus.new`), syncs it to disk and renames it
//!    over the old one.
//! 4. It answers, then exits, and systemd starts the new version
//!    (`Restart=always`). The session host keeps running, and its sessions
//!    with it, unless the new version speaks another host protocol
//!    (SRV-004).
//!
//! Every member's app hears how it's going as `server-update` events, and
//! each call writes a line to the audit log (SRV-210).

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nodus_domain::{Event, ServerUpdateStatus};
use nodus_updater::{DAMAGED, INTERRUPTED, RATE_LIMITED, UNREACHABLE, is_newer, tag_version};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::auth::Who;
use crate::serve::Server;

/// The public key releases are signed with (SRV-201): the one file
/// `scripts/server-signing-key.sh` writes, beside the copy in
/// `install-server.sh`. Until there's a key, a comment saying so.
const RELEASE_KEY: &str = include_str!("../../../packaging/server/signing-key.pub.pem");

/// Debug builds only, for tests: a PEM public key used instead of the
/// release's, so an update can be signed with a throwaway key. A release
/// build never reads it.
#[cfg(debug_assertions)]
pub(crate) const DEV_KEY_VAR: &str = "NODUS_DEV_UPDATE_KEY";

/// The repository releases come from.
const REPO: &str = "saleem-hadad/nodus";
/// How long asking GitHub for its releases may take (UPD-016).
const LOOKUP_TIMEOUT: Duration = nodus_updater::LOOKUP_TIMEOUT;
/// A download that receives nothing for this long is given up (UPD-031).
const STALL: Duration = nodus_updater::STALL;
/// The largest archive taken, and the largest binary in it.
const MAX_ARCHIVE: usize = 256 * 1024 * 1024;
const MAX_BINARY: usize = 512 * 1024 * 1024;
/// How long the server waits after answering before it exits, so the
/// answer and the last events reach the apps first.
const RESTART_AFTER: Duration = Duration::from_millis(1000);

pub(crate) const NO_KEY: &str =
    "This build has no signing key yet, so it can’t check an update. Install a release with install-server.sh.";
pub(crate) const BAD_SIGNATURE: &str =
    "The update’s signature doesn’t match Nodus’s signing key. Nothing was installed.";
pub(crate) const BUSY: &str = "This server is updating already.";
const NO_BINARY: &str = "The update has no nodus in it.";
const OTHER_MACHINE: &str = "The update isn’t built for this machine.";
const NO_EXECUTABLE: &str = "Can’t tell where this server’s binary is, so it can’t replace it.";
const UNREADABLE: &str = "GitHub’s list of releases couldn’t be read.";

/// The server's updates: at most one at a time.
pub(crate) struct Updates {
    /// `GET`: the repository's releases, newest first.
    releases_url: String,
    http: reqwest::Client,
    /// The running binary, as it was when the server started: once it's
    /// replaced, Linux names it `… (deleted)`.
    exe: Option<PathBuf>,
    busy: AtomicBool,
    /// Told once the new version is in place: `serve` stops then, and
    /// systemd starts the new one.
    pub restart: Notify,
}

impl Updates {
    /// Updates from the releases GitHub's API at `github_api` lists.
    pub(crate) fn new(github_api: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("Nodus/{}", nodus_core::VERSION))
            .connect_timeout(LOOKUP_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            releases_url: format!("{github_api}/repos/{REPO}/releases?per_page=30"),
            http,
            exe: std::env::current_exe().ok(),
            busy: AtomicBool::new(false),
            restart: Notify::new(),
        }
    }

    /// Finds, checks and installs the newest release, telling `tell` how
    /// it's going: `latest` when there's nothing newer, `restarting` once
    /// the new binary is in place.
    async fn run(&self, tell: &impl Fn(ServerUpdateStatus)) -> Result<ServerUpdateStatus, String> {
        let key = release_key().ok_or(NO_KEY)?;
        let exe = self.exe.clone().ok_or(NO_EXECUTABLE)?;
        tell(ServerUpdateStatus::Checking);
        let found = self
            .newest(&asset_name())
            .await?
            .filter(|found| is_newer(&found.version, nodus_core::VERSION));
        let Some(found) = found else {
            return Ok(ServerUpdateStatus::Latest {
                version: nodus_core::VERSION.to_owned(),
            });
        };
        let version = found.version.clone();
        let downloading = |percent| ServerUpdateStatus::Downloading {
            version: version.clone(),
            percent,
        };
        tell(downloading(0));
        let mut told = 0;
        let archive = self
            .download(&found.archive, found.size, |percent| {
                if percent != told {
                    told = percent;
                    tell(downloading(percent));
                }
            })
            .await?;
        let signature = self.download(&found.signature, None, |_| {}).await?;
        verify(&key, &archive, &signature)?;
        let binary = unpack(&archive)?;
        check_machine(&binary)?;
        install(&exe, &binary)?;
        Ok(ServerUpdateStatus::Restarting { version })
    }

    /// The newest release with `asset` and its signature (SRV-201).
    async fn newest(&self, asset: &str) -> Result<Option<Found>, String> {
        let asked = self
            .http
            .get(&self.releases_url)
            .header("Accept", "application/vnd.github+json")
            .send();
        let response = match tokio::time::timeout(LOOKUP_TIMEOUT, asked).await {
            Ok(Ok(response)) => response,
            _ => return Err(UNREACHABLE.to_owned()),
        };
        // UPD-018, UPD-019
        let status = response.status().as_u16();
        if status == 403 || status == 429 {
            return Err(RATE_LIMITED.to_owned());
        }
        if !response.status().is_success() {
            return Err(format!("GitHub answered the update check with an error ({status})."));
        }
        let body = match tokio::time::timeout(LOOKUP_TIMEOUT, response.text()).await {
            Ok(Ok(body)) => body,
            _ => return Err(UNREACHABLE.to_owned()),
        };
        let releases: Vec<Value> = serde_json::from_str(&body).map_err(|_| UNREADABLE.to_owned())?;
        Ok(pick(&releases, asset))
    }

    /// A release's file, whole, telling `progress` the percent received
    /// when it changes (UPD-031…UPD-035).
    async fn download(&self, url: &str, size: Option<u64>, mut progress: impl FnMut(u8)) -> Result<Vec<u8>, String> {
        let mut response = match tokio::time::timeout(STALL, self.http.get(url).send()).await {
            Ok(Ok(response)) => response,
            _ => return Err(INTERRUPTED.to_owned()),
        };
        if !response.status().is_success() {
            return Err(format!(
                "GitHub answered the download with an error ({}).",
                response.status().as_u16()
            ));
        }
        let total = size.or_else(|| response.content_length()).filter(|&total| total > 0);
        let mut bytes = Vec::new();
        loop {
            match tokio::time::timeout(STALL, response.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    bytes.extend_from_slice(&chunk);
                    if bytes.len() > MAX_ARCHIVE {
                        return Err(DAMAGED.to_owned());
                    }
                    if let Some(total) = total {
                        let percent = (bytes.len() as u64).saturating_mul(100) / total;
                        progress(percent.min(100) as u8);
                    }
                }
                Ok(Ok(None)) => break,
                _ => return Err(INTERRUPTED.to_owned()),
            }
        }
        if total.is_some_and(|total| total != bytes.len() as u64) {
            return Err(DAMAGED.to_owned());
        }
        Ok(bytes)
    }
}

/// `server:update`, asked by `who`: one at a time. Every member's app hears
/// how it goes, and the audit log gets a line. Once the new version is in
/// place the server stops, a moment after this has answered.
pub(crate) async fn update(server: Arc<Server>, who: Who) -> Result<ServerUpdateStatus, String> {
    let updates = &server.updates;
    if updates.busy.swap(true, Ordering::SeqCst) {
        return Err(BUSY.to_owned());
    }
    let tell = |status: ServerUpdateStatus| {
        server.nodus.emit(Event::ServerUpdate {
            server_id: String::new(),
            status,
        });
    };
    let done = updates.run(&tell).await;
    let running = nodus_core::VERSION;
    match &done {
        Ok(ServerUpdateStatus::Restarting { version }) => {
            server
                .auth
                .record(&who, "server.updated", json!({ "from": running, "to": version }));
        }
        Ok(_) => server
            .auth
            .record(&who, "server.up-to-date", json!({ "version": running })),
        Err(message) => server.auth.record(
            &who,
            "server.update-failed",
            json!({ "version": running, "error": message }),
        ),
    }
    match &done {
        // It stays busy: nothing else until it's gone.
        Ok(status @ ServerUpdateStatus::Restarting { .. }) => {
            tell(status.clone());
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                tokio::time::sleep(RESTART_AFTER).await;
                server.updates.restart.notify_one();
            });
        }
        Ok(status) => {
            tell(status.clone());
            updates.busy.store(false, Ordering::SeqCst);
        }
        Err(message) => {
            tell(ServerUpdateStatus::Error {
                message: message.clone(),
            });
            updates.busy.store(false, Ordering::SeqCst);
        }
    }
    done
}

/// What this machine takes from a release: `nodus-server-linux-x64.tar.gz`
/// or `nodus-server-linux-arm64.tar.gz` (SRV-201).
pub(crate) fn asset_name() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    format!("nodus-server-linux-{arch}.tar.gz")
}

/// A release to update to: its version and where its files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub version: String,
    pub archive: String,
    pub size: Option<u64>,
    pub signature: String,
}

/// The newest release in GitHub's list with `asset` and `<asset>.sig`, both
/// uploaded. Drafts, prereleases and tags that aren't a plain version are
/// left out, and so is a release whose server files didn't make it (the
/// server's jobs can fail without failing the app's release).
pub(crate) fn pick(releases: &[Value], asset: &str) -> Option<Found> {
    let signature_name = format!("{asset}.sig");
    let mut best: Option<Found> = None;
    for release in releases {
        let flagged = |key: &str| release.get(key).and_then(Value::as_bool) == Some(true);
        if flagged("draft") || flagged("prerelease") {
            continue;
        }
        let Some(version) = release.get("tag_name").and_then(Value::as_str).and_then(tag_version) else {
            continue;
        };
        let assets = release
            .get("assets")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let uploaded = |name: &str| {
            assets.iter().find(|a| {
                a.get("name").and_then(Value::as_str) == Some(name)
                    && a.get("state").and_then(Value::as_str) == Some("uploaded")
            })
        };
        let url = |a: &Value| a.get("browser_download_url").and_then(Value::as_str).map(str::to_owned);
        let (Some(archive), Some(signature)) = (uploaded(asset), uploaded(&signature_name)) else {
            continue;
        };
        let (Some(archive_url), Some(signature_url)) = (url(archive), url(signature)) else {
            continue;
        };
        if best.as_ref().is_some_and(|best| !is_newer(version, &best.version)) {
            continue;
        }
        best = Some(Found {
            version: version.to_owned(),
            archive: archive_url,
            size: archive.get("size").and_then(Value::as_u64),
            signature: signature_url,
        });
    }
    best
}

/// The key updates are checked against: the release's, or in a debug
/// build the one [`DEV_KEY_VAR`] gives. None while there's no key.
fn release_key() -> Option<[u8; 32]> {
    #[cfg(debug_assertions)]
    if let Some(pem) = std::env::var(DEV_KEY_VAR).ok().filter(|pem| !pem.is_empty()) {
        return parse_key(&pem);
    }
    parse_key(RELEASE_KEY)
}

/// The DER prefix of an ed25519 SubjectPublicKeyInfo, before its 32 bytes.
const ED25519_SPKI: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];

/// The 32 bytes of an ed25519 public key in PEM, as `openssl pkey -pubout`
/// writes it. None for the placeholder, or anything else.
pub(crate) fn parse_key(pem: &str) -> Option<[u8; 32]> {
    use base64::Engine as _;
    let body: String = pem
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "-----BEGIN PUBLIC KEY-----")
        .skip(1)
        .take_while(|line| *line != "-----END PUBLIC KEY-----")
        .collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body).ok()?;
    der.strip_prefix(&ED25519_SPKI[..])?.try_into().ok()
}

/// SRV-201: the signature over the whole archive.
#[cfg(not(windows))]
fn verify(key: &[u8; 32], archive: &[u8], signature: &[u8]) -> Result<(), String> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(archive, signature)
        .map_err(|_| BAD_SIGNATURE.to_owned())
}

/// A server runs on Linux; on Windows nothing is installed.
#[cfg(windows)]
fn verify(_: &[u8; 32], _: &[u8], _: &[u8]) -> Result<(), String> {
    Err(BAD_SIGNATURE.to_owned())
}

/// `nodus` from a `.tar.gz` as `package.sh` packs it: a plain file at the
/// archive's top.
pub(crate) fn unpack(archive: &[u8]) -> Result<Vec<u8>, String> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(archive)
        .take(MAX_BINARY as u64 + 1024 * 1024)
        .read_to_end(&mut tar)
        .map_err(|_| DAMAGED.to_owned())?;
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let text = |field: &[u8]| {
            let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
            String::from_utf8_lossy(&field[..end]).into_owned()
        };
        let size = octal(&header[124..136]).ok_or(DAMAGED)?;
        let start = at + 512;
        let end = start.checked_add(size).filter(|&end| end <= tar.len()).ok_or(DAMAGED)?;
        let (name, prefix) = (text(&header[..100]), text(&header[345..500]));
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let plain_file = matches!(header[156], b'0' | 0);
        if plain_file && matches!(name.as_str(), "nodus" | "./nodus") {
            if size > MAX_BINARY {
                return Err(DAMAGED.to_owned());
            }
            return Ok(tar[start..end].to_vec());
        }
        at = start + size.div_ceil(512) * 512;
    }
    Err(NO_BINARY.to_owned())
}

/// A tar header's octal number.
fn octal(field: &[u8]) -> Option<usize> {
    let digits: String = field
        .iter()
        .map(|&b| b as char)
        .filter(|c| !matches!(c, '\0' | ' '))
        .collect();
    usize::from_str_radix(&digits, 8).ok()
}

/// On Linux, the new binary is an ELF executable for this machine, as
/// `install-server.sh` checks by running it: a release for the other
/// architecture, or anything else, isn't installed.
fn check_machine(binary: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let machine: u16 = match std::env::consts::ARCH {
            "x86_64" => 62,
            "aarch64" => 183,
            _ => return Ok(()),
        };
        let elf = binary.len() > 20 && binary.starts_with(b"\x7fELF");
        if !elf || u16::from_le_bytes([binary[18], binary[19]]) != machine {
            return Err(OTHER_MACHINE.to_owned());
        }
    }
    let _ = (binary, OTHER_MACHINE);
    Ok(())
}

/// Writes `binary` beside `exe` as `<name>.new`, syncs it to disk and
/// renames it over `exe`, so `exe` is always one version or the other,
/// whole. A process running the old one keeps its copy until it exits.
pub(crate) fn install(exe: &Path, binary: &[u8]) -> Result<(), String> {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
        return Err(NO_EXECUTABLE.to_owned());
    };
    let new = dir.join(format!("{}.new", name.to_string_lossy()));
    let wrote = (|| -> std::io::Result<()> {
        let _ = std::fs::remove_file(&new);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o755);
        }
        let mut file = options.open(&new)?;
        file.write_all(binary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&new, exe)?;
        // The rename itself, on disk.
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    wrote.map_err(|err| {
        let _ = std::fs::remove_file(&new);
        format!("Can’t put the new version in {}: {err}", dir.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, files: &[&str]) -> Value {
        json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "assets": files.iter().map(|name| json!({
                "name": name,
                "state": "uploaded",
                "size": 10,
                "browser_download_url": format!("https://example.com/{tag}/{name}"),
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn picks_the_newest_release_with_a_signed_server_for_this_machine() {
        let asset = "nodus-server-linux-x64.tar.gz";
        let sig = "nodus-server-linux-x64.tar.gz.sig";
        let mut draft = release("v9.0.0", &[asset, sig]);
        draft["draft"] = json!(true);
        let mut pre = release("v8.0.0", &[asset, sig]);
        pre["prerelease"] = json!(true);
        let mut uploading = release("v7.0.0", &[asset, sig]);
        uploading["assets"][1]["state"] = json!("starter");
        let releases = [
            draft,
            pre,
            uploading,
            // No server in it: its jobs failed.
            release("v6.0.0", &["Nodus-arm64.dmg"]),
            // Unsigned.
            release("v5.0.0", &[asset]),
            release("v1.10.0", &[asset, sig]),
            release("v1.9.0", &[asset, sig]),
            release("nightly", &[asset, sig]),
            release(
                "v1.2.0",
                &["nodus-server-linux-arm64.tar.gz", "nodus-server-linux-arm64.tar.gz.sig"],
            ),
        ];
        let found = pick(&releases, asset).unwrap();
        assert_eq!(found.version, "1.10.0");
        assert_eq!(found.archive, format!("https://example.com/v1.10.0/{asset}"));
        assert_eq!(found.signature, format!("https://example.com/v1.10.0/{sig}"));
        assert_eq!(found.size, Some(10));
        assert_eq!(pick(&releases[..5], asset), None);
        assert_eq!(pick(&[], asset), None);
    }

    #[test]
    fn names_the_archive_for_this_machine() {
        let name = asset_name();
        assert!(
            name == "nodus-server-linux-x64.tar.gz" || name == "nodus-server-linux-arm64.tar.gz",
            "{name}"
        );
    }

    #[test]
    fn reads_an_ed25519_public_key_and_nothing_else() {
        // RFC 8410's example key.
        let pem = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----\n";
        let key = parse_key(pem).unwrap();
        assert_eq!(key[..4], [0x19, 0xbf, 0x44, 0x09]);
        assert_eq!(parse_key(RELEASE_KEY.lines().next().unwrap()), None);
        assert_eq!(parse_key("# TODO: placeholder\n"), None);
        assert_eq!(parse_key(""), None);
        // An RSA key isn't one.
        let rsa = "-----BEGIN PUBLIC KEY-----\nMFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8QuKUpRKfFLfRYC9AIKjbJTWit+CqvjWYzvQwECAwEAAQ==\n-----END PUBLIC KEY-----";
        assert_eq!(parse_key(rsa), None);
    }

    /// The public key's PEM without the comment lines a placeholder has: what
    /// `install-server.sh` must hold too.
    fn key_pem(file: &str) -> String {
        file.lines()
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned()
    }

    /// SRV-201: the key the binary builds in and the one `install-server.sh`
    /// checks downloads with are the same, the placeholder included.
    #[test]
    fn the_install_script_and_the_binary_trust_the_same_key() {
        let script = include_str!("../../../packaging/server/install-server.sh");
        let block = script
            .split_once("# BEGIN SIGNING KEY\n")
            .and_then(|(_, rest)| rest.split_once("# END SIGNING KEY"))
            .map(|(block, _)| block)
            .expect("the script's SIGNING KEY markers");
        let embedded = block
            .split_once("readonly SIGNING_KEY='")
            .and_then(|(_, rest)| rest.split_once('\''))
            .map(|(key, _)| key.trim())
            .expect("readonly SIGNING_KEY='…'");
        assert_eq!(embedded, key_pem(RELEASE_KEY));
        // Either both say there's no key, or both hold the same ed25519 key.
        assert_eq!(parse_key(embedded), parse_key(RELEASE_KEY));
        assert_eq!(embedded.is_empty(), parse_key(RELEASE_KEY).is_none());
    }

    fn tar_gz(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut tar = Vec::new();
        for (name, kind, data) in entries {
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..107].copy_from_slice(b"0000755");
            let size = format!("{:011o}", data.len());
            header[124..135].copy_from_slice(size.as_bytes());
            header[156] = *kind;
            header[257..262].copy_from_slice(b"ustar");
            tar.extend_from_slice(&header);
            tar.extend_from_slice(data);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.extend_from_slice(&[0u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn takes_nodus_out_of_the_archive() {
        let archive = tar_gz(&[("README", b'0', b"read me"), ("nodus", b'0', b"\x7fELF the binary")]);
        assert_eq!(unpack(&archive).unwrap(), b"\x7fELF the binary");
        let dotted = tar_gz(&[("./nodus", 0, b"binary")]);
        assert_eq!(unpack(&dotted).unwrap(), b"binary");
        // A link by that name isn't the binary.
        let link = tar_gz(&[("nodus", b'2', b"")]);
        assert_eq!(unpack(&link).unwrap_err(), NO_BINARY);
        assert_eq!(unpack(&tar_gz(&[("other", b'0', b"x")])).unwrap_err(), NO_BINARY);
        let mut cut = tar_gz(&[("nodus", b'0', &[7u8; 5000])]);
        cut.truncate(cut.len() / 2);
        assert_eq!(unpack(&cut).unwrap_err(), DAMAGED);
        assert_eq!(unpack(b"not gzip at all").unwrap_err(), DAMAGED);
    }

    #[test]
    fn swaps_the_binary_whole() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("nodus");
        std::fs::write(&exe, b"old").unwrap();
        install(&exe, b"new version").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new version");
        assert!(!dir.path().join("nodus.new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        let err = install(&dir.path().join("gone/nodus"), b"x").unwrap_err();
        assert!(err.starts_with("Can’t put the new version in "), "{err}");
    }
}
