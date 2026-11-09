//! While Claude names a new section (BOARD-089, BOARD-090; `board.css` ›
//! `.section-glow`, `.section-name.is-naming`, `.is-revealing`): a light
//! circling the frame (a conic gradient ring, and a blurred copy as a halo),
//! `Naming…` with a light sweeping through it beside a twinkling sparkle,
//! then the name Claude gave easing in letter by letter.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ElementId, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement, PathBuilder, Pixels, Rgba, Styled, StyledText, Window, canvas, div, point, px,
};

use super::units::fu;
use crate::components::icon::{Icon, icon};
use crate::components::tween::{enter, transition};
use crate::theme::motion;
use crate::theme::tokens::hex;

/// The ring's colours, at 0°, 60°, … 360° clockwise from the top.
const STOPS: [u32; 7] = [0x0a84ff, 0x5e5ce6, 0xbf5af2, 0xff375f, 0xff9f0a, 0xbf5af2, 0x0a84ff];
/// One turn of the glow.
const TURN_MS: f64 = 3200.0;
/// The sparkle's colour.
pub const SPARKLE: Rgba = hex(0xbf5af2);

/// `conic-gradient(from angle, …)` at a direction (degrees clockwise from up).
fn conic(direction: f64, from: f64) -> Rgba {
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

/// A point on a rounded rect's outline at `t` ∈ [0, 1) of its perimeter,
/// starting at the top-left corner's end of the top edge, clockwise.
fn outline(b: Bounds<Pixels>, r: f32, t: f32) -> gpui::Point<Pixels> {
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
    use std::f32::consts::{FRAC_PI_2, PI};
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
fn paint_ring(bounds: Bounds<Pixels>, radius: f32, width: f32, angle: f64, alpha: f32, window: &mut Window) {
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
    let ring = move |width: f32, alpha: f32| {
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint_ring(bounds, radius, width * zoom, angle, alpha, window),
        )
        .absolute()
        .size_full()
    };
    div()
        .absolute()
        .top(fu(-1.5))
        .left(fu(-1.5))
        .right(fu(-1.5))
        .bottom(fu(-1.5))
        .group_opacity(opacity)
        .child(div().absolute().size_full().blur(px(9.0 * zoom)).child(ring(3.0, 0.55)))
        .child(ring(1.5, 1.0))
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
    let p = f64::from(motion::ease_in_out(((ms % 1800.0) / 1800.0) as f32));
    let offset = 1.0 - p;
    let text = "Naming…";
    let n = text.chars().count();
    let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = text
        .char_indices()
        .enumerate()
        .map(|(i, (b, ch))| {
            // Where the letter's middle falls in the 250% gradient.
            let x = (i as f64 + 0.5) / n as f64;
            let g = (offset * 1.5 + x) / 2.5;
            let tint = sweep(g, color);
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
    let tw = if frozen {
        0.0
    } else {
        let t = (ms % 1600.0) / 1600.0;
        let half = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
        f64::from(motion::ease_in_out(half as f32))
    } as f32;
    div()
        .flex()
        .items_center()
        .gap(fu(6. * scale))
        .font_weight(FontWeight::MEDIUM)
        .child(
            div()
                .flex_none()
                .opacity(1.0 - 0.45 * tw)
                .scale(1.0 - 0.18 * tw)
                .rotate(gpui::radians((12.0 * tw).to_radians()))
                .child(icon(Icon::Sparkles, 13. * zoom * scale, 2., SPARKLE)),
        )
        .child(StyledText::new(text).with_highlights(highlights))
        .into_any_element()
}

/// The sweep's gradient at `g` ∈ [0, 1]: faint, full, purple-tinted, full, faint.
fn sweep(g: f64, color: Rgba) -> Rgba {
    let faint = Rgba {
        a: color.a * 0.45,
        ..color
    };
    let purple = super::section::mix(SPARKLE, 0.8, color);
    let lerp = |a: Rgba, b: Rgba, t: f64| {
        let t = t as f32;
        Rgba {
            r: a.r + (b.r - a.r) * t,
            g: a.g + (b.g - a.g) * t,
            b: a.b + (b.b - a.b) * t,
            a: a.a + (b.a - a.a) * t,
        }
    };
    match g {
        g if g < 0.4 => lerp(faint, color, g / 0.4),
        g if g < 0.5 => lerp(color, purple, (g - 0.4) / 0.1),
        g if g < 0.6 => lerp(purple, color, (g - 0.5) / 0.1),
        g => lerp(color, faint, ((g - 0.6) / 0.4).min(1.0)),
    }
}

/// The landed name, each letter easing in out of a blur (460ms, staggered).
pub fn reveal(id: &str, name: &str, zoom: f32, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = |part: &'static str| crate::components::child_id(ElementId::Name(id.to_owned().into()), part);
    let start = *window
        .use_keyed_state(key("reveal-start"), cx, |_, _| Instant::now())
        .read(cx);
    let letters: Vec<char> = name.chars().collect();
    let step = (28.0f64).min(480.0 / 1f64.max(letters.len() as f64 - 1.0));
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let dur = motion::duration(cx, Duration::from_millis(460)).as_secs_f64() * 1000.0;
    let mut moving = false;
    let spans: Vec<AnyElement> = letters
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let delay = (i as f64 * step).round();
            let t = ((elapsed - delay) / dur).clamp(0.0, 1.0);
            moving |= t < 1.0;
            let e = motion::ease_out(t as f32);
            div()
                .flex_none()
                .whitespace_nowrap()
                .opacity(e)
                .translate(px(0.), px(3.0 * zoom * (1.0 - e)))
                .when_blur(5.0 * zoom * (1.0 - e))
                .child(ch.to_string())
                .into_any_element()
        })
        .collect();
    if moving {
        window.request_animation_frame();
    }
    div()
        .id(ElementId::Name(format!("reveal-{id}").into()))
        .flex()
        .min_w_0()
        .overflow_hidden()
        .children(spans)
        .into_any_element()
}

trait WhenBlur {
    fn when_blur(self, sigma: f32) -> Self;
}

impl WhenBlur for gpui::Div {
    fn when_blur(self, sigma: f32) -> Self {
        if sigma > 0.05 { self.blur(px(sigma)) } else { self }
    }
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
    }

    #[test]
    fn sweeps_from_faint_to_full() {
        let c = hex(0x123456);
        assert!((sweep(0.0, c).a - 0.45).abs() < 1e-6);
        assert_eq!(sweep(0.4, c), c);
        assert!((sweep(1.0, c).a - 0.45).abs() < 1e-6);
    }
}
