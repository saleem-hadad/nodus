//! Report a Problem (docs/specs/13-feedback.md): what a report becomes before
//! the user reviews it, and posting it.
//!
//! - [`text`]: the drafted title and description, the GitHub username, the
//!   footer, and taking personal details out (RPT-020…RPT-027);
//! - [`images`]: screenshots as GitHub takes them (RPT-010…RPT-014);
//! - [`send`]: the request to the feedback server, which holds the GitHub
//!   credentials and files the issue (RPT-030…RPT-036);
//! - [`browser`]: the filled-in new-issue page for when the server can't be
//!   reached (RPT-040…RPT-043).
//!
//! The server's code is in `feedback-server/` at the repository's root.

pub mod browser;
pub mod images;
pub mod send;
pub mod text;

pub use browser::{OfflineIssue, offline_issue};
pub use images::{MAX_IMAGE_BYTES, MAX_SHOTS, PrepareError, png_of_rgba, prepare};
pub use send::{ClientInfo, Posted, Report, SendError, send};
pub use text::{InvalidUsername, Scrub, draft_body, draft_title, footer, github_username};

/// Where reports go: `feedback-server/` on Laravel Cloud (RPT-031). Empty,
/// every send would count as unreachable and the browser take over (RPT-040).
pub const SERVER_URL: &str = "https://nodus-feedback.laravel.cloud";

/// Stands in for [`SERVER_URL`] when set and not empty (tests, a local
/// server).
pub const SERVER_URL_ENV: &str = "NODUS_FEEDBACK_URL";

/// The server's `X-Nodus-Key`. Not a secret: it ships inside every build and
/// only lets the server turn away generic bots (RPT-031).
pub const APP_KEY: &str = "nfb_5fc695f39fb55849978f2fc1";

/// The server to post to: [`SERVER_URL_ENV`], else [`SERVER_URL`].
pub fn server_url() -> String {
    std::env::var(SERVER_URL_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| SERVER_URL.to_owned())
}
