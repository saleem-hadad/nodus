//! The New coordinator dialog (spec 22, COORD-011), opened from the title
//! bar's `New` menu. The coordinator starts at once, a session of its own
//! that the board's other sessions report to.

use gpui::Focusable as _;
use gpui::component::Sizable as _;
use gpui::component::input::{InputEvent, Textarea, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    div, px,
};

use super::canvas::BoardCanvas;
use crate::components::button::button;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::native_tooltip;
use crate::theme::{self, fonts, tokens::*};

/// What a coordinator can look after: label, what to tell it.
pub const EXAMPLES: [(&str, &str); 3] = [
    (
        "Release in batches",
        "Look after this board's sessions. When a good batch of work has landed on main and nobody is in the middle of shipping, cut a release: merge what's ready one at a time, bump the version, and publish. Ask me before you publish.",
    ),
    (
        "Keep main green",
        "When main's CI fails, find out why and start a session to fix it. Tell me what broke and what you did about it.",
    ),
    (
        "Review pull requests",
        "When a session opens a pull request, start a session to review it against our conventions, and send what it finds back to the session that opened it.",
    ),
];

const PLACEHOLDER: &str = "For example: when a good batch of work has landed on main, cut a release.";
const LINE: f32 = 13. * 1.45;
const FIELD_H: f32 = 4. * LINE + 20.;
const WRAP_MARGIN: f32 = 10.;

/// The dialog's state while it's up.
#[derive(Default)]
pub struct DialogUi {
    field: Option<Entity<TextareaState>>,
    creating: bool,
    error: Option<String>,
    _sub: Option<Subscription>,
}

