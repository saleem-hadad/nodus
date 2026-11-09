//! A session's current run of agents as a deck of cards behind its card,
//! each peeking out in its status colour, and the fan that deals them out
//! beside the card (`AgentDeck.tsx`; DECK-001…DECK-029).

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled, Window, div, px, radians,
};
use nodus_domain::{AgentInfo, SessionStatus};

use super::icons::{CardIcon, card_icon};
use super::para::{font, para};
use super::present::{
    DeckState, FanSide, agent_meta, deck_layers, fan_all_text, fan_count, fan_pick, fan_side, running,
};
use super::status_dot::status_dot;
use super::{BoardEnv, Shade, Shadow, Z, anim, baseline, brightness, text, text_tab, ui_font};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::format::agent_step;
use crate::theme::tokens::Tokens;
use crate::theme::{self, fonts, motion};

/// Which parts of a card's node (card, deck, fan, stickers) the pointer is
/// over: the node's `pointerenter`/`pointerleave` (DECK-010).
#[derive(Debug, Default)]
pub struct NodeParts(pub HashSet<SharedString>);

/// Told when a card's fan opens (true) or folds.
type OnOpen = Rc<dyn Fn(bool, &mut App)>;

/// The node's deck state and hover parts.
#[derive(Clone)]
pub struct DeckHandles {
    pub deck: Entity<DeckState>,
    pub parts: Entity<NodeParts>,
    /// Where the node was on screen last frame (DECK-015).
    pub bounds: Entity<Option<Bounds<Pixels>>>,
    /// Told when the fan opens or folds, after the event (DECK-014).
    on_open: OnOpen,
}

impl DeckHandles {
    pub fn get(node: &str, env: &Rc<BoardEnv>, window: &mut Window, cx: &mut App) -> Self {
        let actions = env.actions.clone();
        let id = node.to_owned();
        Self {
            deck: window.use_keyed_state(anim::key(node, "deck"), cx, |_, _| DeckState::default()),
            parts: window.use_keyed_state(anim::key(node, "parts"), cx, |_, _| NodeParts::default()),
            bounds: window.use_keyed_state(anim::key(node, "bounds"), cx, |_, _| None),
            on_open: Rc::new(move |open, cx| actions.raise(&id, open, cx)),
        }
    }

    /// Runs `f` on the deck, and says so if the fan opened or folded.
    fn change(&self, cx: &mut App, f: impl FnOnce(&mut DeckState)) {
        let was = self.deck.read(cx).open();
        self.deck.update(cx, |d, _| f(d));
        let now = self.deck.read(cx).open();
        if was != now {
            let on_open = self.on_open.clone();
            cx.defer(move |cx| on_open(now, cx));
        }
    }

    pub fn open(&self, cx: &App) -> bool {
        self.deck.read(cx).open()
    }

    /// DECK-008: pointing at a trigger opens the fan.
    pub fn point(&self, window: &mut Window, cx: &mut App) {
        self.change(cx, DeckState::point);
        window.refresh();
    }

    /// DECK-009, DECK-012
    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        self.change(cx, DeckState::toggle);
        window.refresh();
    }

    /// DECK-011, DECK-013, DECK-027
    pub fn close(&self, window: &mut Window, cx: &mut App) {
        self.fold(cx);
        window.refresh();
    }

    /// Folds the fan without a window (DECK-013 while rendering).
    pub fn fold(&self, cx: &mut App) {
        self.change(cx, |d| {
            d.close();
            d.side = None;
        });
    }

    /// A part of the node gained or lost the pointer (DECK-010).
    pub fn part_hover(&self, part: &'static str, hovered: bool, window: &mut Window, cx: &mut App) {
        self.parts.update(cx, |p, _| {
            if hovered {
                p.0.insert(part.into());
            } else {
                p.0.remove(part);
            }
        });
        if !self.deck.read(cx).open() {
            return;
        }
        if self.parts.read(cx).0.is_empty() {
            self.deck.update(cx, |d, _| d.leave(Instant::now()));
            let handles = self.clone();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor()
                        .timer(super::present::DECK_CLOSE_DELAY + Duration::from_millis(5))
                        .await;
                    let _ = cx.update(|window, cx| {
                        let mut closed = false;
                        handles.change(cx, |d| {
                            closed = d.tick(Instant::now());
                            if !d.open() {
                                d.side = None;
                            }
                        });
                        if closed {
                            window.refresh();
                        }
                    });
                })
                .detach();
        } else {
            self.deck.update(cx, |d, _| d.stay());
        }
    }

    /// Whether the pointer is anywhere on the node.
    pub fn over_node(&self, cx: &App) -> bool {
        !self.parts.read(cx).0.is_empty()
    }
}

