//! A card expanded in place (`ExpandedCard.tsx`; XCARD-001…XCARD-032): a
//! 40px title bar with Claude's usage and the card's tools over the live
//! terminal, the overlays while it isn't running, the grip and the resize
//! zones. The terminal itself is the terminal track's ([`TerminalHost`]).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, CursorStyle, DispatchPhase, FontWeight, Global, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Rgba, ScrollEvent, StatefulInteractiveElement, Styled, Window, canvas, div, radians, svg,
};
use nodus_domain::{AgentInfo, SessionUsage};

use super::agent_deck::{self, DeckHandles};
use super::icons::{CardIcon, GRIP_PATH, card_icon};
use super::present::{
    self, EXPANDED_MIN_H, EXPANDED_MIN_W, ExpandedStart, Phase, WakeEffect, WheelArbiter, is_terminal, running,
};
use super::resize::resize_zones;
use super::session_card::{SessionNode, apply, heading, surface};
use super::{BoardEnv, anim, fade};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::format::{format_cost, format_duration, format_tokens};
use crate::theme::tokens::rgba;
use crate::theme::{self, fonts, motion};

/// Wakes asked for, by session (the module globals `pendingWake` and
/// `wakers` of ExpandedCard.tsx): a card applies each new one when it
/// renders, or on its first render if it wasn't expanded yet.
#[derive(Default)]
pub struct Wakes {
    requested: HashMap<String, u64>,
    pending: HashSet<String>,
}

impl Global for Wakes {}

/// XCARD-025: `wakeCardTerminal`. Shows the session in its expanded card,
/// starting or resuming it, and focuses its terminal; works before the card
/// has expanded (Expand, opening an expanded card, Resume, the mic).
pub fn wake_card_terminal(session_id: &str, cx: &mut App) {
    let wakes = cx.default_global::<Wakes>();
    *wakes.requested.entry(session_id.to_owned()).or_default() += 1;
    wakes.pending.insert(session_id.to_owned());
    cx.refresh_windows();
}

/// The board's wheel gestures (XCARD-027): the one arbiter, which the card
/// terminals ask through their wheel gate (install.rs).
#[derive(Default)]
pub struct Wheel(pub WheelArbiter);

impl Global for Wheel {}

struct XState {
    start: ExpandedStart,
    seen: u64,
    focused_wake: u64,
}

