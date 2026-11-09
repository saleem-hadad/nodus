//! A session on the board (`SessionCardNode.tsx`; CARD-001…CARD-048): the
//! compact card with its header, merged chip, preview and footer, or the
//! expanded card (expanded_card.rs), with the agent deck behind it, the fan
//! beside it and the notes it was started from stuck on top.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, Entity, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, RenderOnce, Rgba, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, canvas, div, px,
};
use nodus_domain::{AgentInfo, BoardNote, SessionCard, SessionStatus};

use super::agent_deck::{self, DeckHandles};
use super::icons::{CardIcon, card_icon};
use super::para::{font, highlight_runs, para};
use super::present::{
    self, PreviewKind, StackLook, agents_viewer_tooltip, deck_agents, deck_tooltip, is_expanded, is_terminal, preview,
    rename_commit, rename_initial, running, running_status, stack_visual, status_label, time_ago, updated_at,
};
use super::status_dot::status_dot;
use super::sticker::sticker;
use super::{BoardEnv, Shade, Shadow, Z, anim, brightness, expanded_card, fade, mix};
use crate::components::text_field::{focus_all, new_input};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::Tokens;
use crate::theme::{self, fonts, motion};

/// A card node: everything the canvas knows about it this frame.
#[derive(IntoElement)]
pub struct SessionNode {
    pub env: Rc<BoardEnv>,
    pub card: SessionCard,
    /// Title characters (UTF-16 indices) the archive search matched.
    pub matched: Vec<usize>,
    /// Notes the session was started from, stuck on the card (active cards).
    pub stickers: Vec<BoardNote>,
    pub selected: bool,
    pub dragging: bool,
    /// How an archived card sits in the stack or among the results.
    pub stack: Option<StackLook>,
    /// The node's size in flow units: 288×216, or the expanded card's.
    pub size: (f32, f32),
    /// When a search jump started flashing this card (BOARD-036).
    pub flash: Option<Instant>,
}

impl RenderOnce for SessionNode {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let env = self.env.clone();
        let z = env.z();
        let id = self.card.node.session_id.clone();
        let (w, h) = self.size;
        let handles = DeckHandles::get(&id, &env, window, cx);
        let agents: Vec<AgentInfo> = deck_agents(&self.card).to_vec();
        let has_deck = !agents.is_empty();
        // DECK-013: dragging the card, or its run ending, folds the fan.
        if (self.dragging || !has_deck) && handles.open(cx) {
            handles.fold(cx);
        }
        let open = handles.open(cx);
        let expanded = is_expanded(&self.card);

        let mut root = div()
            .id(anim::key(&id, "node"))
            .relative()
            .w(z.px(w))
            .h(z.px(h))
            .font_family(fonts::UI);

        // Where the node is on screen, for the fan's side (DECK-015).
        let bounds = handles.bounds.clone();
        root = root.child(
            canvas(
                move |b: Bounds<Pixels>, _, cx| bounds.update(cx, |s, _| *s = Some(b)),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );

        let mut zones = Vec::new();
        if has_deck {
            let (layers, z_zones) = agent_deck::deck(&env, &id, &agents, &handles, (w, h), window, cx);
            root = root.children(layers);
            zones = z_zones;
        }
        let body = if expanded {
            expanded_card::body(&env, &self, &agents, &handles, window, cx)
        } else {
            compact(&env, &self, &agents, &handles, window, cx)
        };
        root = root.child(body).children(zones);

        if !self.card.node.archived {
            let landing = present::sticker_lands(self.card.node.created_at, env.now_ms);
            for (i, note) in self.stickers.iter().enumerate() {
                root = root.child(sticker(&env, note, i, w, landing, &handles, window, cx));
            }
        }
        if expanded {
            root = root.children(expanded_card::chrome_extras(&env, &self, window, cx));
        }
        if open && has_deck {
            let total = self.card.agent_count.unwrap_or(agents.len() as u32);
            root = root.child(agent_deck::fan(&env, &id, &agents, total, &handles, (w, h), window, cx));
        }
        // DECK-011: a pinned fan closes on a press outside the node, or Escape.
        if handles.deck.read(cx).pinned {
            let h1 = handles.clone();
            let h2 = handles.clone();
            root = root.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                            if phase == gpui::DispatchPhase::Capture && !h1.over_node(cx) {
                                h1.close(window, cx);
                            }
                        });
                        window.on_key_event(move |e: &KeyDownEvent, phase, window, cx| {
                            if phase == gpui::DispatchPhase::Capture && e.keystroke.key == "escape" {
                                h2.close(window, cx);
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            );
        }
        root
    }
}

