//! A goal's tile on the board (spec 21, GOAL-030…GOAL-034): a ring with how
//! far along it is, its title and the number against its target, a small
//! chart of the number (or its steps), what it's doing or when it checks in
//! next, and the last line of its journal.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement, PathBuilder, Rgba, Styled, Window, canvas, div,
    point, px,
};
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};
use nodus_domain::{BoardGoal, GoalStatus, SessionStatus};

use super::BoardEnv;
use super::status_dot::status_dot;
use crate::board::nodes::GoalNode;
use crate::board::units::fu;
use crate::theme::{self, tokens::shadow};

/// The tile's corner radius, in board units.
const RADIUS: f32 = 12.0;

fn stroke(width: f32) -> PathBuilder {
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    PathBuilder::stroke(px(width)).with_style(gpui::PathStyle::Stroke(options))
}

/// A number as the tile writes it: `3.7%`, `1,240 users`, `2.4 s`.
pub fn number(value: f64, unit: &str) -> String {
    let rounded = if value.abs() >= 100.0 {
        format!("{}", value.round())
    } else {
        let s = format!("{value:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    match unit {
        "" => rounded,
        "%" => format!("{rounded}%"),
        unit => format!("{rounded} {unit}"),
    }
}

/// GOAL-031: the line under the title: `3.7% now · target 4% · from 3.1%`,
/// `3 of 5 steps`, or that it's getting started.
pub fn standing(goal: &BoardGoal) -> String {
    if let Some(m) = &goal.metric {
        let mut parts = Vec::new();
        if let Some(v) = m.value {
            parts.push(format!("{} now", number(v, &m.unit)));
        }
        if let Some(t) = m.target {
            parts.push(format!("target {}", number(t, &m.unit)));
        }
        if let Some(s) = m.start {
            parts.push(format!("from {}", number(s, &m.unit)));
        }
        if !parts.is_empty() {
            return parts.join(" · ");
        }
        return m.name.clone();
    }
    if !goal.steps.is_empty() {
        let done = goal.steps.iter().filter(|s| s.done).count();
        return format!("{done} of {} steps", goal.steps.len());
    }
    "Working out how to measure it".to_owned()
}

/// GOAL-032: what it's doing: running, waiting for the user, when it checks
/// in next, paused, reached or stopped.
pub fn state_line(goal: &BoardGoal, running: Option<SessionStatus>, now: f64) -> String {
    match goal.status {
        GoalStatus::Done => return "Reached".to_owned(),
        GoalStatus::Failed => return "Stopped".to_owned(),
        GoalStatus::Paused => return goal.reason.clone().unwrap_or_else(|| "Paused".to_owned()),
        GoalStatus::Active => {}
    }
    match running {
        Some(SessionStatus::Waiting) => "Waiting for you".to_owned(),
        Some(SessionStatus::Working) => {
            if goal.phase.is_empty() {
                "Checking in now".to_owned()
            } else {
                goal.phase.clone()
            }
        }
        _ => match goal.next_at {
            Some(at) => {
                let next = nodus_actions::schedule::span_label((at - now).max(0.0));
                if at <= now {
                    "Checking in soon".to_owned()
                } else if at - now < 48.0 * 3_600_000.0 {
                    format!("Next check-in in {next}")
                } else {
                    format!("Next check-in {}", nodus_actions::schedule::when_label_ms(at, now))
                }
            }
            None => goal.phase.clone(),
        },
    }
}

/// A goal tile node.
pub fn render(env: &Rc<BoardEnv>, node: &GoalNode<'_>, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let z = node.node.zoom;
    let goal = node.goal;
    let now = env.now_ms;
    let pct = goal.percent();
    let waiting = node.running == Some(SessionStatus::Waiting) && goal.status == GoalStatus::Active;
    let ring_color: Rgba = match goal.status {
        GoalStatus::Done => t.status_done,
        GoalStatus::Failed | GoalStatus::Paused => t.text_tertiary,
        GoalStatus::Active if waiting => t.status_waiting,
        GoalStatus::Active => t.accent,
    };
    let track = t.border;
    let progress = pct.map_or(0.0, |p| (p / 100.0) as f32);
    let ring = div()
        .relative()
        .flex_none()
        .size(fu(46.))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let side = f32::from(bounds.size.width.min(bounds.size.height));
                    let width = side * 0.076;
                    let r = side / 2.0 - width / 2.0;
                    let c = bounds.center();
                    let at = |a: f32| c + point(px(r * a.cos()), px(r * a.sin()));
                    let mut path = stroke(width);
                    path.move_to(at(0.0));
                    for i in 1..=72 {
                        path.line_to(at(TAU * i as f32 / 72.0));
                    }
                    if let Ok(p) = path.build() {
                        window.paint_path(p, track);
                    }
                    if progress > 0.0 {
                        let steps = ((72.0 * progress).ceil() as usize).max(2);
                        let mut path = stroke(width);
                        path.move_to(at(-FRAC_PI_2));
                        for i in 1..=steps {
                            path.line_to(at(-FRAC_PI_2 + TAU * progress * i as f32 / steps as f32));
                        }
                        if let Ok(p) = path.build() {
                            window.paint_path(p, ring_color);
                        }
                    }
                },
            )
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_size(fu(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(pct.map_or_else(|| "–".to_owned(), |p| format!("{}%", nodus_domain::js::round(p)))),
        );

    // The number over time against its start and target, or the steps.
    let chart: AnyElement = if let Some(m) = goal.metric.as_ref().filter(|_| !goal.history.is_empty()) {
        let points: Vec<f64> = goal.history.iter().map(|p| p.value).collect();
        let (start, target) = (m.start, m.target);
        let (accent, guide, faint) = (t.accent, t.text_tertiary, t.border_strong);
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let mut lo = points.iter().copied().fold(f64::INFINITY, f64::min);
                let mut hi = points.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                for v in [start, target].into_iter().flatten() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                if (hi - lo).abs() < f64::EPSILON {
                    hi = lo + 1.0;
                }
                let pad = h * 0.12;
                let y = |v: f64| bounds.origin.y + px(h - pad - ((v - lo) / (hi - lo)) as f32 * (h - 2.0 * pad));
                let n = points.len().max(2) - 1;
                let x = |i: usize| bounds.origin.x + px(w * i as f32 / n as f32);
                let line = |v: f64, color: Rgba, window: &mut Window| {
                    let mut p = PathBuilder::stroke(px(1.0));
                    let mut cursor = 0.0;
                    // Dashed: 3 on, 3 off.
                    while cursor < w {
                        p.move_to(point(bounds.origin.x + px(cursor), y(v)));
                        p.line_to(point(bounds.origin.x + px((cursor + 3.0).min(w)), y(v)));
                        cursor += 6.0;
                    }
                    if let Ok(p) = p.build() {
                        window.paint_path(p, color);
                    }
                };
                if let Some(s) = start {
                    line(s, faint, window);
                }
                if let Some(tg) = target {
                    line(tg, guide, window);
                }
                if points.len() > 1 {
                    let mut p = stroke(1.6);
                    p.move_to(point(x(0), y(points[0])));
                    for (i, v) in points.iter().enumerate().skip(1) {
                        p.line_to(point(x(i), y(*v)));
                    }
                    if let Ok(p) = p.build() {
                        window.paint_path(p, accent);
                    }
                }
                let last = points.len() - 1;
                let mut dot = PathBuilder::fill();
                let (cx_, cy_) = (x(last), y(points[last]));
                for i in 0..=16 {
                    let a = TAU * i as f32 / 16.0;
                    let pt = point(cx_ + px(2.6 * a.cos()), cy_ + px(2.6 * a.sin()));
                    if i == 0 {
                        dot.move_to(pt);
                    } else {
                        dot.line_to(pt);
                    }
                }
                if let Ok(p) = dot.build() {
                    window.paint_path(p, accent);
                }
            },
        )
        .w_full()
        .h(fu(40.))
        .into_any_element()
    } else if !goal.steps.is_empty() {
        let rows: Vec<AnyElement> = goal
            .steps
            .iter()
            .take(3)
            .map(|s| {
                div()
                    .flex()
                    .items_center()
                    .gap(fu(6.))
                    .text_size(fu(11.))
                    .line_height(fu(14.))
                    .text_color(if s.done { t.text_tertiary } else { t.text_secondary })
                    .child(
                        div()
                            .flex_none()
                            .size(fu(8.))
                            .rounded_full()
                            .border(fu(1.2))
                            .border_color(if s.done { t.status_done } else { t.border_strong })
                            .when(s.done, |d| d.bg(t.status_done)),
                    )
                    .child(div().truncate().child(s.text.clone()))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(fu(3.))
            .h(fu(40.))
            .children(rows)
            .into_any_element()
    } else {
        div().h(fu(40.)).into_any_element()
    };

    let state = state_line(goal, node.running, now);
    // Only while it's at work or waiting for the user.
    let dot = node
        .running
        .filter(|s| goal.status == GoalStatus::Active && *s != SessionStatus::Done)
        .map(|status| {
            status_dot(
                ElementId::Name(format!("goal-dot-{}", goal.id).into()),
                status,
                true,
                env.z(),
                window,
                cx,
            )
        });
    let journal = goal.journal.last().map_or_else(
        || {
            if goal.check_ins <= 1 && goal.status == GoalStatus::Active {
                "Its first check-in is measuring where it starts.".to_owned()
            } else {
                String::new()
            }
        },
        |e| e.text.clone(),
    );
    let shadows = {
        let layers = if node.node.dragging { &t.shadow_lg } else { &t.shadow_sm };
        let mut s = crate::views::card::shadows(layers, z);
        if node.node.selected || node.node.dragging {
            s.push(shadow(0.0, 0.0, 0.0, 2.0 * z, t.accent));
        }
        s
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(fu(8.))
        .px(fu(14.))
        .pt(fu(12.))
        .pb(fu(10.))
        .rounded(fu(RADIUS))
        .bg(t.bg_elevated)
        .shadow(shadows)
        .child(
            div().flex().items_center().gap(fu(10.)).child(ring).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(fu(2.))
                    .child(
                        div()
                            .text_size(fu(14.))
                            .line_height(fu(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_primary)
                            .truncate()
                            .child(goal.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(fu(12.))
                            .line_height(fu(16.))
                            .text_color(t.text_secondary)
                            .truncate()
                            .child(standing(goal)),
                    ),
            ),
        )
        .child(chart)
        .child(
            div()
                .flex()
                .items_center()
                .gap(fu(6.))
                .text_size(fu(12.))
                .line_height(fu(16.))
                .text_color(if waiting { t.text_primary } else { t.text_secondary })
                .children(dot)
                .child(div().truncate().child(state)),
        )
        .child(div().h(fu(0.5)).bg(t.separator))
        .child(
            div()
                .text_size(fu(11.))
                .line_height(fu(15.))
                .text_color(t.text_tertiary)
                .line_clamp(2)
                .child(journal),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::GoalMetric;

    fn goal() -> BoardGoal {
        serde_json::from_value(serde_json::json!({
            "id": "g", "title": "Signup conversion", "mission": "m", "sessionId": "s", "x": 0, "y": 0,
            "status": "active", "every": 60, "maxCheckIns": 60, "createdAt": 0
        }))
        .unwrap()
    }

    #[test]
    fn says_where_a_goal_stands() {
        let mut g = goal();
        assert_eq!(standing(&g), "Working out how to measure it");
        g.metric = Some(GoalMetric {
            name: "Signup conversion".into(),
            unit: "%".into(),
            start: Some(3.1),
            target: Some(4.0),
            value: Some(3.7),
        });
        assert_eq!(standing(&g), "3.7% now · target 4% · from 3.1%");
        assert_eq!(number(1240.0, "users"), "1240 users");
        assert_eq!(number(2.40, "s"), "2.4 s");
        assert_eq!(state_line(&g, Some(SessionStatus::Waiting), 0.0), "Waiting for you");
        g.next_at = Some(3_600_000.0 * 3.0);
        assert_eq!(state_line(&g, None, 0.0), "Next check-in in 3h");
        g.status = GoalStatus::Done;
        assert_eq!(state_line(&g, None, 0.0), "Reached");
    }
}
