//! The setup code and the setup token (SRV-060).
//!
//! `nodus-server setup-code` and `serve` are two processes, so the code
//! lives in `<data>/setup.json` (0600), which both change only while
//! holding `<data>/setup.lock`. Only hashes are kept:
//!
//! - **The code**, `K7QM-4RXD`, is single-use, and a server takes 10 tries
//!   an hour at it, right or wrong. A new code cancels the old one, every
//!   setup token, and the tries so far.
//! - **A setup token** comes from claiming the code. It's good for wizard
//!   steps 1 and 2, until an hour passes without it being used, and never
//!   once the server has a member: the caller checks that (auth.rs).

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::store::{self, MEMBERS_FILE, now_ms, random_hex, sha256_hex};

const SETUP_FILE: &str = "setup.json";
const LOCK_FILE: &str = "setup.lock";
/// Letters and digits that can't be misread: no 0, O, 1 or I. 32 of them,
/// so a random byte picks one evenly.
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Claims a server takes in an hour, right or wrong.
pub(crate) const TRIES_PER_HOUR: usize = 10;
const HOUR_MS: f64 = 60.0 * 60.0 * 1000.0;
/// How long a setup token lasts unused.
pub(crate) const TOKEN_IDLE_MS: f64 = HOUR_MS;

/// `setup.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupFile {
    /// The code's hash, until it's claimed.
    #[serde(default)]
    code_hash: Option<String>,
    #[serde(default)]
    code_created_at: f64,
    /// When the claims of the last hour came.
    #[serde(default)]
    tries: Vec<f64>,
    #[serde(default)]
    tokens: Vec<SetupToken>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupToken {
    hash: String,
    last_used_at: f64,
}

/// What claiming a code came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Claim {
    /// The setup token, shown once.
    Token(String),
    /// Not the code, or one already claimed or cancelled.
    Wrong,
    /// 10 tries in the last hour.
    TooMany,
}

/// `nodus-server setup-code`: prints a fresh code on standard output, or
/// says on standard error why not; gives the exit code (SRV-001, SRV-060).
pub(crate) fn print_code() -> i32 {
    let fail = |err: &str| {
        let _ = writeln!(std::io::stderr().lock(), "nodus-server: {err}");
        1
    };
    let Some(data) = crate::data::data_dir() else {
        return fail("No data folder: set NODUS_USER_DATA or HOME");
    };
    if let Err(err) = nodus_host_proto::ensure_private_dir(&data) {
        return fail(&format!("Can't make {}: {err}", data.display()));
    }
    match store::read_json::<Vec<nodus_wire::Member>>(&data.join(MEMBERS_FILE)) {
        Ok(members) if members.is_empty() => {}
        Ok(_) => return fail("This server is set up already: members sign in with GitHub, and invite others"),
        Err(err) => return fail(&err),
    }
    match new_code(&data, now_ms()) {
        Ok(code) => {
            let _ = writeln!(std::io::stdout().lock(), "{code}");
            0
        }
        Err(err) => fail(&err),
    }
}

/// A fresh code, which cancels every earlier code and setup token.
pub(crate) fn new_code(data: &Path, now: f64) -> Result<String, String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    let letters: String = bytes.iter().map(|&b| ALPHABET[usize::from(b) % 32] as char).collect();
    let code = format!("{}-{}", &letters[..4], &letters[4..]);
    with_file(data, |file| {
        *file = SetupFile {
            code_hash: Some(sha256_hex(&normalize(&code))),
            code_created_at: now,
            tries: Vec::new(),
            tokens: Vec::new(),
        };
        (true, ())
    })?;
    Ok(code)
}

/// `POST /api/setup/claim`: the code for a setup token, once.
pub(crate) fn claim(data: &Path, code: &str, now: f64) -> Result<Claim, String> {
    with_file(data, |file| {
        file.tries.retain(|&at| now - at < HOUR_MS);
        if file.tries.len() >= TRIES_PER_HOUR {
            return (false, Claim::TooMany);
        }
        file.tries.push(now);
        let given = sha256_hex(&normalize(code));
        let right = file
            .code_hash
            .as_deref()
            .is_some_and(|hash| crate::net::same_token(&given, hash));
        if !right {
            return (true, Claim::Wrong);
        }
        file.code_hash = None;
        let token = random_hex(32);
        file.tokens.push(SetupToken {
            hash: sha256_hex(&token),
            last_used_at: now,
        });
        (true, Claim::Token(token))
    })
}

/// Whether a setup token is good, used now: one claimed from the current
/// code and used within the hour. Using it keeps it good another hour.
pub(crate) fn use_token(data: &Path, token: &str, now: f64) -> Result<bool, String> {
    check_token(data, token, now, true)
}

/// Whether a setup token is good, without using it: for a route that turns
/// it down, which mustn't keep it alive (SRV-060).
pub(crate) fn peek_token(data: &Path, token: &str, now: f64) -> Result<bool, String> {
    check_token(data, token, now, false)
}

