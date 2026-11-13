//! A view's first page landing (MOTION-033): the view was drawn while
//! Claude built it, and its page arrives. The page waits hidden until it has
//! loaded and been pictured (`swap.rs`), then the picture sharpens out of a
//! blur from the top down, a line of Claude's colours at the edge, and the
//! live page takes its place. A view that had its page when first drawn
//! shows it at once.

use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Entity, IntoElement, ParentElement, Rgba, Styled, Window, canvas, div, linear_color_stop, linear_gradient, px,
};

use super::card::CardUi;
use super::frames::{Painted, Rounding};
use crate::theme::{ai, motion};

/// How long the sharpening takes, top to bottom.
const SWEEP: Duration = Duration::from_millis(860);
/// How long the page may stay hidden waiting for its picture before it
/// shows as it would have without a reveal.
const WAIT: Duration = Duration::from_millis(1500);
/// Half the blur's soft edge, as a share of the page's height.
const SOFT: f32 = 0.08;
/// The blur over what hasn't sharpened yet, px at the board's scale 1.
const BLUR: f32 = 10.0;

/// A view's landing, kept with its card's state.
#[derive(Default)]
pub struct Landing {
    /// It was drawn without a page.
    empty_seen: bool,
    reveal: Option<Reveal>,
}

struct Reveal {
    landed: Instant,
    painted: Painted,
}

/// The reveal this frame: the picture the probe paints, for the blurred
/// copy over it, and how far the sharpening is (None until it's pictured).
pub struct Revealing {
    pub painted: Painted,
    pub progress: Option<f32>,
}

/// Steps a view's landing: starts a reveal when its first page arrives
/// (`version` 0 → more, while drawn), ends it once the page has sharpened or
/// waited too long. Asks for frames while one runs.
pub fn step(ui: &Entity<CardUi>, version: u32, framed: bool, window: &mut Window, cx: &mut App) -> Option<Revealing> {
    // Pictures exist only on macOS (WebView2 has none, `webview.rs`), and
    // parity shots and reduced motion show the page at once.
    let able = cfg!(target_os = "macos") && !crate::parity::frozen(cx) && !cx.reduce_motion();
    let now = Instant::now();
    let revealing = ui.update(cx, |u, _| {
        let l = &mut u.landing;
        if version == 0 {
            l.empty_seen = true;
            l.reveal = None;
            return None;
        }
        if !framed {
            return None;
        }
        if std::mem::take(&mut l.empty_seen) && able {
            l.reveal = Some(Reveal {
                landed: now,
                painted: Painted::default(),
            });
        }
        let r = l.reveal.as_ref()?;
        // The sweep runs from when the probe first painted the picture.
        let shown = r.painted.borrow().as_ref().map(|p| p.1);
        let over = match shown {
            Some(shown) => now.duration_since(shown) >= SWEEP,
            None => now.duration_since(r.landed) >= WAIT,
        };
        if over {
            l.reveal = None;
            return None;
        }
        Some(Revealing {
            painted: r.painted.clone(),
            progress: shown.map(|shown| {
                let t = now.duration_since(shown).as_secs_f32() / SWEEP.as_secs_f32();
                motion::ease_in_out(t.clamp(0.0, 1.0))
            }),
        })
    });
    if revealing.is_some() {
        window.request_animation_frame();
    }
    revealing
}

/// Over the probe: the picture blurred where it hasn't sharpened yet (all
/// of it until the sweep starts), and the line of colours at the edge.
pub fn overlay(r: &Revealing, rounding: Rounding, z: f32) -> impl IntoElement {
    let p = r.progress.unwrap_or(0.0);
    // The edge runs from just above the page to just below it.
    let edge = -SOFT + p * (1.0 + 2.0 * SOFT);
    let clear = Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.,
    };
    let solid = Rgba { a: 1.0, ..clear };
    let painted = r.painted.clone();
    let radius = px(rounding.radius);
    let blurred = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        // The blur spreads past the page's round corners: keep it in them.
        .overflow_hidden()
        .when(rounding.top, |d| d.rounded_t(radius))
        .when(rounding.bottom, |d| d.rounded_b(radius))
        .clip_rounded()
        .mask(linear_gradient(
            180.,
            linear_color_stop(clear, edge - SOFT),
            linear_color_stop(solid, edge + SOFT),
        ))
        .child(
            div().absolute().top_0().left_0().size_full().blur(px(BLUR * z)).child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        if let Some((image, _)) = painted.borrow().clone() {
                            let _ = window.paint_image(bounds, bounds, rounding.corners(), image, 0, false);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            ),
        );
    // The line fades in off the top corners and out at the bottom.
    let shown = r
        .progress
        .map_or(0.0, |p| (p / 0.12).min((1.0 - p) / 0.12).clamp(0.0, 1.0));
    let turn = f64::from(p) * 0.6;
    let colours = linear_gradient(
        90.,
        linear_color_stop(ai::hue(turn), 0.),
        linear_color_stop(ai::hue(0.4 + turn), 1.),
    );
    let line = move |h: f32, blur: f32, alpha: f32| {
        let el = div()
            .absolute()
            .left_0()
            .w_full()
            .top(gpui::relative(edge.clamp(0.0, 1.0)))
            .mt(px(-h * z / 2.0))
            .h(px(h * z))
            .opacity(alpha * shown)
            .bg(colours);
        if blur > 0.0 { el.blur(px(blur * z)) } else { el }
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(blurred)
        .when_some(r.progress.filter(|_| shown > 0.0), move |d, _| {
            d.child(line(8.0, 5.0, 0.55)).child(line(2.0, 0.0, 1.0))
        })
}
