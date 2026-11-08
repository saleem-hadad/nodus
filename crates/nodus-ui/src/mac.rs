//! AppKit glue through objc2, where GPUI has no API: the app's appearance,
//! the window's background, the Add Project folder picker, the About panel,
//! the Help menu, and the window's number for screenshots.
//!
//! AppKit calls that dispatch events synchronously must run outside GPUI
//! update closures (S1, Gotchas).

use std::path::PathBuf;

use gpui::{Rgba, Window};
use nodus_domain::ThemePreference;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// GPUI's content view, which raw-window-handle hands out on macOS.
fn ns_view(window: &Window) -> Option<Retained<NSView>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    let ptr = h.ns_view.as_ptr().cast::<NSView>();
    // SAFETY: the pointer is GPUI's live content view, retained for the caller.
    unsafe { Retained::retain(ptr) }
}

/// The NSWindow behind a GPUI window.
pub fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    ns_view(window)?.window()
}

/// The window server's number for the window (`screencapture -l`).
pub fn window_number(window: &Window) -> Option<isize> {
    ns_window(window).map(|w| w.windowNumber())
}

/// `NSApplication.sharedApplication`.
fn app() -> *mut AnyObject {
    // SAFETY: NSApplication exists once the app is running, which it is
    // before anything here is called.
    unsafe { msg_send![class!(NSApplication), sharedApplication] }
}

fn appearance_named(name: &str) -> *mut AnyObject {
    let name = NSString::from_str(name);
    // SAFETY: appearanceNamed: takes an NSString and returns an autoreleased
    // appearance or nil.
    unsafe { msg_send![class!(NSAppearance), appearanceNamed: &*name] }
}

/// `nativeTheme.themeSource`: forces the app's appearance, or follows the
/// system's with `System`. Native menus, dialogs and the window chrome follow
/// it (UI-005).
pub fn set_app_appearance(preference: ThemePreference) {
    let appearance = match preference {
        ThemePreference::System => std::ptr::null_mut(),
        ThemePreference::Light => appearance_named("NSAppearanceNameAqua"),
        ThemePreference::Dark => appearance_named("NSAppearanceNameDarkAqua"),
    };
    // SAFETY: setAppearance: takes an NSAppearance or nil.
    unsafe {
        let _: () = msg_send![app(), setAppearance: appearance];
    }
}

/// Forces one window's appearance, as a system switch does to it (tests and
/// the automation driver; the user's own setting is never touched).
pub fn force_window_appearance(window: &Window, dark: Option<bool>) {
    let Some(w) = ns_window(window) else { return };
    let appearance = match dark {
        None => std::ptr::null_mut(),
        Some(false) => appearance_named("NSAppearanceNameAqua"),
        Some(true) => appearance_named("NSAppearanceNameDarkAqua"),
    };
    // SAFETY: setAppearance: takes an NSAppearance or nil.
    unsafe {
        let _: () = msg_send![&*w, setAppearance: appearance];
    }
}

/// The native window background, so a resize never flashes the other theme
/// (UI-003).
pub fn set_window_background(window: &Window, color: Rgba) {
    let Some(w) = ns_window(window) else { return };
    // SAFETY: colorWithSRGBRed:green:blue:alpha: returns an autoreleased
    // NSColor; setBackgroundColor: retains it.
    unsafe {
        let color: *mut AnyObject = msg_send![class!(NSColor),
            colorWithSRGBRed: f64::from(color.r),
            green: f64::from(color.g),
            blue: f64::from(color.b),
            alpha: f64::from(color.a)];
        let _: () = msg_send![&*w, setBackgroundColor: color];
    }
}

