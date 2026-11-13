//! Claude's signature (spec 15, MOTION-001): the colours and moves that
//! mark what Claude does, and only that. The colours turning round a ring,
//! a light sweeping through words, and the twinkling sparkle first marked a
//! section Claude was naming (BOARD-089, BOARD-090); everything Claude makes
//! or finishes on the board wears them now, and nothing the user does.

use std::f32::consts::{FRAC_PI_2, PI};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, Div, ElementId, IntoElement, ParentElement, PathBuilder, Pixels, Point, Rgba, Styled,
    Window, canvas, div, point, px,
};

use super::motion;
use super::tokens::hex;
use crate::components::icon::{Icon, icon};

/// The ring's colours, at 0°, 60°, … 360° clockwise from the top.
pub const STOPS: [u32; 7] = [0x0a84ff, 0x5e5ce6, 0xbf5af2, 0xff375f, 0xff9f0a, 0xbf5af2, 0x0a84ff];
/// The sparkle's colour.
pub const SPARKLE: Rgba = hex(0xbf5af2);
/// One pass of the light through words (`naming-sweep`).
pub const SWEEP_MS: f64 = 1800.0;
/// One twinkle of the sparkle (`sparkle-twinkle`).
pub const TWINKLE_MS: f64 = 1600.0;

/// `conic-gradient(from angle, …)` at a direction (degrees clockwise from up).
pub fn conic(direction: f64, from: f64) -> Rgba {
    let d = (direction - from).rem_euclid(360.0);
    let i = ((d / 60.0).floor() as usize).min(5);
    let f = (d - i as f64 * 60.0) / 60.0;
    let (a, b) = (hex(STOPS[i]), hex(STOPS[i + 1]));
    let mix = |x: f32, y: f32| x + (y - x) * f as f32;
    Rgba {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
        a: 1.0,
    }
}

/// The colours as a loop: `t` ∈ [0, 1) round it.
pub fn hue(t: f64) -> Rgba {
    conic(t.rem_euclid(1.0) * 360.0, 0.0)
}

/// `a` with `b` mixed in by `t`.
pub fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// A point on a rounded rect's outline at `t` ∈ [0, 1) of its perimeter,
/// starting at the top-left corner's end of the top edge, clockwise.
pub fn outline(b: Bounds<Pixels>, r: f32, t: f32) -> Point<Pixels> {
    let (x, y) = (f32::from(b.origin.x), f32::from(b.origin.y));
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let r = r.min(w / 2.0).min(h / 2.0);
    let (sw, sh) = (w - 2.0 * r, h - 2.0 * r);
    let arc = FRAC_PI_2 * r;
    let total = 2.0 * (sw + sh) + 4.0 * arc;
    let mut s = t.rem_euclid(1.0) * total;
    let corner = |cx: f32, cy: f32, start: f32, s: f32| {
        let a = start + s / r.max(1e-3);
        point(px(cx + r * a.cos()), px(cy + r * a.sin()))
    };
    // Top, top-right corner, right, bottom-right, bottom, bottom-left, left, top-left.
    let lengths = [sw, arc, sh, arc, sw, arc, sh, arc];
    let mut segment = 0;
    while segment < 7 && s > lengths[segment] {
        s -= lengths[segment];
        segment += 1;
    }
    match segment {
        0 => point(px(x + r + s), px(y)),
        1 => corner(x + w - r, y + r, -FRAC_PI_2, s),
        2 => point(px(x + w), px(y + r + s)),
        3 => corner(x + w - r, y + h - r, 0.0, s),
        4 => point(px(x + w - r - s), px(y + h)),
        5 => corner(x + r, y + h - r, FRAC_PI_2, s),
        6 => point(px(x), px(y + h - r - s)),
        _ => corner(x + r, y + r, PI, s),
    }
}

