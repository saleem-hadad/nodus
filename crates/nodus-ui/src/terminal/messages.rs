//! One message of a transcript (`Message`, `ToolCalls`, `Skeleton` in
//! `TranscriptPreview.tsx`; TRANS-010…TRANS-018), shared by the transcript
//! preview and the agents viewer (AGV-020).

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, ElementId, FontWeight, HighlightStyle, InteractiveElement, IntoElement, ParentElement,
    Pixels, SharedString, StatefulInteractiveElement, Styled, StyledText, Window, canvas, div, linear_color_stop,
    linear_gradient, px, relative,
};
use nodus_domain::{MessageRole, SearchSnippet, ToolCall, TranscriptMessage};

use super::text::{CALLS_SHOWN, earlier_calls_label, format_time, is_long};
use crate::components::spaced_text;
use crate::parity;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// A click on one of a message's toggles.
pub type Action = Rc<dyn Fn(&mut Window, &mut App)>;
/// Told where the message lies.
pub type OnBounds = Rc<dyn Fn(Bounds<Pixels>, &mut App)>;

/// How a message is shown.
pub struct MessageProps<'a> {
    pub id: ElementId,
    pub message: &'a TranscriptMessage,
    /// Who said it, when that isn't you and Claude.
    pub author: Option<String>,
    /// The search's terms to mark (TRANS-010).
    pub highlight: &'a [String],
    /// The message the search led to (TRANS-011).
    pub found: bool,
    /// The tool call in it that matched (TRANS-012).
    pub found_call: Option<&'a SearchSnippet>,
    /// Long text shown whole.
    pub expanded: bool,
    /// Every call shown.
    pub all_calls: bool,
    pub on_toggle: Action,
    pub on_all_calls: Action,
    /// Told where the message lies, for scrolling it into view.
    pub on_bounds: Option<OnBounds>,
}

/// UTF-16 ranges as byte ranges of `text`.
fn byte_ranges(text: &str, ranges: &[[u32; 2]]) -> Vec<std::ops::Range<usize>> {
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut units = 0u32;
    for (i, c) in text.char_indices() {
        map.push((units, i));
        units += c.len_utf16() as u32;
    }
    map.push((units, text.len()));
    let at = |u: u32| {
        map.iter()
            .find(|(units, _)| *units >= u)
            .map_or(text.len(), |(_, b)| *b)
    };
    ranges
        .iter()
        .map(|[s, e]| at(*s)..at(*e))
        .filter(|r| r.start < r.end)
        .collect()
}

/// Text with the marked ranges (`Highlight.tsx`) that can be selected
/// (`.selectable`), kept under `id`.
pub fn selectable_highlighted(
    id: impl Into<ElementId>,
    text: &str,
    ranges: &[[u32; 2]],
    cx: &App,
) -> crate::components::selectable::SelectableText {
    let t = theme::tokens(cx);
    let style = HighlightStyle {
        background_color: Some(t.mark.into()),
        ..Default::default()
    };
    crate::components::selectable::selectable_text(id, text.to_owned())
        .highlights(byte_ranges(text, ranges).into_iter().map(|r| (r, style)).collect())
}

/// Text with the marked ranges (`Highlight.tsx`): the mark colour behind, text unchanged.
pub fn highlighted(text: &str, ranges: &[[u32; 2]], cx: &App) -> StyledText {
    let t = theme::tokens(cx);
    let styled = StyledText::new(SharedString::from(text.to_owned()));
    if ranges.is_empty() {
        return styled;
    }
    let style = HighlightStyle {
        background_color: Some(t.mark.into()),
        ..Default::default()
    };
    styled.with_highlights(byte_ranges(text, ranges).into_iter().map(|r| (r, style)))
}

/// A sunken chip with a hairline ring (`.tp-tool`, `.tp-call-name`).
fn chip(label: String, cx: &App) -> gpui::Div {
    let t = theme::tokens(cx);
    div()
        .flex_none()
        .px(px(7.))
        .rounded(px(5.))
        .bg(t.bg_sunken)
        .shadow(vec![inset_shadow(0., 0., 0., 0.5, t.border)])
        .font_family(fonts::mono())
        .text_size(TEXT_XS)
        .text_color(t.text_secondary)
        .child(label)
}

