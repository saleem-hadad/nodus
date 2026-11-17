//! A card's drawer (spec 19, DRAWER-030…041): the pull requests its session
//! opened and the servers it started, pulled out from under the card. At
//! rest it's a lip of chips; pointed at, its rows slide out over whatever is
//! below, and a click on the lip pins it open, as the agents fan does.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, CursorStyle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div,
};
use nodus_domain::SessionCard;

use super::icons::{CardIcon, card_icon};
use super::para::{font, para};
use super::present::{
    DECK_CLOSE_DELAY, DeckState, DrawerItem, DrawerServer, LIP_CHIPS, PrGlyph, Tone, drawer_items, drawer_summary,
    pr_look, pr_title, pr_tooltip, uptime,
};
use super::{BoardEnv, Shade, Shadow, Z, anim, brightness, fade};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::Tokens;
use crate::theme::{self, fonts, motion};

/// Its sides stand this far in from the card's (DRAWER-031).
const INSET: f32 = 12.0;
const PAD_TOP: f32 = 3.0;
const LIP: f32 = 22.0;
const PAD_BOTTOM: f32 = 4.0;
const ROW: f32 = 26.0;
const RADIUS: f32 = 12.0;

/// What the drawer keeps between frames: whether it's open (the fan's
/// rules), the rows it showed last (drawn while it slides shut), and the
/// servers asked to stop.
#[derive(Debug, Default)]
struct DrawerMemory {
    open: DeckState,
    /// The pointer is over the drawer (the fan's state keeps `hover` until
    /// its close comes due).
    inside: bool,
    last: Vec<DrawerItem>,
    stopping: HashSet<u16>,
}