/// The expanded card's body (XCARD-002…XCARD-028, XCARD-031).
pub fn body(
    env: &Rc<BoardEnv>,
    node: &SessionNode,
    agents: &[AgentInfo],
    handles: &DeckHandles,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let card = &node.card;
    let id = card.node.session_id.clone();
    let shell = is_terminal(card);
    let (w, h) = node.size;
    let now = Instant::now();
    let host = env.terminals.clone();
    let needs_open = host.as_ref().is_none_or(|h| h.needs_open(&id));

    // XCARD-019…XCARD-026
    let state = window.use_keyed_state(anim::key(&id, "xstate"), cx, {
        let id = id.clone();
        let live = card.live;
        move |_, cx| {
            let wakes = cx.default_global::<Wakes>();
            let pending = wakes.pending.remove(&id);
            let seen = wakes.requested.get(&id).copied().unwrap_or(0);
            XState {
                start: ExpandedStart::new(live, needs_open, pending, now),
                seen,
                focused_wake: 0,
            }
        }
    });
    let requested = cx.default_global::<Wakes>().requested.get(&id).copied().unwrap_or(0);
    let phase = host.as_ref().map_or(Phase::Idle, |h| h.phase(&id, cx));
    let effect = state.update(cx, |s, _| {
        s.start.saw_live(card.live);
        if requested > s.seen {
            s.seen = requested;
            Some(s.start.wake(card.live, needs_open, &phase, now))
        } else {
            None
        }
    });
    if effect.is_some() {
        cx.default_global::<Wakes>().pending.remove(&id);
    }
    let in_modal = env.terminal_modal.as_deref() == Some(id.as_str());
    let enabled = state.read(cx).start.enabled(in_modal, !env.project_id.is_empty());
    if let Some(host) = &host {
        host.set_enabled(&id, shell, enabled, cx);
        if effect == Some(WakeEffect::Reattach) {
            host.reattach(&id, cx);
        }
        let (should, wakes, focused) = {
            let s = state.read(cx);
            (s.start.should_focus(now), s.start.wakes, s.focused_wake)
        };
        if enabled && should && focused != wakes {
            state.update(cx, |s, _| s.focused_wake = wakes);
            host.focus(&id, window, cx);
        }
    }
    let (started, resumable, loading_label) = {
        let s = &state.read(cx).start;
        (s.started, s.resumable(card.live, &phase), s.loading_label(shell))
    };

    let hover = window.use_keyed_state(anim::key(&id, "hover"), cx, |_, _| false);
    let hovered = *hover.read(cx);
    let s = surface(&t, card, hovered, node.selected, node.dragging, None, true);
    let ms = Duration::from_millis;
    let shadow = transition(
        window,
        cx,
        anim::key(&id, "shadow"),
        s.shadow,
        ms(220),
        motion::ease_out,
    );
    let shadow = super::session_card::flash_shadow(&t, node.flash, window).unwrap_or(shadow);
    let grip = transition(
        window,
        cx,
        anim::key(&id, "grip"),
        if hovered || handles.over_node(cx) { 1.0 } else { 0.0 },
        ms(120),
        motion::ease,
    );

    // The title bar (XCARD-003…XCARD-016).
    let head_w = w - 20.0;
    let mut head = div()
        .id(anim::key(&id, "xhead"))
        .flex()
        .flex_none()
        .items_center()
        .gap(z.px(9.))
        .h(z.px(40.))
        .pl(z.px(14.))
        .pr(z.px(6.))
        .children(heading(env, node, true, window, cx));
    if !shell && head_w > 440.0 {
        // XCARD-005, XCARD-006: the container query on the bar's content box.
        head = head.child(usage(
            env,
            &id,
            card.usage.as_ref(),
            card.live,
            head_w > 560.0,
            window,
            cx,
        ));
    }
    head = head.child(tools(env, node, agents, handles, resumable, window, cx));

    // The body (XCARD-018…XCARD-024).
    let term = if enabled {
        host.as_ref().and_then(|h| h.element(&id, window, cx))
    } else {
        None
    };
    let overlay = if !started {
        Some(blocking(start_button(env, &id, shell, window, cx)))
    } else if enabled && phase == Phase::Loading {
        Some(loading_pill(env, &id, loading_label, window, cx))
    } else if enabled && let Phase::Error(err) = &phase {
        Some(blocking(error_column(env, &id, shell, err.clone(), window, cx)))
    } else {
        None
    };
    let focusing = host.clone();
    let focus_id = id.clone();
    let term_body = div()
        .id(anim::key(&id, "term"))
        .relative()
        .flex_1()
        .min_h_0()
        .pt(z.px(8.))
        .pr(z.px(4.))
        .pb(z.px(8.))
        .pl(z.px(12.))
        .bg(t.term_bg)
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            if enabled && let Some(h) = &focusing {
                h.focus(&focus_id, window, cx);
            }
        })
        .on_click(|_, _, cx| cx.stop_propagation())
        // XCARD-027, XCARD-028: a gesture that began over a terminal scrolls
        // it, not the board (the terminal asks the same in its own content
        // box, through its wheel gate); a disabled one never claims.
        .on_scroll(move |e: &ScrollEvent, _, cx| {
            if enabled
                && cx
                    .default_global::<Wheel>()
                    .0
                    .claims(Instant::now(), e.modifiers.control)
            {
                cx.stop_propagation();
            }
        })
        .when(enabled, |d| {
            // The window sees every wheel event before anything under the
            // pointer does, and marks where gestures begin.
            d.child(
                canvas(
                    |_, _, _| {},
                    |_, _, window, _| {
                        window.on_mouse_event(|_: &ScrollEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                cx.default_global::<Wheel>().0.saw(Instant::now());
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            )
        })
        .when_some(term, |d, el| d.child(div().size_full().child(el)))
        .when_some(overlay, ParentElement::child);

    let hs = hover.clone();
    let parts = handles.clone();
    let mut card_el = div()
        .id(anim::key(&id, "card"))
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(w))
        .h(z.px(h))
        .flex()
        .flex_col()
        .rounded(z.px(14.))
        .overflow_hidden()
        .clip_rounded()
        .bg(t.bg_elevated)
        .shadow(shadow.gpui(z))
        .text_size(z.px(13.))
        .line_height(z.px(18.2))
        .cursor(if node.dragging {
            CursorStyle::ClosedHand
        } else {
            CursorStyle::Arrow
        })
        .on_hover(move |h, window, cx| {
            hs.update(cx, |v, _| *v = *h);
            parts.part_hover("card", *h, window, cx);
            window.refresh();
        })
        .child(head)
        .child(term_body)
        // XCARD-031: the grip, while the pointer is on the node.
        .child(
            svg()
                .path(GRIP_PATH)
                .absolute()
                .right(z.px(4.))
                .bottom(z.px(4.))
                .size(z.px(11.))
                .text_color(rgba(255, 255, 255, 0.32 * grip)),
        );
    card_el = apply(card_el, &s, z, None);
    card_el.into_any_element()
}

/// Resize zones on every edge and corner (XCARD-029, XCARD-030).
pub fn chrome_extras(env: &Rc<BoardEnv>, node: &SessionNode, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let (w, h) = node.size;
    let x = node.card.node.x as f32;
    let y = node.card.node.y as f32;
    resize_zones(
        env,
        &node.card.node.session_id,
        super::Resized::Card(node.card.node.session_id.clone()),
        (x, y, w, h),
        (EXPANDED_MIN_W, EXPANDED_MIN_H),
        18.0,
        None,
        window,
        cx,
    )
}

/// A title-bar tool button (XCARD-008).
#[allow(clippy::too_many_arguments)]
fn tool(
    env: &Rc<BoardEnv>,
    node: &str,
    part: &'static str,
    labelled: bool,
    expanded_flag: bool,
    hook: Option<agent_deck::HoverHook>,
    window: &mut Window,
    cx: &mut App,
) -> (gpui::Stateful<gpui::Div>, Rgba) {
    let z = env.z();
    let t = theme::tokens(cx);
    let hover = window.use_keyed_state(anim::key(node, part), cx, |_, _| false);
    let on = *hover.read(cx);
    let pressed = window.use_keyed_state(anim::key_n(node, part, 1), cx, |_, _| false);
    let down = *pressed.read(cx);
    let target = if down || expanded_flag {
        t.bg_pressed
    } else if on {
        t.bg_hover
    } else {
        transparent()
    };
    let bg = transition(
        window,
        cx,
        anim::key_n(node, part, 2),
        target,
        Duration::from_millis(120),
        motion::ease,
    );
    let color = if on || expanded_flag || labelled {
        t.text_primary
    } else {
        t.text_secondary
    };
    let h2 = hover.clone();
    let p_down = pressed.clone();
    let p_up = pressed.clone();
    let el = div()
        .id(anim::key(node, part))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(z.px(5.))
        .min_w(z.px(28.))
        .h(z.px(28.))
        .when(labelled, |d| d.px(z.px(9.)))
        .rounded(z.px(6.))
        .bg(bg)
        .text_color(color)
        .text_size(z.px(12.))
        .line_height(z.px(16.8))
        .font_weight(FontWeight::MEDIUM)
        .on_hover(move |h, window, cx| {
            h2.update(cx, |s, _| *s = *h);
            if !*h {
                p_up.update(cx, |s, _| *s = false);
            }
            if let Some(hook) = &hook {
                hook(*h, window, cx);
            }
            window.refresh();
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            p_down.update(cx, |s, _| *s = true);
            window.refresh();
        })
        .on_mouse_up(MouseButton::Left, {
            let p = pressed.clone();
            move |_, window, cx| {
                p.update(cx, |s, _| *s = false);
                window.refresh();
            }
        });
    (el, color)
}

/// The tool group (XCARD-007…XCARD-016).
fn tools(
    env: &Rc<BoardEnv>,
    node: &SessionNode,
    agents: &[AgentInfo],
    handles: &DeckHandles,
    resumable: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let card = &node.card;
    let id = card.node.session_id.clone();
    let shell = is_terminal(card);
    let mut group = div()
        .id(anim::key(&id, "tools"))
        .flex()
        .flex_none()
        .items_center()
        .gap(z.px(2.))
        .ml_auto()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation());

    if !agents.is_empty() {
        // XCARD-009
        let open = handles.open(cx);
        let (b, color) = tool(
            env,
            &id,
            "tool-agents",
            false,
            open,
            Some(agent_deck::trigger_hover(handles, "tool-agents")),
            window,
            cx,
        );
        let run = running(agents);
        let mut b = b
            .px(z.px(7.))
            .tooltip(native_tooltip(present::deck_tooltip(agents).into()))
            .child(card_icon(CardIcon::Bot, z.f(14.), 2.0, color));
        if run > 0 {
            b = b.child(
                div()
                    .min_w(z.px(16.))
                    .h(z.px(16.))
                    .px(z.px(4.))
                    .rounded(z.px(8.))
                    .bg(fade(t.status_working, 0.18))
                    .text_color(t.status_working)
                    .text_size(z.px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(z.px(16.))
                    .font_features(fonts::tabular())
                    .flex()
                    .justify_center()
                    .child({
                        let mut font = super::ui_font(FontWeight::SEMIBOLD);
                        font.features = fonts::tabular();
                        super::para::label(run.to_string(), font, 11.0, 16.0, t.status_working, z.0)
                    }),
            );
        }
        group = group.child(agent_deck::trigger(b, handles));
    }
    if !shell {
        // XCARD-010: the mic is voice's (05); without it, its idle look.
        let mic = env.terminals.as_ref().and_then(|h| h.mic(&id, z.0, window, cx));
        group = group.child(match mic {
            Some(el) => el,
            None => {
                let (b, color) = tool(env, &id, "tool-mic", false, false, None, window, cx);
                b.child(card_icon(CardIcon::Mic, z.f(14.), 2.0, color))
                    .into_any_element()
            }
        });
    }
    if resumable {
        // XCARD-011
        let (b, color) = tool(env, &id, "tool-resume", true, false, None, window, cx);
        let sid = id.clone();
        group = group.child(
            b.tooltip(native_tooltip(
                if shell {
                    "Start a new shell in the project folder"
                } else {
                    "Resume the session"
                }
                .into(),
            ))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                wake_card_terminal(&sid, cx);
            })
            .child(card_icon(
                if shell { CardIcon::RotateCcw } else { CardIcon::Play },
                z.f(12.),
                2.2,
                color,
            ))
            .child(super::text(
                z,
                if shell { "Restart" } else { "Resume" },
                12.0,
                FontWeight::MEDIUM,
                color,
            )),
        );
    }
    if !shell && card.live {
        // XCARD-012
        let (b, color) = tool(env, &id, "tool-stop", false, false, None, window, cx);
        let env2 = env.clone();
        let sid = id.clone();
        group = group.child(
            b.tooltip(native_tooltip(
                "Stop the session. The card stays, and Resume picks it up.".into(),
            ))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                env2.actions.stop(std::slice::from_ref(&sid), cx);
            })
            .child(card_icon(CardIcon::Square, z.f(14.), 2.0, color)),
        );
    }
    if !shell {
        // XCARD-013
        let (b, color) = tool(env, &id, "tool-archive", false, false, None, window, cx);
        let env2 = env.clone();
        let sid = id.clone();
        group = group.child(
            b.tooltip(native_tooltip(format!("Archive ({})", env.platform.keys("⌫")).into()))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    env2.actions.archive(std::slice::from_ref(&sid), cx);
                })
                .child(card_icon(CardIcon::Archive, z.f(14.), 2.0, color)),
        );
    }
    {
        // XCARD-014
        let (b, color) = tool(env, &id, "tool-window", false, false, None, window, cx);
        let env2 = env.clone();
        let sid = id.clone();
        group = group.child(
            b.tooltip(native_tooltip("Open in a window".into()))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    env2.actions.open_terminal(&sid, cx);
                })
                .child(card_icon(CardIcon::AppWindowMac, z.f(14.), 2.0, color)),
        );
    }
    {
        // XCARD-015
        let (b, color) = tool(env, &id, "tool-collapse", false, false, None, window, cx);
        let env2 = env.clone();
        let sid = id.clone();
        group = group.child(
            b.tooltip(native_tooltip(
                if shell {
                    "Collapse (E). The shell keeps running."
                } else {
                    "Collapse (E). The session keeps running."
                }
                .into(),
            ))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                env2.actions.expand(std::slice::from_ref(&sid), false, cx);
            })
            .child(card_icon(CardIcon::Minimize2, z.f(14.), 2.0, color)),
        );
    }
    group.into_any_element()
}

