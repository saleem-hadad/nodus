//! Every surface that blurs what's behind it goes through [`frosted`]: the
//! context menu (30px, saturate 1.8), the floating rail (24px, 1.8), the
//! modal backdrop (3px), the folder drop overlay (2px), and the title bar
//! (24px, 1.6, invisible: nothing but the window is behind it).
//!
//! Stock GPUI can't blur behind an element; the gpui-fork track adds
//! `backdrop_blur` and `backdrop_matrix(saturate)` (D13). Until it lands this
//! approximates: a wide blur over the app's mostly plain surfaces averages
//! the backdrop to the colour of the surface underneath (`behind`), so a
//! wide-blur surface is painted as its colour over that, opaque; a narrow
//! blur (a few px) keeps the backdrop readable, so it stays translucent.
//! Switching to the real blur is this one function.

use gpui::{Rgba, Styled};

/// Blurs wider than this read as the average colour behind them.
const WIDE: f32 = 10.0;

/// A frosted surface: `surface` over a `blur` px backdrop blur, saturated by
/// `saturation`, with mostly `behind` beneath it.
pub fn frosted<E: Styled>(element: E, surface: Rgba, blur: f32, saturation: f32, behind: Rgba) -> E {
    let _ = saturation;
    if blur < WIDE {
        return element.bg(surface);
    }
    element.bg(over(surface, behind))
}

/// `top` composited over an opaque `below`.
pub fn over(top: Rgba, below: Rgba) -> Rgba {
    let a = top.a;
    Rgba {
        r: top.r * a + below.r * (1.0 - a),
        g: top.g * a + below.g * (1.0 - a),
        b: top.b * a + below.b * (1.0 - a),
        a: 1.0,
    }
}