/// The card's surface: shadow, transform and opacity for its state
/// (CARD-002…CARD-008, ARCH-013…ARCH-017), as values that transition.
pub struct Surface {
    pub shadow: Shadow,
    pub ty: f32,
    pub scale: f32,
    pub bright: f32,
    pub opacity: f32,
    pub content: f32,
}

pub fn surface(
    t: &Tokens,
    card: &SessionCard,
    hovered: bool,
    selected: bool,
    dragging: bool,
    stack: Option<&StackLook>,
    expanded: bool,
) -> Surface {
    let lifted_top = stack.is_some_and(|s| !s.result && s.depth == 0 && s.lifted);
    let md = &t.shadow_md;
    let shadow = if lifted_top {
        Shadow::with(&[], md)
    } else if dragging {
        Shadow::with(&[Shade::ring(2.0, t.accent)], &t.shadow_lg)
    } else if selected {
        Shadow::with(&[Shade::ring(2.0, t.accent)], md)
    } else if card.status == SessionStatus::Waiting {
        Shadow::with(&[Shade::ring(1.0, fade(t.status_waiting, 0.45))], md)
    } else if hovered {
        Shadow::with(&[], md)
    } else {
        Shadow::with(&[], &t.shadow_sm)
    };
    let drag_scale = if dragging && !expanded { 1.015 } else { 1.0 };
    match stack {
        Some(look) => {
            let (ty, scale, bright, opacity, content) = stack_visual(look);
            // The stack's own transforms win; elsewhere the drag scale shows.
            let plain = look.result || (look.depth == 0 && !look.lifted);
            Surface {
                shadow,
                ty,
                scale: if plain { drag_scale } else { scale },
                bright,
                opacity,
                content: if content { 1.0 } else { 0.0 },
            }
        }
        None => Surface {
            shadow,
            ty: 0.0,
            scale: drag_scale,
            bright: 1.0,
            // CARD-008
            opacity: if card.node.archived && !(hovered || selected || dragging) {
                0.78
            } else {
                1.0
            },
            content: 1.0,
        },
    }
}

/// The surface's values, transitioning as the CSS says: 220ms on the
/// board, the stack's own timings in it (CARD-007, ARCH-015).
fn animate(node: &str, s: &Surface, stacked: bool, window: &mut Window, cx: &mut App) -> Surface {
    let out = motion::ease_out as fn(f32) -> f32;
    let (transform_ms, transform_ease, fade_ms): (u64, fn(f32) -> f32, u64) = if stacked {
        (560, motion::ease_stack, 260)
    } else {
        (220, out, 220)
    };
    let ms = Duration::from_millis;
    Surface {
        shadow: transition(window, cx, anim::key(node, "shadow"), s.shadow, ms(220), out),
        ty: transition(
            window,
            cx,
            anim::key(node, "ty"),
            s.ty,
            ms(transform_ms),
            transform_ease,
        ),
        scale: transition(
            window,
            cx,
            anim::key(node, "scale"),
            s.scale,
            ms(transform_ms),
            transform_ease,
        ),
        bright: transition(
            window,
            cx,
            anim::key(node, "bright"),
            s.bright,
            ms(fade_ms),
            motion::ease,
        ),
        opacity: transition(
            window,
            cx,
            anim::key(node, "opacity"),
            s.opacity,
            ms(fade_ms),
            motion::ease,
        ),
        content: transition(window, cx, anim::key(node, "content"), s.content, ms(200), motion::ease),
    }
}

