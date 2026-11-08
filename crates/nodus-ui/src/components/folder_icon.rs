//! The macOS-style folder of `FolderIcon.tsx` (UI-177…UI-182).
//!
//! GPUI's `svg()` paints one colour, and the folder has gradients, a sheen
//! and a drop shadow, so the exact SVG is rasterized with resvg at the
//! device size and cached per colour, size, scale and shadow. The embossed
//! letter is text (the system font, as Chromium drew it), laid over the
//! picture in two copies; an icon glyph is two lucide icons.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, FontWeight, Global, ImageSource, IntoElement, ParentElement, RenderImage, RenderOnce, Rgba, SharedString,
    Styled, Window, div, img, px,
};
use nodus_domain::FolderColor;

use crate::components::icon::{Icon, icon};
use crate::theme::fonts;
use crate::theme::tokens::hex;

/// The hues of a folder colour, the same in light and dark (UI-182).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hues {
    pub back_top: Rgba,
    pub back_bottom: Rgba,
    /// Also the swatch's top.
    pub front_top: Rgba,
    /// Also the swatch's bottom.
    pub front_bottom: Rgba,
    /// The embossed glyph's colour.
    pub glyph: Rgba,
}

const fn hues(back_top: u32, back_bottom: u32, front_top: u32, front_bottom: u32, glyph: u32) -> Hues {
    Hues {
        back_top: hex(back_top),
        back_bottom: hex(back_bottom),
        front_top: hex(front_top),
        front_bottom: hex(front_bottom),
        glyph: hex(glyph),
    }
}

/// `FOLDER_HUES`.
pub const fn folder_hues(color: FolderColor) -> Hues {
    match color {
        FolderColor::Blue => hues(0x3aa8f0, 0x1d8be0, 0x6dd0fc, 0x2da4f3, 0x1478c8),
        FolderColor::Purple => hues(0xa67ae8, 0x8b5bd9, 0xc8a2f7, 0xa77beb, 0x7447c4),
        FolderColor::Pink => hues(0xf06fa8, 0xe0508f, 0xfba0c8, 0xf074aa, 0xc83a78),
        FolderColor::Red => hues(0xf2615a, 0xe0443e, 0xff9189, 0xf76760, 0xc4302b),
        FolderColor::Orange => hues(0xfa9d3a, 0xee8420, 0xffc374, 0xfba343, 0xd06d10),
        FolderColor::Yellow => hues(0xf5c73a, 0xe6b020, 0xffe27a, 0xf9cb45, 0xbe8e0c),
        FolderColor::Green => hues(0x5cc96a, 0x3db352, 0x93e19c, 0x5dcd6b, 0x2a9340),
        FolderColor::Graphite => hues(0x8e8e93, 0x76767b, 0xbdbdc2, 0x9a9aa0, 0x636368),
    }
}

/// What a folder carries on its front panel.
#[derive(Debug, Clone, PartialEq)]
pub enum Glyph {
    /// A letter (only its first character shows, upper-cased).
    Letter(String),
    /// A lucide icon (Plus, FolderPlus).
    Icon(Icon),
}

/// A folder `size` px square.
#[derive(IntoElement)]
pub struct FolderIcon {
    size: f32,
    color: FolderColor,
    glyph: Option<Glyph>,
    /// `filter: saturate(0.4)` (faded folders).
    saturation: Option<f32>,
}

/// `<FolderIcon size color glyph>`. A missing colour is blue (UI-178).
pub fn folder_icon(size: f32, color: Option<FolderColor>) -> FolderIcon {
    FolderIcon {
        size,
        color: color.unwrap_or(FolderColor::Blue),
        glyph: None,
        saturation: None,
    }
}

impl FolderIcon {
    pub fn glyph(mut self, glyph: Option<Glyph>) -> Self {
        self.glyph = glyph;
        self
    }

    pub fn saturate(mut self, amount: f32) -> Self {
        self.saturation = Some(amount);
        self
    }
}