/// The layers' transform at each depth, at rest and lifted (DECK-004/005).
fn layer_offset(depth: usize, lifted: bool) -> (f32, f32, f32) {
    match (depth, lifted) {
        (1, false) => (7.0, 0.955, 0.975),
        (1, true) => (10.0, 0.955, 0.975),
        (2, false) => (13.0, 0.91, 0.95),
        (2, true) => (19.0, 0.91, 0.95),
        (3, false) => (18.0, 0.865, 0.925),
        _ => (27.0, 0.865, 0.925),
    }
}

/// A status's edge colour (`--edge`).
pub fn edge(t: &Tokens, status: SessionStatus) -> gpui::Rgba {
    t.status(status)
}

/// The deck behind a card `size` big: its layers (painted before the card)
/// and the zones under the card where they can be pointed at.
pub fn deck(
    env: &Rc<BoardEnv>,
    node: &str,
    agents: &[AgentInfo],
    handles: &DeckHandles,
    size: (f32, f32),
    window: &mut Window,
    cx: &mut App,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let z = env.z();
    let t = theme::tokens(cx);
    let (w, h) = size;
    let zone_hover = window.use_keyed_state(anim::key(node, "deck-zone"), cx, |_, _| false);
    let lifted = handles.open(cx) || *zone_hover.read(cx);
    let mut layers = Vec::new();
    let mut zones = Vec::new();
    for (depth, agent) in deck_layers(agents) {
        let (rest_ty, scale, bright) = layer_offset(depth, lifted);
        // DECK-006: 420ms on the spring.
        let ty = transition(
            window,
            cx,
            anim::key_n(node, "deck-ty", depth),
            rest_ty,
            Duration::from_millis(420),
            motion::ease_stack,
        );
        let e = edge(&t, agent.status);
        let rest = Shadow::with(&[Shade::inset(0.0, -3.0, e)], &t.shadow_sm);
        // DECK-007: a waiting agent's layer glows, 1.1s ease-out.
        let shadow = if agent.status == SessionStatus::Waiting {
            let phase = anim::cycle(
                window,
                cx,
                anim::key_n(node, "deck-glow", depth),
                Duration::from_millis(1100),
            );
            let peak = Shadow::of(&[
                Shade::inset(0.0, -3.0, e),
                Shade {
                    y: 4.0,
                    blur: 12.0,
                    color: super::fade(t.status_waiting, 0.35),
                    ..Shade::NONE
                },
            ]);
            use crate::components::tween::Mix;
            rest.mix(peak, anim::half_way(phase, motion::ease_out_keyword))
        } else {
            rest
        };
        layers.push(
            div()
                .absolute()
                .inset_0()
                .rounded(z.px(14.))
                .bg(brightness(t.bg_elevated, bright))
                .shadow(shadow.brightness(bright).gpui(z))
                .transform_origin(0.5, 1.0)
                .translate(px(0.), z.px(ty))
                .scale(scale)
                .into_any_element(),
        );
        // Only the strip that peeks out under the card takes the pointer.
        let zone_w = w * scale;
        let deck_handles = handles.clone();
        let toggler = handles.clone();
        let zh = zone_hover.clone();
        zones.push(
            div()
                .id(anim::key_n(node, "deck-zone", depth))
                .absolute()
                .left(z.px((w - zone_w) / 2.0))
                .top(z.px(h))
                .w(z.px(zone_w))
                .h(z.px(ty.max(0.0)))
                .cursor(CursorStyle::PointingHand)
                .on_hover(move |hovered, window, cx| {
                    zh.update(cx, |s, _| *s = *hovered);
                    if *hovered {
                        deck_handles.point(window, cx);
                    }
                    deck_handles.part_hover("deck", *hovered, window, cx);
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    // Every click toggles, both of a double-click too, as in 0.9.4.
                    toggler.toggle(window, cx);
                })
                .into_any_element(),
        );
    }
    (layers, zones)
}