/// Applies a surface's transform and opacity to the card body.
pub fn apply<E: Styled>(el: E, s: &Surface, z: Z, rise: Option<(f32, f32, f32)>) -> E {
    let mut el = el;
    let (mut tx, mut ty, mut scale, mut opacity) = (0.0, s.ty, s.scale, s.opacity);
    if let Some((fx, fy, p)) = rise {
        // ARCH-032: the rise replaces the transform while it runs.
        let k = 1.0 - p;
        tx = fx * k;
        ty = fy * k;
        scale = 0.88 + 0.12 * p;
        opacity *= p.clamp(0.0, 1.0);
    }
    if tx != 0.0 || ty != 0.0 || scale != 1.0 {
        el = el.transform_origin(0.5, 1.0).translate(z.px(tx), z.px(ty)).scale(scale);
    }
    if opacity < 1.0 {
        el = el.group_opacity(opacity.max(0.0));
    }
    el
}

/// BOARD-036's flash: for two rounds of 800ms (eased `--ease-out`) the
/// card's shadow is `0 0 0 2px accent, 0 0 0 2px→18px accent 55%→0%`, in
/// place of its own (a Web Animation replaces the property). `None` once
/// it's over.
pub fn flash_shadow(t: &Tokens, started: Option<Instant>, window: &mut Window) -> Option<Shadow> {
    const ROUND: f32 = 0.8;
    let elapsed = started?.elapsed().as_secs_f32();
    if elapsed >= 2.0 * ROUND {
        return None;
    }
    window.request_animation_frame();
    let p = motion::ease_out((elapsed % ROUND) / ROUND);
    Some(Shadow::of(&[
        Shade::ring(2.0, t.accent),
        Shade::ring(2.0 + 16.0 * p, fade(t.accent, 0.55 * (1.0 - p))),
    ]))
}

/// The heading's dot (or shell glyph), title (or rename field) and live
/// glyph (CARD-010…CARD-019, XCARD-004).
pub fn heading(
    env: &Rc<BoardEnv>,
    node: &SessionNode,
    expanded: bool,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let z = env.z();
    let t = theme::tokens(cx);
    let card = &node.card;
    let id = &card.node.session_id;
    let shell = is_terminal(card);
    let label = status_label(card);
    let top = |v: f32| if expanded { px(0.) } else { z.px(v) };
    let mut out = Vec::new();
    if shell {
        // CARD-016
        out.push(
            div()
                .id(anim::key(id, "shell"))
                .flex_none()
                .flex()
                .mt(top(3.))
                .tooltip(native_tooltip(format!("Terminal · {label}").into()))
                .child(card_icon(
                    CardIcon::SquareTerminal,
                    z.f(14.),
                    2.0,
                    if card.live { t.text_primary } else { t.text_tertiary },
                ))
                .into_any_element(),
        );
    } else {
        // CARD-011
        let dot = status_dot(anim::key(id, "dot"), card.status, false, z, window, cx);
        out.push(
            div()
                .id(anim::key(id, "dot-tip"))
                .flex_none()
                .mt(top(5.))
                .tooltip(native_tooltip(label.into()))
                .child(dot)
                .into_any_element(),
        );
    }
    let renaming = env.renaming.as_deref() == Some(id.as_str());
    if renaming {
        out.push(title_editor(env, card, window, cx));
    } else {
        // CARD-017, CARD-018, XCARD-004
        let title = present::title(card);
        let marks = highlight_runs(&title, &node.matched)
            .into_iter()
            .map(|r| (r, t.accent.into()))
            .collect();
        let size = if expanded { 13.0 } else { 15.0 };
        let face = font(fonts::DISPLAY, FontWeight::SEMIBOLD, false);
        let p = para(title.clone(), face, z.px(size), z.px(size * 1.3), t.text_primary)
            .zoom(z.0)
            .spacing(z.px(-0.01 * size))
            .marks(marks);
        let p = if expanded { p.ellipsis() } else { p.clamp(2) };
        let actions = env.actions.clone();
        out.push(
            div()
                .id(anim::key(id, "title"))
                .flex_1()
                .min_w_0()
                // `.card-title`: the canvas waits 240ms before opening (BOARD-043).
                .on_mouse_down(MouseButton::Left, move |_, _, _| actions.press_title())
                .tooltip(native_tooltip(title.into()))
                .child(p)
                .into_any_element(),
        );
    }
    if card.live && !shell {
        // CARD-019
        out.push(
            div()
                .id(anim::key(id, "live"))
                .flex_none()
                .flex()
                .mt(top(3.))
                .tooltip(native_tooltip("Claude process running".into()))
                .child(card_icon(CardIcon::SquareTerminal, z.f(13.), 2.0, t.text_tertiary))
                .into_any_element(),
        );
    }
    out
}

