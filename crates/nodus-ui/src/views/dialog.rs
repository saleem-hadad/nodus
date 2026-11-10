//! The New view dialog (`ViewDialog.tsx`, VIEW-151…154), opened from the
//! pane menu's `New view here…` and the New Session button's `New view…`
//! (the canvas's `view_dialog`). Claude builds the view: its brain starts on
//! the description and the view shows up on the board at once, building.

use gpui::Focusable as _;
use gpui::component::Sizable as _;
use gpui::component::input::{InputEvent, Textarea, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, CursorStyle, DispatchPhase, ElementId, Entity, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathBuilder, Pixels, Rgba, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, canvas, div,
    linear_color_stop, linear_gradient, point, px,
};

use super::parts::fade;
use crate::board::canvas::BoardCanvas;
use crate::components::button::button;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::native_tooltip;
use crate::theme::{self, fonts, tokens::*};

/// Descriptions to start from (VIEW-153): clicking one fills the box.
pub const EXAMPLES: [(&str, &str); 5] = [
    (
        "Vercel deployments",
        "A table of the last 3 Vercel deployments of this project: the commit id, who deployed it, its status and when. Refresh every 10 minutes.",
    ),
    (
        "Users in the database",
        "The total number of users in the database, as one big number with when it was counted. Refresh every hour.",
    ),
    (
        "My Linear tickets",
        "All my open Linear tickets as a table: id, title, status and priority, each linking to the ticket. Refresh every hour.",
    ),
    (
        "Open pull requests",
        "The open pull requests on GitHub for this repository, with their author, CI status and age. Refresh every 15 minutes.",
    ),
    (
        "Board at a glance",
        "How this board is doing: how many sessions are working, waiting for me or done, and a list of the ones waiting for me that opens them when clicked.",
    ),
];

/// The box's placeholder, drawn by the dialog: the field's own stays on one
/// line, where Chromium wraps a textarea's.
const PLACEHOLDER: &str =
    "For example: a table of the last 3 Vercel deployments, with the commit, who deployed it and the status.";
/// `.action-description`: 13px at 1.45, `rows="5"`, 10px 12px of padding.
const LINE: f32 = 13. * 1.45;
const PAD_Y: f32 = 10.;
const PAD_X: f32 = 12.;
/// The box's height at 5 rows, and the least its grip shrinks it to (a line).
const FIELD_H: f32 = 5. * LINE + 2. * PAD_Y;
const FIELD_MIN_H: f32 = LINE + 2. * PAD_Y;
/// The field wraps 10px short of its width (gpui-component's RIGHT_MARGIN);
/// Chromium wraps at the padding.
const WRAP_MARGIN: f32 = 10.;

/// The dialog's state while it's up.
#[derive(Default)]
pub struct DialogUi {
    field: Option<Entity<TextareaState>>,
    creating: bool,
    error: Option<String>,
    /// The box's height once its grip has been dragged (`resize: vertical`).
    field_h: Option<f32>,
    /// The grip's drag: where it started, and the height then.
    grip: Option<(Pixels, f32)>,
    _sub: Option<Subscription>,
}

