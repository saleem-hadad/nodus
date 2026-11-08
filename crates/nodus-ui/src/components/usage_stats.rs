//! Claude's status line, for a session's header (`UsageStats.tsx`;
//! UI-109…UI-115): the model, how full its context is, how long it has run
//! and what it has cost. Nothing until Claude has reported.

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use nodus_domain::SessionUsage;

use crate::components::tooltip::native_tooltip;
use crate::format::{format_cost, format_duration, format_tokens};
use crate::parity;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// The usage line for a session; `live` keeps the running time counting.
pub fn usage_stats(
    id: &str,
    usage: Option<&SessionUsage>,
    live: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    // UI-109
    let Some(usage) = usage else {
        return div().into_any_element();
    };
    let t = theme::tokens(cx);
    let level = usage.context_percent.map(|p| {
        if p >= 90.0 {
            t.status_waiting
        } else if p >= 75.0 {
            t.status_working
        } else {
            t.text_tertiary
        }
    });
    let running = usage.duration_ms.map(|ms| {
        if live && !parity::frozen(cx) {
            // UI-113: counts on from the report, re-rendering every second.
            tick_every_second(id, window, cx);
        }
        let since = if live {
            (parity::now_ms(cx) - usage.at).max(0.0)
        } else {
            0.0
        };
        format_duration(ms + since)
    });
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .flex_none()
        .text_size(TEXT_XS)
        .text_color(t.text_tertiary)
        .font_features(fonts::tabular())
        .whitespace_nowrap()
        .when_some(usage.model.clone(), |d, model| {
            d.child(
                div()
                    .id((SharedString::from(format!("{id}-model")), 0usize))
                    .text_color(t.text_secondary)
                    .font_weight(FontWeight::MEDIUM)
                    .tooltip(native_tooltip("Model".into()))
                    .child(model),
            )
        })
        .when_some(usage.context_percent, |d, p| {
            let color = level.unwrap_or(t.text_tertiary);
            d.child(
                div()
                    .id((SharedString::from(format!("{id}-ctx")), 0usize))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(color)
                    .tooltip(native_tooltip(context_title(usage).into()))
                    .child(
                        div()
                            .relative()
                            .w(px(22.))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(t.border)
                            .overflow_hidden()
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(relative((p.min(100.0) / 100.0) as f32))
                                    .rounded(px(2.))
                                    .bg(color),
                            ),
                    )
                    .child(format!("Ctx {}%", nodus_domain::js::round(p))),
            )
        })
        .when_some(running, |d, text| {
            d.child(
                div()
                    .id((SharedString::from(format!("{id}-time")), 0usize))
                    .tooltip(native_tooltip("Time the session has been running".into()))
                    .child(text),
            )
        })
        .when_some(usage.cost_usd, |d, cost| {
            d.child(
                div()
                    .id((SharedString::from(format!("{id}-cost")), 0usize))
                    .tooltip(native_tooltip("Cost so far".into()))
                    .child(format_cost(cost)),
            )
        })
        .into_any_element()
}

/// UI-112.
pub fn context_title(usage: &SessionUsage) -> String {
    match (usage.context_tokens, usage.context_size) {
        (Some(tokens), Some(size)) if size != 0.0 => {
            format!("{} of {} tokens in context", format_tokens(tokens), format_tokens(size))
        }
        _ => "Context window in use".into(),
    }
}

/// Re-renders the current view a second from now, once per pending tick.
fn tick_every_second(id: &str, window: &mut Window, cx: &mut App) {
    let pending = window.use_keyed_state((SharedString::from(format!("{id}-tick")), 0usize), cx, |_, _| false);
    if *pending.read(cx) {
        return;
    }
    pending.update(cx, |p, _| *p = true);
    let view = window.current_view();
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = cx.update(|_, cx| {
                pending.update(cx, |p, _| *p = false);
                cx.notify(view);
            });
        })
        .detach();
}
