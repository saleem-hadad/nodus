//! The panel window as capture drives it (nodus-capture's `PanelWindow`):
//! capture calls from the core's runtime, and each call is carried to the UI
//! thread and answered from there. The window itself lives in [`PanelSlot`].

use std::sync::Arc;
use std::thread::ThreadId;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Entity, Global, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, point, px, size,
};
use nodus_core::Nodus;
use nodus_core::capture_service::PanelWindow;
use nodus_core::capture_service::service::{PANEL_H, PANEL_W};
use nodus_domain::Rect;
use tokio::sync::{mpsc, oneshot};

use super::panel::CapturePanel;
use super::platform;
use super::stubs::Stubs;
use crate::shell::window;

/// Work for the UI thread.
pub type Job = Box<dyn FnOnce(&mut App) + Send>;

/// How long a call waits for the UI thread before it gives up.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The panel window, for capture: each method runs on the UI thread.
pub struct PanelBridge {
    jobs: mpsc::UnboundedSender<Job>,
    /// The UI thread, which must never wait on itself.
    ui: ThreadId,
}

impl PanelBridge {
    /// A bridge whose jobs run on the UI thread, in order.
    pub fn start(cx: &mut App) -> Arc<Self> {
        let (jobs, mut rx) = mpsc::unbounded_channel::<Job>();
        cx.spawn(async move |cx| {
            while let Some(job) = rx.recv().await {
                cx.update(job);
            }
        })
        .detach();
        Arc::new(Self {
            jobs,
            ui: std::thread::current().id(),
        })
    }

    /// Runs `f` on the UI thread and waits for its answer; `fallback` when
    /// the UI is gone or doesn't answer, or when called from the UI thread
    /// itself (capture's commands run on the core's runtime, so it isn't).
    fn call<R: Send + 'static>(&self, fallback: R, f: impl FnOnce(&mut App) -> R + Send + 'static) -> R {
        if std::thread::current().id() == self.ui {
            tracing::warn!("the capture panel was asked something on the UI thread");
            return fallback;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let job: Job = Box::new(move |cx| {
            let _ = tx.send(f(cx));
        });
        if self.jobs.send(job).is_err() {
            return fallback;
        }
        rx.recv_timeout(CALL_TIMEOUT).unwrap_or(fallback)
    }

    /// The board window's content size while it's open (CAP-134).
    pub fn board_window_size(&self) -> Option<(f64, f64)> {
        self.call(None, |cx| {
            let handle = window::main_window(cx)?;
            handle
                .update(cx, |_, window, _| {
                    let size = window.viewport_size();
                    (f64::from(f32::from(size.width)), f64::from(f32::from(size.height)))
                })
                .ok()
        })
    }
}

impl PanelWindow for PanelBridge {
    fn exists(&self) -> bool {
        self.call(false, |cx| PanelSlot::live(cx).is_some())
    }

    fn create(&self) {
        self.call((), PanelSlot::create);
    }

    fn is_visible(&self) -> bool {
        self.call(false, |cx| {
            PanelSlot::live(cx).is_some() && cx.global::<PanelSlot>().visible
        })
    }

    fn is_focused(&self) -> bool {
        self.call(false, PanelSlot::is_focused)
    }

    /// GPUI draws the page as the window opens: it's never loading.
    fn is_loading(&self) -> bool {
        false
    }

    fn bounds(&self) -> Rect {
        self.call(default_bounds(), |cx| cx.global::<PanelSlot>().bounds)
    }

    fn set_bounds(&self, bounds: Rect) {
        self.call((), move |cx| PanelSlot::set_bounds(bounds, cx));
    }

    fn show_and_focus(&self) {
        self.call((), |cx| PanelSlot::show(true, cx));
    }

    fn show_inactive(&self) {
        self.call((), |cx| PanelSlot::show(false, cx));
    }

    fn hide(&self) {
        self.call((), PanelSlot::hide);
    }

    fn work_area_under_pointer(&self) -> Rect {
        self.call(platform::default_work_area(), |_| {
            #[cfg(target_os = "macos")]
            {
                platform::work_area_under_pointer().unwrap_or_else(platform::default_work_area)
            }
            #[cfg(not(target_os = "macos"))]
            {
                platform::default_work_area()
            }
        })
    }

    fn any_window_focused(&self) -> bool {
        self.call(false, |cx| {
            #[cfg(target_os = "macos")]
            {
                let _ = cx;
                platform::any_window_key()
            }
            #[cfg(not(target_os = "macos"))]
            {
                cx.active_window().is_some()
            }
        })
    }

    /// The open dialog, attached to the panel (CAP-060); the test driver's
    /// stand-in paths instead while it drives the app.
    fn pick_files(&self) -> nodus_board_future::BoxFuture<'_, Option<Vec<String>>> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |cx| PanelSlot::pick_files(tx, cx));
        let sent = self.jobs.send(job).is_ok();
        Box::pin(async move {
            if !sent {
                return None;
            }
            rx.await.ok().flatten()
        })
    }
}

/// The future type `PanelWindow::pick_files` returns.
mod nodus_board_future {
    pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;
}

fn default_bounds() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: PANEL_W,
        h: PANEL_H,
    }
}

/// The panel's window on the UI thread: made lazily, reused, made again if
/// it was closed (CAP-001, CAP-013).
pub struct PanelSlot {
    nodus: Nodus,
    handle: Option<WindowHandle<gpui::base::Root>>,
    view: Option<Entity<CapturePanel>>,
    #[cfg(target_os = "macos")]
    native: Option<platform::NativePanel>,
    /// Where it is (top-left origin, logical px), as capture last placed it.
    bounds: Rect,
    /// Whether it's up, as capture last showed or hid it.
    visible: bool,
}

