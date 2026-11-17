//! The New goal dialog (spec 21, GOAL-010…GOAL-014), opened from New
//! Automation's menu, the board's `New goal here…` and a card's `Make this a
//! goal…` (the canvas's `goal_dialog`). The goal starts at once: its tile
//! goes up and its session starts on the first check-in.

use gpui::Focusable as _;
use gpui::component::Sizable as _;
use gpui::component::input::{InputEvent, Textarea, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    div, linear_color_stop, linear_gradient, px,
};

use super::canvas::BoardCanvas;
use super::goals::GoalAsk;
use crate::components::button::button;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::native_tooltip;
use crate::theme::{self, fonts, tokens::*};

/// GOAL-014: goals to start from: label, mission.
pub const EXAMPLES: [(&str, &str); 4] = [
    (
        "Signup conversion",
        "Get signup conversion from where it is now to 4% within a month, measured in our analytics.",
    ),
    (
        "Faster home page",
        "Get the home page’s largest contentful paint under 2 seconds on mobile.",
    ),
    (
        "Fewer errors",
        "Halve the number of unhandled errors in production, as our error tracking counts them.",
    ),
    (
        "Test coverage",
        "Get test coverage of the checkout code above 80%, without slowing the test suite down.",
    ),
];

/// GOAL-010: how often it may check in, at most: label, minutes.
pub const EVERY: [(&str, f64); 4] = [
    ("30 min", 30.0),
    ("1 hour", 60.0),
    ("6 hours", 360.0),
    ("1 day", 1440.0),
];

const PLACEHOLDER: &str = "For example: get signup conversion from 3.1% to 4% within a month.";
const LINE: f32 = 13. * 1.45;
const FIELD_H: f32 = 4. * LINE + 20.;
const WRAP_MARGIN: f32 = 10.;

/// The dialog's state while it's up.
#[derive(Default)]
pub struct DialogUi {
    field: Option<Entity<TextareaState>>,
    every: Option<f64>,
    creating: bool,
    error: Option<String>,
    _sub: Option<Subscription>,
}

