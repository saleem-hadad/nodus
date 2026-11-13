//! Drawing the chat's rows: the user's messages, Claude's words as Markdown,
//! runs of tool calls, and the charts, forms and pages Claude shows.

use gpui::base::text::{TextView, TextViewStyle};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};
use nodus_domain::{Answers, ChatEntry, SessionStatus};
use serde_json::Value;

use super::view::{ChatView, RowKind};
use crate::board::overlays::spinner;
use crate::components::icon::{Icon, icon};
use crate::theme::tokens::{Tokens, alpha};
use crate::theme::{self, fonts};

/// The widest the conversation's column gets in the session window.
const COLUMN_W: f32 = 780.;
/// A run of tool calls shows this many of its last calls until opened (CHAT-009).
const TOOLS_SHOWN: usize = 3;

/// The Markdown look in the theme's colours.
pub(crate) fn markdown_style(t: &Tokens) -> TextViewStyle {
    TextViewStyle::default()
        .with_foreground(t.text_primary.into())
        .with_muted_foreground(t.text_secondary.into())
        .with_link(t.accent.into())
        .with_selection(t.bg_selected.into())
        .with_code_background(t.bg_sunken.into())
        .with_border(t.border.into())
        .with_dark(t.dark)
}

impl ChatView {
    pub(crate) fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let Some(row) = self.rows.get(ix).copied() else {
            return div().into_any_element();
        };
        let last = ix + 1 == self.rows.len();
        let body = match row.kind {
            RowKind::User(i) => self.render_user(i, &t, cx),
            RowKind::Tools(start, end) => self.render_tools(start, end, &t, cx),
            RowKind::Entry(i) => self.render_entry(i, &t, window, cx),
        };
        let mut column = div()
            .w_full()
            .max_w(self.px(COLUMN_W))
            .flex()
            .flex_col()
            .gap(self.px(6.));
        if row.header {
            let at = self.entries.get(entry_index(row.kind)).map_or(0., ChatEntry::at);
            let now = crate::parity::now_ms(cx);
            column = column.child(self.author("Claude", at, now, t.status_working, &t));
        }
        column = column.child(body);
        if last {
            column = column.children(self.working_line(&t, cx));
        }
        let (side, top) = if self.in_card { (12., 10.) } else { (24., 14.) };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(self.px(side))
            .pt(self.px(if row.header || matches!(row.kind, RowKind::User(_)) {
                top + 6.
            } else {
                top * 0.5
            }))
            .when(last, |d| d.pb(self.px(16.)))
            .child(column)
            .into_any_element()
    }

    fn author(&self, name: &'static str, at: f64, now: f64, color: gpui::Rgba, t: &Tokens) -> AnyElement {
        div()
            .flex()
            .items_baseline()
            .gap(self.px(8.))
            .text_size(self.px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .whitespace_nowrap()
            .child(name)
            .when(at > 0., |d| {
                d.child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(t.text_tertiary)
                        .child(crate::terminal::text::format_time(at, now)),
                )
            })
            .into_any_element()
    }

    /// CHAT-005, CHAT-006, CHAT-007: what the user said, on the right.
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
            .gap(self.px(3.))
            .when(*queued, |d| {
                d.child(
                    div()
                        .text_size(self.px(11.))
                        .text_color(t.text_tertiary)
                        .child("Queued while Claude worked"),
                )
            })
            .child(
                div()
                    .max_w(nodus_ratio(0.86))
                    .px(self.px(12.))
                    .py(self.px(8.))
                    .rounded(self.px(14.))
                    .bg(t.bg_sunken)
                    .border_1()
                    .border_color(t.separator)
                    .child(inner),
            )
            .into_any_element()
    }

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
                        .child(div().child(SharedString::from(value))),
                );
            }
        }
        div()
            .min_w(self.px(260.))
            .flex()
            .flex_col()
            .gap(self.px(5.))
            .text_size(self.px(12.5))
            .line_height(self.px(18.))
            .child(
                div()
                    .text_size(self.px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_secondary)
                    .child(SharedString::from(format!("Answers · {}", answers.title))),
            )
            .children(lines)
            .into_any_element()
    }

    /// CHAT-009: a run of tool calls, the last few showing until opened.
    fn render_tools(&mut self, start: usize, end: usize, t: &Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let first_id = self.entries.get(start).map(|e| e.id().to_owned()).unwrap_or_default();
        let open = self.opened.contains(&first_id);
        let count = end - start;
        let from = if open || count <= TOOLS_SHOWN {
            start
        } else {
            end - TOOLS_SHOWN
        };
        let hidden = from - start;
        let mut lines = Vec::new();
        for i in from..end {
            if let Some(ChatEntry::Tool {
                name,
                detail,
                failed,
                done,
                ..
            }) = self.entries.get(i)
            {
                let mark = if *failed {
                    icon(Icon::X, 11. * self.scale, 2.4, t.status_waiting).into_any_element()
                } else if *done {
                    icon(Icon::Check, 11. * self.scale, 2.4, t.text_tertiary).into_any_element()
                } else {
                    spinner("chat-tool-spin", 10. * self.scale, 1.6, t.status_working, cx)
                };
                lines.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(self.px(8.))
                        .h(self.px(20.))
                        .min_w_0()
                        .child(div().flex_none().w(self.px(12.)).flex().justify_center().child(mark))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if *failed { t.status_waiting } else { t.text_secondary })
                                .child(SharedString::from(name.clone())),
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
                        ),
                );
            }
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
                .cursor_pointer()
                .text_size(self.px(11.5))
                .text_color(t.text_tertiary)
                .hover(|d| d.text_color(t.text_secondary))
                .child(label)
                .on_click(move |_, _, cx| {
                    let _ = me.update(cx, |this, cx| this.toggle_tools(&id, cx));
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(self.px(1.))
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
                        .style(markdown_style(t))
                        .selectable(true),
                )
                .into_any_element(),
            ChatEntry::Notice { text, .. } => div()
                .w_full()
                .flex()
                .justify_center()
                .text_size(self.px(11.5))
                .line_height(self.px(16.))
                .text_color(t.text_tertiary)
                .child(
                    div()
                        .max_w(nodus_ratio(0.9))
                        .px(self.px(10.))
                        .py(self.px(3.))
                        .rounded(self.px(8.))
                        .bg(t.bg_sunken)
                        .child(SharedString::from(text)),
                )
                .into_any_element(),
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
                .px(self.px(10.))
                .py(self.px(4.))
                .rounded(self.px(7.))
                .border_1()
                .border_color(t.border)
                .bg(t.bg_window)
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
            .rounded(self.px(12.))
            .border_1()
            .border_color(t.border)
            .bg(t.bg_window)
            .child(
                div()
                    .flex_none()
                    .size(self.px(36.))
                    .rounded(self.px(8.))
                    .bg(alpha(t.accent, 0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(Icon::LayoutDashboard, 18. * self.scale, 1.8, t.accent)),
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

    /// CHAT-019: under the last row, what Claude is doing.
    fn working_line(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let card = self.card(cx)?;
        if !card.live {
            return None;
        }
        let (label, color) = match card.status {
            SessionStatus::Working => ("Working…", t.status_working),
            SessionStatus::Waiting => ("Claude needs you below", t.status_waiting),
            SessionStatus::Done => return None,
        };
        let mark = if card.status == SessionStatus::Working {
            spinner("chat-working-spin", 11. * self.scale, 1.8, color, cx)
        } else {
            div().size(self.px(7.)).rounded_full().bg(color).into_any_element()
        };
        Some(
            div()
                .pt(self.px(8.))
                .flex()
                .items_center()
                .gap(self.px(8.))
                .text_size(self.px(12.))
                .text_color(t.text_secondary)
                .child(mark)
                .child(label)
                .into_any_element(),
        )
    }
}

fn entry_index(kind: RowKind) -> usize {
    match kind {
        RowKind::User(i) | RowKind::Entry(i) | RowKind::Tools(i, _) => i,
    }
}

fn nodus_ratio(share: f32) -> gpui::DefiniteLength {
    gpui::relative(share)
}