impl Global for PanelSlot {}

impl PanelSlot {
    pub fn new(nodus: Nodus) -> Self {
        Self {
            nodus,
            handle: None,
            view: None,
            #[cfg(target_os = "macos")]
            native: None,
            bounds: default_bounds(),
            visible: false,
        }
    }

    /// Whether capture has it up (resting with the receipt included).
    pub fn is_up(&self) -> bool {
        self.visible
    }

    /// The window, while it's open.
    pub fn live(cx: &App) -> Option<WindowHandle<gpui::base::Root>> {
        let handle = cx.try_global::<Self>()?.handle?;
        cx.windows()
            .iter()
            .any(|w| w.window_id() == handle.window_id())
            .then_some(handle)
    }

    /// The panel's page, while its window is open.
    pub fn view(cx: &App) -> Option<Entity<CapturePanel>> {
        Self::live(cx)?;
        cx.try_global::<Self>()?.view.clone()
    }

    /// `getPanel` (CAP-001, CAP-002, CAP-007): hidden, frameless,
    /// see-through, fixed, `PANEL_W` wide, at the bounds capture last set.
    pub fn create(cx: &mut App) {
        if Self::live(cx).is_some() {
            return;
        }
        let Some(slot) = cx.try_global::<Self>() else { return };
        let nodus = slot.nodus.clone();
        let at = slot.bounds;
        let quiet = window::quiet();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(at.x as f32), px(at.y as f32)),
                size(px(at.w as f32), px(at.h as f32)),
            ))),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            inactive_frame_interval: None,
            app_id: Some("dev.saleem.nodus".into()),
            ..WindowOptions::default()
        };
        let mut view = None;
        let Ok(handle) = cx.open_window(options, |window, cx| {
            let panel = cx.new(|cx| CapturePanel::new(nodus, window, cx));
            view = Some(panel.clone());
            cx.new(|cx| gpui::base::Root::new(panel, window, cx))
        }) else {
            return;
        };
        #[cfg(target_os = "macos")]
        let native = handle
            .update(cx, |_, window, _| {
                crate::mac::tag_layer_srgb(window);
                if quiet {
                    crate::mac::ignore_real_mouse(window);
                }
                platform::configure(window)
            })
            .ok()
            .flatten();
        #[cfg(not(target_os = "macos"))]
        let _ = quiet;
        let slot = cx.global_mut::<Self>();
        slot.handle = Some(handle);
        slot.view = view;
        slot.visible = false;
        #[cfg(target_os = "macos")]
        {
            slot.native = native;
        }
    }

    fn is_focused(cx: &mut App) -> bool {
        let Some(handle) = Self::live(cx) else { return false };
        #[cfg(target_os = "macos")]
        {
            let _ = handle;
            cx.global::<Self>().native.as_ref().is_some_and(|w| platform::is_key(w))
        }
        #[cfg(not(target_os = "macos"))]
        {
            handle
                .update(cx, |_, window, _| window.is_window_active())
                .unwrap_or(false)
        }
    }

    fn set_bounds(bounds: Rect, cx: &mut App) {
        cx.global_mut::<Self>().bounds = bounds;
        let Some(handle) = Self::live(cx) else { return };
        #[cfg(target_os = "macos")]
        {
            let _ = handle;
            if let Some(native) = cx.global::<Self>().native.clone() {
                // Resizing calls back into GPUI: not inside this update.
                cx.foreground_executor()
                    .spawn(async move { platform::set_frame(&native, bounds) })
                    .detach();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = handle.update(cx, |_, window, _| {
                window.resize(size(px(bounds.w as f32), px(bounds.h as f32)));
            });
        }
    }

    /// Shows it, with the keyboard or without (`showInactive`).
    fn show(focus: bool, cx: &mut App) {
        if Self::live(cx).is_none() {
            Self::create(cx);
        }
        let Some(handle) = Self::live(cx) else { return };
        cx.global_mut::<Self>().visible = true;
        let quiet = window::quiet();
        #[cfg(target_os = "macos")]
        {
            let _ = handle;
            if let Some(native) = cx.global::<Self>().native.clone() {
                let bounds = cx.global::<Self>().bounds;
                cx.foreground_executor()
                    .spawn(async move {
                        platform::set_frame(&native, bounds);
                        platform::show(&native, focus && !quiet);
                    })
                    .detach();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if focus && !quiet {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                });
            }
        }
    }

    fn hide(cx: &mut App) {
        cx.global_mut::<Self>().visible = false;
        let Some(handle) = Self::live(cx) else { return };
        #[cfg(target_os = "macos")]
        {
            let _ = handle;
            if let Some(native) = cx.global::<Self>().native.clone() {
                cx.foreground_executor()
                    .spawn(async move { platform::hide(&native) })
                    .detach();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            // GPUI can't hide a window elsewhere: it closes, and the next
            // show makes it again where capture places it.
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    fn pick_files(reply: oneshot::Sender<Option<Vec<String>>>, cx: &mut App) {
        if let Some(stubbed) = cx.try_global::<Stubs>().and_then(|s| s.pick_files.clone()) {
            let _ = reply.send(Some(stubbed));
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let parent = cx.global::<Self>().native.clone();
            // AppKit runs the sheet's events itself: outside any GPUI update.
            cx.foreground_executor()
                .spawn(async move {
                    platform::pick_files(parent, move |paths| {
                        let _ = reply.send(paths);
                    });
                })
                .detach();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: None,
            });
            cx.foreground_executor()
                .spawn(async move {
                    let paths = picked.await.ok().and_then(Result::ok).flatten().map(|paths| {
                        paths
                            .into_iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                    });
                    let _ = reply.send(paths);
                })
                .detach();
        }
    }
}
