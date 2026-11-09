//! Resizing a selected view (VIEW-140, React Flow's `NodeResizer`):
//! invisible 10-unit zones straddling the edges, 9×9 corner handles above
//! them, never smaller than 240 × 160, the live box held by the canvas while
//! dragging and saved rounded on release. While it's resized the page shows
//! its snapshot and lets the pointer through (VIEW-141).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, CursorStyle, DispatchPhase, ElementId, Entity, InteractiveElement, IntoElement, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Styled, Window, canvas, div,
};
use nodus_board::geometry::{VIEW_MIN_H, VIEW_MIN_W};
use nodus_domain::{BoardView, Rect, ViewPatch, js};

use super::card::CardUi;
use crate::board::nodes::{BoardCx, NodeState};
use crate::board::units::fu;
use crate::theme;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    edges: Edges,
    start: Point<Pixels>,
    from: Rect,
    zoom: f32,
}

/// The box `from` resized by a pointer moved `(dx, dy)` flow units with
/// `edges` held, the far edges still.
fn resized(from: Rect, edges: Edges, dx: f64, dy: f64) -> Rect {
    let mut r = from;
    if edges.right {
        r.w = (from.w + dx).max(VIEW_MIN_W);
    }
    if edges.left {
        r.w = (from.w - dx).max(VIEW_MIN_W);
        r.x = from.x + from.w - r.w;
    }
    if edges.bottom {
        r.h = (from.h + dy).max(VIEW_MIN_H);
    }
    if edges.top {
        r.h = (from.h - dy).max(VIEW_MIN_H);
        r.y = from.y + from.h - r.h;
    }
    r
}

/// The zones and handles for a selected view, in its node's box.
pub fn zones(
    view: &BoardView,
    state: &NodeState,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let t = theme::tokens(cx);
    let (w, h) = (state.size.width, state.size.height);
    let drag = window.use_keyed_state(
        ElementId::Name(format!("view-resize-{}", view.id).into()),
        cx,
        |_, _| None::<Drag>,
    );
    let edge = |l: bool, r: bool, t: bool, b: bool| Edges {
        left: l,
        right: r,
        top: t,
        bottom: b,
    };
    // (edges, x, y, w, h, cursor, corner)
    let half = 5.0;
    let c = 4.5;
    let areas = [
        (
            edge(false, false, true, false),
            0.0,
            -half,
            w,
            10.0,
            CursorStyle::ResizeUpDown,
            false,
        ),
        (
            edge(false, false, false, true),
            0.0,
            h - half,
            w,
            10.0,
            CursorStyle::ResizeUpDown,
            false,
        ),
        (
            edge(true, false, false, false),
            -half,
            0.0,
            10.0,
            h,
            CursorStyle::ResizeLeftRight,
            false,
        ),
        (
            edge(false, true, false, false),
            w - half,
            0.0,
            10.0,
            h,
            CursorStyle::ResizeLeftRight,
            false,
        ),
        (
            edge(true, false, true, false),
            -c,
            -c,
            9.0,
            9.0,
            CursorStyle::ResizeUpLeftDownRight,
            true,
        ),
        (
            edge(false, true, true, false),
            w - c,
            -c,
            9.0,
            9.0,
            CursorStyle::ResizeUpRightDownLeft,
            true,
        ),
        (
            edge(true, false, false, true),
            -c,
            h - c,
            9.0,
            9.0,
            CursorStyle::ResizeUpRightDownLeft,
            true,
        ),
        (
            edge(false, true, false, true),
            w - c,
            h - c,
            9.0,
            9.0,
            CursorStyle::ResizeUpLeftDownRight,
            true,
        ),
    ];
    let from = Rect {
        x: view.x,
        y: view.y,
        w: f64::from(w),
        h: f64::from(h),
    };
    let zoom = state.zoom;
    let mut out: Vec<AnyElement> = areas
        .into_iter()
        .enumerate()
        .map(|(i, (edges, x, y, zw, zh, cursor, corner))| {
            let (drag, ui) = (drag.clone(), ui.clone());
            div()
                .id(ElementId::Name(format!("view-resize-{}-{i}", view.id).into()))
                .absolute()
                .left(fu(x))
                .top(fu(y))
                .w(fu(zw))
                .h(fu(zh))
                .cursor(cursor)
                .when(corner, |d| {
                    d.border(fu(1.5))
                        .border_color(t.accent)
                        .rounded(fu(2.))
                        .bg(t.bg_elevated)
                })
                .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                    cx.stop_propagation();
                    drag.update(cx, |d, cx| {
                        *d = Some(Drag {
                            edges,
                            start: e.position,
                            from,
                            zoom,
                        });
                        cx.notify();
                    });
                    ui.update(cx, |u, cx| {
                        u.resizing = true;
                        cx.notify();
                    });
                })
                .into_any_element()
        })
        .collect();
    // While dragging, the window's pointer is the resize's.
    if let Some(d) = *drag.read(cx) {
        let (drag, ui, actions, id) = (drag.clone(), ui.clone(), board.actions.clone(), view.id.clone());
        out.push(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let (actions_move, id_move) = (actions.clone(), id.clone());
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let z = f64::from(d.zoom.max(1e-3));
                        let r = resized(
                            d.from,
                            d.edges,
                            f64::from(f32::from(e.position.x - d.start.x)) / z,
                            f64::from(f32::from(e.position.y - d.start.y)) / z,
                        );
                        actions_move.hold_rect(&id_move, Some(r), cx);
                    });
                    let (drag, ui, actions, id) = (drag.clone(), ui.clone(), actions.clone(), id.clone());
                    window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let z = f64::from(d.zoom.max(1e-3));
                        let r = resized(
                            d.from,
                            d.edges,
                            f64::from(f32::from(e.position.x - d.start.x)) / z,
                            f64::from(f32::from(e.position.y - d.start.y)) / z,
                        );
                        drag.update(cx, |d, cx| {
                            *d = None;
                            cx.notify();
                        });
                        ui.update(cx, |u, cx| {
                            u.resizing = false;
                            cx.notify();
                        });
                        actions.patch_view(
                            ViewPatch {
                                id: id.clone(),
                                x: Some(js::round(r.x)),
                                y: Some(js::round(r.y)),
                                w: Some(js::round(r.w)),
                                h: Some(js::round(r.h)),
                                title: None,
                                starts_sessions: None,
                            },
                            cx,
                        );
                        actions.hold_rect(&id, None, cx);
                    });
                },
            )
            .absolute()
            .size_0()
            .into_any_element(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: Rect = Rect {
        x: 100.0,
        y: 50.0,
        w: 480.0,
        h: 360.0,
    };

    #[test]
    fn keeps_the_far_edges_still() {
        let e = Edges {
            left: true,
            top: true,
            ..Edges::default()
        };
        let r = resized(FROM, e, 40.0, -20.0);
        assert_eq!((r.x, r.y, r.w, r.h), (140.0, 30.0, 440.0, 380.0));
        assert_eq!(r.x + r.w, FROM.x + FROM.w);
    }

    #[test]
    fn stops_at_the_minimum() {
        let e = Edges {
            right: true,
            bottom: true,
            ..Edges::default()
        };
        let r = resized(FROM, e, -1000.0, -1000.0);
        assert_eq!((r.w, r.h), (240.0, 160.0));
        let e = Edges {
            left: true,
            ..Edges::default()
        };
        let r = resized(FROM, e, 1000.0, 0.0);
        assert_eq!((r.x, r.w), (100.0 + 480.0 - 240.0, 240.0));
    }
}
