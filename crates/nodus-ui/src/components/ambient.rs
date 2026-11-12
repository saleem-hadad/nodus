//! Ambient motion: what moves for as long as a state lasts (a working
//! session's dot breathing, a waiting one's pulse, a view's edge marching,
//! a running action's mark). Each of those used to ask GPUI for the next
//! frame, so one working session kept the window drawing at the display's
//! rate, every card of the board each time, for as long as it worked: a
//! whole core. Ambient motion moves on one shared clock instead, at [`FPS`]:
//! every request made during a frame is answered by the same tick, so
//! however many things breathe, the window draws [`FPS`] frames a second for
//! them, and only the views that asked are drawn again (a board node is its
//! own cached view, `board::node_view`).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{App, ElementId, EntityId, Global, Window};

/// Frames a second for ambient motion: slow breaths and pulses read as
/// smooth at this rate, and it divides 60 Hz and 120 Hz displays evenly.
pub const FPS: u32 = 20;
const TICK: Duration = Duration::from_millis(1000 / FPS as u64);

#[derive(Default)]
struct Ticker {
    /// Views to draw again at the next tick.
    pending: Vec<EntityId>,
    /// A tick is on its way.
    armed: bool,
    /// When the clock started; ticks fall on whole [`TICK`]s from it.
    epoch: Option<Instant>,
    /// The views the last tick drew again, and when it fired.
    notified: HashSet<EntityId>,
    fired: Option<Instant>,
    /// Requests are dropped while set: what's drawn is off screen.
    suspended: bool,
}

impl Global for Ticker {}

/// Draws the view being rendered again at the next tick (instead of
/// [`Window::request_animation_frame`], which draws it again next frame).
pub fn request_frame(window: &Window, cx: &mut App) {
    let view = window.current_view();
    let ticker = cx.default_global::<Ticker>();
    if ticker.suspended {
        return;
    }
    if !ticker.pending.contains(&view) {
        ticker.pending.push(view);
    }
    if ticker.armed {
        return;
    }
    ticker.armed = true;
    let now = Instant::now();
    let epoch = *ticker.epoch.get_or_insert(now);
    let into = now.duration_since(epoch).as_nanos() % TICK.as_nanos();
    let wait = TICK - Duration::from_nanos(into as u64);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        cx.update(|cx| {
            let ticker = cx.default_global::<Ticker>();
            ticker.armed = false;
            ticker.fired = Some(Instant::now());
            let pending = std::mem::take(&mut ticker.pending);
            ticker.notified = pending.iter().copied().collect();
            for view in pending {
                cx.notify(view);
            }
        });
    })
    .detach();
}

/// Where a looping animation of `period` is (0…1, linear), started the
/// first time `key` rendered, and a frame at the next tick to move it on.
/// Reduced motion holds it at 0.
pub fn cycle(window: &mut Window, cx: &mut App, key: impl Into<ElementId>, period: Duration) -> f32 {
    if cx.reduce_motion() || period.is_zero() {
        return 0.0;
    }
    let start = window.use_keyed_state(key, cx, |_, _| Instant::now());
    let elapsed = start.read(cx).elapsed().as_secs_f32();
    request_frame(window, cx);
    (elapsed / period.as_secs_f32()).fract()
}

/// Runs `f` with ambient requests dropped when `hidden`: a board node off
/// screen keeps still until it's back in view (which draws it again).
pub fn hidden<R>(hidden: bool, cx: &mut App, f: impl FnOnce(&mut App) -> R) -> R {
    let was = std::mem::replace(&mut cx.default_global::<Ticker>().suspended, hidden);
    let r = f(cx);
    cx.default_global::<Ticker>().suspended = was;
    r
}

/// The views the latest tick drew again, if it fired within the last tick
/// (so this frame is the one it asked for).
pub fn ticked(cx: &App) -> Option<&HashSet<EntityId>> {
    let ticker = cx.try_global::<Ticker>()?;
    ticker.fired.filter(|at| at.elapsed() < TICK).map(|_| &ticker.notified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_is_a_whole_number_of_display_frames() {
        for hz in [60, 120] {
            assert_eq!(hz % FPS, 0, "{hz} Hz");
        }
        assert_eq!(TICK, Duration::from_millis(50));
    }
}
