//! Drawing the chat's rows: the user's messages, Claude's words as Markdown,
//! runs of tool calls, and the charts, forms and pages Claude shows.

use std::time::Duration;

use gpui::base::text::{TextView, TextViewStyle};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Window, div,
};
use gpui::{HighlightStyle, StyleRefinement, px, relative, rems};
use nodus_domain::{Answers, ChatEntry, SessionStatus};
use serde_json::Value;

use super::look;
use super::view::{ChatView, RowKind};
use crate::board::overlays::spinner;
use crate::components::icon::{Icon, icon};
use crate::theme::motion;
use crate::theme::tokens::Tokens;
use crate::theme::{self, fonts};

/// The widest the conversation's column gets in the session window.
const COLUMN_W: f32 = 780.;
/// A run of tool calls shows this many of its last calls until opened (CHAT-009).
const TOOLS_SHOWN: usize = 3;

/// The Markdown look in the theme's colours, every size at `s` (CHAT-009).
pub(crate) fn markdown_style(t: &Tokens, s: f32) -> TextViewStyle {
    TextViewStyle::default()
        .with_dark(t.dark)
        .with_foreground(t.text_primary.into())
        .with_muted_foreground(t.text_secondary.into())
        .with_link(t.accent.into())
        .with_selection(t.bg_selected.into())
        .with_code_background(t.bg_sunken.into())
        .with_border(t.border.into())
        // Inline code keeps a light wash on the light theme's: name it for both.
        .with_inline_code(HighlightStyle {
            background_color: Some(if t.dark { t.bg_pressed } else { t.bg_hover }.into()),
            ..HighlightStyle::default()
        })
        .with_paragraph_gap(rems(0.6 * s))
        .with_heading(move |level| {
            let size = match level {
                1 => 17.,
                2 => 15.,
                _ => 13.5,
            };
            StyleRefinement::default()
                .text_size(px(size * s))
                .font_weight(FontWeight::SEMIBOLD)
        })
        .with_code_block(StyleRefinement::default().text_size(px(12. * s)).rounded(px(8. * s)))
}

/// How a row moves in (CHAT-040).
#[derive(Clone, Copy)]
enum Arrival {
    /// The user's bubble pops up from its corner.
    Pop,
    /// Claude's words show from the top down.
    Reveal,
    /// A chart, form or page lifts into place.
    Lift,
    /// A notice or a run of tools fades in.
    Fade,
}

