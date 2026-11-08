//! The easings and timings of `tokens.css` and the CSS keywords the app uses,
//! as functions GPUI's animations take, and the OS's reduced-motion switch
//! (UI-012).

use std::time::Duration;

use gpui::App;

/// `cubic-bezier(x1, y1, x2, y2)`, solved for x as browsers do.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 + Copy {
    move |t: f32| {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let (x1, y1, x2, y2) = (f64::from(x1), f64::from(y1), f64::from(x2), f64::from(y2));
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;
        let sample_x = |s: f64| ((ax * s + bx) * s + cx) * s;
        let sample_y = |s: f64| ((ay * s + by) * s + cy) * s;
        let slope_x = |s: f64| (3.0 * ax * s + 2.0 * bx) * s + cx;
        let x = f64::from(t);
        // Newton's method first, bisection when the slope is too flat.
        let mut s = x;
        for _ in 0..8 {
            let err = sample_x(s) - x;
            if err.abs() < 1e-7 {
                return sample_y(s) as f32;
            }
            let d = slope_x(s);
            if d.abs() < 1e-6 {
                break;
            }
            s -= err / d;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        s = x;
        for _ in 0..64 {
            let v = sample_x(s);
            if (v - x).abs() < 1e-7 {
                break;
            }
            if v < x {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
        sample_y(s) as f32
    }
}

/// `--ease-out`: `cubic-bezier(0.2, 0.8, 0.2, 1)`.
pub fn ease_out(t: f32) -> f32 {
    cubic_bezier(0.2, 0.8, 0.2, 1.0)(t)
}

/// `--ease-spring`: `cubic-bezier(0.34, 1.4, 0.64, 1)`, which overshoots.
pub fn ease_spring(t: f32) -> f32 {
    cubic_bezier(0.34, 1.4, 0.64, 1.0)(t)
}

/// CSS `ease`.
pub fn ease(t: f32) -> f32 {
    cubic_bezier(0.25, 0.1, 0.25, 1.0)(t)
}

/// CSS `ease-in-out`.
pub fn ease_in_out(t: f32) -> f32 {
    cubic_bezier(0.42, 0.0, 0.58, 1.0)(t)
}

/// The CSS keyword `ease-out` (not the token).
pub fn ease_out_keyword(t: f32) -> f32 {
    cubic_bezier(0.0, 0.0, 0.58, 1.0)(t)
}

/// `--ease-stack`'s 29 outputs, at equally spaced inputs.
const STACK: [f32; 29] = [
    0.0, 0.05, 0.1697, 0.3218, 0.4803, 0.6281, 0.7553, 0.8579, 0.9355, 0.9902, 1.0254, 1.0452, 1.0534, 1.0536, 1.0487,
    1.0411, 1.0326, 1.0242, 1.0166, 1.0104, 1.0055, 1.0019, 0.9995, 0.998, 0.9973, 0.9971, 0.9972, 0.9975, 1.0,
];

/// `--ease-stack`: the `linear(…)` spring of the archive stack.
pub fn ease_stack(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let steps = (STACK.len() - 1) as f32;
    let at = t * steps;
    let i = (at.floor() as usize).min(STACK.len() - 2);
    let f = at - i as f32;
    STACK[i] + (STACK[i + 1] - STACK[i]) * f
}

/// How long a transition or animation of `duration` really takes: 0.01 ms
/// while the OS asks for reduced motion (UI-012).
pub fn duration(cx: &App, duration: Duration) -> Duration {
    if cx.reduce_motion() {
        Duration::from_micros(10)
    } else {
        duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_like_a_browser() {
        // Chromium's values for these curves, to 3 decimals.
        assert!((ease(0.5) - 0.8024).abs() < 1e-3);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-3);
        assert!((ease(0.25) - 0.4094).abs() < 1e-3, "{}", ease(0.25));
        assert!((ease_out(0.25) - 0.7673).abs() < 1e-3);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        // The spring overshoots.
        assert!((0..100).any(|i| ease_spring(i as f32 / 100.0) > 1.0));
    }

    #[test]
    fn steps_through_the_stack_linearly() {
        assert_eq!(ease_stack(0.0), 0.0);
        assert_eq!(ease_stack(1.0), 1.0);
        assert!((ease_stack(1.0 / 28.0) - 0.05).abs() < 1e-6);
        assert!((ease_stack(0.5 / 28.0) - 0.025).abs() < 1e-6);
    }
}
