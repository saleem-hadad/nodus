//! Views: small live pages on the board that Claude builds from what the user
//! describes. This owns their files in the data folder, the commands the user
//! allowed and their schedules, their brains (the Claude sessions that build
//! and refresh them), and the page a view's frame loads with `window.nodus`.
//!
//! Ports `src/main/views.ts`, `src/main/viewPage.ts` and the `views:*`
//! handlers in `src/main/ipc.ts`.
//!
//! A view's page and what it holds at run time (its data, its saved state,
//! its commands' last output, the errors it hit) live in
//! `<userData>/views/<id>/`, beside state.json and never in the repository.
//! The page is served on the `nodus-view` scheme ([`Views::serve_page`]) into
//! a sandboxed frame, and everything it asks the board for comes through
//! [`Views::call`], for that view alone.
//!
//! Fresh data comes from commands the user allowed on the card, which run in
//! the project folder, and from the view's brain: a Claude session of its own,
//! hidden from the board like an action run, that builds the page and updates
//! it when asked or on a schedule. A brain stops after a few idle minutes and
//! is resumed when it's next needed.
//!
//! # Wiring
//!
//! nodus-core builds one [`Views`] ([`ViewsConfig`]) and calls
//! [`Views::start_schedules`] at startup and [`Views::flush`] on quit. It
//! passes each status change to [`Views::brain_status`] (after the board's
//! `on_done`), each terminal write to [`Views::brain_input`], and a removed
//! project's board to [`Views::remove_project_views`]; `session:open` and
//! `session:send` take a brain's arguments from [`brain_args`]. The window's
//! frames are served by [`Views::serve_page`] and report through
//! [`Views::report`]; the `views:*` commands map one to one onto the methods
//! named after them.

mod brains;
mod check;
mod command;
mod page;
mod runtime;
mod serve;
mod tools;
mod views;

use std::fmt;

use nodus_board::BoxFuture;
use nodus_sessions::launch::SpawnOptions;
use serde_json::Value;

pub use brains::{AskOrigin, brain_args, brain_prompt};
pub use check::{
    clamp_every, clip, is_allowed, is_command_waiting, is_denied, is_view_id, normalize_commands, normalize_refresh,
};
pub use command::COMMAND_TIMEOUT;
pub use page::{BRIDGE, EMPTY_PAGE, THEME, VIEW_CSP, VIEW_GUIDE, VIEW_SCHEME, ViewBoot, build_view_page, script_json};
pub use runtime::{PageError, Runtime};
pub use serve::{PageResponse, view_id_from_url};
pub use views::{CommandRun, Views, ViewsConfig};

/// Why something asked of a view failed, in the words the page, the card or
/// Claude is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewError(String);

impl ViewError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ViewError {}

/// How views start and type into their brains' processes. nodus-core
/// implements it with `nodus_sessions::launch` and the session host.
pub trait BrainLauncher: Send + Sync {
    /// `pty.spawnSession`: starts a brain's `claude` process. The error is the
    /// host's, as it is.
    fn spawn(&self, opts: SpawnOptions) -> BoxFuture<'_, Result<(), String>>;
    /// `pty.write`: sends text to a live session's terminal.
    fn write(&self, session_id: &str, data: &str);
}

/// What a page may ask for that other domains have: the repository's git
/// status (05) and the user's browser (01).
pub trait ViewHost: Send + Sync {
    /// `gitStatus(repoPath)`: the status as the window gets it, or `null`.
    fn git_status(&self, repo_path: &str) -> BoxFuture<'_, Value>;
    /// `shell.openExternal(url)`, for an http or https link: the default
    /// browser, through `nodus_platform::shell`.
    fn open_url(&self, url: &str) {
        nodus_platform::shell::open_external(url);
    }
}
