//! Compares GPUI's line widths with Chromium's (track gpui-text).
//!
//! usage: `cargo run -p nodus-ui --example text_metrics -- <chromium.json>`
//!
//! The JSON is a list of `{size, weight, text, width}`, as Electron 44.5.1
//! (0.9.4's runtime) measured them with `Range.getBoundingClientRect()` in
//! the system UI font. Prints every row that differs by more than 0.02px,
//! and the font's ascent and descent at each size.

use gpui_kit::{Font, FontRun, FontStyle, FontWeight, px};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: text_metrics <chromium.json>"))?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let platform = gpui_kit::platform::current_platform(true);
    let text_system = platform.text_system();
    let mut worst = 0f32;
    for row in rows {
        let number = |key: &str| row[key].as_f64().unwrap_or_default() as f32;
        let (size, weight, chromium) = (number("size"), number("weight"), number("width"));
        let text = row["text"].as_str().unwrap_or_default();
        let font = Font {
            family: nodus_ui::theme::fonts::UI.into(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight(weight),
            style: FontStyle::Normal,
        };
        let id = text_system.font_id(&font)?;
        let run = FontRun { font_id: id, len: text.len() };
        let width = f32::from(text_system.layout_line(text, px(size), &[run]).width);
        let diff = width - chromium;
        worst = worst.max(diff.abs());
        if diff.abs() > 0.02 {
            println!("{size:>5} {weight:>4} {text:<45} gpui {width:>9.4} chromium {chromium:>9.4} diff {diff:>+7.3}");
        }
        if text == "a" && weight == 400.0 {
            let m = text_system.font_metrics(id);
            let scale = size / m.units_per_em as f32;
            println!("{size:>5} ascent {:.4} descent {:.4}", m.ascent * scale, -m.descent * scale);
        }
    }
    println!("worst {worst:.4}px");
    Ok(())
}
