//! Report a Problem (docs/specs/13-feedback.md): the core's half. The
//! dialog is the UI's; posting, the screenshot of part of the screen, the
//! personal details to take out and the remembered GitHub username are here.

use std::path::PathBuf;

use nodus_capture::screen::{self, Screenshot};
use nodus_feedback::{Posted, Report, Scrub, SendError};

use crate::Nodus;

/// Where the GitHub username is kept between reports (RPT-006), beside the
/// other values the app's pages keep.
const USERNAME_KEY: &str = "nodus.report.githubUsername";

/// `Report a Problem…`'s commands.
pub struct FeedbackApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

/// What a screenshot of part of the screen came to (RPT-008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaShot {
    /// The picture's bytes, as the screenshot tool saved them.
    Image(Vec<u8>),
    /// macOS kept the screen from Nodus: no Screen Recording permission.
    NeedsPermission,
    /// The user cancelled.
    None,
}

impl FeedbackApi<'_> {
    /// RPT-006: the username the last report named.
    pub fn username(&self) -> Option<String> {
        self.nodus
            .local_storage()
            .get(USERNAME_KEY)
            .filter(|name| !name.trim().is_empty())
    }

    /// RPT-006: kept once a report is posted; none forgets it.
    pub fn remember_username(&self, name: Option<&str>) {
        self.nodus.local_storage().set(USERNAME_KEY, name);
    }

    /// RPT-024: this computer's personal details, to take out of a report.
    pub async fn scrub(&self) -> Scrub {
        self.nodus
            .on_core(async { tokio::task::spawn_blocking(Scrub::system).await.unwrap_or_default() })
            .await
    }

    /// RPT-008: macOS's crosshair (off a Mac, the screen under the pointer),
    /// the shot read back from where the tool saved it.
    pub async fn shoot_area(&self) -> AreaShot {
        let user_data = self.nodus.inner.user_data.clone();
        self.nodus
            .on_core(async move {
                match screen::take_screenshot(&user_data).await {
                    Screenshot::Image(shot) => match tokio::fs::read(&shot.path).await {
                        Ok(bytes) => AreaShot::Image(bytes),
                        Err(_) => AreaShot::None,
                    },
                    Screenshot::NeedsPermission => AreaShot::NeedsPermission,
                    Screenshot::None => AreaShot::None,
                }
            })
            .await
    }

    /// RPT-009: System Settings › Privacy & Security › Screen Recording.
    pub fn open_screen_settings(&self) {
        nodus_platform::permissions::open_screen_capture_settings();
    }

    /// RPT-030…RPT-035: posts the reviewed report to the feedback server.
    pub async fn send(&self, report: Report) -> Result<Posted, SendError> {
        self.nodus
            .on_core(async move {
                let server = nodus_feedback::server_url();
                nodus_feedback::send(&server, nodus_feedback::APP_KEY, &report).await
            })
            .await
    }

    /// RPT-043: the screenshots saved where the user can drag them into the
    /// browser's issue page: `<userData>/captures/report-<id>/screenshot-N.<ext>`.
    /// The first file, to show in its folder; none when there were none or
    /// they couldn't be saved.
    pub async fn keep_for_browser(&self, shots: Vec<Vec<u8>>) -> Option<PathBuf> {
        if shots.is_empty() {
            return None;
        }
        let dir = screen::shots_dir(&self.nodus.inner.user_data).join(format!("report-{}", uuid::Uuid::new_v4()));
        self.nodus
            .on_core(async move {
                tokio::fs::create_dir_all(&dir).await.ok()?;
                let mut first = None;
                for (i, shot) in shots.iter().enumerate() {
                    let ext = match nodus_feedback::images::kind_of(shot) {
                        Some("image/jpeg") => "jpg",
                        Some("image/gif") => "gif",
                        Some("image/webp") => "webp",
                        _ => "png",
                    };
                    let path = dir.join(format!("screenshot-{}.{ext}", i + 1));
                    tokio::fs::write(&path, shot).await.ok()?;
                    first.get_or_insert(path);
                }
                first
            })
            .await
    }
}
