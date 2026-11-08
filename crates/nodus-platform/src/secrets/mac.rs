//! The macOS format: `"v10"` + AES-128-CBC under a key derived from a
//! Keychain password (Chromium's `os_crypt_mac.mm` and
//! `keychain_password_mac.mm`, as Electron 44 patches them).
//!
//! The algorithm is compiled everywhere so it's tested everywhere; only
//! [`SystemKeychain`] needs macOS.

use std::sync::OnceLock;

use aes::Aes128;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};

use super::{Scheme, encode_base64, random_bytes};

/// The app's name, which names the Keychain item: Electron sets the service
/// to `<name> Safe Storage` and the account to `<name>`, which its patch
/// suffixes with ` Key`.
pub const APP_NAME: &str = "Nodus";

const PREFIX: &[u8] = b"v10";
const SALT: &[u8] = b"saltysalt";
const ITERATIONS: u32 = 1003;
const IV: [u8; 16] = [b' '; 16];

/// `errSecItemNotFound`.
pub const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// The AES key for a Keychain password: PBKDF2-HMAC-SHA1 over the password's
/// bytes as stored, salt `saltysalt`, 1003 iterations, 16 bytes.
pub fn derive_key(password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, SALT, ITERATIONS, &mut key);
    key
}

/// Seals `plaintext`: `"v10"` + AES-128-CBC/PKCS#7 with sixteen spaces for
/// an IV. Empty text seals to nothing at all, as Chromium has it.
pub fn encrypt(key: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    if plaintext.is_empty() {
        return Vec::new();
    }
    let sealed = cbc::Encryptor::<Aes128>::new(key.into(), &IV.into()).encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    [PREFIX, &sealed].concat()
}

/// Opens what [`encrypt`] sealed. None without the `v10` prefix or when the
/// padding doesn't check out (the wrong key, most likely).
pub fn decrypt(key: &[u8; 16], ciphertext: &[u8]) -> Option<Vec<u8>> {
    if ciphertext.is_empty() {
        return Some(Vec::new());
    }
    let sealed = ciphertext.strip_prefix(PREFIX)?;
    cbc::Decryptor::<Aes128>::new(key.into(), &IV.into())
        .decrypt_padded_vec_mut::<Pkcs7>(sealed)
        .ok()
}

/// The Keychain's generic passwords, by service and account. Errors are
/// `OSStatus` codes.
pub trait PasswordStore: Send + Sync {
    fn find(&self, service: &str, account: &str) -> Result<Vec<u8>, i32>;
    /// Adds the item, or updates its password if it's there.
    fn add(&self, service: &str, account: &str, password: &[u8]) -> Result<(), i32>;
    fn delete(&self, service: &str, account: &str) -> Result<(), i32>;
}

/// The Keychain password Chromium uses for `app`, as `GetPasswordImpl` finds
/// it: the item under `<app> Key`; else one under the old account `<app>`,
/// moved to `<app> Key`; else a new random one. None when the Keychain
/// fails (it's locked, or the user denied access) or can't take a new item.
pub fn keychain_password(store: &dyn PasswordStore, app: &str) -> Option<Vec<u8>> {
    let service = format!("{app} Safe Storage");
    let account = format!("{app} Key");
    let not_found = match store.find(&service, &account) {
        Ok(password) => return Some(password),
        Err(status) => status == ERR_SEC_ITEM_NOT_FOUND,
    };
    if !not_found {
        return None;
    }
    match store.find(&service, app) {
        Ok(password) => {
            // Copied to the suffixed account; the old one goes only once it's there.
            if store.add(&service, &account, &password).is_ok() {
                let _ = store.delete(&service, app);
            }
            return Some(password);
        }
        Err(status) if status != ERR_SEC_ITEM_NOT_FOUND => return None,
        Err(_) => {}
    }
    // 128 bits of randomness, base64: 24 characters.
    let password = encode_base64(&random_bytes::<16>()?).into_bytes();
    store.add(&service, &account, &password).ok()?;
    Some(password)
}

/// The macOS scheme: the key comes from the Keychain the first time it's
/// needed, and that answer, a failure included, holds until the app quits.
pub struct KeychainScheme<S> {
    store: S,
    app: String,
    key: OnceLock<Option<[u8; 16]>>,
}

impl<S: PasswordStore> KeychainScheme<S> {
    pub fn new(store: S, app: &str) -> Self {
        Self {
            store,
            app: app.to_owned(),
            key: OnceLock::new(),
        }
    }

    fn key(&self) -> Option<&[u8; 16]> {
        self.key
            .get_or_init(|| {
                let password = keychain_password(&self.store, &self.app)?;
                (!password.is_empty()).then(|| derive_key(&password))
            })
            .as_ref()
    }
}

