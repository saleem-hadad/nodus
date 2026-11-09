//! The view card (`ViewNode.tsx`, `views.css`; VIEW-122…150): a title bar
//! (its drag handle) with the brain's status, the title and the tools, the
//! approvals strip, the body with the page (or its empty state), the error
//! line and the ask bar. Lengths are flow units ([`fu`]), so the card scales
//! with the board.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::StyledImage as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Bounds, BoxShadow, ElementId, Entity, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Task, Transformation, Window, canvas, div, img, percentage, px, relative,
};
use nodus_domain::{BoardView, SessionStatus};

use super::frames::{self, Geometry, Slot};
use super::icons::{ViewIcon, view_icon};
use super::parts;
use crate::board::nodes::{BoardCx, ViewNode};
use crate::board::units::fu;
use crate::components::icon::{Icon, icon};
use crate::components::spaced_text;
use crate::components::tooltip::{TooltipPress, pressable_tooltip};
use crate::theme::{self, Tokens, fonts, tokens::Layer};

/// A view's own UI state (the card's React state).
#[derive(Default)]
pub struct CardUi {
    pub asking: bool,
    pub refreshing: bool,
    /// The card's error line, and its serial for the 6s timer (VIEW-139).
    pub error: Option<String>,
    pub error_serial: u64,
    pub error_timer: Option<Task<()>>,
    pub hovered: bool,
    pub resizing: bool,
    pub ask: Option<parts::AskBar>,
    pub title: Option<parts::TitleEditor>,
}

impl CardUi {
    /// Shows `message` on the error line; it clears itself 6s after it was
    /// last set (VIEW-139).
    pub fn set_error(this: &Entity<Self>, message: String, cx: &mut App) {
        this.update(cx, |ui, cx| {
            ui.error_serial += 1;
            let serial = ui.error_serial;
            ui.error = Some(message);
            ui.error_timer = Some(cx.spawn(async move |ui, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                let _ = ui.update(cx, |ui, cx| {
                    if ui.error_serial == serial {
                        ui.error = None;
                        cx.notify();
                    }
                });
            }));
            cx.notify();
        });
    }
}

/// The view's UI state.
pub fn ui(id: &str, window: &mut Window, cx: &mut App) -> Entity<CardUi> {
    window.use_keyed_state(ElementId::Name(format!("view-ui-{id}").into()), cx, |_, _| {
        CardUi::default()
    })
}

/// The brain's status as the card shows it (VIEW-124): the brain card's
/// while its process is live, else done.
pub fn brain_status(view: &BoardView, cx: &App) -> (SessionStatus, bool) {
    let Some(brain) = view.session_id.as_deref() else {
        return (SessionStatus::Done, false);
    };
    let card = crate::shell::window::main_shell(cx).and_then(|shell| {
        let state = shell.read(cx).state.clone();
        let s = state.read(cx);
        s.board
            .as_ref()?
            .cards
            .iter()
            .find(|c| c.node.session_id == brain)
            .map(|c| (c.status, c.live))
    });
    match card {
        Some((status, true)) => (status, true),
        _ => (SessionStatus::Done, false),
    }
}

/// A token's shadow layers at the board's zoom, in GPUI's order.
pub fn shadows(layers: &[Layer], z: f32) -> Vec<BoxShadow> {
    layers
        .iter()
        .rev()
        .map(|&(x, y, b, s, c)| theme::tokens::shadow(x * z, y * z, b * z, s * z, c))
        .collect()
}

/// `0 0 0 w c`, at the zoom.
fn ring(w: f32, c: Rgba, z: f32) -> BoxShadow {
    theme::tokens::shadow(0.0, 0.0, 0.0, w * z, c)
}

/// The card's shadow by state, later rules winning (views.css).
fn card_shadow(t: &Tokens, hovered: bool, waiting: bool, selected: bool, dragging: bool, z: f32) -> Vec<BoxShadow> {
    if dragging {
        let mut s = shadows(&t.shadow_lg, z);
        s.push(ring(2.0, t.accent, z));
        return s;
    }
    if selected {
        let mut s = shadows(&t.shadow_md, z);
        s.push(ring(2.0, t.accent, z));
        return s;
    }
    if waiting {
        let mut s = shadows(&t.shadow_md, z);
        s.push(ring(
            1.0,
            Rgba {
                a: t.status_waiting.a * 0.45,
                ..t.status_waiting
            },
            z,
        ));
        return s;
    }
    if hovered {
        shadows(&t.shadow_md, z)
    } else {
        shadows(&t.shadow_sm, z)
    }
}

