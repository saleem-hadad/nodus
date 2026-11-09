//! The bar's mic (spec 11 BAR-034…BAR-040, voice internals spec 05): the
//! shared `voice::Mic` with the window's one listening slot, the round mic
//! beside the pill, the status line in the field's place while listening,
//! and the voice notice above the row.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, CursorStyle, Entity, Focusable as _, InteractiveElement, IntoElement,
    MouseButton, ParentElement, StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_voice::controller::Phase;

use super::super::icons::{OvIcon, ov_icon};
use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::motion;
use crate::theme::tokens::{DUR_FAST, Tokens, hard, ring, shadows};
use crate::voice::{Mic, NoticeAnchor, Z, spinner, voice_notice, voice_status};

impl BoardCanvas {
    /// The bar's mic, made the first time the bar is drawn.
    pub(crate) fn bar_mic(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Entity<Mic> {
        if let Some(mic) = &self.overlays.bar.mic {
            return mic.clone();
        }
        let nodus = self.state.read(cx).nodus.clone();
        let project_id = self.project_id.clone();
        let me = cx.entity().downgrade();
        let mic = cx.new(|cx| {
            Mic::new(
                &nodus,
                project_id,
                std::rc::Rc::new(move |said: String, cx: &mut gpui::App| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, |c, cx| {
                            c.overlays.bar.heard.push(said);
                            cx.notify();
                        });
                    }
                }),
                window,
                cx,
            )
        });
        // Esc in the bar's own field cancels too (VOICE-033).
        if let Some(field) = &self.overlays.bar.field {
            let handle = field.read(cx).focus_handle(cx);
            mic.update(cx, |m, _| m.set_own_field(Some(handle)));
        }
        cx.observe(&mic, |_, _, cx| cx.notify()).detach();
        self.overlays.bar.mic = Some(mic.clone());
        mic
    }

    /// The voice phase (idle until the mic exists).
    pub(crate) fn bar_phase(&self, cx: &gpui::App) -> Phase {
        self.overlays
            .bar
            .mic
            .as_ref()
            .map_or(Phase::Idle, |m| m.read(cx).phase())
    }

    /// BAR-039: what was said lands in the field, after what's there, and
    /// the keyboard comes back to it with the caret at the end.
    pub(crate) fn bar_take_heard(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.bar.heard.is_empty() || self.bar_listening_in(cx) {
            return;
        }
        for said in std::mem::take(&mut self.overlays.bar.heard) {
            let text = &self.overlays.bar.text;
            let next = if nodus_domain::js::trim(text).is_empty() {
                said
            } else {
                format!("{} {said}", nodus_domain::js::trim_end(text))
            };
            self.bar_set_text(next, window, cx);
        }
        self.bar_focus_field(window, cx);
    }

    /// Recording or transcribing.
    pub(crate) fn bar_listening_in(&self, cx: &gpui::App) -> bool {
        matches!(self.bar_phase(cx), Phase::Recording | Phase::Transcribing)
    }

    /// BAR-034 (BOARD-166): the board's V: record, or stop and transcribe.
    pub(crate) fn bar_toggle_voice(&mut self, cx: &mut Context<'_, Self>) {
        let Some(mic) = &self.overlays.bar.mic else { return };
        let m = mic.read(cx);
        match m.phase() {
            Phase::Recording => m.stop(),
            Phase::Idle => m.record(),
            _ => {}
        }
    }

    /// BAR-037: the status line in the field's place while listening.
    pub(crate) fn render_bar_status(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let mic = self.overlays.bar.mic.clone().expect("listening has a mic");
        let status = voice_status("bar-voice-status", &mic, false, Z(1.), window, cx);
        let hover = window.use_keyed_state("bar-cancel-hover", cx, |_, _| false);
        let hovered = *hover.read(cx);
        let cancel_mic = mic.clone();
        div()
            .flex_1()
            .min_w_0()
            .h(px(34.))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor(CursorStyle::Arrow)
            .children(status)
            .child(div().flex_1())
            .child(
                div()
                    .id("bar-voice-cancel")
                    .flex_none()
                    .size(px(24.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(hovered, |d| d.bg(t.bg_hover))
                    .tooltip(native_tooltip("Cancel (Esc)".into()))
                    .on_hover(move |h, _, cx| {
                        hover.update(cx, |s, cx| {
                            *s = *h;
                            cx.notify();
                        });
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, _, cx| cancel_mic.read(cx).cancel())
                    .child(icon(
                        Icon::X,
                        13.,
                        2.,
                        if hovered { t.text_primary } else { t.text_tertiary },
                    )),
            )
            .into_any_element()
    }

    /// BAR-040: the voice notice, directly above the row.
    pub(crate) fn render_bar_notice(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let mic = self.overlays.bar.mic.clone()?;
        voice_notice("bar-voice-notice", &mic, NoticeAnchor::Bar, Z(1.), window, cx)
    }

    /// BAR-035, BAR-036: the mic beside the bar.
    pub(crate) fn render_orb(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let mic = self.bar_mic(window, cx);
        let phase = mic.read(cx).phase();
        let recording = phase == Phase::Recording;
        let enabled = !matches!(phase, Phase::Opening | Phase::Transcribing);
        // The loudest of the five bands.
        let level = mic
            .read(cx)
            .levels()
            .map_or(0.0, |(bars, _)| bars.iter().copied().fold(0.0, f64::max)) as f32;
        if recording {
            window.request_animation_frame();
        }
        let spread = transition(
            window,
            cx,
            "bar-orb-halo",
            if recording { 3.6 + level * 14.4 } else { 0. },
            std::time::Duration::from_millis(80),
            |t| t,
        );
        let hovered = window.use_keyed_state("bar-orb-hover", cx, |_, _| false);
        let hover = *hovered.read(cx) && enabled;
        let rest = if hover { t.text_primary } else { t.text_secondary };
        let fg = transition(
            window,
            cx,
            "bar-orb-fg",
            if recording { hard::WHITE } else { rest },
            DUR_FAST,
            motion::ease,
        );
        let label = super::text::orb_label(phase);
        let glyph: AnyElement = match phase {
            // A filled `Square` 16: its 18-unit rect, radius 2.
            Phase::Recording => div().size(px(12.)).rounded(px(1.33)).bg(fg).into_any_element(),
            Phase::Transcribing => spinner("bar-orb-spin", 22., 2., fg, cx),
            _ => ov_icon(OvIcon::Mic, 22., 2., fg).into_any_element(),
        };
        let mut halo = shadows(&t.shadow_md);
        if recording {
            halo.push(ring(spread, rgba_mix(t.status_waiting, 0.2)));
        }
        let press = mic.clone();
        div()
            .id("bar-orb")
            .flex_none()
            .size(px(52.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .map(|d| {
                if recording {
                    d.bg(t.status_waiting)
                } else {
                    frosted(d, rgba_mix(t.bg_elevated, 0.9), 24., 1.8)
                }
            })
            .shadow(halo)
            .occlude()
            .tooltip(native_tooltip(label.into()))
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
            })
            .when(enabled, |d| {
                d.active(|s| s.opacity(1.)).on_click(move |_, _, cx| {
                    let m = press.read(cx);
                    if m.phase() == Phase::Recording {
                        m.stop();
                    } else {
                        m.record();
                    }
                })
            })
            .child(glyph)
            .into_any_element()
    }
}
