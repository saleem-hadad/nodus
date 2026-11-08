//! The Settings theme tiles' miniature board windows (UI-127). Each tone pins
//! its own colours whatever the app's theme; System lays the dark one over
//! the light one, split on a diagonal. The dots on the mini cards take the
//! app's current status colours. Drawn as SVG and rasterized with resvg,
//! which clips the System split to its polygon as CSS's `clip-path` does.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use gpui::{App, Global, RenderImage, Rgba};
use nodus_domain::ThemePreference;

use crate::theme::tokens::hard::{MINI_CARD_SHADOW, MINI_DARK, MINI_LIGHT, MINI_LIGHTS, MiniTone};

/// The tile's preview size with no scrollbar beside it: (396 − 2·16) / 3
/// wide, 16:10.
pub const WIDTH: f32 = (396.0 - 32.0) / 3.0;

fn css(c: Rgba) -> String {
    format!(
        "rgba({},{},{},{})",
        (c.r * 255.0).round(),
        (c.g * 255.0).round(),
        (c.b * 255.0).round(),
        c.a
    )
}

/// One tone's window, in SVG, `w`×`h`.
fn window(tone: MiniTone, w: f32, h: f32, working: Rgba, done: Rgba) -> String {
    let mut s = String::new();
    let _ = write!(s, r#"<rect width="{w}" height="{h}" fill="{}"/>"#, css(tone.canvas));
    // The canvas's dot grid: a dot per 6×6 tile, solid to 0.6px, gone by 0.9px.
    let _ = write!(
        s,
        r#"<defs><radialGradient id="dot{0}" r="0.9" cx="3" cy="3" fx="3" fy="3" gradientUnits="userSpaceOnUse"><stop offset="0.6667" stop-color="{1}"/><stop offset="1" stop-color="{1}" stop-opacity="0"/></radialGradient><pattern id="grid{0}" width="6" height="6" patternUnits="userSpaceOnUse" x="0" y="13"><rect width="6" height="6" fill="url(#dot{0})"/></pattern></defs>"#,
        tone_id(tone),
        css(tone.dot)
    );
    let _ = write!(
        s,
        r#"<rect y="13" width="{w}" height="{}" fill="url(#grid{})"/>"#,
        h - 13.0,
        tone_id(tone)
    );
    // The title bar, its hairline and the traffic lights.
    let _ = write!(s, r#"<rect width="{w}" height="13" fill="{}"/>"#, css(tone.bar));
    let _ = write!(
        s,
        r#"<rect y="12.5" width="{w}" height="0.5" fill="{}"/>"#,
        css(tone.border)
    );
    for (i, light) in MINI_LIGHTS.iter().enumerate() {
        let _ = write!(
            s,
            r#"<circle cx="{}" cy="6.25" r="2" fill="{}"/>"#,
            6.0 + 2.0 + i as f32 * 7.0,
            css(*light)
        );
    }
    // Two cards in the canvas's 9px × 8px padding, 6px apart.
    let card_w = (w - 16.0 - 6.0) / 2.0;
    for (i, dot) in [working, done].into_iter().enumerate() {
        let x = 8.0 + i as f32 * (card_w + 6.0);
        let y = 13.0 + 9.0;
        let card_h = 5.0 + 5.0 + 4.0 + 3.0 + 4.0 + 3.0 + 5.0;
        // Shadow `0 1px 2px` (σ 1) and a 0.5px ring.
        let _ = write!(
            s,
            r#"<filter id="sh{i}{t}" x="-20%" y="-20%" width="140%" height="160%"><feGaussianBlur stdDeviation="1"/></filter><rect x="{x}" y="{}" width="{card_w}" height="{card_h}" rx="4" fill="{}" filter="url(#sh{i}{t})"/>"#,
            y + 1.0,
            css(MINI_CARD_SHADOW),
            t = tone_id(tone)
        );
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="4.5" fill="{}"/>"#,
            x - 0.5,
            y - 0.5,
            card_w + 1.0,
            card_h + 1.0,
            css(tone.border)
        );
        let _ = write!(
            s,
            r#"<rect x="{x}" y="{y}" width="{card_w}" height="{card_h}" rx="4" fill="{}"/>"#,
            css(tone.card)
        );
        let _ = write!(
            s,
            r#"<circle cx="{}" cy="{}" r="2.5" fill="{}"/>"#,
            x + 7.5,
            y + 7.5,
            css(dot)
        );
        let inner = card_w - 10.0;
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{inner}" height="3" rx="1.5" fill="{}"/>"#,
            x + 5.0,
            y + 14.0,
            css(tone.line)
        );
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="3" rx="1.5" fill="{}"/>"#,
            x + 5.0,
            y + 21.0,
            inner * 0.6,
            css(tone.line)
        );
    }
    s
}