/// The folder's SVG at `size`, as FolderIcon renders it (the glyph apart).
pub fn folder_svg(color: FolderColor, shadow: bool) -> String {
    let h = folder_hues(color);
    let c = |c: Rgba| {
        format!(
            "#{:02x}{:02x}{:02x}",
            (c.r * 255.0).round() as u8,
            (c.g * 255.0).round() as u8,
            (c.b * 255.0).round() as u8
        )
    };
    let filter = if shadow { r#" filter="url(#shadow)""# } else { "" };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="back" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{bt}"/><stop offset="1" stop-color="{bb}"/>
    </linearGradient>
    <linearGradient id="front" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{ft}"/><stop offset="1" stop-color="{fb}"/>
    </linearGradient>
    <linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.55"/>
      <stop offset="0.05" stop-color="#fff" stop-opacity="0.12"/>
      <stop offset="0.5" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="130%">
      <feDropShadow dx="0" dy="1.5" stdDeviation="1.6" flood-color="#000" flood-opacity="0.2"/>
    </filter>
  </defs>
  <g{filter}>
    <path d="M10 30C10 23.4 15.4 18 22 18H46C49.5 18 52.6 19.6 54.8 22.3L58 26.2C59.8 28.3 62.4 29.5 65.2 29.5H108C114.6 29.5 118 33 118 39.5V100C118 106.6 114.6 110 108 110H20C13.4 110 10 106.6 10 100Z" fill="url(#back)"/>
    <rect x="10" y="39" width="108" height="71" rx="9" fill="url(#front)"/>
    <rect x="10" y="39" width="108" height="71" rx="9" fill="url(#sheen)"/>
    <path d="M19 39.8H109" stroke="#fff" stroke-opacity="0.6" stroke-width="1.2" stroke-linecap="round"/>
  </g>
</svg>"##,
        bt = c(h.back_top),
        bb = c(h.back_bottom),
        ft = c(h.front_top),
        fb = c(h.front_bottom),
    )
}

/// CSS `saturate(s)` on one straight-alpha sRGB pixel.
pub fn saturate(rgb: [f32; 3], s: f32) -> [f32; 3] {
    let [r, g, b] = rgb;
    [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
    ]
    .map(|v| v.clamp(0.0, 1.0))
}

fn saturate_color(c: Rgba, s: Option<f32>) -> Rgba {
    match s {
        Some(s) => {
            let [r, g, b] = saturate([c.r, c.g, c.b], s);
            Rgba { r, g, b, a: c.a }
        }
        None => c,
    }
}

/// Rasterizes the folder `px` device pixels square, in straight-alpha BGRA.
pub fn rasterize(color: FolderColor, shadow: bool, px_size: u32, saturation: Option<f32>) -> Option<Vec<u8>> {
    let svg = folder_svg(color, shadow);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px_size, px_size)?;
    let scale = px_size as f32 / 128.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut out = Vec::with_capacity((px_size * px_size * 4) as usize);
    for p in pixmap.pixels() {
        let c = p.demultiply();
        let mut rgb = [c.red(), c.green(), c.blue()].map(|v| f32::from(v) / 255.0);
        if let Some(s) = saturation {
            rgb = saturate(rgb, s);
        }
        let [r, g, b] = rgb.map(|v| (v * 255.0).round() as u8);
        out.extend_from_slice(&[b, g, r, c.alpha()]);
    }
    Some(out)
}

#[derive(Default)]
struct Rasters(HashMap<(FolderColor, bool, u32, Option<u32>), Arc<RenderImage>>);

impl Global for Rasters {}

fn raster(
    cx: &mut App,
    color: FolderColor,
    shadow: bool,
    px_size: u32,
    saturation: Option<f32>,
) -> Option<Arc<RenderImage>> {
    let key = (color, shadow, px_size, saturation.map(|s| (s * 1000.0) as u32));
    if let Some(image) = cx.try_global::<Rasters>().and_then(|r| r.0.get(&key)) {
        return Some(image.clone());
    }
    let pixels = rasterize(color, shadow, px_size, saturation)?;
    let buffer = image::RgbaImage::from_raw(px_size, px_size, pixels)?;
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    cx.default_global::<Rasters>().0.insert(key, image.clone());
    Some(image)
}

