//! A view on the board (VIEW-122…150): the page itself, a rounded panel
//! straight on the board, with a caption above it on the board (its drag
//! handle: the brain's status and the title). The panel holds the approvals
//! strip, the page (or its empty state), the error line and the ask bar. Its
//! actions are in its menu, a right-click away anywhere on it, the page
//! included. Lengths are flow units ([`fu`]), so the view scales with the
//! board.

use std::collections::HashMap;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Bounds, BoxShadow, ElementId, Entity, FontWeight, Global,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Task, Transformation, WeakEntity, Window, div, percentage, px, relative,
};
use nodus_domain::{BoardView, SessionStatus};

use super::frames::{self, Geometry, Rounding, Slot};
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
    /// Its first page landing (MOTION-033).
    pub landing: super::reveal::Landing,
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

/// The caption's height above the page: the view's drag handle.
pub const CAPTION_H: f32 = 28.0;
/// The page panel's corner radius.
const RADIUS: f32 = 14.0;

/// Every drawn view's UI state, for the view's menu to reach.
#[derive(Default)]
struct CardUis(HashMap<String, WeakEntity<CardUi>>);

impl Global for CardUis {}

/// The view's UI state.
pub fn ui(id: &str, window: &mut Window, cx: &mut App) -> Entity<CardUi> {
    let ui = window.use_keyed_state(ElementId::Name(format!("view-ui-{id}").into()), cx, |_, _| {
        CardUi::default()
    });
    let uis = &mut cx.default_global::<CardUis>().0;
    if uis.get(id).and_then(WeakEntity::upgrade).as_ref() != Some(&ui) {
        uis.retain(|_, ui| ui.upgrade().is_some());
        uis.insert(id.to_owned(), ui.downgrade());
    }
    ui
}

/// A drawn view's UI state.
fn find(view_id: &str, cx: &App) -> Option<Entity<CardUi>> {
    cx.try_global::<CardUis>()?.0.get(view_id)?.upgrade()
}

/// The menu's `Ask Claude to change it…`: opens the ask bar (VIEW-130).
pub fn ask(view_id: &str, cx: &mut App) {
    if let Some(ui) = find(view_id, cx) {
        ui.update(cx, |u, cx| {
            u.asking = true;
            cx.notify();
        });
    }
}

/// The menu's `Refresh` (VIEW-129): runs the view's commands, with the
/// caption spinning meanwhile and the first failure on the view. False when
/// the view isn't drawn.
pub fn refresh(project_id: &str, view_id: &str, cx: &mut App) -> bool {
    let Some(ui) = find(view_id, cx) else {
        return false;
    };
    parts::refresh(&ui, project_id, view_id, cx);
    true
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

/// `ViewNode`: the view in its node's box. (By value: the canvas's renderer type.)
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
    let (hovered, resizing, asking, refreshing, error) = {
        let u = ui.read(cx);
        (
            u.hovered || shown.hovered,
            u.resizing,
            u.asking,
            u.refreshing,
            u.error.clone(),
        )
    };
    let framed = view.version > 0 && !board.project_id.is_empty();
    let revealing = super::reveal::step(&ui, view.version, framed, window, cx);

    let caption = caption(
        &view,
        board,
        &ui,
        (working, waiting, refreshing),
        renaming,
        hovered || state.selected,
        z,
        window,
        cx,
    );
    let approvals = parts::approvals(&view, board, &ui, z, window, cx);
    let rounding = Rounding {
        radius: RADIUS * z,
        top: approvals.is_none(),
        bottom: error.is_none() && !asking,
    };
    let body = body(
        &view,
        board,
        &ui,
        framed,
        revealing,
        working,
        waiting,
        (resizing, state.dragging),
        rounding,
        z,
        window,
        cx,
    );

    // The page's panel: nothing but the page, unless the view asks for
    // approval, shows an error or takes a request.
    let panel = div()
        .id(ElementId::Name(format!("view-{id}").into()))
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .rounded(fu(RADIUS))
        .bg(t.bg_elevated)
        .shadow(card_shadow(&t, hovered, waiting, state.selected, state.dragging, z))
        .overflow_hidden()
        .children(approvals)
        .child(body)
        .when_some(error, |d, e| d.child(parts::error_line(e, z, &t)))
        .when(asking && !board.project_id.is_empty(), |d| {
            d.child(parts::ask_bar(&view, board, &ui, z, window, cx))
        });
    let card = div()
        .id(ElementId::Name(format!("view-box-{id}").into()))
        .relative()
        .flex()
        .flex_col()
        .size_full()
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
        .child(caption)
        .child(panel);

    let resize = (state.selected && !renaming).then(|| super::resize::zones(&view, &state, board, &ui, window, cx));
    div()
        .relative()
        .size_full()
        // Resize handles under the view's contents, so nothing of it draws
        // over the page's probe (`swap.rs`).
        .children(resize.into_iter().flatten())
        .child(card)
        .into_any_element()
}