/// The compact card's body (CARD-001…CARD-042).
fn compact(
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
    let hover = window.use_keyed_state(anim::key(&id, "hover"), cx, |_, _| false);
    let hovered = *hover.read(cx);
    let s = surface(
        &t,
        card,
        hovered,
        node.selected,
        node.dragging,
        node.stack.as_ref(),
        false,
    );
    let mut s = animate(&id, &s, node.stack.is_some(), window, cx);
    if let Some(flash) = flash_shadow(&t, node.flash, window) {
        s.shadow = flash;
    }
    let rise = node.stack.and_then(|l| l.rise).map(|(fx, fy, slot)| {
        let p = anim::enter(
            window,
            cx,
            anim::key(&id, "rise"),
            Duration::from_millis(slot.min(4) as u64 * 30),
            Duration::from_millis(620),
            motion::ease_stack,
        );
        (fx, fy, p.unwrap_or(0.0))
    });

    let hs = hover.clone();
    let parts = handles.clone();
    let mut body = div()
        .id(anim::key(&id, "card"))
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(288.))
        .h(z.px(216.))
        .flex()
        .flex_col()
        .gap(z.px(8.))
        .pt(z.px(14.))
        .px(z.px(16.))
        .pb(z.px(11.))
        .rounded(z.px(14.))
        .bg(brightness(t.bg_elevated, s.bright))
        .shadow(s.shadow.brightness(s.bright).gpui(z))
        .text_size(z.px(13.))
        .line_height(z.px(18.2))
        // CARD-009
        .cursor(if node.dragging {
            CursorStyle::ClosedHand
        } else {
            CursorStyle::Arrow
        })
        .on_hover(move |h, window, cx| {
            hs.update(cx, |v, _| *v = *h);
            parts.part_hover("card", *h, window, cx);
            window.refresh();
        });
    body = apply(body, &s, z, rise);

    let content = s.content;
    let header = div()
        .flex()
        .items_start()
        .gap(z.px(9.))
        .min_h(z.px(20.))
        .opacity(content)
        .children(heading(env, node, false, window, cx));
    body = body.child(header);

    if !card.node.archived {
        body = body.child(corner(env, card, hovered, content, window, cx));
    }

    let chip = card.node.parents.as_ref().filter(|p| !p.is_empty()).map(|parents| {
        // CARD-025
        div()
            .id(anim::key(&id, "chip"))
            .flex()
            .flex_none()
            .items_center()
            .gap(z.px(4.))
            .self_start()
            .h(z.px(19.))
            .px(z.px(7.))
            .rounded_full()
            .bg(fade(t.accent, 0.10))
            .text_color(t.accent)
            .text_size(z.px(11.))
            .line_height(z.px(15.4))
            .font_weight(FontWeight::MEDIUM)
            .opacity(content)
            .tooltip(native_tooltip("Started from the context of earlier sessions".into()))
            .child(card_icon(CardIcon::GitMerge, z.f(11.), 2.2, t.accent))
            .child(super::text(
                z,
                format!("merged from {}", parents.len()),
                11.0,
                FontWeight::MEDIUM,
                t.accent,
            ))
    });
    let has_chip = chip.is_some();
    body = body.when_some(chip, ParentElement::child);

    // CARD-026…CARD-032
    let pv = preview(card, env.repo_path.as_deref());
    let preview_el = match pv.kind {
        PreviewKind::Path => para(
            pv.text,
            font(fonts::MONO, FontWeight::NORMAL, false),
            z.px(11.),
            z.px(15.95),
            t.text_secondary,
        )
        .zoom(z.0)
        .clamp(2),
        kind => {
            let italic = kind == PreviewKind::Prompt;
            let color = if kind == PreviewKind::Empty {
                t.text_tertiary
            } else {
                t.text_secondary
            };
            para(
                pv.text,
                font(fonts::UI, FontWeight::NORMAL, italic),
                z.px(12.),
                z.px(17.4),
                color,
            )
            .zoom(z.0)
            .clamp(if has_chip { 3 } else { 4 })
        }
    };
    body = body.child(div().flex_none().min_h_0().opacity(content).child(preview_el));

    body = body.child(footer(env, card, agents, handles, content, window, cx));
    let _ = shell;
    body.into_any_element()
}

