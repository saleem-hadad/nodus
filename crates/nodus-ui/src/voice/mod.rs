//! Speaking prompts in the window (`useVoice.ts`, `VoiceParts.tsx`,
//! `SessionMic.tsx`, `voice.css`; VOICE-033, VOICE-038…VOICE-045).
//!
//! - [`Mic`]: one mic, `useVoice`: `nodus_voice::controller::VoiceController`
//!   on the core's runtime, with the window's one listening slot (VOICE-025),
//!   Esc taken before anything else while it's busy (VOICE-033), and what was
//!   said handed to the caller on the main thread.
//! - [`voice_status`] and [`voice_notice`]: the recording look (red dot,
//!   clock, five-bar meter, or the spinner while transcribing) and the notice
//!   with its Settings / System Settings / Retry / Dismiss buttons. The prompt
//!   bar (spec 11) draws its mic with these.
//! - [`session_mic`]: the mic in an open Claude session's title bar, for the
//!   session window (`MicVariant::Window`) and the expanded card
//!   (`MicVariant::Card`); what's said is typed in, not sent (VOICE-041).

use std::f32::consts::TAU;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle,
    FontWeight, Global, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Transformation, Window, WindowId, div, px, radians,
};
use nodus_core::Nodus;
use nodus_voice::controller::{
    EscTarget, MICROPHONE_SETTINGS_LABEL, Phase, VoiceController, VoiceSlot, VoiceState, bar_scale, clock, halo_spread,
    session_mic_button,
};
use tokio::runtime::Handle;

use crate::actions::icons::{Glyph, filled_glyph, glyph};
use crate::components::frosted::frosted;
use crate::components::native_tooltip;
use crate::components::tween::{enter, transition};
use crate::parity;
use crate::theme::tokens::*;
use crate::theme::{self, fonts, motion};

/// The meter's bars, left to right, show these bands: the lowest in the middle.
pub const BAR_BANDS: [usize; 5] = [3, 1, 0, 2, 4];

/// The window's mics: the one listening slot they share (VOICE-025) and the
/// way a missing key opens Settings from the core's threads.
struct Mics {
    slot: VoiceSlot,
    settings: tokio::sync::mpsc::UnboundedSender<()>,
}

impl Global for Mics {}

fn mics(cx: &mut App) -> (VoiceSlot, tokio::sync::mpsc::UnboundedSender<()>) {
    if let Some(m) = cx.try_global::<Mics>() {
        return (m.slot.clone(), m.settings.clone());
    }
    let (settings, mut asks) = tokio::sync::mpsc::unbounded_channel::<()>();
    cx.spawn(async move |cx| {
        while asks.recv().await.is_some() {
            cx.update(open_settings);
        }
    })
    .detach();
    let slot = VoiceSlot::default();
    cx.set_global(Mics {
        slot: slot.clone(),
        settings: settings.clone(),
    });
    (slot, settings)
}

/// Opens the Settings dialog in the main window.
fn open_settings(cx: &mut App) {
    if let Some(shell) = crate::shell::window::main_shell(cx) {
        let state = shell.read(cx).state.clone();
        state.update(cx, crate::state::AppState::open_settings);
    }
}

/// What a mic does with what was said.
pub type OnText = Rc<dyn Fn(String, &mut App)>;

/// One mic (`useVoice`).
pub struct Mic {
    controller: Arc<VoiceController>,
    runtime: Handle,
    nodus: Nodus,
    /// What it shows.
    pub state: VoiceState,
    on_text: OnText,
    /// The caller's own text field, where Esc still cancels (VOICE-033).
    own_field: Option<FocusHandle>,
    window: WindowId,
    _watch: Task<()>,
    _texts: Task<()>,
    _escape: Subscription,
}

