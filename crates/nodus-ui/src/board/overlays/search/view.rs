//! Drawing ⌘K search (`search.css`): a modal high on the window, the field,
//! the results and the footer (SRCH-005…SRCH-017).

use gpui::component::input::Input;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};
use nodus_domain::{SearchSnippet, SearchSnippetKind, SessionStatus};

use super::text::Mark;
use super::{Row, highlighted, time_ago, updated_at};
use crate::board::canvas::BoardCanvas;
use crate::board::layout::card_title;
use crate::board::overlays::{kbd, spinner};
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::status_dot::status_dot;
use crate::components::tween::enter;
use crate::platform::keys;
use crate::theme::tokens::{DUR_MED, RADIUS_MD, RADIUS_SM, RADIUS_XL, TEXT_LG, TEXT_SM, TEXT_XS, Tokens};
use crate::theme::{self, fonts, motion};

impl BoardCanvas {
    /// SRCH-003, SRCH-004: the modal, over the whole window.
    pub(crate) fn render_search(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        if !self.search_open {
            return None;
        }
        let t = theme::tokens(cx);
        let field = self.search_field(window, cx);
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let progress = enter(window, cx, "search-enter", DUR_MED, motion::ease_out);
        let rows = self.search_rows();
        let current = self.overlays.search.active.min(rows.len().saturating_sub(1));
        let body = self.render_search_results(&t, &rows, current, cx);
        let foot = self.render_search_foot(&t, &rows, current, cx);
        let loading = self.search_loading();
        let me = cx.entity().downgrade();
        let panel = div()
            .id("search-panel")
            .absolute()
            .top(px(vh * 0.11))
            .left(px(((vw - (680f32).min(vw - 48.)) / 2.).max(24.)))
            .w(px(680f32.min(vw - 48.)))
            .max_h(px(vh * 0.89 - 48.))
            .flex()
            .flex_col()
            .rounded(RADIUS_XL)
            .bg(t.bg_elevated)
            .shadow(t.modal())
            .overflow_hidden()
            .translate(px(0.), px(8. * (1. - progress)))
            .scale(0.985 + 0.015 * progress)
            .group_opacity(progress)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                if this.search_key_down(e, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(14.))
                    .px(px(16.))
                    .border_b(px(1.))
                    .border_color(t.separator)
                    .child(if loading {
                        spinner("search-spin", 16., 2., t.text_tertiary, cx)
                    } else {
                        icon(Icon::Search, 16., 2.2, t.text_tertiary).into_any_element()
                    })
                    .child(
                        div().flex_1().min_w_0().text_size(TEXT_LG).child(Styled::h(
                            Input::new(&field)
                                .appearance(false)
                                .px(px(2.))
                                .py(px(1.))
                                .text_size(TEXT_LG)
                                .line_height(px(21.)),
                            px(23.),
                        )),
                    )
                    .child(kbd(&t, "esc")),
            )
            .child(body)
            .child(foot);
        let backdrop = div()
            .id("search-backdrop")
            .occlude()
            .w(viewport.width)
            .h(viewport.height)
            .map(|d| frosted(d, t.modal_backdrop, 3., 1.))
            .group_opacity(progress)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                if let Some(c) = me.upgrade() {
                    c.update(cx, |c, cx| c.search_close(cx));
                }
            })
            .child(panel);
        Some(
            deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
                .with_priority(500)
                .into_any_element(),
        )
    }

    fn render_search_results(
        &mut self,
        t: &Tokens,
        rows: &[Row],
        current: usize,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let trimmed = nodus_domain::js::trim(&self.search_query).to_owned();
        let s = &self.overlays.search;
        let terms = nodus_transcripts::search::query_terms(s.answer.as_ref().map_or("", |(q, _)| q.as_str()));
        let empty = !trimmed.is_empty()
            && !self.search_loading()
            && s.answer.as_ref().is_some_and(|(q, _)| *q == trimmed)
            && rows.is_empty()
            && s.error.is_none();
        let error = s.error.clone();
        let marked = s.marked.clone();
        let mut children: Vec<AnyElement> = Vec::new();
        if trimmed.is_empty() && !rows.is_empty() {
            children.push(
                div()
                    .pt(px(6.))
                    .pb(px(4.))
                    .px(px(10.))
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_tertiary)
                    .child("Latest sessions")
                    .into_any_element(),
            );
        }
        for (i, row) in rows.iter().enumerate() {
            let is_marked = marked.contains(&row.card.node.session_id);
            children.push(self.render_search_row(t, i, row, &terms, i == current, is_marked, cx));
        }
        let empty_box = || {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .py(px(28.))
                .px(px(16.))
                .items_center()
        };
        if empty {
            children.push(
                empty_box()
                    .text_color(t.text_secondary)
                    .child(
                        div()
                            .flex()
                            .child("No session on this board mentions ")
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text_primary)
                                    .child(trimmed.clone()),
                            )
                            .child("."),
                    )
                    .child(
                        div()
                            .text_size(TEXT_XS)
                            .text_color(t.text_tertiary)
                            .child("Prompts, replies, file paths, commands and edits are searched; tool output isn’t."),
                    )
                    .into_any_element(),
            );
        }
        if let Some(error) = error {
            children.push(empty_box().text_color(t.status_waiting).child(error).into_any_element());
        }
        if trimmed.is_empty() && rows.is_empty() {
            children.push(
                empty_box()
                    .text_color(t.text_secondary)
                    .child("No sessions on this board yet.")
                    .into_any_element(),
            );
        }
        div()
            .id("search-results")
            .track_scroll(&self.overlays.search.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.))
            .children(children)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_search_row(
        &mut self,
        t: &Tokens,
        index: usize,
        row: &Row,
        terms: &[String],
        active: bool,
        marked: bool,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let card = &row.card;
        let title = card_title(card);
        let id = card.node.session_id.clone();
        let (hover, open, toggle) = (
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
        );
        let row_open = row.clone();
        let toggle_id = id.clone();
        let lead = if card.node.archived {
            icon(Icon::Archive, 12., 2.2, t.text_tertiary).into_any_element()
        } else {
            let label = match card.status {
                SessionStatus::Working => "Working",
                SessionStatus::Waiting => "Needs you",
                SessionStatus::Done => "Done",
            };
            div()
                .id(("search-dot", index))
                .tooltip(crate::components::tooltip::native_tooltip(label.into()))
                .child(status_dot(("search-status", index), card.status).small())
                .into_any_element()
        };
        let title_ranges = nodus_transcripts::search::match_ranges(&title, terms);
        let check = div()
            .id(("search-check", index))
            .flex_none()
            .size(px(16.))
            .mt(px(1.))
            .rounded(px(5.))
            .border(px(1.5))
            .border_color(if marked { t.accent } else { t.border_strong })
            .when(marked, |d| d.bg(t.accent))
            .flex()
            .items_center()
            .justify_center()
            .opacity(if active || marked { 1. } else { 0. })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                if let Some(c) = toggle.upgrade() {
                    c.update(cx, |c, cx| c.search_toggle(&toggle_id, cx));
                }
            })
            .when(marked, |d| d.child(icon(Icon::Check, 11., 3., t.text_on_accent)));
        let mut body: Vec<AnyElement> = vec![
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .min_w_0()
                .child(lead)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(highlighted(
                            &title,
                            &title_ranges,
                            Mark::Underline(t.mark_strong.into()),
                        )),
                )
                .children(row.hit.as_ref().filter(|h| h.matches > 0).map(|h| {
                    let n = h.matches;
                    meta_text(t, format!("{n} {}", if n == 1 { "match" } else { "matches" }))
                }))
                .child(meta_text(t, time_ago(updated_at(card), crate::parity::now_ms(cx))))
                .into_any_element(),
        ];
        match &row.hit {
            Some(hit) => body.extend(hit.snippets.iter().map(|s| snippet_line(t, s))),
            None => {
                if let Some(prompt) = card.meta.as_ref().and_then(|m| m.last_prompt.clone()) {
                    body.push(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(TEXT_SM)
                            .line_height(px(17.4))
                            .text_color(t.text_secondary)
                            .child(prompt)
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .id(("search-row", index))
            .flex()
            .gap(px(10.))
            .py(px(8.))
            .px(px(10.))
            .rounded(RADIUS_MD)
            .when(active, |d| d.bg(t.bg_hover))
            .when(marked, |d| d.bg(t.bg_selected))
            .on_mouse_move(move |_, _, cx| {
                if let Some(c) = hover.upgrade() {
                    c.update(cx, |c, cx| {
                        if c.overlays.search.active != index {
                            c.overlays.search.active = index;
                            cx.notify();
                        }
                    });
                }
            })
            .on_click(move |e, _, cx| {
                let modk = crate::platform::Platform::CURRENT.mod_key(&e.modifiers());
                if let Some(c) = open.upgrade() {
                    c.update(cx, |c, cx| {
                        if modk {
                            c.search_toggle(&row_open.card.node.session_id.clone(), cx);
                        } else {
                            c.search_open_row(&row_open, cx);
                        }
                    });
                }
            })
            .child(check)
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(3.)).children(body))
            .into_any_element()
    }

    fn render_search_foot(
        &mut self,
        t: &Tokens,
        rows: &[Row],
        current: usize,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let marked = self.overlays.search.marked.len();
        let ready = marked > 0;
        let disabled = rows.is_empty() && marked == 0;
        let archived = rows.get(current).is_some_and(|r| r.card.node.archived);
        let label = match marked {
            0 => "New session from this".to_owned(),
            1 => "New session from 1".to_owned(),
            n => format!("New session from {n}"),
        };
        let cap = |k: String| foot_kbd(t, &k, false);
        let hint = |caps: Vec<AnyElement>, text: &str| {
            div()
                .flex()
                .items_center()
                .children(caps)
                .child(div().ml(px(2.75)).child(text.to_owned()))
        };
        let me = cx.entity().downgrade();
        let fg = if ready { t.text_on_accent } else { t.accent };
        let start = div()
            .id("search-start")
            .ml_auto()
            .flex()
            .items_center()
            .gap(px(6.))
            .pt(px(4.))
            .pb(px(4.))
            .pr(px(6.))
            .pl(px(9.))
            .rounded(RADIUS_SM)
            .when(ready, |d| d.bg(t.accent))
            .when(!ready && !disabled, |d| d.hover(|s| s.bg(t.bg_hover)))
            .when(ready, |d| d.hover(|s| s.bg(t.accent_hover)))
            .when(disabled, |d| d.opacity(0.4))
            .text_size(TEXT_XS)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg)
            .when(!disabled, |d| {
                d.on_click(move |_, _, cx| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, |c, cx| c.search_start_merge(cx));
                    }
                })
            })
            .child(icon(Icon::Sparkles, 13., 2.2, fg))
            .child(label)
            .child(foot_kbd(t, &keys("⌘"), ready))
            .child(foot_kbd(t, &keys("↩"), ready));
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .pt(px(8.))
            .pb(px(8.))
            .pr(px(10.))
            .pl(px(16.))
            .border_t(px(1.))
            .border_color(t.separator)
            .text_size(TEXT_XS)
            .text_color(t.text_tertiary)
            .child(hint(vec![cap("↑".into()), cap("↓".into())], "move"))
            .child(hint(
                vec![cap(keys("↩"))],
                if archived { "read it" } else { "go to card" },
            ))
            .child(hint(vec![cap("⇥".into())], "select"))
            .child(start)
            .into_any_element()
    }
}

