//! A button for the default action with a chevron beside it that drops a
//! menu of the others (`SplitButton.tsx`; UI-172…UI-176). The projects
//! grid's `New Project` in the title bar is one.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, Corners, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, SharedString, Styled, Window, canvas, div, px,
};

use crate::components::button::button;
use crate::components::context_menu::{MenuItem, MenuLayer};
use crate::components::icon::Icon;
use crate::components::menu_button::toggle_menu;
use crate::theme::tokens::*;
use crate::theme::{self};

pub struct SplitButton {
    pub id: ElementId,
    pub label: SharedString,
    pub icon: Option<(Icon, f32, f32)>,
    pub title: Option<SharedString>,
    pub on_click: crate::components::Handler,
    pub items: Vec<MenuItem>,
    /// The chevron's tooltip and name; `More` by default.
    pub menu_label: SharedString,
}

impl SplitButton {
    pub fn render(self, menus: &Entity<MenuLayer>, window: &mut Window, cx: &mut App) -> AnyElement {
        let t = theme::tokens(cx);
        let bounds = window.use_keyed_state((self.id.clone(), "bounds"), cx, |_, _| Bounds::<Pixels>::default());
        let open_state = window.use_keyed_state((self.id.clone(), "open"), cx, |_, _| false);
        let open = *open_state.read(cx) && menus.read(cx).is_open();
        let on_click = self.on_click.clone();
        let items = self.items.clone();
        let menus = menus.clone();
        let group_bounds = bounds.clone();
        let chevron_open = open_state.clone();

        let mut main = button((self.id.clone(), "main"))
            .no_shadow()
            .corners(Corners {
                top_left: RADIUS_SM,
                bottom_left: RADIUS_SM,
                top_right: px(0.),
                bottom_right: px(0.),
            })
            .label(self.label.clone())
            .on_click(move |_, window, cx| on_click(window, cx));
        if let Some((icon, size, stroke)) = self.icon {
            main = main.icon(icon, size, stroke);
        }
        if let Some(title) = self.title.clone() {
            main = main.tooltip(title);
        }

        div()
            .relative()
            .flex()
            .flex_none()
            .rounded(RADIUS_SM)
            .shadow(t.sm())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(main)
            .child(
                button((self.id.clone(), "more"))
                    .no_shadow()
                    .no_left_border()
                    .width(px(22.))
                    .corners(Corners {
                        top_left: px(0.),
                        bottom_left: px(0.),
                        top_right: RADIUS_SM,
                        bottom_right: RADIUS_SM,
                    })
                    .icon(Icon::ChevronDown, 13., 2.2)
                    .colors(t.text_secondary, t.text_primary)
                    .pressed(open)
                    .when(open, |b| b.colors(t.text_primary, t.text_primary))
                    .tooltip(self.menu_label.clone())
                    // UI-173: toggles on press, not click.
                    .on_press(move |window, cx| {
                        let at = *group_bounds.read(cx);
                        toggle_menu(&menus, &chevron_open, at, items.clone(), window, cx);
                    }),
            )
            .child(
                canvas(move |b, _, cx| bounds.update(cx, |s, _| *s = b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .into_any_element()
    }
}
