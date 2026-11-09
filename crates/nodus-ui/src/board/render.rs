//! Drawing the board, back to front (spec 09 § Rust mapping › Render
//! layers): the dotted background, the nodes below the edges (sections, the
//! archive, the pad), the edges, every other node by z-index, the selection
//! rectangle and the marquee, then the zoom pill, minimap, hint, toast and
//! confirm. Nodes glide to where they belong as their CSS transitions did
//! (§ Node motion).

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, DispatchPhase, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Render, Styled, Window, canvas,
    div, fill, point, px, size,
};
use nodus_board::geometry::Point;
use nodus_domain::{BoardNote, FolderColor, Rect};

use super::canvas::{BoardCanvas, Gesture, Glide};
use super::edges;
use super::hud::rgba_mix;
use super::layout::{note_pad_rect, pad_sheet_spot};
use super::nodes::{
    ArchiveNode, BoardCx, CardNode, NodeRenderers, NodeState, NoteNode, PadNode, StackPlace, StackStyle, ViewNode,
    renderers,
};
use super::scene::{FlowNode, NodeKind, paint_order};
use super::section::SectionProps;
use super::units::zoomed;
use crate::theme::{self, motion, tokens::Tokens};

/// How a node's drawn box follows its place (`transition` in board.css).
enum Motion {
    /// No transition: it jumps.
    None,
    /// `transform` (position) over a duration, with a delay.
    Position(u64, u64, fn(f32) -> f32),
    /// Position and size (section frames).
    Box(u64),
    /// Size only (the archive frame).
    Size(u64),
}

impl BoardCanvas {
    /// The transition a node has right now.
    fn motion(&self, node: &FlowNode) -> Motion {
        if node.dragging {
            return Motion::None;
        }
        match &node.kind {
            NodeKind::Card { stack, .. } => {
                if self.moving_archive || self.moving_section {
                    return Motion::None;
                }
                match stack {
                    Some(StackStyle {
                        place: StackPlace::Result { slot },
                        ..
                    }) => {
                        if self.stack_scrolling {
                            Motion::None
                        } else {
                            Motion::Position(620, (*slot).min(4) as u64 * 30, motion::ease_stack)
                        }
                    }
                    _ => Motion::Position(420, 0, motion::ease_out),
                }
            }
            NodeKind::Section { .. } => Motion::Box(420),
            NodeKind::Archive { .. } => Motion::Size(460),
            NodeKind::Note { .. } => {
                if self.moving_archive {
                    Motion::None
                } else {
                    Motion::Position(420, 0, motion::ease_out)
                }
            }
            NodeKind::Pad | NodeKind::View { .. } => Motion::None,
        }
    }

    /// Where a node is drawn this frame, gliding from where it was; and
    /// whether it's still moving.
    fn drawn(&mut self, node: &FlowNode, now: Instant, cx: &gpui::App) -> (Rect, bool) {
        let target = node.rect();
        let motion = self.motion(node);
        let previous = self.glides.get(&node.id).copied();
        let Some(previous) = previous else {
            self.glides.insert(node.id.clone(), still(target, now));
            return (target, false);
        };
        let current = previous.at(now);
        if matches!(motion, Motion::None) {
            self.glides.insert(node.id.clone(), still(target, now));
            return (target, false);
        }
        if previous.to != target {
            let (from, ms, delay, ease) = match motion {
                Motion::Position(ms, delay, ease) => (
                    Rect {
                        w: target.w,
                        h: target.h,
                        ..current
                    },
                    ms,
                    delay,
                    ease,
                ),
                Motion::Box(ms) => (current, ms, 0, motion::ease_out as fn(f32) -> f32),
                Motion::Size(ms) => (
                    Rect {
                        x: target.x,
                        y: target.y,
                        ..current
                    },
                    ms,
                    0,
                    motion::ease_out as fn(f32) -> f32,
                ),
                Motion::None => unreachable!(),
            };
            let glide = Glide {
                from,
                to: target,
                start: now,
                delay: motion::duration(cx, Duration::from_millis(delay)),
                duration: motion::duration(cx, Duration::from_millis(ms)),
                ease,
            };
            self.glides.insert(node.id.clone(), glide);
            return (glide.at(now), !glide.done(now));
        }
        (current, !previous.done(now))
    }

