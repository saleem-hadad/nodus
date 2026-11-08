//! The macOS-style context menu (`ContextMenu.tsx`, `ContextMenu.css`;
//! UI-158…UI-167). One window has at most one open, in the [`MenuLayer`],
//! drawn above everything (z 1000).

use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, Bounds, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, Render, SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};

use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tween::enter;
use crate::theme::tokens::*;
use crate::theme::{self, motion};

pub type Action = Rc<dyn Fn(&mut Window, &mut App)>;
pub type Custom = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// One row of a menu.
#[derive(Clone)]
pub enum MenuItem {
    Separator,
    Item {
        label: SharedString,
        icon: Option<Icon>,
        /// Right-aligned, e.g. a shortcut like `⌘O`.
        hint: Option<SharedString>,
        danger: bool,
        disabled: bool,
        on_select: Action,
    },
    /// A row drawn as given (the colour swatches).
    Custom(Custom),
}

impl MenuItem {
    pub fn item(
        label: impl Into<SharedString>,
        icon: Icon,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Item {
            label: label.into(),
            icon: Some(icon),
            hint: None,
            danger: false,
            disabled: false,
            on_select: Rc::new(on_select),
        }
    }

    pub fn hint(mut self, text: impl Into<SharedString>) -> Self {
        if let Self::Item { hint, .. } = &mut self {
            *hint = Some(text.into());
        }
        self
    }

    pub fn danger(mut self) -> Self {
        if let Self::Item { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
}

/// An open menu.
#[derive(Clone)]
pub struct Menu {
    /// Window coordinates: the menu's top-left, or with `align_right` its top-right.
    pub at: Point<Pixels>,
    pub align_right: bool,
    pub items: Vec<MenuItem>,
    /// The button that opened it: presses on it are left to it (UI-161).
    pub anchor: Option<Bounds<Pixels>>,
    /// Told whenever the menu closes, however it closes.
    pub on_close: Option<Action>,
    /// Bumped per opening, so each one animates in.
    pub serial: u64,
}

impl Menu {
    pub fn at(at: Point<Pixels>, items: Vec<MenuItem>) -> Self {
        Self {
            at,
            align_right: false,
            items,
            anchor: None,
            on_close: None,
            serial: 0,
        }
    }
}

/// The window's menu, if one is open.
#[derive(Default)]
pub struct MenuLayer {
    pub menu: Option<Menu>,
    serial: u64,
}

impl MenuLayer {
    pub fn open(&mut self, mut menu: Menu, cx: &mut Context<'_, Self>) {
        self.serial += 1;
        menu.serial = self.serial;
        self.menu = Some(menu);
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Closes the menu and tells its owner.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(menu) = self.menu.take() {
            cx.notify();
            if let Some(on_close) = menu.on_close {
                on_close(window, cx);
            }
        }
    }
}

impl Render for MenuLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let Some(menu) = self.menu.clone() else {
            return div().into_any_element();
        };
        let t = theme::tokens(cx);
        let this = cx.entity();
        // UI-167: fades in over 90ms --ease-out.
        let opacity = enter(
            window,
            cx,
            ("ctx-menu-in", menu.serial as usize),
            Duration::from_millis(90),
            motion::ease_out,
        );
        let anchor = menu.anchor;
        let rows = menu.items.iter().enumerate().map(|(i, item)| match item {
            MenuItem::Separator => div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(t.separator).into_any_element(),
            MenuItem::Custom(render) => render(window, cx),
            MenuItem::Item {
                label,
                icon: item_icon,
                hint,
                danger,
                disabled,
                on_select,
            } => {
                let hovered = window.use_keyed_state(("ctx-item", menu.serial as usize * 100 + i), cx, |_, _| false);
                let on = *hovered.read(cx) && !disabled;
                let color = if on {
                    t.text_on_accent
                } else if *danger {
                    t.status_waiting
                } else {
                    t.text_primary
                };
                let hint_color = if on {
                    Rgba8::with_alpha(color, 0.8)
                } else {
                    t.text_tertiary
                };
                let on_select = on_select.clone();
                let layer = this.clone();
                div()
                    .id(("ctx-item", i))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .w_full()
                    .h(px(26.))
                    .pl(px(6.))
                    .pr(px(10.))
                    .rounded(px(5.))
                    .text_color(color)
                    .when(on, |d| d.bg(t.accent))
                    .when(*disabled, |d| d.opacity(0.4))
                    .when(!disabled, |d| {
                        d.on_hover(move |h, _, cx| {
                            hovered.update(cx, |s, cx| {
                                *s = *h;
                                cx.notify();
                            });
                        })
                        // UI-162: closes, then runs the item.
                        .on_click(move |_, window, cx| {
                            layer.update(cx, |l, cx| l.close(window, cx));
                            on_select(window, cx);
                        })
                    })
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .w(px(16.))
                            .flex_none()
                            .opacity(0.85)
                            .when_some(*item_icon, |d, i| d.child(icon(i, 14., 2., color))),
                    )
                    .child(div().flex_1().child(label.clone()))
                    .when_some(hint.clone(), |d, hint| {
                        d.child(div().text_size(TEXT_SM).text_color(hint_color).child(hint))
                    })
                    .into_any_element()
            }
        });
        let rows: Vec<AnyElement> = rows.collect();
        let layer = this.clone();
        let close_out = this.clone();

        let panel = div()
            .id("ctx-menu")
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(200.))
            .p(px(5.))
            .rounded(RADIUS_MD)
            .map(|d| frosted(d, t.menu_surface, 30., 1.8))
            .shadow(t.lg())
            .text_size(TEXT_MD)
            .font_weight(FontWeight::NORMAL)
            .opacity(opacity)
            // UI-166: no other menu over a menu.
            .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
            // UI-160: a press anywhere else closes it; a press on the button
            // that opened it is left to that button (UI-161).
            .on_mouse_down_out(move |e, window, cx| {
                if anchor.is_some_and(|a| a.contains(&e.position)) {
                    return;
                }
                close_out.update(cx, |l, cx| l.close(window, cx));
            })
            .children(rows);
        let _ = layer;

        deferred(
            anchored()
                .position(menu.at)
                .anchor(if menu.align_right {
                    Anchor::TopRight
                } else {
                    Anchor::TopLeft
                })
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .with_priority(1000)
        .into_any_element()
    }
}

/// Colour helpers for rows.
struct Rgba8;

impl Rgba8 {
    fn with_alpha(c: gpui::Rgba, share: f32) -> gpui::Rgba {
        gpui::Rgba { a: c.a * share, ..c }
    }
}

/// Where a menu dropped from a button goes: right-aligned 4px below it.
pub fn below_right(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(bounds.right(), bounds.bottom() + px(4.))
}