/// The Add Project folder picker (PLAT-068): title and button `Add Project`,
/// the message, folders only, New Folder; a sheet on `parent` when given.
/// `done` gets the folder, or none when cancelled.
pub fn pick_folder(parent: Option<Retained<NSWindow>>, done: impl FnOnce(Option<PathBuf>) + 'static) {
    // SAFETY: plain NSOpenPanel configuration on the main thread; the
    // completion block is called once, on the main thread.
    unsafe {
        let panel: *mut AnyObject = msg_send![class!(NSOpenPanel), openPanel];
        if panel.is_null() {
            done(None);
            return;
        }
        let title = NSString::from_str("Add Project");
        let message = NSString::from_str("Choose the local repository for this project");
        let _: () = msg_send![panel, setTitle: &*title];
        let _: () = msg_send![panel, setPrompt: &*title];
        let _: () = msg_send![panel, setMessage: &*message];
        let _: () = msg_send![panel, setCanChooseDirectories: Bool::YES];
        let _: () = msg_send![panel, setCanChooseFiles: Bool::NO];
        let _: () = msg_send![panel, setAllowsMultipleSelection: Bool::NO];
        let _: () = msg_send![panel, setCanCreateDirectories: Bool::YES];
        let panel_ptr = panel as usize;
        let done = std::cell::RefCell::new(Some(done));
        let block = block2::RcBlock::new(move |response: isize| {
            let Some(done) = done.borrow_mut().take() else { return };
            // NSModalResponseOK is 1.
            if response != 1 {
                return done(None);
            }
            let panel = panel_ptr as *mut AnyObject;
            let urls: *mut AnyObject = msg_send![panel, URLs];
            let count: usize = msg_send![urls, count];
            if count == 0 {
                return done(None);
            }
            let url: *mut AnyObject = msg_send![urls, objectAtIndex: 0usize];
            let path: *mut NSString = msg_send![url, path];
            if path.is_null() {
                return done(None);
            }
            done(Some(PathBuf::from((*path).to_string())));
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

/// `About Nodus`: the standard About panel (PLAT-058).
pub fn show_about_panel() {
    // SAFETY: a standard NSApplication action with no argument.
    unsafe {
        let _: () = msg_send![app(), orderFrontStandardAboutPanel: std::ptr::null_mut::<AnyObject>()];
    }
}

/// Registers the menu titled `Help` as the app's Help menu, so macOS adds its
/// search field (PLAT-044).
pub fn register_help_menu() {
    // SAFETY: reads NSApp.mainMenu's items and sets one as the Help menu.
    unsafe {
        let main: *mut AnyObject = msg_send![app(), mainMenu];
        if main.is_null() {
            return;
        }
        let title = NSString::from_str("Help");
        let item: *mut AnyObject = msg_send![main, itemWithTitle: &*title];
        if item.is_null() {
            return;
        }
        let submenu: *mut AnyObject = msg_send![item, submenu];
        if !submenu.is_null() {
            let _: () = msg_send![app(), setHelpMenu: submenu];
        }
    }
}

/// Hides or shows the traffic lights (screenshots of the content only).
pub fn set_traffic_lights_hidden(window: &Window, hidden: bool) {
    let Some(w) = ns_window(window) else { return };
    for kind in 0usize..3 {
        // SAFETY: standardWindowButton: returns the window's own button or nil.
        unsafe {
            let button: *mut AnyObject = msg_send![&*w, standardWindowButton: kind];
            if !button.is_null() {
                let _: () = msg_send![button, setHidden: Bool::new(hidden)];
            }
        }
    }
}

/// Makes the window deaf to the real pointer (the automation driver), so a
/// test run ignores the user's mouse.
pub fn ignore_real_mouse(window: &Window) {
    if let Some(w) = ns_window(window) {
        w.setIgnoresMouseEvents(true);
    }
}

/// `process.getSystemVersion()`: `major.minor.patch`.
pub fn os_version() -> String {
    let v = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    format!("{}.{}.{}", v.majorVersion, v.minorVersion, v.patchVersion)
}

/// Shows the window without making the app active (the automation driver,
/// so a test run never takes the user's focus).
pub fn order_front_without_focus(window: &Window) {
    if let Some(w) = ns_window(window) {
        w.orderFrontRegardless();
    }
}