/// The dialog, while the canvas has it up.
pub fn render(canvas: &BoardCanvas, window: &mut Window, cx: &mut Context<'_, BoardCanvas>) -> Option<AnyElement> {
    let state = window.use_keyed_state(ElementId::Name("coordinator-dialog".into()), cx, |_, _| {
        DialogUi::default()
    });
    if !canvas.coordinator_dialog {
        if state.read(cx).field.is_some() || state.read(cx).error.is_some() {
            state.update(cx, |s, _| *s = DialogUi::default());
        }
        return None;
    }
    let t = theme::tokens(cx);
    let me = cx.entity().downgrade();
    let field = match state.read(cx).field.clone() {
        Some(field) => field,
        None => {
            let field = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(4)
                    .soft_wrap(true)
                    .context_menu(false)
            });
            let watch = state.clone();
            let me = me.clone();
            let sub = window.subscribe(&field, cx, move |field, e: &InputEvent, _, cx| match e {
                InputEvent::Change => watch.update(cx, |_, cx| cx.notify()),
                InputEvent::PressEnter { secondary: true, .. } => start(&me, &watch, &field, cx),
                _ => {}
            });
            state.update(cx, |s, _| {
                s.field = Some(field.clone());
                s._sub = Some(sub);
            });
            field
        }
    };
    let (creating, error) = {
        let s = state.read(cx);
        (s.creating, s.error.clone())
    };
    let prompt = field.read(cx).value().to_string();
    let blank = nodus_domain::js::trim(&prompt).is_empty();
    let handle = field.read(cx).focus_handle(cx);
    crate::views::inputs::focus_once("coordinator-dialog-focus", &handle, window, cx);
    let focused = handle.is_focused(window);
    let viewport = window.viewport_size();

    let close = {
        let me = me.clone();
        move |_: &mut Window, cx: &mut App| {
            if let Some(c) = me.upgrade() {
                c.update(cx, BoardCanvas::close_coordinator_dialog);
            }
        }
    };
    let go = {
        let (me, state, field) = (me.clone(), state.clone(), field.clone());
        move |_: &mut Window, cx: &mut App| start(&me, &state, &field, cx)
    };
    let go_key = go.clone();

    let examples: Vec<AnyElement> = EXAMPLES
        .iter()
        .enumerate()
        .map(|(i, (label, text))| {
            let field = field.clone();
            let active = prompt == *text;
            div()
                .id(ElementId::Name(format!("coordinator-chip-{i}").into()))
                .flex()
                .items_center()
                .h(px(26.))
                .px(px(11.))
                .border(px(0.5))
                .border_color(if active { t.accent } else { t.border_strong })
                .rounded(px(13.))
                .bg(if active {
                    alpha(t.accent, 0.12)
                } else {
                    t.bg_elevated.into()
                })
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .text_color(if active { t.accent } else { t.text_primary })
                .when(creating, |d| d.opacity(0.45))
                .tooltip(native_tooltip((*text).to_owned().into()))
                .child(*label)
                .when(!creating, |d| {
                    d.on_click(move |_, window, cx| {
                        field.update(cx, |f, cx| f.set_value(text.to_string(), window, cx));
                    })
                })
                .into_any_element()
        })
        .collect();
    let small = |text: &'static str, color| {
        div()
            .text_size(TEXT_SM)
            .line_height(px(16.8))
            .text_color(color)
            .child(text)
    };

    let content = div()
        .flex()
        .flex_col()
        .min_h_0()
        .max_h(px((f32::from(viewport.height) * 0.86).min(720.)))
        .capture_key_down(move |e: &KeyDownEvent, window, cx| {
            let m = &e.keystroke.modifiers;
            let mod_key = if cfg!(target_os = "macos") { m.platform } else { m.control };
            if e.keystroke.key == "enter" && mod_key {
                cx.stop_propagation();
                go_key(window, cx);
            }
        })
        .on_key_down({
            let close = close.clone();
            move |e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    close(window, cx);
                }
            }
        })
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .flex_none()
                .pt(px(20.))
                .px(px(22.))
                .pb(px(16.))
                .border_b(px(0.5))
                .border_color(t.separator)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_none()
                        .size(px(32.))
                        .rounded(px(9.))
                        .bg(t.bg_hover)
                        .child(icon(Icon::Network, 16., 2.0, t.text_primary)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .text_size(TEXT_LG)
                                        .line_height(px(21.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.text_primary)
                                        .child("New coordinator"),
                                )
                                .child(
                                    div()
                                        .px(px(6.))
                                        .rounded(px(5.))
                                        .border(px(0.5))
                                        .border_color(t.border_strong)
                                        .text_size(px(11.))
                                        .line_height(px(16.))
                                        .text_color(t.text_secondary)
                                        .child("Beta"),
                                ),
                        )
                        .child(div().mt(px(2.)).child(small(
                            "It looks after this board’s sessions: they report to it when they finish, and it can start sessions of its own. Tell it what to look after.",
                            t.text_secondary,
                        ))),
                ),
        )
        .child(
            div()
                .id("coordinator-dialog-body")
                .flex()
                .flex_col()
                .gap(px(14.))
                .flex_1()
                .min_h_0()
                .py(px(16.))
                .px(px(22.))
                .overflow_y_scroll()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .border(px(0.5))
                        .border_color(if focused { t.accent } else { t.border_strong })
                        .rounded(px(8.))
                        .bg(t.bg_window)
                        .when(focused, |d| d.shadow(vec![shadow(0., 0., 0., 3., t.focus_ring)]))
                        .when(creating, |d| d.opacity(0.6))
                        .font_family(fonts::UI)
                        .text_size(TEXT_MD)
                        .line_height(px(LINE))
                        .text_color(t.text_primary)
                        .when(prompt.is_empty(), |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .top(px(10.))
                                    .left(px(12.))
                                    .right(px(12.))
                                    .text_color(t.text_tertiary)
                                    .child(PLACEHOLDER),
                            )
                        })
                        .child(
                            Textarea::new(&field)
                                .appearance(false)
                                .with_size(gpui::component::Size::Large)
                                .disabled(creating)
                                .text_size(TEXT_MD)
                                .line_height(px(LINE))
                                .fractional_line_height()
                                .h(px(FIELD_H))
                                .mr(px(-WRAP_MARGIN)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(TEXT_SM)
                                .line_height(px(16.8))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text_secondary)
                                .child("Start from an example"),
                        )
                        .child(div().flex().flex_wrap().gap(px(6.)).children(examples)),
                )
                .child(small(
                    "Every session on this board reports to it, including the ones you start. It works in auto mode, and asks you before it merges, pushes, publishes a release or deletes a branch.",
                    t.text_secondary,
                )),
        )
        .when_some(error, |d, error| {
            d.child(
                div()
                    .mx(px(22.))
                    .mb(px(12.))
                    .py(px(8.))
                    .px(px(12.))
                    .rounded(px(8.))
                    .bg(alpha(t.status_waiting, 0.10))
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.text_primary)
                    .child(error),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .flex_none()
                .py(px(12.))
                .px(px(22.))
                .border_t(px(0.5))
                .border_color(t.separator)
                .child(div().flex_1())
                .child(
                    button("coordinator-dialog-cancel")
                        .ghost()
                        .label("Cancel")
                        .disabled(creating)
                        .on_click({
                            let close = close.clone();
                            move |_, window, cx| close(window, cx)
                        }),
                )
                .child(
                    button("coordinator-dialog-start")
                        .primary()
                        .child(if creating { "Starting…" } else { "Start coordinator" })
                        .disabled(blank || creating)
                        .on_click(move |_, window, cx| go(window, cx)),
                ),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    let width = px(600.0_f32.min(f32::from(viewport.width) - 48.));
    Some(Modal::new(ElementId::Name("coordinator-dialog-modal".into()), width, close).render(content, window, cx))
}

/// COORD-012: starts the coordinator; the dialog closes on success and says
/// why not on failure.
fn start(me: &WeakEntity<BoardCanvas>, state: &Entity<DialogUi>, field: &Entity<TextareaState>, cx: &mut App) {
    let value = field.read(cx).value().to_string();
    let prompt = nodus_domain::js::trim(&value).to_owned();
    if prompt.is_empty() || state.read(cx).creating {
        return;
    }
    let Some(canvas) = me.upgrade() else { return };
    state.update(cx, |s, cx| {
        s.creating = true;
        s.error = None;
        cx.notify();
    });
    let task = canvas.update(cx, |c, cx| c.create_coordinator(&prompt, cx));
    let (me, state) = (me.clone(), state.clone());
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => {
                state.update(cx, |s, _| s.creating = false);
                if let Some(c) = me.upgrade() {
                    c.update(cx, BoardCanvas::close_coordinator_dialog);
                }
            }
            Err(err) => state.update(cx, |s, cx| {
                s.creating = false;
                s.error = Some(format!("Couldn’t start it: {err}"));
                cx.notify();
            }),
        });
    })
    .detach();
}
