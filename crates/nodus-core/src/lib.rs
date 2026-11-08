//! Every domain composed into one service, with no UI: async commands that
//! mirror `window.api` one to one, and a stream of the events the Electron
//! app sends on its `ev:*` channels. The UI talks to the rest of the app only
//! through this, and a headless Nodus can serve the same API.
//!
//! Ports `src/main/ipc.ts`, `src/shared/api.ts` and `src/preload/index.ts`,
//! and when to notify from `src/main/notify.ts`.
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
//! window.api.takeOpenRequest()           nodus.app().take_open_request().await
//! window.api.on.menu(cb)                 nodus.subscribe() → Event::Menu(..)
//! ```
//!
//! A command that fails gives a [`CommandError`] with 0.9.4's message. Events
//! go to every [`Nodus::subscribe`]r; domains emit them through
//! [`Nodus::sink`]. What only a window can do (the folder picker) is asked of
//! the [`Dialogs`] the app was started with.

mod app;
mod error;
mod groups;
mod nodus;
mod os;
mod projects;
mod realpath;
mod settings;

pub use app::AppApi;
pub use error::{CommandError, CommandResult};
pub use groups::GroupsApi;
pub use nodus::{Dialogs, EnvSource, Nodus, NodusConfig, PickFolder, SettingsHook};
pub use projects::ProjectsApi;
pub use settings::SettingsApi;

/// The types the API takes and gives, for callers that don't depend on
/// `nodus-domain` themselves.
pub use nodus_domain as domain;

#[cfg(test)]
mod tests {
    use super::*;

    /// A service on an empty data folder in a temporary folder, with the
    /// app's own environment and no dialogs.
    pub(crate) fn nodus() -> (tempfile::TempDir, Nodus) {
        let dir = tempfile::tempdir().unwrap();
        let nodus = Nodus::start(NodusConfig {
            user_data: dir.path().join("userData"),
            env: EnvSource::Given(nodus_platform::Env::from_process()),
            dialogs: None,
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
