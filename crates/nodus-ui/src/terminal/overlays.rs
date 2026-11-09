//! What sits over a terminal's body (`terminal.css`, `.term-overlay`): the
//! loading pill, which lets typing through (MODAL-009, TERM-101), the error
//! overlay with `Try again` (MODAL-011), and an expanded card's not-started
//! overlay (TERM-094).

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, Transformation, Window, div, percentage, px,
};

use super::icons::{TermIcon, term_icon};
use crate::components::{Handler, button};
use crate::parity;
use crate::theme::tokens::*;
use crate::theme::{fonts, motion};

/// `rgba(255, 255, 255, a)`.
fn white(a: f32) -> gpui::Rgba {
    rgba(255, 255, 255, a)
}

/// `.term-overlay` with `.term-overlay-pill`: centred, passing the pointer
/// through; fades in over 300 ms after 150 ms.
pub fn loading_pill(id: impl Into<ElementId>, label: &str, window: &mut Window, cx: &mut App) -> AnyElement {
    let id: ElementId = id.into();
    let frozen = parity::frozen(cx);
    // `term-fade-in 300ms 150ms ease-out backwards`.
    let progress = crate::components::tween::enter(window, cx, (id.clone(), "fade"), Duration::from_millis(450), |t| t);
    let opacity = if frozen {
        1.0
    } else {
        motion::ease_out(((progress * 450.0 - 150.0) / 300.0).clamp(0.0, 1.0))
    };
    let spinner = term_icon(TermIcon::LoaderCircle, 14., 2., white(0.72));
    let spinner: AnyElement = if frozen {
        spinner.into_any_element()
    } else {
        spinner
            .with_animation(
                (id.clone(), "spin"),
                Animation::new(Duration::from_millis(900)).repeat(),
                |svg, t| svg.with_transformation(Transformation::rotate(percentage(t))),
            )
            .into_any_element()
    };
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(30.))
                .px(px(14.))
                .rounded(px(999.))
                .bg(white(0.07))
                .shadow(vec![ring(0.5, white(0.1))])
                .text_color(white(0.72))
                .text_size(TEXT_SM)
                .opacity(opacity)
                .child(spinner)
                .child(SharedString::from(label.to_owned())),
        )
        .into_any_element()
}

/// `.term-overlay.is-blocking`: the term background at 82 %, taking the pointer.
fn blocking(id: impl Into<ElementId>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(30, 30, 32, 0.82))
        .occlude()
}

/// The error overlay: title, the error (selectable, monospace) and `Try again`.
pub fn error_overlay(id: impl Into<ElementId>, title: &str, detail: Option<&str>, retry: Handler) -> AnyElement {
    let id: ElementId = id.into();
    blocking(id.clone())
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.))
                .max_w(px(460.))
                .p(px(24.))
                .text_center()
                .text_color(white(0.85))
                .child(
                    div()
                        .text_size(TEXT_LG)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(title.to_owned())),
                )
                .when_some(detail.filter(|d| !d.is_empty()), |d, detail| {
                    d.child(
                        div()
                            .font_family(fonts::MONO)
                            .text_size(TEXT_SM)
                            .text_color(white(0.55))
                            .cursor_text()
                            .child(SharedString::from(detail.to_owned())),
                    )
                })
                .child(
                    button((id, "retry"))
                        .primary()
                        .label("Try again")
                        .on_click(move |_, window, cx| retry(window, cx)),
                ),
        )
        .into_any_element()
}

/// TERM-094: an expanded card whose process isn't running waits for the user.
pub fn not_started_overlay(id: impl Into<ElementId>, shell: bool, wake: Handler, cx: &App) -> AnyElement {
    let id: ElementId = id.into();
    let t = crate::theme::tokens(cx);
    let (icon, label) = if shell {
        (TermIcon::SquareTerminal, "Start shell")
    } else {
        (TermIcon::Play, "Resume session")
    };
    blocking(id.clone())
        .child(
            button((id, "wake"))
                .child(term_icon(icon, 13., 2., t.text_primary))
                .child(label)
                .on_click(move |_, window, cx| wake(window, cx)),
        )
        .into_any_element()
}
