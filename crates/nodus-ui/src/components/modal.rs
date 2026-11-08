//! A centred panel over a dimmed, blurred backdrop (`Modal.tsx`,
//! `Modal.css`; UI-153…UI-157). Content and sizing come from the caller.
//! Escape is the owner's: it handles the key where 0.9.4's listener did.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Styled, Window,
    anchored, deferred, div, point, px,
};

use crate::components::frosted::frosted;
use crate::components::tween::enter;
use crate::theme::tokens::*;
use crate::theme::{self, motion};

pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// A modal's panel and how it closes.
pub struct Modal {
    pub id: ElementId,
    /// The panel's width (it's never wider than the window less 48px).
    pub width: Pixels,
    /// Pressing the backdrop closes it (UI-156).
    pub close_on_backdrop: bool,
    pub on_close: OnClose,
    /// Drawn above the numbered rail? No: modals are z 500.
    pub z: usize,
}

impl Modal {
    pub fn new(id: impl Into<ElementId>, width: Pixels, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            id: id.into(),
            width,
            close_on_backdrop: true,
            on_close: Rc::new(on_close),
            z: 500,
        }
    }

    /// The backdrop and its panel holding `content`, over the whole window.
    pub fn render(self, content: impl IntoElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let t = theme::tokens(cx);
        let viewport = window.viewport_size();
        // UI-154: both fade in over 220ms, the panel rising 8px.
        let progress = enter(window, cx, (self.id.clone(), "enter"), DUR_MED, motion::ease_out);
        let on_close = self.on_close.clone();
        let close_on_backdrop = self.close_on_backdrop;
        let backdrop = div()
            .id(self.id.clone())
            .occlude()
            .w(viewport.width)
            .h(viewport.height)
            .flex()
            .items_center()
            .justify_center()
            .map(|d| frosted(d, t.modal_backdrop, 3., 1.))
            .opacity(progress)
            // UI-156: a press on the backdrop itself, not inside the panel.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if close_on_backdrop {
                    on_close(window, cx);
                }
            })
            .child(
                div()
                    .id((self.id.clone(), "panel"))
                    .relative()
                    .top(px(8. * (1. - progress)))
                    .flex()
                    .flex_col()
                    .w(self.width)
                    .max_w(viewport.width - px(48.))
                    .max_h(viewport.height - px(48.))
                    .rounded(RADIUS_XL)
                    .bg(t.bg_elevated)
                    .shadow(t.modal())
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(true, |d| d.child(content)),
            );
        deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
            .with_priority(self.z)
            .into_any_element()
    }
}
