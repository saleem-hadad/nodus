//! The archive frame (`ArchiveZoneNode.tsx`; ARCH-001…ARCH-036): a dashed
//! frame holding the stack of archived cards, whose search field shows when
//! pointed at, and which opens into a frosted panel of results. The cards
//! in it are session_card.rs's, given a [`super::StackLook`].

use std::rc::Rc;
use std::time::Duration;

use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, Focusable as _, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use super::border::Stroke;
use super::icons::{CardIcon, card_icon};
use super::note_pad::{label_row, label_text};
use super::para::para;
use super::present::{
    ARCHIVE_HEADER_H, ARCHIVE_PAD, CARD_H, CARD_W, STACK_BAND_H, STACK_RESULTS_X, STACK_SEARCH_H, STACK_SEARCH_Y,
    SearchKey, dealt_hint, dealt_placeholder, dealt_title, no_match_text, scrollbar, search_key, search_visible,
};
use super::{BoardEnv, Shade, Shadow, anim, ui_font};
use crate::board::nodes::{ArchiveNode, BoardCx};
use crate::components::text_field::new_input;
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::{self, fonts, motion};

const ID: &str = "__archive__";

pub fn render(
    env: &Rc<BoardEnv>,
    node: &ArchiveNode,
    board: &BoardCx,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let _ = board;
    archive(
        env,
        node.count,
        (node.node.size.width, node.node.size.height),
        node.node.dragging,
        window,
        cx,
    )
}

