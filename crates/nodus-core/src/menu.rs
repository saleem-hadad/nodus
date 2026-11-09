//! What the application menu's own items do in the core (`src/main/menu.ts`):
//! the ones that aren't just a menu command for the window.

use nodus_domain::{Event, MenuCommand};

use crate::Nodus;

/// The application menu's items that reach into the core.
pub struct MenuApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl MenuApi<'_> {
    /// `Check for Updates…` (PLAT-046, UPD-079): Settings opens to show how
    /// the check goes (the `settings` menu command), and a check the user
    /// asked for starts. Done once the lookup is.
    pub async fn check_for_updates(&self) {
        self.nodus
            .on_core(async {
                self.nodus.emit(Event::Menu(MenuCommand::Settings));
                self.nodus.inner.updater.check(true).await;
            })
            .await;
    }

    /// `Quit and Stop Sessions` (PLAT-034, HOST-112, HOST-225): stops every
    /// session and the session host, waiting until the host has gone or 5 s
    /// have passed, then quits through the shell, even if stopping failed.
    pub async fn quit_and_stop_sessions(&self) {
        self.nodus
            .on_core(async {
                self.nodus.inner.host.stop_all().await;
                self.nodus.inner.shell.quit();
            })
            .await;
    }
}
