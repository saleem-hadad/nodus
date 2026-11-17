//! A button that only drops a menu: a label with a chevron after it (UI-339).
//! The board's `New` in the title bar is one. Its menu opens the way a split
//! button's chevron opens its own (UI-173, UI-175).

use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    SharedString, Styled, Window, canvas, div,
};

use crate::components::button::button;
use crate::components::context_menu::{Menu, MenuItem, MenuLayer, below_right};
use crate::components::icon::{Icon, icon};
use crate::theme;

pub struct MenuButton {
    pub id: ElementId,
    pub label: SharedString,
    pub icon: Option<(Icon, f32, f32)>,
    pub items: Vec<MenuItem>,
}

impl MenuButton {
    pub fn render(self, menus: &Entity<MenuLayer>, window: &mut Window, cx: &mut App) -> AnyElement {
        let t = theme::tokens(cx);
        let bounds = window.use_keyed_state((self.id.clone(), "bounds"), cx, |_, _| Bounds::<Pixels>::default());
        let open_state = window.use_keyed_state((self.id.clone(), "open"), cx, |_, _| false);
        let open = *open_state.read(cx) && menus.read(cx).is_open();
        let (menus, items, at) = (menus.clone(), self.items, bounds.clone());
        let chevron = if open { t.text_primary } else { t.text_secondary };

        let mut b = button(self.id.clone())
            .label(self.label)
            .child(icon(Icon::ChevronDown, 13., 2.2, chevron))
            .pressed(open)
            .on_press(move |window, cx| toggle_menu(&menus, &open_state, *at.read(cx), items.clone(), window, cx));
        if let Some((i, size, stroke)) = self.icon {
            b = b.icon(i, size, stroke);
        }

        div()
            .relative()
            .flex_none()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(b)
            .child(
                canvas(move |b, _, cx| bounds.update(cx, |s, _| *s = b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .into_any_element()
    }
}

/// UI-173: opens `items` right-aligned below `at`, the button's bounds, or
/// closes them if they're open; `open` says whether this button's menu is.
pub(crate) fn toggle_menu(
    menus: &Entity<MenuLayer>,
    open: &Entity<bool>,
    at: Bounds<Pixels>,
    items: Vec<MenuItem>,
    window: &mut Window,
    cx: &mut App,
) {
    if menus.read(cx).is_open() && *open.read(cx) {
        menus.update(cx, |m, cx| m.close(window, cx));
        return;
    }
    let state = open.clone();
    let mut menu = Menu::at(below_right(at), items);
    menu.align_right = true;
    menu.anchor = Some(at);
    menu.on_close = Some(Rc::new(move |_, cx| {
        state.update(cx, |s, cx| {
            *s = false;
            cx.notify();
        });
    }));
    open.update(cx, |s, cx| {
        *s = true;
        cx.notify();
    });
    menus.update(cx, |m, cx| m.open(menu, cx));
}
