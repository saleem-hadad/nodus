//! The receipt (`Receipt`, CAP-140…CAP-147): a sticky note for a note, a
//! card for a session, a message sent or a view, shrinking in from the panel.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, linear_color_stop, linear_gradient, px, radians, relative,
};
use nodus_core::capture_service::page::clip;
use nodus_domain::CaptureKind;

use super::panel::{CapturePanel, Shown};
use super::render::{dot, spinner, stage, swatch};
use super::style::{self, Inks};
use crate::components::tween::enter;
use crate::parity;
use crate::theme::motion;
use crate::theme::tokens::Tokens;

/// `.cap-receipt-stage`'s top padding.
const RECEIPT_TOP: f32 = 52.0;
/// `cap-shrink`: 280 ms on `--ease-spring`.
const SHRINK: std::time::Duration = std::time::Duration::from_millis(280);

/// Where `cap-shrink` is: opacity, scale and lift for an eased progress `e`
/// (from `opacity: 0.4; transform: scale(1.5) translateY(18px)`).
fn shrink(e: f32) -> (f32, f32, f32) {
    let opacity = (0.4 + 0.6 * e).clamp(0.0, 1.0);
    let scale = 1.5 - 0.5 * e;
    let lift = 18.0 * (1.0 - e);
    (opacity, scale, lift)
}

impl CapturePanel {
    pub(super) fn render_receipt(
        &mut self,
        shown: &Shown,
        t: Tokens,
        ink: Inks,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let progress = enter(
            window,
            cx,
            ("cap-receipt-shrink", self.receipt_epoch as usize),
            SHRINK,
            |x| x,
        );
        let e = motion::ease_spring(progress);
        let receipt = &shown.receipt;
        let project_id = receipt.project_id.clone();
        let session_id = receipt.session_id.clone();
        let open = cx.listener(move |p, _, _, cx| p.open(project_id.clone(), session_id.clone(), cx));
        let paper = style::note_paper(&t);
        let body = if receipt.kind == CaptureKind::Note {
            // CAP-141
            let (opacity, scale, lift) = shrink(e);
            let note = div()
                .relative()
                .flex_none()
                .w(px(216.))
                .min_h(px(160.))
                .px(px(18.))
                .py(px(16.))
                .rounded(px(4.))
                .bg(paper)
                .shadow(style::note_shadow(&t, true))
                .text_color(t.note_ink)
                .text_size(px(15.))
                .line_height(relative(1.4))
                .whitespace_normal()
                // `transform: rotate(-1.5deg)`, which the shrink animates towards.
                .when(progress < 1.0, |d| d.group_opacity(opacity))
                .translate(px(0.), px(lift * 1.5))
                .rotate(radians((-1.5 * e).to_radians()))
                .scale(scale)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(22.))
                        .rounded_t(px(4.))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(style::NOTE_SHADE, 0.),
                            linear_color_stop(gpui::transparent_black(), 1.),
                        )),
                )
                .child(div().relative().child(SharedString::from(clip(&shown.text, 160))));
            let caption = div()
                .flex()
                .flex_none()
                .items_center()
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(t.bg_elevated)
                .shadow(t.sm())
                .text_size(px(12.))
                .text_color(t.text_secondary)
                .child(SharedString::from(format!(
                    "On {}’s board ·\u{a0}",
                    receipt.project_name
                )))
                .child(link("cap-open", "Open", ink).on_click(open));
            vec![note.into_any_element(), caption.into_any_element()]
        } else {
            vec![self.render_receipt_card(shown, t, ink, e, progress, open, cx)]
        };
        stage(RECEIPT_TOP)
            .children(body)
            .child(self.measure())
            .into_any_element()
    }

    /// CAP-142, CAP-143: the card for a session, a message or a view.
    #[allow(clippy::too_many_arguments)]
    fn render_receipt_card(
        &self,
        shown: &Shown,
        t: Tokens,
        ink: Inks,
        e: f32,
        progress: f32,
        open: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let receipt = &shown.receipt;
        let view = receipt.kind == CaptureKind::View;
        let session = receipt.kind == CaptureKind::Session;
        let frozen = parity::frozen(cx);
        let color = self
            .targets
            .as_ref()
            .and_then(|ts| ts.projects.iter().find(|p| p.id == receipt.project_id))
            .map(|p| p.color);
        let top = div().flex().items_center().gap(px(7.)).text_size(px(11.)).map(|d| {
            if view {
                d.text_color(ink.view)
                    .child(spinner("cap-receipt-spin", 12., ink.view, frozen))
                    .child("Building view")
            } else {
                d.child(dot(Some(nodus_domain::SessionStatus::Working), t)).child(
                    div().text_color(ink.working).font_weight(FontWeight::SEMIBOLD).child(
                        if receipt.kind == CaptureKind::Into {
                            "Sent"
                        } else {
                            "Working"
                        },
                    ),
                )
            }
        });
        let line = match receipt.kind {
            CaptureKind::Session => "Claude is starting on it, expanded on the board.",
            CaptureKind::Into => "Your message is in. Claude is on it.",
            _ => "Its brain is putting it together.",
        };
        let (opacity, scale, lift) = shrink(e);
        div()
            .relative()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(8.))
            .w(px(288.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(12.))
            .bg(t.bg_elevated)
            .shadow(style::receipt_shadow())
            .when(progress < 1.0, |d| {
                d.group_opacity(opacity).scale(scale).translate(px(0.), px(lift))
            })
            .child(top)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(px(20.))
                    .when(session, |d| d.pr(px(56.)))
                    .child(SharedString::from(receipt.title.clone())),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(t.text_tertiary)
                    .child(line),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(7.))
                    .border_t_1()
                    .border_color(t.separator)
                    .text_size(px(11.))
                    .child(link("cap-open-board", "Open on board", ink).on_click(open))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_color(t.text_secondary)
                            .child(swatch(color))
                            .child(SharedString::from(receipt.project_name.clone()))
                            .when(session, |d| {
                                d.child(div().text_color(t.text_tertiary).child("· auto mode"))
                            }),
                    ),
            )
            // A new session wears what was asked, as its card does on the board.
            .when(session, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-20.))
                        .right(px(-36.))
                        .w(px(104.))
                        .px(px(8.))
                        .py(px(7.))
                        .rounded(px(2.))
                        .bg(style::note_paper(&t))
                        .shadow(style::note_shadow(&t, false))
                        .text_color(t.note_ink)
                        .text_size(px(10.))
                        .line_height(px(13.))
                        .rotate(radians(4f32.to_radians()))
                        .child(SharedString::from(clip(&shown.text, 70))),
                )
            })
            .into_any_element()
    }
}

/// `.cap-link`: the receipt's link, underlined on hover.
fn link(id: &'static str, label: &'static str, ink: Inks) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink.into)
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(label)
}