/// The count and the time at a row's right.
fn meta_text(t: &Tokens, text: String) -> AnyElement {
    div()
        .flex_none()
        .text_size(TEXT_XS)
        .text_color(t.text_tertiary)
        .font_features(fonts::tabular())
        .child(text)
        .into_any_element()
}

/// The footer's key caps (min 17px wide, 4px padding); on the filled
/// start button they're a white wash with no border.
fn foot_kbd(t: &Tokens, key: &str, on_accent: bool) -> AnyElement {
    let base = div()
        .flex_none()
        .min_w(px(17.))
        .px(px(4.))
        .mx(px(1.))
        .rounded(px(4.))
        .border(px(0.5))
        .text_size(TEXT_XS)
        .line_height(px(17.))
        .flex()
        .justify_center()
        .child(key.to_owned());
    if on_accent {
        base.border_color(gpui::transparent_black())
            .bg(theme::tokens::rgba(255, 255, 255, 0.2))
            .into_any_element()
    } else {
        base.border_color(t.border_strong)
            .bg(t.bg_elevated)
            .shadow(vec![theme::tokens::shadow(0., 1., 0., 0., t.border)])
            .text_color(t.text_secondary)
            .into_any_element()
    }
}

/// SRCH-011: who said it (or which tool, on what file) and the text.
fn snippet_line(t: &Tokens, s: &SearchSnippet) -> AnyElement {
    let (who, color) = match s.kind {
        SearchSnippetKind::Prompt => ("You".to_owned(), t.accent),
        SearchSnippetKind::Reply => ("Claude".to_owned(), t.text_tertiary),
        SearchSnippetKind::Tool => (s.tool.clone().unwrap_or_default(), t.text_tertiary),
    };
    let code = s.kind == SearchSnippetKind::Tool;
    let label = div()
        .flex_none()
        .flex()
        .gap(px(4.))
        .min_w(px(46.))
        .max_w(gpui::relative(0.36))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(TEXT_XS)
        .font_weight(FontWeight::SEMIBOLD)
        .line_height(px(17.))
        .text_color(color)
        .when(s.agent == Some(true), |d| {
            d.child(div().font_weight(FontWeight::MEDIUM).child("Agent"))
        })
        .child(who)
        .when_some(s.file.clone(), |d, file| {
            d.child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .font_family(fonts::MONO)
                    .font_weight(FontWeight::MEDIUM)
                    .child(file),
            )
        });
    let text = div()
        .flex_1()
        .min_w_0()
        .line_clamp(2)
        .overflow_hidden()
        .when(code, |d| {
            d.font_family(fonts::MONO).text_size(TEXT_XS).line_height(px(17.))
        })
        .child(highlighted(&s.text, &s.ranges, Mark::Background(t.mark.into())));
    div()
        .flex()
        .gap(px(8.))
        .min_w_0()
        .text_size(TEXT_SM)
        .line_height(px(17.4))
        .text_color(t.text_secondary)
        .child(label)
        .child(text)
        .into_any_element()
}
