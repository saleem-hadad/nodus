//! Drawing the merge dialog (board.css `.merge-*`): the header, the
//! gathering list or the review (chips, Context, instruction, Name), and
//! the footer (MERGE-005, MERGE-008, MERGE-011, MERGE-013, MERGE-017).

use gpui::component::Sizable as _;
use gpui::component::input::{Input, Textarea};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, linear_color_stop, linear_gradient, px,
};
use nodus_board::merge::Step;
use nodus_domain::SummarizePhase;

use super::super::icons::{OvIcon, ov_icon};
use super::super::spinner;
use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::button::button;
use crate::components::icon::Icon;
use crate::components::modal::Modal;
use crate::platform::keys;
use crate::theme::tokens::{TEXT_LG, TEXT_MD, TEXT_SM, TEXT_XS, Tokens, hard, hex, ring};
use crate::theme::{self, fonts};

/// A phase's icon, 15px (`PhaseIcon`).
fn phase_icon(t: &Tokens, phase: SummarizePhase, id: &'static str, cx: &gpui::App) -> AnyElement {
    match phase {
        SummarizePhase::Running => spinner(id, 15., 2., t.status_working, cx),
        SummarizePhase::Done => ov_icon(OvIcon::CheckCircle2, 15., 2., t.status_done).into_any_element(),
        SummarizePhase::Error => ov_icon(OvIcon::AlertCircle, 15., 2., t.status_waiting).into_any_element(),
        SummarizePhase::Queued => ov_icon(OvIcon::Circle, 15., 2., t.text_tertiary).into_any_element(),
    }
}

/// An input's frame: `--bg-window`, a hairline, the accent and a focus ring
/// while focused.
fn frame(t: &Tokens, focused: bool) -> gpui::Div {
    div()
        .rounded(px(8.))
        .border(px(0.5))
        .border_color(if focused { t.accent } else { t.border_strong })
        .bg(t.bg_window)
        .when(focused, |d| d.shadow(vec![ring(3., t.focus_ring)]))
}

