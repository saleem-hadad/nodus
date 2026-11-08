//! Electron's `safeStorage`, as 0.9.4 uses it to keep the xAI key: the same
//! ciphertext, under the same key, so a key saved by 0.9.4 still opens and a
//! key saved here still opens in 0.9.4 (PLAT-115…PLAT-121, decision D6).
//!
//! The formats, checked against Electron 44.5.1's own copy of Chromium's
//! `components/os_crypt/sync` (its `revert_oscrypt_remove_sync_backend.patch`)
//! and `components/os_crypt/common/keychain_password_mac.mm`:
//!
//! - **macOS** ([`mac`]): `"v10"` + AES-128-CBC with PKCS#7 padding, the IV
//!   sixteen spaces, the key PBKDF2-HMAC-SHA1 of the Keychain password (its
//!   bytes as stored) with salt `saltysalt`, 1003 iterations, 16 bytes. The
//!   password is a generic password in the login keychain, service
//!   `Nodus Safe Storage`, account `Nodus Key`; when there's none, Chromium
//!   moves one saved under the old account `Nodus` to it, or else creates 16
//!   random bytes, base64-encoded (24 characters). The Keychain is asked once
//!   per launch, and a failure lasts until the next one.
//! - **Windows** ([`win`]): `"v10"` + a random 12-byte nonce + AES-256-GCM
//!   (no associated data) + its 16-byte tag. The 32-byte key is in
//!   `<userData>\Local State` at `os_crypt.encrypted_key`: the base64 of
//!   `"DPAPI"` and the key protected with DPAPI for the current user. Chromium creates it
//!   at startup when it's missing or DPAPI can't open it.
//!
//! In state.json the ciphertext is base64 ([`encode_base64`], and Node's
//! lenient [`decode_base64`] to read it).

pub mod mac;
pub mod win;

use std::path::Path;

/// What Settings says when a key can't be saved (PLAT-119; the text belongs to
/// spec 05's `voice:set-key`).
pub const CANNOT_SAVE_KEY: &str = if cfg!(target_os = "macos") {
    "The Keychain isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead."
} else {
    "Encrypted storage isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead."
};

/// Why `safeStorage` refused, in Electron's words. Only ever logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SafeStorageError {
    #[error("Error while encrypting the text provided to safeStorage.encryptString. Encryption is not available.")]
    EncryptionUnavailable,
    #[error("Error while encrypting the text provided to safeStorage.encryptString.")]
    EncryptFailed,
    #[error(
        "Error while decrypting the ciphertext provided to safeStorage.decryptString. Decryption is not available."
    )]
    DecryptionUnavailable,
    #[error(
        "Error while decrypting the ciphertext provided to safeStorage.decryptString. Ciphertext does not appear to be encrypted."
    )]
    NotEncrypted,
    #[error("Error while decrypting the ciphertext provided to safeStorage.decryptString.")]
    DecryptFailed,
}

/// One OS's way of sealing strings (Chromium's `OSCryptImpl`).
pub(crate) trait Scheme: Send + Sync {
    /// Whether there's a key, finding or making it the first time.
    fn available(&self) -> bool;
    fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>>;
    fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>>;
}

/// No key anywhere: an OS 0.9.4 doesn't ship for.
struct Unavailable;

impl Scheme for Unavailable {
    fn available(&self) -> bool {
        false
    }
    fn encrypt(&self, _: &[u8]) -> Option<Vec<u8>> {
        None
    }
    fn decrypt(&self, _: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

/// Electron's `safeStorage`: encrypts and decrypts strings with the OS's key.
pub struct SafeStorage {
    scheme: Box<dyn Scheme>,
}

impl std::fmt::Debug for SafeStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SafeStorage")
    }
}

impl SafeStorage {
    /// The OS's own: the Keychain on macOS, `<user_data>/Local State` and
    /// DPAPI on Windows. Nothing is read until it's first used.
    pub fn system(user_data: &Path) -> Self {
        #[cfg(target_os = "macos")]
        {
            let _ = user_data;
            Self::from_scheme(mac::KeychainScheme::new(mac::SystemKeychain, mac::APP_NAME))
        }
        #[cfg(windows)]
        {
            Self::from_scheme(win::LocalStateScheme::new(
                user_data.join(win::LOCAL_STATE),
                win::SystemDpapi,
            ))
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = user_data;
            Self::unavailable()
        }
    }

    /// One where encryption is never available.
    pub fn unavailable() -> Self {
        Self::from_scheme(Unavailable)
    }

