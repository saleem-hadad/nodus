//! Drawing the bar (`sidekick.css`): the column bottom centre, what's
//! stacked above the row, the pill with its token, field, hint and send
//! button, and the mic beside it (BAR-001…BAR-008, BAR-013, BAR-019,
//! BAR-035).

use gpui::component::Sizable as _;
use gpui::component::input::Textarea;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, CursorStyle, Focusable as _, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Rgba, StatefulInteractiveElement, Styled, Window, div, linear_color_stop, linear_gradient, px,
};
use nodus_board::compose::Target;

use super::super::icons::{OvIcon, ov_icon};
use super::super::{kbd, mix_layers, spinner};
use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::folder_icon::folder_hues;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::{enter, transition};
use crate::theme::tokens::{DUR_FAST, DUR_MED, TEXT_SM, Tokens, inset_shadow, rgba};
use crate::theme::{self, motion};

/// `--view-hue` and `--view-ink` (the bar only).
pub(crate) fn view_hue(t: &Tokens) -> (Rgba, Rgba) {
    if t.dark {
        (rgba(191, 90, 242, 1.), rgba(217, 163, 247, 1.))
    } else {
        (rgba(175, 82, 222, 1.), rgba(137, 68, 171, 1.))
    }
}

/// A note colour's front gradient (`FOLDER_HUES`), top to bottom.
pub(crate) fn note_gradient(color: nodus_domain::FolderColor) -> gpui::Background {
    let hues = folder_hues(color);
    linear_gradient(
        180.,
        linear_color_stop(hues.front_top, 0.),
        linear_color_stop(hues.front_bottom, 1.),
    )
}