/// A trigger that fans the deck: the footer's agent count, the title bar's
/// Agents tool (CARD-037, XCARD-009).
pub fn trigger<E: InteractiveElement + StatefulInteractiveElement + Styled + 'static>(
    el: E,
    handles: &DeckHandles,
) -> E {
    let toggling = handles.clone();
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            // Every click toggles, both of a double-click too, as in 0.9.4.
            toggling.toggle(window, cx);
        })
}

/// What pointing at a trigger does (DECK-008), for the trigger's own hover
/// listener to call (an element takes one).
pub fn trigger_hover(handles: &DeckHandles, part: &'static str) -> HoverHook {
    let pointing = handles.clone();
    Rc::new(move |hovered, window, cx| {
        if hovered {
            pointing.point(window, cx);
        }
        pointing.part_hover(part, hovered, window, cx);
    })
}

/// Something told when an element gains or loses the pointer.
pub type HoverHook = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// DECK-015, DECK-016: where the fan goes, decided once per opening.
fn side(env: &BoardEnv, handles: &DeckHandles, cx: &mut App) -> FanSide {
    if let Some(side) = handles.deck.read(cx).side {
        return side;
    }
    let side = match *handles.bounds.read(cx) {
        Some(card) => {
            let board = env.board_bounds;
            fan_side(
                (f32::from(card.left()), f32::from(card.right())),
                (f32::from(board.left()), f32::from(board.right())),
                env.zoom,
            )
        }
        None => FanSide::Right,
    };
    handles.deck.update(cx, |d, _| d.side = Some(side));
    side
}

/// The fan's deal-in (DECK-026): from `translate(dx, 10px) scale(0.92)
/// rotate(-2.5deg)` and transparent to rest, `delay` in, on the spring.
fn dealt<E: Styled + IntoElement + ParentElement>(el: E, z: Z, dx: f32, p: Option<f32>) -> E {
    let Some(p) = p else {
        return el.group_opacity(0.0);
    };
    // Done is exactly 1; on the way the spring overshoots past 1 and back.
    #[allow(clippy::float_cmp, reason = "anim::enter gives exactly 1 once done")]
    let done = p == 1.0;
    if done {
        return el;
    }
    let k = 1.0 - p;
    el.translate(z.px(dx * k), z.px(10.0 * k))
        .scale(0.92 + 0.08 * p)
        .rotate(radians((-2.5 * k).to_radians()))
        .group_opacity(p.clamp(0.0, 1.0))
}

