//! Native-looking tooltips. 0.9.4's tooltips are HTML `title`s, which
//! Chromium shows as the OS's own tooltip after the OS's hover delay; GPUI
//! draws its tooltips itself, so this one is drawn to look like macOS's
//! (spec 08, Rust mapping risk 4).

use gpui::{
    AnyView, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Window, div, px,
};

use crate::theme::{self, fonts, tokens::*};

/// What a `.tooltip(…)` takes to show `text` (lines split on `\n`).
pub fn native_tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        if menu_open(cx) {
            cx.new(|_| Nothing).into()
        } else {
            cx.new(|_| NativeTooltip { text: text.clone() }).into()
        }
    }
}

/// A native tooltip that a press dismisses until the pointer leaves and
/// comes back, as the OS's does. See [`TooltipPress`].
pub fn pressable_tooltip(
    text: SharedString,
    press: Entity<TooltipPress>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        if press.read(cx).pressed || menu_open(cx) {
            cx.new(|_| Nothing).into()
        } else {
            cx.new(|_| NativeTooltip { text: text.clone() }).into()
        }
    }
}

fn menu_open(cx: &App) -> bool {
    cx.try_global::<crate::components::context_menu::MenuOpen>()
        .is_some_and(|m| m.0)
}

/// Whether an element's tooltip was pressed away: set on a press, cleared
/// once the pointer has left and then moves over it again (leaving alone
/// isn't enough: a menu opening over it makes it "unhovered" too).
#[derive(Debug, Default, Clone, Copy)]
pub struct TooltipPress {
    pub pressed: bool,
    left: bool,
}

impl TooltipPress {
    pub fn press(&mut self) {
        self.pressed = true;
        self.left = false;
    }

    pub fn hover(&mut self, hovered: bool) {
        if !hovered {
            self.left = true;
        }
    }

    /// The pointer moved over the element.
    pub fn moved(&mut self) {
        if self.left {
            self.pressed = false;
            self.left = false;
        }
    }
}

/// No tooltip.
struct Nothing;

impl Render for Nothing {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
    }
}

pub struct NativeTooltip {
    text: SharedString,
}

impl Render for NativeTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let (bg, fg, edge) = if t.dark {
            (
                rgba(43, 43, 45, 0.98),
                rgba(255, 255, 255, 0.88),
                rgba(255, 255, 255, 0.14),
            )
        } else {
            (rgba(246, 246, 246, 0.98), rgba(0, 0, 0, 0.85), rgba(0, 0, 0, 0.12))
        };
        // Tooltips sit below and right of the pointer, as AppKit's do.
        div().pl(px(10.)).pt(px(18.)).child(
            div()
                .flex()
                .flex_col()
                .px(px(7.))
                .py(px(3.))
                .rounded(px(5.))
                .bg(bg)
                .border(px(0.5))
                .border_color(edge)
                .shadow(vec![shadow(0., 2., 8., 0., rgba(0, 0, 0, 0.18))])
                .font_family(fonts::UI)
                .text_size(TEXT_XS)
                .line_height(px(14.))
                .text_color(fg)
                .max_w(px(320.))
                .children(
                    self.text
                        .split('\n')
                        .map(|line| div().child(SharedString::from(line.to_owned()))),
                ),
        )
    }
}