/// The drawer under a card `size` big, below its agent deck's layers
/// (`below` past the card's bottom edge); none when it has nothing to show
/// and has slid shut.
pub fn drawer(
    env: &Rc<BoardEnv>,
    card: &SessionCard,
    size: (f32, f32),
    below: f32,
    fan_open: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = card.node.session_id.clone();
    let memory = window.use_keyed_state(anim::key(&id, "drawer"), cx, |_, _| DrawerMemory::default());
    let items = drawer_items(card, env.server_host.as_deref());
    // DRAWER-035: it slides out when its first item arrives, and back in
    // after its last goes, drawing what it showed last on the way.
    let shown = transition(
        window,
        cx,
        anim::key(&id, "drawer-shown"),
        if items.is_empty() { 0.0 } else { 1.0 },
        Duration::from_millis(420),
        motion::ease_stack,
    );
    if !items.is_empty() {
        memory.update(cx, |m, _| {
            m.stopping
                .retain(|port| items.iter().any(|i| i.key() == format!(":{port}")));
            m.last = items.clone();
        });
    }
    if shown <= 0.001 {
        if items.is_empty() {
            memory.update(cx, |m, _| m.open.close());
        }
        return None;
    }
    let items = memory.read(cx).last.clone();
    let stopping = memory.read(cx).stopping.clone();
    let open_now = memory.read(cx).open.open();
    let open = transition(
        window,
        cx,
        anim::key(&id, "drawer-open"),
        if open_now { 1.0 } else { 0.0 },
        Duration::from_millis(220),
        motion::ease_out,
    );
    let (w, h) = size;
    let rows_h = items.len() as f32 * ROW + 2.0;
    let full = PAD_TOP + LIP + PAD_BOTTOM + open * rows_h;
    let visible = full * shown.clamp(0.0, 1.2);
    // A shade under the card's edge, and a shadow that deepens as it opens.
    let shadow = if open > 0.5 { &t.shadow_md } else { &t.shadow_sm };
    let shadow = Shadow::with(
        &[Shade {
            y: 6.0,
            blur: 6.0,
            spread: -6.0,
            color: fade(rgba_black(), if t.dark { 0.5 } else { 0.1 }),
            inset: true,
            ..Shade::NONE
        }],
        shadow,
    );

    let pinned = memory.read(cx).open.pinned;
    let lip = lip(z, &t, &id, &items, open, pinned, env, &memory);
    let rows = div()
        .flex()
        .flex_col()
        .pt(z.px(2.))
        .opacity(open)
        .children(items.iter().enumerate().map(|(i, item)| {
            row(
                env,
                &t,
                &id,
                i,
                item,
                stopping.contains(&port_of(item)),
                &memory,
                window,
                cx,
            )
        }));

    let hover_memory = memory.clone();
    let raise_env = env.clone();
    let raise_id = id.clone();
    let mut el = div()
        .id(anim::key(&id, "drawer-box"))
        .aria_label(SharedString::from(drawer_summary(&items)))
        .absolute()
        .left(z.px(INSET))
        .top(z.px(h + below))
        .w(z.px(w - 2.0 * INSET))
        .h(z.px(visible))
        .rounded_b(z.px(RADIUS))
        .bg(brightness(t.bg_elevated, if t.dark { 0.9 } else { 0.98 }))
        .shadow(shadow.soft(z))
        .overflow_hidden()
        .font_family(fonts::UI)
        .cursor(CursorStyle::Arrow)
        // DRAWER-036: the drawer is its own; presses on it never reach the
        // card or the board.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_hover(move |hovered, window, cx| {
            let memory = hover_memory.clone();
            let was = memory.read(cx).open.open();
            memory.update(cx, |m, _| {
                m.inside = *hovered;
                if *hovered {
                    m.open.point();
                } else {
                    m.open.leave(Instant::now());
                }
            });
            if !*hovered && !memory.read(cx).open.pinned {
                let memory = memory.clone();
                let raise_env = raise_env.clone();
                let raise_id = raise_id.clone();
                window
                    .spawn(cx, async move |cx| {
                        cx.background_executor()
                            .timer(DECK_CLOSE_DELAY + Duration::from_millis(5))
                            .await;
                        let _ = cx.update(|window, cx| {
                            let closed = memory.update(cx, |m, _| m.open.tick(Instant::now()));
                            if closed {
                                raised(&raise_env, &raise_id, false, fan_open, cx);
                                window.refresh();
                            }
                        });
                    })
                    .detach();
            } else if !was && memory.read(cx).open.open() {
                raised(&raise_env, &raise_id, true, fan_open, cx);
            }
            window.refresh();
        })
        .child(
            // Anchored to the bottom, so it comes out from under the card.
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(z.px(full))
                .flex()
                .flex_col()
                .pt(z.px(PAD_TOP))
                .px(z.px(6.))
                .pb(z.px(PAD_BOTTOM))
                .child(lip)
                .child(rows),
        );

    // DRAWER-037: a pinned drawer closes on a press anywhere else, or Escape.
    if memory.read(cx).open.pinned {
        let (m1, m2) = (memory.clone(), memory.clone());
        let (e1, e2) = (env.clone(), env.clone());
        let (i1, i2) = (id.clone(), id.clone());
        el = el.child(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let (m1, e1, i1) = (m1.clone(), e1.clone(), i1.clone());
                    window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                        if phase == gpui::DispatchPhase::Capture && !m1.read(cx).inside {
                            m1.update(cx, |m, _| m.open.close());
                            raised(&e1, &i1, false, fan_open, cx);
                            window.refresh();
                        }
                    });
                    let (m2, e2, i2) = (m2.clone(), e2.clone(), i2.clone());
                    window.on_key_event(move |e: &KeyDownEvent, phase, window, cx| {
                        if phase == gpui::DispatchPhase::Capture && e.keystroke.key == "escape" {
                            m2.update(cx, |m, _| m.open.close());
                            raised(&e2, &i2, false, fan_open, cx);
                            window.refresh();
                        }
                    });
                },
            )
            .absolute()
            .size_0(),
        );
    }
    Some(el.into_any_element())
}

/// DRAWER-038: an open drawer lifts its card over the others, as the fan
/// does; it lets go unless the fan still holds it up.
fn raised(env: &Rc<BoardEnv>, id: &str, up: bool, fan_open: bool, cx: &mut App) {
    if up || !fan_open {
        let (env, id) = (env.clone(), id.to_owned());
        cx.defer(move |cx| env.actions.raise(&id, up, cx));
    }
}

fn rgba_black() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 1.,
    }
}

fn port_of(item: &DrawerItem) -> u16 {
    match item {
        DrawerItem::Server(s) => s.port,
        DrawerItem::Pr(_) => 0,
    }
}

fn tone(t: &Tokens, tone: Tone) -> Rgba {
    match tone {
        Tone::Quiet => t.text_tertiary,
        Tone::Green => t.diff_add,
        Tone::Orange => t.status_working,
        Tone::Red => t.diff_del,
        Tone::Purple => crate::chat::purple(t),
    }
}

fn glyph(g: PrGlyph) -> CardIcon {
    match g {
        PrGlyph::Open => CardIcon::GitPullRequest,
        PrGlyph::Draft => CardIcon::GitPullRequestDraft,
        PrGlyph::Merged => CardIcon::GitMerge,
        PrGlyph::Closed => CardIcon::GitPullRequestClosed,
    }
}

fn dot(z: Z, color: Rgba) -> gpui::Div {
    div().flex_none().size(z.px(6.)).rounded_full().bg(color)
}