/// Paints the ring: a band `width` wide inside `bounds`, coloured by the
/// conic gradient turned to `angle`.
pub fn paint_ring(bounds: Bounds<Pixels>, radius: f32, width: f32, angle: f64, alpha: f32, window: &mut Window) {
    let inset = width / 2.0;
    let mid = Bounds::from_corners(
        point(bounds.origin.x + px(inset), bounds.origin.y + px(inset)),
        point(bounds.right() - px(inset), bounds.bottom() - px(inset)),
    );
    let centre = mid.center();
    const STEPS: usize = 180;
    for i in 0..STEPS {
        let (t0, t1) = (i as f32 / STEPS as f32, (i + 1) as f32 / STEPS as f32);
        let (a, b) = (outline(mid, radius - inset, t0), outline(mid, radius - inset, t1));
        let m = outline(mid, radius - inset, (t0 + t1) / 2.0);
        let dx = f64::from(f32::from(m.x - centre.x));
        let dy = f64::from(f32::from(m.y - centre.y));
        // Clockwise from up, as CSS measures a conic gradient.
        let direction = dx.atan2(-dy).to_degrees();
        let mut color = conic(direction, angle);
        color.a = alpha;
        let mut path = PathBuilder::stroke(px(width));
        path.move_to(a);
        path.line_to(b);
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

/// The glow round a box: a crisp ring of the colours turned to `angle`,
/// and a blurred copy of it as a halo, filling the element they're put in.
/// `radius` and `zoom` in screen px per board unit.
pub fn glow_layers(angle: f64, radius: f32, zoom: f32) -> Vec<AnyElement> {
    let ring = move |width: f32, alpha: f32| {
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint_ring(bounds, radius, width * zoom, angle, alpha, window),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    vec![
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .blur(px(9.0 * zoom))
            .child(ring(3.0, 0.55))
            .into_any_element(),
        ring(1.5, 1.0).into_any_element(),
    ]
}

/// The light through words: faint at `rest`, full at `peak` across the
/// band, tinted with the sparkle at its middle; `g` ∈ [0, 1] across the
/// gradient.
pub fn sweep(g: f64, rest: Rgba, peak: Rgba) -> Rgba {
    let purple = crate::board::cards::mix(SPARKLE, 0.8, peak);
    match g {
        g if g < 0.4 => lerp(rest, peak, (g / 0.4) as f32),
        g if g < 0.5 => lerp(peak, purple, ((g - 0.4) / 0.1) as f32),
        g if g < 0.6 => lerp(purple, peak, ((g - 0.5) / 0.1) as f32),
        g => lerp(peak, rest, ((g - 0.6) / 0.4).min(1.0) as f32),
    }
}

/// Where a letter at `x` ∈ [0, 1] of the words falls in the sweep's
/// gradient at `phase` ∈ [0, 1) of a pass: `background-position` 100% → 0%
/// over a gradient 2.5× as wide as the words, eased in and out.
pub fn sweep_at(x: f64, phase: f64) -> f64 {
    let offset = 1.0 - f64::from(motion::ease_in_out(phase.rem_euclid(1.0) as f32));
    (offset * 1.5 + x) / 2.5
}

/// How far into a twinkle the sparkle is (0 at rest, 1 at its smallest),
/// `phase` ∈ [0, 1) of one.
pub fn twinkle(phase: f64) -> f32 {
    let t = phase.rem_euclid(1.0);
    let half = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
    motion::ease_in_out(half as f32)
}

/// The sparkle, `size` px, at `tw` into its twinkle: at the most, opacity
/// 0.55, scale 0.82, turned 12°.
pub fn sparkle(size: f32, stroke: f32, color: Rgba, tw: f32) -> Div {
    div()
        .flex_none()
        .opacity(1.0 - 0.45 * tw)
        .scale(1.0 - 0.18 * tw)
        .rotate(gpui::radians((12.0 * tw).to_radians()))
        .child(icon(Icon::Sparkles, size, stroke, color))
}

/// `div` holding `child` with the sparkle's twinkle, for elements that
/// aren't the sparkle (a dot that thinks).
pub fn twinkling(child: impl IntoElement, tw: f32) -> Div {
    div()
        .flex_none()
        .opacity(1.0 - 0.45 * tw)
        .scale(1.0 - 0.18 * tw)
        .child(child)
}

/// Words Claude gave, each letter easing in out of a blur: 460ms on
/// `--ease-out`, 3px up and 5px of blur at `zoom` 1, staggered 28ms (the
/// whole word within 480ms). Started the first time `key` rendered; asks
/// for frames until the last letter lands.
pub fn letters(key: impl Into<ElementId>, text: &str, zoom: f32, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let start = *window.use_keyed_state(key, cx, |_, _| Instant::now()).read(cx);
    let letters: Vec<char> = text.chars().collect();
    let step = (28.0f64).min(480.0 / 1f64.max(letters.len() as f64 - 1.0));
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let dur = motion::duration(cx, Duration::from_millis(460)).as_secs_f64() * 1000.0;
    let mut moving = false;
    let spans = letters
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let delay = if cx.reduce_motion() {
                0.0
            } else {
                (i as f64 * step).round()
            };
            let t = ((elapsed - delay) / dur).clamp(0.0, 1.0);
            moving |= t < 1.0;
            let e = motion::ease_out(t as f32);
            let blur = 5.0 * zoom * (1.0 - e);
            let letter = div()
                .flex_none()
                .whitespace_nowrap()
                .opacity(e)
                .translate(px(0.), px(3.0 * zoom * (1.0 - e)));
            let letter = if blur > 0.05 { letter.blur(px(blur)) } else { letter };
            letter.child(ch.to_string()).into_any_element()
        })
        .collect();
    if moving {
        window.request_animation_frame();
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_the_conic_gradient() {
        assert_eq!(conic(0.0, 0.0), hex(0x0a84ff));
        assert_eq!(conic(180.0, 0.0), hex(0xff375f));
        assert_eq!(conic(240.0, 60.0), hex(0xff375f));
        let mid = conic(30.0, 0.0);
        assert!(mid.r > hex(0x0a84ff).r && mid.r < hex(0x5e5ce6).r);
        assert_eq!(hue(0.5), hex(0xff375f));
        assert_eq!(hue(1.5), hue(0.5));
    }

    #[test]
    fn sweeps_from_rest_to_peak() {
        let peak = hex(0x123456);
        let rest = Rgba { a: 0.45, ..peak };
        assert!((sweep(0.0, rest, peak).a - 0.45).abs() < 1e-6);
        assert_eq!(sweep(0.4, rest, peak), peak);
        assert!((sweep(1.0, rest, peak).a - 0.45).abs() < 1e-6);
        // At a pass's start the light is past the words' start.
        assert!((sweep_at(0.0, 0.0) - 0.6).abs() < 1e-9);
        assert!((sweep_at(1.0, 0.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn twinkles_and_rests() {
        assert_eq!(twinkle(0.0), 0.0);
        assert!((twinkle(0.5) - 1.0).abs() < 1e-6);
        assert!(twinkle(0.999) < 0.01);
    }
}