/// The dialog, while the canvas has it up.
pub fn render(canvas: &BoardCanvas, window: &mut Window, cx: &mut Context<'_, BoardCanvas>) -> Option<AnyElement> {
    let state = window.use_keyed_state(ElementId::Name("view-dialog".into()), cx, |_, _| DialogUi::default());
    let Some(at) = canvas.view_dialog else {
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
            let field = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(5)
                    .soft_wrap(true)
                    .context_menu(false)
            });
            let watch = state.clone();
            let me = me.clone();
            let sub = window.subscribe(&field, cx, move |field, e: &InputEvent, _, cx| match e {
                InputEvent::Change => watch.update(cx, |_, cx| cx.notify()),
                // VIEW-154: ⌘Enter in the box (the field takes it as its
                // secondary Enter before the dialog's key listener sees it).
                InputEvent::PressEnter { secondary: true, .. } => create_view(&me, &watch, &field, at, cx),
                _ => {}
            });
            state.update(cx, |s, _| {
                s.field = Some(field.clone());
                s._sub = Some(sub);
            });
            field
        }
    };
    let (creating, error, field_h) = {
        let s = state.read(cx);
        (s.creating, s.error.clone(), s.field_h.unwrap_or(FIELD_H))
    };
    let description = field.read(cx).value().to_string();
    let blank = nodus_domain::js::trim(&description).is_empty();
    // autoFocus.
    let handle = field.read(cx).focus_handle(cx);
    super::inputs::focus_once("view-dialog-focus", &handle, window, cx);
    let focused = handle.is_focused(window);
    let viewport = window.viewport_size();

    let close = {
        let me = me.clone();
        move |_: &mut Window, cx: &mut App| {
            if let Some(c) = me.upgrade() {
                c.update(cx, BoardCanvas::close_view_dialog);
            }
        }
    };
    let create = {
        let (me, state, field) = (me.clone(), state.clone(), field.clone());
        move |_: &mut Window, cx: &mut App| create_view(&me, &state, &field, at, cx)
    };
    let create_key = create.clone();

    let chips: Vec<AnyElement> = EXAMPLES
        .iter()
        .enumerate()
        .map(|(i, (label, text))| {
            let active = description == *text;
            let hovered =
                window.use_keyed_state(ElementId::Name(format!("view-chip-hover-{i}").into()), cx, |_, _| false);
            let hot = *hovered.read(cx) && !creating;
            let (field, h) = (field.clone(), hovered.clone());
            div()
                .id(ElementId::Name(format!("view-chip-{i}").into()))
                .flex()
                .items_center()
                .h(px(26.))
                .px(px(11.))
                .border(px(0.5))
                .border_color(if active { t.accent } else { t.border_strong })
                .rounded(px(13.))
                .bg(if active {
                    fade(t.accent, 0.12)
                } else if hot {
                    t.bg_hover
                } else {
                    t.bg_elevated
                })
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .text_color(if active { t.accent } else { t.text_primary })
                .when(creating, |d| d.opacity(0.45))
                .child(*label)
                .when(!creating, |d| {
                    d.on_hover(move |v, _, cx| {
                        h.update(cx, |s, cx| {
                            *s = *v;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, window, cx| {
                        field.update(cx, |f, cx| f.set_value(text.to_string(), window, cx));
                    })
                })
                .tooltip(native_tooltip((*text).into()))
                .into_any_element()
        })
        .collect();

    let content = div()
        .flex()
        .flex_col()
        .min_h_0()
        .max_h(px((f32::from(viewport.height) * 0.86).min(760.)))
        // VIEW-154: ⌘Enter (Ctrl+Enter elsewhere) anywhere in the dialog.
        .capture_key_down(move |e: &KeyDownEvent, window, cx| {
            let m = &e.keystroke.modifiers;
            let mod_key = if cfg!(target_os = "macos") { m.platform } else { m.control };
            if e.keystroke.key == "enter" && mod_key {
                cx.stop_propagation();
                create_key(window, cx);
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
                        .child(icon(Icon::LayoutDashboard, 16., 2.2, hex(0xffffff))),
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
                                .child("New view"),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_size(TEXT_SM)
                                .line_height(px(16.8))
                                .text_color(t.text_secondary)
                                .child(
                                    "Say what it should show and where the data comes from. Claude builds it on the board and keeps it up to date.",
                                ),
                        ),
                ),
        )
        .child(
            div()
                .id("view-dialog-body")
                .flex()
                .flex_col()
                .gap(px(14.))
                .flex_1()
                .min_h_0()
                .py(px(16.))
                .px(px(22.))
                .overflow_y_scroll()
                .child(
                    // `.action-input.action-description`: 0.5px border, 8px
                    // corners, 10px 12px of padding (Size::Large's), five
                    // rows, the placeholder wrapping, and the grip
                    // `resize: vertical` gives it.
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
                        // Under the field, so its caret draws over it.
                        .when(description.is_empty(), |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .top(px(PAD_Y))
                                    .left(px(PAD_X))
                                    .right(px(PAD_X))
                                    .max_h(px((field_h - 2. * PAD_Y).max(0.)))
                                    .overflow_hidden()
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
                                .h(px(field_h))
                                .mr(px(-WRAP_MARGIN)),
                        )
                        .child(grip(&state, &t, cx)),
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
                        .child(div().flex().flex_wrap().gap(px(6.)).children(chips)),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.text_secondary)
                        .child(
                            "Its page runs in a sandbox with no network. To fetch data it runs commands you allow on the card, or asks its own Claude session, which costs tokens each time.",
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
                    .bg(fade(t.status_waiting, 0.10))
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
                    button("view-dialog-cancel")
                        .ghost()
                        .label("Cancel")
                        .disabled(creating)
                        .on_click({
                            let close = close.clone();
                            move |_, window, cx| close(window, cx)
                        }),
                )
                .child({
                    let b = button("view-dialog-create").primary();
                    let b = if creating {
                        b.child(crate::views::card::spinning(
                            icon(Icon::Loader2, 14., 2., t.text_on_accent),
                            "view-dialog-spin",
                            cx,
                        ))
                    } else {
                        b
                    };
                    b.child("Create view")
                        .disabled(blank || creating)
                        .on_click(move |_, window, cx| create(window, cx))
                }),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    let width = px(620.0_f32.min(f32::from(viewport.width) - 48.));
    Some(Modal::new(ElementId::Name("view-dialog-modal".into()), width, close).render(content, window, cx))
}

/// The box's resize grip (`resize: vertical`): Chromium's two diagonal
/// strokes in its bottom-right corner, shaded in the dark theme (as the
/// merge dialog's instruction draws them). Dragging it makes the box taller
/// or shorter, a line at the least; the dialog's body scrolls past its height.
fn grip(state: &Entity<DialogUi>, t: &Tokens, cx: &App) -> AnyElement {
    let (line, shade) = if t.dark {
        (rgba(255, 255, 255, 0.62), Some(rgba(0, 0, 0, 0.9)))
    } else {
        (rgba(0, 0, 0, 0.55), None)
    };
    let dragging = state.read(cx).grip.is_some();
    let drag = state.clone();
    let strokes = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // The corner, inside the box's border.
            let c = point(bounds.right(), bounds.bottom());
            let seg = |a: (f32, f32), b: (f32, f32), d: f32, color: Rgba, window: &mut Window| {
                let mut path = PathBuilder::stroke(px(1.));
                path.move_to(point(c.x + px(a.0 + d), c.y + px(a.1 + d)));
                path.line_to(point(c.x + px(b.0 + d), c.y + px(b.1 + d)));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            let long = ((-7.8, -1.45), (-0.9, -8.35));
            let short = ((-3.75, -1.15), (-0.9, -3.95));
            for (a, b) in [long, short] {
                if let Some(shade) = shade {
                    seg(a, b, -0.5, shade, window);
                }
                seg(a, b, 0., line, window);
            }
            if dragging {
                let s = drag.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        s.update(cx, |s, cx| {
                            if let Some((y, h)) = s.grip {
                                s.field_h = Some((h + f32::from(e.position.y - y)).max(FIELD_MIN_H));
                                cx.notify();
                            }
                        });
                    }
                });
                let s = drag.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        s.update(cx, |s, cx| {
                            s.grip = None;
                            cx.notify();
                        });
                    }
                });
            }
        },
    )
    .size_full();
    let press = state.clone();
    div()
        .id("view-dialog-grip")
        .absolute()
        .right(px(0.5))
        .bottom(px(0.5))
        .size(px(12.))
        // The corner is the grip's, not the text's (Chromium's resizer).
        .occlude()
        .cursor(CursorStyle::ResizeUpDown)
        .on_mouse_down(MouseButton::Left, move |e, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            press.update(cx, |s, cx| {
                s.grip = Some((e.position.y, s.field_h.unwrap_or(FIELD_H)));
                cx.notify();
            });
        })
        .child(strokes)
        .into_any_element()
}

