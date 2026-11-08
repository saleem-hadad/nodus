//! Every surface that blurs what's behind it goes through [`frosted`]: the
//! context menu (30px, saturate 1.8), the floating rail (24px, 1.8), the
//! modal backdrop (3px), the folder drop overlay (2px), and the title bar
//! (24px, 1.6, invisible: nothing but the window is behind it).
//!
//! Stock GPUI can't blur behind an element; the gpui-fork track adds
//! `backdrop_blur` and `backdrop_matrix(saturate)` (D13). Until it lands this
//! paints the surface's own colour, which is exactly what 0.9.4 shows over a
//! plain background and only differs where detailed content sits behind
//! (it shows through sharp instead of blurred). Switching to the real blur is
//! this one function.

use gpui::{Rgba, Styled};

/// A frosted surface: `surface` over a `blur` px backdrop blur, saturated by
/// `saturation`.
pub fn frosted<E: Styled>(element: E, surface: Rgba, blur: f32, saturation: f32) -> E {
    let _ = (blur, saturation);
    element.bg(surface)
}
