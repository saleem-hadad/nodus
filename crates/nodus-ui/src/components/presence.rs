//! Leaving (spec 15, MOTION-011): an overlay that rose in fades out and
//! sinks back the way it came instead of vanishing the frame it closes.
//! GPUI can't keep an element once it's gone, so whatever draws an overlay
//! keeps what it drew in a [`Leaving`] and draws it again for the exit,
//! [`inert`]: seen, but never hit, so a press during the exit reaches
//! whatever is under it and can't run the overlay's actions again.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use crate::theme::motion;

/// How long an exit takes.
pub const EXIT: Duration = Duration::from_millis(130);

/// `cubic-bezier(0.4, 0, 1, 1)`: an exit picks up speed as it goes.
pub fn ease_in(t: f32) -> f32 {
    motion::cubic_bezier(0.4, 0.0, 1.0, 1.0)(t)
}

/// What an overlay drew, kept while it leaves.
pub struct Leaving<T> {
    pub what: T,
    since: Instant,
}

impl<T> Leaving<T> {
    /// How much of it still shows: 1 as it starts to leave, 0 once gone.
    /// It stands in for the enter's progress, run backwards.
    pub fn shown(&self) -> f32 {
        let t = self.since.elapsed().as_secs_f32() / EXIT.as_secs_f32();
        1.0 - ease_in(t.clamp(0.0, 1.0))
    }

    pub fn gone(&self) -> bool {
        self.since.elapsed() >= EXIT
    }
}

/// `what` leaving from now; `None` while the OS asks for reduced motion or
/// parity shots hold still, where it vanishes as before.
pub fn leave<T>(what: T, cx: &App) -> Option<Leaving<T>> {
    if cx.reduce_motion() || crate::parity::frozen(cx) {
        return None;
    }
    Some(Leaving {
        what,
        since: Instant::now(),
    })
}

/// The exit to draw this frame, if there is one: drops it once it's gone,
/// and asks for the next frame while it isn't.
pub fn step<'a, T>(slot: &'a mut Option<Leaving<T>>, window: &mut Window) -> Option<&'a Leaving<T>> {
    if slot.as_ref().is_some_and(Leaving::gone) {
        *slot = None;
    }
    let leaving = slot.as_ref()?;
    window.request_animation_frame();
    Some(leaving)
}

/// An overlay drawn from a value each frame (the toast's message, the slash
/// menu's commands), and the exit of what it stopped drawing.
pub struct Presence<T> {
    leaving: Option<Leaving<T>>,
}

impl<T> Default for Presence<T> {
    fn default() -> Self {
        Self { leaving: None }
    }
}

impl<T: Clone + 'static> Presence<T> {
    /// This frame's value, `None` while it's closed. What to draw: the
    /// value, with `None` while it's open (live, entering as it does), or
    /// how much of it still shows while it leaves (inert).
    ///
    /// What was drawn is kept as `key`'s element state, which lasts only
    /// while it's drawn each frame: one that went with something around it
    /// (a bar hidden) isn't brought back to leave when that comes back.
    pub fn frame(
        &mut self,
        key: impl Into<ElementId>,
        open: Option<T>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(T, Option<f32>)> {
        let drawn = window.use_keyed_state(key, cx, |_, _| None::<T>);
        if let Some(open) = open {
            self.leaving = None;
            drawn.update(cx, |d, _| *d = Some(open.clone()));
            return Some((open, None));
        }
        if let Some(was) = drawn.update(cx, |d, _| d.take()) {
            self.leaving = leave(was, cx);
        }
        let leaving = step(&mut self.leaving, window)?;
        Some((leaving.what.clone(), Some(leaving.shown())))
    }
}

/// `child` drawn as it is, but with every hitbox it inserts empty: nothing
/// in it is hovered, clicked, scrolled or given a tooltip, and nothing in it
/// blocks what's under it.
pub fn inert(child: impl IntoElement) -> Inert {
    Inert {
        child: child.into_any_element(),
    }
}

/// See [`inert`].
pub struct Inert {
    child: AnyElement,
}

impl IntoElement for Inert {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Inert {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // Hitboxes are inserted in prepaint, clipped to the content mask
        // then (and a transform below keeps the mask outside it); paint has
        // its own masks, so the child still draws in full.
        let child = &mut self.child;
        window.with_content_mask(
            Some(ContentMask {
                bounds: Bounds::default(),
            }),
            |window| {
                child.prepaint(window, cx);
            },
        );
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_out_from_shown_to_gone() {
        assert_eq!(ease_in(0.0), 0.0);
        assert_eq!(ease_in(1.0), 1.0);
        // Slow to start, as an exit is.
        assert!(ease_in(0.25) < 0.25);
        let leaving = Leaving {
            what: (),
            since: Instant::now(),
        };
        assert!(leaving.shown() > 0.9 && !leaving.gone());
        let done = Leaving {
            what: (),
            since: Instant::now() - EXIT,
        };
        assert!(done.gone() && done.shown().abs() < 1e-6);
    }
}
