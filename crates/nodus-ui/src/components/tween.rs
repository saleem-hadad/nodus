//! CSS transitions for GPUI: a value that glides to a new target over a
//! duration with an easing, instead of jumping (`transition: background
//! 120ms ease`). Each transition is element state, keyed like an element id,
//! and asks for frames only while it moves.

use std::time::{Duration, Instant};

use gpui::{App, ElementId, Rgba, Window};

/// Something a transition can glide between.
pub trait Mix: Copy + PartialEq + 'static {
    fn mix(self, to: Self, t: f32) -> Self;
}

impl Mix for f32 {
    fn mix(self, to: Self, t: f32) -> Self {
        self + (to - self) * t
    }
}

impl Mix for Rgba {
    fn mix(self, to: Self, t: f32) -> Self {
        Rgba {
            r: self.r.mix(to.r, t),
            g: self.g.mix(to.g, t),
            b: self.b.mix(to.b, t),
            a: self.a.mix(to.a, t),
        }
    }
}

struct Tween<T> {
    from: T,
    to: T,
    start: Instant,
    duration: Duration,
    ease: fn(f32) -> f32,
}

impl<T: Mix> Tween<T> {
    fn value_at(&self, now: Instant) -> T {
        if self.duration.is_zero() {
            return self.to;
        }
        let t = now.duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32();
        if t >= 1.0 {
            return self.to;
        }
        self.from.mix(self.to, (self.ease)(t))
    }
}

/// The value of the transition `key` this frame, gliding toward `target`
/// over `duration` with `ease` whenever `target` changes. It starts at
/// `target`, with nothing to glide from, and a transition interrupted midway
/// starts again from where it was, as CSS's do.
pub fn transition<T: Mix>(
    window: &mut Window,
    cx: &mut App,
    key: impl Into<ElementId>,
    target: T,
    duration: Duration,
    ease: fn(f32) -> f32,
) -> T {
    let duration = crate::theme::motion::duration(cx, duration);
    let state = window.use_keyed_state(key, cx, |_, _| Tween {
        from: target,
        to: target,
        start: Instant::now(),
        duration,
        ease,
    });
    let now = Instant::now();
    let (value, moving) = state.update(cx, |tween, _| {
        if tween.to != target {
            tween.from = tween.value_at(now);
            tween.to = target;
            tween.start = now;
            tween.duration = duration;
            tween.ease = ease;
        }
        (tween.value_at(now), now.duration_since(tween.start) < tween.duration)
    });
    if moving {
        window.request_animation_frame();
    }
    value
}

/// A one-shot enter animation's progress (0 → 1 eased), started the first
/// time `key` renders: CSS `animation: name dur ease` on an element that
/// just appeared. 1 once it has finished.
pub fn enter(
    window: &mut Window,
    cx: &mut App,
    key: impl Into<ElementId>,
    duration: Duration,
    ease: fn(f32) -> f32,
) -> f32 {
    let duration = crate::theme::motion::duration(cx, duration);
    let start = window.use_keyed_state(key, cx, |_, _| Instant::now());
    let elapsed = start.read(cx).elapsed();
    if duration.is_zero() || elapsed >= duration {
        return 1.0;
    }
    window.request_animation_frame();
    ease(elapsed.as_secs_f32() / duration.as_secs_f32())
}