fn tone_id(tone: MiniTone) -> &'static str {
    if tone.canvas == MINI_LIGHT.canvas { "l" } else { "d" }
}

/// The whole preview for a theme choice, as SVG.
pub fn preview_svg(theme: ThemePreference, w: f32, working: Rgba, done: Rgba) -> String {
    let h = w * 10.0 / 16.0;
    let mut body = String::new();
    if theme != ThemePreference::Dark {
        body.push_str(&window(MINI_LIGHT, w, h, working, done));
    }
    if theme != ThemePreference::Light {
        let dark = window(MINI_DARK, w, h, working, done);
        if theme == ThemePreference::System {
            let _ = write!(
                body,
                r#"<clipPath id="split"><polygon points="{},0 {w},0 {w},{h} {},{h}"/></clipPath><g clip-path="url(#split)">{dark}</g>"#,
                w * 0.62,
                w * 0.38
            );
        } else {
            body.push_str(&dark);
        }
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><clipPath id="round"><rect width="{w}" height="{h}" rx="7"/></clipPath><g clip-path="url(#round)">{body}</g></svg>"#
    )
}

/// A preview: theme, (width, scale) in hundredths, the two status colours.
type PreviewKey = (ThemePreference, (u32, u32), [u32; 2]);

#[derive(Default)]
struct Previews(HashMap<PreviewKey, Arc<RenderImage>>);

impl Global for Previews {}

fn key(c: Rgba) -> u32 {
    u32::from_be_bytes([
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
        (c.a * 255.0) as u8,
    ])
}

/// The preview rasterized at `scale`, cached.
pub fn preview_image(
    theme: ThemePreference,
    width: f32,
    working: Rgba,
    done: Rgba,
    scale: f32,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    let cache_key = (
        theme,
        ((width * scale * 100.0) as u32, (scale * 100.0) as u32),
        [key(working), key(done)],
    );
    if let Some(image) = cx.try_global::<Previews>().and_then(|p| p.0.get(&cache_key)) {
        return Some(image.clone());
    }
    let svg = preview_svg(theme, width, working, done);
    let height = width * 10.0 / 16.0;
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let (pw, ph) = ((width * scale).round() as u32, (height * scale).round() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(pw as f32 / width, ph as f32 / height),
        &mut pixmap.as_mut(),
    );
    let mut out = Vec::with_capacity((pw * ph * 4) as usize);
    for p in pixmap.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
    }
    let buffer = image::RgbaImage::from_raw(pw, ph, out)?;
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    cx.default_global::<Previews>().0.insert(cache_key, image.clone());
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::Tokens;

    #[test]
    fn draws_every_tone() {
        let t = Tokens::LIGHT;
        for theme in ThemePreference::ALL {
            let svg = preview_svg(*theme, WIDTH, t.status_working, t.status_done);
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default());
            assert!(tree.is_ok(), "{theme}: {svg}");
        }
        assert!(
            preview_svg(ThemePreference::System, WIDTH, t.status_working, t.status_done)
                .contains("clip-path=\"url(#split)\"")
        );
    }
}