/// The caption (`.view-head`, [`CAPTION_H`] tall) on the board above the
/// page: the brain's status (or the view's icon) and the title, brighter
/// while the view is hovered or selected. The canvas drags the node by it
/// (VIEW-122).
#[allow(clippy::too_many_arguments)]
fn caption(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    (working, waiting, refreshing): (bool, bool, bool),
    renaming: bool,
    lit: bool,
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
    } else if refreshing {
        spinning(
            icon(Icon::RefreshCw, 12.0 * z, 2.0, t.text_tertiary),
            ElementId::Name(format!("view-refresh-spin-{id}").into()),
            cx,
        )
    } else {
        icon(Icon::LayoutDashboard, 12.0 * z, 2.0, t.text_tertiary).into_any_element()
    };
    let title: AnyElement = if renaming {
        parts::title_editor(view, board, ui, z, window, cx)
    } else {
        let font = gpui::Font {
            family: fonts::DISPLAY.into(),
            weight: FontWeight::MEDIUM,
            ..gpui::Font::default()
        };
        let actions = board.actions.clone();
        let rename_id = id.clone();
        let press = window.use_keyed_state(ElementId::Name(format!("view-title-tip-{id}").into()), cx, |_, _| {
            TooltipPress::default()
        });
        div()
            .id(ElementId::Name(format!("view-title-{id}").into()))
            .min_w_0()
            .overflow_hidden()
            .child(
                spaced_text(
                    view.title.clone(),
                    font,
                    px(12.0 * z),
                    px(-0.06 * z),
                    px(16.0 * z),
                    if lit { t.text_primary } else { t.text_secondary },
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
    div()
        .id(ElementId::Name(format!("view-head-{id}").into()))
        .flex()
        .items_center()
        .gap(fu(6.))
        .flex_none()
        .h(fu(CAPTION_H))
        .pb(fu(4.))
        .px(fu(4.))
        // Double-clicks on the caption never reach the board (VIEW-126).
        .on_mouse_down(MouseButton::Left, |e, _, cx| {
            if e.click_count >= 2 {
                cx.stop_propagation();
            }
        })
        .child(lead)
        .child(title)
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
    revealing: Option<super::reveal::Revealing>,
    working: bool,
    waiting: bool,
    (resizing, dragging): (bool, bool),
    rounding: Rounding,
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
    let id = view.id.clone();
    let pointer = board.pointer;
    // An ink tool is out: strokes draw over the page (INKUI-018, D3).
    let ink = board.ink_tool;
    let parity = crate::parity::frozen(cx);
    let reveal = revealing.is_some();
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
                rounding,
                forced: parity || ink || dragging || resizing || marquee,
                passthrough: pointer.dragging
                    || pointer.selection_rect
                    || pointer.marquee.is_some()
                    || pointer.moving
                    || resizing,
                edge: 5.0 * z,
                reveal,
            }
        }
    };
    // The probe paints the page's snapshot while it's behind one. The fill
    // takes the panel's corners: GPUI's overflow clip is square.
    let base = base
        .bg(t.bg_elevated)
        .when(rounding.top, |d| d.rounded_t(fu(RADIUS)))
        .when(rounding.bottom, |d| d.rounded_b(fu(RADIUS)));
    let Some(revealing) = revealing else {
        return base.child(frames::probe(id, Slot::Card, geometry)).into_any_element();
    };
    // MOTION-033: the empty state stays until the page's picture paints
    // over it, then the picture sharpens out of a blur.
    base.child(parts::empty_state(view, board, ui, working, waiting, z, window, cx))
        .child(frames::probe_into(
            id,
            Slot::Card,
            geometry,
            Some(revealing.painted.clone()),
        ))
        .child(super::reveal::overlay(&revealing, rounding, z))
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

/// `relative(p)`, for max heights in percent.
pub fn percent(p: f32) -> gpui::DefiniteLength {
    relative(p)
}