/// Claude's status line in the title bar (XCARD-005; UsageStats.tsx), the
/// model hidden when the bar is narrow.
fn usage(
    env: &Rc<BoardEnv>,
    node: &str,
    usage: Option<&SessionUsage>,
    live: bool,
    with_model: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let Some(usage) = usage else {
        return div().into_any_element();
    };
    let t = theme::tokens(cx);
    let level = usage.context_percent.map(|p| {
        if p >= 90.0 {
            t.status_waiting
        } else if p >= 75.0 {
            t.status_working
        } else {
            t.text_tertiary
        }
    });
    let running = usage.duration_ms.map(|ms| {
        if live {
            anim::tick(window, cx, anim::key(node, "usage-tick"), Duration::from_secs(1));
        }
        let since = if live { (env.now_ms - usage.at).max(0.0) } else { 0.0 };
        format_duration(ms + since)
    });
    let item = |part: &'static str| div().id(anim::key(node, part)).flex().items_center().gap(z.px(5.));
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(z.px(10.))
        .text_size(z.px(11.))
        .line_height(z.px(15.4))
        .text_color(t.text_tertiary)
        .font_features(fonts::tabular())
        .whitespace_nowrap()
        .when_some(usage.model.clone().filter(|_| with_model), |d, model| {
            d.child(
                item("usage-model")
                    .text_color(t.text_secondary)
                    .font_weight(FontWeight::MEDIUM)
                    .tooltip(native_tooltip("Model".into()))
                    .child(super::text_tab(z, model, 11.0, FontWeight::MEDIUM, t.text_secondary)),
            )
        })
        .when_some(usage.context_percent, |d, p| {
            let color = level.unwrap_or(t.text_tertiary);
            d.child(
                item("usage-ctx")
                    .text_color(color)
                    .tooltip(native_tooltip(context_title(usage).into()))
                    .child(
                        div()
                            .relative()
                            .w(z.px(22.))
                            .h(z.px(4.))
                            .rounded(z.px(2.))
                            .bg(t.border)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(z.px(22.0 * (p.min(100.0) / 100.0) as f32))
                                    .rounded(z.px(2.))
                                    .bg(color),
                            ),
                    )
                    .child(super::text_tab(
                        z,
                        format!("Ctx {}%", nodus_domain::js::round(p)),
                        11.0,
                        FontWeight::NORMAL,
                        color,
                    )),
            )
        })
        .when_some(running, |d, text| {
            d.child(
                item("usage-time")
                    .tooltip(native_tooltip("Time the session has been running".into()))
                    .child(super::text_tab(z, text, 11.0, FontWeight::NORMAL, t.text_tertiary)),
            )
        })
        .when_some(usage.cost_usd, |d, cost| {
            d.child(
                item("usage-cost")
                    .tooltip(native_tooltip("Cost so far".into()))
                    .child(super::text_tab(
                        z,
                        format_cost(cost),
                        11.0,
                        FontWeight::NORMAL,
                        t.text_tertiary,
                    )),
            )
        })
        .into_any_element()
}