/// `ViewNode`: the card in its node's box. (By value: the canvas's renderer type.)
#[allow(clippy::needless_pass_by_value)]
pub fn render(node: ViewNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let view = node.view.clone();
    let state = node.node.clone();
    let z = state.zoom;
    let id = view.id.clone();
    let ui = ui(&id, window, cx);
    let renaming = board.renaming.as_deref() == Some(id.as_str());
    if !renaming && ui.read(cx).title.is_some() {
        ui.update(cx, |u, _| u.title = None);
    }
    let (status, live) = brain_status(&view, cx);
    let working = status == SessionStatus::Working;
    let waiting = live && status == SessionStatus::Waiting;
    let shown = frames::shown(&id, Slot::Card, cx);
    let (hovered, resizing, asking, error) = {
        let u = ui.read(cx);
        (u.hovered || shown.hovered, u.resizing, u.asking, u.error.clone())
    };
    let framed = view.version > 0 && !board.project_id.is_empty();

    let header = header(
        &view,
        board,
        &ui,
        working,
        waiting,
        renaming,
        hovered || state.selected,
        z,
        window,
        cx,
    );
    let approvals = parts::approvals(&view, board, &ui, z, window, cx);
    let bottom_round = error.is_none() && !asking;
    let body = body(
        &view,
        board,
        &ui,
        framed,
        &shown,
        working,
        waiting,
        (resizing, state.dragging),
        bottom_round,
        z,
        window,
        cx,
    );

    let card = div()
        .id(ElementId::Name(format!("view-{id}").into()))
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .rounded(fu(14.))
        .bg(t.bg_elevated)
        .shadow(card_shadow(&t, hovered, waiting, state.selected, state.dragging, z))
        .overflow_hidden()
        .on_hover({
            let ui = ui.clone();
            move |h: &bool, _, cx| {
                ui.update(cx, |u, cx| {
                    if u.hovered != *h {
                        u.hovered = *h;
                        cx.notify();
                    }
                });
            }
        })
        .child(header)
        .children(approvals)
        .child(body)
        .when_some(error, |d, e| d.child(parts::error_line(e, z, &t)))
        .when(asking && !board.project_id.is_empty(), |d| {
            d.child(parts::ask_bar(&view, board, &ui, z, window, cx))
        });

    let resize = (state.selected && !renaming).then(|| super::resize::zones(&view, &state, board, &ui, window, cx));
    div()
        .relative()
        .size_full()
        // Resize handles under the card's contents, so nothing of the card
        // draws over the page's probe (`swap.rs`).
        .children(resize.into_iter().flatten())
        .child(card)
        .into_any_element()
}

/// The title bar (`.view-head`, 38 tall): status or icon, title, tools. The
/// canvas drags the node by it (VIEW-122).
#[allow(clippy::too_many_arguments)]
fn header(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    working: bool,
    waiting: bool,
    renaming: bool,
    tools_on: bool,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let id = view.id.clone();
    let lead = if working || waiting {
        let tip = if waiting {
            "Claude needs you to carry on with this view"
        } else {
            "Claude is working on this view"
        };
        let status = if waiting {
            SessionStatus::Waiting
        } else {
            SessionStatus::Working
        };
        parts::status_dot(&id, status, z, tip, window, cx)
    } else {
        icon(Icon::LayoutDashboard, 14.0 * z, 2.0, t.text_tertiary).into_any_element()
    };
    let title: AnyElement = if renaming {
        parts::title_editor(view, board, ui, z, window, cx)
    } else {
        let font = gpui::Font {
            family: fonts::DISPLAY.into(),
            weight: FontWeight::SEMIBOLD,
            ..gpui::Font::default()
        };
        let actions = board.actions.clone();
        let rename_id = id.clone();
        let press = window.use_keyed_state(ElementId::Name(format!("view-title-tip-{id}").into()), cx, |_, _| {
            TooltipPress::default()
        });
        div()
            .id(ElementId::Name(format!("view-title-{id}").into()))
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(
                spaced_text(
                    view.title.clone(),
                    font,
                    px(13.0 * z),
                    px(-0.13 * z),
                    px(16.0 * z),
                    t.text_primary,
                )
                .truncate(),
            )
            // VIEW-126: double-clicking the title renames, and stops there.
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                if e.click_count == 2 {
                    cx.stop_propagation();
                    actions.set_renaming(Some(&rename_id), cx);
                }
            })
            .when(!view.description.is_empty(), |d| {
                let leave = id.clone();
                d.on_hover(move |h, _, cx| {
                    if !*h {
                        super::swap::set_tooltip(&leave, false, cx);
                    }
                })
                .tooltip(tip_over_page(&id, view.description.clone().into(), press))
            })
            .into_any_element()
    };
    let tools = parts::tools(view, board, ui, z, window, cx);
    div()
        .id(ElementId::Name(format!("view-head-{id}").into()))
        .flex()
        .items_center()
        .gap(fu(8.))
        .flex_none()
        .h(fu(38.))
        .pl(fu(13.))
        .pr(fu(6.))
        .border_b(fu(0.5))
        .border_color(t.separator)
        // Double-clicks on the title bar never reach the board (VIEW-126).
        .on_mouse_down(MouseButton::Left, |e, _, cx| {
            if e.click_count >= 2 {
                cx.stop_propagation();
            }
        })
        .child(lead)
        .child(title)
        .child(
            div()
                .flex()
                .items_center()
                .gap(fu(1.))
                .flex_none()
                .ml_auto()
                .opacity(if tools_on { 1.0 } else { 0.55 })
                .children(tools),
        )
        .into_any_element()
}

