//! The card's parts (`ViewNode.tsx`, `views.css`): the status dot, the
//! tools (VIEW-128…134), the approvals strip (VIEW-135), the empty state
//! (VIEW-137, VIEW-138), the error line (VIEW-139) and the card's buttons,
//! all in flow units.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Bounds, ElementId, Entity, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Rgba, SharedString, StatefulInteractiveElement, Styled, Window,
    div, point, px, relative,
};
use nodus_domain::{BoardView, SessionStatus, ViewCommand};

use super::card::{CardUi, bounds_probe, more_icon, shadows, spinning, tip_over_page};
use super::icons::{ViewIcon, view_icon};
use crate::board::nodes::BoardCx;
use crate::board::units::fu;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::TooltipPress;
use crate::theme::{self, Tokens, fonts};

pub use super::inputs::{AskBar, TitleEditor, ask_bar, title_editor};

/// `color-mix(in srgb, a p, b)`, premultiplied as Chromium mixes.
pub fn mix(a: Rgba, p: f32, b: Rgba) -> Rgba {
    let alpha = p * a.a + (1.0 - p) * b.a;
    if alpha <= 0.0 {
        return Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        };
    }
    let ch = |x: f32, y: f32| (p * a.a * x + (1.0 - p) * b.a * y) / alpha;
    Rgba {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
        a: alpha,
    }
}

/// `c` at `p` of its alpha.
pub fn fade(c: Rgba, p: f32) -> Rgba {
    Rgba { a: c.a * p, ..c }
}

/// A command is waiting for the user (VIEW-017).
pub fn is_waiting(c: &ViewCommand) -> bool {
    c.approved.as_deref() != Some(c.run.as_str()) && c.denied.as_deref() != Some(c.run.as_str())
}

/// ⟳ shows when the view has a refresh or an allowed command (VIEW-128).
pub fn can_refresh(view: &BoardView) -> bool {
    view.refresh.is_some()
        || view
            .commands
            .as_ref()
            .is_some_and(|cs| cs.iter().any(|c| c.approved.as_deref() == Some(c.run.as_str())))
}

/// Runs a core command and hands its result back on the UI thread.
pub fn on_core<T: Send + 'static>(
    cx: &mut App,
    fut: impl Future<Output = T> + Send + 'static,
    done: impl FnOnce(T, &mut App) + 'static,
) {
    let rt = crate::board::core_rt(cx);
    cx.spawn(async move |cx| {
        let value = crate::board::on_core(&rt, fut).await;
        cx.update(|cx| done(value, cx));
    })
    .detach();
}

/// The core.
pub fn nodus(cx: &App) -> nodus_core::Nodus {
    cx.global::<crate::shell::window::Core>().0.clone()
}

/// The 7px status dot (`.status-dot.is-small`): working breathes, waiting
/// pulses (held at their first keyframe in parity shots).
pub fn status_dot(
    view_id: &str,
    status: SessionStatus,
    z: f32,
    tip: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let frozen = crate::parity::frozen(cx);
    let working = status == SessionStatus::Working;
    let fill = if working { t.status_working } else { t.status_waiting };
    let ring_at = move |delta: f32| -> gpui::BoxShadow {
        if working {
            // status-breathe-small, 1.6s ease-in-out: 2px at 22% ⇄ 3.5px at 10%.
            let e = (1.0 - (delta * std::f32::consts::TAU).cos()) / 2.0;
            let spread = 2.0 + 1.5 * e;
            let a = 0.22 - 0.12 * e;
            theme::tokens::shadow(0.0, 0.0, 0.0, spread * z, fade(fill, a))
        } else {
            // status-pulse, 1.1s ease-out: 0 at 55% → 8px transparent.
            let e = 1.0 - (1.0 - delta).powi(3);
            theme::tokens::shadow(0.0, 0.0, 0.0, 8.0 * e * z, fade(fill, 0.55 * (1.0 - e)))
        }
    };
    let id = ElementId::Name(format!("view-dot-{view_id}").into());
    let press = window.use_keyed_state((id.clone(), "tip"), cx, |_, _| TooltipPress::default());
    let leave = view_id.to_owned();
    let dot = div()
        .id(id.clone())
        .flex_none()
        .size(fu(7.))
        .rounded_full()
        .bg(fill)
        .shadow(vec![ring_at(0.0)])
        .on_hover(move |h, _, cx| {
            if !*h {
                super::swap::set_tooltip(&leave, false, cx);
            }
        })
        .tooltip(tip_over_page(view_id, tip.into(), press));
    if frozen {
        return dot.into_any_element();
    }
    let period = if working { 1600 } else { 1100 };
    let delta = crate::components::ambient::cycle(window, cx, (id, "breathe"), Duration::from_millis(period));
    dot.shadow(vec![ring_at(delta)]).into_any_element()
}