fn context_title(usage: &SessionUsage) -> String {
    match (usage.context_tokens, usage.context_size) {
        (Some(tokens), Some(size)) if size != 0.0 => {
            format!("{} of {} tokens in context", format_tokens(tokens), format_tokens(size))
        }
        _ => "Context window in use".into(),
    }
}

/// A blocking overlay over the body (`rgba(30,30,32,0.82)`, both themes).
fn blocking(content: AnyElement) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(30, 30, 32, 0.82))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(content)
        .into_any_element()
}

/// `.btn`: the overlays' buttons (spec 08), at the board's scale.
fn btn(
    env: &Rc<BoardEnv>,
    node: &str,
    part: &'static str,
    primary: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let z = env.z();
    let t = theme::tokens(cx);
    let hover = window.use_keyed_state(anim::key(node, part), cx, |_, _| false);
    let on = *hover.read(cx);
    let (rest, over) = if primary {
        (t.accent, t.accent_hover)
    } else {
        (t.bg_elevated, t.bg_hover)
    };
    // The hover colour of `.btn` is translucent: it shows the dark overlay through.
    let bg = transition(
        window,
        cx,
        anim::key_n(node, part, 1),
        if on { over } else { rest },
        Duration::from_millis(120),
        motion::ease,
    );
    let h2 = hover.clone();
    div()
        .id(anim::key(node, part))
        .flex()
        .flex_none()
        .items_center()
        .gap(z.px(6.))
        .h(z.px(28.))
        .px(z.px(12.))
        .rounded(z.px(6.))
        .border(z.px(0.5))
        .border_color(if primary { transparent() } else { t.border_strong })
        .bg(bg)
        .shadow(super::Shadow::with(&[], &t.shadow_sm).gpui(z))
        .text_size(z.px(13.))
        .line_height(z.px(18.2))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .text_color(if primary { t.text_on_accent } else { t.text_primary })
        .active(move |s| if primary { s } else { s.bg(t.bg_pressed) })
        .on_hover(move |h, window, cx| {
            h2.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// XCARD-022: `Resume session` / `Start shell`.
fn start_button(env: &Rc<BoardEnv>, node: &str, shell: bool, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let sid = node.to_owned();
    btn(env, node, "start", false, window, cx)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            wake_card_terminal(&sid, cx);
        })
        .child(card_icon(
            if shell {
                CardIcon::SquareTerminal
            } else {
                CardIcon::Play
            },
            z.f(13.),
            2.0,
            t.text_primary,
        ))
        .child(super::text(
            z,
            if shell { "Start shell" } else { "Resume session" },
            13.0,
            FontWeight::MEDIUM,
            t.text_primary,
        ))
        .into_any_element()
}

/// XCARD-023: the pill while the terminal starts, fading in after 150ms.
fn loading_pill(env: &Rc<BoardEnv>, node: &str, label: &'static str, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let fade_in = anim::enter(
        window,
        cx,
        anim::key(node, "loading"),
        Duration::from_millis(150),
        Duration::from_millis(300),
        motion::ease_out,
    )
    .unwrap_or(0.0);
    let spin = anim::cycle(window, cx, anim::key(node, "spin"), Duration::from_millis(900));
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap(z.px(8.))
                .h(z.px(30.))
                .px(z.px(14.))
                .rounded_full()
                .bg(rgba(255, 255, 255, 0.07))
                .shadow(super::Shadow::of(&[super::Shade::ring(0.5, rgba(255, 255, 255, 0.1))]).gpui(z))
                .text_color(rgba(255, 255, 255, 0.72))
                .text_size(z.px(12.))
                .line_height(z.px(16.8))
                .opacity(fade_in)
                .child(
                    div()
                        .flex()
                        .rotate(radians(spin * std::f32::consts::TAU))
                        .child(card_icon(
                            CardIcon::LoaderCircle,
                            z.f(14.),
                            2.0,
                            rgba(255, 255, 255, 0.72),
                        )),
                )
                .child(super::text(
                    z,
                    label,
                    12.0,
                    FontWeight::NORMAL,
                    rgba(255, 255, 255, 0.72),
                )),
        )
        .into_any_element()
}

