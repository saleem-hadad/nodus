//! Voice prompts: recording from the microphone and turning speech into text
//! through xAI's API, with the key saved in Settings or taken from the user's
//! shell.
//!
//! Ports `src/main/voice.ts` and the recording in
//! `src/renderer/src/components/voice/`:
//!
//! - [`VoiceKeys`]: where the key comes from, saving and forgetting it
//!   (VOICE-001…VOICE-008), sealed with nodus-platform's `SafeStorage`;
//! - [`Voice::transcribe`]: the xAI request and its answer, with hint words
//!   from the project's git status (VOICE-011…VOICE-019).
//!
//! The microphone's permission and System Settings pane are nodus-platform's
//! `permissions::{microphone, open_microphone_settings}` (VOICE-009, VOICE-010).

mod capture;
pub mod encode;
mod key;
mod keyterms;
pub mod meter;
mod recorder;
pub mod resample;
pub mod stt;
pub mod webm;

pub use recorder::{Audio, OpenError, Recording, Sink, Source};

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nodus_domain::{Project, TranscribeResult, VoiceKeySource};
use nodus_git::Git;
use nodus_platform::Env;
use nodus_platform::secrets::SafeStorage;
use nodus_store::Store;
use serde_json::Value;

pub use key::{KeyError, VoiceKeys, key_arg, shell_key};
pub use keyterms::{MAX_KEYTERM_CHARS, MAX_KEYTERMS, MIN_KEYTERM_CHARS, keyterms, keyterms_from};

/// Voice's side of the core: the key, and transcribing recordings.
pub struct Voice {
    keys: VoiceKeys,
    client: reqwest::Client,
    url: String,
    timeout: Duration,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Voice").field("url", &self.url).finish_non_exhaustive()
    }
}

impl Voice {
    pub fn new(store: Arc<Store>, storage: Arc<SafeStorage>) -> Self {
        Self {
            keys: VoiceKeys::new(store, storage),
            client: reqwest::Client::new(),
            url: stt::STT_URL.to_owned(),
            timeout: stt::TIMEOUT,
        }
    }

    /// Sends recordings somewhere else, with another time limit: for tests.
    pub fn with_endpoint(mut self, url: impl Into<String>, timeout: Duration) -> Self {
        self.url = url.into();
        self.timeout = timeout;
        self
    }

    pub fn keys(&self) -> &VoiceKeys {
        &self.keys
    }

    /// `voice:key-source`, once the login-shell env is ready (VOICE-001, VOICE-002).
    pub fn key_source(&self, env: &Env) -> Option<VoiceKeySource> {
        self.keys.key_source(shell_key(env).as_deref())
    }

    /// `voice:set-key`, once the login-shell env is ready (VOICE-003…VOICE-005).
    pub fn set_key(&self, env: &Env, key: Option<&str>) -> Result<Option<VoiceKeySource>, KeyError> {
        self.keys.set_key(key, shell_key(env).as_deref())
    }

    /// `voice:transcribe` for a project, once the login-shell env is ready
    /// (VOICE-011…VOICE-019). `shell_key` is [`shell_key`]'s answer.
    pub async fn transcribe(
        &self,
        git: &Git,
        project: &Project,
        audio: Vec<u8>,
        mime_type: &str,
        shell_key: Option<&str>,
    ) -> TranscribeResult {
        // VOICE-011: no request without a key.
        let Some(key) = self.keys.key(shell_key) else {
            return stt::no_key();
        };
        let terms = keyterms(git, &project.name, Path::new(&project.repo_path)).await;
        stt::send(&self.client, &self.url, &key, audio, mime_type, &terms, self.timeout).await
    }
}

/// VOICE-019: `String(mimeType ?? '')`, for an argument that arrives as JSON.
pub fn mime_arg(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => nodus_domain::json::number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        // Array.prototype.join: null and undefined are empty.
        Value::Array(items) => items.iter().map(mime_arg).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_mime_type_argument_as_javascript_does() {
        assert_eq!(mime_arg(&json!(null)), "");
        assert_eq!(mime_arg(&json!("audio/webm;codecs=opus")), "audio/webm;codecs=opus");
        assert_eq!(mime_arg(&json!(5)), "5");
        assert_eq!(mime_arg(&json!(true)), "true");
        assert_eq!(mime_arg(&json!(["a", null, 1])), "a,,1");
        assert_eq!(mime_arg(&json!({})), "[object Object]");
    }
}
