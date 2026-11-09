//! The terminal's font and cell size, as xterm.js 6's WebGL renderer
//! measured them in 0.9.4 (`CharSizeService`, `WebglRenderer._updateDimensions`):
//! 13 px SF Mono (Cascadia Mono, else Consolas, on Windows), the cell as wide
//! as the `W` advance floored to device pixels and 1.2 × the character
//! height, the glyphs sat on an ideographic baseline at the bottom of the
//! character box, centred in the cell.
//!
//! Chromium measures the character height from the font's ascent and
//! descent each rounded to whole CSS pixels, and places the ideographic
//! baseline at the descent rounded at the device font size. At 2× that
//! gives SF Mono a 15 × 36 device-pixel cell (7.5 × 18 px) with the
//! alphabetic baseline 27 device pixels below the cell's top, as the goldens
//! show.

use gpui::{Font, FontStyle, FontWeight, Pixels, Window, px};
use nodus_terminal::fit::{self, CellMetrics, FONT_SIZE};

use crate::theme::fonts;

/// `fontFamily: 'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', Consolas, monospace`.
pub fn term_font() -> Font {
    #[allow(unused_mut)]
    let mut font = gpui::font(fonts::MONO);
    #[cfg(not(target_os = "macos"))]
    {
        font.fallbacks = Some(gpui::FontFallbacks::from_fonts(vec!["Consolas".into()]));
    }
    font
}

/// The font for a cell: weight 700 when bold (`fontWeightBold`), italic when asked.
pub fn cell_font(bold: bool, italic: bool) -> Font {
    let mut font = term_font();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    if italic {
        font.style = FontStyle::Italic;
    }
    font
}

/// Everything the element needs to place cells and glyphs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TermMetrics {
    /// Device pixels per logical pixel.
    pub dpr: f32,
    pub cell: CellMetrics,
    /// The glyphs' alphabetic baseline, in device pixels below the cell's top.
    pub baseline: f32,
}

impl TermMetrics {
    pub fn font_size() -> Pixels {
        px(FONT_SIZE as f32)
    }

    /// A cell's width in logical pixels.
    pub fn cell_w(&self) -> f32 {
        self.cell.css_cell_width as f32
    }

    /// A cell's height in logical pixels.
    pub fn cell_h(&self) -> f32 {
        self.cell.css_cell_height as f32
    }

    pub fn device_cell_w(&self) -> f32 {
        self.cell.device_cell_width as f32
    }

    pub fn device_cell_h(&self) -> f32 {
        self.cell.device_cell_height as f32
    }
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// The metrics for a font whose `W` advances `advance` px and whose ascent
/// and descent are `ascent` and `descent` px at 13 px, on a display of `dpr`.
pub fn compute(advance: f64, ascent: f64, descent: f64, dpr: f64) -> TermMetrics {
    // Chromium rounds the font's ascent and descent to whole pixels.
    let height = js_round(ascent) + js_round(descent);
    let cell = fit::cell_metrics(advance, height, dpr);
    // `fillText(…, charHeight)` with the ideographic baseline, which sits on
    // the descent line of the font at the device size.
    let baseline = cell.device_char_top + cell.device_char_height - js_round(descent * dpr);
    TermMetrics {
        dpr: dpr as f32,
        cell,
        baseline: baseline as f32,
    }
}

thread_local! {
    static CACHE: std::cell::RefCell<Option<TermMetrics>> = const { std::cell::RefCell::new(None) };
}

/// The terminal's metrics in this window (measured once per scale factor).
pub fn metrics(window: &Window) -> TermMetrics {
    let dpr = window.scale_factor();
    if let Some(m) = CACHE.with(|c| *c.borrow()).filter(|m| m.dpr == dpr) {
        return m;
    }
    let ts = window.text_system();
    let font_id = ts.resolve_font(&term_font());
    let size = TermMetrics::font_size();
    let advance = ts.advance(font_id, size, 'W').map_or(7.8, |s| f32::from(s.width));
    let ascent = f32::from(ts.ascent(font_id, size));
    let descent = f32::from(ts.descent(font_id, size)).abs();
    let m = compute(f64::from(advance), f64::from(ascent), f64::from(descent), f64::from(dpr));
    CACHE.with(|c| *c.borrow_mut() = Some(m));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_sf_mono_cell_matches_the_goldens() {
        // SF Mono at 13 px: 'W' advances 7.8 px, ascent ≈ 12.4, descent ≈ 2.9.
        let m = compute(7.8, 12.4, 2.9, 2.0);
        assert_eq!((m.cell.device_cell_width, m.cell.device_cell_height), (15.0, 36.0));
        assert_eq!((m.cell_w(), m.cell_h()), (7.5, 18.0));
        assert_eq!(m.cell.device_char_top, 3.0);
        assert_eq!(m.baseline, 27.0);
        let m = compute(7.8, 12.4, 2.9, 1.0);
        assert_eq!((m.cell_w(), m.cell_h()), (7.0, 18.0));
    }
}