    /// A node's box on screen, with its contents.
    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &mut self,
        node: &FlowNode,
        drawn: Rect,
        board: &BoardCx,
        renderers: &Rc<dyn NodeRenderers>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let Some(snapshot) = self.board.clone() else {
            return div().into_any_element();
        };
        let vp = self.viewport;
        let z = vp.zoom as f32;
        let left = (vp.x + drawn.x * vp.zoom) as f32;
        let top = (vp.y + drawn.y * vp.zoom) as f32;
        let (w, h) = ((drawn.w * vp.zoom) as f32, (drawn.h * vp.zoom) as f32);
        let state = NodeState {
            id: node.id.clone().into(),
            size: size(drawn.w as f32, drawn.h as f32),
            selected: node.selected,
            dragging: node.dragging,
            zoom: z,
        };
        let id = node.id.clone();
        let element_id = ElementId::Name(format!("node-{id}").into());
        let content: AnyElement = match &node.kind {
            NodeKind::Archive { count } => renderers.archive(
                ArchiveNode {
                    node: state,
                    count: *count,
                },
                board,
                window,
                cx,
            ),
            NodeKind::Pad => renderers.pad(PadNode { node: state }, board, window, cx),
            NodeKind::View { index } => renderers.view(
                ViewNode {
                    node: state,
                    view: &snapshot.views[*index],
                },
                board,
                window,
                cx,
            ),
            NodeKind::Note { index } => {
                let draft;
                let note = match index {
                    Some(i) => &snapshot.notes[*i],
                    None => {
                        let spot = pad_sheet_spot(note_pad_rect(snapshot.archive));
                        draft = BoardNote {
                            id: id.clone(),
                            text: String::new(),
                            color: FolderColor::Yellow,
                            x: spot.x,
                            y: spot.y,
                            w: None,
                            h: None,
                            session_id: None,
                            from: None,
                            from_project: None,
                            from_title: None,
                            created_at: 0.0,
                            extra: serde_json::Map::new(),
                        };
                        &draft
                    }
                };
                renderers.note(
                    NoteNode {
                        node: state,
                        note,
                        draft: index.is_none(),
                    },
                    board,
                    window,
                    cx,
                )
            }
            NodeKind::Card { index, stack, matched } => {
                let card = &snapshot.cards[*index];
                let (_, stuck) = super::layout::split_notes(&snapshot.notes, &snapshot.cards);
                let stickers: Vec<BoardNote> = if card.node.archived {
                    Vec::new()
                } else {
                    stuck
                        .into_iter()
                        .find(|(sid, _)| sid == &id)
                        .map(|(_, notes)| notes.into_iter().cloned().collect())
                        .unwrap_or_default()
                };
                renderers.card(
                    CardNode {
                        node: state,
                        card,
                        matched: matched.as_deref(),
                        stickers: &stickers,
                        stack: *stack,
                    },
                    board,
                    window,
                    cx,
                )
            }
            NodeKind::Section { index, count, status } => {
                let props = SectionProps {
                    section: snapshot.sections[*index].clone(),
                    count: *count,
                    status: *status,
                    hot: self.hot_section.as_deref() == Some(id.as_str()),
                    zoom: z,
                    naming: self.naming.get(&id).cloned(),
                };
                self.render_section(&props, window, cx)
            }
        };
        let content = zoomed(z, content);