/// VIEW-154: creates the view; the dialog closes on success, says why not on
/// failure. Closing it meanwhile leaves the creation going.
fn create_view(
    me: &WeakEntity<BoardCanvas>,
    state: &Entity<DialogUi>,
    field: &Entity<TextareaState>,
    at: Option<nodus_board::geometry::Point>,
    cx: &mut App,
) {
    let value = field.read(cx).value().to_string();
    let what = nodus_domain::js::trim(&value).to_owned();
    if what.is_empty() || state.read(cx).creating {
        return;
    }
    let Some(canvas) = me.upgrade() else { return };
    state.update(cx, |s, cx| {
        s.creating = true;
        s.error = None;
        cx.notify();
    });
    let task = canvas.update(cx, |c, cx| c.create_view(what, at, cx));
    let (me, state) = (me.clone(), state.clone());
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => {
                state.update(cx, |s, _| s.creating = false);
                if let Some(c) = me.upgrade() {
                    c.update(cx, BoardCanvas::close_view_dialog);
                }
            }
            Err(err) => state.update(cx, |s, cx| {
                s.creating = false;
                s.error = Some(format!(
                    "Couldn’t start building it: {}",
                    super::reason(&err.ipc_message())
                ));
                cx.notify();
            }),
        });
    })
    .detach();
}