/// A `.card-tool` (28 square): `pressed` holds `--bg-pressed`
/// (`aria-expanded`), `disabled` fades it to 40%. It doesn't start drags.
#[allow(clippy::too_many_arguments)]
fn card_tool(
    board: &BoardCx,
    view_id: &str,
    name: &'static str,
    glyph: impl Fn(Rgba) -> AnyElement,
    tip: SharedString,
    pressed: bool,
    disabled: bool,
    on_click: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let key = format!("view-tool-{name}-{view_id}");
    let hovered = window.use_keyed_state(ElementId::Name(format!("{key}-hover").into()), cx, |_, _| false);
    let press = window.use_keyed_state(ElementId::Name(format!("{key}-tip").into()), cx, |_, _| {
        TooltipPress::default()
    });
    let bounds = window.use_keyed_state(ElementId::Name(format!("{key}-bounds").into()), cx, |_, _| {
        Rc::new(Cell::new(Bounds::<Pixels>::default()))
    });
    let bounds = bounds.read(cx).clone();
    let hot = *hovered.read(cx) && !disabled;
    let color = if hot || pressed {
        t.text_primary
    } else {
        t.text_secondary
    };
    let id = view_id.to_owned();
    let actions = board.actions.clone();
    div()
        .id(ElementId::Name(key.into()))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .min_w(fu(28.))
        .h(fu(28.))
        .rounded(fu(6.))
        .when(pressed, |d| d.bg(t.bg_pressed))
        .when(hot && !pressed, |d| d.bg(t.bg_hover))
        .when(disabled, |d| d.opacity(0.4))
        .child(glyph(color))
        .child(bounds_probe(bounds.clone()))
        .when(!disabled, |d| {
            let hover = hovered.clone();
            let leave = press.clone();
            let moved = press.clone();
            let down = press.clone();
            let tip_id = id.clone();
            d.on_hover(move |h, _, cx| {
                hover.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
                leave.update(cx, |p, _| p.hover(*h));
                if !*h {
                    super::swap::set_tooltip(&tip_id, false, cx);
                }
            })
            .on_mouse_move(move |_, _, cx| moved.update(cx, |p, _| p.moved()))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                // `nodrag`: the press is the tool's.
                cx.stop_propagation();
                down.update(cx, |p, _| p.press());
            })
            .active(|s| s.bg(t.bg_pressed))
            .on_click({
                let select_id = id.clone();
                move |e, window, cx| {
                    // The click still selects the view (React Flow's node click).
                    actions.click_select(&select_id, &e.modifiers(), cx);
                    on_click(bounds.get(), window, cx);
                }
            })
            .tooltip(tip_over_page(&id, tip, press.clone()))
        })
        .when(disabled, |d| {
            d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        })
        .into_any_element()
}