/// The fan beside a card `size` big (DECK-015…DECK-029).
#[allow(clippy::too_many_arguments)]
pub fn fan(
    env: &Rc<BoardEnv>,
    node: &str,
    agents: &[AgentInfo],
    total: u32,
    handles: &DeckHandles,
    size: (f32, f32),
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let side = side(env, handles, cx);
    let (w, _) = size;
    let (x, y) = match side {
        FanSide::Right => (w + 18.0, -4.0),
        FanSide::Left => (-18.0 - 260.0, -4.0),
        FanSide::Inside => (w - 14.0 - 260.0, 52.0),
    };
    let dx = side.deal_x();
    let run = running(agents);
    if run > 0 {
        // DECK-024: elapsed times count on.
        anim::tick(window, cx, anim::key(node, "fan-tick"), Duration::from_secs(1));
    }
    let now = env.now_ms;
    let shown = fan_pick(agents);

    // The header: `Agents`, then the count, on one baseline.
    let title_font = ui_font(FontWeight::SEMIBOLD);
    let count_font = ui_font(FontWeight::NORMAL);
    let b_title = baseline(window, &title_font, z.px(12.), z.px(16.8));
    let b_count = baseline(window, &count_font, z.px(11.), z.px(15.4));
    let head_p = anim::enter(
        window,
        cx,
        anim::key(node, "fan-head"),
        Duration::ZERO,
        Duration::from_millis(380),
        motion::ease_stack,
    );
    let inside = side == FanSide::Inside;
    let head = div()
        .flex()
        .items_start()
        .gap(z.px(8.))
        .px(z.px(4.))
        .when(inside, |d| {
            d.self_start()
                .px(z.px(10.))
                .py(z.px(4.))
                .rounded(z.px(12.))
                .bg(t.bg_elevated)
                .shadow(Shadow::with(&[], &t.shadow_sm).gpui(z))
        })
        .child(
            div()
                .flex_none()
                .mt(px(f32::from(b_count - b_title).max(0.)))
                .child(text(z, "Agents", 12.0, FontWeight::SEMIBOLD, t.text_primary)),
        )
        .child(
            div()
                .flex_none()
                .mt(px(f32::from(b_title - b_count).max(0.)))
                .child(text_tab(
                    z,
                    fan_count(agents),
                    11.0,
                    FontWeight::NORMAL,
                    t.text_tertiary,
                )),
        );
    let mut column = div()
        .id(anim::key(node, "fan"))
        .absolute()
        .left(z.px(x))
        .top(z.px(y))
        .w(z.px(260.))
        .flex()
        .flex_col()
        .gap(z.px(8.))
        .cursor(CursorStyle::Arrow)
        .font_family(fonts::UI)
        .child(dealt(head, z, dx, head_p));

    for (i, agent) in shown.iter().enumerate() {
        let p = anim::enter(
            window,
            cx,
            anim::key_n(node, "fan-row", i),
            Duration::from_millis(i as u64 * 45 + 30),
            Duration::from_millis(520),
            motion::ease_stack,
        );
        let row = fan_row(env, node, i, agent, now, handles, window, cx);
        column = column.child(dealt(row, z, dx, p));
    }

    // DECK-025: the "all agents" pill, dealt last.
    let i = shown.len();
    let p = anim::enter(
        window,
        cx,
        anim::key_n(node, "fan-row", i),
        Duration::from_millis(i as u64 * 45 + 30),
        Duration::from_millis(520),
        motion::ease_stack,
    );
    let pill_hover = window.use_keyed_state(anim::key(node, "fan-all-hover"), cx, |_, _| false);
    let ph = pill_hover.clone();
    let opening = handles.clone();
    let env_all = env.clone();
    let node_all = node.to_owned();
    let pill = div()
        .id(anim::key(node, "fan-all"))
        .flex()
        .items_center()
        .gap(z.px(5.))
        .self_start()
        .h(z.px(26.))
        .px(z.px(10.))
        .rounded(z.px(13.))
        .bg(t.bg_elevated)
        .shadow(
            Shadow::with(
                &[],
                if *pill_hover.read(cx) {
                    &t.shadow_md
                } else {
                    &t.shadow_sm
                },
            )
            .gpui(z),
        )
        .text_color(t.accent)
        .text_size(z.px(11.))
        .line_height(z.px(15.4))
        .font_weight(FontWeight::MEDIUM)
        .on_hover(move |h, window, cx| {
            ph.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            opening.close(window, cx);
            env_all.actions.open_agents(&node_all, None, cx);
        })
        .child(text(
            z,
            fan_all_text(shown.len(), agents.len(), total),
            11.0,
            FontWeight::MEDIUM,
            t.accent,
        ))
        .child(card_icon(CardIcon::ArrowRight, z.f(12.), 2.2, t.accent));
    column = column.child(dealt(pill, z, dx, p));

    // DECK-028: the fan keeps its clicks, presses and wheel from the board.
    let staying = handles.clone();
    column
        .on_hover(move |hovered, window, cx| staying.part_hover("fan", *hovered, window, cx))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .into_any_element()
}

