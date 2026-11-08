//! `Zoomed`: renders its child with an overridden rem size.
//!
//! GPUI has no general element transform. A board card is therefore laid out in *screen* space
//! (position/size = world * zoom + pan) and everything inside the card is styled in `rems`, so
//! overriding the rem size to `16px * zoom` for the subtree scales text, padding, radii, borders
//! (anything expressed in rems) with the board. Custom elements (the terminal) read
//! `window.rem_size() / 16` as their scale.
use gpui::*;

pub const BASE_REM: f32 = 16.0;

pub struct Zoomed {
    rem: Pixels,
    child: AnyElement,
}

pub fn zoomed(scale: f32, child: impl IntoElement) -> Zoomed {
    Zoomed { rem: px(BASE_REM * scale), child: child.into_any_element() }
}

/// Scale a custom element should apply (1.0 outside of a `Zoomed`).
pub fn current_scale(window: &Window) -> f32 {
    f32::from(window.rem_size()) / BASE_REM
}

impl IntoElement for Zoomed {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Zoomed {
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
        let rem = self.rem;
        let child = &mut self.child;
        let id = window.with_rem_size(Some(rem), |window| child.request_layout(window, cx));
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem = self.rem;
        let child = &mut self.child;
        window.with_rem_size(Some(rem), |window| child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem = self.rem;
        let child = &mut self.child;
        window.with_rem_size(Some(rem), |window| child.paint(window, cx));
    }
}
