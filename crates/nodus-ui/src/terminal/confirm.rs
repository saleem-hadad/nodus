//! The small macOS-style alert the session window's Stop asks with
//! (`ConfirmDialog.tsx`, `.confirm-*` in `board.css`; MODAL-023): a title,
//! the message, `Cancel` and the confirm button, which has the keyboard.
//! Enter confirms and Escape cancels.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    SharedString, Styled, Window, div, px,
};
use nodus_core::StopConfirmation;

use crate::components::modal::Modal;
use crate::components::{Kind, button};
use crate::theme::tokens::*;
use crate::theme::{self};

/// Called with whether the user confirmed.
pub type Resolve = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// The dialog for `request`; `focus` is the confirm button's handle.
pub fn confirm_dialog(
    id: impl Into<ElementId>,
    request: &StopConfirmation,
    focus: &FocusHandle,
    resolve: Resolve,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id: ElementId = id.into();
    let t = theme::tokens(cx);
    let cancel = resolve.clone();
    let confirm = resolve.clone();
    let keys = resolve.clone();
    let on_close = resolve.clone();
    let body = div()
        .id((id.clone(), "body"))
        .track_focus(focus)
        .flex()
        .flex_col()
        .pt(px(22.))
        .px(px(22.))
        .pb(px(18.))
        .on_key_down(move |e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                keys(false, window, cx);
            }
            "enter" => {
                cx.stop_propagation();
                keys(true, window, cx);
            }
            _ => {}
        })
        .child(
            div()
                .mb(px(6.))
                .text_size(TEXT_LG)
                .line_height(px(21.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(SharedString::from(request.title.clone())),
        )
        .child(
            div()
                .mb(px(20.))
                .text_color(t.text_secondary)
                .line_height(px(13.0 * 1.45))
                .child(SharedString::from(request.message.clone())),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    button((id.clone(), "cancel"))
                        .label("Cancel")
                        .on_click(move |_, window, cx| cancel(false, window, cx)),
                )
                .child(
                    button((id.clone(), "confirm"))
                        .kind(if request.danger { Kind::Danger } else { Kind::Primary })
                        .label(SharedString::from(request.confirm_label.clone()))
                        .focus(focus)
                        .on_click(move |_, window, cx| confirm(true, window, cx)),
                ),
        );
    let mut modal = Modal::new(id, px(380.), move |window, cx| on_close(false, window, cx));
    modal.z = 600;
    modal.render(body, window, cx)
}
