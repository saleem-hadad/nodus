//! `window.api.voice`: the board's mic buttons, speech to text through xAI
//! (`voice:*` in `src/main/ipc.ts`). The key never leaves the core.

use std::sync::Arc;

use nodus_domain::{TranscribeResult, VoiceKeySource};
use nodus_git::Git;
use nodus_voice::controller::{BoxFuture, LiveRecording, VoiceBackend};
use nodus_voice::shell_key;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.voice`.
pub struct VoiceApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl VoiceApi<'_> {
    /// `voice:key-source` (VOICE-001, VOICE-002): where the key comes from,
    /// a saved one or the shell's `XAI_API_KEY`, once the login shell's
    /// environment is in.
    pub async fn key_source(&self) -> Option<VoiceKeySource> {
        self.nodus
            .on_core(async {
                let env = self.nodus.env().await;
                self.nodus.inner.voice.key_source(&env)
            })
            .await
    }

    /// `voice:set-key` (VOICE-003…VOICE-008): saves the key, encrypted as
    /// 0.9.4's safeStorage does; none forgets it. Fails when there's no
    /// Keychain (or encrypted storage on Windows) to keep it in.
    pub async fn set_key(&self, key: Option<&str>) -> CommandResult<Option<VoiceKeySource>> {
        self.nodus
            .on_core(async {
                let env = self.nodus.env().await;
                self.nodus.inner.voice.set_key(&env, key).on("voice:set-key")
            })
            .await
    }

    /// `voice:microphone` (VOICE-009): asks macOS for the microphone the
    /// first time; false once the user has said no. True off macOS.
    pub async fn microphone(&self) -> bool {
        self.nodus
            .on_core(async { nodus_platform::permissions::microphone().await })
            .await
    }

    /// `voice:microphone-settings` (VOICE-010): Privacy & Security ›
    /// Microphone in System Settings.
    pub async fn open_microphone_settings(&self) {
        self.nodus
            .on_core(async {
                nodus_platform::permissions::open_microphone_settings();
            })
            .await;
    }

    /// `voice:transcribe` (VOICE-011…VOICE-019): a recording to text, hinted
    /// with names from the project so they come back spelled right, once the
    /// login shell's environment is in. `Unknown project <id>`.
    pub async fn transcribe(
        &self,
        project_id: &str,
        audio: Vec<u8>,
        mime_type: &str,
    ) -> CommandResult<TranscribeResult> {
        self.nodus
            .on_core(async {
                const CHANNEL: &str = "voice:transcribe";
                let env = self.nodus.env().await;
                let project = self
                    .nodus
                    .inner
                    .store
                    .read(|state| state.project(project_id).cloned())
                    .on(CHANNEL)?;
                let key = shell_key(&env);
                Ok(self
                    .nodus
                    .inner
                    .voice
                    .transcribe(&Git::from_env(&env), &project, audio, mime_type, key.as_deref())
                    .await)
            })
            .await
    }

    /// What a window's mics run on (`nodus_voice::controller::VoiceBackend`):
    /// these commands for the board `project` names now, the real
    /// microphone, and `open_settings` for a missing key.
    pub fn backend(
        &self,
        project: Arc<dyn Fn() -> Option<String> + Send + Sync>,
        open_settings: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<dyn VoiceBackend> {
        Arc::new(Mics {
            nodus: self.nodus.clone(),
            project,
            open_settings,
        })
    }
}

/// A window's mics, on the core.
struct Mics {
    nodus: Nodus,
    project: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    open_settings: Arc<dyn Fn() + Send + Sync>,
}

impl VoiceBackend for Mics {
    fn key_source(&self) -> BoxFuture<'_, Result<Option<VoiceKeySource>, String>> {
        Box::pin(async move { Ok(self.nodus.voice().key_source().await) })
    }

    fn microphone(&self) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(async move { Ok(self.nodus.voice().microphone().await) })
    }

    fn start_recording(&self) -> BoxFuture<'_, Result<Box<dyn LiveRecording>, String>> {
        Box::pin(nodus_voice::controller::open_microphone())
    }

    fn transcribe(&self, audio: Vec<u8>, mime_type: String) -> BoxFuture<'_, Result<TranscribeResult, String>> {
        Box::pin(async move {
            let project = (self.project)().unwrap_or_default();
            self.nodus
                .voice()
                .transcribe(&project, audio, &mime_type)
                .await
                .map_err(|err| err.ipc_message())
        })
    }

    fn open_settings(&self) {
        (self.open_settings)();
    }
}
