//! Every surface that blurs what's behind it goes through [`frosted`]: the
//! context menu (30px, saturate 1.8), the floating rail (24px, 1.8), the
//! modal backdrop (3px), the folder drop overlay (2px), and the title bar
//! (24px, 1.6, invisible: nothing but the window is behind it).
//!
//! CSS `backdrop-filter: blur(Npx) saturate(S)` is the patched GPUI's
//! `.backdrop_blur(px(N)).backdrop_saturate(S)` (D13): the backdrop is
//! blurred inside the element's border box, mirrored at its edges as
//! Chromium does, and the surface colour is drawn over it.
//!
//! A frosted surface is see-through, so its shadows are drawn only outside
//! it (`shadow_outside`, PATCHES 14), as CSS clips an outer `box-shadow`:
//! otherwise GPUI draws them under the whole box, where they'd darken the
//! fill and be blurred into the backdrop.

use gpui::{Rgba, Styled, px};

/// A frosted surface: `surface` over a `blur` px backdrop blur, saturated by
/// `saturation`.
pub fn frosted<E: Styled>(element: E, surface: Rgba, blur: f32, saturation: f32) -> E {
    let element = element.backdrop_blur(px(blur)).shadow_outside();
    let element = if (saturation - 1.0).abs() > f32::EPSILON {
        element.backdrop_saturate(saturation)
    } else {
        element
    };
    element.bg(surface)
}