        // Sections take the pointer by their header only.
        if let NodeKind::Section { .. } = node.kind {
            let header_id = ElementId::Name(format!("section-header-{id}").into());
            let header = self
                .node_hit(div().id(header_id), &id, cx)
                .absolute()
                .left_0()
                .top_0()
                .w_full()
                .h(px(super::layout::SECTION_HEADER_H as f32 * z))
                .cursor(if node.dragging {
                    CursorStyle::ClosedHand
                } else {
                    CursorStyle::OpenHand
                });
            return div()
                .id(element_id)
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .child(content)
                .child(header)
                .into_any_element();
        }
        // Pile cards under the top one let the pointer through to the archive.
        let click_through = matches!(
            node.kind,
            NodeKind::Card {
                stack: Some(StackStyle {
                    place: StackPlace::Pile { depth: 1.. },
                    ..
                }),
                ..
            }
        );
        let wrapper = div()
            .id(element_id)
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(w))
            .h(px(h))
            // `grabbing` while dragged; otherwise `default` (board.css).
            .when(node.dragging, |d| d.cursor(CursorStyle::ClosedHand))
            .child(content);
        let wrapper = if click_through {
            wrapper.into_any_element()
        } else {
            self.node_hit(wrapper, &id, cx).into_any_element()
        };
        // BOARD-014: a result crossing the panel's edges is cut there.
        match node.clip {
            Some((cut_top, cut_bottom)) => {
                let bleed = super::scene::CLIP_BLEED as f32 * z;
                let (ct, cb) = (cut_top as f32 * z, cut_bottom as f32 * z);
                div()
                    .absolute()
                    .left(px(left - bleed))
                    .top(px(top + ct))
                    .w(px(w + 2.0 * bleed))
                    .h(px((h - ct - cb).max(0.0)))
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .left(px(bleed))
                            .top(px(-ct))
                            .w(px(w))
                            .h(px(h))
                            .child(wrapper),
                    )
                    .into_any_element()
            }
            None => wrapper,
        }
    }

    /// The pointer handlers every node's box (or section header) carries.
    fn node_hit(
        &self,
        el: gpui::Stateful<gpui::Div>,
        id: &str,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (left, middle, right) = (id.to_owned(), id.to_owned(), id.to_owned());
        el.occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.node_mouse_down(&left, e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.node_mouse_down(&middle, e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.release_keys();
                    this.node_menu(&right, e.position, cx);
                }),
            )
            .on_scroll(cx.listener(|this, e, _, cx| this.wheel(e, cx)))
            .on_pinch(cx.listener(|this, e, _, cx| this.pinch(e, cx)))
    }

    /// The post-marquee selection rectangle (`NodesSelection`), over the nodes.
    fn render_selection_rect(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        if !self.selection_rect {
            return None;
        }
        let rects: Vec<Rect> = self
            .nodes
            .iter()
            .filter(|n| n.selected && !n.hidden)
            .map(FlowNode::rect)
            .collect();
        if rects.is_empty() {
            return None;
        }
        let b = super::layout::bounds_of(&rects);
        let vp = self.viewport;
        Some(
            div()
                .id("nodes-selection")
                .absolute()
                .left(px((vp.x + b.x * vp.zoom) as f32))
                .top(px((vp.y + b.y * vp.zoom) as f32))
                .w(px((b.w * vp.zoom) as f32))
                .h(px((b.h * vp.zoom) as f32))
                .bg(rgba_mix(t.accent, 0.08))
                .border_1()
                .border_color(rgba_mix(t.accent, 0.55))
                .cursor(CursorStyle::OpenHand)
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, e: &MouseDownEvent, window, cx| this.selection_mouse_down(e, window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(|this, e: &MouseDownEvent, window, cx| this.selection_mouse_down(e, window, cx)),
                )
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_scroll(cx.listener(|this, e, _, cx| this.wheel(e, cx)))
                .into_any_element(),
        )
    }

    /// The marquee (z 6).
    fn render_marquee(&self, t: &Tokens) -> Option<AnyElement> {
        let (x, y, w, h) = self.marquee_rect()?;
        Some(
            div()
                .absolute()
                .left(px(x as f32))
                .top(px(y as f32))
                .w(px(w as f32))
                .h(px(h as f32))
                .bg(rgba_mix(t.accent, 0.08))
                .border_1()
                .border_color(rgba_mix(t.accent, 0.55))
                .into_any_element(),
        )
    }

    /// The flow rect the canvas shows.
    fn visible_rect(&self) -> Rect {
        let (w, h) = self.canvas_size();
        let vp = self.viewport;
        Rect {
            x: -vp.x / vp.zoom,
            y: -vp.y / vp.zoom,
            w: w / vp.zoom,
            h: h / vp.zoom,
        }
    }

    /// The edges, in canvas pixels: their Bézier, stroke, width and dashes
    /// (BOARD-025…BOARD-031).
    fn edge_paths(&self, t: &Tokens) -> Vec<EdgePaint> {
        let vp = self.viewport;
        let rect_of = |id: &str| self.node(id).filter(|n| !n.hidden).map(FlowNode::rect);
        let view = self.visible_rect();
        let only_visible = self.board.as_ref().is_some_and(|b| b.cards.len() > VISIBLE_ONLY_ABOVE);
        self.edges
            .iter()
            .filter_map(|e| {
                let (s, d) = (rect_of(&e.source)?, rect_of(&e.target)?);
                // React Flow's `isEdgeVisible`: the box around both ends.
                if only_visible && !overlaps(super::layout::bounds_of(&[s, d]), view) {
                    return None;
                }
                let pts = edges::path(s, d).map(|p| {
                    let (x, y) = super::viewport::to_screen(vp, p.x, p.y);
                    Point { x, y }
                });
                let z = vp.zoom;
                Some(if e.view {
                    EdgePaint {
                        pts,
                        color: rgba_mix(t.accent, 0.7),
                        width: 2.0 * z,
                        dash: Some((6.0 * z, 4.0 * z, 1000.0)),
                        round: true,
                    }
                } else if e.animated {
                    EdgePaint {
                        pts,
                        color: t.status_working,
                        width: 1.5 * z,
                        dash: Some((5.0 * z, 5.0 * z, 500.0)),
                        round: false,
                    }
                } else {
                    EdgePaint {
                        pts,
                        color: t.border_strong,
                        width: 1.5 * z,
                        dash: None,
                        round: false,
                    }
                })
            })
            .collect()
    }
}

