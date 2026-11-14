//! The composer under the chat: the prompts waiting in the queue, the field,
//! and the buttons that send, queue or stop (CHAT-020…CHAT-023, CHAT-049).

use std::time::Duration;

use gpui::component::Sizable as _;
use gpui::component::input::{IndentInline, MoveDown, MoveUp, Textarea};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_domain::SessionStatus;

use super::look;
use super::view::ChatView;
use crate::board::overlays::icons::{OvIcon, ov_icon};
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::{DUR_FAST, Tokens, rgba, shadow};
use crate::theme::{self, ai, motion};

/// The composer's corners.
const RADIUS: f32 = 22.;

/// The widest the composer gets in the session window, as the column above.
const COMPOSER_W: f32 = 780.;

/// Which queued prompts have `Send now` (CHAT-023, CHAT-049).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SendNow {
    /// None: the session isn't running, or Claude is asking something.
    Never,
    /// The first: Claude is idle, interrupted before its turn ended.
    First,
    /// Each: Claude works, and takes one at its next step.
    Any,
}

impl ChatView {
    /// CHAT-020: Return sends (or queues while Claude works), ⌘Return sends
    /// at once, Esc stops Claude; ⇧Return is a new line. CHAT-045: Esc
    /// closes the `/` menu first.
    fn composer_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let k = &e.keystroke;
        let m = k.modifiers;
        let busy = self.card(cx).is_some_and(|c| c.live && c.status != SessionStatus::Done);
        match k.key.as_str() {
            // The field sends on Return itself (InputEvent::PressEnter); it
            // goes no further.
            "enter" if !m.shift && !m.alt => cx.stop_propagation(),
            // MODEL-032: Esc closes the model menu first.
            "escape" if self.model_open => {
                cx.stop_propagation();
                self.model_open = false;
                cx.notify();
            }
            "escape" if self.slash_dismiss(window, cx) => cx.stop_propagation(),
            "escape" if busy => {
                cx.stop_propagation();
                self.interrupt(cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_composer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let card = self.card(cx);
        let live = card.as_ref().is_some_and(|c| c.live);
        let status = card.as_ref().map(|c| c.status);
        let busy = live && matches!(status, Some(SessionStatus::Working | SessionStatus::Waiting));
        let placeholder = if !live {
            "Message Claude to resume this session…"
        } else if busy {
            "Queue a message for when Claude is done…"
        } else {
            "Message Claude…"
        };
        self.composer
            .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        // MOTION-043: while Claude works, a faint light circles the field.
        let working = live && status == Some(SessionStatus::Working);
        // CHAT-049: a queued prompt can go now while Claude works; not while
        // it asks something, which the words would answer.
        let send_now = if !live {
            SendNow::Never
        } else if !busy {
            SendNow::First
        } else if working && !self.dialog {
            SendNow::Any
        } else {
            SendNow::Never
        };
        let queue = self.render_queue(&t, busy, send_now, cx);
        let error = self.error.clone().map(|e| {
            div()
                .text_size(self.px(12.))
                .text_color(t.status_waiting)
                .child(SharedString::from(e))
        });
        let menu = self.render_slash_menu(&t, window, cx);
        let menu_open = menu.is_some();
        let mic = self.mic(window, cx);
        let send = self.send_button(&t, busy, window, cx);
        let stop = busy.then(|| self.stop_button(&t, cx));
        let field = Textarea::new(&self.composer)
            .appearance(false)
            .xsmall()
            .text_size(self.px(14.))
            .line_height(self.px(20.))
            .py(self.px(6.))
            .px(self.px(0.));
        // CHAT-041: the field lifts off the chat, and a little more while
        // it has the keyboard.
        let focused = self.composer_handle(cx).is_focused(window);
        let id = cx.entity_id();
        let focus = transition(
            window,
            cx,
            ("chat-focus", id),
            if focused { 1f32 } else { 0. },
            Duration::from_millis(160),
            motion::ease_out,
        );
        let mut shadows = t.md();
        if focus > 0.01 {
            shadows.push(shadow(
                0.,
                6. * self.scale,
                22. * self.scale,
                0.,
                rgba(0, 0, 0, focus * if t.dark { 0.3 } else { 0.07 }),
            ));
        }
        let lit = transition(
            window,
            cx,
            ("chat-lit", id),
            if working { 1f32 } else { 0. },
            Duration::from_millis(600),
            motion::ease_out,
        );
        let light = if lit > 0.01 {
            look::light(RADIUS * self.scale, lit, t.text_tertiary, window, cx)
        } else {
            None
        };
        let pill = div()
            .relative()
            .flex()
            .items_end()
            .gap(self.px(6.))
            .pl(self.px(16.))
            .pr(self.px(7.))
            .py(self.px(7.))
            .rounded(self.px(RADIUS))
            .border_1()
            .border_color(ai::lerp(t.border, t.border_strong, focus))
            .bg(look::field_bg(&t))
            .shadow(shadows)
            .children(light)
            .child(div().flex_1().min_w_0().child(field))
            .child(mic)
            .children(stop)
            .child(send)
            .children(menu);
        let side = if self.in_card { 10. } else { 24. };
        div()
            .flex_none()
            .w_full()
            .flex()
            .justify_center()
            .px(self.px(side))
            .pt(self.px(4.))
            .pb(self.px(if self.in_card { 10. } else { 18. }))
            .on_key_down(cx.listener(Self::composer_key))
            // CHAT-045: ↑, ↓ and Tab are the field's own bindings, which run
            // before any key handler, so the `/` menu takes them as actions.
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.slash_move(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.slash_move(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.slash_complete(None, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .w_full()
                    .max_w(self.px(COMPOSER_W))
                    .flex()
                    .flex_col()
                    .gap(self.px(8.))
                    .children(queue)
                    .children(error)
                    .child(pill)
                    .child(self.hint(&t, live, status, menu_open, window, cx)),
            )
            .into_any_element()
    }

    /// CHAT-021, CHAT-023, CHAT-049: the prompts waiting for the turn to end,
    /// under those sent while Claude works, until it takes them (CHAT-052).
    fn render_queue(
        &mut self,
        t: &Tokens,
        busy: bool,
        send_now: SendNow,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let items = self.queue(cx);
        if items.is_empty() && self.handed.is_empty() {
            return None;
        }
        let me = cx.entity().downgrade();
        let tile = |t: &Tokens| {
            div()
                .flex_none()
                .size(self.px(18.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.bg_hover)
                .text_color(t.text_secondary)
                .text_size(self.px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
        };
        let words = |text: &str| {
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(text.replace('\n', " ")))
        };
        let row = || {
            div()
                .flex()
                .items_center()
                .gap(self.px(8.))
                .px(self.px(6.))
                .py(self.px(4.))
                .rounded(self.px(8.))
        };
        // With prompts queued under them, they say how they differ; alone,
        // the box's head says it.
        let mixed = !items.is_empty();
        let handed: Vec<AnyElement> = self
            .handed
            .iter()
            .map(|h| {
                row()
                    .child(tile(t).child(ov_icon(OvIcon::ArrowUp, 10. * self.scale, 2.4, t.text_secondary)))
                    .child(words(&h.text))
                    .when(mixed, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .whitespace_nowrap()
                                .px(self.px(5.))
                                .text_color(t.text_tertiary)
                                .child("Sent · at Claude’s next step"),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(n, item)| {
                let (edit_me, remove_me, now_me) = (me.clone(), me.clone(), me.clone());
                let (edit_id, remove_id, now_id) = (item.id.clone(), item.id.clone(), item.id.clone());
                let action = |id: String, label: &'static str| {
                    div()
                        .id(SharedString::from(id))
                        .flex_none()
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .px(self.px(5.))
                        .rounded(self.px(4.))
                        .text_color(t.text_tertiary)
                        .hover(|d| d.text_color(t.text_primary).bg(t.bg_hover))
                        .child(label)
                };
                row()
                    .child(tile(t).child(SharedString::from((n + 1).to_string())))
                    .child(words(&item.text))
                    .when(
                        send_now == SendNow::Any || (n == 0 && send_now == SendNow::First),
                        |d| {
                            let tip = if busy {
                                "Send it now: Claude reads it at its next step"
                            } else {
                                "Send it now"
                            };
                            d.child(
                                action(format!("chat-q-now-{now_id}"), "Send now")
                                    .tooltip(native_tooltip(tip.into()))
                                    .on_click(move |_, _, cx| {
                                        let _ = now_me.update(cx, |this, cx| this.send_queued_now(&now_id, cx));
                                    }),
                            )
                        },
                    )
                    .child(
                        action(format!("chat-q-edit-{edit_id}"), "Edit").on_click(move |_, window, cx| {
                            let _ = edit_me.update(cx, |this, cx| this.edit_queued(&edit_id, window, cx));
                        }),
                    )
                    .child(
                        action(format!("chat-q-x-{remove_id}"), "Remove").on_click(move |_, _, cx| {
                            let _ = remove_me.update(cx, |this, cx| {
                                this.unqueue(&remove_id, cx);
                            });
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        let (title, head) = if items.is_empty() {
            ("Sent".to_owned(), "Claude reads it at its next step")
        } else if busy {
            (
                format!("Queued · {}", items.len()),
                "sent one at a time, as each turn ends",
            )
        } else {
            (
                format!("Queued · {}", items.len()),
                "Claude stopped before its turn ended",
            )
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(self.px(1.))
                .p(self.px(6.))
                .rounded(self.px(16.))
                .border_1()
                .border_color(t.separator)
                .bg(t.bg_sunken)
                .text_size(self.px(12.5))
                .child(
                    div()
                        .flex()
                        .gap(self.px(6.))
                        .px(self.px(6.))
                        .pb(self.px(3.))
                        .text_size(self.px(11.5))
                        .child(
                            div()
                                .flex_none()
                                .whitespace_nowrap()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text_secondary)
                                .child(SharedString::from(title)),
                        )
                        .child(div().flex_1().min_w_0().text_color(t.text_tertiary).child(head)),
                )
                .children(handed)
                .children(rows)
                .into_any_element(),
        )
    }

    /// CHAT-030, VOICE-041: the mic, right of the field; what's said goes in
    /// at the caret, not sent.
    fn mic(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let on_text: crate::voice::OnText = std::rc::Rc::new(move |text: String, cx: &mut gpui::App| {
            let _ = me.update(cx, |this, cx| this.insert_later(&text, cx));
        });
        let place = if self.in_card { "card" } else { "window" };
        crate::voice::session_mic(
            SharedString::from(format!("chat-mic-{place}-{}", self.session_id)),
            crate::voice::SessionMicProps {
                project_id: self.project_id.clone(),
                variant: crate::voice::MicVariant::Composer,
                scale: self.scale,
                own_field: Some(self.composer_handle(cx)),
                on_text,
            },
            window,
            cx,
        )
    }

    /// CHAT-041: the accent once there's something to send, springing up
    /// as it fills; grey while the field is empty.
    fn send_button(&mut self, t: &Tokens, busy: bool, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let empty = self.composer.read(cx).value().trim().is_empty();
        let id = cx.entity_id();
        let target = if empty { 0f32 } else { 1. };
        let ready = transition(
            window,
            cx,
            ("chat-send-ready", id),
            target,
            Duration::from_millis(180),
            motion::ease_out,
        );
        let pop = transition(
            window,
            cx,
            ("chat-send-pop", id),
            target,
            Duration::from_millis(420),
            motion::ease_spring,
        );
        let fg = ai::lerp(t.text_tertiary, t.text_on_accent, ready);
        let glyph = if self.sending {
            look::dots(15. * self.scale, look::clock(window, cx), fg).into_any_element()
        } else {
            ov_icon(OvIcon::ArrowUp, 15. * self.scale, 2.4, fg).into_any_element()
        };
        let me = cx.entity().downgrade();
        let tip = if busy {
            "Queue (↩) · Send now (⌘↩)"
        } else {
            "Send (↩)"
        };
        div()
            .id("chat-send")
            .flex_none()
            .size(self.px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(ai::lerp(t.bg_pressed, t.accent, ready))
            .when((pop - 1.).abs() > 1e-3, |d| d.scale(0.9 + 0.1 * pop))
            .when(!empty, Styled::cursor_pointer)
            .child(glyph)
            .tooltip(native_tooltip(tip.into()))
            .on_click(move |e, window, cx| {
                let now = e.modifiers().platform || e.modifiers().control;
                let _ = me.update(cx, |this, cx| this.submit(now, window, cx));
            })
            .into_any_element()
    }

    /// CHAT-041: stop, a solid round button with a square on it.
    fn stop_button(&mut self, t: &Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let square = if t.dark { t.bg_elevated } else { rgba(255, 255, 255, 1.) };
        div()
            .id("chat-stop")
            .flex_none()
            .size(self.px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(t.text_primary)
            .hover(|d| d.opacity(0.82))
            .child(div().size(self.px(10.)).rounded(self.px(2.5)).bg(square))
            .tooltip(native_tooltip("Stop Claude (Esc)".into()))
            .on_click(move |_, _, cx| {
                let _ = me.update(cx, ChatView::interrupt);
            })
            .into_any_element()
    }

    /// MODEL-030, MODEL-033: the chip naming the session's model and effort,
    /// and the menu it opens above it.
    fn model_picker(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let card = self.card(cx);
        let choice = card.as_ref().map(crate::model_menu::choice_of).unwrap_or_default();
        let tip = if card.as_ref().is_some_and(crate::model_menu::pending) {
            "Model and effort · Claude switches when this turn ends"
        } else {
            "Model and effort"
        };
        let me = cx.entity().downgrade();
        let chip = crate::model_menu::chip(t, "chat-model", &choice, self.model_open, self.scale, tip)
            .relative()
            .child(crate::model_menu::probe(me.clone(), |this: &mut Self, b| {
                this.model_chip = Some(b);
            }))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click({
                let me = me.clone();
                move |_, _, cx| {
                    let _ = me.update(cx, |this, cx| {
                        this.model_open = !this.model_open;
                        cx.notify();
                    });
                }
            });
        let menu = self.model_open.then(|| {
            let (model_me, effort_me, out_me) = (me.clone(), me.clone(), me.clone());
            let on_model: crate::model_menu::OnModel = std::rc::Rc::new(move |model, _, cx| {
                let _ = model_me.update(cx, |this, cx| {
                    this.switch_model(nodus_domain::ModelSwitch::Model(model), cx);
                });
            });
            let on_effort: crate::model_menu::OnEffort = std::rc::Rc::new(move |effort, _, cx| {
                let _ = effort_me.update(cx, |this, cx| {
                    this.switch_model(nodus_domain::ModelSwitch::Effort(effort), cx);
                });
            });
            let body = crate::model_menu::menu(t, "chat-model-row", &choice, self.scale, &on_model, &on_effort);
            let id = cx.entity_id();
            let rise = crate::components::tween::enter(window, cx, ("chat-model-in", id), DUR_FAST, motion::ease_out);
            div().absolute().right_0().bottom_full().pb(self.px(6.)).child(
                div()
                    .id("chat-model-menu")
                    .occlude()
                    .rounded(self.px(14.))
                    .border_1()
                    .border_color(t.border)
                    .bg(look::field_bg(t))
                    .shadow(t.lg())
                    .opacity(rise)
                    .translate(px(0.), self.px(6. * (1. - rise)))
                    .on_mouse_down_out(move |e, _, cx| {
                        let _ = out_me.update(cx, |this, cx| {
                            if this.model_chip.is_some_and(|b| b.contains(&e.position)) {
                                return;
                            }
                            this.model_open = false;
                            cx.notify();
                        });
                    })
                    .child(body),
            )
        });
        div()
            .relative()
            .flex_none()
            .child(chip)
            .children(menu)
            .into_any_element()
    }

    /// Under the field: what Return does now, and the terminal when it's
    /// needed by hand (CHAT-026).
    fn hint(
        &mut self,
        t: &Tokens,
        live: bool,
        status: Option<SessionStatus>,
        menu_open: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let text = match (live, status) {
            _ if menu_open => "↑↓ choose · ↩ runs · ⇥ completes · Esc closes",
            _ if self.dialog => "Claude is asking something in the terminal below",
            (false, _) => "Not running · sending resumes it",
            (true, Some(SessionStatus::Working)) => "Working · ↩ queues · ⌘↩ sends now · Esc stops",
            (true, Some(SessionStatus::Waiting)) => "Claude is asking something below · ↩ queues",
            _ => "↩ sends · ⇧↩ new line · / for commands",
        };
        let open = self.dock.open;
        let me = cx.entity().downgrade();
        let picker = self.model_picker(t, window, cx);
        div()
            .flex()
            .items_center()
            .gap(self.px(4.))
            .px(self.px(4.))
            .text_size(self.px(11.))
            .text_color(t.text_tertiary)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(text),
            )
            .child(picker)
            .when(!open, |d| {
                d.child(
                    div()
                        .id("chat-open-terminal")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(self.px(4.))
                        .px(self.px(5.))
                        .rounded(self.px(4.))
                        .hover(|d| d.bg(t.bg_hover).text_color(t.text_secondary))
                        .child(icon(Icon::SquareTerminal, 12. * self.scale, 1.8, t.text_tertiary))
                        .child("Terminal")
                        .tooltip(native_tooltip("Show Claude’s terminal under the chat".into()))
                        .on_click(move |_, _, cx| {
                            let _ = me.update(cx, |this, cx| this.set_dock(true, cx));
                        }),
                )
            })
            .into_any_element()
    }
}