/// One line of mono text, as wide as it is (`#53`, `:5173`, an address).
fn mono(z: Z, s: impl Into<SharedString>, size: f32, color: Rgba) -> super::para::Para {
    super::para::label(
        s,
        font(fonts::mono(), FontWeight::NORMAL, false),
        size,
        size * 1.4,
        color,
        z.0,
    )
}

#[allow(clippy::too_many_arguments)]
/// DRAWER-032: the lip: a chip for each item (`#53 ●`, `:5173`), `+N` past
/// four, and a chevron; open, it says what's in the drawer instead.
fn lip(
    z: Z,
    t: &Tokens,
    id: &str,
    items: &[DrawerItem],
    open: f32,
    pinned: bool,
    env: &Rc<BoardEnv>,
    memory: &gpui::Entity<DrawerMemory>,
) -> AnyElement {
    let chips = items.iter().take(LIP_CHIPS).enumerate().map(|(i, item)| {
        let (icon, color, text, check, tip) = match item {
            DrawerItem::Pr(pr) => {
                let (g, c, _, check) = pr_look(pr);
                (glyph(g), tone(t, c), format!("#{}", pr.number), check, pr_tooltip(pr))
            }
            DrawerItem::Server(s) => (
                CardIcon::AppWindowMac,
                t.diff_add,
                format!(":{}", s.port),
                None,
                format!("{} · {}", s.url.trim_start_matches("http://"), s.label),
            ),
        };
        div()
            .id(anim::key_n(id, "drawer-chip", i))
            .flex()
            .flex_none()
            .items_center()
            .gap(z.px(4.))
            .tooltip(native_tooltip(tip.into()))
            .child(card_icon(icon, z.f(12.), 2.0, color))
            .child(mono(z, text, 11.0, t.text_secondary))
            .when_some(check, |d, c| d.child(dot(z, tone(t, c))))
    });
    let more = items.len().saturating_sub(LIP_CHIPS);
    let pin = memory.clone();
    let pin_env = env.clone();
    let pin_id = id.to_owned();
    div()
        .id(anim::key(id, "drawer-lip"))
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .h(z.px(LIP))
        .px(z.px(6.))
        .rounded(z.px(6.))
        .cursor(CursorStyle::PointingHand)
        .tooltip(native_tooltip(
            if pinned {
                "Click to let it close"
            } else {
                "Click to keep it open"
            }
            .into(),
        ))
        // DRAWER-037: a click pins it open, another lets it close.
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            let was = pin.read(cx).open.open();
            pin.update(cx, |m, _| m.open.toggle());
            let now = pin.read(cx).open.open();
            if was != now {
                raised(&pin_env, &pin_id, now, false, cx);
            }
            window.refresh();
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(z.px(10.))
                .opacity(1.0 - open)
                .children(chips)
                .when(more > 0, |d| {
                    d.child(mono(z, format!("+{more}"), 11.0, t.text_tertiary))
                }),
        )
        .child(
            div()
                .absolute()
                .left(z.px(6.))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .opacity(open)
                .child(super::text(
                    z,
                    drawer_summary(items),
                    11.0,
                    FontWeight::NORMAL,
                    t.text_tertiary,
                )),
        )
        .child(
            div().flex_none().ml(z.px(6.)).child(
                card_icon(CardIcon::ChevronDown, z.f(12.), 2.0, t.text_tertiary)
                    .with_transformation(gpui::Transformation::rotate(gpui::percentage(open * 0.5))),
            ),
        )
        .into_any_element()
}

