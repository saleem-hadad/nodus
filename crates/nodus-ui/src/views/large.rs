//! A view open large (`ViewModal`, VIEW-133, VIEW-147): a modal over the
//! board with a header and a second frame of the same page filling the body.
//! Escape (while the board, not the page, has the keyboard), a press on the
//! backdrop or ✕ closes it.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, Context, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, StatefulInteractiveElement, Styled, Window, div, img, px,
};

use super::frames::{self, Geometry, Slot};
use crate::board::canvas::BoardCanvas;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::native_tooltip;
use crate::theme::{self, Tokens, fonts, tokens::*};

/// Opens a view large.
pub fn open(view_id: &str, window: &mut Window, cx: &mut App) {
    let rc = frames::frames(cx);
    rc.borrow_mut().large = Some(view_id.to_owned());
    window.refresh();
}

/// Closes the large window.
pub fn close(window: &mut Window, cx: &mut App) {
    if let Some(rc) = frames::existing(cx) {
        rc.borrow_mut().large = None;
    }
    window.refresh();
}

/// The large window, while a view is open large.
pub fn render(canvas: &BoardCanvas, window: &mut Window, cx: &mut Context<'_, BoardCanvas>) -> Option<AnyElement> {
    let id = frames::existing(cx)?.borrow().large.clone()?;
    let view = canvas.board()?.views.iter().find(|v| v.id == id)?.clone();
    let t = theme::tokens(cx);
    let viewport = window.viewport_size();
    let focus = window.use_keyed_state(ElementId::Name("view-modal-focus".into()), cx, |_, cx| {
        (cx.focus_handle(), false)
    });
    let (handle, focused_once) = focus.read(cx).clone();
    if !focused_once {
        focus.update(cx, |f, _| f.1 = true);
        let h = handle.clone();
        window.on_next_frame(move |window, cx| window.focus(&h, cx));
    }
    let shown = frames::shown(&id, Slot::Large, cx);
    let parity = crate::parity::frozen(cx);
    let ink = super::swap::ink_tool_out(cx);
    let geometry = move |document: Bounds<Pixels>, clip: Bounds<Pixels>| Geometry {
        document,
        clip,
        zoom: 1.0,
        radius: 18.0,
        forced: parity || ink,
        passthrough: false,
        edge: 0.0,
    };
    let reset = focus.clone();
    let on_close = move |window: &mut Window, cx: &mut App| {
        reset.update(cx, |f, _| f.1 = false);
        close(window, cx);
    };
    let close_x = on_close.clone();
    let close_key = on_close.clone();
    let width = px(1400.0_f32.min(f32::from(viewport.width) * 0.92));
    let content = div()
        .id(ElementId::Name(format!("view-modal-{id}").into()))
        .track_focus(&handle)
        .flex()
        .flex_col()
        .w_full()
        .h(viewport.height * 0.88)
        .max_h(viewport.height - px(48.))
        .on_key_down(move |e: &KeyDownEvent, window, cx| {
            if e.keystroke.key == "escape" {
                cx.stop_propagation();
                close_key(window, cx);
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .flex_none()
                .h(px(44.))
                .pl(px(16.))
                .pr(px(8.))
                .border_b(px(0.5))
                .border_color(t.separator)
                .child(icon(Icon::LayoutDashboard, 15., 2., t.text_tertiary))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(fonts::DISPLAY)
                        .text_size(TEXT_LG)
                        .line_height(px(19.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text_primary)
                        .child(view.title.clone()),
                )
                .child(close_button(&t, move |window, cx| close_x(window, cx), window, cx)),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .bg(t.bg_elevated)
                .when_some(shown.snapshot, |d, image| {
                    d.child(
                        img(image)
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .rounded_bl(RADIUS_XL)
                            .rounded_br(RADIUS_XL),
                    )
                })
                .when(view.version > 0, |d| {
                    d.child(frames::probe(id.clone(), Slot::Large, geometry))
                }),
        );
    Some(Modal::new(ElementId::Name("view-modal".into()), width, on_close).render(content, window, cx))
}

/// ✕ (`.card-tool` with `X` 15): `Close`.
fn close_button(
    t: &Tokens,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let hovered = window.use_keyed_state(ElementId::Name("view-modal-close-hover".into()), cx, |_, _| false);
    let hot = *hovered.read(cx);
    let h = hovered.clone();
    div()
        .id("view-modal-close")
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .min_w(px(28.))
        .h(px(28.))
        .rounded(RADIUS_SM)
        .when(hot, |d| d.bg(t.bg_hover))
        .active(|s| s.bg(t.bg_pressed))
        .child(icon(
            Icon::X,
            15.,
            2.,
            if hot { t.text_primary } else { t.text_secondary },
        ))
        .on_hover(move |v, _, cx| {
            h.update(cx, |s, cx| {
                *s = *v;
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_click(window, cx))
        .tooltip(native_tooltip("Close".into()))
        .into_any_element()
}

/// The focus the large window holds, for the board's key routing.
pub fn focus_handle(window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(ElementId::Name("view-modal-focus".into()), cx, |_, cx| {
            (cx.focus_handle(), false)
        })
        .read(cx)
        .0
        .clone()
}
