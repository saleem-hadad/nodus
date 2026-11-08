//! Native-looking tooltips. 0.9.4's tooltips are HTML `title`s, which
//! Chromium shows as the OS's own tooltip after the OS's hover delay; GPUI
//! draws its tooltips itself, so this one is drawn to look like macOS's
//! (spec 08, Rust mapping risk 4).

use gpui::{
    AnyView, App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled, Window, div,
    px,
};

use crate::theme::{self, fonts, tokens::*};

/// What a `.tooltip(…)` takes to show `text` (lines split on `\n`).
pub fn native_tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| NativeTooltip { text: text.clone() }).into()
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
