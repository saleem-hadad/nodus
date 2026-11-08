//! What only the app around the core can do: its window, quitting, and the
//! questions it puts to the user. The windowed app implements [`AppShell`];
//! a headless Nodus has [`Headless`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use nodus_sessions::client::ReplaceChoice;

/// A boxed future, for the traits the app implements.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The app around the core: its main window (for notifications and cards a
/// notification opens, 02), quitting (the updater, Quit and Stop Sessions,
/// 01 and 06), and the question about another version's sessions (HOST-102).
///
/// Called from any thread; the windowed app hands the work to its main thread.
pub trait AppShell: Send + Sync {
    /// Whether there's a main window that hasn't been closed.
    fn is_open(&self) -> bool;
    /// Whether that window has focus.
    fn is_focused(&self) -> bool;
    /// Whether it's still loading, so it can't take a card to open yet and
    /// asks with `app.take_open_request()` once it can (UI-044).
    fn is_loading(&self) -> bool;
    /// `showWindow()` (PLAT-023): the main window, opened again if it was
    /// closed, restored if it was minimised, then shown and focused.
    fn show(&self);
    /// `app.quit()`: the normal quit path. Windows close, the app calls
    /// [`Nodus::shutdown`](crate::Nodus::shutdown), and the process exits.
    fn quit(&self);
    /// The warning box `Sessions from another version of Nodus are still
    /// running`, detail `This version can’t connect to them. Stop them to
    /// carry on, or quit and let them finish.`, buttons `Stop Sessions`
    /// (default) and `Quit` (cancel). It can come up at launch, before the
    /// window exists (HOST-102).
    fn other_version(&self) -> BoxFuture<ReplaceChoice>;
    /// `app.exit(0)`: leaves at once, without the quit path, after the user
    /// chose Quit in [`AppShell::other_version`] (HOST-103).
    fn exit_now(&self) {
        std::process::exit(0);
    }
}

/// No window and no one to ask: a headless Nodus, and tests. Notifications
/// see the app as in the background, quitting is up to whoever runs it, and
/// another version's sessions get the dialog's default, `Stop Sessions`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Headless;

impl AppShell for Headless {
    fn is_open(&self) -> bool {
        false
    }

    fn is_focused(&self) -> bool {
        false
    }

    fn is_loading(&self) -> bool {
        false
    }

    fn show(&self) {}

    fn quit(&self) {}

    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        Box::pin(async { ReplaceChoice::StopSessions })
    }
}

/// The shell as notifications need it (`notify.ts`'s window).
pub(crate) struct ShellWindow(pub(crate) Arc<dyn AppShell>);

impl nodus_sessions::notify::AppWindow for ShellWindow {
    fn is_open(&self) -> bool {
        self.0.is_open()
    }

    fn is_focused(&self) -> bool {
        self.0.is_focused()
    }

    fn is_loading(&self) -> bool {
        self.0.is_loading()
    }

    fn show(&self) {
        self.0.show();
    }
}

/// The shell as the session host client asks it (HOST-102).
pub(crate) struct ShellPrompt(pub(crate) Arc<dyn AppShell>);

impl nodus_sessions::client::HostPrompt for ShellPrompt {
    fn other_version(&self) -> nodus_sessions::client::BoxFuture<ReplaceChoice> {
        self.0.other_version()
    }
}