impl BoardCanvas {
    pub(crate) fn render_merge(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        self.merge_sync(window, cx);
        self.merge_prepare(window, cx);
        let d = self.overlays.merge.as_ref()?;
        let t = theme::tokens(cx);
        let review = d.m.step == Step::Review;
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let width = if review {
            820f32.min(vw - 48.)
        } else {
            560f32.min(vw - 48.)
        };
        let header = self.render_merge_head(&t);
        let body = if review {
            self.render_merge_review(&t, window, cx)
        } else {
            self.render_merge_gathering(&t, cx)
        };
        let foot = self.render_merge_foot(&t, cx);
        let content = div()
            .id("merge-inner")
            .flex()
            .flex_col()
            .min_h_0()
            .when(review, |d| d.h(px((vh * 0.86).min(780.).min(vh - 48.))))
            .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                let m = e.keystroke.modifiers;
                match e.keystroke.key.as_str() {
                    // MERGE-002: Esc anywhere, even in a field.
                    "escape" => {
                        cx.stop_propagation();
                        this.merge_cancel(cx);
                    }
                    "enter" if crate::platform::Platform::CURRENT.mod_key(&m) => {
                        cx.stop_propagation();
                        this.merge_begin(cx);
                    }
                    _ => {}
                }
            }))
            .child(header)
            .child(body)
            .child(foot);
        let mut modal = Modal::new("merge-dialog", px(width), |_, _| {});
        modal.close_on_backdrop = false;
        Some(modal.render(content, window, cx))
    }

    fn render_merge_head(&self, t: &Tokens) -> AnyElement {
        let d = self.overlays.merge.as_ref().expect("open");
        let (title, subtitle) = match (d.m.step, &d.m.focus) {
            (Step::Gathering, Some(focus)) => (
                "Gathering context",
                format!(
                    "Each session sums up what it did about “{focus}” in a forked copy. The originals stay untouched."
                ),
            ),
            (Step::Gathering, None) => (
                "Gathering context",
                "Each session summarizes itself in a forked copy. The originals stay untouched.".to_owned(),
            ),
            (Step::Review, _) => (
                "New session from context",
                "Review and edit the handoff, then tell the new session what to do.".to_owned(),
            ),
        };
        div()
            .flex()
            .items_start()
            .gap(px(12.))
            .pt(px(20.))
            .px(px(22.))
            .pb(px(16.))
            .border_b(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .rounded(px(9.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(t.accent, 0.),
                        linear_color_stop(hex(0x5e5ce6), 1.),
                    ))
                    .child(crate::components::icon::icon(Icon::Sparkles, 16., 2.2, hard::WHITE)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(TEXT_LG)
                            .line_height(px(21.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(TEXT_SM)
                            .line_height(px(16.8))
                            .text_color(t.text_secondary)
                            .child(subtitle),
                    ),
            )
            .into_any_element()
    }

    /// MERGE-005.
    fn render_merge_gathering(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let d = self.overlays.merge.as_ref().expect("open");
        const SPIN: [&str; 8] = ["m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7"];
        let rows = d.m.list().enumerate().map(|(i, (_, it))| {
            let state = match it.phase {
                SummarizePhase::Queued => "Queued",
                SummarizePhase::Running => "Summarizing…",
                SummarizePhase::Done => "Ready",
                SummarizePhase::Error => "Failed",
            };
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(38.))
                .px(px(12.))
                .rounded(px(9.))
                .bg(t.bg_sunken)
                .child(phase_icon(t, it.phase, SPIN[i % SPIN.len()], cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_weight(FontWeight::MEDIUM)
                        .child(it.title.clone()),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(if it.phase == SummarizePhase::Error {
                            t.status_waiting
                        } else {
                            t.text_tertiary
                        })
                        .child(state),
                )
        });
        let s = if crate::parity::frozen(cx) {
            0
        } else {
            d.since.elapsed().as_secs()
        };
        let elapsed = if s > 0 {
            format!("{s}s elapsed · ")
        } else {
            String::new()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .py(px(16.))
            .px(px(22.))
            .child(div().flex().flex_col().gap(px(4.)).children(rows))
            .child(
                div()
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.text_tertiary)
                    .child(format!("{elapsed}Usually 10–30 seconds per session")),
            )
            .into_any_element()
    }

    /// MERGE-008, MERGE-011, MERGE-013.
    fn render_merge_review(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let d = self.overlays.merge.as_ref().expect("open");
        let chips: Vec<AnyElement> =
            d.m.list()
                .enumerate()
                .map(|(i, (id, it))| self.merge_chip(t, i, id, it, cx))
                .collect();
        let (context, instruction, name) = (d.context.clone(), d.instruction.clone(), d.name.clone());
        let instruction_spell = d.instruction_spell.clone();
        let dirty = d.m.dirty;
        let instruction_h = d.instruction_h;
        let grip = self.render_instruction_grip(t, cx);
        let focused = |f: &gpui::FocusHandle| f.is_focused(window);
        use gpui::Focusable as _;
        let (cf, inf, nf) = (
            focused(&context.read(cx).focus_handle(cx)),
            focused(&instruction.read(cx).focus_handle(cx)),
            focused(&name.read(cx).focus_handle(cx)),
        );
        let label = |text: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_secondary)
                .child(text)
        };
        let me = cx.entity().downgrade();
        let reset = dirty.then(|| {
            div()
                .id("merge-reset")
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.accent)
                .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(move |_, _, cx| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, BoardCanvas::merge_reset_context);
                    }
                })
                .child("Reset to generated")
        });
        div()
            .id("merge-review")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .py(px(16.))
            .px(px(22.))
            .overflow_y_scroll()
            .child(div().flex().flex_wrap().gap(px(6.)).children(chips))
            .child(
                div()
                    .flex_1()
                    .min_h(px(220.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("Context").children(reset))
                    .child(
                        frame(t, cf).flex_1().min_h_0().py(px(12.)).pl(px(10.)).child(
                            Textarea::new(&context)
                                .appearance(false)
                                .xsmall()
                                .h(gpui::relative(1.))
                                .font_family(fonts::MONO)
                                .text_size(TEXT_SM)
                                .line_height(px(18.6)),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("What should the new session do?"))
                    .child(
                        frame(t, inf)
                            .relative()
                            .py(px(10.))
                            .pl(px(8.))
                            .when_some(instruction_h, |d, h| d.h(px(h)))
                            .child(
                                Textarea::new(&instruction)
                                    .appearance(false)
                                    .xsmall()
                                    .when_some(instruction_h, |d, h| d.h(px((h - 21.).max(0.))))
                                    .text_size(TEXT_MD)
                                    .line_height(px(18.85)),
                            )
                            .child(crate::components::spellcheck::underlines(
                                &instruction_spell,
                                &instruction,
                                TEXT_MD,
                                1.,
                            ))
                            .child(grip),
                    ),
            )
            .child(
                div().flex().flex_col().gap(px(6.)).child(label("Name")).child(
                    frame(t, nf)
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .child(Styled::h(
                            Input::new(&name)
                                .appearance(false)
                                .px(px(0.))
                                .py(px(0.))
                                .text_size(TEXT_MD)
                                .line_height(px(18.2)),
                            px(18.2),
                        )),
                ),
            )
            .into_any_element()
    }

    /// A source chip (MERGE-008): its phase, title, and Retry / Skip when failed.
    fn merge_chip(
        &self,
        t: &Tokens,
        i: usize,
        id: &str,
        it: &nodus_board::merge::Item,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        const SPIN: [&str; 8] = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"];
        let failed = it.phase == SummarizePhase::Error;
        let blocking = failed && !it.skipped;
        let icon_phase = if it.skipped { SummarizePhase::Queued } else { it.phase };
        let (retry, skip) = (cx.entity().downgrade(), cx.entity().downgrade());
        let (retry_id, skip_id) = (id.to_owned(), id.to_owned());
        let actions = blocking.then(|| {
            div()
                .flex()
                .gap(px(2.))
                .ml(px(4.))
                .child(mini(
                    ("merge-retry", i),
                    t,
                    Some(ov_icon(OvIcon::RotateCcw, 12., 2.2, t.text_primary).into_any_element()),
                    "Retry",
                    move |cx| {
                        if let Some(c) = retry.upgrade() {
                            c.update(cx, |c, cx| c.merge_retry(retry_id.clone(), cx));
                        }
                    },
                ))
                .child(mini(("merge-skip", i), t, None, "Skip", move |cx| {
                    if let Some(c) = skip.upgrade() {
                        c.update(cx, |c, cx| c.merge_skip(&skip_id, cx));
                    }
                }))
        });
        div()
            .max_w_full()
            .py(px(5.))
            .px(px(10.))
            .rounded(px(8.))
            .bg(if failed {
                rgba_mix(t.status_waiting, 0.08)
            } else {
                t.bg_sunken
            })
            .when(it.skipped, |d| d.opacity(0.5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .child(phase_icon(t, icon_phase, SPIN[i % SPIN.len()], cx))
                    .child(
                        div()
                            .max_w(px(280.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(it.title.clone()),
                    )
                    .children(actions)
                    .when(it.skipped, |d| {
                        d.child(
                            div()
                                .text_size(TEXT_XS)
                                .line_height(px(15.4))
                                .text_color(t.text_tertiary)
                                .child("Skipped"),
                        )
                    }),
            )
            .when_some(it.error.clone().filter(|_| blocking), |d, error| {
                d.child(
                    div()
                        .max_w(px(560.))
                        .mt(px(4.))
                        .ml(px(22.))
                        .text_size(TEXT_XS)
                        .line_height(px(15.4))
                        .text_color(t.status_waiting)
                        .child(error),
                )
            })
            .into_any_element()
    }

    /// MERGE-017 and the buttons.
    fn render_merge_foot(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let d = self.overlays.merge.as_ref().expect("open");
        let instruction = d.instruction.read(cx).value().to_string();
        let can_start = d.m.can_start(&instruction, d.starting);
        let note: AnyElement = if let Some(error) = &d.start_error {
            div()
                .text_color(t.status_waiting)
                .child(error.clone())
                .into_any_element()
        } else if d.m.blocking() > 0 {
            div()
                .child("Retry or skip the failed sessions to continue.")
                .into_any_element()
        } else if d.m.step == Step::Review {
            div()
                .flex()
                .items_center()
                .child(super::super::kbd(t, &keys("⌘")))
                .child(super::super::kbd(t, &keys("↩")))
                .child(div().ml(px(3.)).child("to start"))
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let (cancel, start) = (cx.entity().downgrade(), cx.entity().downgrade());
        let starting = d.starting;
        let start_button = button("merge-start")
            .primary()
            .label("Start session")
            .disabled(!can_start)
            .on_click(move |_, _, cx| {
                if let Some(c) = start.upgrade() {
                    c.update(cx, BoardCanvas::merge_begin);
                }
            });
        let start_button = if starting {
            start_button.child(spinner("merge-starting", 14., 2., t.text_on_accent, cx))
        } else {
            start_button.icon(Icon::Sparkles, 14., 2.2)
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(12.))
            .px(px(22.))
            .border_t(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.text_tertiary)
                    .child(note),
            )
            .child(button("merge-cancel").label("Cancel").on_click(move |_, _, cx| {
                if let Some(c) = cancel.upgrade() {
                    c.update(cx, BoardCanvas::merge_cancel);
                }
            }))
            .child(start_button)
            .into_any_element()
    }
}

/// `.merge-mini`: a ghost `.btn` 22px tall, 7px across, 11px, an icon then
/// the label.
fn mini(
    id: (&'static str, usize),
    t: &Tokens,
    icon: Option<AnyElement>,
    label: &'static str,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let (hover, pressed) = (t.bg_hover, t.bg_pressed);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(22.))
        .px(px(7.))
        .rounded(px(6.))
        .text_size(TEXT_XS)
        .line_height(px(15.4))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| on_click(cx))
        .children(icon)
        .child(label)
        .into_any_element()
}
