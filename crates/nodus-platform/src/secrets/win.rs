//! The Windows format: `"v10"` + nonce + AES-256-GCM, under a key kept in
//! `<userData>\Local State`, protected with DPAPI (Chromium's
//! `os_crypt_win.cc`, as Electron 44 restores it).
//!
//! Everything but the DPAPI calls is compiled everywhere so it's tested
//! everywhere; [`SystemDpapi`] needs Windows.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde_json::{Map, Value};

use super::{Scheme, decode_base64_strict, encode_base64, random_bytes};

/// Chromium's browser-wide preferences file in userData.
pub const LOCAL_STATE: &str = "Local State";

const PREFIX: &[u8] = b"v10";
const DPAPI_PREFIX: &[u8] = b"DPAPI";
const NONCE_LEN: usize = 12;
const SECTION: &str = "os_crypt";
const ENCRYPTED_KEY: &str = "encrypted_key";
const AUDIT_ENABLED: &str = "audit_enabled";

/// Seals `plaintext`: `"v10"` + `nonce` + AES-256-GCM with no associated
/// data, the 16-byte tag last.
pub fn encrypt(key: &[u8; 32], nonce: [u8; NONCE_LEN], plaintext: &[u8]) -> Option<Vec<u8>> {
    let sealed = Aes256Gcm::new(key.into())
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .ok()?;
    Some([PREFIX, &nonce, &sealed].concat())
}

/// Opens what [`encrypt`] sealed. None without the prefix, or when the tag
/// doesn't check out.
pub fn decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Option<Vec<u8>> {
    let rest = ciphertext.strip_prefix(PREFIX)?;
    if rest.len() < NONCE_LEN {
        return None;
    }
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    Aes256Gcm::new(key.into())
        .decrypt(Nonce::from_slice(nonce), sealed)
        .ok()
}

/// The current user's DPAPI: `CryptProtectData`, `CryptUnprotectData`.
pub trait Dpapi: Send + Sync {
    fn protect(&self, data: &[u8]) -> Option<Vec<u8>>;
    fn unprotect(&self, data: &[u8]) -> Option<Vec<u8>>;
}

/// What Chromium's `InitWithExistingKey` found.
#[derive(Debug, PartialEq, Eq)]
enum Existing {
    Key([u8; 32]),
    DoesNotExist,
    InvalidFormat,
    DecryptionFailed,
}

/// The key in Local State, or a new one put there: Chromium's
/// `OSCryptImpl::Init`. A key DPAPI can't open is replaced (the user's
/// password was reset); one that isn't DPAPI's at all means no encryption.
pub fn init_key(path: &Path, dpapi: &dyn Dpapi) -> Option<[u8; 32]> {
    let mut state = read_local_state(path);
    match existing_key(&state, dpapi) {
        Existing::Key(key) => {
            let section = section(&mut state);
            if section.get(AUDIT_ENABLED) != Some(&Value::Bool(true)) {
                // Chromium protects the key again with auditing on, once.
                if let Some(encrypted) = protect_key(&key, dpapi) {
                    section.insert(ENCRYPTED_KEY.into(), Value::String(encrypted));
                }
                section.insert(AUDIT_ENABLED.into(), Value::Bool(true));
                write_local_state(path, &state);
            }
            Some(key)
        }
        Existing::InvalidFormat => None,
        Existing::DoesNotExist | Existing::DecryptionFailed => {
            let key = random_bytes::<32>()?;
            let encrypted = protect_key(&key, dpapi)?;
            let section = section(&mut state);
            section.insert(ENCRYPTED_KEY.into(), Value::String(encrypted));
            section.insert(AUDIT_ENABLED.into(), Value::Bool(true));
            write_local_state(path, &state);
            Some(key)
        }
    }
}