/// The dialog, while the canvas has it up.
pub fn render(canvas: &BoardCanvas, window: &mut Window, cx: &mut Context<'_, BoardCanvas>) -> Option<AnyElement> {
    let state = window.use_keyed_state(ElementId::Name("goal-dialog".into()), cx, |_, _| DialogUi::default());
    let Some(ask) = canvas.goal_dialog.clone() else {
        if state.read(cx).field.is_some() || state.read(cx).error.is_some() {
            state.update(cx, |s, _| *s = DialogUi::default());
        }
        return None;
    };
    let t = theme::tokens(cx);
    let me = cx.entity().downgrade();
    let field = match state.read(cx).field.clone() {
        Some(field) => field,
        None => {
            // GOAL-012: a session's goal starts from what the session is about.
            let first = ask.session.as_ref().map(|(_, title)| title.clone()).unwrap_or_default();
            let field = cx.new(|cx| {
                let mut f = TextareaState::new(window, cx)
                    .rows(4)
                    .soft_wrap(true)
                    .context_menu(false);
                f.set_value(first, window, cx);
                f
            });
            let watch = state.clone();
            let (me, ask) = (me.clone(), ask.clone());
            let sub = window.subscribe(&field, cx, move |field, e: &InputEvent, _, cx| match e {
                InputEvent::Change => watch.update(cx, |_, cx| cx.notify()),
                InputEvent::PressEnter { secondary: true, .. } => start(&me, &watch, &field, &ask, cx),
                _ => {}
            });
            state.update(cx, |s, _| {
                s.field = Some(field.clone());
                s._sub = Some(sub);
            });
            field
        }
    };
    let (creating, error, every) = {
        let s = state.read(cx);
        (
            s.creating,
            s.error.clone(),
            s.every.unwrap_or(nodus_board::goals::DEFAULT_EVERY),
        )
    };
    let mission = field.read(cx).value().to_string();
    let blank = nodus_domain::js::trim(&mission).is_empty();
    let handle = field.read(cx).focus_handle(cx);
    crate::views::inputs::focus_once("goal-dialog-focus", &handle, window, cx);
    let focused = handle.is_focused(window);
    let viewport = window.viewport_size();

    let close = {
        let me = me.clone();
        move |_: &mut Window, cx: &mut App| {
            if let Some(c) = me.upgrade() {
                c.update(cx, BoardCanvas::close_goal_dialog);
            }
        }
    };
    let go = {
        let (me, state, field, ask) = (me.clone(), state.clone(), field.clone(), ask.clone());
        move |_: &mut Window, cx: &mut App| start(&me, &state, &field, &ask, cx)
    };
    let go_key = go.clone();

    let chip = |id: String, label: &str, active: bool, tip: Option<&str>| {
        div()
            .id(ElementId::Name(id.into()))
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
            .when_some(tip, |d, tip| d.tooltip(native_tooltip(tip.to_owned().into())))
            .child(label.to_owned())
    };
    let examples: Vec<AnyElement> = if ask.session.is_some() {
        Vec::new()
    } else {
        EXAMPLES
            .iter()
            .enumerate()
            .map(|(i, (label, text))| {
                let field = field.clone();
                chip(format!("goal-chip-{i}"), label, mission == *text, Some(text))
                    .when(!creating, |d| {
                        d.on_click(move |_, window, cx| {
                            field.update(cx, |f, cx| f.set_value(text.to_string(), window, cx));
                        })
                    })
                    .into_any_element()
            })
            .collect()
    };
    let paces: Vec<AnyElement> = EVERY
        .iter()
        .map(|(label, minutes)| {
            let s = state.clone();
            let minutes = *minutes;
            chip(
                format!("goal-every-{minutes}"),
                label,
                (every - minutes).abs() < 0.5,
                None,
            )
            .when(!creating, |d| {
                d.on_click(move |_, _, cx| {
                    s.update(cx, |s, cx| {
                        s.every = Some(minutes);
                        cx.notify();
                    });
                })
            })
            .into_any_element()
        })
        .collect();
    let label = |text: &'static str| {
        div()
            .text_size(TEXT_SM)
            .line_height(px(16.8))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.text_secondary)
            .child(text)
    };
    let (title, subtitle) = match &ask.session {
        Some(_) => (
            "Make this a goal",
            "Say what it should get to. This session works on it, checks in on its own and remembers what it tried.",
        ),
        None => (
            "New goal",
            "Say what you want to get to. Claude works on it in one session, checks in on its own and remembers what it tried.",
        ),
    };

    let content = div()
        .flex()
        .flex_col()
        .min_h_0()
        .max_h(px((f32::from(viewport.height) * 0.86).min(760.)))
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
                        .bg(linear_gradient(
                            135.,
                            linear_color_stop(t.accent, 0.),
                            linear_color_stop(hex(0x5e5ce6), 1.),
                        ))
                        .child(icon(Icon::Target, 16., 2.2, hex(0xffffff))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(TEXT_LG)
                                .line_height(px(21.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text_primary)
                                .child(title),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_size(TEXT_SM)
                                .line_height(px(16.8))
                                .text_color(t.text_secondary)
                                .child(subtitle),
                        ),
                ),
        )
        .child(
            div()
                .id("goal-dialog-body")
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
                        .when(mission.is_empty(), |d| {
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
                .when(!examples.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(label("Start from an example"))
                            .child(div().flex().flex_wrap().gap(px(6.)).children(examples)),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(label("Check in at most every"))
                        .child(div().flex().flex_wrap().gap(px(6.)).children(paces)),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.text_secondary)
                        .child(
                            "Claude picks when to look again, never sooner than this. It works in auto mode and asks you only when it isn’t sure. After 60 check-ins it pauses until you resume it.",
                        ),
                ),
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
                .child(button("goal-dialog-cancel").ghost().label("Cancel").disabled(creating).on_click({
                    let close = close.clone();
                    move |_, window, cx| close(window, cx)
                }))
                .child(
                    button("goal-dialog-start")
                        .primary()
                        .child(if creating { "Starting…" } else { "Start goal" })
                        .disabled(blank || creating)
                        .on_click(move |_, window, cx| go(window, cx)),
                ),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    let width = px(620.0_f32.min(f32::from(viewport.width) - 48.));
    Some(Modal::new(ElementId::Name("goal-dialog-modal".into()), width, close).render(content, window, cx))
}

/// GOAL-011: starts the goal; the dialog closes on success and says why not
/// on failure.
fn start(
    me: &WeakEntity<BoardCanvas>,
    state: &Entity<DialogUi>,
    field: &Entity<TextareaState>,
    ask: &GoalAsk,
    cx: &mut App,
) {
    let value = field.read(cx).value().to_string();
    let mission = nodus_domain::js::trim(&value).to_owned();
    if mission.is_empty() || state.read(cx).creating {
        return;
    }
    let Some(canvas) = me.upgrade() else { return };
    let every = state.read(cx).every.unwrap_or(nodus_board::goals::DEFAULT_EVERY);
    state.update(cx, |s, cx| {
        s.creating = true;
        s.error = None;
        cx.notify();
    });
    let task = canvas.update(cx, |c, cx| c.create_goal(&mission, every, ask, cx));
    let (me, state) = (me.clone(), state.clone());
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => {
                state.update(cx, |s, _| s.creating = false);
                if let Some(c) = me.upgrade() {
                    c.update(cx, BoardCanvas::close_goal_dialog);
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