impl RenderOnce for FolderIcon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = self.size;
        // UI-179: small folders have no shadow and no glyph.
        let small = size < 32.0;
        let px_size = (size * window.scale_factor()).round().max(1.0) as u32;
        let image = raster(cx, self.color, !small, px_size, self.saturation);
        let unit = size / 128.0;
        let hues = folder_hues(self.color);
        let glyph_color = saturate_color(hues.glyph, self.saturation);
        let white = saturate_color(hex(0xffffff), self.saturation);

        div()
            .relative()
            .flex_none()
            .size(px(size))
            .when_some(image, |d, image| {
                d.child(img(ImageSource::Render(image)).size(px(size)))
            })
            .when(!small, |d| match self.glyph {
                Some(Glyph::Letter(letter)) => {
                    let letter = glyph_letter(&letter);
                    let font_size = 34.0 * unit;
                    // "central" baseline: the middle of ascent and descent sits on y.
                    let line = font_size * 2.0;
                    let text = |y: f32, color: Rgba| {
                        div()
                            .absolute()
                            .left(px(0.))
                            .w(px(size))
                            .top(px(y * unit - line / 2.0))
                            .h(px(line))
                            .flex()
                            .justify_center()
                            .font_family(fonts::UI)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(font_size))
                            .line_height(px(line))
                            .text_color(color)
                            .child(SharedString::from(letter.clone()))
                    };
                    d.child(text(76.2, Rgba { a: 0.35, ..white }))
                        .child(text(75.0, Rgba { a: 0.62, ..glyph_color }))
                }
                Some(Glyph::Icon(i)) => {
                    let box_size = 30.0 * unit;
                    let at = |y: f32, color: Rgba| {
                        div()
                            .absolute()
                            .left(px(49.0 * unit))
                            .top(px(y * unit))
                            .child(icon(i, box_size, 2.4, color))
                    };
                    d.child(at(61.2, Rgba { a: 0.35, ..white }))
                        .child(at(60.0, Rgba { a: 0.62, ..glyph_color }))
                }
                None => d,
            })
    }
}

/// The letter a glyph string shows: its first character, upper-cased (a
/// character whose capital is longer, like `ß`, shows it whole).
pub fn glyph_letter(glyph: &str) -> String {
    glyph
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}

/// `projectGlyph`: the first Unicode letter or digit in a project's name
/// (UI-295).
pub fn project_glyph(name: &str) -> Option<Glyph> {
    name.chars()
        .find(|c| c.is_alphabetic() || c.is_numeric())
        .map(|c| Glyph::Letter(c.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_blue_and_keeps_hues_per_colour() {
        assert_eq!(folder_icon(18.0, None).color, FolderColor::Blue);
        assert_eq!(folder_hues(FolderColor::Green).front_top, hex(0x93e19c));
    }

    #[test]
    fn picks_the_first_letter_or_digit() {
        assert_eq!(project_glyph("storefront"), Some(Glyph::Letter("s".into())));
        assert_eq!(project_glyph("--3d"), Some(Glyph::Letter("3".into())));
        assert_eq!(project_glyph("---"), None);
        assert_eq!(project_glyph("éclair"), Some(Glyph::Letter("é".into())));
        assert_eq!(glyph_letter("s"), "S");
        assert_eq!(glyph_letter("ß"), "SS");
    }

    #[test]
    fn rasterizes_the_folder() {
        let pixels = rasterize(FolderColor::Blue, true, 64, None).unwrap();
        assert_eq!(pixels.len(), 64 * 64 * 4);
        // The middle of the front panel is opaque blue.
        let at = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        let mid = at(32, 45);
        assert_eq!(mid[3], 255);
        assert!(mid[0] > mid[2], "BGRA, blue over red: {mid:?}");
        // The corner is empty.
        assert_eq!(at(0, 0)[3], 0);
        let grey = rasterize(FolderColor::Blue, true, 64, Some(0.0)).unwrap();
        let g = &grey[(45 * 64 + 32) * 4..(45 * 64 + 32) * 4 + 3];
        assert!(g[0].abs_diff(g[1]) <= 1 && g[1].abs_diff(g[2]) <= 1);
    }

    #[test]
    fn saturates_as_css_does() {
        assert_eq!(saturate([1.0, 0.0, 0.0], 1.0), [1.0, 0.0, 0.0]);
        let grey = saturate([1.0, 0.0, 0.0], 0.0);
        assert!((grey[0] - 0.213).abs() < 1e-6 && (grey[1] - 0.213).abs() < 1e-6);
    }
}