/// One agent in the fan (DECK-019…DECK-023).
#[allow(clippy::too_many_arguments)]
fn fan_row(
    env: &Rc<BoardEnv>,
    node: &str,
    i: usize,
    agent: &AgentInfo,
    now: f64,
    handles: &DeckHandles,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let z = env.z();
    let t = theme::tokens(cx);
    let e = edge(&t, agent.status);
    let hover = window.use_keyed_state(anim::key_n(node, "fan-row-hover", i), cx, |_, _| false);
    let target = if *hover.read(cx) {
        Shadow::with(
            &[Shade::inset(3.0, 0.0, e), Shade::ring(1.0, t.border_strong)],
            &t.shadow_lg,
        )
    } else {
        Shadow::with(&[Shade::inset(3.0, 0.0, e)], &t.shadow_md)
    };
    // DECK-020: 120ms ease.
    let shadow = transition(
        window,
        cx,
        anim::key_n(node, "fan-row-shadow", i),
        target,
        Duration::from_millis(120),
        motion::ease,
    );
    let step = agent_step(agent);
    let dot = status_dot(anim::key_n(node, "fan-dot", i), agent.status, true, z, window, cx);
    let mono = font(fonts::MONO, FontWeight::NORMAL, false);
    let step_line = (step.tool.is_some() || step.text.is_some()).then(|| {
        let mut line = div().flex().items_start().gap(z.px(6.)).pl(z.px(14.)).min_w_0();
        if let Some(tool) = step.tool.clone() {
            line = line.child(div().flex_none().child(super::para::label(
                tool,
                mono.clone(),
                11.0,
                15.4,
                t.text_primary,
                z.0,
            )));
        }
        if let Some(text) = step.text.clone() {
            line = line.child(if step.tool.is_some() {
                para(text, mono.clone(), z.px(11.), z.px(15.4), t.text_tertiary)
                    .zoom(z.0)
                    .ellipsis()
                    .grow()
                    .into_any_element()
            } else {
                para(
                    text,
                    ui_font(FontWeight::NORMAL),
                    z.px(11.),
                    z.px(15.4),
                    t.text_secondary,
                )
                .zoom(z.0)
                .clamp(2)
                .grow()
                .into_any_element()
            });
        }
        line
    });
    let meta = agent_meta(agent, now);
    let status_color = match agent.status {
        SessionStatus::Working => t.status_working,
        SessionStatus::Waiting => t.status_waiting,
        SessionStatus::Done => t.text_tertiary,
    };
    let opening = handles.clone();
    let env_row = env.clone();
    let node_row = node.to_owned();
    let agent_id = agent.agent_id.clone();
    let h = hover.clone();
    div()
        .id(anim::key_n(node, "fan-row", i))
        .flex()
        .flex_col()
        .gap(z.px(4.))
        .w_full()
        .pt(z.px(10.))
        .px(z.px(12.))
        .pb(z.px(11.))
        .rounded(z.px(10.))
        .bg(t.bg_elevated)
        .shadow(shadow.gpui(z))
        .text_color(t.text_secondary)
        .text_size(z.px(13.))
        .line_height(z.px(18.2))
        .tooltip(native_tooltip(agent.description.clone().into()))
        .on_hover(move |hovered, window, cx| {
            h.update(cx, |s, _| *s = *hovered);
            window.refresh();
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            // DECK-027
            opening.close(window, cx);
            env_row.actions.open_agents(&node_row, Some(&agent_id), cx);
        })
        .child(
            div().flex().items_center().gap(z.px(7.)).min_w_0().child(dot).child(
                para(
                    agent.description.clone(),
                    ui_font(FontWeight::SEMIBOLD),
                    z.px(13.),
                    z.px(18.2),
                    t.text_primary,
                )
                .zoom(z.0)
                .ellipsis()
                .grow(),
            ),
        )
        .when_some(step_line, ParentElement::child)
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_x(z.px(8.))
                .pl(z.px(14.))
                .text_size(z.px(11.))
                .line_height(z.px(15.4))
                .text_color(t.text_tertiary)
                .font_features(fonts::tabular())
                .children(meta.into_iter().enumerate().map(|(k, s)| {
                    text_tab(
                        z,
                        s,
                        11.0,
                        FontWeight::NORMAL,
                        if k == 0 { status_color } else { t.text_tertiary },
                    )
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifts_each_depth_further() {
        // DECK-004, DECK-005
        assert_eq!(layer_offset(1, false).0, 7.0);
        assert_eq!(layer_offset(2, true).0, 19.0);
        assert_eq!(layer_offset(3, true), (27.0, 0.865, 0.925));
    }
}
