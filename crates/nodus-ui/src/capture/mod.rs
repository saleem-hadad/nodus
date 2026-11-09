//! Floating capture's panel (spec 07, "Panel UI"; `src/renderer/src/capture/`):
//! the window that double-tapping ⇧ floats up over any app, and its page.
//!
//! nodus-capture decides when the panel opens, where and what it's told;
//! this is the window it drives ([`bridge`], [`platform`]) and the page in it
//! ([`panel`], drawn by `render`, `menu` and `receipt`). The app hands the
//! window to the core once, at startup, with [`register`].

pub mod automation;
pub mod bridge;
pub mod icons;
mod menu;
pub mod panel;
pub mod platform;
mod receipt;
mod render;
pub mod stubs;
pub mod style;

use std::sync::Arc;

use gpui::App;
use nodus_core::capture_service::{SystemHook, SystemPermissions, SystemScreen};
use nodus_core::{CaptureSystem, Nodus, WindowSize};

pub use bridge::{PanelBridge, PanelSlot};
pub use panel::CapturePanel;

use crate::shell::window;

/// `registerCapture` (CAP-038): gives the core the panel window, the
/// screen, the permissions, the key hook and the board window's size. Under
/// the test driver nothing is read from the screen and no key is heard.
pub fn register(nodus: &Nodus, cx: &mut App) {
    let bridge = PanelBridge::start(cx);
    cx.set_global(PanelSlot::new(nodus.clone()));
    let sizes = Arc::clone(&bridge);
    let window_size: WindowSize = Arc::new(move || sizes.board_window_size());
    let system = if window::quiet() {
        let stubs = stubs::Stubs::default();
        let trusted = Arc::clone(&stubs.trusted);
        cx.set_global(stubs);
        CaptureSystem {
            panel: bridge,
            window_size,
            permissions: Arc::new(stubs::DriverPermissions(trusted)),
            hook: Arc::new(stubs::DriverHook),
            ..CaptureSystem::headless()
        }
    } else {
        CaptureSystem {
            panel: bridge,
            screen: Arc::new(SystemScreen),
            permissions: Arc::new(SystemPermissions),
            hook: Arc::new(SystemHook),
            window_size,
        }
    };
    nodus.capture().register(system);
}

/// File › Capture… (CAP-042, PLAT-048) and Settings' Try It (CAP-043,
/// UI-135): opens the panel, never toggles.
pub fn show(nodus: &Nodus, cx: &mut App) {
    let nodus = nodus.clone();
    cx.spawn(async move |_| nodus.capture().show().await).detach();
}