/// A corner button: its part, icon, tooltip and action.
type CornerButton = (&'static str, CardIcon, String, Rc<dyn Fn(&mut App)>);

/// The corner's Archive and Expand buttons, shown on hover (CARD-020…CARD-024).
fn corner(
    env: &Rc<BoardEnv>,
    card: &SessionCard,
    hovered: bool,
    content: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = card.node.session_id.clone();
    let shown = transition(
        window,
        cx,
        anim::key(&id, "corner"),
        if hovered { 1.0 } else { 0.0 },
        Duration::from_millis(120),
        motion::ease,
    );
    let mut group = div()
        .id(anim::key(&id, "corner-group"))
        .absolute()
        .top(z.px(9.))
        .right(z.px(9.))
        .flex()
        .gap(z.px(2.))
        .opacity(content)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation());
    let mut buttons: Vec<CornerButton> = Vec::new();
    if !is_terminal(card) {
        let env = env.clone();
        let id = id.clone();
        buttons.push((
            "corner-archive",
            CardIcon::Archive,
            format!("Archive ({})", env.platform.keys("⌫")),
            Rc::new(move |cx| env.actions.archive(std::slice::from_ref(&id), cx)),
        ));
    }
    {
        let env = env.clone();
        let id = id.clone();
        buttons.push((
            "corner-expand",
            CardIcon::Maximize2,
            "Expand on the board (E)".into(),
            Rc::new(move |cx| env.actions.expand(std::slice::from_ref(&id), true, cx)),
        ));
    }
    for (part, icon, tip, act) in buttons {
        let bh = window.use_keyed_state(anim::key(&id, part), cx, |_, _| false);
        let on = *bh.read(cx);
        // CARD-021: --text-primary 7% into --bg-elevated.
        let bg = transition(
            window,
            cx,
            anim::key(
                &id,
                if part == "corner-archive" {
                    "corner-archive-bg"
                } else {
                    "corner-expand-bg"
                },
            ),
            if on {
                mix(t.text_primary, 0.07, t.bg_elevated)
            } else {
                t.bg_elevated
            },
            Duration::from_millis(120),
            motion::ease,
        );
        let color = if on { t.text_primary } else { t.text_secondary };
        let bh2 = bh.clone();
        group = group.child(
            div()
                .id(anim::key(&id, part))
                .flex()
                .items_center()
                .justify_center()
                .size(z.px(26.))
                .rounded(z.px(6.))
                .bg(bg)
                .opacity(shown)
                .tooltip(native_tooltip(tip.into()))
                .on_hover(move |h, window, cx| {
                    bh2.update(cx, |s, _| *s = *h);
                    window.refresh();
                })
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    act(cx);
                })
                .child(card_icon(icon, z.f(13.), 2.0, color)),
        );
    }
    group.into_any_element()
}

