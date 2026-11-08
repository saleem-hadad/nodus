//! `window.api`'s top level: what the app runs on, cards a notification asked
//! for, and menu commands sent from the window.

use nodus_domain::{AppInfo, Event, MenuCommand, OpenCardRequest};

use crate::Nodus;
use crate::nodus::lock;

/// `window.api.appInfo`, `takeOpenRequest` and `menuCommand`.
///
/// `showMenu(x, y)` isn't here: the menu it pops up is the window's own, so
/// the UI opens it itself (PLAT-060).
pub struct AppApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl AppApi<'_> {
    /// `app:info`: where `claude` is, its version, and Node's name for the
    /// OS. Waits for the login shell's environment (PLAT-128).
    pub async fn info(&self) -> AppInfo {
        let env = self.nodus.env().await;
        AppInfo {
            claude_path: env.claude_path().map(|p| p.to_string_lossy().into_owned()),
            claude_version: env.claude_version().map(str::to_owned),
            platform: node_platform().into(),
        }
    }

    /// `app:take-open-request`: the card a notification clicked while no
    /// window was there asked to open, once; none after that (UI-044).
    pub async fn take_open_request(&self) -> Option<OpenCardRequest> {
        lock(&self.nodus.inner.open_request).take()
    }

    /// `app:menu-command`: sends a menu command to the window, as if its menu
    /// item were chosen. Anything that isn't one of the seven is ignored
    /// (PLAT-061, PLAT-062).
    pub async fn menu_command(&self, command: &str) {
        if let Some(command) = MenuCommand::parse(command) {
            self.nodus.emit(Event::Menu(command));
        }
    }
}

/// `process.platform`.
fn node_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    }
}