/// XCARD-024: what went wrong, and Try again.
fn error_column(
    env: &Rc<BoardEnv>,
    node: &str,
    shell: bool,
    error: Option<String>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let host = env.terminals.clone();
    let sid = node.to_owned();
    let retry = btn(env, node, "retry", true, window, cx)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            if let Some(h) = &host {
                h.reattach(&sid, cx);
            }
        })
        .child(super::text(
            z,
            "Try again",
            13.0,
            FontWeight::MEDIUM,
            theme::tokens(cx).text_on_accent,
        ));
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(z.px(10.))
        .max_w(z.px(460.))
        .p(z.px(24.))
        .text_color(rgba(255, 255, 255, 0.85))
        .child(
            div()
                .text_size(z.px(15.))
                .line_height(z.px(21.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if shell {
                    "Couldn’t start the shell"
                } else {
                    "Couldn’t start Claude"
                }),
        )
        .when_some(error.filter(|e| !e.is_empty()), |d, e| {
            d.child(
                div()
                    .font_family(fonts::MONO)
                    .text_size(z.px(12.))
                    .line_height(z.px(16.8))
                    .text_color(rgba(255, 255, 255, 0.55))
                    .text_center()
                    .child(e),
            )
        })
        .child(retry)
        .into_any_element()
}

fn transparent() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.,
    }
}