/// The footer: branch, messages, agents, shell state and time (CARD-033…CARD-042).
fn footer(
    env: &Rc<BoardEnv>,
    card: &SessionCard,
    agents: &[AgentInfo],
    handles: &DeckHandles,
    content: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = card.node.session_id.clone();
    let meta = card.meta.as_ref();
    let item = || div().flex().flex_none().items_center().gap(z.px(4.));
    // 11px, tabular, `--text-tertiary` unless said otherwise.
    let words = |s: String, color: Rgba| super::text_tab(z, s, 11.0, FontWeight::NORMAL, color);
    let mut foot = div()
        .flex()
        .items_center()
        .gap(z.px(12.))
        .mt_auto()
        .pt(z.px(9.))
        .border_t(z.px(0.5))
        .border_color(t.separator)
        .h(z.px(9.0 + 0.5 + 15.4))
        .opacity(content);
    if let Some(branch) = meta.and_then(|m| m.git_branch.clone()) {
        // CARD-034
        foot = foot.child(
            item()
                .id(anim::key(&id, "branch"))
                .flex_shrink(1.)
                .min_w_0()
                .tooltip(native_tooltip(branch.clone().into()))
                .child(card_icon(CardIcon::GitBranch, z.f(11.), 2.2, t.text_tertiary))
                .child(
                    super::text_tab(z, branch, 11.0, FontWeight::NORMAL, t.text_tertiary)
                        .ellipsis()
                        .hug()
                        .max_width(z.px(130.)),
                ),
        );
    }
    if let Some(n) = meta.map(|m| m.message_count).filter(|&n| n > 0) {
        // CARD-035
        foot = foot.child(
            item()
                .id(anim::key(&id, "messages"))
                .tooltip(native_tooltip(format!("{n} messages").into()))
                .child(card_icon(CardIcon::MessageSquare, z.f(11.), 2.2, t.text_tertiary))
                .child(words(n.to_string(), t.text_tertiary)),
        );
    }
    // CARD-039: the agents button.
    let agents_button = |count: String,
                         running: Option<(gpui::Div, usize)>,
                         hook: Option<agent_deck::HoverHook>,
                         window: &mut Window,
                         cx: &mut App| {
        let hover = window.use_keyed_state(anim::key(&id, "foot-agents"), cx, |_, _| false);
        let on = *hover.read(cx);
        let bg = transition(
            window,
            cx,
            anim::key(&id, "foot-agents-bg"),
            if on { t.bg_hover } else { transparent() },
            Duration::from_millis(120),
            motion::ease,
        );
        let fg = if on { t.text_primary } else { t.text_tertiary };
        let h2 = hover.clone();
        div()
            .id(anim::key(&id, "foot-agents"))
            .flex()
            .flex_none()
            .items_center()
            .gap(z.px(4.))
            .h(z.px(18.))
            .my(z.px(-2.))
            .mx(z.px(-5.))
            .px(z.px(5.))
            .rounded(z.px(5.))
            .bg(bg)
            .on_hover(move |h, window, cx| {
                h2.update(cx, |s, _| *s = *h);
                if let Some(hook) = &hook {
                    hook(*h, window, cx);
                }
                window.refresh();
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(card_icon(CardIcon::Bot, z.f(11.), 2.2, fg))
            .child(words(count, fg))
            .when_some(running, |d, (dot, run)| {
                // The running badge, `--text-secondary`.
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(z.px(4.))
                        .ml(z.px(3.))
                        .child(dot)
                        .child(words(run.to_string(), t.text_secondary)),
                )
            })
    };
    if !agents.is_empty() {
        // CARD-036, CARD-037
        let run = running(agents);
        let hook = agent_deck::trigger_hover(handles, "foot-agents");
        let badge = (run > 0).then(|| {
            let dot = status_dot(anim::key(&id, "foot-dot"), running_status(agents), true, z, window, cx);
            (dot, run)
        });
        let b = agents_button(agents.len().to_string(), badge, Some(hook), window, cx)
            .tooltip(native_tooltip(deck_tooltip(agents).into()));
        foot = foot.child(agent_deck::trigger(b, handles));
    } else if !card.node.archived
        && let Some(count) = card.agent_count.filter(|&c| c > 0)
    {
        // CARD-038: no run on now; the count opens the agents viewer.
        let env = env.clone();
        let sid = id.clone();
        foot = foot.child(
            agents_button(count.to_string(), None, None, window, cx)
                .tooltip(native_tooltip(agents_viewer_tooltip(count).into()))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    env.actions.open_agents(&sid, None, cx);
                }),
        );
    }
    if is_terminal(card) {
        // CARD-040
        foot = foot.child(item().child(words(status_label(card).into(), t.text_tertiary)));
    }
    // CARD-041, CARD-042
    foot = foot.child(
        item()
            .ml_auto()
            .child(words(time_ago(updated_at(card), env.now_ms), t.text_tertiary)),
    );
    foot.into_any_element()
}