/// The tools, right-aligned: ⟳ ✎ ⌨ ⤢ ⋯ (VIEW-128…134).
pub fn tools(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let id = view.id.clone();
    let (refreshing, asking) = {
        let u = ui.read(cx);
        (u.refreshing, u.asking)
    };
    let mut out = Vec::new();
    if can_refresh(view) {
        let tip = if view.refresh.is_some() {
            "Refresh: run its commands, and ask its brain for fresh data"
        } else {
            "Refresh: run its commands"
        };
        let (ui2, project) = (ui.clone(), board.project_id.to_string());
        let view_id = id.clone();
        let spin_key = format!("view-refresh-spin-{id}");
        out.push(card_tool(
            board,
            &id,
            "refresh",
            move |c| {
                let glyph = icon(Icon::RefreshCw, 14.0 * z, 2.0, c);
                if refreshing {
                    spinning_glyph(glyph, spin_key.clone())
                } else {
                    glyph.into_any_element()
                }
            },
            tip.into(),
            false,
            refreshing,
            move |_, _, cx| refresh(&ui2, &project, &view_id, cx),
            window,
            cx,
        ));
    }
    let ask_ui = ui.clone();
    out.push(card_tool(
        board,
        &id,
        "change",
        move |c| icon(Icon::PencilLine, 14.0 * z, 2.0, c).into_any_element(),
        "Ask Claude to change this view".into(),
        asking,
        false,
        move |_, _, cx| {
            ask_ui.update(cx, |u, cx| {
                u.asking = !u.asking;
                if !u.asking {
                    u.ask = None;
                }
                cx.notify();
            });
        },
        window,
        cx,
    ));
    let (actions, brain_id) = (board.actions.clone(), id.clone());
    out.push(card_tool(
        board,
        &id,
        "session",
        move |c| icon(Icon::SquareTerminal, 14.0 * z, 2.0, c).into_any_element(),
        "Open its brain, the Claude session that builds this view".into(),
        false,
        false,
        move |_, _, cx| actions.open_view_session(&brain_id, cx),
        window,
        cx,
    ));
    let large_id = id.clone();
    out.push(card_tool(
        board,
        &id,
        "large",
        move |c| icon(Icon::Maximize2, 14.0 * z, 2.0, c).into_any_element(),
        "Open large".into(),
        false,
        view.version == 0,
        move |_, window, cx| super::large::open(&large_id, window, cx),
        window,
        cx,
    ));
    let (actions, menu_id) = (board.actions.clone(), id.clone());
    out.push(card_tool(
        board,
        &id,
        "more",
        move |c| more_icon(z, c).into_any_element(),
        "More".into(),
        false,
        false,
        move |b, _, cx| actions.view_menu(&menu_id, point(b.left(), b.bottom() + px(4.)), cx),
        window,
        cx,
    ));
    out
}

fn spinning_glyph(glyph: gpui::Svg, key: String) -> AnyElement {
    glyph
        .with_animation(
            ElementId::Name(key.into()),
            Animation::new(Duration::from_millis(900)).repeat(),
            |svg, delta| svg.with_transformation(gpui::Transformation::rotate(gpui::percentage(delta))),
        )
        .into_any_element()
}

/// VIEW-129: runs the view's commands (and asks its brain, with a refresh);
/// the first that failed shows on the card.
fn refresh(ui: &Entity<CardUi>, project_id: &str, view_id: &str, cx: &mut App) {
    if project_id.is_empty() || ui.read(cx).refreshing {
        return;
    }
    ui.update(cx, |u, cx| {
        u.refreshing = true;
        cx.notify();
    });
    let nodus = nodus(cx);
    let (p, v) = (project_id.to_owned(), view_id.to_owned());
    let ui = ui.clone();
    on_core(
        cx,
        async move {
            nodus
                .views()
                .refresh(&p, &v)
                .await
                .map_err(|e| super::reason(&e.ipc_message()))
        },
        move |results, cx| {
            ui.update(cx, |u, cx| {
                u.refreshing = false;
                cx.notify();
            });
            let message = match results {
                Ok(results) => results
                    .iter()
                    .find(|r| r.error.is_some() || r.code != Some(0))
                    .map(|r| match &r.error {
                        Some(error) => format!("“{}” failed: {error}", r.name),
                        None => format!(
                            "“{}” exited with code {}",
                            r.name,
                            r.code.map_or_else(|| "null".to_owned(), |c| c.to_string())
                        ),
                    }),
                Err(reason) => Some(reason),
            };
            if let Some(message) = message {
                CardUi::set_error(&ui, message, cx);
            }
        },
    );
}

/// A `.btn` in flow units: `.btn`, `.btn-primary` or `.btn-ghost`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Default,
    Primary,
    Ghost,
}

