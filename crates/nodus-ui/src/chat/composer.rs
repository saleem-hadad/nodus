//! The composer under the chat: the prompts waiting in the queue, the field,
//! and the buttons that send, queue or stop (CHAT-020…CHAT-023).

use gpui::component::Sizable as _;
use gpui::component::input::Textarea;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};
use nodus_domain::SessionStatus;

use super::view::ChatView;
use crate::board::overlays::icons::{OvIcon, ov_icon};
use crate::board::overlays::spinner;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::theme::tokens::{Tokens, alpha};
use crate::theme::{self};

/// The widest the composer gets in the session window, as the column above.
const COMPOSER_W: f32 = 780.;

impl ChatView {
    /// CHAT-020: Return sends (or queues while Claude works), ⌘Return sends
    /// at once, Esc stops Claude; ⇧Return is a new line.
    fn composer_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let k = &e.keystroke;
        let m = k.modifiers;
        let busy = self.card(cx).is_some_and(|c| c.live && c.status != SessionStatus::Done);
        match k.key.as_str() {
            "enter" if !m.shift && !m.alt => {
                cx.stop_propagation();
                self.submit(m.platform || m.control, window, cx);
            }
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
        let queue = self.render_queue(&t, busy, cx);
        let error = self.error.clone().map(|e| {
            div()
                .text_size(self.px(12.))
                .text_color(t.status_waiting)
                .child(SharedString::from(e))
        });
        let send = self.send_button(&t, busy, cx);
        let stop = busy.then(|| self.stop_button(&t, cx));
        let field = Textarea::new(&self.composer)
            .appearance(false)
            .xsmall()
            .text_size(self.px(14.))
            .line_height(self.px(20.))
            .py(self.px(6.))
            .px(self.px(0.));
        let pill = div()
            .flex()
            .items_end()
            .gap(self.px(6.))
            .pl(self.px(12.))
            .pr(self.px(6.))
            .py(self.px(5.))
            .rounded(self.px(16.))
            .border_1()
            .border_color(t.border_strong)
            .bg(t.bg_window)
            .child(div().flex_1().min_w_0().child(field))
            .children(stop)
            .child(send);
        let side = if self.in_card { 10. } else { 24. };
        div()
            .flex_none()
            .w_full()
            .flex()
            .justify_center()
            .px(self.px(side))
            .pt(self.px(6.))
            .pb(self.px(if self.in_card { 10. } else { 18. }))
            .on_key_down(cx.listener(Self::composer_key))
            .child(
                div()
                    .w_full()
                    .max_w(self.px(COMPOSER_W))
                    .flex()
                    .flex_col()
                    .gap(self.px(6.))
                    .children(queue)
                    .children(error)
                    .child(pill)
                    .child(self.hint(&t, live, status, cx)),
            )
            .into_any_element()
    }

    /// CHAT-021, CHAT-023: the prompts waiting for the turn to end.
    fn render_queue(&mut self, t: &Tokens, busy: bool, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let items = self.queue(cx);
        if items.is_empty() {
            return None;
        }
        let me = cx.entity().downgrade();
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(n, item)| {
                let (edit_me, remove_me, now_me) = (me.clone(), me.clone(), me.clone());
                let (edit_id, remove_id, now_id) = (item.id.clone(), item.id.clone(), item.id.clone());
                let action = |id: String, label: &'static str| {
                    div()
                        .id(SharedString::from(id))
                        .cursor_pointer()
                        .px(self.px(5.))
                        .rounded(self.px(4.))
                        .text_color(t.text_tertiary)
                        .hover(|d| d.text_color(t.text_primary).bg(t.bg_hover))
                        .child(label)
                };
                div()
                    .flex()
                    .items_center()
                    .gap(self.px(8.))
                    .px(self.px(6.))
                    .py(self.px(3.))
                    .rounded(self.px(6.))
                    .child(
                        div()
                            .flex_none()
                            .w(self.px(14.))
                            .text_color(t.text_tertiary)
                            .text_size(self.px(11.))
                            .child(SharedString::from((n + 1).to_string())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(item.text.replace('\n', " "))),
                    )
                    .when(n == 0 && !busy, |d| {
                        d.child(
                            action(format!("chat-q-now-{now_id}"), "Send now").on_click(move |_, _, cx| {
                                let _ = now_me.update(cx, |this, cx| this.send_queued_now(&now_id, cx));
                            }),
                        )
                    })
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
        let head = if busy {
            "sent one at a time, as each turn ends"
        } else {
            "Claude stopped before its turn ended"
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(self.px(1.))
                .p(self.px(6.))
                .rounded(self.px(12.))
                .border_1()
                .border_color(t.border)
                .bg(alpha(t.bg_window, 0.6))
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
                                .child(SharedString::from(format!("Queued · {}", items.len()))),
                        )
                        .child(div().flex_1().min_w_0().text_color(t.text_tertiary).child(head)),
                )
                .children(rows)
                .into_any_element(),
        )
    }

    fn send_button(&mut self, t: &Tokens, busy: bool, cx: &mut Context<'_, Self>) -> AnyElement {
        let empty = self.composer.read(cx).value().trim().is_empty();
        let bg = if empty { t.bg_pressed } else { t.accent };
        let fg = if empty { t.text_tertiary } else { t.text_on_accent };
        let glyph = if self.sending {
            spinner("chat-send-spin", 14. * self.scale, 2., fg, cx)
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
            .bg(bg)
            .when(!empty, |d| d.cursor_pointer())
            .child(glyph)
            .tooltip(native_tooltip(tip.into()))
            .on_click(move |e, window, cx| {
                let now = e.modifiers().platform || e.modifiers().control;
                let _ = me.update(cx, |this, cx| this.submit(now, window, cx));
            })
            .into_any_element()
    }

    fn stop_button(&mut self, t: &Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        div()
            .id("chat-stop")
            .flex_none()
            .size(self.px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .border_1()
            .border_color(t.border_strong)
            .hover(|d| d.bg(t.bg_hover))
            .child(icon(Icon::Square, 11. * self.scale, 2.6, t.text_secondary))
            .tooltip(native_tooltip("Stop Claude (Esc)".into()))
            .on_click(move |_, _, cx| {
                let _ = me.update(cx, |this, cx| this.interrupt(cx));
            })
            .into_any_element()
    }

    /// Under the field: what Return does now, and the terminal when it's
    /// needed by hand (CHAT-026).
    fn hint(
        &mut self,
        t: &Tokens,
        live: bool,
        status: Option<SessionStatus>,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let text = match (live, status) {
            _ if self.dialog => "Claude is asking something in the terminal below",
            (false, _) => "Not running · sending resumes it",
            (true, Some(SessionStatus::Working)) => "Working · ↩ queues · ⌘↩ sends now · Esc stops",
            (true, Some(SessionStatus::Waiting)) => "Claude is asking something below · ↩ queues",
            _ => "↩ sends · ⇧↩ new line",
        };
        let open = self.dock.open;
        let me = cx.entity().downgrade();
        div()
            .flex()
            .items_center()
            .px(self.px(4.))
            .text_size(self.px(11.))
            .text_color(t.text_tertiary)
            .child(div().flex_1().child(text))
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
