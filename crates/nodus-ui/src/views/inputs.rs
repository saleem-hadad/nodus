//! The card's two text fields: the ask bar (VIEW-130, VIEW-131) and the
//! title editor (VIEW-127). Neither lets its keys, presses or double-clicks
//! reach the board.

use gpui::Focusable as _;
use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Entity, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    Styled, Subscription, Window, div, px,
};
use nodus_domain::BoardView;

use super::card::{CardUi, spinning};
use super::parts::{Btn, card_btn, nodus, on_core};
use crate::board::nodes::BoardCx;
use crate::board::units::fu;
use crate::components::icon::{Icon, icon};
use crate::components::text_field::{focus_all, input, new_input};
use crate::theme::{self, fonts};

/// The ask bar's field and whether it's sending.
pub struct AskBar {
    pub input: Entity<InputState>,
    pub sending: bool,
    _subs: Vec<Subscription>,
}

/// The rename field.
pub struct TitleEditor {
    pub input: Entity<InputState>,
    _sub: Subscription,
}

/// The ask bar (`.view-ask`): what to change, and Send.
pub fn ask_bar(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let project = board.project_id.to_string();
    let existing = ui.read(cx).ask.as_ref().map(|a| (a.input.clone(), a.sending));
    let (field, sending) = match existing {
        Some(found) => found,
        None => {
            let field = new_input("", "Tell Claude what to change…", false, window, cx);
            let (ui2, p, v) = (ui.clone(), project.clone(), view.id.clone());
            let ui3 = ui.clone();
            let subs = vec![window.subscribe(&field, cx, move |_, e: &InputEvent, _, cx| match e {
                InputEvent::PressEnter { .. } => send(&ui2, &p, &v, cx),
                InputEvent::Change => ui3.update(cx, |_, cx| cx.notify()),
                _ => {}
            })];
            // Focused on open, without scrolling (VIEW-131).
            let focus = field.read(cx).focus_handle(cx);
            focus_soon(focus, window);
            ui.update(cx, |u, _| {
                u.ask = Some(AskBar {
                    input: field.clone(),
                    sending: false,
                    _subs: subs,
                });
            });
            (field, false)
        }
    };
    let value = field.read(cx).value().to_string();
    let blank = nodus_domain::js::trim(&value).is_empty();
    let focused = field.read(cx).focus_handle(cx).is_focused(window);
    let (ui_send, p, v) = (ui.clone(), project, view.id.clone());
    let close = ui.clone();
    let send_label: AnyElement = if sending {
        spinning(
            icon(Icon::Loader2, 14.0 * z, 2.0, t.text_on_accent),
            gpui::ElementId::Name(format!("view-ask-spin-{}", view.id).into()),
            cx,
        )
    } else {
        div().child("Send").into_any_element()
    };
    div()
        .flex()
        .flex_none()
        .gap(fu(6.))
        .p(fu(8.))
        .border_t(fu(0.5))
        .border_color(t.separator)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .capture_key_down(move |e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "escape" {
                cx.stop_propagation();
                close.update(cx, |u, cx| {
                    u.asking = false;
                    u.ask = None;
                    cx.notify();
                });
            }
        })
        .child(
            div()
                .id(gpui::ElementId::Name(format!("view-ask-field-{}", view.id).into()))
                .flex()
                .items_center()
                .flex_1()
                .min_w_0()
                .h(fu(28.))
                .px(fu(10.))
                .border(fu(0.5))
                .border_color(if focused { t.accent } else { t.border_strong })
                .rounded(fu(6.))
                .bg(t.bg_window)
                .when(focused, |d| {
                    d.shadow(vec![theme::tokens::shadow(0.0, 0.0, 0.0, 3.0 * z, t.focus_ring)])
                })
                .when(sending, |d| d.opacity(0.6))
                .font_family(fonts::UI)
                .text_size(fu(13.))
                .text_color(t.text_primary)
                // `nowheel`: the field's wheel doesn't pan the board.
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .child(input(&field, px(18.0 * z), false)),
        )
        .child(card_btn(
            Some((board, &view.id)),
            format!("view-ask-send-{}", view.id),
            Btn::Primary,
            None,
            Some(send_label),
            blank || sending,
            z,
            move |_, cx| send(&ui_send, &p, &v, cx),
            window,
            cx,
        ))
        .into_any_element()
}

