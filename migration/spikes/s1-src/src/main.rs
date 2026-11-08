//! Spike S1: GPUI (+ gpui-component) / alacritty_terminal / portable-pty / wry / floating panel.
//!
//! Usage: `s1 [window|board|panel|all]`
//!   window – step 1 only (title bar, theme, gpui-component button + input)
//!   board  – steps 1–4: board canvas with cards, terminal card + terminal panel, webview card
//!   panel  – step 5: floating capture-like panel
//!   all    – board + panel
//!
//! Env:
//!   S1_APPEARANCE=light|dark  force the window's NSAppearance (simulates the system switch per window)
//!   S1_AUTOPILOT=1            drive synthetic input through Window::dispatch_event and print results
//!   S1_NO_ACTIVATE=1          never steal focus (no makeKey / activate)
extern crate gpui_kit as gpui;

mod autopilot;
mod board;
#[cfg(target_os = "macos")]
mod mac;
mod panel;
mod rotated;
mod shell;
mod terminal;
mod theme;
mod webview;
mod zoom;

use gpui::*;

pub fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| !v.is_empty() && v != "0").unwrap_or(false)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,s1=info")).init();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let started = std::time::Instant::now();

    gpui::application()
        .with_assets(gpui::assets::Assets)
        .run(move |cx: &mut App| {
            gpui::init(cx);
            theme::init(cx);
            // Nodus' terminal font is 'SF Mono'. It isn't a public family to CoreText enumeration, but
            // the system ships it; load the file at runtime (never bundle it: Apple's license).
            if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNSMono.ttf") {
                let _ = cx.text_system().add_fonts(vec![std::borrow::Cow::Owned(bytes)]);
            }
            if env_flag("S1_FONTS") {
                let names = cx.text_system().all_font_names();
                for want in ["SF Mono", ".SF NS Mono", "SF NS Mono", "Menlo", ".AppleSystemUIFontMonospaced", "SF Pro", ".SystemUIFont"] {
                    println!("S1 FONT {want:?} available={}", names.iter().any(|n| n == want));
                }
                println!("S1 FONT matches_sf: {:?}", names.iter().filter(|n| n.contains("SF") || n.contains("Mono")).take(20).collect::<Vec<_>>());
                cx.quit();
                return;
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            match mode.as_str() {
                "window" => {
                    shell::open_main_window(shell::Content::Window, cx);
                }
                "board" => {
                    shell::open_main_window(shell::Content::Board, cx);
                }
                "panel" => {
                    panel::open_panel(cx);
                }
                _ => {
                    shell::open_main_window(shell::Content::Board, cx);
                    panel::open_panel(cx);
                }
            }
            log::info!("app ready in {:?}", started.elapsed());
            println!("S1 READY {}ms", started.elapsed().as_millis());
        });
}