/// A `.tp-more` text button.
fn more(id: impl Into<ElementId>, label: String, on_click: Action, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    div()
        .id(id)
        .text_size(TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.accent)
        .hover(Styled::underline)
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(label)
        .into_any_element()
}

/// `ToolCalls` (TRANS-018): the last eight until the rest are asked for.
fn tool_calls(id: &ElementId, calls: &[ToolCall], all: bool, on_all: Action, top: f32, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    let hidden = if all {
        0
    } else {
        calls.len().saturating_sub(CALLS_SHOWN)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .mt(px(top))
        .when(hidden > 0, |d| {
            d.child(div().flex().mb(px(2.)).child(more(
                (id.clone(), "earlier"),
                earlier_calls_label(hidden),
                on_all,
                cx,
            )))
        })
        .children(calls[hidden..].iter().enumerate().map(|(i, call)| {
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .min_w_0()
                .font_family(fonts::mono())
                .text_size(TEXT_XS)
                .line_height(px(18.))
                .child(chip(call.name.clone(), cx))
                .when_some(call.detail.clone(), |d, detail| {
                    // `.tp-call-detail.selectable`.
                    d.child(div().min_w_0().truncate().text_color(t.text_tertiary).child(
                        crate::components::selectable::selectable_text(
                            (id.clone(), SharedString::from(format!("call-{}", hidden + i))),
                            detail,
                        ),
                    ))
                })
        }))
        .into_any_element()
}

/// `Message`.
pub fn message(p: &MessageProps<'_>, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    let m = p.message;
    let text = nodus_domain::js::trim(&m.text).to_owned();
    let long = is_long(&text);
    let user = m.role == MessageRole::User;
    let notice = m.notice == Some(true);
    let ranges = if p.highlight.is_empty() {
        Vec::new()
    } else {
        nodus_transcripts::search::match_ranges(&text, p.highlight)
    };
    let author = p
        .author
        .clone()
        .unwrap_or_else(|| if user { "You".into() } else { "Claude".into() });
    let author_color = if m.role == MessageRole::Assistant {
        t.status_working
    } else {
        t.text_secondary
    };
    let now = parity::now_ms(cx);
    let collapsed = long && !p.expanded;

    let meta = div()
        .flex()
        .items_baseline()
        .gap(px(8.))
        .mb(px(5.))
        .text_size(TEXT_XS)
        .text_color(t.text_tertiary)
        // `letter-spacing: 0.01em`.
        .child(spaced_text(
            author,
            gpui::Font {
                weight: FontWeight::SEMIBOLD,
                ..gpui::font(fonts::UI)
            },
            TEXT_XS,
            px(0.01 * 11.0),
            px(11.0 * 1.4),
            author_color,
        ))
        .when(m.timestamp > 0.0, |d| d.child(format_time(m.timestamp, now)));

    let content = (!text.is_empty()).then(|| {
        div()
            .text_size(if notice { TEXT_SM } else { TEXT_MD })
            .line_height(relative(1.55))
            .when(user && !notice, |d| {
                d.px(px(14.))
                    .py(px(10.))
                    .rounded(RADIUS_MD)
                    .bg(t.bg_sunken)
                    .shadow(vec![inset_shadow(0., 0., 0., 0.5, t.separator)])
            })
            .when(notice, |d| d.text_color(t.text_secondary))
            .child(
                div()
                    .when(collapsed, |d| {
                        d.max_h(px(13.0 * 1.55 * 12.0)).overflow_hidden().mask(linear_gradient(
                            180.,
                            linear_color_stop(gpui::black(), 0.7),
                            linear_color_stop(gpui::transparent_black(), 1.0),
                        ))
                    })
                    .cursor_text()
                    .child(selectable_highlighted((p.id.clone(), "text"), &text, &ranges, cx)),
            )
            .when(long, |d| {
                let label = if p.expanded { "Show less" } else { "Show more" };
                d.child(div().flex().mt(px(4.)).child(more(
                    (p.id.clone(), "more"),
                    label.into(),
                    p.on_toggle.clone(),
                    cx,
                )))
            })
    });

    // The message is block flow in 0.9.4, so a margin right under the meta
    // row collapses into its 5 px bottom margin: the larger of the two shows.
    let after_meta = |margin: f32| {
        if content.is_none() {
            (margin - 5.0).max(0.0)
        } else {
            margin
        }
    };
    let found_call = p.found_call.map(|s| {
        let tool = s.tool.clone().unwrap_or_default();
        let name = if s.agent == Some(true) {
            format!("Agent · {tool}")
        } else {
            tool
        };
        div()
            .flex()
            .items_baseline()
            .gap(px(8.))
            .min_w_0()
            .mt(px(after_meta(8.)))
            .font_family(fonts::mono())
            .text_size(TEXT_XS)
            .line_height(px(18.))
            .child(chip(name, cx).when_some(s.file.clone(), |d, file| {
                d.child(div().ml(px(6.)).text_color(t.text_tertiary).child(file))
            }))
            .child(
                div()
                    .min_w_0()
                    .text_color(t.text_secondary)
                    .cursor_text()
                    .child(selectable_highlighted((p.id.clone(), "found"), &s.text, &s.ranges, cx)),
            )
    });

    let tools_top = if found_call.is_some() { 6. } else { after_meta(6.) };
    let tools = match (&m.calls, &m.tools) {
        (Some(calls), _) => Some(tool_calls(
            &p.id,
            calls,
            p.all_calls,
            p.on_all_calls.clone(),
            tools_top,
            cx,
        )),
        (None, Some(tools)) if !tools.is_empty() => Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .mt(px(tools_top))
                .children(tools.iter().map(|tool| chip(tool.clone(), cx).py(px(1.))))
                .into_any_element(),
        ),
        _ => None,
    };

    let on_bounds = p.on_bounds.clone();
    div()
        .id(p.id.clone())
        .relative()
        .flex()
        .flex_col()
        .when(p.found, |d| {
            d.mx(px(-12.))
                .my(px(-8.))
                .px(px(12.))
                .py(px(8.))
                .rounded(RADIUS_MD)
                .bg(t.bg_sunken)
                .shadow(vec![inset_shadow(2., 0., 0., 0., t.mark_strong)])
        })
        .child(meta)
        .children(content)
        .children(found_call)
        .children(tools)
        .when_some(on_bounds, |d, on_bounds| {
            d.child(
                canvas(move |bounds, _, cx| on_bounds(bounds, cx), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
        })
        .into_any_element()
}

/// `Skeleton` (TRANS-004): five shimmering rows, labelled `Loading transcript`.
pub fn skeleton(cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    let line = |w: gpui::DefiniteLength, h: f32| div().w(w).h(px(h)).rounded(px(6.)).bg(t.bg_sunken);
    div()
        .flex()
        .flex_col()
        .gap(px(22.))
        .pt(px(16.))
        .children([72., 94., 58., 86., 40.].into_iter().map(|w: f32| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(line(px(64.).into(), 9.))
                .child(line(relative(w / 100.), 12.))
        }))
        .into_any_element()
}

/// `.tp-empty`: a quiet centred note, an optional detail, an optional action.
pub fn empty_note(title: &str, detail: Option<&str>, action: Option<AnyElement>, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .px(px(24.))
        .py(px(80.))
        .text_center()
        .text_color(t.text_secondary)
        .child(SharedString::from(title.to_owned()))
        .when_some(detail, |d, detail| {
            d.child(
                div()
                    .max_w(px(520.))
                    .font_family(fonts::mono())
                    .text_size(TEXT_SM)
                    .text_color(t.text_tertiary)
                    .cursor_text()
                    // `.tp-empty-detail.selectable`.
                    .child(crate::components::selectable::selectable_text(
                        "tp-empty-detail",
                        detail.to_owned(),
                    )),
            )
        })
        .children(action)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trans_010_marks_utf16_ranges() {
        assert_eq!(byte_ranges("añb button", &[[4, 10]]), vec![5..11]);
        assert_eq!(byte_ranges("x", &[[1, 1]]), Vec::<std::ops::Range<usize>>::new());
    }
}