fn existing_key(state: &Map<String, Value>, dpapi: &dyn Dpapi) -> Existing {
    let Some(value) = state.get(SECTION).and_then(|section| section.get(ENCRYPTED_KEY)) else {
        return Existing::DoesNotExist;
    };
    let encoded = value.as_str().unwrap_or_default();
    let with_header = decode_base64_strict(encoded).unwrap_or_default();
    let Some(protected) = with_header.strip_prefix(DPAPI_PREFIX) else {
        return Existing::InvalidFormat;
    };
    let Some(key) = dpapi.unprotect(protected) else {
        return Existing::DecryptionFailed;
    };
    // Chromium would crash on a key of another size; here it's no encryption.
    match <[u8; 32]>::try_from(key.as_slice()) {
        Ok(key) => Existing::Key(key),
        Err(_) => Existing::InvalidFormat,
    }
}

/// base64(`"DPAPI"` + the key protected with DPAPI).
fn protect_key(key: &[u8; 32], dpapi: &dyn Dpapi) -> Option<String> {
    let protected = dpapi.protect(key)?;
    Some(encode_base64(&[DPAPI_PREFIX, &protected].concat()))
}

/// The `os_crypt` dictionary, made if it's missing or isn't one.
fn section(state: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let entry = state.entry(SECTION).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    match entry {
        Value::Object(section) => section,
        _ => unreachable!("made an object just above"),
    }
}

/// Local State as JSON; a missing or unreadable file is empty, as Chromium
/// starts afresh from one.
fn read_local_state(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

/// Writes Local State back whole, compact as Chromium does, through a
/// temporary file renamed over it. Every other preference is kept as it was.
fn write_local_state(path: &Path, state: &Map<String, Value>) {
    let Ok(text) = nodus_domain::json::to_string(state) else {
        return;
    };
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", std::process::id()));
    let temp = PathBuf::from(temp);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&temp, text).is_ok() && std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
}

/// The Windows scheme: the key from Local State, found or made the first time
/// it's needed (Chromium does it at startup), and that answer kept.
pub struct LocalStateScheme<D> {
    path: PathBuf,
    dpapi: D,
    key: OnceLock<Option<[u8; 32]>>,
}

impl<D: Dpapi> LocalStateScheme<D> {
    pub fn new(path: PathBuf, dpapi: D) -> Self {
        Self {
            path,
            dpapi,
            key: OnceLock::new(),
        }
    }

    fn key(&self) -> Option<&[u8; 32]> {
        self.key.get_or_init(|| init_key(&self.path, &self.dpapi)).as_ref()
    }
}

impl<D: Dpapi> Scheme for LocalStateScheme<D> {
    fn available(&self) -> bool {
        self.key().is_some()
    }

    fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
        encrypt(self.key()?, random_bytes::<NONCE_LEN>()?, plaintext)
    }

    /// Anything without the prefix is taken for data DPAPI sealed directly,
    /// as Chromium's oldest versions did.
    fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        if !ciphertext.starts_with(PREFIX) {
            return self.dpapi.unprotect(ciphertext);
        }
        decrypt(self.key()?, ciphertext)
    }
}

/// A key given outright, for tests.
pub(super) struct FixedKey(pub(super) [u8; 32]);

impl Scheme for FixedKey {
    fn available(&self) -> bool {
        true
    }
    fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
        encrypt(&self.0, random_bytes::<NONCE_LEN>()?, plaintext)
    }
    fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        decrypt(&self.0, ciphertext)
    }
}

/// DPAPI for the current user, no entropy, as Chromium calls it: protecting
/// with `CRYPTPROTECT_AUDIT`, unprotecting with no flags.
#[cfg(windows)]
pub struct SystemDpapi;

#[cfg(windows)]
impl Dpapi for SystemDpapi {
    fn protect(&self, data: &[u8]) -> Option<Vec<u8>> {
        use windows_sys::Win32::Security::Cryptography::{CRYPTPROTECT_AUDIT, CryptProtectData};
        let description: Vec<u16> = "Nodus".encode_utf16().chain(Some(0)).collect();
        let input = blob(data)?;
        let mut output = empty_blob();
        // SAFETY: `input` points at `data`, which outlives the call;
        // `description` is NUL-terminated; the optional pointers are null;
        // `output` is written by the call and freed by `take_blob`.
        let ok = unsafe {
            CryptProtectData(
                &input,
                description.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_AUDIT,
                &mut output,
            )
        };
        (ok != 0).then(|| take_blob(output))
    }

