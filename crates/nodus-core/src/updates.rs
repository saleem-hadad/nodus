//! `window.api.updates`: the app updating itself from GitHub releases
//! (`updates:*` in `src/main/ipc.ts`, spec 06). Every change of status
//! arrives as `Event::UpdateStatus`.

use nodus_domain::UpdateStatus;
pub use nodus_updater::{RestartOutcome, StopSessionsQuestion};

use crate::Nodus;

/// `window.api.updates`.
pub struct UpdatesApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl UpdatesApi<'_> {
    /// `updates:status` (UPD-076).
    pub async fn status(&self) -> UpdateStatus {
        self.nodus.inner.updater.status()
    }

    /// `updates:check` (UPD-077): looks for a newer version now, as the user
    /// asked, and starts downloading it; done once the lookup is.
    pub async fn check(&self) {
        self.nodus.inner.updater.check(true).await;
    }

    /// `updates:install` (UPD-078): quits into the downloaded version. On
    /// Windows with sessions running it gives [`RestartOutcome::Ask`]
    /// instead: the window shows that question (`Stop and Update`, `Not
    /// Now`) and answers with [`UpdatesApi::answer_stop_sessions`].
    pub async fn install(&self) -> RestartOutcome {
        self.nodus.inner.updater.restart_to_update().await
    }

    /// The answer to [`RestartOutcome::Ask`]: `Stop and Update` (true) or
    /// `Not Now` (false, and closing the question).
    pub async fn answer_stop_sessions(&self, stop: bool) -> RestartOutcome {
        self.nodus.inner.updater.answer_stop_sessions(stop).await
    }
}