    /// The macOS format with a Keychain password given here instead, for
    /// tests: nothing touches the real Keychain.
    pub fn with_mac_password(password: &[u8]) -> Self {
        Self::from_scheme(mac::FixedKey(mac::derive_key(password)))
    }

    /// The Windows format with the AES key given here instead, for tests.
    pub fn with_windows_key(key: [u8; 32]) -> Self {
        Self::from_scheme(win::FixedKey(key))
    }

    pub(crate) fn from_scheme(scheme: impl Scheme + 'static) -> Self {
        Self {
            scheme: Box::new(scheme),
        }
    }

    /// `safeStorage.isEncryptionAvailable()`. On macOS the first call reads
    /// the Keychain item, creating it if there's none.
    pub fn is_encryption_available(&self) -> bool {
        self.scheme.available()
    }

    /// `safeStorage.encryptString(plaintext)`.
    pub fn encrypt_string(&self, plaintext: &str) -> Result<Vec<u8>, SafeStorageError> {
        if !self.scheme.available() {
            return Err(SafeStorageError::EncryptionUnavailable);
        }
        self.scheme
            .encrypt(plaintext.as_bytes())
            .ok_or(SafeStorageError::EncryptFailed)
    }

    /// `safeStorage.decryptString(ciphertext)`. Only `v10` and `v11`
    /// ciphertexts are taken, and invalid UTF-8 becomes U+FFFD, as V8 reads it.
    pub fn decrypt_string(&self, ciphertext: &[u8]) -> Result<String, SafeStorageError> {
        if !self.scheme.available() {
            return Err(SafeStorageError::DecryptionUnavailable);
        }
        if ciphertext.is_empty() {
            return Ok(String::new());
        }
        if !ciphertext.starts_with(b"v10") && !ciphertext.starts_with(b"v11") {
            return Err(SafeStorageError::NotEncrypted);
        }
        let plaintext = self.scheme.decrypt(ciphertext).ok_or(SafeStorageError::DecryptFailed)?;
        Ok(String::from_utf8_lossy(&plaintext).into_owned())
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `buffer.toString('base64')`: standard alphabet, padded.
pub fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_value(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// `Buffer.from(text, 'base64')`: lenient, as Node is. The URL-safe alphabet
/// is read too, anything outside the alphabet is skipped, decoding stops at
/// the first `=`, and a final group of two or three characters still gives
/// its bytes.
pub fn decode_base64(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut group = [0u8; 4];
    let mut len = 0;
    for &c in text.as_bytes() {
        if c == b'=' {
            break;
        }
        let Some(value) = base64_value(c) else { continue };
        group[len] = value;
        len += 1;
        if len == 4 {
            out.extend_from_slice(&decode_group(&group, 4));
            len = 0;
        }
    }
    if len >= 2 {
        out.extend_from_slice(&decode_group(&group, len));
    }
    out
}

/// Base64 as Chromium writes it, read strictly: standard alphabet, padded to
/// a multiple of four, nothing else. None for anything else.
pub(crate) fn decode_base64_strict(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut group = [0u8; 4];
        for (slot, &c) in group.iter_mut().zip(&chunk[..4 - pad]) {
            if c == b'-' || c == b'_' {
                return None;
            }
            *slot = base64_value(c)?;
        }
        out.extend_from_slice(&decode_group(&group, 4 - pad));
    }
    Some(out)
}

/// The bytes in the first `len` (2 to 4) characters of a group.
fn decode_group(group: &[u8; 4], len: usize) -> Vec<u8> {
    let n =
        (u32::from(group[0]) << 18) | (u32::from(group[1]) << 12) | (u32::from(group[2]) << 6) | u32::from(group[3]);
    let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
    bytes[..len - 1].to_vec()
}

/// Random bytes from the OS.
pub(crate) fn random_bytes<const N: usize>() -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic Keychain password, shaped like the ones Chromium makes.
    const PASSWORD: &[u8] = b"c3ludGhldGljLXRlc3Qta2V5";
    /// "xai-synthetic-0123456789abcdef" sealed under PASSWORD, as state.json
    /// would hold it (made with Python's hashlib and the openssl tool).
    const SEALED_MAC: &str = "djEwkKGEyUPtZyBXDEAvIrDkjvSHeaewSxsCaRrcSZeeFEA=";

    #[test]
    fn opens_a_key_sealed_the_macos_way() {
        let storage = SafeStorage::with_mac_password(PASSWORD);
        assert!(storage.is_encryption_available());
        let sealed = decode_base64(SEALED_MAC);
        assert_eq!(
            storage.decrypt_string(&sealed).unwrap(),
            "xai-synthetic-0123456789abcdef"
        );
        // Sealing is deterministic on macOS: the IV is fixed.
        let again = storage.encrypt_string("xai-synthetic-0123456789abcdef").unwrap();
        assert_eq!(encode_base64(&again), SEALED_MAC);
    }

    #[test]
    fn round_trips_both_formats() {
        for storage in [
            SafeStorage::with_mac_password(b"another-synthetic-password"),
            SafeStorage::with_windows_key([7; 32]),
        ] {
            for text in ["xai-key", "héllo 🎉", &"x".repeat(1000)] {
                let sealed = storage.encrypt_string(text).unwrap();
                assert!(sealed.starts_with(b"v10"));
                assert_eq!(storage.decrypt_string(&sealed).unwrap(), text);
            }
        }
    }

    #[test]
    fn refuses_what_electron_refuses() {
        let storage = SafeStorage::with_mac_password(PASSWORD);
        assert_eq!(storage.decrypt_string(b""), Ok(String::new()));
        assert_eq!(
            storage.decrypt_string(b"plain text"),
            Err(SafeStorageError::NotEncrypted)
        );
        // v11 passes Electron's check, then OSCrypt finds no v10 and fails.
        assert_eq!(
            storage.decrypt_string(b"v11aaaaaaaaaaaaaaaa"),
            Err(SafeStorageError::DecryptFailed)
        );
        // Sealed under another password: the padding doesn't check out.
        let other = SafeStorage::with_mac_password(b"wrong")
            .encrypt_string("secret")
            .unwrap();
        assert_eq!(storage.decrypt_string(&other), Err(SafeStorageError::DecryptFailed));
        assert_eq!(
            SafeStorageError::NotEncrypted.to_string(),
            "Error while decrypting the ciphertext provided to safeStorage.decryptString. \
             Ciphertext does not appear to be encrypted."
        );
    }

    #[test]
    fn says_so_when_there_is_no_key() {
        let storage = SafeStorage::unavailable();
        assert!(!storage.is_encryption_available());
        assert_eq!(
            storage.encrypt_string("x"),
            Err(SafeStorageError::EncryptionUnavailable)
        );
        assert_eq!(
            storage.decrypt_string(b"v10"),
            Err(SafeStorageError::DecryptionUnavailable)
        );
        assert_eq!(
            SafeStorageError::EncryptionUnavailable.to_string(),
            "Error while encrypting the text provided to safeStorage.encryptString. Encryption is not available."
        );
    }

    #[test]
    fn writes_base64_as_node_does() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(&[0xfb, 0xff, 0xfe]), "+//+");
    }

    #[test]
    fn reads_base64_leniently_as_node_does() {
        assert_eq!(decode_base64("Zm9v"), b"foo");
        assert_eq!(decode_base64("Zg=="), b"f");
        assert_eq!(decode_base64("Zg"), b"f");
        assert_eq!(decode_base64("Zm8"), b"fo");
        assert_eq!(decode_base64(" Zm\n9v\t"), b"foo");
        assert_eq!(decode_base64("-__-"), [0xfb, 0xff, 0xfe]);
        assert_eq!(decode_base64("Zm9v!Zm9v"), b"foofoo");
        assert_eq!(decode_base64("Zg==Zm9v"), b"f");
        assert_eq!(decode_base64("Z"), b"");
        assert_eq!(decode_base64(""), b"");
    }

    #[test]
    fn reads_base64_strictly_as_chromium_writes_it() {
        assert_eq!(decode_base64_strict("Zm9v").as_deref(), Some(&b"foo"[..]));
        assert_eq!(decode_base64_strict("Zg==").as_deref(), Some(&b"f"[..]));
        assert_eq!(decode_base64_strict("").as_deref(), Some(&b""[..]));
        assert_eq!(decode_base64_strict("Zg"), None);
        assert_eq!(decode_base64_strict("Zg==Zm9v"), None);
        assert_eq!(decode_base64_strict("Zm 9"), None);
        assert_eq!(decode_base64_strict("-__-"), None);
        for bytes in [&b""[..], b"a", b"ab", b"abc", b"DPAPI\x01\x02\x03"] {
            assert_eq!(decode_base64_strict(&encode_base64(bytes)).as_deref(), Some(bytes));
        }
    }

    #[test]
    fn says_why_a_key_cannot_be_saved() {
        if cfg!(target_os = "macos") {
            assert!(CANNOT_SAVE_KEY.starts_with("The Keychain isn’t available"));
        } else {
            assert!(CANNOT_SAVE_KEY.starts_with("Encrypted storage isn’t available"));
        }
        assert!(CANNOT_SAVE_KEY.ends_with("Set XAI_API_KEY in your shell instead."));
    }
}
