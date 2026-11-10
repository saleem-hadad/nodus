//! Flow units on screen. React Flow draws every node inside one
//! `transform: translate(x, y) scale(zoom)`, so a node's contents are laid out
//! in flow units and scaled with the board. GPUI has no layout-preserving
//! scale for a subtree that reaches off screen, so the canvas lays each node
//! out at its screen rectangle and draws its contents in [`Zoomed`]: the rem
//! size there is `16px × zoom`, and a length written as [`fu`] (flow units,
//! as rems) scales with the board. Text, padding, radii and borders written
//! in `fu` come out as Chromium draws the scaled DOM, with crisp text.
//!
//! Custom elements read [`scale`] (the zoom) from the window.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Rems, Window, px, rems,
};

/// The rem size at zoom 1: 16px, as in the Electron app.
pub const BASE_REM: f32 = 16.0;

/// A length of `v` flow units (React Flow's `px` inside a node), scaled with
/// the board when drawn inside [`Zoomed`].
pub fn fu(v: f32) -> Rems {
    rems(v / BASE_REM)
}

/// The board's zoom where an element is drawn: 1 outside a node.
pub fn scale(window: &Window) -> f32 {
    f32::from(window.rem_size()) / BASE_REM
}

/// Glyph sizes per doubling while the zoom changes: a bitmap is at most
/// about 3% larger or smaller than the glyph it draws.
const ZOOMING_GLYPH_STEPS: u32 = 12;

/// Draws `child` with rems `16px × zoom`, so its [`fu`] lengths scale with
/// the board.
pub struct Zoomed {
    rem: Pixels,
    stepped: bool,
    child: AnyElement,
}

/// `child`, scaled by `zoom` through its rem-based lengths.
pub fn zoomed(zoom: f32, child: impl IntoElement) -> Zoomed {
    Zoomed {
        rem: px(BASE_REM * zoom),
        stepped: false,
        child: child.into_any_element(),
    }
}

impl Zoomed {
    /// While the board's zoom changes, the text's size changes every frame,
    /// and rasterizing every glyph again at each size made a zoom gesture
    /// drop to a few frames a second. Its glyphs then come from a few sizes
    /// per doubling, scaled, and are drawn crisp once the zoom settles.
    pub fn stepped_glyphs(mut self, stepped: bool) -> Self {
        self.stepped = stepped;
        self
    }
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
        let child = &mut self.child;
        let id = window.with_rem_size(Some(self.rem), |window| child.request_layout(window, cx));
        (id, ())
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
        let child = &mut self.child;
        window.with_rem_size(Some(self.rem), |window| child.prepaint(window, cx));
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
        let child = &mut self.child;
        let steps = self.stepped.then_some(ZOOMING_GLYPH_STEPS);
        window.with_rem_size(Some(self.rem), |window| {
            window.with_stepped_glyph_sizes(steps, |window| child.paint(window, cx));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_flow_units_as_rems() {
        assert_eq!(fu(16.0), rems(1.0));
        assert_eq!(fu(288.0), rems(18.0));
    }
}