/// A card button (`nodrag`), `children` before `label`.
#[allow(clippy::too_many_arguments)]
pub fn card_btn(
    select: Option<(&BoardCx, &str)>,
    key: String,
    kind: Btn,
    label: Option<&'static str>,
    lead: Option<AnyElement>,
    disabled: bool,
    z: f32,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let hovered = window.use_keyed_state(ElementId::Name(format!("{key}-hover").into()), cx, |_, _| false);
    let hot = *hovered.read(cx) && !disabled;
    let select = select.map(|(b, id)| (b.actions.clone(), id.to_owned()));
    let (rest, hover, border, text, shadow) = match kind {
        Btn::Default => (t.bg_elevated, t.bg_hover, t.border_strong, t.text_primary, true),
        Btn::Primary => (t.accent, t.accent_hover, fade(t.accent, 0.0), t.text_on_accent, true),
        Btn::Ghost => (
            fade(t.bg_elevated, 0.0),
            t.bg_hover,
            fade(t.bg_elevated, 0.0),
            t.text_primary,
            false,
        ),
    };
    div()
        .id(ElementId::Name(key.into()))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(fu(6.))
        .h(fu(28.))
        .px(fu(12.))
        .rounded(fu(6.))
        .border(fu(0.5))
        .border_color(border)
        .bg(if hot { hover } else { rest })
        .when(shadow, |d| d.shadow(shadows(&t.shadow_sm, z)))
        .text_size(fu(13.))
        .line_height(fu(18.2))
        .font_family(fonts::UI)
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .text_color(text)
        .when(disabled, |d| d.opacity(0.45))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(!disabled, |d| {
            let h = hovered.clone();
            d.on_hover(move |v, _, cx| {
                h.update(cx, |s, cx| {
                    *s = *v;
                    cx.notify();
                });
            })
            .when(kind != Btn::Primary, |d| d.active(move |s| s.bg(t.bg_pressed)))
            .on_click(move |e, window, cx| {
                if let Some((actions, id)) = &select {
                    actions.click_select(id, &e.modifiers(), cx);
                }
                on_click(window, cx);
            })
        })
        .children(lead)
        .children(label)
        .into_any_element()
}

/// The error line (`.view-error`, `role="status"`).
pub fn error_line(message: String, _z: f32, t: &Tokens) -> AnyElement {
    div()
        .flex_none()
        .px(fu(12.))
        .py(fu(7.))
        .border_t(fu(0.5))
        .border_color(t.separator)
        .text_size(fu(12.))
        .line_height(fu(16.8))
        .text_color(t.status_waiting)
        .child(message)
        .into_any_element()
}

/// Commands waiting for the user (VIEW-135), drawn by the board outside the
/// page so the page can't draw over them.
pub fn approvals(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let waiting: Vec<ViewCommand> = view
        .commands
        .iter()
        .flatten()
        .filter(|c| is_waiting(c))
        .cloned()
        .collect();
    if waiting.is_empty() {
        return None;
    }
    let t = theme::tokens(cx);
    let mut items = Vec::new();
    for (i, c) in waiting.iter().enumerate() {
        let every = c
            .every
            .map(|e| format!(" {}", super::every_label(e)))
            .unwrap_or_default();
        let actions: Vec<AnyElement> = [(false, Btn::Ghost, "Don’t allow"), (true, Btn::Primary, "Allow")]
            .into_iter()
            .map(|(allow, kind, label)| {
                let (ui, project, view_id, name) = (
                    ui.clone(),
                    board.project_id.to_string(),
                    view.id.clone(),
                    c.name.clone(),
                );
                card_btn(
                    Some((board, &view.id)),
                    format!("view-approve-{allow}-{i}-{}", view.id),
                    kind,
                    Some(label),
                    None,
                    false,
                    z,
                    move |_, cx| approve(&ui, &project, &view_id, &name, allow, cx),
                    window,
                    cx,
                )
            })
            .collect();
        items.push(
            div()
                .flex()
                .flex_col()
                .gap(fu(8.))
                .child(
                    div()
                        .text_size(fu(12.))
                        .line_height(fu(16.8))
                        .text_color(t.text_secondary)
                        .child(format!("This view wants to run{every}:"))
                        .child(
                            div()
                                .id(ElementId::Name(format!("view-approve-cmd-{i}-{}", view.id).into()))
                                .mt(fu(5.))
                                .max_h(fu(72.))
                                .px(fu(8.))
                                .py(fu(6.))
                                .rounded(fu(6.))
                                .bg(t.bg_sunken)
                                .font_family(fonts::mono())
                                .text_size(fu(11.))
                                .line_height(fu(16.5))
                                .text_color(t.text_primary)
                                .overflow_y_scroll()
                                .child(c.run.clone()),
                        ),
                )
                .child(div().flex().justify_end().gap(fu(6.)).children(actions))
                .into_any_element(),
        );
    }
    Some(
        div()
            .id(ElementId::Name(format!("view-approvals-{}", view.id).into()))
            .flex()
            .flex_col()
            .gap(fu(8.))
            .flex_none()
            .max_h(relative(0.45))
            .px(fu(12.))
            .py(fu(10.))
            .border_b(fu(0.5))
            .border_color(t.separator)
            .bg(mix(t.accent, 0.07, t.bg_elevated))
            .overflow_y_scroll()
            .children(items)
            .into_any_element(),
    )
}

