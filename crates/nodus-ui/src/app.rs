//! Starting the app (`src/main/index.ts`; spec 01 PLAT-009…PLAT-036): the
//! theme before the window, the menu, the main window, reopening it from the
//! Dock, and flushing state on quit.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::App;
use nodus_core::{Dialogs, EnvSource, Nodus, NodusConfig, PickFolder};

use crate::components::icon::Assets;
use crate::shell::{menus, window};
use crate::theme;

/// The windowed dialogs the core asks for: the folder picker runs on the
/// main thread, as a sheet on the main window.
struct WindowDialogs {
    requests: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<Option<PathBuf>>>,
}

impl Dialogs for WindowDialogs {
    fn pick_folder(&self) -> PickFolder {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sent = self.requests.send(tx).is_ok();
        Box::pin(async move {
            if !sent {
                return None;
            }
            rx.await.ok().flatten()
        })
    }
}

/// Runs the app until it quits. `user_data` is the data folder (PLAT-003,
/// PLAT-004).
pub fn run(user_data: PathBuf) -> anyhow::Result<()> {
    // PLAT-002: Windows' AppUserModelID, before any window or notification.
    nodus_platform::app::set_app_user_model_id();
    // The core's own work (the login shell, later the host) runs on tokio.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("nodus-core")
        .build()?;
    let (requests, mut picks) = tokio::sync::mpsc::unbounded_channel();
    let nodus = Nodus::start(NodusConfig {
        user_data,
        env: EnvSource::Capture,
        dialogs: Some(Arc::new(WindowDialogs { requests })),
        runtime: runtime.handle().clone(),
        home: std::env::home_dir().unwrap_or_default(),
        // The rest as a headless core has them until the app wires its
        // window, notifications and host in.
        ..NodusConfig::new(PathBuf::new(), runtime.handle().clone())
    })?;
    // PLAT-010: adopt a running session host before the window opens.
    let _ = runtime.block_on(nodus.launch());

    // UI-004: the saved settings, read before the window exists.
    let settings = runtime.block_on(nodus.settings().get());
    // PLAT-066: settings are applied after every update, from whichever
    // thread; the window's side of it runs on the main thread.
    let (applied_tx, mut applied) = tokio::sync::mpsc::unbounded_channel();
    nodus.on_settings_applied(Arc::new(move |settings| {
        let _ = applied_tx.send(settings.clone());
    }));

    let core_runtime = runtime.handle().clone();
    let app = gpui::application().with_assets(Assets);
    // PLAT-030: a Dock click with no window opens one.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            window::open_main_window(cx);
        }
    });
    app.run(move |cx: &mut App| {
        gpui::init(cx);
        theme::fonts::register(cx);
        cx.set_global(crate::board::CoreRuntime(core_runtime));
        crate::board::init(cx);
        cx.set_global(window::Core(nodus.clone()));
        theme::apply_preference(settings.theme, cx);
        menus::install_menus(&settings, cx);
        menus::bind_shortcuts(cx);
        menus::register_actions(&nodus, cx);
        cx.spawn(async move |cx| {
            while let Some(settings) = applied.recv().await {
                cx.update(|cx| {
                    theme::apply_preference(settings.theme, cx);
                    menus::install_menus(&settings, cx);
                    if let Some(handle) = window::main_window(cx) {
                        let _ = handle.update(cx, |_, window, cx| theme::sync_window(window, cx));
                    }
                });
            }
        })
        .detach();

        // The folder picker, on the main thread.
        cx.spawn(async move |cx| {
            while let Some(reply) = picks.recv().await {
                cx.update(|cx| pick_folder(reply, cx));
            }
        })
        .detach();

        // PLAT-032: quitting writes a pending save now.
        let flush = nodus.clone();
        cx.on_app_quit(move |_| {
            flush.shutdown();
            async {}
        })
        .detach();
        // PLAT-029, PLAT-031: closing the window keeps the app on a Mac;
        // elsewhere the last window closing quits.
        cx.on_window_closed(|cx, _| {
            if cfg!(not(target_os = "macos")) && cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        window::open_main_window(cx);
        crate::automation::start(cx);
    });
    drop(runtime);
    Ok(())
}

/// Shows the folder picker and answers `reply` with the folder.
fn pick_folder(reply: tokio::sync::oneshot::Sender<Option<PathBuf>>, cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        let parent = window::main_window(cx)
            .and_then(|h| h.update(cx, |_, window, _| crate::mac::ns_window(window)).ok())
            .flatten();
        // Outside any GPUI update: AppKit runs the sheet's events itself.
        cx.defer(move |_| {
            crate::mac::pick_folder(parent, move |path| {
                let _ = reply.send(path);
            });
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add Project".into()),
        });
        cx.foreground_executor()
            .spawn(async move {
                let path = picked
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten()
                    .and_then(|p| p.into_iter().next());
                let _ = reply.send(path);
            })
            .detach();
    }
}
