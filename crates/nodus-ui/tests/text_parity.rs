//! Text laid out by the patched GPUI as Chromium lays it out (track
//! gpui-text): checks against the platform's real text system, so it runs
//! on the main thread without the test harness (the Mac platform asks for
//! it). Off a Mac there is nothing to check yet.

#![allow(clippy::print_stdout, reason = "reports each check as the test harness would")]

fn main() {
    #[cfg(target_os = "macos")]
    mac::run();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::Arc;

    use gpui_kit::{Font, FontRun, FontStyle, FontWeight, PlatformTextSystem, px};

    type Check = fn(&Arc<dyn PlatformTextSystem>);

    pub fn run() {
        let platform = gpui_kit::platform::current_platform(true);
        let text_system = platform.text_system();
        let checks: &[(&str, Check)] = &[("system font weights are Chromium's", system_font_weights)];
        for (name, check) in checks {
            check(&text_system);
            println!("test {name} ... ok");
        }
    }

    fn system_font(weight: f32) -> Font {
        Font {
            family: nodus_ui::theme::fonts::UI.into(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight(weight),
            style: FontStyle::Normal,
        }
    }

    /// Every CSS weight of the system font is a font of its own, set on the
    /// `wght` axis as Chromium sets it (500 was Regular), and the glyphs come
    /// back with that font's id, so they are drawn at that weight too.
    fn system_font_weights(text_system: &Arc<dyn PlatformTextSystem>) {
        let text = "The quick brown fox jumps over the lazy dog";
        let mut previous = 0.;
        let mut ids = Vec::new();
        for weight in [400., 500., 600., 700.] {
            let font_id = text_system.font_id(&system_font(weight)).expect("system font");
            assert!(!ids.contains(&font_id), "{weight} has a font of its own");
            ids.push(font_id);
            let run = FontRun {
                font_id,
                len: text.len(),
            };
            let layout = text_system.layout_line(text, px(13.), &[run]);
            let width = f32::from(layout.width);
            assert!(
                width > previous + 2.,
                "{weight} ({width}) is wider than the weight below ({previous})"
            );
            previous = width;
            assert_eq!(layout.runs.len(), 1);
            assert_eq!(layout.runs[0].font_id, font_id, "{weight} is drawn at its weight");
        }
    }
}