/// Past this many cards the board draws only what's in view.
const VISIBLE_ONLY_ABOVE: usize = 150;

/// Two rects share some area (React Flow's partial overlap).
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// A glide that has arrived.
fn still(r: Rect, now: Instant) -> Glide {
    Glide {
        from: r,
        to: r,
        start: now,
        delay: Duration::ZERO,
        duration: Duration::ZERO,
        ease: motion::ease_out,
    }
}

/// One edge to paint.
struct EdgePaint {
    pts: [Point; 4],
    color: gpui::Rgba,
    width: f64,
    /// Dash on, off, and the ms one period of `dashdraw` takes.
    dash: Option<(f64, f64, f64)>,
    round: bool,
}

fn bezier(p: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point {
        x: a * p[0].x + b * p[1].x + c * p[2].x + d * p[3].x,
        y: a * p[0].y + b * p[1].y + c * p[2].y + d * p[3].y,
    }
}

/// Paints an edge; `phase` (0…1) is how far the dashes have marched.
fn paint_edge(edge: &EdgePaint, origin: gpui::Point<Pixels>, phase: f64, window: &mut Window) {
    let at = |p: Point| point(origin.x + px(p.x as f32), origin.y + px(p.y as f32));
    let width = px(edge.width as f32);
    let Some((on, off, _)) = edge.dash else {
        let mut path = PathBuilder::stroke(width);
        path.move_to(at(edge.pts[0]));
        path.cubic_bezier_to(at(edge.pts[3]), at(edge.pts[1]), at(edge.pts[2]));
        if let Ok(path) = path.build() {
            window.paint_path(path, edge.color);
        }
        return;
    };
    // Flatten, then cut into dashes along the length: `stroke-dashoffset`
    // runs 10 → 0 (in flow units, scaled) over a period.
    const STEPS: usize = 96;
    let pts: Vec<Point> = (0..=STEPS)
        .map(|i| bezier(&edge.pts, i as f64 / STEPS as f64))
        .collect();
    let period = on + off;
    let offset = period * (1.0 - phase);
    let mut path = PathBuilder::stroke(width);
    let mut ends: Vec<Point> = Vec::new();
    let mut s = 0.0;
    let mut drawing = false;
    for pair in pts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
        // Split this segment at dash boundaries.
        let mut t0 = 0.0;
        while t0 < 1.0 {
            let pos = (s + t0 * len + offset).rem_euclid(period);
            let in_dash = pos < on;
            let to_boundary = if in_dash { on - pos } else { period - pos };
            let t1 = if len > 0.0 {
                (t0 + to_boundary / len).min(1.0)
            } else {
                1.0
            };
            let p0 = Point {
                x: a.x + (b.x - a.x) * t0,
                y: a.y + (b.y - a.y) * t0,
            };
            let p1 = Point {
                x: a.x + (b.x - a.x) * t1,
                y: a.y + (b.y - a.y) * t1,
            };
            if in_dash {
                if !drawing {
                    path.move_to(at(p0));
                    ends.push(p0);
                    drawing = true;
                }
                path.line_to(at(p1));
                if t1 < 1.0 {
                    drawing = false;
                    ends.push(p1);
                }
            } else {
                drawing = false;
            }
            t0 = t1.max(t0 + 1e-6);
        }
        s += len;
    }
    if drawing && let Some(last) = pts.last() {
        ends.push(*last);
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, edge.color);
    }
    if edge.round {
        let r = edge.width / 2.0;
        for p in ends {
            let b = Bounds::new(
                at(Point { x: p.x - r, y: p.y - r }),
                size(px((2.0 * r) as f32), px((2.0 * r) as f32)),
            );
            window.paint_quad(fill(b, edge.color).corner_radii(px(r as f32)));
        }
    }
}

