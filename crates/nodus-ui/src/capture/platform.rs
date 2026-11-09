//! The panel window's OS side.
//!
//! macOS (CAP-002…CAP-006, spike S1 step 5): GPUI's `WindowKind::PopUp` is
//! already an NSPanel subclass with `NonactivatingPanel`, level 101 and
//! `CanJoinAllSpaces | FullScreenAuxiliary`; this re-asserts that, takes the
//! window's own shadow off (the card draws its own), makes it see-through,
//! and moves, shows and hides it. AppKit calls that call back into GPUI
//! (ordering front, becoming key, resizing) run from a foreground task, never
//! inside a GPUI update.
//!
//! Elsewhere the panel is GPUI's pop-up window: a topmost tool window with no
//! taskbar button. It can't be moved once open, so it opens where it's
//! placed and closes when it hides; the next show makes it again (CAP-001).

use nodus_domain::Rect;

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use gpui::Window;
    use nodus_domain::Rect;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    /// The NSWindow behind the panel, kept by the UI thread.
    pub type NativePanel = Retained<NSWindow>;

    /// What Electron's `type: 'panel'`, `setVisibleOnAllWorkspaces(true,
    /// { visibleOnFullScreen: true, skipTransformProcessType: true })` and
    /// `setAlwaysOnTop(true, 'pop-up-menu')` give, on GPUI's pop-up panel.
    pub fn configure(window: &Window) -> Option<NativePanel> {
        let w = crate::mac::ns_window(window)?;
        // SAFETY: plain NSPanel/NSWindow setters on the main thread; none of
        // them calls back into GPUI.
        unsafe {
            // CAP-004. `setFloatingPanel:` resets the level, so it goes first.
            let _: () = msg_send![&*w, setFloatingPanel: Bool::YES];
            // CAP-006: NSPopUpMenuWindowLevel.
            w.setLevel(101);
            // CAP-005: every Space, over full-screen apps; the activation
            // policy is left alone, so the Dock icon stays.
            w.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            // CAP-002: no window shadow, fully transparent, fixed.
            w.setHasShadow(false);
            w.setOpaque(false);
            let clear: *mut AnyObject = msg_send![objc2::class!(NSColor), clearColor];
            let _: () = msg_send![&*w, setBackgroundColor: clear];
            let _: () = msg_send![&*w, setHidesOnDeactivate: Bool::NO];
            let _: () = msg_send![&*w, setBecomesKeyOnlyIfNeeded: Bool::NO];
            let _: () = msg_send![&*w, setMovable: Bool::NO];
            let _: () = msg_send![&*w, setMovableByWindowBackground: Bool::NO];
            let title = NSString::from_str("Nodus Capture");
            w.setTitle(&title);
        }
        Some(w)
    }

    /// The height of the primary screen, which Cocoa's y runs up from.
    fn primary_height() -> f64 {
        // SAFETY: NSScreen.screens is read on the main thread; the first is
        // the one with the menu bar.
        unsafe {
            let screens: *mut AnyObject = msg_send![objc2::class!(NSScreen), screens];
            if screens.is_null() {
                return 0.0;
            }
            let count: usize = msg_send![screens, count];
            if count == 0 {
                return 0.0;
            }
            let first: *mut AnyObject = msg_send![screens, objectAtIndex: 0usize];
            let frame: NSRect = msg_send![first, frame];
            frame.size.height
        }
    }

    /// A Cocoa rect (bottom-left origin) as Electron's top-left one.
    fn top_left(rect: NSRect, primary: f64) -> Rect {
        Rect {
            x: rect.origin.x,
            y: primary - rect.origin.y - rect.size.height,
            w: rect.size.width,
            h: rect.size.height,
        }
    }

    /// `screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).workArea`
    /// (CAP-008): the visible frame (without menu bar and Dock) of the
    /// screen under the pointer, or the nearest one.
    pub fn work_area_under_pointer() -> Option<Rect> {
        // SAFETY: NSEvent.mouseLocation and NSScreen's frames, read on the
        // main thread.
        unsafe {
            let mouse: NSPoint = msg_send![objc2::class!(NSEvent), mouseLocation];
            let screens: *mut AnyObject = msg_send![objc2::class!(NSScreen), screens];
            if screens.is_null() {
                return None;
            }
            let count: usize = msg_send![screens, count];
            let primary = primary_height();
            let mut best: Option<(f64, Rect)> = None;
            for i in 0..count {
                let screen: *mut AnyObject = msg_send![screens, objectAtIndex: i];
                let frame: NSRect = msg_send![screen, frame];
                let visible: NSRect = msg_send![screen, visibleFrame];
                let dx = (frame.origin.x - mouse.x)
                    .max(mouse.x - (frame.origin.x + frame.size.width))
                    .max(0.0);
                let dy = (frame.origin.y - mouse.y)
                    .max(mouse.y - (frame.origin.y + frame.size.height))
                    .max(0.0);
                let distance = dx * dx + dy * dy;
                if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                    best = Some((distance, top_left(visible, primary)));
                }
            }
            best.map(|(_, rect)| rect)
        }
    }

    /// Moves and sizes the panel to `rect` (top-left origin), keeping its top
    /// fixed as its height changes (CAP-008, CAP-009).
    pub fn set_frame(w: &NSWindow, rect: Rect) {
        let primary = primary_height();
        let frame = NSRect::new(
            NSPoint::new(rect.x, primary - rect.y - rect.h),
            NSSize::new(rect.w, rect.h),
        );
        w.setFrame_display(frame, true);
    }

    /// Up and with the keyboard, without bringing Nodus forward (CAP-004):
    /// the app in front stays the active app. `quiet` (the test driver)
    /// never takes the keyboard.
    pub fn show(w: &NSWindow, focus: bool) {
        w.orderFrontRegardless();
        if focus {
            w.makeKeyWindow();
        }
    }

    pub fn hide(w: &NSWindow) {
        w.orderOut(None);
    }

    pub fn is_key(w: &NSWindow) -> bool {
        w.isKeyWindow()
    }

    /// Whether any of Nodus's windows has the keyboard (`BrowserWindow.getFocusedWindow()`).
    pub fn any_window_key() -> bool {
        // SAFETY: NSApp.keyWindow, read on the main thread.
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            let key: *mut AnyObject = msg_send![app, keyWindow];
            !key.is_null()
        }
    }

    /// The open dialog of CAP-060: files only, several at once, as a sheet
    /// on the panel. `done` gets the paths, or none when cancelled.
    pub fn pick_files(parent: Option<NativePanel>, done: impl FnOnce(Option<Vec<String>>) + 'static) {
        // SAFETY: NSOpenPanel set up on the main thread; its completion block
        // runs once, on the main thread.
        unsafe {
            let panel: *mut AnyObject = msg_send![objc2::class!(NSOpenPanel), openPanel];
            if panel.is_null() {
                done(None);
                return;
            }
            let _: () = msg_send![panel, setCanChooseFiles: Bool::YES];
            let _: () = msg_send![panel, setCanChooseDirectories: Bool::NO];
            let _: () = msg_send![panel, setAllowsMultipleSelection: Bool::YES];
            let panel_ptr = panel as usize;
            let done = std::cell::RefCell::new(Some(done));
            let block = block2::RcBlock::new(move |response: isize| {
                let Some(done) = done.borrow_mut().take() else { return };
                // NSModalResponseOK.
                if response != 1 {
                    return done(None);
                }
                let panel = panel_ptr as *mut AnyObject;
                let urls: *mut AnyObject = msg_send![panel, URLs];
                let count: usize = msg_send![urls, count];
                let mut paths = Vec::new();
                for i in 0..count {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                    let path: *mut NSString = msg_send![url, path];
                    if !path.is_null() {
                        paths.push(PathBuf::from((*path).to_string()).to_string_lossy().into_owned());
                    }
                }
                done(Some(paths));
            });
            match parent {
                Some(window) => {
                    let _: () = msg_send![panel, beginSheetModalForWindow: &*window, completionHandler: &*block];
                }
                None => {
                    let _: () = msg_send![panel, beginWithCompletionHandler: &*block];
                }
            }
        }
    }
}

/// A work area to fall back on when the OS gives none.
pub fn default_work_area() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1440.0,
        h: 900.0,
    }
}
