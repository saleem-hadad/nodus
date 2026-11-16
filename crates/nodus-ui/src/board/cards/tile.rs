//! A scheduled action's tile on the board (spec 19, SCHED-030…SCHED-036):
//! the clock with the ring that fills toward the next run, the action's name
//! and schedule, when it runs next, and below a hairline what its last run
//! said, with a dot until the user has looked.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div,
};
use nodus_actions::schedule;

use super::BoardEnv;
use super::status_dot::status_dot;
use crate::board::nodes::TileNode;
use crate::board::units::fu;
use crate::components::clock_ring::clock_ring;
use crate::theme::{self, tokens::shadow};

/// The tile's corner radius, in board units.
const RADIUS: f32 = 12.0;

/// What the state line says: when it runs next, that it's running, or that
/// it's paused (SCHED-031).
pub fn state_line(node: &TileNode<'_>, now: f64) -> String {
    let Some(action) = node.action else {
        return String::new();
    };
    let Some(s) = &action.schedule else {
        return String::new();
    };
    if node.running.is_some() {
        return "Running now".to_owned();
    }
    if s.paused == Some(true) {
        return "Paused".to_owned();
    }
    schedule::next_label_ms(s, now).map_or_else(String::new, |next| format!("Next {next}"))
}

/// What the last run said and when it ran (SCHED-034): `9:00`, `yesterday
/// 9:00 · late`, `Tue 9:00 · by hand`.
pub fn last_line(node: &TileNode<'_>, now: f64) -> Option<(String, String)> {
    let last = node.tile.last.as_ref()?;
    if node.running.is_some() && last.done_at.is_none() {
        return None;
    }
    let message = last
        .message
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| if last.done_at.is_some() { "Finished" } else { "Ran" }.to_owned());
    let mut when = schedule::when_label_ms(last.due.unwrap_or(last.at), now);
    if last.late == Some(true) {
        when.push_str(" · ran late, Nodus was closed");
    } else if last.due.is_none() {
        when.push_str(" · run by hand");
    }
    Some((message, when))
}

