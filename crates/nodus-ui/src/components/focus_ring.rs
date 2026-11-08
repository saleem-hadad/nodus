//! The keyboard focus ring (`:focus-visible`, UI-010): a 2px `--focus-ring`
//! outline 2px outside the border box, its corners the element's radius plus
//! the offset.

use gpui::{Corners, Div, Pixels, Rgba, Styled, div, px};

/// A 2px ring `offset` outside its parent (which must be `relative`),
/// following the parent's `corners`.
pub fn focus_ring(corners: Corners<Pixels>, offset: Pixels, color: Rgba) -> Div {
    ring(corners, offset, px(2.), color)
}

/// A ring of `width`, `offset` outside its parent.
pub fn ring(corners: Corners<Pixels>, offset: Pixels, width: Pixels, color: Rgba) -> Div {
    let reach = offset + width;
    div()
        .absolute()
        .top(-reach)
        .left(-reach)
        .right(-reach)
        .bottom(-reach)
        .rounded_tl(corners.top_left + reach)
        .rounded_tr(corners.top_right + reach)
        .rounded_bl(corners.bottom_left + reach)
        .rounded_br(corners.bottom_right + reach)
        .border(width)
        .border_color(color)
}