fn approve(ui: &Entity<CardUi>, project_id: &str, view_id: &str, name: &str, allow: bool, cx: &mut App) {
    if project_id.is_empty() {
        return;
    }
    let nodus = nodus(cx);
    let (p, v, n) = (project_id.to_owned(), view_id.to_owned(), name.to_owned());
    let ui = ui.clone();
    on_core(
        cx,
        async move {
            nodus
                .views()
                .approve(&p, &v, &n, allow)
                .await
                .map_err(|e| super::reason(&e.ipc_message()))
        },
        move |result, cx| {
            if let Err(reason) = result {
                CardUi::set_error(&ui, reason, cx);
            }
        },
    );
}

/// Before there's a page (VIEW-137): building, waiting on the user, or
/// nothing yet; then the description.
#[allow(clippy::too_many_arguments)]
pub fn empty_state(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    working: bool,
    waiting: bool,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let id = view.id.clone();
    let title = |text: &'static str| {
        div()
            .text_size(fu(13.))
            .line_height(fu(18.2))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.text_primary)
            .child(text)
    };
    let open = {
        let (actions, id) = (board.actions.clone(), id.clone());
        move |_: &mut Window, cx: &mut App| actions.open_view_session(&id, cx)
    };
    let mut children: Vec<AnyElement> = Vec::new();
    if working {
        children.push(spinning(
            icon(Icon::Loader2, 18.0 * z, 2.0, t.text_secondary),
            ElementId::Name(format!("view-empty-spin-{id}").into()),
            cx,
        ));
        children.push(title("Claude is building this view…").into_any_element());
        children.push(card_btn(
            Some((board, &id)),
            format!("view-watch-{id}"),
            Btn::Ghost,
            Some("Watch it work"),
            None,
            false,
            z,
            open,
            window,
            cx,
        ));
    } else if waiting {
        children.push(title("Claude needs you to carry on").into_any_element());
        children.push(card_btn(
            Some((board, &id)),
            format!("view-open-{id}"),
            Btn::Default,
            Some("Open its session"),
            None,
            false,
            z,
            open,
            window,
            cx,
        ));
    } else {
        children.push(title("No page yet").into_any_element());
        let (ui, project, view_id) = (ui.clone(), board.project_id.to_string(), id.clone());
        children.push(card_btn(
            Some((board, &id)),
            format!("view-build-{id}"),
            Btn::Default,
            Some("Build it"),
            Some(view_icon(ViewIcon::WandSparkles, 14.0 * z, 2.0, t.text_primary).into_any_element()),
            false,
            z,
            move |_, cx| build(&ui, &project, &view_id, cx),
            window,
            cx,
        ));
    }
    children.push(
        div()
            .max_w(fu(44.0 * 7.2))
            .text_size(fu(12.))
            .line_height(fu(17.4))
            .text_color(t.text_tertiary)
            .line_clamp(4)
            .child(view.description.clone())
            .into_any_element(),
    );
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(fu(10.))
        .px(fu(28.))
        .py(fu(20.))
        .text_center()
        .text_color(t.text_secondary)
        .children(children)
        .into_any_element()
}

/// VIEW-138: `Build it` asks the brain to build the view.
fn build(ui: &Entity<CardUi>, project_id: &str, view_id: &str, cx: &mut App) {
    if project_id.is_empty() {
        return;
    }
    let nodus = nodus(cx);
    let (p, v) = (project_id.to_owned(), view_id.to_owned());
    let ui = ui.clone();
    on_core(
        cx,
        async move {
            nodus
                .views()
                .ask(&p, &v, "Build this view.")
                .await
                .map_err(|e| super::reason(&e.ipc_message()))
        },
        move |result, cx| {
            if let Err(reason) = result {
                CardUi::set_error(&ui, reason, cx);
            }
        },
    );
}
