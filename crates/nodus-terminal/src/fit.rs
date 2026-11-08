//! The terminal's geometry as xterm.js 6 computes it: the cell size from the
//! font (`WebglRenderer._updateDimensions`) and how many columns and rows fit
//! a container (`FitAddon.proposeDimensions`, TERM-023). The UI measures the
//! font and the container; the arithmetic, and its rounding, is here.

/// registry.ts: `fontSize: 13`, `lineHeight: 1.2`.
pub const FONT_SIZE: f64 = 13.0;
pub const LINE_HEIGHT: f64 = 1.2;

/// The strip xterm keeps for its scrollbar (`DEFAULT_SCROLL_BAR_WIDTH`), left
/// out of the width a fit uses because the terminal has scrollback (TERM-075).
pub const SCROLLBAR_WIDTH: f64 = 14.0;

/// xterm's smallest grid.
pub const MIN_COLS: usize = crate::terminal::MIN_COLS;
pub const MIN_ROWS: usize = crate::terminal::MIN_ROWS;

/// A cell's size in device pixels and CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    pub dpr: f64,
    pub device_char_width: f64,
    pub device_char_height: f64,
    pub device_cell_width: f64,
    pub device_cell_height: f64,
    /// The glyph's offset from the top of its cell, in device pixels (centred).
    pub device_char_top: f64,
    pub css_cell_width: f64,
    pub css_cell_height: f64,
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// The cell for a font whose `W` advances `advance` CSS px and whose
/// ascent + descent is `height` CSS px (at 13 px), at `dpr` device pixels
/// per CSS pixel: width floored and height ceiled to device pixels, the cell
/// 1.2 × the character height (floored), the glyph centred in it.
pub fn cell_metrics(advance: f64, height: f64, dpr: f64) -> CellMetrics {
    let device_char_width = (advance * dpr).floor();
    let device_char_height = (height * dpr).ceil();
    let device_cell_height = (device_char_height * LINE_HEIGHT).floor();
    let device_char_top = js_round((device_cell_height - device_char_height) / 2.0);
    // letterSpacing 0.
    let device_cell_width = device_char_width;
    CellMetrics {
        dpr,
        device_char_width,
        device_char_height,
        device_cell_width,
        device_cell_height,
        device_char_top,
        css_cell_width: device_cell_width / dpr,
        css_cell_height: device_cell_height / dpr,
    }
}

impl CellMetrics {
    /// The grid's size on the page for `cols` × `rows` (`css.canvas`).
    pub fn grid_size(&self, cols: usize, rows: usize) -> (f64, f64) {
        (
            js_round(cols as f64 * self.device_cell_width / self.dpr),
            js_round(rows as f64 * self.device_cell_height / self.dpr),
        )
    }
}

/// `proposeDimensions`: the columns and rows that fit a container of
/// `width` × `height` CSS px (its layout size, which a board zoom doesn't
/// change), or `None` before the font is measured. Sizes are cut to whole
/// pixels as `parseInt` reads the computed style.
pub fn fit(width: f64, height: f64, cell: &CellMetrics) -> Option<(usize, usize)> {
    if cell.css_cell_width <= 0.0 || cell.css_cell_height <= 0.0 || !width.is_finite() || !height.is_finite() {
        return None;
    }
    let available_width = width.trunc().max(0.0) - SCROLLBAR_WIDTH;
    let available_height = height.trunc();
    let cols = (available_width / cell.css_cell_width).floor();
    let rows = (available_height / cell.css_cell_height).floor();
    let cols = if cols.is_finite() && cols > MIN_COLS as f64 {
        cols as usize
    } else {
        MIN_COLS
    };
    let rows = if rows.is_finite() && rows > MIN_ROWS as f64 {
        rows as usize
    } else {
        MIN_ROWS
    };
    Some((cols, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SF Mono at 13 px: advance 1266/2048 em, ascent + descent (1980 + 432)/2048 em.
    fn sf_mono(dpr: f64) -> CellMetrics {
        cell_metrics(13.0 * 1266.0 / 2048.0, 13.0 * 2412.0 / 2048.0, dpr)
    }

    #[test]
    fn cell_size_for_sf_mono() {
        let m = sf_mono(2.0);
        assert_eq!((m.device_char_width, m.device_char_height), (16.0, 31.0));
        assert_eq!(
            (m.device_cell_width, m.device_cell_height, m.device_char_top),
            (16.0, 37.0, 3.0)
        );
        assert_eq!((m.css_cell_width, m.css_cell_height), (8.0, 18.5));
        let m = sf_mono(1.0);
        assert_eq!((m.css_cell_width, m.css_cell_height), (8.0, 19.0));
        assert_eq!(sf_mono(2.0).grid_size(80, 24), (640.0, 444.0));
    }

    #[test]
    fn term_023_fit_keeps_the_scrollbar_strip_and_minimums() {
        let m = sf_mono(2.0);
        // (800 - 14) / 8 = 98.25 → 98; 600 / 18.5 = 32.4 → 32.
        assert_eq!(fit(800.7, 600.9, &m), Some((98, 32)));
        assert_eq!(fit(10.0, 5.0, &m), Some((2, 1)));
        assert_eq!(fit(0.0, 0.0, &m), Some((2, 1)));
        assert_eq!(fit(800.0, 600.0, &cell_metrics(0.0, 0.0, 2.0)), None);
    }
}
