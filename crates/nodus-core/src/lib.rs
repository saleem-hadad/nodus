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
//! # Server projects
//!
//! A project linked from a Nodus server (`Project::server`) works like a
//! local one through the same calls: every method that takes a project or a
//! session sends a server project's to its server and gives back the same
//! types, and the server's events arrive among the core's own (SRV-160).
//! [`Nodus::servers`] adds, links and watches servers.
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
mod feedback;
mod git;
mod groups;
mod hosting;
mod local_storage;
mod menu;
mod nodus;
mod notifications;
mod pool;
mod projects;
mod prompts;
mod realpath;
mod routing;
mod servers;
mod sessions;
mod settings;
mod shell;
mod updates;
mod views;
mod voice;
mod wiring;
pub mod zoom;

pub use actions::ActionsApi;
pub use app::AppApi;
pub use board::BoardApi;
pub use board::images_dir as board_images_dir;
pub use capture::{CaptureApi, CaptureSystem, WindowSize};
pub use error::{CommandError, CommandResult};
pub use feedback::{AreaShot, FeedbackApi};
pub use git::GitApi;
pub use groups::GroupsApi;
pub use hosting::{BoxFuture as HostingFuture, GitIdentity, Hosting, PREVIEW_PORTS, TokenScope, branch_name};
pub use local_storage::LocalStorageApi;
pub use menu::MenuApi;
pub use nodus::{
    By, Dialogs, EnvSource, HostMode, Nodus, NodusConfig, PickFolder, QuitAtOnce, RunnerSource, SettingsHook,
};
/// What the windowed app gives capture, and what it answers.
pub use nodus_capture as capture_service;
/// Where the xAI key is sealed; tests give a synthetic one.
pub use nodus_platform::secrets::SafeStorage;
/// A repository a server's GitHub App reaches (SRV-120).
pub use nodus_remote::GithubRepo;
/// How often an app may say where it is on a server board (SRV-180).
pub use nodus_remote::PRESENCE_EVERY;
/// What a mic implements and runs, for the window's mics (`VoiceApi::backend`).
pub use nodus_voice::controller::{VoiceBackend, VoiceController};
pub use notifications::Notifications;
pub use pool::{AccountSession, CONTINUE, PoolAccount, TOKEN_VAR as CLAUDE_TOKEN_VAR};
pub use projects::ProjectsApi;
pub use servers::{ServersApi, SetupSession, SignIn};
/// What a Nodus server says of itself and its people, as `nodus.servers()`
/// hands it out (SRV-040…063, SRV-210): setting it up, signing in, its
/// members, devices, audit and GitHub App.
pub mod hosted {
    pub use crate::servers::{SETUP_EXPIRED, install_url};
    pub use nodus_remote::wire::{
        ACCOUNT_FAILED, AccountState, AuditDevice, AuditLine, AuditMember, ClaudeAccount, DEVICE_SIGNED_IN, Device,
        GithubApp, Hello, Installation, Member, Nonce, ServerInfo as ServerDetails, SetupStatus, SetupStep,
        SignInStarted, TAILSCALE_TAG, TailscaleIdentity, TailscaleStatus, UsageWindow,
    };
}
pub use sessions::{Delivery, SessionsApi};
pub use settings::SettingsApi;
pub use shell::{AppShell, BoxFuture, Headless, ReplaceChoice};
pub use updates::{RestartOutcome, StopSessionsQuestion, UpdatesApi};
pub use views::ViewsApi;
pub use voice::VoiceApi;
pub use zoom::ZoomApi;

/// The board's stop rules, for the window's Stop (SESS-149…152).
pub use nodus_board::{StopConfirmation, stop_confirmation, stoppable};
/// The types the API takes and gives, for callers that don't depend on
/// `nodus-domain` themselves.
pub use nodus_domain as domain;
/// How a session host is started, for a caller that starts its own, and
/// the runner that runs every process in it (SRV-005).
pub use nodus_sessions::client::{HostLauncher, HostRunner, ProcessLauncher};
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
