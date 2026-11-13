//! `window.api.updates`: the app updating itself from GitHub releases
//! (`updates:*` in `src/main/ipc.ts`, spec 06). Every change of status
//! arrives as `Event::UpdateStatus`.

use nodus_domain::UpdateStatus;
use nodus_updater::is_newer;
pub use nodus_updater::{RestartOutcome, StopSessionsQuestion};

use crate::Nodus;

/// The `localStorage` key holding the newest version whose What's new was
/// closed (UPD-097).
pub const WHATS_NEW_KEY: &str = "nodus.whatsNewSeen";

/// What's new that the user hasn't seen (UPD-096): the slides of the
/// versions after `seen`, up to this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unseen {
    /// None after an update from a version without What's new: every slide
    /// this version has is new.
    pub seen: Option<String>,
}

impl Unseen {
    /// Whether the slides of `version` are new to the user.
    pub fn includes(&self, version: &str) -> bool {
        !is_newer(version, crate::VERSION) && self.seen.as_deref().is_none_or(|seen| is_newer(version, seen))
    }
}

/// `window.api.updates`.
pub struct UpdatesApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl UpdatesApi<'_> {
    /// `updates:status` (UPD-076).
    pub async fn status(&self) -> UpdateStatus {
        self.nodus.on_core(async { self.nodus.inner.updater.status() }).await
    }

    /// `updates:check` (UPD-077): looks for a newer version now, as the user
    /// asked, and starts downloading it; done once the lookup is.
    pub async fn check(&self) {
        self.nodus
            .on_core(async {
                self.nodus.inner.updater.check(true).await;
            })
            .await;
    }

    /// `updates:install` (UPD-078): quits into the downloaded version. On
    /// Windows with sessions running it gives [`RestartOutcome::Ask`]
    /// instead: the window shows that question (`Stop and Update`, `Not
    /// Now`) and answers with [`UpdatesApi::answer_stop_sessions`].
    pub async fn install(&self) -> RestartOutcome {
        self.nodus
            .on_core(async { self.nodus.inner.updater.restart_to_update().await })
            .await
    }

    /// The answer to [`RestartOutcome::Ask`]: `Stop and Update` (true) or
    /// `Not Now` (false, and closing the question).
    pub async fn answer_stop_sessions(&self, stop: bool) -> RestartOutcome {
        self.nodus
            .on_core(async { self.nodus.inner.updater.answer_stop_sessions(stop).await })
            .await
    }

    /// UPD-096: whether this launch shows What's new, and from which
    /// version on. None in a development build, when this version (or a
    /// newer one) was seen, and on a fresh install, whose version is stored
    /// as seen: it has no projects yet.
    pub fn whats_new(&self) -> Option<Unseen> {
        let version = crate::VERSION;
        if !self.nodus.inner.updater.packaged() {
            return None;
        }
        match self.nodus.local_storage().get(WHATS_NEW_KEY) {
            Some(seen) if !is_newer(version, &seen) => None,
            Some(seen) => Some(Unseen { seen: Some(seen) }),
            None if self.nodus.inner.store.read(|state| state.projects.is_empty()) => {
                self.whats_new_seen();
                None
            }
            None => Some(Unseen { seen: None }),
        }
    }

    /// UPD-097: What's new was closed. It shows again only after an update,
    /// and never for this version or one before it.
    pub fn whats_new_seen(&self) {
        let version = crate::VERSION;
        let storage = self.nodus.local_storage();
        if storage.get(WHATS_NEW_KEY).is_none_or(|seen| is_newer(version, &seen)) {
            storage.set(WHATS_NEW_KEY, Some(version));
        }
    }
}