/// Enter or Send: the trimmed request goes to the view's brain; the bar
/// closes when it's taken, or shows why not (VIEW-131).
fn send(ui: &Entity<CardUi>, project_id: &str, view_id: &str, cx: &mut App) {
    let Some((field, sending)) = ui.read(cx).ask.as_ref().map(|a| (a.input.clone(), a.sending)) else {
        return;
    };
    let value = field.read(cx).value().to_string();
    let prompt = nodus_domain::js::trim(&value).to_owned();
    if prompt.is_empty() || sending || project_id.is_empty() {
        return;
    }
    ui.update(cx, |u, cx| {
        if let Some(a) = u.ask.as_mut() {
            a.sending = true;
        }
        cx.notify();
    });
    field.update(cx, |f, cx| f.set_disabled(true, cx));
    let nodus = nodus(cx);
    let (p, v) = (project_id.to_owned(), view_id.to_owned());
    let ui = ui.clone();
    on_core(
        cx,
        async move {
            nodus
                .views()
                .ask(&p, &v, &prompt)
                .await
                .map_err(|e| super::reason(&e.ipc_message()))
        },
        move |result, cx| match result {
            Ok(()) => ui.update(cx, |u, cx| {
                u.asking = false;
                u.ask = None;
                cx.notify();
            }),
            Err(reason) => {
                CardUi::set_error(&ui, reason, cx);
                ui.update(cx, |u, cx| {
                    if let Some(a) = u.ask.as_mut() {
                        a.sending = false;
                    }
                    cx.notify();
                });
                field.update(cx, |f, cx| f.set_disabled(false, cx));
            }
        },
    );
}

/// Focuses `handle` once a frame has drawn it: a handle no frame has drawn
/// yet doesn't keep the focus (as `text_field::focus_all`).
pub fn focus_soon(handle: gpui::FocusHandle, window: &mut Window) {
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, cx| window.focus(&handle, cx));
    });
}

/// The rename field (`.view-title-input`): the title, focused and all
/// selected; Enter or leaving commits once, Escape cancels (VIEW-127).
pub fn title_editor(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let existing = ui.read(cx).title.as_ref().map(|e| e.input.clone());
    let field = match existing {
        Some(field) => field,
        None => {
            let field = new_input(&view.title, "View name", false, window, cx);
            let done = std::rc::Rc::new(std::cell::Cell::new(false));
            let (actions, id, title) = (board.actions.clone(), view.id.clone(), view.title.clone());
            let sub = window.subscribe(&field, cx, move |field, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) && !done.replace(true) {
                    let value = field.read(cx).value().to_string();
                    let trimmed = nodus_domain::js::trim(&value).to_owned();
                    if !trimmed.is_empty() && trimmed != title {
                        actions.rename_view(&id, &trimmed, cx);
                    }
                    actions.set_renaming(None, cx);
                }
            });
            focus_all(&field, window, cx);
            ui.update(cx, |u, _| {
                u.title = Some(TitleEditor {
                    input: field.clone(),
                    _sub: sub,
                });
            });
            field
        }
    };
    let (actions, ui_cancel) = (board.actions.clone(), ui.clone());
    div()
        .id(gpui::ElementId::Name(format!("view-title-input-{}", view.id).into()))
        .flex()
        .items_center()
        .flex_1()
        .min_w_0()
        .h(fu(26.))
        .px(fu(8.))
        .border(fu(0.5))
        .border_color(t.accent)
        .rounded(fu(6.))
        .bg(t.bg_window)
        .shadow(vec![theme::tokens::shadow(0.0, 0.0, 0.0, 3.0 * z, t.focus_ring)])
        .font_family(fonts::DISPLAY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(fu(13.))
        .text_color(t.text_primary)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .capture_key_down(move |e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "escape" {
                cx.stop_propagation();
                // Cancelled: dropping the field doesn't commit it.
                ui_cancel.update(cx, |u, _| u.title = None);
                actions.set_renaming(None, cx);
            }
        })
        .child(input(&field, px(18.0 * z), false))
        .into_any_element()
}
