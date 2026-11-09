//! Resizing a node from its edges and corners, as React Flow's
//! `NodeResizer` does for expanded cards (XCARD-029, XCARD-030) and notes
//! (NOTEUI-025, NOTEUI-026): invisible 10px zones straddling the edges,
//! corner zones above them, the far edges held still, a minimum size, the
//! box shown live while dragging and saved rounded on release.

use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Rgba, StatefulInteractiveElement, Styled, Window, canvas, div,
};

use super::present::{ResizeEdges, resize, rounded};
use super::{BoardEnv, Resized, anim};

#[derive(Debug, Clone, Copy)]
struct Drag {
    edges: ResizeEdges,
    start: Point<Pixels>,
    rect: (f32, f32, f32, f32),
}

/// The corner handles' look, when they show (notes).
#[derive(Debug, Clone, Copy)]
pub struct Handles {
    pub size: f32,
    pub border: f32,
    pub border_color: Rgba,
    pub fill: Rgba,
    pub radius: f32,
}

const EDGE: f32 = 10.0;

/// The zones for the node `node` at `rect` (flow units), relative to the
/// node's own corner, never smaller than `min`.
#[allow(clippy::too_many_arguments)]
pub fn resize_zones(
    env: &Rc<BoardEnv>,
    node: &str,
    target: Resized,
    rect: (f32, f32, f32, f32),
    min: (f32, f32),
    corner: f32,
    handles: Option<Handles>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let z = env.z();
    let (_, _, w, h) = rect;
    let drag = window.use_keyed_state(anim::key(node, "resize"), cx, |_, _| None::<Drag>);
    let mut out = Vec::new();
    let edge = |l: bool, r: bool, t: bool, b: bool| ResizeEdges {
        left: l,
        right: r,
        top: t,
        bottom: b,
    };
    // Edges, then corners above them (z 2 and 3).
    let zones: [(ResizeEdges, (f32, f32, f32, f32), CursorStyle, bool); 8] = [
        (edge(true, false, false, false), (-EDGE / 2.0, 0.0, EDGE, h), CursorStyle::ResizeLeftRight, false),
        (edge(false, true, false, false), (w - EDGE / 2.0, 0.0, EDGE, h), CursorStyle::ResizeLeftRight, false),
        (edge(false, false, true, false), (0.0, -EDGE / 2.0, w, EDGE), CursorStyle::ResizeUpDown, false),
        (edge(false, false, false, true), (0.0, h - EDGE / 2.0, w, EDGE), CursorStyle::ResizeUpDown, false),
        (edge(true, false, true, false), (-corner / 2.0, -corner / 2.0, corner, corner), CursorStyle::ResizeUpLeftDownRight, true),
        (edge(false, true, true, false), (w - corner / 2.0, -corner / 2.0, corner, corner), CursorStyle::ResizeUpRightDownLeft, true),
        (edge(true, false, false, true), (-corner / 2.0, h - corner / 2.0, corner, corner), CursorStyle::ResizeUpRightDownLeft, true),
        (edge(false, true, false, true), (w - corner / 2.0, h - corner / 2.0, corner, corner), CursorStyle::ResizeUpLeftDownRight, true),
    ];
    for (i, (edges, (x, y, zw, zh), cursor, is_corner)) in zones.into_iter().enumerate() {
        let drag = drag.clone();
        let mut zone = div()
            .id(anim::key_n(node, "resize-zone", i))
            .absolute()
            .left(z.px(x))
            .top(z.px(y))
            .w(z.px(zw))
            .h(z.px(zh))
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                drag.update(cx, |d, _| {
                    *d = Some(Drag {
                        edges,
                        start: e.position,
                        rect,
                    });
                });
            })
            .on_click(|_, _, cx| cx.stop_propagation());
        if is_corner && let Some(hs) = handles {
            zone = zone.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(z.px(hs.radius))
                    .border(z.px(hs.border))
                    .border_color(hs.border_color)
                    .bg(hs.fill),
            );
        }
        out.push(zone.into_any_element());
    }

    // While dragging, follow the pointer anywhere in the window.
    if drag.read(cx).is_some() {
        let env = env.clone();
        let target = Rc::new(target);
        out.push(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let zoom = env.zoom.max(0.01);
                    let moving = drag.clone();
                    let (env_move, node_move) = (env.clone(), target.clone());
                    window.on_mouse_event(move |e: &MouseMoveEvent, _, window, cx| {
                        let Some(d) = *moving.read(cx) else {
                            return;
                        };
                        let dx = f32::from(e.position.x - d.start.x) / zoom;
                        let dy = f32::from(e.position.y - d.start.y) / zoom;
                        let r = resize(d.rect, d.edges, dx, dy, min);
                        env_move.actions.resize(&node_move, r, false, cx);
                        window.refresh();
                    });
                    let ending = drag.clone();
                    window.on_mouse_event(move |e: &MouseUpEvent, _, window, cx| {
                        let Some(d) = *ending.read(cx) else {
                            return;
                        };
                        ending.update(cx, |d, _| *d = None);
                        let dx = f32::from(e.position.x - d.start.x) / zoom;
                        let dy = f32::from(e.position.y - d.start.y) / zoom;
                        let (x, y, w, h) = rounded(resize(d.rect, d.edges, dx, dy, min));
                        env.actions
                            .resize(&target, (x as f32, y as f32, w as f32, h as f32), true, cx);
                        window.refresh();
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

/// Whether `node` is being resized.
pub fn resizing(node: &str, window: &mut Window, cx: &mut App) -> bool {
    let drag = window.use_keyed_state(anim::key(node, "resize"), cx, |_, _| None::<Drag>);
    drag.read(cx).is_some()
}
