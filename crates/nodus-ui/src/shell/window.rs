//! The main window (`createWindow`, `showWindow`; spec 01 PLAT-014…PLAT-023).

use gpui::{
    App, AppContext as _, Bounds, Global, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowOptions, point, px, size,
};
use std::sync::Arc;

use nodus_core::Nodus;

use crate::shell::core_shell::Flags;
use crate::shell::root::Shell;

/// The core, for whoever opens a window.
#[derive(Clone)]
pub struct Core(pub Nodus);

impl Global for Core {}

/// The main window, while it's open (there's at most one, PLAT-022).
#[derive(Default)]
pub struct MainWindow {
    pub handle: Option<WindowHandle<gpui::base::Root>>,
    pub shell: Option<gpui::Entity<Shell>>,
}

impl Global for MainWindow {}

/// The main window, if it's open.
pub fn main_window(cx: &App) -> Option<WindowHandle<gpui::base::Root>> {
    let handle = cx.try_global::<MainWindow>()?.handle?;
    cx.windows()
        .iter()
        .any(|w| w.window_id() == handle.window_id())
        .then_some(handle)
}

/// The main window's shell view, if it's open.
pub fn main_shell(cx: &App) -> Option<gpui::Entity<Shell>> {
    main_window(cx)?;
    cx.try_global::<MainWindow>()?.shell.clone()
}

/// Whether windows open without taking focus (the automation driver).
pub fn quiet() -> bool {
    std::env::var_os("NODUS_AUTOMATION").is_some_and(|v| !v.is_empty())
}

/// `createWindow`: 1440×920 centred, at least 960×620, titled Nodus, with
/// its own title bar under the traffic lights at (18, 18) (PLAT-014…PLAT-017).
pub fn open_main_window(cx: &mut App) -> Option<WindowHandle<gpui::base::Root>> {
    let nodus = cx.global::<Core>().0.clone();
    let quiet = quiet();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1440.), px(920.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Nodus".into()),
            appears_transparent: true,
            traffic_light_position: cfg!(target_os = "macos").then(|| point(px(18.), px(18.))),
        }),
        app_owns_titlebar_drag: true,
        window_min_size: Some(size(px(960.), px(620.))),
        focus: !quiet,
        show: true,
        window_background: WindowBackgroundAppearance::Opaque,
        // Inactive windows keep drawing at the display's rate, as Chromium does.
        inactive_frame_interval: None,
        app_id: Some("dev.saleem.nodus".into()),
        ..WindowOptions::default()
    };
    let flags = cx.try_global::<Flags>().map(|f| Arc::clone(&f.0));
    let mut shell = None;
    let handle = cx
        .open_window(options, |window, cx| {
            let view = cx.new(|cx| Shell::new(nodus.clone(), window, cx));
            shell = Some(view.clone());
            cx.new(|cx| {
                // Notifications only show while the app is in the background (HOST-205).
                if let Some(flags) = flags.clone() {
                    flags.set_focused(window.is_window_active());
                    cx.observe_window_activation(window, move |_, window, _| {
                        flags.set_focused(window.is_window_active());
                    })
                    .detach();
                }
                gpui::base::Root::new(view, window, cx)
            })
        })
        .ok()?;
    if let Some(flags) = cx.try_global::<Flags>() {
        flags.0.set_open(true);
    }
    // UPD-080: the update status as it is now; changes arrive as events.
    if let Some(view) = &shell {
        let state = view.read(cx).state.clone();
        cx.spawn(async move |cx| {
            let status = nodus.updates().status().await;
            state.update(cx, |s, cx| {
                s.update_status = Some(status);
                cx.notify();
            });
        })
        .detach();
    }
    // PLAT-139: the page zoom the user left it at, here or in 0.9.4.
    let zoom = nodus_core::zoom::factor(cx.global::<Core>().0.zoom().level()) as f32;
    let _ = handle.update(cx, |_, window, cx| {
        window.set_ui_zoom(zoom, cx);
        crate::theme::sync_window(window, cx);
        #[cfg(target_os = "macos")]
        if quiet {
            crate::mac::order_front_without_focus(window);
            crate::mac::ignore_real_mouse(window);
        }
    });
    cx.set_global(MainWindow {
        handle: Some(handle),
        shell,
    });
    Some(handle)
}

/// View › Actual Size, Zoom In and Zoom Out (PLAT-057): the level goes to
/// 0, or half a step up or down, is kept, and the whole main window is
/// drawn at 1.2^level. Nothing without a main window.
pub fn zoom(step: Option<f64>, cx: &mut App) {
    let Some(handle) = main_window(cx) else { return };
    let zoom = cx.global::<Core>().0.zoom();
    let level = zoom.set_level(step.map_or(0.0, |step| zoom.level() + step));
    let factor = nodus_core::zoom::factor(level) as f32;
    // A shortcut runs this while its window is being updated: after that.
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| window.set_ui_zoom(factor, cx));
    });
}

/// `showWindow` (PLAT-023): opens the main window when there's none, else
/// restores and shows it.
pub fn show_window(cx: &mut App) {
    match main_window(cx) {
        Some(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        None => {
            open_main_window(cx);
        }
    }
}

/// View › Reload (debug builds only): the window starts over, as a fresh
/// renderer would: on the grid, nothing open.
pub fn reload(cx: &mut App) {
    let Some(handle) = main_window(cx) else { return };
    let nodus = cx.global::<Core>().0.clone();
    let mut shell = None;
    let _ = handle.update(cx, |root, window, cx| {
        let view = cx.new(|cx| Shell::new(nodus, window, cx));
        shell = Some(view.clone());
        *root = gpui::base::Root::new(view, window, cx);
        cx.notify();
    });
    if let Some(shell) = shell {
        cx.global_mut::<MainWindow>().shell = Some(shell);
    }
}
