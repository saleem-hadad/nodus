//! The pad of sticky notes under the archive (`NotePadNode.tsx`;
//! NOTEUI-040…NOTEUI-042). Its top sheet is a note node of its own (the
//! canvas's draft, drawn by note.rs); the sheets here are what's left
//! underneath when it's peeled off.

use std::rc::Rc;

use gpui::{AnyElement, App, Div, FontWeight, IntoElement, ParentElement, Styled, Window, div, linear_color_stop, linear_gradient, px};

use super::icons::{CardIcon, card_icon};
use super::paper::paper;
use super::present::{NOTE_COLOR, NOTE_H, NOTE_W, label_scale};
use super::{BoardEnv, Shadow, Z, brightness};
use crate::board::nodes::PadNode;
use crate::theme::tokens::rgba;
use crate::theme::{self, fonts};

/// Where the pad's sheets sit inside it: centred under the archive's top card.
pub const PAD_SHEET_X: f32 = (344.0 - NOTE_W) / 2.0;

pub fn render(env: &Rc<BoardEnv>, node: &PadNode, window: &mut Window, cx: &mut App) -> AnyElement {
    pad(env, (node.node.size.width, node.node.size.height), window, cx)
}

/// A frame's header label: icon, words, at a scale that keeps it legible
/// zoomed out (ARCH-008, NOTEUI-041).
pub fn label_row(z: Z, zoom: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(z.px(8.))
        .transform_origin(0.0, 0.5)
        .scale(label_scale(zoom))
        .whitespace_nowrap()
}

/// Text in a frame label: 0.01em of its 13px apart (inherited by every part).
pub fn label_text(text: &str, z: Z, size: f32, weight: FontWeight, color: gpui::Rgba) -> AnyElement {
    super::text(z, text.to_owned(), size, weight, color)
        .spacing(z.px(0.13))
        .into_any_element()
}

/// The pad, `size` big.
pub fn pad(env: &Rc<BoardEnv>, size: (f32, f32), _window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let (w, h) = size;
    let label = t.archive_label;
    let header = div()
        .flex()
        .items_center()
        .h(z.px(52.))
        .px(z.px(22.))
        .text_color(label)
        .child(
            label_row(z, env.zoom)
                .child(card_icon(CardIcon::StickyNote, z.f(15.), 2.0, label))
                .child(label_text("Notes", z, 13.0, FontWeight::SEMIBOLD, label))
                .child(
                    div()
                        .ml(z.px(6.))
                        .child(label_text("Drag one onto the board", z, 12.0, FontWeight::NORMAL, label)),
                ),
        );
    // NOTEUI-042: two sheets peek out under the top one, like a block of paper.
    let yellow = paper(NOTE_COLOR, &t);
    let sheet = |ty: f32, scale: f32, bright: f32| {
        let s = div()
            .absolute()
            .inset_0()
            .rounded(z.px(4.))
            .bg(brightness(yellow, bright))
            .shadow(Shadow::with(&[], &t.note_shadow).brightness(bright).gpui(z))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w_full()
                    .h(z.px(22.))
                    .rounded_t(z.px(4.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgba(0, 0, 0, 0.035), 0.),
                        linear_color_stop(rgba(0, 0, 0, 0.0), 1.),
                    )),
            );
        if ty != 0.0 || scale != 1.0 {
            s.transform_origin(0.5, 1.0).translate(px(0.), z.px(ty)).scale(scale)
        } else {
            s
        }
    };
    div()
        .relative()
        .w(z.px(w))
        .h(z.px(h))
        .rounded(z.px(24.))
        .border(z.px(1.5))
        .border_dashed()
        .border_color(t.archive_border)
        .bg(t.archive_bg)
        .font_family(fonts::UI)
        .child(header)
        .child(
            div()
                .absolute()
                .left(z.px(PAD_SHEET_X))
                .top(z.px(52.0))
                .w(z.px(NOTE_W))
                .h(z.px(NOTE_H))
                .child(sheet(8.0, 0.97, 0.88))
                .child(sheet(4.0, 0.985, 0.94))
                .child(sheet(0.0, 1.0, 1.0)),
        )
        .into_any_element()
}
