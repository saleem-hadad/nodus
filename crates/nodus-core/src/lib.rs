//! Every domain composed into one service, with no UI: async commands that
//! mirror `window.api` one to one, and a stream of the events the Electron
//! app sends on its `ev:*` channels. The UI talks to the rest of the app only
//! through this, and a headless Nodus can serve the same API.
//!
//! Ports `src/main/ipc.ts`, `src/shared/api.ts`, `src/preload/index.ts`, the
//! startup and quit order of `src/main/index.ts`, and the glue between the
//! main process's modules.
//!
//! # Shape
//!
//! [`Nodus`] is the service. Each area of `window.api` is an accessor that
//! borrows it, with one async method per command, named as in `api.ts`:
//!
//! ```text
//! window.api.settings.update(patch)      nodus.settings().update(patch).await
//! window.api.projects.add(repoPath?)     nodus.projects().add(repo_path).await
//! window.api.groups.create(name)         nodus.groups().create(name).await
//! window.api.board.load(projectId)       nodus.board().load(project_id).await
//! window.api.sessions.write(id, data)    nodus.sessions().write(id, data)
//! window.api.views.call(p, v, m, args)   nodus.views().call(p, v, m, &args).await
//! window.api.updates.check()             nodus.updates().check().await
//! window.api.takeOpenRequest()           nodus.app().take_open_request().await
//! window.api.on.menu(cb)                 nodus.subscribe() → Event::Menu(..)
//! ```
//!
//! A command that fails gives a [`CommandError`] with 0.9.4's message. Events
//! go to every [`Nodus::subscribe`]r; domains emit them through
//! [`Nodus::sink`]. What only a window can do (the folder picker, the
//! questions at launch and before an update, bringing the window up,
//! quitting) is asked of the [`Dialogs`] and the [`AppShell`] the app was
//! started with.
//!
//! # Lifecycle
//!
//! As `src/main/index.ts` runs it (PLAT-010, PLAT-032):
//!
//! 1. [`Nodus::start`]: state.json, settings applied, the login shell read in
//!    the background, every domain set up, the updater started.
//! 2. [`Nodus::launch`]`.await`: the sessions a running session host has are
//!    adopted, then notifications armed. The window opens after this.
//! 3. [`Nodus::shutdown`] on every normal quit; [`MenuApi::quit_and_stop_sessions`]
//!    stops the sessions first.

mod actions;
mod app;
mod board;
mod capture;
#[cfg(test)]
mod core_tests;
mod error;
mod git;
mod groups;
mod local_storage;
mod menu;
mod nodus;
mod notifications;
mod projects;
mod realpath;
mod sessions;
mod settings;
mod shell;
mod updates;
mod views;
mod voice;
mod wiring;

pub use actions::ActionsApi;
pub use app::AppApi;
pub use board::BoardApi;
pub use capture::{CaptureApi, CaptureSystem, WindowSize};
pub use error::{CommandError, CommandResult};
pub use git::GitApi;
pub use groups::GroupsApi;
pub use local_storage::LocalStorageApi;
pub use menu::MenuApi;
pub use nodus::{Dialogs, EnvSource, Nodus, NodusConfig, PickFolder, QuitAtOnce, SettingsHook};
/// What the windowed app gives capture, and what it answers.
pub use nodus_capture as capture_service;
/// Where the xAI key is sealed; tests give a synthetic one.
pub use nodus_platform::secrets::SafeStorage;
/// What a mic implements and runs, for the window's mics (`VoiceApi::backend`).
pub use nodus_voice::controller::{VoiceBackend, VoiceController};
pub use notifications::Notifications;
pub use projects::ProjectsApi;
pub use sessions::SessionsApi;
pub use settings::SettingsApi;
pub use shell::{AppShell, BoxFuture, Headless, ReplaceChoice};
pub use updates::{RestartOutcome, StopSessionsQuestion, UpdatesApi};
pub use views::ViewsApi;
pub use voice::VoiceApi;

/// The board's stop rules, for the window's Stop (SESS-149…152).
pub use nodus_board::{StopConfirmation, stop_confirmation, stoppable};
/// The types the API takes and gives, for callers that don't depend on
/// `nodus-domain` themselves.
pub use nodus_domain as domain;
/// How a session host is started, for a caller that starts its own.
pub use nodus_sessions::client::{HostLauncher, ProcessLauncher};
/// What a custom notifier implements, and the ids its clicks come back with.
pub use nodus_sessions::notify::{Notification, NotificationId, Notifier};
/// A view's page, as the frame's custom protocol answers it.
pub use nodus_views::PageResponse;

/// The app's version (`app.getVersion()`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    /// A service on an empty data folder in a temporary folder, with the
    /// app's own environment and no dialogs.
    pub(crate) fn nodus() -> (tempfile::TempDir, Nodus) {
        let dir = tempfile::tempdir().unwrap();
        let nodus = Nodus::start(NodusConfig {
            env: EnvSource::Given(nodus_platform::Env::from_process()),
            home: dir.path().join("home"),
            ..NodusConfig::new(dir.path().join("userData"), tokio::runtime::Handle::current())
        })
        .unwrap();
        (dir, nodus)
    }

    #[tokio::test]
    async fn sends_menu_commands_to_every_subscriber_and_ignores_the_rest() {
        let (_dir, nodus) = nodus();
        let mut first = nodus.subscribe();
        let mut second = nodus.subscribe();
        nodus.app().menu_command("settings").await;
        nodus.app().menu_command("explode").await;
        nodus.emit(domain::Event::ProjectsChanged);
        for rx in [&mut first, &mut second] {
            assert_eq!(
                rx.recv().await.unwrap(),
                domain::Event::Menu(domain::MenuCommand::Settings)
            );
            assert_eq!(rx.recv().await.unwrap(), domain::Event::ProjectsChanged);
        }
    }

    #[tokio::test]
    async fn gives_the_open_request_once() {
        let (_dir, nodus) = nodus();
        let request = domain::OpenCardRequest {
            project_id: "p".into(),
            session_id: Some("s".into()),
        };
        nodus.set_open_request(Some(request.clone()));
        assert_eq!(nodus.app().take_open_request().await, Some(request));
        assert_eq!(nodus.app().take_open_request().await, None);
    }

    #[tokio::test]
    async fn says_which_platform_it_runs_on() {
        let (_dir, nodus) = nodus();
        let info = nodus.app().info().await;
        assert!(["darwin", "win32", "linux"].contains(&info.platform.as_str()));
    }
}