fn transparent() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.,
    }
}

/// The inline rename field (CARD-044…CARD-048).
struct TitleEditor {
    input: Entity<InputState>,
    done: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

fn title_editor(env: &Rc<BoardEnv>, card: &SessionCard, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = card.node.session_id.clone();
    let initial = rename_initial(card);
    let editor = window.use_keyed_state(anim::key(&id, "rename"), cx, {
        let env = env.clone();
        let id = id.clone();
        move |window, cx: &mut Context<'_, TitleEditor>| {
            let input = new_input(&initial, "", false, window, cx);
            let done = Rc::new(Cell::new(false));
            let commit = done.clone();
            let sub = cx.subscribe(&input, move |_, input, e: &InputEvent, cx| {
                // CARD-046: Enter or leaving the field commits, once.
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) && !commit.replace(true) {
                    let value = input.read(cx).value().to_string();
                    if let Some(title) = rename_commit(&value, &initial) {
                        env.actions.rename(&id, &title, cx);
                    }
                    env.actions.set_renaming(None, cx);
                }
            });
            focus_all(&input, window, cx);
            TitleEditor {
                input,
                done,
                _subscriptions: vec![sub],
            }
        }
    });
    let (field, done) = {
        let e = editor.read(cx);
        (e.input.clone(), e.done.clone())
    };
    let env_esc = env.clone();
    div()
        .id(anim::key(&id, "rename-frame"))
        .flex_1()
        .min_w_0()
        .h(z.px(24.))
        .my(z.px(-2.))
        .px(z.px(5.))
        .rounded(z.px(5.))
        .bg(t.bg_window)
        .shadow(Shadow::of(&[Shade::ring(2.0, t.accent)]).gpui(z))
        .font_family(fonts::DISPLAY)
        .text_size(z.px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_primary)
        // CARD-047: Escape ends without saving; CARD-048: every key stays here.
        .capture_key_down(move |e: &KeyDownEvent, window, cx| {
            if e.keystroke.key == "escape" {
                cx.stop_propagation();
                done.set(true);
                env_esc.actions.set_renaming(None, cx);
                window.blur(cx);
            }
        })
        .on_key_down(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(super::field_line(
            &field,
            z.px(24.),
            z.px(15.),
            fonts::DISPLAY,
            FontWeight::SEMIBOLD,
        ))
        .into_any_element()
}

/// The element ids of a card's parts, for tests and the canvas's hit
/// regions.
pub fn card_id(session_id: &str) -> SharedString {
    SharedString::from(session_id.to_owned())
}