fn check_token(data: &Path, token: &str, now: f64, using: bool) -> Result<bool, String> {
    let hash = sha256_hex(token);
    with_file(data, |file| {
        let before = file.tokens.len();
        file.tokens.retain(|kept| now - kept.last_used_at < TOKEN_IDLE_MS);
        let expired = file.tokens.len() != before;
        match file
            .tokens
            .iter_mut()
            .find(|kept| crate::net::same_token(&kept.hash, &hash))
        {
            Some(kept) if using => {
                kept.last_used_at = now;
                (true, true)
            }
            Some(_) => (expired, true),
            None => (expired, false),
        }
    })
}

/// Setup is over: the code and every setup token go (SRV-060).
pub(crate) fn finish(data: &Path) -> Result<(), String> {
    with_file(data, |file| {
        let had = file.code_hash.is_some() || !file.tokens.is_empty();
        *file = SetupFile::default();
        (had, ())
    })
}

/// `K7QM-4RXD`, `k7qm4rxd` and ` K7QM 4RXD ` are the same code.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Reads `setup.json` under the lock, lets `change` look and change it,
/// and writes it back when `change` says it changed it.
fn with_file<R>(data: &Path, change: impl FnOnce(&mut SetupFile) -> (bool, R)) -> Result<R, String> {
    let lock_path = data.join(LOCK_FILE);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|err| format!("Can't open {}: {err}", lock_path.display()))?;
    lock.lock()
        .map_err(|err| format!("Can't lock {}: {err}", lock_path.display()))?;
    let path = data.join(SETUP_FILE);
    let mut file: SetupFile = store::read_json(&path)?;
    let (changed, result) = change(&mut file);
    if changed {
        store::write_json(&path, &file)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_700_000_000_000.0;

    #[test]
    fn a_code_is_claimed_once() {
        let dir = tempfile::tempdir().unwrap();
        let code = new_code(dir.path(), NOW).unwrap();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        assert!(code.bytes().all(|b| b == b'-' || ALPHABET.contains(&b)), "{code}");

        // Typed any way, it's the same code.
        let typed = code.to_lowercase().replace('-', " ");
        let Claim::Token(token) = claim(dir.path(), &typed, NOW).unwrap() else {
            panic!("the code is right");
        };
        assert_eq!(claim(dir.path(), &code, NOW).unwrap(), Claim::Wrong);
        assert!(use_token(dir.path(), &token, NOW + 1000.0).unwrap());
        assert!(!use_token(dir.path(), "not-it", NOW).unwrap());
        // Only hashes are kept.
        let kept = std::fs::read_to_string(dir.path().join(SETUP_FILE)).unwrap();
        assert!(!kept.contains(&token) && !kept.contains(&code[..4]), "{kept}");
    }

    #[test]
    fn takes_ten_tries_an_hour() {
        let dir = tempfile::tempdir().unwrap();
        let code = new_code(dir.path(), NOW).unwrap();
        for i in 0..TRIES_PER_HOUR {
            assert_eq!(claim(dir.path(), "AAAA-AAAA", NOW + i as f64).unwrap(), Claim::Wrong);
        }
        // The right code too, until the hour has passed.
        assert_eq!(claim(dir.path(), &code, NOW + 60_000.0).unwrap(), Claim::TooMany);
        assert!(matches!(
            claim(dir.path(), &code, NOW + HOUR_MS).unwrap(),
            Claim::Token(_)
        ));
    }

    #[test]
    fn a_token_lasts_an_hour_unused_and_a_new_code_cancels_it() {
        let dir = tempfile::tempdir().unwrap();
        let code = new_code(dir.path(), NOW).unwrap();
        let Claim::Token(token) = claim(dir.path(), &code, NOW).unwrap() else {
            panic!("the code is right");
        };
        // Used, it lasts another hour from then.
        assert!(use_token(dir.path(), &token, NOW + TOKEN_IDLE_MS - 1.0).unwrap());
        assert!(use_token(dir.path(), &token, NOW + 2.0 * TOKEN_IDLE_MS - 2.0).unwrap());
        // Looking at it doesn't keep it alive.
        assert!(peek_token(dir.path(), &token, NOW + 3.0 * TOKEN_IDLE_MS - 3.0).unwrap());
        assert!(!peek_token(dir.path(), &token, NOW + 3.0 * TOKEN_IDLE_MS - 1.0).unwrap());
        assert!(!use_token(dir.path(), &token, NOW + 3.0 * TOKEN_IDLE_MS).unwrap());

        let code = new_code(dir.path(), NOW).unwrap();
        let Claim::Token(token) = claim(dir.path(), &code, NOW).unwrap() else {
            panic!("the code is right");
        };
        let newer = new_code(dir.path(), NOW).unwrap();
        assert!(!use_token(dir.path(), &token, NOW).unwrap());
        // And finishing setup ends the code too.
        finish(dir.path()).unwrap();
        assert_eq!(claim(dir.path(), &newer, NOW).unwrap(), Claim::Wrong);
    }
}
