//! The app as the core sees it (`nodus_core::AppShell`): whether the main
//! window is open and focused, bringing it up, quitting, and the questions
//! only the app can put to the user (HOST-102's other-version box, UPD-066's
//! stop-the-sessions question). Also the menu items and buttons that run
//! core commands with a question in between: Restart to Update, Check for
//! Updates…, Quit and Stop Sessions.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, Global, PromptButton, PromptLevel};
use nodus_core::{AppShell, BoxFuture, Nodus, ReplaceChoice, RestartOutcome, StopSessionsQuestion};
use tokio::sync::{mpsc, oneshot};

use crate::shell::window;

/// HOST-102's box, verbatim.
pub const OTHER_VERSION_MESSAGE: &str = "Sessions from another version of Nodus are still running";
pub const OTHER_VERSION_DETAIL: &str =
    "This version can’t connect to them. Stop them to carry on, or quit and let them finish.";

/// What the main window is doing, kept where any thread can read it.
#[derive(Debug, Default)]
pub struct WindowFlags {
    open: AtomicBool,
    focused: AtomicBool,
}

impl WindowFlags {
    pub fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::SeqCst);
        if !open {
            self.focused.store(false, Ordering::SeqCst);
        }
    }

    pub fn set_focused(&self, focused: bool) {
        self.focused.store(focused, Ordering::SeqCst);
    }
}

/// The flags, for the window code on the main thread.
#[derive(Clone)]
pub struct Flags(pub Arc<WindowFlags>);

impl Global for Flags {}

/// What the core asks the main thread for.
pub enum ShellRequest {
    Show,
    Quit,
    /// Off a Mac, HOST-102's box goes on the main window.
    OtherVersion(oneshot::Sender<ReplaceChoice>),
}

/// `nodus_core::AppShell` for the windowed app.
pub struct UiShell {
    flags: Arc<WindowFlags>,
    requests: mpsc::UnboundedSender<ShellRequest>,
}

impl UiShell {
    /// The shell, and where its requests arrive for [`serve`] on the main thread.
    pub fn new(flags: Arc<WindowFlags>) -> (Self, mpsc::UnboundedReceiver<ShellRequest>) {
        let (requests, received) = mpsc::unbounded_channel();
        (Self { flags, requests }, received)
    }
}

impl AppShell for UiShell {
    fn is_open(&self) -> bool {
        self.flags.open.load(Ordering::SeqCst)
    }

    fn is_focused(&self) -> bool {
        self.is_open() && self.flags.focused.load(Ordering::SeqCst)
    }

    /// A window's state subscribes as it's made and asks for a pending card
    /// at once (UI-044), so it's never "loading" as a page was.
    fn is_loading(&self) -> bool {
        false
    }

    fn show(&self) {
        let _ = self.requests.send(ShellRequest::Show);
    }

    fn quit(&self) {
        let _ = self.requests.send(ShellRequest::Quit);
    }

    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        let (tx, rx) = oneshot::channel();
        #[cfg(target_os = "macos")]
        {
            // Off any GPUI update: the alert runs AppKit's modal loop itself,
            // app-modal as Electron's parentless message box is.
            dispatch2::DispatchQueue::main().exec_async(move || {
                let _ = tx.send(mac_other_version());
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = self.requests.send(ShellRequest::OtherVersion(tx));
        }
        // A box that never answers (the app quitting) counts as Quit, its cancel.
        Box::pin(async move { rx.await.unwrap_or(ReplaceChoice::Quit) })
    }
}

/// Carries out the core's requests on the main thread, for the life of the app.
pub fn serve(mut requests: mpsc::UnboundedReceiver<ShellRequest>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(request) = requests.recv().await {
            match request {
                // PLAT-023
                ShellRequest::Show => cx.update(window::show_window),
                ShellRequest::Quit => cx.update(|cx| cx.quit()),
                ShellRequest::OtherVersion(answer) => cx.update(|cx| ask_other_version(answer, cx)),
            }
        }
    })
    .detach();
}

