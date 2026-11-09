//! The animation pieces the cards need beyond [`crate::components::tween`]:
//! transitions with a delay, one-shot animations with a delay that show
//! their first frame meanwhile (`animation-fill-mode: backwards`), and
//! infinite cycles that hold still in parity mode. All keyed element state,
//! asking for frames only while something moves.

use std::time::{Duration, Instant};

use gpui::{App, ElementId, SharedString, Window};

use crate::components::child_id;
use crate::components::tween::Mix;
use crate::parity;
use crate::theme::motion;

/// The id of a node's named part.
pub fn key(node: &str, part: &'static str) -> ElementId {
    child_id(ElementId::Name(SharedString::from(node.to_owned())), part)
}

/// The id of a node's numbered part.
pub fn key_n(node: &str, part: &'static str, n: usize) -> ElementId {
    child_id(
        ElementId::NamedInteger(SharedString::from(node.to_owned()), n as u64),
        part,
    )
}

struct Delayed<T> {
    from: T,
    to: T,
    start: Instant,
    delay: Duration,
    duration: Duration,
    ease: fn(f32) -> f32,
}

impl<T: Mix> Delayed<T> {
    fn at(&self, now: Instant) -> (T, bool) {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed < self.delay {
            return (self.from, true);
        }
        let run = elapsed - self.delay;
        if self.duration.is_zero() || run >= self.duration {
            return (self.to, false);
        }
        let t = run.as_secs_f32() / self.duration.as_secs_f32();
        (self.from.mix(self.to, (self.ease)(t)), true)
    }
}

/// `transition: <prop> <duration> <ease> <delay>`: glides to `target`
/// after `delay`, from wherever it was.
pub fn transition_delayed<T: Mix>(
    window: &mut Window,
    cx: &mut App,
    key: impl Into<ElementId>,
    target: T,
    duration: Duration,
    delay: Duration,
    ease: fn(f32) -> f32,
) -> T {
    let duration = motion::duration(cx, duration);
    let delay = if cx.reduce_motion() { Duration::ZERO } else { delay };
    let state = window.use_keyed_state(key, cx, |_, _| Delayed {
        from: target,
        to: target,
        start: Instant::now(),
        delay,
        duration,
        ease,
    });
    let now = Instant::now();
    let (value, moving) = state.update(cx, |d, _| {
        if d.to != target {
            let (current, _) = d.at(now);
            d.from = current;
            d.to = target;
            d.start = now;
            d.delay = delay;
            d.duration = duration;
            d.ease = ease;
        }
        d.at(now)
    });
    if moving {
        window.request_animation_frame();
    }
    value
}

/// A one-shot animation that started the first time `key` rendered:
/// `None` while its `delay` runs (draw the `from` frame), then its eased
/// progress, 1 once done. Reduced motion: done at once.
pub fn enter(
    window: &mut Window,
    cx: &mut App,
    key: impl Into<ElementId>,
    delay: Duration,
    duration: Duration,
    ease: fn(f32) -> f32,
) -> Option<f32> {
    if cx.reduce_motion() {
        return Some(1.0);
    }
    let start = window.use_keyed_state(key, cx, |_, _| Instant::now());
    let elapsed = start.read(cx).elapsed();
    if elapsed < delay {
        window.request_animation_frame();
        return None;
    }
    let run = elapsed - delay;
    if duration.is_zero() || run >= duration {
        return Some(1.0);
    }
    window.request_animation_frame();
    Some(ease(run.as_secs_f32() / duration.as_secs_f32()))
}

/// Where an infinite animation of `period` is (0…1, linear), started the
/// first time `key` rendered; 0 in parity mode, where shots hold still.
pub fn cycle(window: &mut Window, cx: &mut App, key: impl Into<ElementId>, period: Duration) -> f32 {
    if parity::frozen(cx) || cx.reduce_motion() || period.is_zero() {
        return 0.0;
    }
    let start = window.use_keyed_state(key, cx, |_, _| Instant::now());
    let elapsed = start.read(cx).elapsed().as_secs_f32();
    window.request_animation_frame();
    (elapsed / period.as_secs_f32()).fract()
}

/// A keyframe animation that defines only `50%`: how far toward the peak
/// it is at `t`, `ease` running on each half from that half's start to its
/// end (spec 10, Easings).
pub fn half_way(t: f32, ease: fn(f32) -> f32) -> f32 {
    if t < 0.5 {
        ease(t * 2.0)
    } else {
        1.0 - ease(t * 2.0 - 1.0)
    }
}

/// Requests a re-render in `every`, once per pending tick (a clock that
/// ticks while something counts up).
pub fn tick(window: &mut Window, cx: &mut App, key: impl Into<ElementId>, every: Duration) {
    if parity::frozen(cx) {
        return;
    }
    let pending = window.use_keyed_state(key, cx, |_, _| false);
    if *pending.read(cx) {
        return;
    }
    pending.update(cx, |p, _| *p = true);
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(every).await;
            let _ = cx.update(|window, cx| {
                pending.update(cx, |p, _| *p = false);
                window.refresh();
            });
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_half_way() {
        assert_eq!(half_way(0.0, motion::ease_out_keyword), 0.0);
        assert!((half_way(0.5, motion::ease_out_keyword) - 1.0).abs() < 1e-5);
        assert!(half_way(1.0, motion::ease_out_keyword).abs() < 1e-5);
    }

    #[test]
    fn waits_out_the_delay_on_the_first_frame() {
        let d = Delayed {
            from: 0.0f32,
            to: 1.0,
            start: Instant::now(),
            delay: Duration::from_millis(120),
            duration: Duration::from_millis(220),
            ease: motion::ease,
        };
        assert_eq!(d.at(d.start + Duration::from_millis(100)), (0.0, true));
        assert_eq!(d.at(d.start + Duration::from_millis(400)), (1.0, false));
    }
}
