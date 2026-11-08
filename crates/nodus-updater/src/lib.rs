//! Keeps the app up to date from GitHub releases: finding the latest release,
//! downloading the build for this machine and checking it against GitHub's
//! digest, then swapping it in for the running app once it quits (macOS), or
//! running its installer (Windows). Ports `src/main/updater.ts` (spec 06,
//! UPD-001…UPD-075).
//!
//! nodus-core builds an [`Updater`] with its [`App`] side and a
//! [`RealSystem`], calls [`Updater::start`] at launch, and maps
//! `updates:status` to [`Updater::status`], `updates:check` to
//! [`Updater::check`]`(true)`, `updates:install` to
//! [`Updater::restart_to_update`] (showing the [`StopSessionsQuestion`] it
//! may return, then [`Updater::answer_stop_sessions`]), a `settings:update`
//! with a truthy `autoUpdate` to `check(false)` (UPD-013), and its quit path
//! to [`Updater::on_will_quit`]. Every status change arrives as
//! `Event::UpdateStatus`.

mod env;
mod release;
mod system;
mod updater;

pub use env::{App, Body, BoxFuture, HttpResponse, Platform, RunOutput, System, writable};
pub use release::{
    LATEST_URL, RELEASES_URL, Release, asset_name, digest_sha256, installer_version, is_newer, newest_installer,
    parse_release, tag_version, team_identifier,
};
pub use system::RealSystem;
pub use updater::*;
