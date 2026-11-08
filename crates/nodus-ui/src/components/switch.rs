//! The macOS-style switch (`.switch`, UI-018).

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Corners, CursorStyle, ElementId, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    RenderOnce, StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::components::focus_ring::focus_ring;
use crate::components::tween::transition;
use crate::theme::tokens::{DUR_FAST, hard, inset_shadow, shadow};
use crate::theme::{self, motion};

/// A switch: 32×18 track, 14px knob.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_change: Option<crate::components::BoolHandler>,
}

pub fn switch(id: impl Into<ElementId>, checked: bool) -> Switch {
    Switch {
        id: id.into(),
        checked,
        disabled: false,
        focus: None,
        on_change: None,
    }
}

impl Switch {
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Takes keyboard focus through `handle`; Space toggles it.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Called with the new state when it's toggled.
    pub fn on_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::tokens(cx);
        let track = if self.checked { t.accent } else { t.bg_pressed };
        let track = transition(window, cx, (self.id.clone(), "track"), track, DUR_FAST, motion::ease);
        let ring_alpha = transition(
            window,
            cx,
            (self.id.clone(), "ring"),
            if self.checked { 0.0 } else { 1.0 },
            DUR_FAST,
            motion::ease,
        );
        let knob_x = transition(
            window,
            cx,
            (self.id.clone(), "knob"),
            if self.checked { 16.0 } else { 2.0 },
            DUR_FAST,
            motion::ease,
        );
        let focused = self
            .focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window) && window.last_input_was_keyboard());
        let mut border = t.border;
        border.a *= ring_alpha;
        let checked = self.checked;
        let on_change = self.on_change.clone();
        let on_key = self.on_change.clone();

        div()
            .id(self.id)
            .relative()
            .flex_none()
            .w(px(32.))
            .h(px(18.))
            .mt(px(1.))
            .rounded(px(9.))
            .bg(track)
            .shadow(vec![inset_shadow(0., 0., 0., 0.5, border)])
            .when(self.disabled, |d| d.opacity(0.5))
            .when(!self.disabled, |d| {
                d.cursor(CursorStyle::PointingHand)
                    .when_some(self.focus.as_ref(), InteractiveElement::track_focus)
                    .when_some(on_change, |d, f| {
                        d.on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            f(!checked, window, cx);
                        })
                    })
                    .when_some(on_key, |d, f| {
                        d.on_key_down(move |e: &KeyDownEvent, window, cx| {
                            if e.keystroke.key == "space" {
                                cx.stop_propagation();
                                f(!checked, window, cx);
                            }
                        })
                    })
            })
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(knob_x))
                    .size(px(14.))
                    .rounded_full()
                    .bg(hard::SWITCH_KNOB)
                    .shadow(vec![shadow(0., 1., 2., 0., hard::SWITCH_KNOB_SHADOW)]),
            )
            .when(focused, |d| {
                d.child(focus_ring(Corners::all(px(9.)), px(2.), t.focus_ring))
            })
    }
}
