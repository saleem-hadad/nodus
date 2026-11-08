//! A row of colour swatches, sized for a context menu's custom row
//! (`ColorSwatches.tsx`; UI-168…UI-171).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div,
    linear_color_stop, linear_gradient, px,
};
use nodus_domain::FolderColor;

use crate::components::folder_icon::folder_hues;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::*;
use crate::theme::{self, motion};

/// The eight swatches, `value` ticked; `on_pick` gets the colour clicked.
pub fn color_swatches(
    value: FolderColor,
    on_pick: &crate::components::ColorHandler,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    div()
        .flex()
        .gap(px(6.))
        .pt(px(5.))
        .pb(px(5.))
        .pl(px(30.))
        .pr(px(8.))
        .children(FolderColor::ALL.iter().map(|&color| {
            let hues = folder_hues(color);
            let current = color == value;
            let hovered = window.use_keyed_state(("swatch-hover", color as usize), cx, |_, _| false);
            // UI-170: grows to 118% over 120ms --ease-out.
            let scale = transition(
                window,
                cx,
                ("swatch-scale", color as usize),
                if *hovered.read(cx) { 1.18 } else { 1.0 },
                DUR_FAST,
                motion::ease_out,
            );
            let name = color.as_str();
            let title = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
            let on_pick = on_pick.clone();
            // Later shadows paint over earlier ones, the reverse of CSS's list.
            let mut shadows = Vec::new();
            if current {
                // UI-169: a 2px gap, then a 1.5px ring.
                shadows.push(ring(3.5, t.border_strong));
                shadows.push(ring(2., t.bg_elevated));
            }
            shadows.push(inset_shadow(0., 0., 0., 0.5, hard::SWATCH_RING));
            div()
                .size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .child(
                    div()
                        .id(("swatch", color as usize))
                        .size(px(16.))
                        .scale(scale)
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(hues.front_top, 0.),
                            linear_color_stop(hues.front_bottom, 1.),
                        ))
                        .shadow(shadows)
                        .tooltip(native_tooltip(title.into()))
                        .on_hover(move |h, _, cx| {
                            hovered.update(cx, |s, cx| {
                                *s = *h;
                                cx.notify();
                            });
                        })
                        .on_click(move |_, window, cx| on_pick(color, window, cx))
                        .when(current, |d| d.child(icon(Icon::Check, 10., 3.5, hard::WHITE))),
                )
        }))
        .into_any_element()
}