impl ChatView {
    pub(crate) fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let Some(row) = self.rows.get(ix).copied() else {
            return div().into_any_element();
        };
        let last = ix + 1 == self.rows.len();
        let first = self.entries.get(entry_index(row.kind));
        let born = first.and_then(|e| self.born.get(e.id()).copied());
        let at = first.map_or(0., ChatEntry::at);
        let (arrival, ms) = match (row.kind, first) {
            (RowKind::User(_), _) => (Arrival::Pop, 420),
            (RowKind::Tools(..), _) | (_, Some(ChatEntry::Notice { .. })) => (Arrival::Fade, 280),
            (_, Some(ChatEntry::Text { .. })) => (Arrival::Reveal, 900),
            _ => (Arrival::Lift, 480),
        };
        let p = look::arrival(born, Duration::from_millis(ms), window, cx);
        let body = match row.kind {
            RowKind::User(i) => self.render_user(i, &t, cx),
            RowKind::Tools(start, end) => self.render_tools(start, end, &t, window, cx),
            RowKind::Entry(i) => self.render_entry(i, &t, window, cx),
        };
        let mut said = div().w_full().flex().flex_col().gap(self.px(8.));
        if row.header {
            let now = crate::parity::now_ms(cx);
            said = said.child(self.author(at, now, &t));
        }
        said = said.child(body);
        let e = motion::ease_out(p);
        let said = match arrival {
            Arrival::Pop => look::rise(said, e, 10. * self.scale)
                .scale(0.9 + 0.1 * motion::ease_spring(p))
                .transform_origin(1., 1.),
            Arrival::Reveal => look::reveal(said, e).when(p < 1., |d| d.translate(px(0.), self.px(4. * (1. - e)))),
            Arrival::Lift => look::rise(said, e, 12. * self.scale).when(p < 1., |d| d.scale(0.98 + 0.02 * e)),
            Arrival::Fade => look::rise(said, e, 4. * self.scale),
        };
        let mut column = div().w_full().max_w(self.px(COLUMN_W)).flex().flex_col().child(said);
        if last {
            column = column.children(self.working_line(&t, window, cx));
        }
        let (side, top) = if self.in_card { (12., 10.) } else { (24., 14.) };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(self.px(side))
            .pt(self.px(if row.header || matches!(row.kind, RowKind::User(_)) {
                top + 8.
            } else {
                top * 0.5
            }))
            .when(last, |d| d.pb(self.px(20.)))
            .child(column)
            .into_any_element()
    }

    /// CHAT-009, CHAT-035: Claude's mark and name, and when it spoke.
    fn author(&self, at: f64, now: f64, t: &Tokens) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(self.px(8.))
            .text_size(self.px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(look::avatar(22. * self.scale, None, t))
            .child("Claude")
            .when(at > 0., |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(self.px(12.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(t.text_tertiary)
                        .child(crate::terminal::text::format_time(at, now)),
                )
            })
            .into_any_element()
    }

    /// Ms into Claude's turn while it works, for the motion that runs as
    /// long as it does (held at 0 in parity shots and under reduced motion).
    pub(crate) fn working_ms(&self, window: &mut Window, cx: &gpui::App) -> Option<f64> {
        let since = self.working_since?;
        if crate::parity::frozen(cx) || cx.reduce_motion() {
            return Some(0.);
        }
        window.request_animation_frame();
        Some(since.elapsed().as_secs_f64() * 1000.)
    }

    /// CHAT-005, CHAT-006, CHAT-007, CHAT-037: what the user said, in a
    /// bubble on the right.
    fn render_user(&mut self, i: usize, t: &Tokens, _cx: &mut Context<'_, Self>) -> AnyElement {
        let Some(ChatEntry::User {
            text, queued, answers, ..
        }) = self.entries.get(i)
        else {
            return div().into_any_element();
        };
        let inner = match answers {
            Some(answers) => self.answers_card(answers, t),
            None => div()
                .text_size(self.px(13.5))
                .line_height(self.px(20.))
                .child(SharedString::from(text.clone()))
                .into_any_element(),
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(self.px(4.))
            .when(*queued, |d| {
                d.child(
                    div()
                        .pr(self.px(4.))
                        .text_size(self.px(11.))
                        .text_color(t.text_tertiary)
                        .child("Queued while Claude worked"),
                )
            })
            .child(
                div()
                    .max_w(relative(0.82))
                    .px(self.px(14.))
                    .py(self.px(9.))
                    .rounded(self.px(18.))
                    .rounded_br(self.px(6.))
                    .bg(t.bg_hover)
                    .child(inner),
            )
            .into_any_element()
    }

    /// CHAT-007: the answers to a form, on the user's bubble.
    fn answers_card(&self, answers: &Answers, t: &Tokens) -> AnyElement {
        let mut lines = Vec::new();
        if let Value::Object(map) = &answers.answers {
            for (key, value) in map {
                let value = match value {
                    Value::String(s) => s.clone(),
                    Value::Array(items) => items
                        .iter()
                        .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                        .collect::<Vec<_>>()
                        .join(", "),
                    Value::Null => "skipped".into(),
                    other => other.to_string(),
                };
                lines.push(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(self.px(11.5))
                                .text_color(t.text_secondary)
                                .child(SharedString::from(key.clone())),
                        )
                        .child(div().font_weight(FontWeight::MEDIUM).child(SharedString::from(value))),
                );
            }
        }
        div()
            .min_w(self.px(260.))
            .flex()
            .flex_col()
            .gap(self.px(6.))
            .text_size(self.px(12.5))
            .line_height(self.px(18.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(self.px(6.))
                    .text_size(self.px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_secondary)
                    .child(
                        div()
                            .flex_none()
                            .child(icon(Icon::Check, 11. * self.scale, 2.6, t.text_secondary)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(SharedString::from(format!("Answers · {}", answers.title))),
                    ),
            )
            .children(lines)
            .into_any_element()
    }

    /// CHAT-009, CHAT-038: a run of tool calls on a card, the last few
    /// showing until opened.
    fn render_tools(
        &mut self,
        start: usize,
        end: usize,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let first_id = self.entries.get(start).map(|e| e.id().to_owned()).unwrap_or_default();
        let open = self.opened.contains(&first_id);
        let count = end - start;
        let from = if open || count <= TOOLS_SHOWN {
            start
        } else {
            end - TOOLS_SHOWN
        };
        let hidden = from - start;
        let working = self.working_ms(window, cx);
        let mut lines = Vec::new();
        for i in from..end {
            let Some(ChatEntry::Tool {
                id,
                name,
                detail,
                failed,
                done,
                ..
            }) = self.entries.get(i)
            else {
                continue;
            };
            let running = !*done && !*failed;
            let glyph = if running {
                spinner("chat-tool-spin", 12. * self.scale, 1.8, t.text_secondary, cx)
            } else {
                icon(
                    look::tool_icon(name),
                    12.5 * self.scale,
                    1.9,
                    if *failed { t.status_waiting } else { t.text_tertiary },
                )
                .into_any_element()
            };
            // A light passes through the name of what's running.
            let label = match working {
                Some(ms) if running => StyledText::new(SharedString::from(name.clone()))
                    .with_highlights(look::shimmer(name, ms, t.text_primary))
                    .into_any_element(),
                _ => SharedString::from(name.clone()).into_any_element(),
            };
            let p = look::arrival(self.born.get(id).copied(), Duration::from_millis(320), window, cx);
            let e = motion::ease_out(p);
            let line = div()
                .flex()
                .items_center()
                .gap(self.px(8.))
                .h(self.px(26.))
                .px(self.px(8.))
                .rounded(self.px(8.))
                .min_w_0()
                .when(running, |d| d.bg(t.bg_hover))
                .child(div().flex_none().w(self.px(14.)).flex().justify_center().child(glyph))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if *failed { t.status_waiting } else { t.text_secondary })
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(fonts::mono())
                        .text_size(self.px(11.5))
                        .text_color(t.text_tertiary)
                        .child(SharedString::from(detail.clone())),
                )
                .when(!running, |d| d.child(look::outcome(*failed, 11. * self.scale, t)));
            lines.push(if p < 1. {
                line.opacity(e).translate(self.px(-8. * (1. - e)), px(0.))
            } else {
                line
            });
        }
        let me = cx.entity().downgrade();
        let toggle = (hidden > 0 || (open && count > TOOLS_SHOWN)).then(|| {
            let label = if open {
                "Show fewer".to_owned()
            } else if hidden == 1 {
                "Show 1 earlier call".to_owned()
            } else {
                format!("Show {hidden} earlier calls")
            };
            let id = first_id.clone();
            div()
                .id(SharedString::from(format!("chat-tools-{first_id}")))
                .flex()
                .items_center()
                .gap(self.px(6.))
                .h(self.px(24.))
                .px(self.px(8.))
                .rounded(self.px(8.))
                .cursor_pointer()
                .text_size(self.px(11.5))
                .text_color(t.text_tertiary)
                .hover(|d| d.text_color(t.text_secondary).bg(t.bg_hover))
                .child(div().flex_none().child(icon(
                    if open { Icon::ChevronDown } else { Icon::ChevronRight },
                    11. * self.scale,
                    2.2,
                    t.text_tertiary,
                )))
                .child(div().flex_1().min_w_0().whitespace_nowrap().child(label))
                .on_click(move |_, _, cx| {
                    let _ = me.update(cx, |this, cx| this.toggle_tools(&id, cx));
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(self.px(1.))
            .p(self.px(4.))
            .rounded(self.px(12.))
            .bg(t.bg_sunken)
            .border_1()
            .border_color(t.separator)
            .text_size(self.px(12.))
            .children(toggle)
            .children(lines)
            .into_any_element()
    }

    fn render_entry(&mut self, i: usize, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let Some(entry) = self.entries.get(i).cloned() else {
            return div().into_any_element();
        };
        match entry {
            ChatEntry::Text { id, markdown, .. } => div()
                .w_full()
                .text_size(self.px(13.5))
                .line_height(self.px(21.))
                .child(
                    TextView::markdown(SharedString::from(format!("chat-md-{id}")), markdown)
                        .style(markdown_style(t, self.scale))
                        .selectable(true),
                )
                .into_any_element(),
            // CHAT-039: a notice sits between hairlines, in the middle.
            ChatEntry::Notice { text, .. } => {
                let rule = || div().flex_1().min_w(self.px(16.)).h(px(1.)).bg(t.separator);
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(self.px(10.))
                    .text_size(self.px(11.5))
                    .line_height(self.px(16.))
                    .text_color(t.text_tertiary)
                    .child(rule())
                    .child(
                        div()
                            .max_w(relative(0.8))
                            .px(self.px(10.))
                            .py(self.px(3.))
                            .rounded(self.px(10.))
                            .bg(t.bg_sunken)
                            .child(SharedString::from(text)),
                    )
                    .child(rule())
                    .into_any_element()
            }
            ChatEntry::Visual { id, spec, .. } => self.render_visual(&id, &spec, t, window, cx),
            ChatEntry::Questions { id, spec, answered, .. } => {
                self.render_questions(&id, &spec, answered.as_ref(), t, window, cx)
            }
            ChatEntry::Page {
                title,
                view_id,
                answered,
                ..
            } => self.render_page(&title, view_id.as_deref(), answered.is_some(), t, cx),
            ChatEntry::User { .. } | ChatEntry::Tool { .. } => div().into_any_element(),
        }
    }

    /// CHAT-028: a page Claude put on the board.
    fn render_page(
        &mut self,
        title: &str,
        view_id: Option<&str>,
        answered: bool,
        t: &Tokens,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let line = match (view_id, answered) {
            (None, _) => "Putting it on the board…",
            (Some(_), true) => "On the board · Answered",
            (Some(_), false) => "On the board, beside this session",
        };
        let me = cx.entity().downgrade();
        let button = view_id.map(|view| {
            let view = view.to_owned();
            div()
                .id(SharedString::from(format!("chat-page-{view}")))
                .flex_none()
                .cursor_pointer()
                .px(self.px(12.))
                .py(self.px(5.))
                .rounded_full()
                .border_1()
                .border_color(t.border)
                .hover(|d| d.bg(t.bg_hover))
                .text_size(self.px(12.))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap()
                .child("Show on board")
                .on_click(move |_, _, cx| {
                    let _ = me.update(cx, |this, cx| this.show_on_board(&view, cx));
                })
        });
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(self.px(12.))
            .p(self.px(10.))
            .rounded(self.px(14.))
            .border_1()
            .border_color(t.border)
            .bg(t.bg_elevated)
            .shadow(t.sm())
            .child(
                div()
                    .flex_none()
                    .size(self.px(38.))
                    .rounded(self.px(10.))
                    .bg(t.bg_sunken)
                    .border_1()
                    .border_color(t.separator)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(Icon::LayoutDashboard, 18. * self.scale, 1.8, t.text_secondary)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(SharedString::from(title.to_owned())),
                    )
                    .child(div().text_size(self.px(12.)).text_color(t.text_secondary).child(line)),
            )
            .children(button)
            .into_any_element()
    }

    /// CHAT-019: under the last row, what Claude is doing: its mark with the
    /// sparkle twinkling, and a light passing through `Thinking…` (before it
    /// has said anything since the user did) or `Working…`.
    fn working_line(&self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let card = self.card(cx)?;
        if !card.live {
            return None;
        }
        let line = div()
            .pt(self.px(12.))
            .flex()
            .items_center()
            .gap(self.px(9.))
            .text_size(self.px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.text_secondary);
        match card.status {
            SessionStatus::Working => {
                let ms = self.working_ms(window, cx).unwrap_or(0.);
                let quiet = matches!(self.rows.last().map(|r| r.kind), Some(RowKind::User(_)));
                let label = if quiet { "Thinking…" } else { "Working…" };
                let p = look::arrival(self.working_since, Duration::from_millis(320), window, cx);
                Some(
                    look::rise(line, motion::ease_out(p), 6. * self.scale)
                        .child(look::avatar(20. * self.scale, Some(ms), t))
                        .child(StyledText::new(label).with_highlights(look::shimmer(label, ms, t.text_primary)))
                        .into_any_element(),
                )
            }
            SessionStatus::Waiting => Some(
                line.child(
                    div()
                        .size(self.px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(waiting_dot(t, self.scale)),
                )
                .child("Claude needs you below")
                .into_any_element(),
            ),
            SessionStatus::Done => None,
        }
    }
}

/// The red dot of a session waiting on the user.
fn waiting_dot(t: &Tokens, scale: f32) -> Div {
    div().size(px(7. * scale)).rounded_full().bg(t.status_waiting)
}

fn entry_index(kind: RowKind) -> usize {
    match kind {
        RowKind::User(i) | RowKind::Entry(i) | RowKind::Tools(i, _) => i,
    }
}