/// The dotted background (`Background` variant Dots, gap 22, size 1.6).
fn paint_dots(bounds: Bounds<Pixels>, vp: nodus_domain::Viewport, color: gpui::Rgba, window: &mut Window) {
    let gap = {
        let g = 22.0 * vp.zoom;
        if g == 0.0 { 1.0 } else { g }
    };
    let r = 0.8 * vp.zoom;
    let (w, h) = (
        f64::from(f32::from(bounds.size.width)),
        f64::from(f32::from(bounds.size.height)),
    );
    let cols = (w / gap).ceil() + 2.0;
    let rows = (h / gap).ceil() + 2.0;
    // Too many dots to see apart: Chromium's pattern is a faint wash then.
    if cols * rows > 40_000.0 {
        return;
    }
    // Pattern origin `x % gap`, shifted back half a gap; circles at (r, r).
    let first = |t: f64| (t - gap / 2.0 + r).rem_euclid(gap) - gap;
    let (x0, y0) = (first(vp.x), first(vp.y));
    let d = px((2.0 * r) as f32);
    let mut y = y0;
    while y <= h + gap {
        let mut x = x0;
        while x <= w + gap {
            let b = Bounds::new(
                point(
                    bounds.origin.x + px((x - r) as f32),
                    bounds.origin.y + px((y - r) as f32),
                ),
                size(d, d),
            );
            window.paint_quad(fill(b, color).corner_radii(px(r as f32)));
            x += gap;
        }
        y += gap;
    }
}

