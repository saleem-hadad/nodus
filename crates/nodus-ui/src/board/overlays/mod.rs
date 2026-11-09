//! The board's overlays (spec 11): the ink layer and its dock, the prompt
//! bar with the selection bar and toast stacked above it, ⌘K search and the
//! merge dialog. The canvas holds their state in [`Overlays`] and draws them
//! from its render; the board's side of what they do is in `board/bar.rs`.

pub mod bar;
pub mod icons;
pub mod ink;
pub mod merge;
pub mod search;
pub mod selection;

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, BoxShadow, IntoElement, ParentElement, Rgba, Styled, Transformation,
    div, percentage, px,
};

use crate::components::icon::{Icon, icon};
use crate::theme::tokens::{Layer, TEXT_XS, Tokens, shadow, shadows};

/// Nodes at this z or above (the open archive panel and its results,
/// `RAISED_Z`) draw over the ink (INKUI-025).
pub const INK_Z_BELOW: i32 = super::scene::RAISED_Z;

/// The overlays' state on the board canvas: everything is per board mount,
/// so switching project starts each one over (BAR-041, INKUI-021).
#[derive(Default)]
pub struct Overlays {
    pub(crate) ink: ink::Ink,
    pub(crate) bar: bar::Bar,
    pub(crate) search: search::Search,
    pub(crate) merge: Option<merge::MergeDialog>,
}

/// An empty layer the size of its parent that records the parent's box
/// in window coordinates each frame (for presses outside a popover).
pub(crate) fn bounds_probe(
    canvas: gpui::WeakEntity<crate::board::canvas::BoardCanvas>,
    put: fn(&mut crate::board::canvas::BoardCanvas, gpui::Bounds<gpui::Pixels>),
) -> AnyElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, _, cx| {
            if let Some(c) = canvas.upgrade() {
                c.update(cx, |c, _| put(c, bounds));
            }
        },
    )
    .absolute()
    .size_full()
    .into_any_element()
}

/// Whether the text is empty once trimmed as JavaScript trims.
pub(crate) fn trimmed_empty(text: &str) -> bool {
    nodus_domain::js::trim(text).is_empty()
}

/// A key cap (`kbd` in board.css): 11px, 17px tall, at least 18px wide.
pub(crate) fn kbd(t: &Tokens, key: &str) -> AnyElement {
    div()
        .flex_none()
        .min_w(px(18.))
        .px(px(5.))
        .mx(px(1.))
        .rounded(px(4.))
        .border(px(0.5))
        .border_color(t.border_strong)
        .bg(t.bg_elevated)
        .shadow(vec![shadow(0., 1., 0., 0., t.border)])
        .text_size(TEXT_XS)
        .line_height(px(17.))
        .flex()
        .justify_center()
        .text_color(t.text_secondary)
        .child(key.to_owned())
        .into_any_element()
}

/// Two shadow tokens mixed by `t` (a `box-shadow` transition).
pub(crate) fn mix_layers(from: &[Layer; 2], to: &[Layer; 2], t: f32) -> Vec<BoxShadow> {
    if t <= 0.0 {
        return shadows(from);
    }
    if t >= 1.0 {
        return shadows(to);
    }
    let mix = |a: f32, b: f32| a + (b - a) * t;
    let layers: Vec<Layer> = from
        .iter()
        .zip(to.iter())
        .map(|(a, b)| {
            (
                mix(a.0, b.0),
                mix(a.1, b.1),
                mix(a.2, b.2),
                mix(a.3, b.3),
                Rgba {
                    r: mix(a.4.r, b.4.r),
                    g: mix(a.4.g, b.4.g),
                    b: mix(a.4.b, b.4.b),
                    a: mix(a.4.a, b.4.a),
                },
            )
        })
        .collect();
    shadows(&layers)
}

/// `Loader2` turning once per 0.9s (`.spin`); still while parity shots are
/// taken.
pub(crate) fn spinner(id: &'static str, size: f32, stroke: f32, color: Rgba, cx: &App) -> AnyElement {
    let glyph = icon(Icon::Loader2, size, stroke, color);
    if crate::parity::frozen(cx) {
        return glyph.into_any_element();
    }
    glyph
        .with_animation(id, Animation::new(Duration::from_millis(900)).repeat(), |svg, delta| {
            svg.with_transformation(Transformation::rotate(percentage(delta)))
        })
        .into_any_element()
}
