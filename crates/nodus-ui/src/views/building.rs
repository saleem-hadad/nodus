//! While Claude builds a view (MOTION-032): in the empty state's place of
//! a spinner, the page it's making as bars of a page with Claude's light
//! running through them, and `Claude is building this view…` with the
//! sparkle twinkling beside it and the light through its words, as a
//! section being named shows them (BOARD-090). Both move on the ambient
//! clock and hold still in parity shots and under reduced motion.

use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, ElementId, FontWeight, IntoElement, ParentElement, Pixels, Rgba, Styled, StyledText,
    Window, canvas, div, fill, linear_color_stop, linear_gradient, point, px, size,
};

use super::parts::mix;
use crate::board::units::fu;
use crate::components::ambient;
use crate::theme::{self, Tokens, ai};

/// The bars: a share of the width and a height (flow units) each, a title
/// and three lines of text.
const ROWS: [(f32, f32); 4] = [(0.46, 9.0), (0.92, 6.0), (0.78, 6.0), (0.6, 6.0)];
/// The bars' width, and the space between them (flow units).
const WIDTH: f32 = 168.0;
const GAP: f32 = 6.0;
/// One pass of the light through the bars; it crosses in the first part
/// and rests off them for the rest.
const PASS_MS: u64 = 1700;
const CROSSING: f32 = 0.72;

/// Whether the building state moves (not in parity shots or under reduced
/// motion).
fn moving(cx: &App) -> bool {
    !crate::parity::frozen(cx) && !cx.reduce_motion()
}

/// The page being built, as bars a light crosses.
pub fn skeleton(view_id: &str, z: f32, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let phase = moving(cx).then(|| {
        let key = ElementId::Name(format!("view-skeleton-{view_id}").into());
        ambient::cycle(window, cx, key, Duration::from_millis(PASS_MS))
    });
    let height = ROWS.iter().map(|r| r.1).sum::<f32>() + GAP * (ROWS.len() - 1) as f32;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint_bars(bounds, z, phase, &t, window),
    )
    .w(fu(WIDTH))
    .h(fu(height))
    .flex_none()
    .into_any_element()
}

/// Paints the bars, each a dim pill, and the light where it's crossing:
/// its middle tinted with Claude's colours, each bar a little behind the
/// one above.
fn paint_bars(bounds: Bounds<Pixels>, z: f32, phase: Option<f32>, t: &Tokens, window: &mut Window) {
    let dim = mix(t.text_primary, 0.08, t.bg_elevated);
    let (x0, mut y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let w = f32::from(bounds.size.width);
    let band = w * 0.45;
    let lag = 16.0 * z;
    let travel = w + band + lag * ROWS.len() as f32;
    for (i, &(share, h)) in ROWS.iter().enumerate() {
        let (bw, bh) = (w * share, h * z);
        let r = px(bh / 2.0);
        let bar = Bounds::new(point(px(x0), px(y)), size(px(bw), px(bh)));
        window.paint_quad(fill(bar, dim).corner_radii(r));
        if let Some(phase) = phase.filter(|p| *p < CROSSING) {
            // The light's middle, crossing from before the bars to past them.
            let mid = x0 - band / 2.0 + travel * (phase / CROSSING) - lag * i as f32;
            let tint = ai::hue(0.15 + 0.1 * i as f64 + f64::from(phase) * 0.25);
            let bright = mix(tint, 0.42, dim);
            // Only the straight part of the pill takes the light, so its
            // round ends stay whole.
            let (lo, hi) = (x0 + bh / 2.0, x0 + bw - bh / 2.0);
            let ramp = |a: f32, b: f32, ca: Rgba, cb: Rgba, window: &mut Window| {
                let (s, e) = (a.max(lo), b.min(hi));
                if e - s < 0.5 {
                    return;
                }
                let at = |x: f32| ai::lerp(ca, cb, ((x - a) / (b - a)).clamp(0.0, 1.0));
                let piece = Bounds::new(point(px(s), px(y)), size(px(e - s), px(bh)));
                window.paint_quad(fill(
                    piece,
                    linear_gradient(90., linear_color_stop(at(s), 0.), linear_color_stop(at(e), 1.)),
                ));
            };
            ramp(mid - band / 2.0, mid, dim, bright, window);
            ramp(mid, mid + band / 2.0, bright, dim, window);
        }
        y += bh + GAP * z;
    }
}

/// `Claude is building this view…`: the sparkle twinkling beside the words,
/// the light passing through them.
pub fn title(view_id: &str, z: f32, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let text = "Claude is building this view…";
    let (sweep, tw) = if moving(cx) {
        let key = |part: &'static str| ElementId::Name(format!("view-building-{part}-{view_id}").into());
        let sweep = ambient::cycle(window, cx, key("sweep"), Duration::from_millis(ai::SWEEP_MS as u64));
        let twinkle = ambient::cycle(window, cx, key("twinkle"), Duration::from_millis(ai::TWINKLE_MS as u64));
        (Some(f64::from(sweep)), ai::twinkle(f64::from(twinkle)))
    } else {
        (None, 0.0)
    };
    let color = t.text_primary;
    let words = match sweep {
        Some(phase) => {
            let highlights = ai::sweep_highlights(text, phase, color);
            StyledText::new(text).with_highlights(highlights).into_any_element()
        }
        None => text.into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(fu(6.))
        .text_size(fu(13.))
        .line_height(fu(18.2))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(ai::sparkle(13.0 * z, 2.0, ai::SPARKLE, tw))
        .child(words)
        .into_any_element()
}