/// The frame, `size` big (the folded stack or the open panel), holding
/// `count` archived cards.
pub fn archive(
    env: &Rc<BoardEnv>,
    count: usize,
    size: (f32, f32),
    dragging: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let stack = &env.stack;
    let hot = env.archive_hot;
    let open = stack.open;
    let empty = count == 0;
    let searching = search_visible(count, stack.hover, open, &stack.query);
    let ms = Duration::from_millis;

    // ARCH-005: the frame eases between its folded and open sizes.
    let w = transition(window, cx, anim::key(ID, "w"), size.0, ms(460), motion::ease_out);
    let h = transition(window, cx, anim::key(ID, "h"), size.1, ms(460), motion::ease_out);

    // ARCH-002 folded, ARCH-003 hot (solid accent, `Drop to archive`),
    // ARCH-004 open (open wins where both apply).
    let (bg, border, dashed) = if open {
        (t.archive_panel, t.border, false)
    } else if hot {
        (t.archive_drop, t.accent, false)
    } else {
        (t.archive_bg, t.archive_border, true)
    };
    let bg = transition(window, cx, anim::key(ID, "bg"), bg, ms(220), motion::ease);
    let border = transition(window, cx, anim::key(ID, "border"), border, ms(220), motion::ease);
    let shadow = transition(
        window,
        cx,
        anim::key(ID, "shadow"),
        if open {
            Shadow::with(&[], &t.shadow_lg)
        } else {
            Shadow::of(&[])
        },
        ms(320),
        motion::ease_out,
    );

    // ARCH-007…ARCH-009: the header.
    let header_hover = window.use_keyed_state(anim::key(ID, "header-hover"), cx, |_, _| false);
    let hint = transition(
        window,
        cx,
        anim::key(ID, "hint"),
        if *header_hover.read(cx) || hot { 1.0 } else { 0.0 },
        ms(220),
        motion::ease,
    );
    let label_color = if hot { t.accent } else { t.archive_label };
    let hh = header_hover.clone();
    let header = div()
        .id(anim::key(ID, "header"))
        .flex()
        .items_center()
        .h(z.px(ARCHIVE_HEADER_H))
        .px(z.px(22.))
        .rounded_t(z.px(24.))
        .cursor(if dragging {
            CursorStyle::ClosedHand
        } else {
            CursorStyle::OpenHand
        })
        .tooltip(native_tooltip("Drag to move the archive".into()))
        .on_hover(move |h, window, cx| {
            hh.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        .child(
            label_row(z, env.zoom)
                .child(card_icon(CardIcon::Archive, z.f(15.), 2.0, label_color))
                .child(label_text("Archive", z, 13.0, FontWeight::SEMIBOLD, label_color))
                .child(div().px(z.px(7.)).rounded_full().bg(t.bg_sunken).child({
                    let mut font = ui_font(FontWeight::SEMIBOLD);
                    font.features = fonts::tabular();
                    super::para::label(count.to_string(), font, 11.0, 18.0, label_color, z.0).spacing(z.px(0.13))
                }))
                .child(div().ml(z.px(6.)).opacity(hint).child(label_text(
                    if hot {
                        "Drop to archive"
                    } else {
                        "Drag cards out to reactivate"
                    },
                    z,
                    12.0,
                    FontWeight::NORMAL,
                    label_color,
                ))),
        );

    let env_click = env.clone();
    let mut frame = div()
        .id(anim::key(ID, "frame"))
        .relative()
        .w(z.px(w))
        .h(z.px(h))
        .rounded(z.px(24.))
        // The border is drawn by `border::border` (exact width and dashes);
        // this one only insets the contents as CSS's does.
        .border(z.px(1.5))
        .border_color(gpui::transparent_black())
        .bg(bg)
        .font_family(fonts::UI)
        .cursor(CursorStyle::Arrow)
        .when(open, |d| d.backdrop_blur(z.px(30.)).backdrop_saturate(1.8))
        // ARCH-011: a click on the folded frame opens the panel.
        .when(!open, |d| d.on_click(move |_, _, cx| env_click.actions.open_stack(cx)))
        .child(header);

    // Children are placed from the padding box, inside the 1.5px border.
    if empty {
        // ARCH-010
        frame = frame.child(placeholder(env, None, window, cx));
    }
    if dealt_placeholder(open, count, stack.result_count) {
        // ARCH-033
        frame = frame.child(placeholder(env, Some(count), window, cx));
    }
    if count > 0 {
        frame = frame.child(search(env, count, searching, window, cx));
    }
    // ARCH-034: the divider fades in after the panel opens.
    let divider = anim::transition_delayed(
        window,
        cx,
        anim::key(ID, "divider"),
        if open { 1.0 } else { 0.0 },
        ms(220),
        if open { ms(120) } else { ms(0) },
        motion::ease,
    );
    frame = frame.child(
        div()
            .absolute()
            .left(z.px(STACK_RESULTS_X - 20.0))
            .top(z.px(ARCHIVE_HEADER_H))
            .w(z.px(1.))
            .h(z.px((h - 3.0 - ARCHIVE_HEADER_H - ARCHIVE_PAD).max(0.0)))
            .bg(t.separator)
            .opacity(divider),
    );
    if open && stack.result_count == 0 {
        // ARCH-035
        let fade = anim::enter(window, cx, anim::key(ID, "no-match"), ms(160), ms(240), motion::ease).unwrap_or(0.0);
        frame = frame.child(
            div()
                .absolute()
                .left(z.px(STACK_RESULTS_X))
                .top(z.px(ARCHIVE_HEADER_H))
                .w(z.px(CARD_W))
                .h(z.px(CARD_H))
                .flex()
                .items_center()
                .justify_center()
                .opacity(fade)
                .child(
                    para(
                        no_match_text(&stack.query),
                        ui_font(FontWeight::NORMAL),
                        z.px(13.),
                        z.px(18.2),
                        t.text_tertiary,
                    )
                    .zoom(z.0)
                    .center(),
                ),
        );
    }
    if open && stack.scroll_max > 0.0 {
        // ARCH-036
        let (thumb, offset) = scrollbar(stack.scroll.min(stack.scroll_max), stack.scroll_max);
        let fade = anim::enter(window, cx, anim::key(ID, "scrollbar"), ms(160), ms(240), motion::ease).unwrap_or(0.0);
        frame = frame.child(
            div()
                .absolute()
                .right(z.px(11.))
                .top(z.px(ARCHIVE_HEADER_H))
                .w(z.px(5.))
                .h(z.px(STACK_BAND_H))
                .opacity(fade)
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(z.px(offset))
                        .w_full()
                        .h(z.px(thumb))
                        .rounded(z.px(3.))
                        .bg(t.text_tertiary)
                        .opacity(0.55),
                ),
        );
    }
    // ARCH-004: the open panel's shadow, outside it only (it's see-through).
    div()
        .relative()
        .w(z.px(w))
        .h(z.px(h))
        .children(super::outer_shadow(&shadow, w, h, 24.0, z))
        .children(shadow.rings(24.0, z))
        .child(frame)
        .child(super::border::border(
            (w, h),
            24.0,
            Stroke {
                width: 1.5,
                color: border,
                dashed,
            },
            z,
        ))
        .into_any_element()
}

/// The dashed outline at the stack's spot: empty (ARCH-010), or with every
/// card dealt out (ARCH-033).
fn placeholder(env: &Rc<BoardEnv>, dealt: Option<usize>, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let outline = div()
        .absolute()
        .left(z.px(ARCHIVE_PAD))
        .top(z.px(ARCHIVE_HEADER_H))
        .w(z.px(CARD_W))
        .h(z.px(CARD_H))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .rounded(z.px(14.))
        .child(super::border::border(
            (CARD_W, CARD_H),
            14.0,
            Stroke {
                width: 1.5,
                color: t.archive_border,
                dashed: true,
            },
            z,
        ))
        .text_color(t.archive_label)
        .text_size(z.px(12.))
        .line_height(z.px(16.8));
    match dealt {
        None => outline
            .child(super::text(
                z,
                "Drag a card here to archive it",
                12.0,
                FontWeight::NORMAL,
                t.archive_label,
            ))
            .into_any_element(),
        Some(count) => {
            let fade = anim::enter(
                window,
                cx,
                anim::key(ID, "dealt"),
                Duration::from_millis(200),
                Duration::from_millis(240),
                motion::ease,
            )
            .unwrap_or(0.0);
            outline
                .gap(z.px(3.))
                .opacity(fade)
                .child(super::text(
                    z,
                    dealt_title(count),
                    13.0,
                    FontWeight::SEMIBOLD,
                    t.archive_label,
                ))
                .child(super::text(
                    z,
                    dealt_hint(env.stack.scroll_max),
                    12.0,
                    FontWeight::NORMAL,
                    t.archive_label,
                ))
                .into_any_element()
        }
    }
}

/// The search field's state: the field, and whether it showed (and the
/// panel was open) last frame, to focus it as it appears.
struct Search {
    field: Entity<InputState>,
    /// The count the placeholder names.
    count: usize,
    shown: bool,
    open: bool,
    _subscriptions: Vec<Subscription>,
}

/// The search under the stack (ARCH-019…ARCH-028).
fn search(env: &Rc<BoardEnv>, count: usize, visible: bool, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let stack = env.stack.clone();
    let state = window.use_keyed_state(anim::key(ID, "search"), cx, {
        let env = env.clone();
        let query = stack.query.clone();
        move |window, cx: &mut Context<'_, Search>| {
            let field = new_input(&query, &format!("Search {count} sessions"), false, window, cx);
            let typing = env.clone();
            // ARCH-022: typing sets the query.
            let sub = cx.subscribe(&field, move |_, field, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let value = field.read(cx).value().to_string();
                    typing.actions.set_stack_query(&value, cx);
                }
            });
            Search {
                field,
                count,
                shown: false,
                open: false,
                _subscriptions: vec![sub],
            }
        }
    });
    let field = state.read(cx).field.clone();
    // ARCH-021
    let placeholder = format!("Search {count} sessions");
    if state.read(cx).count != count {
        state.update(cx, |s, _| s.count = count);
        let f = field.clone();
        window.defer(cx, move |window, cx| {
            f.update(cx, |f, cx| f.set_placeholder(placeholder, window, cx));
        });
    }
    // The query is the board's: follow it when it changes elsewhere (closing clears it).
    if field.read(cx).value().as_ref() != stack.query {
        let q = stack.query.clone();
        let f = field.clone();
        window.defer(cx, move |window, cx| f.update(cx, |f, cx| f.set_value(q, window, cx)));
    }
    // ARCH-020: focus as it appears (or the panel opens), unless a text field
    // has the focus; leave as it hides.
    let (was_shown, was_open) = {
        let s = state.read(cx);
        (s.shown, s.open)
    };
    state.update(cx, |s, _| {
        s.shown = visible;
        s.open = stack.open;
    });
    let focus_handle = field.focus_handle(cx);
    let focused = focus_handle.is_focused(window);
    if visible && (!was_shown || (stack.open && !was_open)) && !env.text_focus && !focused {
        let f = field.clone();
        window.defer(cx, move |window, cx| f.update(cx, |f, cx| f.focus(window, cx)));
    } else if !visible && focused {
        window.defer(cx, Window::blur);
    }

    let ms = Duration::from_millis;
    let opacity = transition(
        window,
        cx,
        anim::key(ID, "search-o"),
        if visible { 1.0 } else { 0.0 },
        ms(180),
        motion::ease,
    );
    let ty = transition(
        window,
        cx,
        anim::key(ID, "search-ty"),
        if visible { 0.0 } else { -6.0 },
        ms(360),
        motion::ease_stack,
    );
    let scale = transition(
        window,
        cx,
        anim::key(ID, "search-s"),
        if visible { 1.0 } else { 0.97 },
        ms(360),
        motion::ease_stack,
    );
    // ARCH-028
    let ring = transition(
        window,
        cx,
        anim::key(ID, "search-ring"),
        if focused {
            Shadow::with(&[Shade::ring(3.0, t.focus_ring)], &t.shadow_sm)
        } else {
            Shadow::with(&[], &t.shadow_sm)
        },
        ms(120),
        motion::ease,
    );

    let env_click = env.clone();
    let f_click = field.clone();
    let env_key = env.clone();
    let mut el = div()
        .id(anim::key(ID, "search-box"))
        // ARCH-021
        .aria_label("Search archived sessions")
        .absolute()
        .left(z.px(ARCHIVE_PAD))
        .top(z.px(STACK_SEARCH_Y))
        .w(z.px(CARD_W))
        .h(z.px(STACK_SEARCH_H))
        .flex()
        .items_center()
        .gap(z.px(7.))
        .pl(z.px(11.))
        .pr(z.px(9.))
        .rounded(z.px(10.))
        .bg(t.bg_elevated)
        .shadow(ring.soft(z))
        .children(ring.rings(10.0, z))
        .text_color(t.text_primary)
        .font_family(fonts::UI)
        .text_size(z.px(13.))
        .cursor(CursorStyle::IBeam)
        .opacity(opacity)
        .transform_origin(0.5, 0.5)
        .translate(px(0.), z.px(ty))
        .scale(scale)
        .child(card_icon(CardIcon::Search, z.f(13.), 2.2, t.text_tertiary))
        .child(div().flex_1().min_w_0().child(super::field_line(
            &field,
            z.px(STACK_SEARCH_H),
            z.px(13.),
            fonts::UI,
            FontWeight::NORMAL,
        )));
    if visible {
        el = el
            // ARCH-026: a click anywhere in the box opens the panel.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                env_click.actions.open_stack(cx);
                f_click.update(cx, |f, cx| f.focus(window, cx));
            })
            // ARCH-023…ARCH-025
            .capture_key_down(move |e: &KeyDownEvent, window, cx| {
                match search_key(&e.keystroke.key, env_key.stack.open) {
                    SearchKey::Close => {
                        cx.stop_propagation();
                        env_key.actions.close_stack(cx);
                        window.blur(cx);
                    }
                    // ARCH-024
                    SearchKey::Submit => {
                        cx.stop_propagation();
                        env_key.actions.submit_stack(window, cx);
                    }
                    SearchKey::Open => {
                        cx.stop_propagation();
                        env_key.actions.open_stack(cx);
                    }
                    SearchKey::Nothing => {}
                }
            })
            .on_key_down(|_, _, cx| cx.stop_propagation());
        if !stack.query.is_empty() {
            // ARCH-027
            let env_clear = env.clone();
            let f_clear = field.clone();
            el = el.child(
                div()
                    .id(anim::key(ID, "search-clear"))
                    .aria_label("Clear search")
                    .flex_none()
                    .size(z.px(15.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.text_tertiary)
                    .cursor(CursorStyle::Arrow)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        env_clear.actions.set_stack_query("", cx);
                        f_clear.update(cx, |f, cx| {
                            f.set_value("", window, cx);
                            f.focus(window, cx);
                        });
                    })
                    .child(card_icon(CardIcon::X, z.f(9.), 3.0, t.bg_elevated)),
            );
        }
    }
    el.into_any_element()
}