/// A native tooltip that also tells the view's frame it's up, so the page
/// shows its snapshot while it is (decision D3).
pub fn tip_over_page(
    view_id: &str,
    text: SharedString,
    press: Entity<TooltipPress>,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let inner = pressable_tooltip(text, press);
    let id = view_id.to_owned();
    move |window, cx| {
        super::swap::set_tooltip(&id, true, cx);
        inner(window, cx)
    }
}

/// The body (`.view-body`): the page, or the empty state until there's one
/// (VIEW-030, VIEW-137).
#[allow(clippy::too_many_arguments)]
fn body(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    framed: bool,
    shown: &frames::Shown,
    working: bool,
    waiting: bool,
    (resizing, dragging): (bool, bool),
    bottom_round: bool,
    z: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let base = div().relative().flex_1().min_h_0();
    if !framed {
        return base
            .child(parts::empty_state(view, board, ui, working, waiting, z, window, cx))
            .into_any_element();
    }
    let radius = if bottom_round { 14.0 * z } else { 0.0 };
    let id = view.id.clone();
    let pointer = board.pointer;
    // An ink tool is out: strokes draw over the page (INKUI-018, D3).
    let ink = board.ink_tool;
    let parity = crate::parity::frozen(cx);
    let geometry = {
        move |document: Bounds<Pixels>, clip: Bounds<Pixels>| {
            let marquee = pointer.marquee.is_some_and(|(x, y, w, h)| {
                let m = Bounds::new(
                    gpui::point(clip.origin.x + px(x as f32), clip.origin.y + px(y as f32)),
                    gpui::size(px(w as f32), px(h as f32)),
                );
                m.intersects(&document.intersect(&clip))
            });
            Geometry {
                document,
                clip,
                zoom: z,
                radius,
                forced: parity || ink || dragging || resizing || marquee,
                passthrough: pointer.dragging
                    || pointer.selection_rect
                    || pointer.marquee.is_some()
                    || pointer.moving
                    || resizing,
                edge: 5.0 * z,
            }
        }
    };
    base.bg(t.bg_elevated)
        .when_some(shown.snapshot.clone(), |d, image| {
            d.child(
                img(image)
                    .object_fit(gpui::ObjectFit::Fill)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .rounded_bl(px(radius))
                    .rounded_br(px(radius)),
            )
        })
        .child(frames::probe(id, Slot::Card, geometry))
        .into_any_element()
}

/// A spinning icon (`.spin`: a turn every 0.9s), still in parity shots.
pub fn spinning(el: gpui::Svg, key: impl Into<ElementId>, cx: &App) -> AnyElement {
    if crate::parity::frozen(cx) {
        return el.into_any_element();
    }
    el.with_animation(
        key,
        Animation::new(Duration::from_millis(900)).repeat(),
        |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
    )
    .into_any_element()
}

/// Ellipsis icon, for the ⋯ tool.
pub fn more_icon(z: f32, color: Rgba) -> gpui::Svg {
    view_icon(ViewIcon::Ellipsis, 14.0 * z, 2.0, color)
}

/// Keeps the bounds an element was last laid out at.
pub fn bounds_probe(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |b, _, _| cell.set(b), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// `relative(p)`, for max heights in percent.
pub fn percent(p: f32) -> gpui::DefiniteLength {
    relative(p)
}
