//! While Claude names a new section (BOARD-089, BOARD-090; `board.css` ›
//! `.section-glow`, `.section-name.is-naming`, `.is-revealing`): a light
//! circling the frame (a conic gradient ring, and a blurred copy as a halo),
//! `Naming…` with a light sweeping through it beside a twinkling sparkle,
//! then the name Claude gave easing in letter by letter.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ElementId, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement, Rgba,
    Styled, StyledText, Window, div,
};

use super::units::fu;
use crate::components::tween::{enter, transition};
use crate::theme::ai::{self, SPARKLE};
use crate::theme::motion;

/// One turn of the glow.
const TURN_MS: f64 = 3200.0;

/// The glow around a frame of `size` (screen px) at `zoom`: the ring, and
/// its blurred halo. `fading` once the name has landed.
pub fn glow(id: &str, zoom: f32, fading: bool, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = |part: &'static str| crate::components::child_id(ElementId::Name(id.to_owned().into()), part);
    let frozen = crate::parity::frozen(cx);
    let reduced = cx.reduce_motion();
    // glow-in: 600ms --ease-out; the fade: 700ms ease.
    let shown = enter(window, cx, key("glow-in"), Duration::from_millis(600), motion::ease_out);
    let fade = transition(
        window,
        cx,
        key("glow-fade"),
        if fading { 0.0f32 } else { 1.0 },
        Duration::from_millis(700),
        motion::ease,
    );
    let start = *window
        .use_keyed_state(key("glow-start"), cx, |_, _| Instant::now())
        .read(cx);
    let angle = if frozen || reduced {
        0.0
    } else {
        window.request_animation_frame();
        (start.elapsed().as_secs_f64() * 1000.0 % TURN_MS) / TURN_MS * 360.0
    };
    let opacity = shown * fade;
    let radius = 22.0 * zoom;
    div()
        .absolute()
        .top(fu(-1.5))
        .left(fu(-1.5))
        .right(fu(-1.5))
        .bottom(fu(-1.5))
        .group_opacity(opacity)
        .children(ai::glow_layers(angle, radius, zoom))
        .into_any_element()
}

/// `Naming…` with the sweep (frozen at its first keyframe in parity mode),
/// after the twinkling sparkle.
pub fn naming_label(id: &str, zoom: f32, scale: f32, color: Rgba, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = |part: &'static str| crate::components::child_id(ElementId::Name(id.to_owned().into()), part);
    let frozen = crate::parity::frozen(cx) || cx.reduce_motion();
    let start = *window
        .use_keyed_state(key("sweep-start"), cx, |_, _| Instant::now())
        .read(cx);
    let ms = if frozen {
        0.0
    } else {
        start.elapsed().as_secs_f64() * 1000.0
    };
    if !frozen {
        window.request_animation_frame();
    }
    // naming-sweep: 1.8s ease-in-out, background-position 100% → 0%; the
    // gradient is 2.5× the text's width.
    let phase = ms / ai::SWEEP_MS;
    let faint = Rgba {
        a: color.a * 0.45,
        ..color
    };
    let text = "Naming…";
    let n = text.chars().count();
    let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = text
        .char_indices()
        .enumerate()
        .map(|(i, (b, ch))| {
            // Where the letter's middle falls in the 250% gradient.
            let x = (i as f64 + 0.5) / n as f64;
            let tint = ai::sweep(ai::sweep_at(x, phase), faint, color);
            (
                b..b + ch.len_utf8(),
                HighlightStyle {
                    color: Some(tint.into()),
                    ..HighlightStyle::default()
                },
            )
        })
        .collect();
    // sparkle-twinkle: 1.6s ease-in-out; at 50% opacity 0.55, scale 0.82, 12°.
    let tw = if frozen { 0.0 } else { ai::twinkle(ms / ai::TWINKLE_MS) };
    div()
        .flex()
        .items_center()
        .gap(fu(6. * scale))
        .font_weight(FontWeight::MEDIUM)
        .child(ai::sparkle(13. * zoom * scale, 2., SPARKLE, tw))
        .child(StyledText::new(text).with_highlights(highlights))
        .into_any_element()
}

/// The landed name, each letter easing in out of a blur (460ms, staggered).
pub fn reveal(id: &str, name: &str, zoom: f32, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = crate::components::child_id(ElementId::Name(id.to_owned().into()), "reveal-start");
    div()
        .id(ElementId::Name(format!("reveal-{id}").into()))
        .flex()
        .min_w_0()
        .overflow_hidden()
        .children(ai::letters(key, name, zoom, window, cx))
        .into_any_element()
}
