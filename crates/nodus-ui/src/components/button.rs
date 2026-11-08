//! The shared `.btn` and its variants (`global.css`; UI-015…UI-017).

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, Corners, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Pixels, RenderOnce, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::components::focus_ring::focus_ring;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::pressable_tooltip;
use crate::components::tween::transition;
use crate::theme::{self, fonts, motion, tokens::*};

/// Which `.btn` it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.btn`
    Default,
    /// `.btn-primary`
    Primary,
    /// `.btn-ghost`
    Ghost,
    /// The remove dialog's `.pv-btn-danger`.
    Danger,
}

pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A `.btn`: a 28px-tall row with an optional icon and a label.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    kind: Kind,
    square: bool,
    icon: Option<(Icon, f32, f32)>,
    label: Option<SharedString>,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<OnClick>,
    on_press: Option<crate::components::Handler>,
    color: Option<Rgba>,
    hover_color: Option<Rgba>,
    pressed_bg: bool,
    width: Option<Pixels>,
    corners: Option<Corners<Pixels>>,
    no_left_border: bool,
    no_shadow: bool,
    focus: Option<FocusHandle>,
    children: Vec<AnyElement>,
}

/// `<button class="btn">`.
pub fn button(id: impl Into<ElementId>) -> Button {
    Button {
        id: id.into(),
        kind: Kind::Default,
        square: false,
        icon: None,
        label: None,
        disabled: false,
        tooltip: None,
        on_click: None,
        on_press: None,
        color: None,
        hover_color: None,
        pressed_bg: false,
        width: None,
        corners: None,
        no_left_border: false,
        no_shadow: false,
        focus: None,
        children: Vec::new(),
    }
}

impl Button {
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }
    pub fn primary(self) -> Self {
        self.kind(Kind::Primary)
    }
    pub fn ghost(self) -> Self {
        self.kind(Kind::Ghost)
    }
    /// `.btn-icon`: 28px square, the icon centred.
    pub fn square(mut self) -> Self {
        self.square = true;
        self
    }
    /// A lucide icon before the label, `size` px, stroke `stroke`.
    pub fn icon(mut self, icon: Icon, size: f32, stroke: f32) -> Self {
        self.icon = Some((icon, size, stroke));
        self
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// The native tooltip (`title`).
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
    /// Runs on pointer down rather than click (the split button's chevron).
    pub fn on_press(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(f));
        self
    }
    /// The text colour at rest, and while hovered.
    pub fn colors(mut self, rest: Rgba, hover: Rgba) -> Self {
        self.color = Some(rest);
        self.hover_color = Some(hover);
        self
    }
    /// Holds the pressed background (an open menu's chevron).
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed_bg = pressed;
        self
    }
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }
    pub fn corners(mut self, corners: Corners<Pixels>) -> Self {
        self.corners = Some(corners);
        self
    }
    pub fn no_left_border(mut self) -> Self {
        self.no_left_border = true;
        self
    }
    pub fn no_shadow(mut self) -> Self {
        self.no_shadow = true;
        self
    }
    /// Takes keyboard focus through `handle` (a tab stop with a focus ring).
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::tokens(cx);
        let hovered_state = window.use_keyed_state((self.id.clone(), "hover"), cx, |_, _| false);
        let pressed = window.use_keyed_state((self.id.clone(), "pressed"), cx, |_, _| false);
        let hovered = *hovered_state.read(cx) && !self.disabled;

        let (rest_bg, hover_bg, border, text, shadow) = match self.kind {
            Kind::Default => (t.bg_elevated, t.bg_hover, t.border_strong, None, true),
            Kind::Primary => (t.accent, t.accent_hover, transparent(), Some(t.text_on_accent), true),
            Kind::Ghost => (transparent(), t.bg_hover, transparent(), None, false),
            Kind::Danger => (
                t.status_waiting,
                t.danger_hover,
                transparent(),
                Some(t.text_on_accent),
                true,
            ),
        };
        let target_bg = if self.pressed_bg {
            t.bg_pressed
        } else if hovered {
            hover_bg
        } else {
            rest_bg
        };
        // UI-015: the background fades over 120ms `ease`.
        let bg = transition(window, cx, (self.id.clone(), "bg"), target_bg, DUR_FAST, motion::ease);
        let color = match (hovered || self.pressed_bg, self.hover_color, self.color) {
            (true, Some(hover), _) => Some(hover),
            (_, _, Some(rest)) => Some(rest),
            _ => text,
        };
        // `.btn-primary:hover` wins over `:active`; a pressed primary that
        // the pointer slid off shows `--bg-pressed`.
        let active_bg = match self.kind {
            Kind::Primary | Kind::Danger => None,
            _ => Some(t.bg_pressed),
        };
        let focused = self
            .focus
            .as_ref()
            .is_some_and(|f| f.is_focused(window) && window.last_input_was_keyboard());
        let corners = self.corners.unwrap_or(Corners::all(RADIUS_SM));
        let on_click = self.on_click.clone();
        let on_press = self.on_press.clone();
        let tooltip = self.tooltip.clone();

        div()
            .id(self.id.clone())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(28.))
            .when(self.square, |d| d.w(px(28.)).justify_center())
            .when(!self.square && self.width.is_none(), |d| d.px(px(12.)))
            .when_some(self.width, |d, w| d.w(w).justify_center())
            .rounded_tl(corners.top_left)
            .rounded_tr(corners.top_right)
            .rounded_bl(corners.bottom_left)
            .rounded_br(corners.bottom_right)
            .border(px(0.5))
            .when(self.no_left_border, |d| d.border_l(px(0.)))
            .border_color(border)
            .bg(bg)
            .when(shadow && !self.no_shadow, |d| d.shadow(t.sm()))
            .text_size(TEXT_MD)
            .font_family(fonts::UI)
            .font_weight(gpui::FontWeight::MEDIUM)
            .line_height(px(18.2))
            .whitespace_nowrap()
            .when_some(color, Styled::text_color)
            .when(self.disabled, |d| d.opacity(0.45))
            .when_some(
                self.focus.as_ref().filter(|_| !self.disabled),
                InteractiveElement::track_focus,
            )
            .when(!self.disabled, |d| {
                let state = hovered_state.clone();
                let leave = pressed.clone();
                let press = pressed.clone();
                d.on_hover(move |h, _, cx| {
                    state.update(cx, |s, cx| {
                        *s = *h;
                        cx.notify();
                    });
                    if !*h {
                        leave.update(cx, |p, _| *p = false);
                    }
                })
                .capture_any_mouse_down(move |_, _, cx| press.update(cx, |p, _| *p = true))
                .when_some(active_bg, |d, bg| d.active(move |s| s.bg(bg)))
                .when_some(on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
                .when_some(on_press, |d, f| {
                    d.on_mouse_down(gpui::MouseButton::Left, move |_, w, cx| {
                        cx.stop_propagation();
                        f(w, cx);
                    })
                })
                .when_some(tooltip, |d, text| d.tooltip(pressable_tooltip(text, pressed.clone())))
            })
            .when_some(self.icon, |d, (i, size, stroke)| {
                d.child(icon(i, size, stroke, color.unwrap_or(t.text_primary)))
            })
            .when_some(self.label, ParentElement::child)
            .children(self.children)
            .when(focused, |d| d.child(focus_ring(corners, px(2.), t.focus_ring)))
    }
}

pub fn transparent() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.,
    }
}