/// A tile node.
pub fn render(env: &Rc<BoardEnv>, node: &TileNode<'_>, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let z = node.node.zoom;
    let tile = node.tile;
    let now = env.now_ms;
    let name = node.action.map_or("Scheduled action", |a| a.name.as_str()).to_owned();
    let schedule = node.action.and_then(|a| a.schedule.as_ref());
    let said = schedule.map(schedule::describe).unwrap_or_default();
    let paused = schedule.is_some_and(|s| s.paused == Some(true));
    let progress = match (schedule, node.running) {
        (_, Some(_)) => 1.0,
        (Some(s), None) if !paused => schedule::progress(s, now) as f32,
        _ => 0.0,
    };
    let state = state_line(node, now);
    let last = last_line(node, now);
    let unread = tile
        .last
        .as_ref()
        .is_some_and(|l| l.done_at.is_some() && l.seen.is_none());

    // The board's own dot, whose ring stays round (CARD-012).
    let dot = node.running.map(|status| {
        status_dot(
            ElementId::Name(format!("tile-dot-{}", tile.id).into()),
            status,
            true,
            env.z(),
            window,
            cx,
        )
    });
    let ring = if paused {
        clock_ring(fu(22.), 0.0).without_ring()
    } else {
        clock_ring(fu(22.), progress)
    };
    let shadows = {
        let layers = if node.node.dragging { &t.shadow_lg } else { &t.shadow_sm };
        let mut s = crate::views::card::shadows(layers, z);
        if node.node.selected || node.node.dragging {
            s.push(shadow(0.0, 0.0, 0.0, 2.0 * z, t.accent));
        }
        s
    };
    let head = div().flex().items_start().gap(fu(10.)).child(ring).child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(fu(1.))
            .child(
                div()
                    .text_size(fu(14.))
                    .line_height(fu(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_primary)
                    .truncate()
                    .child(name),
            )
            .child(
                div()
                    .text_size(fu(12.))
                    .line_height(fu(16.))
                    .text_color(t.text_secondary)
                    .truncate()
                    .child(said),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(fu(5.))
                    .text_size(fu(12.))
                    .line_height(fu(16.))
                    .text_color(t.text_tertiary)
                    .children(dot)
                    .child(state),
            ),
    );
    let foot = match last {
        Some((message, when)) => {
            let actions = env.actions.clone();
            let id = tile.id.clone();
            div()
                .id(ElementId::Name(format!("tile-last-{}", tile.id).into()))
                .flex()
                .gap(fu(7.))
                .mx(fu(-6.))
                .px(fu(6.))
                .py(fu(4.))
                .rounded(fu(6.))
                .hover(|d| d.bg(t.bg_hover))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    actions.open_tile(&id, cx);
                })
                .child(
                    div()
                        .flex_none()
                        .mt(fu(5.))
                        .size(fu(6.))
                        .rounded_full()
                        .when(unread, |d| d.bg(t.accent)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(fu(12.))
                                .line_height(fu(16.))
                                .text_color(t.text_primary)
                                .when(!unread, |d| d.text_color(t.text_secondary))
                                .truncate()
                                .child(message),
                        )
                        .child(
                            div()
                                .text_size(fu(11.))
                                .line_height(fu(14.))
                                .text_color(t.text_tertiary)
                                .truncate()
                                .child(when),
                        ),
                )
                .into_any_element()
        }
        None => div()
            .text_size(fu(12.))
            .line_height(fu(16.))
            .py(fu(4.))
            .text_color(t.text_tertiary)
            .child(if node.running.is_some() {
                "Its run is beside it."
            } else {
                "No runs yet."
            })
            .into_any_element(),
    };
    div()
        .id(ElementId::Name(format!("tile-{}", tile.id).into()))
        .size_full()
        .flex()
        .flex_col()
        .gap(fu(8.))
        .px(fu(14.))
        .pt(fu(12.))
        .pb(fu(8.))
        .rounded(fu(RADIUS))
        .bg(t.bg_elevated)
        .shadow(shadows)
        .child(head)
        .child(div().h(fu(0.5)).bg(t.separator))
        .child(foot)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardTile, ProjectAction, SessionStatus, TileRun};
    use serde_json::json;

    fn action(schedule: &serde_json::Value) -> ProjectAction {
        serde_json::from_value(json!({
            "id": "a", "name": "PR sweep", "icon": "zap", "description": "", "prompt": "p", "createdAt": 0,
            "schedule": schedule
        }))
        .unwrap()
    }

    fn tile(last: Option<TileRun>) -> BoardTile {
        BoardTile {
            id: "a".into(),
            x: 0.0,
            y: 0.0,
            handled: 0.0,
            last,
            created_at: 0.0,
            extra: serde_json::Map::new(),
        }
    }

    fn node<'a>(tile: &'a BoardTile, action: &'a ProjectAction, running: Option<SessionStatus>) -> TileNode<'a> {
        TileNode {
            node: crate::board::nodes::NodeState {
                id: "a".into(),
                size: gpui::size(288.0, 136.0),
                selected: false,
                dragging: false,
                zoom: 1.0,
            },
            tile,
            action: Some(action),
            running,
        }
    }

    #[test]
    fn says_what_the_tile_is_doing() {
        let a = action(&json!({ "every": 120 }));
        let empty = tile(None);
        assert!(state_line(&node(&empty, &a, None), 0.0).starts_with("Next "));
        assert_eq!(
            state_line(&node(&empty, &a, Some(SessionStatus::Working)), 0.0),
            "Running now"
        );
        let paused = action(&json!({ "every": 120, "paused": true }));
        assert_eq!(state_line(&node(&empty, &paused, None), 0.0), "Paused");
        assert_eq!(last_line(&node(&empty, &a, None), 0.0), None);
        let ran = tile(Some(TileRun {
            session_id: "s".into(),
            at: 5.0,
            due: Some(0.0),
            late: Some(true),
            done_at: Some(9.0),
            message: Some("3 PRs wait for review".into()),
            seen: None,
        }));
        let (message, when) = last_line(&node(&ran, &a, None), 0.0).unwrap();
        assert_eq!(message, "3 PRs wait for review");
        assert!(when.ends_with(" · ran late, Nodus was closed"), "{when}");
    }
}
