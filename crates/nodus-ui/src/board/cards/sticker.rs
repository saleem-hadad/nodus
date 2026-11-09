//! A note stuck on the card of the session it started (`NoteSticker` in
//! NoteNode.tsx; NOTEUI-033…NOTEUI-039).

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, linear_color_stop, linear_gradient, px, radians,
};
use nodus_domain::BoardNote;

use super::agent_deck::DeckHandles;
use super::icons::{CardIcon, card_icon};
use super::para::{Space, para};
use super::paper::paper;
use super::{BoardEnv, Shadow, anim, ui_font};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::rgba;
use crate::theme::{self, motion};

pub const STICKER_W: f32 = 84.0;
pub const STICKER_H: f32 = 54.0;

/// Sticker `index` on a card node `width` wide. `landing` plays it sticking
/// on (NOTEUI-038).
#[allow(clippy::too_many_arguments)]
pub fn sticker(
    env: &Rc<BoardEnv>,
    note: &BoardNote,
    index: usize,
    width: f32,
    landing: bool,
    handles: &DeckHandles,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let paper = paper(note.color, &t);
    let key = |part| anim::key_n(&note.id, part, index);
    let hover_state = window.use_keyed_state(key("sticker-hover"), cx, |_, _| false);
    let hovered = *hover_state.read(cx);

    // NOTEUI-033: right edge 18 + 30i in, bottom edge 6px below the card's top.
    let left = width - (18.0 + 30.0 * index as f32) - STICKER_W;
    let top = 6.0 - STICKER_H;

    // NOTEUI-034/035: 3° at rest, 1° and 2px up pointed at.
    let ease = motion::ease_out as fn(f32) -> f32;
    let rot = transition(window, cx, key("sticker-rot"), if hovered { 1.0 } else { 3.0 }, Duration::from_millis(220), ease);
    let ty = transition(window, cx, key("sticker-ty"), if hovered { -2.0 } else { 0.0 }, Duration::from_millis(220), ease);
    let shadow = transition(
        window,
        cx,
        key("sticker-shadow"),
        Shadow::with(&[], if hovered { &t.note_shadow_lifted } else { &t.note_shadow }),
        Duration::from_millis(220),
        ease,
    );
    let x_opacity = transition(window, cx, key("sticker-x"), if hovered { 1.0 } else { 0.0 }, Duration::from_millis(120), motion::ease);

    // NOTEUI-038: a session just started from the note, 520ms on the spring.
    let land = if landing && !cx.reduce_motion() {
        anim::enter(window, cx, key("sticker-land"), Duration::ZERO, Duration::from_millis(520), motion::ease_spring)
    } else {
        Some(1.0)
    };
    let p = land.unwrap_or(0.0);
    let k = 1.0 - p;
    let (land_ty, land_rot, land_scale) = (-14.0 * k, -6.0 * k + rot * p, 1.35 * k + p);
    let (final_rot, final_ty, scale) = if p < 1.0 { (land_rot, land_ty + ty * p, land_scale) } else { (rot, ty, 1.0) };

    let hs = hover_state.clone();
    let parts = handles.clone();
    let env_x = env.clone();
    let note_id = note.id.clone();
    let mut el = div()
        .id(key("sticker"))
        .absolute()
        .left(z.px(left))
        .top(z.px(top))
        .w(z.px(STICKER_W))
        .h(z.px(STICKER_H))
        .pt(z.px(6.))
        .px(z.px(8.))
        .rounded(z.px(3.))
        .bg(paper)
        .shadow(shadow.gpui(z))
        .transform_origin(0.5, 1.0)
        .translate(px(0.), z.px(final_ty))
        .rotate(radians(final_rot.to_radians()))
        .tooltip(native_tooltip(note.text.clone().into()))
        .on_hover(move |h, window, cx| {
            hs.update(cx, |s, _| *s = *h);
            parts.part_hover("sticker", *h, window, cx);
            window.refresh();
        })
        // The paper's top edge, a touch darker where it sticks.
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .w_full()
                .h(z.px(10.))
                .rounded_t(z.px(3.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0, 0, 0, 0.04), 0.),
                    linear_color_stop(rgba(0, 0, 0, 0.0), 1.),
                )),
        )
        // NOTEUI-036
        .child(
            para(note.text.clone(), ui_font(gpui::FontWeight::NORMAL), z.px(11.), z.px(13.75), t.note_ink)
                .space(Space::PreWrap)
                .clamp(3),
        )
        // NOTEUI-037: × deletes at once.
        .child(
            div()
                .id(key("sticker-remove"))
                .absolute()
                .top(z.px(-6.))
                .right(z.px(-6.))
                .size(z.px(16.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.note_ink)
                .opacity(x_opacity)
                .cursor(CursorStyle::Arrow)
                .tooltip(native_tooltip("Delete note".into()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    env_x.actions.remove_notes(std::slice::from_ref(&note_id), cx);
                })
                .child(card_icon(CardIcon::X, z.f(9.), 3.0, t.note_paper)),
        );
    if scale != 1.0 {
        el = el.scale(scale);
    }
    if p < 1.0 {
        el = el.group_opacity(p.clamp(0.0, 1.0));
    }
    el.into_any_element()
}