    fn unprotect(&self, data: &[u8]) -> Option<Vec<u8>> {
        use windows_sys::Win32::Security::Cryptography::CryptUnprotectData;
        let input = blob(data)?;
        let mut output = empty_blob();
        // SAFETY: as in `protect`; the description out-pointer is null, so
        // nothing else is allocated.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            )
        };
        (ok != 0).then(|| take_blob(output))
    }
}

#[cfg(windows)]
fn blob(data: &[u8]) -> Option<windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB> {
    Some(windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).ok()?,
        pbData: data.as_ptr().cast_mut(),
    })
}

#[cfg(windows)]
fn empty_blob() -> windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
    windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    }
}

/// Copies out what DPAPI allocated, and frees it.
#[cfg(windows)]
fn take_blob(output: windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if output.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: DPAPI returned `cbData` bytes at `pbData`, allocated with
    // LocalAlloc; they're copied before being freed, once.
    unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        windows_sys::Win32::Foundation::LocalFree(output.pbData.cast());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::*;

    /// Stands in for DPAPI: "sealed by" a user, reversible only by them.
    struct FakeDpapi {
        user: &'static str,
        unprotects: Mutex<usize>,
    }

    impl FakeDpapi {
        fn new(user: &'static str) -> Self {
            Self {
                user,
                unprotects: Mutex::new(0),
            }
        }
    }

    impl Dpapi for FakeDpapi {
        fn protect(&self, data: &[u8]) -> Option<Vec<u8>> {
            Some([self.user.as_bytes(), b":", data].concat())
        }
        fn unprotect(&self, data: &[u8]) -> Option<Vec<u8>> {
            *self.unprotects.lock().unwrap() += 1;
            data.strip_prefix(self.user.as_bytes())?
                .strip_prefix(b":")
                .map(<[u8]>::to_vec)
        }
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn local_state(dir: &TempDir) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.path().join(LOCAL_STATE)).unwrap()).unwrap()
    }

    fn stored_key(state: &Value) -> Vec<u8> {
        let encoded = state[SECTION][ENCRYPTED_KEY].as_str().unwrap();
        decode_base64_strict(encoded).unwrap()
    }

    #[test]
    fn seals_with_aes_256_gcm_as_boringssl_does() {
        // McGrew and Viega's GCM test cases 13 and 14 (zero key and nonce).
        let zero = [0u8; 32];
        assert_eq!(
            encrypt(&zero, [0; 12], b"").unwrap()[15..],
            hex("530f8afbc74536b9a963b4f1c4cb738b")[..]
        );
        assert_eq!(
            encrypt(&zero, [0; 12], &[0; 16]).unwrap()[15..],
            hex("cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919")[..]
        );
        // A synthetic key and nonce, sealed with OpenSSL's EVP_aes_256_gcm.
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let nonce: [u8; 12] = std::array::from_fn(|i| 0xa0 + i as u8);
        let sealed = encrypt(&key, nonce, b"xai-synthetic-0123456789abcdef").unwrap();
        let expected = [
            &b"v10"[..],
            &nonce,
            &hex("9e79150036b26ccb0a00f3ba6457f0ef429f6d25a4807a55fd6c45e21acd"),
            &hex("09df3221df6fc801d629856de61cb3a0"),
        ]
        .concat();
        assert_eq!(sealed, expected);
        assert_eq!(decrypt(&key, &sealed).unwrap(), b"xai-synthetic-0123456789abcdef");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(decrypt(&key, &tampered), None);
        assert_eq!(decrypt(&key, b"v10short"), None);
        assert_eq!(decrypt(&key, b"v11aaaaaaaaaaaaaaaaaaaaaaaaaaaa"), None);
    }

    #[test]
    fn makes_a_key_when_local_state_has_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(LOCAL_STATE);
        std::fs::write(
            &path,
            r#"{"browser":{"enabled_labs_experiments":[]},"os_crypt":{"other":1}}"#,
        )
        .unwrap();
        let key = init_key(&path, &FakeDpapi::new("alice")).unwrap();
        let state = local_state(&dir);
        assert_eq!(stored_key(&state), [&b"DPAPIalice:"[..], &key].concat());
        assert_eq!(state[SECTION][AUDIT_ENABLED], Value::Bool(true));
        // Everything else stays, in its order.
        assert_eq!(state["browser"]["enabled_labs_experiments"], Value::Array(vec![]));
        assert_eq!(state[SECTION]["other"], Value::from(1));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with(r#"{"browser":{"enabled_labs_experiments":[]},"os_crypt":{"other":1,"encrypted_key":""#)
        );
        // With no file at all.
        let empty = TempDir::new().unwrap();
        assert!(init_key(&empty.path().join(LOCAL_STATE), &FakeDpapi::new("alice")).is_some());
        assert_eq!(stored_key(&local_state(&empty)).len(), 5 + 6 + 32);
    }

    #[test]
    fn uses_the_key_local_state_has() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(LOCAL_STATE);
        let key = [9u8; 32];
        let encrypted = encode_base64(&[&b"DPAPIalice:"[..], &key].concat());
        let text = format!(r#"{{"os_crypt":{{"audit_enabled":true,"encrypted_key":"{encrypted}"}}}}"#);
        std::fs::write(&path, &text).unwrap();
        assert_eq!(init_key(&path, &FakeDpapi::new("alice")), Some(key));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "left as it was");
    }

    #[test]
    fn turns_auditing_on_for_an_older_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(LOCAL_STATE);
        let key = [9u8; 32];
        let encrypted = encode_base64(&[&b"DPAPIalice:"[..], &key].concat());
        std::fs::write(&path, format!(r#"{{"os_crypt":{{"encrypted_key":"{encrypted}"}}}}"#)).unwrap();
        assert_eq!(init_key(&path, &FakeDpapi::new("alice")), Some(key));
        let state = local_state(&dir);
        assert_eq!(state[SECTION][AUDIT_ENABLED], Value::Bool(true));
        assert_eq!(stored_key(&state), [&b"DPAPIalice:"[..], &key].concat());
    }

    #[test]
    fn replaces_a_key_dpapi_cannot_open() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(LOCAL_STATE);
        let encrypted = encode_base64(&[&b"DPAPIbob:"[..], &[9u8; 32]].concat());
        std::fs::write(&path, format!(r#"{{"os_crypt":{{"encrypted_key":"{encrypted}"}}}}"#)).unwrap();
        let key = init_key(&path, &FakeDpapi::new("alice")).unwrap();
        assert_ne!(key, [9u8; 32]);
        assert_eq!(stored_key(&local_state(&dir)), [&b"DPAPIalice:"[..], &key].concat());
    }

    #[test]
    fn has_no_key_when_local_state_holds_something_else() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(LOCAL_STATE);
        for stored in [encode_base64(b"NOTDPAPI"), "not base64!".into(), String::new()] {
            let text = format!(r#"{{"os_crypt":{{"encrypted_key":"{stored}"}}}}"#);
            std::fs::write(&path, &text).unwrap();
            assert_eq!(init_key(&path, &FakeDpapi::new("alice")), None);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "left alone");
        }
    }

    #[test]
    fn reads_local_state_once_and_round_trips() {
        let dir = TempDir::new().unwrap();
        let scheme = LocalStateScheme::new(dir.path().join(LOCAL_STATE), FakeDpapi::new("alice"));
        assert!(scheme.available());
        let sealed = scheme.encrypt(b"xai-key").unwrap();
        assert_eq!(sealed.len(), 3 + 12 + 7 + 16);
        assert_eq!(scheme.decrypt(&sealed).unwrap(), b"xai-key");
        // A fresh nonce each time.
        assert_ne!(scheme.encrypt(b"xai-key").unwrap(), sealed);
        // A second launch reads the same key.
        let again = LocalStateScheme::new(dir.path().join(LOCAL_STATE), FakeDpapi::new("alice"));
        assert_eq!(again.decrypt(&sealed).unwrap(), b"xai-key");
        // Unprefixed data is DPAPI's own.
        assert_eq!(again.decrypt(b"alice:legacy").unwrap(), b"legacy");
        assert_eq!(
            *again.dpapi.unprotects.lock().unwrap(),
            2,
            "one for the key, one for the legacy blob"
        );
    }
}