/// DRAWER-033/034: one row: a pull request (glyph, number, title, where it
/// stands) or a server (address, what it runs, how long), which opens in the
/// browser; pointed at, its actions take the right-hand side.
#[allow(clippy::too_many_arguments)]
fn row(
    env: &Rc<BoardEnv>,
    t: &Tokens,
    id: &str,
    i: usize,
    item: &DrawerItem,
    stopping: bool,
    memory: &gpui::Entity<DrawerMemory>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let hover = window.use_keyed_state(anim::key_n(id, "drawer-row-hover", i), cx, |_, _| false);
    let on = *hover.read(cx) && !stopping;
    let url = match item {
        DrawerItem::Pr(pr) => pr.url.clone(),
        DrawerItem::Server(s) => s.url.clone(),
    };
    let mut el = div()
        .id(anim::key_n(id, "drawer-row", i))
        .flex()
        .flex_none()
        .items_center()
        .gap(z.px(7.))
        .h(z.px(ROW))
        .px(z.px(6.))
        .rounded(z.px(6.))
        .when(on, |d| d.bg(t.bg_hover))
        .when(stopping, |d| d.opacity(0.45))
        .cursor(CursorStyle::PointingHand)
        .on_hover({
            let hover = hover.clone();
            move |h, window, cx| {
                hover.update(cx, |v, _| *v = *h);
                window.refresh();
            }
        })
        .on_click({
            let url = url.clone();
            move |_, _, cx| {
                cx.stop_propagation();
                crate::servers::open_url(&url, cx);
            }
        });
    match item {
        DrawerItem::Pr(pr) => {
            let (g, color, says, check) = pr_look(pr);
            el = el
                .aria_label(SharedString::from(format!(
                    "Pull request #{}: {}",
                    pr.number,
                    pr_title(pr)
                )))
                .tooltip(native_tooltip(pr_tooltip(pr).into()))
                .child(card_icon(glyph(g), z.f(13.), 2.0, tone(t, color)))
                .child(mono(z, format!("#{}", pr.number), 11.0, t.text_tertiary))
                .child(
                    para(
                        pr_title(pr),
                        font(fonts::UI, FontWeight::NORMAL, false),
                        z.px(12.),
                        z.px(16.8),
                        t.text_primary,
                    )
                    .zoom(z.0)
                    .ellipsis()
                    .grow(),
                );
            if on {
                el = el.child(action(
                    z,
                    t,
                    id,
                    i,
                    "open",
                    CardIcon::ArrowUpRight,
                    "Open on GitHub",
                    None,
                ));
            } else if !says.is_empty() {
                el = el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(z.px(4.))
                        .when_some(check, |d, c| d.child(dot(z, tone(t, c))))
                        .child(super::text_tab(z, says, 11.0, FontWeight::NORMAL, t.text_tertiary)),
                );
            }
        }
        DrawerItem::Server(s) => {
            el = el
                .aria_label(SharedString::from(format!("Server on port {}: {}", s.port, s.label)))
                .tooltip(native_tooltip(format!("Open {}", s.url).into()))
                .child(card_icon(CardIcon::AppWindowMac, z.f(13.), 2.0, t.diff_add))
                .child(mono(
                    z,
                    s.url.trim_start_matches("http://").to_owned(),
                    11.5,
                    t.text_primary,
                ))
                .child(
                    super::text(z, s.label.clone(), 11.0, FontWeight::NORMAL, t.text_tertiary)
                        .ellipsis()
                        .grow(),
                );
            if on {
                el = el.child(action(
                    z,
                    t,
                    id,
                    i,
                    "open",
                    CardIcon::ArrowUpRight,
                    "Open in the browser",
                    None,
                ));
                if s.stoppable {
                    el = el.child(stop_action(env, t, id, i, s, memory));
                }
            } else if let Some(started) = s.started_at {
                el = el.child(super::text_tab(
                    z,
                    uptime(started, env.now_ms),
                    11.0,
                    FontWeight::NORMAL,
                    t.text_tertiary,
                ));
            }
        }
    }
    el.into_any_element()
}

/// What a row's button does when clicked.
type OnClick = Box<dyn Fn(&mut Window, &mut App)>;

/// A small square button on a row; a click on it is the row's own unless
/// `click` says otherwise.
#[allow(clippy::too_many_arguments)]
fn action(
    z: Z,
    t: &Tokens,
    id: &str,
    i: usize,
    part: &'static str,
    icon: CardIcon,
    tip: &'static str,
    click: Option<OnClick>,
) -> gpui::Stateful<gpui::Div> {
    let filled = icon == CardIcon::SquareFilled;
    let mut el = div()
        .id(anim::key_n(id, part, i))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(z.px(20.))
        .mr(z.px(-3.))
        .rounded(z.px(5.))
        .hover(|d| d.bg(t.bg_pressed))
        .tooltip(native_tooltip(tip.into()))
        .child(card_icon(
            icon,
            z.f(if filled { 10. } else { 12. }),
            2.0,
            t.text_secondary,
        ));
    if let Some(click) = click {
        el = el.on_click(move |_, window, cx| {
            cx.stop_propagation();
            click(window, cx);
        });
    }
    el
}

/// DRAWER-024: Stop, which asks the server's process to end and dims the
/// row until it has.
fn stop_action(
    env: &Rc<BoardEnv>,
    t: &Tokens,
    id: &str,
    i: usize,
    s: &DrawerServer,
    memory: &gpui::Entity<DrawerMemory>,
) -> gpui::Stateful<gpui::Div> {
    let (env2, session, port, memory) = (env.clone(), id.to_owned(), s.port, memory.clone());
    action(
        env.z(),
        t,
        id,
        i,
        "stop",
        CardIcon::SquareFilled,
        "Stop the server",
        Some(Box::new(move |window, cx| {
            memory.update(cx, |m, _| {
                m.stopping.insert(port);
            });
            env2.actions.stop_server(&session, port, cx);
            window.refresh();
        })),
    )
}