/// HOST-102 off a Mac: the warning box, on the main window, which comes up
/// for it if it isn't there yet. A box that can't be shown counts as Quit.
fn ask_other_version(answer: oneshot::Sender<ReplaceChoice>, cx: &mut App) {
    let asked = window::main_window(cx)
        .or_else(|| window::open_main_window(cx))
        .and_then(|handle| {
            handle
                .update(cx, |_, window, cx| {
                    window.prompt(
                        PromptLevel::Warning,
                        OTHER_VERSION_MESSAGE,
                        Some(OTHER_VERSION_DETAIL),
                        &[
                            PromptButton::Ok("Stop Sessions".into()),
                            PromptButton::Cancel("Quit".into()),
                        ],
                        cx,
                    )
                })
                .ok()
        });
    let Some(asked) = asked else {
        let _ = answer.send(ReplaceChoice::Quit);
        return;
    };
    cx.spawn(async move |_| {
        let choice = match asked.await {
            Ok(0) => ReplaceChoice::StopSessions,
            _ => ReplaceChoice::Quit,
        };
        let _ = answer.send(choice);
    })
    .detach();
}

/// HOST-102 on a Mac: a warning alert, app-modal, `Stop Sessions` the default
/// (Return) and `Quit` the cancel (Escape). Main thread only.
#[cfg(target_os = "macos")]
fn mac_other_version() -> ReplaceChoice {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication};
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return ReplaceChoice::Quit;
    };
    NSApplication::sharedApplication(mtm).activate();
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str(OTHER_VERSION_MESSAGE));
    alert.setInformativeText(&NSString::from_str(OTHER_VERSION_DETAIL));
    alert.addButtonWithTitle(&NSString::from_str("Stop Sessions"));
    let quit = alert.addButtonWithTitle(&NSString::from_str("Quit"));
    quit.setKeyEquivalent(&NSString::from_str("\u{1b}"));
    if alert.runModal() == NSAlertFirstButtonReturn {
        ReplaceChoice::StopSessions
    } else {
        ReplaceChoice::Quit
    }
}

/// UI-108, UI-149, UPD-078: Restart to Update. On Windows with sessions
/// running, UPD-066's question comes first, on the main window.
pub fn install_update(nodus: Nodus, cx: &mut App) {
    cx.spawn(async move |cx| {
        let RestartOutcome::Ask(question) = nodus.updates().install().await else {
            return;
        };
        cx.update(|cx| ask_stop_sessions(nodus, &question, cx));
    })
    .detach();
}

/// UPD-066's question: `Stop and Update` the default, `Not Now` the cancel.
/// Closing it, or a window that isn't there, is Not Now.
fn ask_stop_sessions(nodus: Nodus, question: &StopSessionsQuestion, cx: &mut App) {
    let [stop, not_now] = question.buttons.clone();
    let default_id = question.default_id;
    let asked = window::main_window(cx).and_then(|handle| {
        handle
            .update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &question.message,
                    Some(&question.detail),
                    &[PromptButton::Ok(stop.into()), PromptButton::Cancel(not_now.into())],
                    cx,
                )
            })
            .ok()
    });
    cx.spawn(async move |_| {
        let stop = match asked {
            Some(asked) => asked.await == Ok(default_id),
            None => false,
        };
        nodus.updates().answer_stop_sessions(stop).await;
    })
    .detach();
}

/// PLAT-046, UPD-079: `Check for Updates…` opens Settings to show how the
/// check goes (dropped with no window, PLAT-036) and starts a check the user
/// asked for.
pub fn check_for_updates(nodus: Nodus, cx: &mut App) {
    let window_open = window::main_window(cx).is_some();
    cx.spawn(async move |_| {
        if window_open {
            nodus.menu().check_for_updates().await;
        } else {
            nodus.updates().check().await;
        }
    })
    .detach();
}

/// PLAT-034: `Quit and Stop Sessions` stops every session and the host (5 s
/// at most), then quits.
pub fn quit_and_stop_sessions(nodus: Nodus, cx: &mut App) {
    cx.spawn(async move |_| nodus.menu().quit_and_stop_sessions().await)
        .detach();
}