impl Mic {
    /// A mic for `project_id`'s prompts, in `window`.
    pub fn new(
        nodus: &Nodus,
        project_id: String,
        on_text: OnText,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let (slot, settings) = mics(cx);
        let project = project_id;
        let backend = nodus.voice().backend(
            Arc::new(move || Some(project.clone())),
            Arc::new(move || {
                let _ = settings.send(());
            }),
        );
        let (text_tx, mut text_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let controller = Arc::new(VoiceController::new(backend, slot, move |text| {
            let _ = text_tx.send(text);
        }));
        let mut rx = controller.subscribe();
        let watch = cx.spawn(async move |this, cx| {
            while rx.changed().await.is_ok() {
                let state = rx.borrow_and_update().clone();
                if this
                    .update(cx, |m, cx| {
                        m.state = state;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let texts = cx.spawn(async move |this, cx| {
            while let Some(text) = text_rx.recv().await {
                let _ = this.update(cx, |m, cx| {
                    let on_text = m.on_text.clone();
                    on_text(text, cx);
                });
            }
        });
        // VOICE-033: while busy, Esc is taken before anything else sees it.
        let weak = cx.weak_entity();
        let escape = cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.key != "escape" {
                return;
            }
            let Some(mic) = weak.upgrade() else { return };
            if mic.read(cx).window != window.window_handle().window_id() {
                return;
            }
            let focused = window.focused(cx);
            let own = mic.read(cx).own_field.clone();
            let target = if own.is_some() && focused == own {
                EscTarget::OwnField
            } else if event
                .context_stack
                .iter()
                .any(|c| c.contains("Input") || c.contains("Terminal"))
            {
                EscTarget::OtherField
            } else {
                EscTarget::Elsewhere
            };
            if mic.update(cx, |m, _| m.on_escape(target)) {
                cx.stop_propagation();
            }
        });
        let state = controller.state();
        Self {
            controller,
            // The controller's timers and tasks run on the core's runtime.
            runtime: crate::board::core_rt(cx).unwrap_or_else(Handle::current),
            nodus: nodus.clone(),
            state,
            on_text,
            own_field: None,
            window: window.window_handle().window_id(),
            _watch: watch,
            _texts: texts,
            _escape: escape,
        }
    }

    /// What the caller does with what was said, as of its latest render.
    pub fn set_on_text(&mut self, on_text: OnText) {
        self.on_text = on_text;
    }

    /// The caller's own text field (the prompt bar's), where Esc cancels too.
    pub fn set_own_field(&mut self, field: Option<FocusHandle>) {
        self.own_field = field;
    }

    pub fn phase(&self) -> Phase {
        self.state.phase
    }

    /// VOICE-026.
    pub fn record(&self) {
        let c = self.controller.clone();
        self.runtime.spawn(async move { c.record().await });
    }

    /// VOICE-028.
    pub fn stop(&self) {
        let c = self.controller.clone();
        self.runtime.spawn(async move { c.stop().await });
    }

    /// VOICE-031.
    pub fn retry(&self) {
        let c = self.controller.clone();
        self.runtime.spawn(async move { c.retry().await });
    }

    /// VOICE-030.
    pub fn cancel(&self) {
        let _runtime = self.runtime.enter();
        self.controller.cancel();
    }

    /// VOICE-032.
    pub fn dismiss(&self) {
        let _runtime = self.runtime.enter();
        self.controller.dismiss();
    }

    /// VOICE-033: Esc while busy; whether it was taken.
    pub fn on_escape(&self, target: EscTarget) -> bool {
        let _runtime = self.runtime.enter();
        self.controller.on_escape(target)
    }

    /// VOICE-027: the bars (in [`BAR_BANDS`] order) and the root level, read
    /// once per frame while recording.
    pub fn levels(&self) -> Option<([f64; 5], f64)> {
        self.controller.levels()
    }
}

/// VOICE-040: `Open System Settings` / `Open Settings`.
fn open_microphone_settings(nodus: Nodus, cx: &mut App) {
    cx.spawn(async move |_| nodus.voice().open_microphone_settings().await)
        .detach();
}

/// The meter bars' transition keys.
const BAR_NAMES: [&str; 5] = ["bar-0", "bar-1", "bar-2", "bar-3", "bar-4"];

impl Drop for Mic {
    /// VOICE-035: unmounting drops any answer and lets go of the microphone.
    fn drop(&mut self) {
        let _runtime = self.runtime.enter();
        self.controller.unmount();
    }
}

/// Lengths at a scale (an expanded card is drawn at the board's zoom).
#[derive(Debug, Clone, Copy)]
pub struct Z(pub f32);

impl Z {
    pub fn px(self, v: f32) -> Pixels {
        px(v * self.0)
    }
    pub fn f(self, v: f32) -> f32 {
        v * self.0
    }
}

/// VOICE-038, VOICE-039: the clock and meter while recording (the red dot
/// unless `hide_dot`), the spinner and `Transcribing…` while transcribing,
/// nothing otherwise. Asks for a frame every frame while recording.
pub fn voice_status(
    id: impl Into<ElementId>,
    mic: &Entity<Mic>,
    hide_dot: bool,
    z: Z,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let id = id.into();
    let t = theme::tokens(cx);
    let m = mic.read(cx);
    let row = || {
        div()
            .flex()
            .items_center()
            .gap(z.px(8.))
            .text_color(t.text_secondary)
            .text_size(z.px(12.))
            .line_height(z.px(16.8))
            .whitespace_nowrap()
    };
    match m.state.phase {
        Phase::Recording => {
            let levels = m.levels().map_or([0.0; 5], |(bars, _)| bars);
            let elapsed = m.state.elapsed;
            window.request_animation_frame();
            let frozen = parity::frozen(cx);
            let dot = (!hide_dot).then(|| {
                let dot = div().size(z.px(8.)).rounded_full().bg(t.status_waiting);
                if frozen {
                    dot.into_any_element()
                } else {
                    dot.with_animation(
                        (id.clone(), "rec-dot"),
                        Animation::new(Duration::from_millis(1200)).repeat(),
                        |d, p| {
                            // `voice-blink`: 0.3 at 50%, ease-in-out each way.
                            let half = if p < 0.5 { p * 2.0 } else { (1.0 - p) * 2.0 };
                            d.opacity(1.0 - 0.7 * motion::ease_in_out(half))
                        },
                    )
                    .into_any_element()
                }
            });
            let bars: Vec<AnyElement> = levels
                .iter()
                .enumerate()
                .map(|(i, level)| {
                    // `transform: scaleY(clamp(0.18, level × 1.4, 1))`, 70ms linear.
                    let scale = transition(
                        window,
                        cx,
                        (id.clone(), BAR_NAMES[i]),
                        bar_scale(*level) as f32,
                        Duration::from_millis(70),
                        |t| t,
                    );
                    div()
                        .w(z.px(3.))
                        .h(z.px(16. * scale))
                        .rounded(z.px(2.))
                        .bg(t.text_secondary)
                        .into_any_element()
                })
                .collect();
            Some(
                row()
                    .children(dot)
                    .child(
                        div()
                            .min_w(z.px(28.))
                            .text_color(t.text_primary)
                            .font_weight(FontWeight::MEDIUM)
                            .font_features(fonts::tabular())
                            .child(clock(elapsed)),
                    )
                    .child(div().flex().items_center().gap(z.px(2.)).h(z.px(16.)).children(bars))
                    .into_any_element(),
            )
        }
        Phase::Transcribing => Some(
            row()
                .child(spinner((id, "spin"), z.f(13.), 2., t.text_secondary, cx))
                .child("Transcribing…")
                .into_any_element(),
        ),
        Phase::Idle | Phase::Opening => None,
    }
}

/// `loader-circle` turning once per 0.9 s (`.spin`); still in parity shots.
pub fn spinner(id: impl Into<ElementId>, size: f32, stroke: f32, color: gpui::Rgba, cx: &App) -> AnyElement {
    let icon = glyph(Glyph::LoaderCircle, size, stroke, color);
    if parity::frozen(cx) {
        return icon.into_any_element();
    }
    icon.with_animation(id, Animation::new(Duration::from_millis(900)).repeat(), |svg, p| {
        svg.with_transformation(Transformation::rotate(radians(p * TAU)))
    })
    .into_any_element()
}

/// How a notice sits: the prompt bar's grows from its right edge's middle,
/// the session mic's hangs from its top right and is at most 80% of the
/// window wide (VOICE-045).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAnchor {
    Bar,
    SessionMic,
}

/// VOICE-040: what went wrong, and what can be done about it.
pub fn voice_notice(
    id: impl Into<ElementId>,
    mic: &Entity<Mic>,
    anchor: NoticeAnchor,
    z: Z,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let id: ElementId = id.into();
    let t = theme::tokens(cx);
    let notice = mic.read(cx).state.notice.clone()?;
    let viewport = window.viewport_size();
    let max_w = match anchor {
        NoticeAnchor::Bar => z.px(440.),
        NoticeAnchor::SessionMic => z.px(440.).min(viewport.width * 0.8),
    };
    // `voice-pill-in`: 220ms from opacity 0, 10px right, 96%.
    let key = SharedString::from(notice.message.clone());
    let p = enter(window, cx, (id.clone(), key), DUR_MED, motion::ease_out);
    let origin = match anchor {
        NoticeAnchor::Bar => (1.0, 0.5),
        NoticeAnchor::SessionMic => (1.0, 0.0),
    };
    let action = |name: &'static str, label: &'static str, icon: Option<Glyph>| {
        let hovered_bg = t.bg_hover;
        div()
            .id((id.clone(), name))
            .flex()
            .flex_none()
            .items_center()
            .gap(z.px(4.))
            .h(z.px(24.))
            .px(z.px(8.))
            .rounded(z.px(6.))
            .border(z.px(0.5))
            .border_color(t.border_strong)
            .bg(t.bg_elevated)
            .shadow(t.sm())
            .text_size(z.px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.text_primary)
            .whitespace_nowrap()
            .hover(move |s| s.bg(hovered_bg))
            .active(|s| s.bg(t.bg_pressed))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when_some(icon, |d, i| d.child(glyph(i, z.f(12.), 2., t.text_primary)))
            .child(label)
    };
    let settings = notice
        .settings
        .then(|| action("settings", "Settings", Some(Glyph::Settings)).on_click(|_, _, cx| open_settings(cx)));
    let microphone = notice.microphone.then(|| {
        let mic = mic.clone();
        action("microphone", MICROPHONE_SETTINGS_LABEL, None).on_click(move |_, _, cx| {
            let nodus = mic.read(cx).nodus.clone();
            open_microphone_settings(nodus, cx);
        })
    });
    let retry = notice.retry.then(|| {
        let mic = mic.clone();
        action("retry", "Retry", Some(Glyph::RotateCcw))
            .tooltip(native_tooltip("Send the same recording again".into()))
            .on_click(move |_, _, cx| mic.read(cx).retry())
    });
    let dismiss = {
        let mic = mic.clone();
        round_button((id.clone(), "dismiss"), "Dismiss", z, window, cx).on_click(move |_, _, cx| mic.read(cx).dismiss())
    };
    Some(
        div()
            .id(id.clone())
            .flex()
            .items_center()
            .gap(z.px(8.))
            .max_w(max_w)
            .min_h(z.px(36.))
            .pt(z.px(6.))
            .pr(z.px(6.))
            .pb(z.px(6.))
            .pl(z.px(12.))
            .rounded(z.px(18.))
            .map(|d| frosted(d, alpha_rgba(t.bg_elevated, 0.86), 20., 1.6))
            .shadow(t.md())
            .text_size(z.px(12.))
            .line_height(z.px(16.2))
            .transform_origin(origin.0, origin.1)
            .translate(z.px(10. * (1. - p)), px(0.))
            .scale(0.96 + 0.04 * p)
            .group_opacity(p)
            .occlude()
            .child(glyph(Glyph::CircleAlert, z.f(14.), 2., t.status_waiting))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(t.text_primary)
                    .child(notice.message.clone()),
            )
            .children(settings)
            .children(microphone)
            .children(retry)
            .child(dismiss)
            .into_any_element(),
    )
}

/// `--bg-elevated` at a share of its alpha, as an `Rgba`.
fn alpha_rgba(c: gpui::Rgba, share: f32) -> gpui::Rgba {
    gpui::Rgba { a: c.a * share, ..c }
}

/// `.voice-icon-btn`: a 24px round `x` (cancel, dismiss), tertiary until
/// pointed at.
fn round_button(
    id: impl Into<ElementId>,
    label: &'static str,
    z: Z,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let id: ElementId = id.into();
    let t = theme::tokens(cx);
    let tooltip = if label == "Cancel" { "Cancel (Esc)" } else { label };
    let hovered = window.use_keyed_state((id.clone(), "hover"), cx, |_, _| false);
    let on = *hovered.read(cx);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(z.px(24.))
        .rounded_full()
        .when(on, |d| d.bg(t.bg_hover))
        .on_hover(move |h, _, cx| {
            hovered.update(cx, |s, cx| {
                *s = *h;
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(native_tooltip(tooltip.into()))
        .child(glyph(
            Glyph::X,
            z.f(13.),
            2.,
            if on { t.text_primary } else { t.text_tertiary },
        ))
}

/// Where a session mic sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicVariant {
    /// The session window's title bar: a ghost icon button (`.btn-icon`).
    Window,
    /// An expanded card's header: a `.card-tool`, drawn at the board's zoom.
    Card,
}

/// What a session mic needs from the title bar it's in.
#[derive(Clone)]
pub struct SessionMicProps {
    /// The session's project: its name, branch and files hint xAI.
    pub project_id: String,
    pub variant: MicVariant,
    /// 1 in the session window; the board's zoom on an expanded card.
    pub scale: f32,
    /// Types what was said at the session's prompt, without sending it
    /// (12's `registry.typeIn`, which resumes a stopped session).
    pub on_text: OnText,
}

/// VOICE-041: the mic in an open Claude session's title bar. `id` names it
/// (one per session and variant, e.g. `("session-mic", session_id)`); its
/// recording lives while the element is drawn and stops when it isn't
/// (VOICE-035).
pub fn session_mic(id: impl Into<ElementId>, props: SessionMicProps, window: &mut Window, cx: &mut App) -> AnyElement {
    let id: ElementId = id.into();
    let nodus = cx.global::<crate::shell::window::Core>().0.clone();
    let on_text = props.on_text.clone();
    let project = props.project_id.clone();
    let mic = window.use_keyed_state((id.clone(), "mic"), cx, move |window, cx| {
        cx.new(|cx| Mic::new(&nodus, project, on_text, window, cx))
    });
    let mic = mic.read(cx).clone();
    mic.update(cx, |m, _| m.set_on_text(props.on_text.clone()));
    let view = window.use_keyed_state((id.clone(), "view"), cx, |_, cx| {
        cx.new(|_| SessionMic {
            id: id.clone(),
            mic: mic.clone(),
            props: props.clone(),
        })
    });
    let view = view.read(cx).clone();
    view.update(cx, |v, _| v.props = props);
    view.into_any_element()
}

/// `SessionMic.tsx`.
pub struct SessionMic {
    id: ElementId,
    mic: Entity<Mic>,
    props: SessionMicProps,
}

impl Render for SessionMic {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let z = Z(match self.props.variant {
            MicVariant::Window => 1.0,
            MicVariant::Card => self.props.scale,
        });
        let phase = self.mic.read(cx).phase();
        let listening = phase == Phase::Recording;
        let (title, _label, enabled) = session_mic_button(phase);
        let level = self.mic.read(cx).levels().map_or(0.0, |(_, root)| root);
        let id = self.id.clone();

        // VOICE-042: while recording, the status (its red dot hidden) and a
        // round cancel button come first.
        let status = listening
            .then(|| voice_status((id.clone(), "status"), &self.mic, true, z, window, cx))
            .flatten();
        let cancel = listening.then(|| {
            let mic = self.mic.clone();
            round_button((id.clone(), "cancel"), "Cancel", z, window, cx)
                .on_click(move |_, _, cx| mic.read(cx).cancel())
        });

        // VOICE-043, VOICE-044: the button; red and round with a halo while
        // recording, whose spread follows the voice.
        let hovered = window.use_keyed_state((id.clone(), "hover"), cx, |_, _| false);
        let hover = *hovered.read(cx) && enabled;
        let spread = transition(
            window,
            cx,
            (id.clone(), "halo"),
            if listening { halo_spread(level) as f32 } else { 0. },
            Duration::from_millis(80),
            |t| t,
        );
        let (bg, fg) = if listening {
            (Some(t.status_waiting), hard::WHITE)
        } else {
            let rest = match self.props.variant {
                MicVariant::Window => t.text_primary,
                MicVariant::Card => t.text_secondary,
            };
            (hover.then_some(t.bg_hover), if hover { t.text_primary } else { rest })
        };
        let icon: AnyElement = match phase {
            Phase::Recording => filled_glyph(Glyph::Square, z.f(9.), fg).into_any_element(),
            Phase::Transcribing => spinner((id.clone(), "spin"), z.f(14.), 2., fg, cx),
            Phase::Idle | Phase::Opening => glyph(Glyph::Mic, z.f(14.), 2., fg).into_any_element(),
        };
        let mic = self.mic.clone();
        let button = div()
            .id((id.clone(), "button"))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(z.px(28.))
            .map(|d| match self.props.variant {
                MicVariant::Window => d.w(z.px(28.)),
                MicVariant::Card => d.min_w(z.px(28.)),
            })
            .map(|d| {
                if listening {
                    d.rounded_full()
                } else {
                    d.rounded(z.px(6.))
                }
            })
            .when_some(bg, Styled::bg)
            .when(listening, |d| {
                d.shadow(vec![ring(z.f(spread), rgba_share(t.status_waiting, 0.2))])
            })
            // `.btn:disabled` dims; a disabled `.card-tool` looks the same.
            .when(!enabled && self.props.variant == MicVariant::Window, |d| {
                d.opacity(0.45)
            })
            .when(enabled && !listening, |d| d.active(|s| s.bg(t.bg_pressed)))
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .tooltip(native_tooltip(title.into()))
            .when(enabled, |d| {
                d.on_click(move |_, _, cx| {
                    let m = mic.read(cx);
                    if m.phase() == Phase::Recording {
                        m.stop();
                    } else {
                        m.record();
                    }
                })
            })
            .child(icon);

        // VOICE-045: the notice hangs 8px below, right edges lined up.
        let notice = voice_notice(
            (id.clone(), "notice"),
            &self.mic,
            NoticeAnchor::SessionMic,
            z,
            window,
            cx,
        )
        .map(|n| {
            // Over the terminal under the title bar (z-index 20).
            gpui::deferred(div().absolute().top(z.px(28. + 8.)).right_0().child(n)).with_priority(20)
        });

        div()
            .relative()
            .flex()
            .items_center()
            .gap(z.px(6.))
            .font_family(fonts::UI)
            .children(status)
            .children(cancel)
            .child(button)
            .children(notice)
    }
}

/// A colour at a share of its alpha.
fn rgba_share(c: gpui::Rgba, share: f32) -> gpui::Rgba {
    gpui::Rgba { a: c.a * share, ..c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_the_bands_out_lowest_in_the_middle() {
        // VOICE-027, voice.css: bars left to right show bands 3, 1, 0, 2, 4.
        assert_eq!(BAR_BANDS, [3, 1, 0, 2, 4]);
        assert_eq!(Z(0.5).px(28.), px(14.));
    }
}