impl Render for BoardCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        if self.board.is_none() {
            return div().size_full().bg(t.bg_canvas).into_any_element();
        }
        if self.step_animation(cx) {
            window.request_animation_frame();
        }
        if self.auto_pan_step(cx) {
            window.request_animation_frame();
        }
        self.init_viewport(cx);
        self.take_open_request(cx);

        let board_cx = self.board_cx(cx);
        let renderers = renderers(cx);
        let now = Instant::now();
        let order = paint_order(&self.nodes);
        // `onlyRenderVisibleElements` above 150 cards: only what overlaps the
        // canvas is drawn (an expanded card off screen lets its terminal go).
        let view = self.visible_rect();
        let only_visible = self.board.as_ref().is_some_and(|b| b.cards.len() > VISIBLE_ONLY_ABOVE);
        let nodes: Vec<FlowNode> = order
            .iter()
            .map(|&i| &self.nodes[i])
            .filter(|n| !only_visible || overlaps(n.rect(), view))
            .cloned()
            .collect();
        let mut below = Vec::new();
        let mut above = Vec::new();
        let mut gliding = false;
        for node in &nodes {
            let (drawn, moving) = self.drawn(node, now, cx);
            gliding |= moving;
            let el = self.render_node(node, drawn, &board_cx, &renderers, window, cx);
            if node.z_drawn() < 0 {
                below.push(el);
            } else {
                above.push(el);
            }
        }
        if gliding {
            window.request_animation_frame();
        }
        self.glides.retain(|id, _| nodes.iter().any(|n| &n.id == id));

        let edge_paths = self.edge_paths(&t);
        let frozen = crate::parity::frozen(cx);
        let animated_edges = edge_paths.iter().any(|e| e.dash.is_some());
        if animated_edges && !frozen {
            window.request_animation_frame();
        }
        let elapsed = self.clock.elapsed().as_secs_f64() * 1000.0;

        let me = cx.entity();
        let vp = self.viewport;
        let dot = t.canvas_dot;
        let panning = matches!(self.gesture, Gesture::Pan { .. });
        let empty = self
            .board
            .as_ref()
            .is_some_and(|b| b.cards.iter().all(|c| c.node.archived));

        let background = canvas(
            {
                let me = me.clone();
                move |bounds, _, cx| {
                    me.update(cx, |c, cx| {
                        if c.bounds != bounds {
                            c.bounds = bounds;
                            cx.notify();
                        }
                    });
                }
            },
            move |bounds, _, window, _| {
                paint_dots(bounds, vp, dot, window);
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        m.update(cx, |c, cx| c.pointer_move(e, cx));
                    }
                });
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        m.update(cx, |c, cx| {
                            if c.gesture != Gesture::Idle {
                                c.pointer_up(e, window, cx);
                            }
                        });
                    }
                });
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.capture_mouse_down(e, cx));
                    }
                });
            },
        )
        .absolute()
        .size_full();

        // BOARD-028: edges are painted, never hit: the pointer goes to what's
        // under them.
        let edges_layer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for edge in &edge_paths {
                    let phase = match edge.dash {
                        Some((_, _, period)) if !frozen => (elapsed % period) / period,
                        _ => 0.0,
                    };
                    paint_edge(edge, bounds.origin, phase, window);
                }
            },
        )
        .absolute()
        .size_full();

        let selection = self.render_selection_rect(&t, cx);
        let marquee = self.render_marquee(&t);
        let pill = self.render_zoom_pill(window, cx);
        let minimap = self.render_minimap(cx);
        let hint = empty.then(|| self.render_hint(window, cx));
        let toast = self.render_toast(window, cx);
        let confirm = self.render_confirm(window, cx);

        div()
            .id("board-canvas")
            .key_context("Board")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(t.bg_canvas)
            .when(panning, |d| d.cursor(CursorStyle::ClosedHand))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| this.pane_mouse_down(e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseDownEvent, window, cx| this.pane_mouse_down(e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.release_keys();
                    this.pane_menu(e.position, cx);
                }),
            )
            .on_scroll(cx.listener(|this, e, _, cx| this.wheel(e, cx)))
            .on_pinch(cx.listener(|this, e, _, cx| this.pinch(e, cx)))
            .child(background)
            .children(below)
            .child(edges_layer)
            .children(above)
            .children(selection)
            .children(marquee)
            .children(hint)
            .child(pill)
            .child(minimap)
            .children(toast)
            .children(confirm)
            .into_any_element()
    }
}
