//! The xAI key (VOICE-001…VOICE-008): saved from Settings, sealed with
//! Electron's `safeStorage` (nodus-platform's `SafeStorage`, so a key 0.9.4
//! saved still opens), or the `XAI_API_KEY` in the user's login shell. A
//! saved one wins. The key never leaves the core: the window only learns
//! where it comes from (VOICE-007).

use std::sync::Arc;

use nodus_domain::VoiceKeySource;
use nodus_domain::js::trim;
use nodus_platform::Env;
use nodus_platform::secrets::{CANNOT_SAVE_KEY, SafeStorage, SafeStorageError, decode_base64, encode_base64};
use nodus_store::Store;
use serde_json::Value;

/// Why a key couldn't be saved; the text is what Settings shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// VOICE-003: no Keychain (macOS) or encrypted storage (Windows).
    #[error("{}", CANNOT_SAVE_KEY)]
    Unavailable,
    /// `safeStorage.encryptString` failed after saying it could.
    #[error(transparent)]
    Seal(#[from] SafeStorageError),
}

/// The shell's key: `XAI_API_KEY` from the login-shell environment, trimmed,
/// if that leaves anything.
pub fn shell_key(env: &Env) -> Option<String> {
    trimmed(env.var("XAI_API_KEY"))
}

fn trimmed(key: Option<&str>) -> Option<String> {
    key.map(trim).filter(|k| !k.is_empty()).map(str::to_owned)
}

/// VOICE-005: `voice:set-key`'s argument; anything but a string is `null`.
pub fn key_arg(value: &Value) -> Option<&str> {
    value.as_str()
}

/// The saved key and the store it's sealed in.
pub struct VoiceKeys {
    store: Arc<Store>,
    storage: Arc<SafeStorage>,
}

impl std::fmt::Debug for VoiceKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceKeys")
    }
}

impl VoiceKeys {
    pub fn new(store: Arc<Store>, storage: Arc<SafeStorage>) -> Self {
        Self { store, storage }
    }

    /// VOICE-001: `settings` while a key is saved (even one that won't open),
    /// else `env` while the shell has one, else none. `shell_key` is
    /// [`shell_key`]'s answer.
    pub fn key_source(&self, shell_key: Option<&str>) -> Option<VoiceKeySource> {
        // VOICE-008: the store keeps only a non-empty saved key.
        if self
            .store
            .read(|s| s.voice_key.as_deref().is_some_and(|k| !k.is_empty()))
        {
            return Some(VoiceKeySource::Settings);
        }
        trimmed(shell_key).map(|_| VoiceKeySource::Env)
    }

    /// VOICE-003, VOICE-004: saves the key, trimmed, sealed and in base64; an
    /// empty or missing one forgets the saved key. Returns where the key
    /// comes from now.
    pub fn set_key(&self, key: Option<&str>, shell_key: Option<&str>) -> Result<Option<VoiceKeySource>, KeyError> {
        let sealed = match trimmed(key) {
            Some(key) => {
                if !self.storage.is_encryption_available() {
                    return Err(KeyError::Unavailable);
                }
                Some(encode_base64(&self.storage.encrypt_string(&key)?))
            }
            None => None,
        };
        self.store.update(|state| state.voice_key = sealed);
        Ok(self.key_source(shell_key))
    }

    /// VOICE-006: the saved key, opened; none when there isn't one, it won't
    /// open (logged as 0.9.4 logs it) or it opens to nothing.
    pub fn saved_key(&self) -> Option<String> {
        let sealed = self.store.read(|s| s.voice_key.clone()).filter(|k| !k.is_empty())?;
        match self.storage.decrypt_string(&decode_base64(&sealed)) {
            Ok(key) => Some(key).filter(|k| !k.is_empty()),
            Err(err) => {
                tracing::error!("[voice] could not decrypt the saved key {err}");
                None
            }
        }
    }

    /// The key a transcription uses: the saved one, else the shell's.
    pub fn key(&self, shell_key: Option<&str>) -> Option<String> {
        self.saved_key().or_else(|| trimmed(shell_key))
    }
}