impl<S: PasswordStore> Scheme for KeychainScheme<S> {
    fn available(&self) -> bool {
        self.key().is_some()
    }

    fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
        Some(encrypt(self.key()?, plaintext))
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        decrypt(self.key()?, ciphertext)
    }
}

/// A key given outright, for tests.
pub(super) struct FixedKey(pub(super) [u8; 16]);

impl Scheme for FixedKey {
    fn available(&self) -> bool {
        true
    }
    fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
        Some(encrypt(&self.0, plaintext))
    }
    fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        decrypt(&self.0, ciphertext)
    }
}

/// The login keychain, through the `SecItem` API as Chromium's `KeychainV2`
/// uses it. A signed Nodus with 0.9.4's designated requirement reads the item
/// 0.9.4 made without a prompt; an unsigned build makes macOS ask.
#[cfg(target_os = "macos")]
pub struct SystemKeychain;

#[cfg(target_os = "macos")]
impl PasswordStore for SystemKeychain {
    fn find(&self, service: &str, account: &str) -> Result<Vec<u8>, i32> {
        security_framework::passwords::get_generic_password(service, account)
            .map_err(security_framework::base::Error::code)
    }

    fn add(&self, service: &str, account: &str, password: &[u8]) -> Result<(), i32> {
        security_framework::passwords::set_generic_password(service, account, password)
            .map_err(security_framework::base::Error::code)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), i32> {
        security_framework::passwords::delete_generic_password(service, account)
            .map_err(security_framework::base::Error::code)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    /// A Keychain in memory, recording what's asked of it.
    #[derive(Default)]
    struct FakeKeychain {
        items: Mutex<BTreeMap<(String, String), Vec<u8>>>,
        calls: Mutex<Vec<String>>,
        /// Every lookup fails with this, as a locked or denied Keychain does.
        find_error: Option<i32>,
        add_error: Option<i32>,
    }

    impl FakeKeychain {
        fn with(items: &[(&str, &str, &[u8])]) -> Self {
            let fake = Self::default();
            for (service, account, password) in items {
                fake.items
                    .lock()
                    .unwrap()
                    .insert(((*service).into(), (*account).into()), password.to_vec());
            }
            fake
        }
        fn item(&self, service: &str, account: &str) -> Option<Vec<u8>> {
            self.items
                .lock()
                .unwrap()
                .get(&(service.into(), account.into()))
                .cloned()
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl PasswordStore for &FakeKeychain {
        fn find(&self, service: &str, account: &str) -> Result<Vec<u8>, i32> {
            self.calls.lock().unwrap().push(format!("find {service} / {account}"));
            if let Some(status) = self.find_error {
                return Err(status);
            }
            self.item(service, account).ok_or(ERR_SEC_ITEM_NOT_FOUND)
        }
        fn add(&self, service: &str, account: &str, password: &[u8]) -> Result<(), i32> {
            self.calls.lock().unwrap().push(format!("add {service} / {account}"));
            if let Some(status) = self.add_error {
                return Err(status);
            }
            self.items
                .lock()
                .unwrap()
                .insert((service.into(), account.into()), password.to_vec());
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), i32> {
            self.calls.lock().unwrap().push(format!("delete {service} / {account}"));
            self.items.lock().unwrap().remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    #[test]
    fn derives_chromiums_key() {
        // Python: hashlib.pbkdf2_hmac('sha1', password, b'saltysalt', 1003, 16)
        assert_eq!(
            derive_key(b"c3ludGhldGljLXRlc3Qta2V5"),
            [
                0xc0, 0x7d, 0xa3, 0xe8, 0xb9, 0xdd, 0x30, 0x4e, 0x14, 0x1b, 0xfd, 0x0b, 0x99, 0x28, 0x7f, 0x00
            ]
        );
        assert_eq!(
            derive_key(b"peanuts"),
            [
                0xd9, 0xa0, 0x9d, 0x49, 0x9b, 0x4e, 0x1b, 0x74, 0x61, 0xf2, 0x8e, 0x67, 0x97, 0x2c, 0x6d, 0xbd
            ]
        );
    }

    #[test]
    fn seals_as_chromium_does() {
        let key = derive_key(b"c3ludGhldGljLXRlc3Qta2V5");
        // openssl enc -aes-128-cbc -K <key> -iv 2020…20
        let sealed = encrypt(&key, b"xai-synthetic-0123456789abcdef");
        assert_eq!(&sealed[..3], b"v10");
        assert_eq!(
            sealed[3..],
            [
                0x90, 0xa1, 0x84, 0xc9, 0x43, 0xed, 0x67, 0x20, 0x57, 0x0c, 0x40, 0x2f, 0x22, 0xb0, 0xe4, 0x8e, 0xf4,
                0x87, 0x79, 0xa7, 0xb0, 0x4b, 0x1b, 0x02, 0x69, 0x1a, 0xdc, 0x49, 0x97, 0x9e, 0x14, 0x40
            ]
        );
        assert_eq!(decrypt(&key, &sealed).unwrap(), b"xai-synthetic-0123456789abcdef");
        assert_eq!(encrypt(&key, b""), b"");
        assert_eq!(decrypt(&key, b"").unwrap(), b"");
        assert_eq!(decrypt(&key, b"v11abc"), None);
        assert_eq!(decrypt(&key, b"v10abc"), None);
    }

    #[test]
    fn uses_the_password_the_keychain_has() {
        let keychain = FakeKeychain::with(&[("Nodus Safe Storage", "Nodus Key", b"existing")]);
        assert_eq!(
            keychain_password(&&keychain, APP_NAME).as_deref(),
            Some(&b"existing"[..])
        );
        assert_eq!(keychain.calls(), ["find Nodus Safe Storage / Nodus Key"]);
    }

    #[test]
    fn moves_a_password_from_the_old_account() {
        let keychain = FakeKeychain::with(&[("Nodus Safe Storage", "Nodus", b"legacy")]);
        assert_eq!(keychain_password(&&keychain, APP_NAME).as_deref(), Some(&b"legacy"[..]));
        assert_eq!(
            keychain.calls(),
            [
                "find Nodus Safe Storage / Nodus Key",
                "find Nodus Safe Storage / Nodus",
                "add Nodus Safe Storage / Nodus Key",
                "delete Nodus Safe Storage / Nodus"
            ]
        );
        assert_eq!(
            keychain.item("Nodus Safe Storage", "Nodus Key").as_deref(),
            Some(&b"legacy"[..])
        );
        assert_eq!(keychain.item("Nodus Safe Storage", "Nodus"), None);
    }

    #[test]
    fn makes_a_random_password_when_there_is_none() {
        let keychain = FakeKeychain::default();
        let password = keychain_password(&&keychain, APP_NAME).unwrap();
        assert_eq!(password.len(), 24);
        assert!(password.ends_with(b"=="));
        assert_eq!(
            super::super::decode_base64(std::str::from_utf8(&password).unwrap()).len(),
            16
        );
        assert_eq!(keychain.item("Nodus Safe Storage", "Nodus Key"), Some(password.clone()));
        let again = keychain_password(&&keychain, APP_NAME).unwrap();
        assert_eq!(again, password);
        let other = keychain_password(&&FakeKeychain::default(), APP_NAME).unwrap();
        assert_ne!(other, password);
    }

    #[test]
    fn gives_up_when_the_keychain_fails() {
        // errSecAuthFailed: the user denied access, or the keychain is locked.
        let denied = FakeKeychain {
            find_error: Some(-25293),
            ..FakeKeychain::default()
        };
        assert_eq!(keychain_password(&&denied, APP_NAME), None);
        assert_eq!(denied.calls(), ["find Nodus Safe Storage / Nodus Key"]);
        let full = FakeKeychain {
            add_error: Some(-61),
            ..FakeKeychain::default()
        };
        assert_eq!(keychain_password(&&full, APP_NAME), None);
    }

    #[test]
    fn asks_the_keychain_once_per_launch() {
        let denied = FakeKeychain {
            find_error: Some(-25293),
            ..FakeKeychain::default()
        };
        let scheme = KeychainScheme::new(&denied, APP_NAME);
        assert!(!scheme.available());
        assert!(!scheme.available());
        assert_eq!(scheme.encrypt(b"x"), None);
        assert_eq!(denied.calls().len(), 1);

        let keychain = FakeKeychain::with(&[("Nodus Safe Storage", "Nodus Key", b"c3ludGhldGljLXRlc3Qta2V5")]);
        let scheme = KeychainScheme::new(&keychain, APP_NAME);
        let sealed = scheme.encrypt(b"xai-synthetic-0123456789abcdef").unwrap();
        assert_eq!(
            encode_base64(&sealed),
            "djEwkKGEyUPtZyBXDEAvIrDkjvSHeaewSxsCaRrcSZeeFEA="
        );
        assert_eq!(scheme.decrypt(&sealed).unwrap(), b"xai-synthetic-0123456789abcdef");
        assert_eq!(keychain.calls().len(), 1);
    }
}