impl BoardCanvas {
    /// BAR-001, BAR-002: the column, bottom centre of the board, with the
    /// toast and the selection bar above the row.
    pub(crate) fn render_bar(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let field = self.bar_field(window, cx);
        // The field's focus, as the frame has it (its events may not fire
        // for a focus given from outside).
        let focused = field.read(cx).focus_handle(cx).is_focused(window);
        if focused != self.overlays.bar.focused {
            self.overlays.bar.focused = focused;
            if !focused {
                // BAR-016: blurring resets the menu's dismissal.
                self.overlays.bar.menu_dismissed = false;
            }
        }
        self.bar_follow_into();
        self.bar_take_focus_request(window, cx);
        self.bar_mic(window, cx);
        self.overlays.bar.voice_busy = self.bar_phase(cx) != nodus_voice::controller::Phase::Idle;
        self.bar_take_heard(window, cx);
        if std::mem::take(&mut self.overlays.bar.clear_pending) {
            self.bar_set_text(String::new(), window, cx);
        }
        let placeholder = self.bar_placeholder();
        if self.overlays.bar.placeholder != placeholder {
            self.overlays.bar.placeholder = placeholder.clone();
            field.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        let (w, _) = self.canvas_size();
        let col_w = (w as f32 - 416.).clamp(300., 672.).max(300.);
        let above = self.render_above(col_w, window, cx);
        let row = self.render_row(col_w, window, cx);
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(20.))
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(col_w))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .children(above)
                    .child(row),
            )
            .into_any_element()
    }

    /// BAR-006.
    fn bar_placeholder(&self) -> String {
        super::text::placeholder(
            self.overlays.bar.target,
            self.bar_sends_into().is_some(),
            self.bar_context_count(),
        )
    }

    /// BAR-003: the pill and the mic, bottom-aligned.
    fn render_row(&mut self, col_w: f32, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let open = self.bar_open();
        // The pill's preferred width glides 520 ↔ 600; it shrinks, never grows, to fit.
        let basis = transition(
            window,
            cx,
            "bar-basis",
            if open { 600f32 } else { 520. },
            DUR_MED,
            motion::ease_out,
        );
        let lift = transition(
            window,
            cx,
            "bar-shadow",
            if open { 1f32 } else { 0. },
            DUR_MED,
            motion::ease,
        );
        let pill_w = basis.min(col_w - 62.);
        let mut shadow = mix_layers(&t.shadow_md, &t.shadow_lg, lift);
        // BAR-036: a 1.5px red ring while recording.
        if self.bar_phase(cx) == nodus_voice::controller::Phase::Recording {
            shadow.push(theme::tokens::ring(1.5, rgba_mix(t.status_waiting, 0.55)));
        }
        let pill = self.render_pill(pill_w, shadow, window, cx);
        div()
            .w_full()
            .flex()
            .items_end()
            .justify_center()
            .gap(px(10.))
            .child(pill)
            .child(self.render_orb(&t, window, cx))
            .into_any_element()
    }

    fn render_pill(
        &mut self,
        width: f32,
        shadow: Vec<gpui::BoxShadow>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = theme::tokens(cx);
        let field = self.overlays.bar.field.clone().expect("drawn after bar_field");
        let target = self.overlays.bar.target;
        let into = self.bar_sends_into();
        let has_token = target.is_some() || into.is_some();
        let text_empty = self.overlays.bar.text.is_empty();
        let listening = self.bar_listening_in(cx);
        let idle = self.bar_phase(cx) == nodus_voice::controller::Phase::Idle;
        let hint = text_empty && !has_token && self.bar_context_count() == 0 && !listening;
        let can_send = !crate::board::overlays::trimmed_empty(&self.overlays.bar.text)
            && idle
            && !self.overlays.bar.sending
            && self.bar_commands().is_empty();
        let token = self.render_token(&t, window, cx);
        let popover = self.render_bar_popover(&t, window, cx);
        let me = cx.entity().downgrade();
        let label = super::text::send_label(target, into.as_ref().map(|i| i.title.as_str()));
        let send = self.render_send(&t, can_send, label, window, cx);
        div()
            .id("bar-pill")
            .relative()
            .flex_none()
            .w(px(width))
            .min_h(px(48.))
            .flex()
            .items_end()
            .gap(px(8.))
            .pt(px(7.))
            .pb(px(7.))
            .pr(px(7.))
            .pl(px(if has_token { 8. } else { 18. }))
            .rounded(px(24.))
            .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.9), 24., 1.8))
            .shadow(shadow)
            .cursor(CursorStyle::IBeam)
            .occlude()
            // BAR-008: the pill's background is the field's too.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                if let Some(c) = me.upgrade() {
                    c.update(cx, |c, cx| c.bar_focus_field(window, cx));
                }
            })
            .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                if this.bar_key_down(e, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .children(popover)
            .children(token)
            .when(listening, |d| d.child(self.render_bar_status(&t, window, cx)))
            .child(
                // Hidden while listening, keeping its value (BAR-037).
                // XSmall's editor insets are 4px across and none down: taken
                // back, so the text sits where 0.9.4's textarea puts it; and
                // the 10px it keeps clear on the right, so it wraps there too.
                div()
                    .flex_1()
                    .min_w_0()
                    .ml(px(-4.))
                    .mr(px(-14.))
                    .when(listening, |d| d.hidden())
                    .child(
                        Textarea::new(&field)
                            .appearance(false)
                            .xsmall()
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .py(px(7.))
                            .px(px(0.)),
                    ),
            )
            .when(hint, |d| {
                d.child(
                    div()
                        .flex_none()
                        .self_center()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.text_tertiary)
                        .whitespace_nowrap()
                        .child(kbd(&t, "/"))
                        .child("view or note"),
                )
            })
            .child(send)
            .into_any_element()
    }

    /// BAR-012, BAR-013: the send button.
    fn render_send(
        &mut self,
        t: &Tokens,
        enabled: bool,
        label: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let hovered = window.use_keyed_state("bar-send-hover", cx, |_, _| false);
        let hover = *hovered.read(cx) && enabled;
        let target_bg = match (enabled, hover) {
            (false, _) => t.bg_pressed,
            (true, true) => t.accent_hover,
            (true, false) => t.accent,
        };
        let bg = transition(window, cx, "bar-send-bg", target_bg, DUR_FAST, motion::ease);
        let fg = if enabled { t.text_on_accent } else { t.text_tertiary };
        let glyph = if self.overlays.bar.sending {
            spinner("bar-send-spin", 15., 2., fg, cx)
        } else {
            ov_icon(OvIcon::ArrowUp, 16., 2.4, fg).into_any_element()
        };
        let me = cx.entity().downgrade();
        div()
            .id("bar-send")
            .flex_none()
            .size(px(34.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(bg)
            .cursor(CursorStyle::Arrow)
            .tooltip(native_tooltip(format!("{label} (Return)").into()))
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                })
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
            })
            .when(enabled, |d| {
                d.on_click(move |_, _, cx| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, |c, cx| c.bar_submit(cx));
                    }
                })
            })
            .child(glyph)
            .into_any_element()
    }

    /// BAR-019: the view, note or session token at the pill's start.
    fn render_token(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let target = self.overlays.bar.target;
        let into = self.bar_sends_into();
        let key = match (target, &into) {
            (Some(Target::View), _) => "view".to_owned(),
            (Some(Target::Note), _) => "note".to_owned(),
            (None, Some(i)) => format!("into-{}", i.session_id),
            (None, None) => return None,
        };
        let pop = enter(
            window,
            cx,
            gpui::ElementId::Name(format!("bar-token-in-{key}").into()),
            DUR_FAST,
            motion::ease_out,
        );
        let me = cx.entity().downgrade();
        let (bg, fg, lead, label, clear_label): (Rgba, Rgba, AnyElement, String, &str) = match (target, into) {
            (Some(Target::View), _) => {
                let (hue, ink) = view_hue(t);
                (
                    Rgba { a: 0.15, ..hue },
                    ink,
                    icon(Icon::LayoutDashboard, 13., 2., ink).into_any_element(),
                    "View".into(),
                    "Start a session instead",
                )
            }
            (Some(Target::Note), _) => (
                t.bg_hover,
                t.text_primary,
                self.render_note_swatch(cx),
                "Note".into(),
                "Start a session instead",
            ),
            (None, Some(i)) => (
                t.bg_selected,
                t.folder_label_selected,
                ov_icon(OvIcon::CornerDownRight, 13., 2., t.folder_label_selected).into_any_element(),
                i.title,
                "Start a new session instead",
            ),
            (None, None) => return None,
        };
        let is_into = target.is_none();
        let clear_hover = window.use_keyed_state("bar-token-clear-hover", cx, |_, _| false);
        let clear_hovered = *clear_hover.read(cx);
        let clear = div()
            .id("bar-token-clear")
            .flex_none()
            .size(px(22.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(clear_hovered, |d| d.bg(t.bg_hover))
            .opacity(if clear_hovered { 1. } else { 0.7 })
            .tooltip(native_tooltip(clear_label.into()))
            .on_hover(move |h, _, cx| {
                clear_hover.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                })
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                if let Some(c) = me.upgrade() {
                    c.update(cx, |c, cx| {
                        if is_into {
                            c.bar_drop_into(cx)
                        } else {
                            c.bar_clear_target(cx)
                        }
                    });
                }
            })
            .child(icon(Icon::X, 11., 2.6, fg));
        Some(
            div()
                .id("bar-token")
                .flex_none()
                .max_w(px(240.))
                .h(px(28.))
                .mb(px(3.))
                .flex()
                .items_center()
                .gap(px(6.))
                .pr(px(3.))
                .pl(px(if target == Some(Target::Note) { 5. } else { 10. }))
                .rounded(px(14.))
                .bg(bg)
                .text_color(fg)
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .font_weight(gpui::FontWeight::MEDIUM)
                .whitespace_nowrap()
                .cursor(CursorStyle::Arrow)
                .opacity(pop)
                .scale(0.9 + 0.1 * pop)
                .when(is_into, |d| {
                    let title = format!("Sent into “{label}”");
                    d.tooltip(native_tooltip(title.into()))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(lead)
                .child(div().min_w_0().overflow_hidden().text_ellipsis().child(label))
                .child(clear)
                .into_any_element(),
        )
    }

    /// BAR-022: the note token's swatch, which toggles the colour picker.
    fn render_note_swatch(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let color = self.overlays.bar.color;
        let me = cx.entity().downgrade();
        div()
            .id("bar-note-swatch")
            .flex_none()
            .size(px(18.))
            .rounded(px(5.))
            .bg(note_gradient(color))
            .shadow(vec![inset_shadow(0., 0., 0., 0.5, rgba(0, 0, 0, 0.2))])
            .relative()
            .child(crate::board::overlays::bounds_probe(cx.entity().downgrade(), |c, b| {
                c.overlays.bar.picker_area[1] = Some(b)
            }))
            .tooltip(native_tooltip("Note colour".into()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                if let Some(c) = me.upgrade() {
                    c.update(cx, |c, cx| {
                        c.overlays.bar.picking = !c.overlays.bar.picking;
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }
}
