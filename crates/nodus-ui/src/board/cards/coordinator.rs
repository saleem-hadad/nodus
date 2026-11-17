//! The coordinator on its card and on its members' (spec 22, beta): its mark
//! before its title, a dot for each member that opens the members menu, and
//! a member's mark saying whom it reports to.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div,
};

use super::icons::{CardIcon, card_icon};
use super::status_dot::status_dot;
use super::{BoardEnv, Coordination, Member, Z, anim};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::{self, motion};

/// At most this many members get a dot of their own; the rest are counted.
const DOTS: usize = 8;

/// COORD-050: the coordinator's mark before its title.
pub fn mark(z: Z, size: f32, top: f32, color: Rgba) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .mt(z.px(top))
        .child(card_icon(CardIcon::Network, z.f(size), 2.0, color))
        .into_any_element()
}

/// A member's dot: its status dot on the board, a ring of its colour inside.
fn member_dot(z: Z, m: &Member, window: &mut Window, cx: &mut App) -> AnyElement {
    if !m.inside {
        return status_dot(anim::key(&m.id, "member-dot"), m.status, true, z, window, cx).into_any_element();
    }
    let t = theme::tokens(cx);
    div()
        .flex_none()
        .size(z.px(7.))
        .rounded_full()
        .border(z.px(1.5))
        .border_color(t.status(m.status))
        .into_any_element()
}

/// COORD-050: what follows its status: a dot for each member, how many,
/// and how many wait for the user. A press opens the members menu (COORD-051).
pub fn members_button(env: &Rc<BoardEnv>, c: &Coordination, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let hover = window.use_keyed_state(anim::key(&c.id, "members"), cx, |_, _| false);
    let on = *hover.read(cx);
    let bg = transition(
        window,
        cx,
        anim::key(&c.id, "members-bg"),
        if on { t.bg_hover } else { super::fade(t.bg_hover, 0.0) },
        Duration::from_millis(120),
        motion::ease,
    );
    let fg = if on { t.text_primary } else { t.text_tertiary };
    let waiting = c
        .members
        .iter()
        .filter(|m| m.status == nodus_domain::SessionStatus::Waiting)
        .count();
    let count = match c.members.len() {
        0 => "No sessions".to_owned(),
        1 => "1 session".to_owned(),
        n => format!("{n} sessions"),
    };
    let mut dots = div().flex().flex_none().items_center().gap(z.px(3.));
    for m in c.members.iter().take(DOTS) {
        dots = dots.child(member_dot(z, m, window, cx));
    }
    let actions = env.actions.clone();
    let mut button = div()
        .id(anim::key(&c.id, "members-button"))
        .flex()
        .flex_none()
        .items_center()
        .gap(z.px(5.))
        .h(z.px(18.))
        .px(z.px(5.))
        .rounded(z.px(5.))
        .bg(bg)
        .on_hover(move |h, window, cx| {
            hover.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            actions.open_members(e.position, cx);
        })
        .tooltip(native_tooltip("The sessions it looks after".into()));
    if !c.members.is_empty() {
        button = button.child(dots);
    }
    button = button.child(super::text_tab(z, count, 11.0, FontWeight::NORMAL, fg));
    if waiting > 0 {
        button = button.child(super::text_tab(
            z,
            format!("· {waiting} waiting for you"),
            11.0,
            FontWeight::NORMAL,
            t.text_secondary,
        ));
    }
    button.into_any_element()
}

/// COORD-053: a member's mark: whom it reports to, and a press moves the
/// board to its coordinator.
pub fn member_mark(env: &Rc<BoardEnv>, c: &Coordination, m: &Member, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let tip = if m.started {
        format!("Started by {}", c.title)
    } else {
        format!("Reports to {}", c.title)
    };
    let actions = env.actions.clone();
    div()
        .id(anim::key(&m.id, "reports-to"))
        .flex()
        .flex_none()
        .items_center()
        .h(z.px(18.))
        .px(z.px(3.))
        .mx(z.px(-3.))
        .rounded(z.px(5.))
        .hover(|d| d.bg(t.bg_hover))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            actions.show_coordinator(cx);
        })
        .tooltip(native_tooltip(tip.into()))
        .child(card_icon(CardIcon::Network, z.f(11.), 2.2, t.text_tertiary))
        .into_any_element()
}
