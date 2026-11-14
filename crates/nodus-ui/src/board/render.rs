//! Drawing the board, back to front (spec 09 § Rust mapping › Render
//! layers): the dotted background, the nodes below the edges (sections, the
//! archive, the pad), the edges, every other node by z-index, the selection
//! rectangle and the marquee, then the zoom pill, minimap, hint, toast and
//! confirm. Nodes glide to where they belong as their CSS transitions did
//! (§ Node motion).

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Bounds, Context, CursorStyle, DispatchPhase, ElementId, Entity, EntityId,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathBuilder, Pixels, Render, Styled, Window, canvas, div, fill, point, px, size,
};
use nodus_board::geometry::Point;
use nodus_domain::{BoardNote, BoardSnapshot, FolderColor, Rect};

use super::canvas::{BoardCanvas, Gesture, Glide};
use super::edges::{self, EdgeKind};
use super::hud::rgba_mix;
use super::layout::{note_pad_rect, pad_sheet_spot};
use super::moments::{self, Arrival};
use super::node_view::{self, NodeView};
use super::nodes::{
    ArchiveNode, BoardCx, CardNode, NodeRenderers, NodeState, NoteNode, PadNode, ShapeNode, StackPlace, StackStyle,
    ViewNode, renderers,
};
use super::scene::{FlowNode, NodeKind, paint_order};
use super::section::SectionProps;
use super::units::zoomed;
use crate::theme::{self, ai, motion, tokens::Tokens};

/// Whether a node may keep last frame's drawing.
#[derive(Clone, Copy)]
struct NodeCache {
    /// Nothing but the nodes changed since the last frame (`node_view`).
    partial: bool,
    /// The node is on the canvas.
    visible: bool,
}

/// Where a node is in a moment (spec 15): arriving, raised, ringed.
#[derive(Clone, Copy, Default)]
struct NodeMoment {
    arrival: Option<Arrival>,
    /// MOTION-013: board units up (negative) while a card raises its hand.
    lift: f32,
    /// MOTION-031: 0…1 through the ring round a card whose long run finished.
    ring: Option<f32>,
    /// MOTION-035, MOTION-044: 0…1 through the glow of a card a finished
    /// child's pulse, or a message, reached.
    news: Option<f32>,
}

impl NodeMoment {
    /// The node's box, moved for its moment: a pop (MOTION-010), a note or
    /// view Claude put down (MOTION-023), a card raised (MOTION-013).
    fn apply<E: Styled>(self, el: E, kind: &NodeKind, z: f32) -> E {
        let (mut ty, mut scale, mut turn, mut opacity) = (self.lift, 1.0, 0.0, 1.0);
        match (self.arrival, kind) {
            (Some(Arrival::Pop(p)), _) => (scale, opacity) = moments::pop_at(p),
            (Some(Arrival::Drawing(p)), NodeKind::Note { .. } | NodeKind::View { .. }) => {
                let (down, placed) = (motion::ease_out(p), motion::ease_spring(p));
                ty += -14.0 * (1.0 - down);
                scale = 1.08 - 0.08 * placed;
                opacity = motion::ease_out((p / 0.35).min(1.0));
                if matches!(kind, NodeKind::Note { .. }) {
                    turn = -4.0 * (1.0 - down);
                }
            }
            _ => {}
        }
        let mut el = el;
        if ty != 0.0 || scale != 1.0 || turn != 0.0 {
            el = el
                .transform_origin(0.5, 0.5)
                .translate(px(0.), px(ty * z))
                .rotate(gpui::radians(turn.to_radians()))
                .scale(scale);
        }
        if opacity < 1.0 {
            el = el.group_opacity(opacity.max(0.0));
        }
        el
    }
}

/// MOTION-031: Claude's colours once round a compact card (corners of 14),
/// `t` through the ring, for a node `w`×`h` px at `z`.
fn done_ring(t: f32, w: f32, h: f32, z: f32) -> AnyElement {
    let (angle, opacity) = moments::ring_at(t);
    card_glow(angle, opacity, w, h, z)
}

/// MOTION-035, MOTION-044: the edge of the card a pulse or a message
/// reached lighting up, then fading, `t` through it.
fn news_glow(t: f32, w: f32, h: f32, z: f32) -> AnyElement {
    card_glow(0.0, 0.9 * (1.0 - motion::ease_out(t)), w, h, z)
}

/// Claude's glow round a compact card, turned to `angle`.
fn card_glow(angle: f64, opacity: f32, w: f32, h: f32, z: f32) -> AnyElement {
    let out = 1.5 * z;
    div()
        .absolute()
        .left(px(-out))
        .top(px(-out))
        .w(px(w + 2.0 * out))
        .h(px(h + 2.0 * out))
        .group_opacity(opacity)
        .children(ai::glow_layers(angle, 14.0 * z + out, z))
        .into_any_element()
}

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
            // FRAME-013, FRAME-014: a frame follows the pointer as it's
            // resized, and one inside a frame being dragged goes with it.
            NodeKind::Section { .. } => {
                if self.moving_section || self.held.contains_key(&node.id) {
                    Motion::None
                } else {
                    Motion::Box(420)
                }
            }
            NodeKind::Archive { .. } => Motion::Size(460),
            NodeKind::Note { .. } => {
                if self.moving_archive || self.moving_section {
                    Motion::None
                } else {
                    Motion::Position(420, 0, motion::ease_out)
                }
            }
            NodeKind::Pad | NodeKind::View { .. } => Motion::None,
            // SHAPE-010: as notes, so ⌘Z and a session's moves glide.
            NodeKind::Shape { .. } => {
                if self.moving_section {
                    Motion::None
                } else {
                    Motion::Position(420, 0, motion::ease_out)
                }
            }
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

    /// Where a node is in its moment this frame (spec 15).
    fn node_moment(&self, node: &FlowNode, now: Instant) -> NodeMoment {
        if !self.moments.active() {
            return NodeMoment::default();
        }
        let active_card = matches!(node.kind, NodeKind::Card { stack: None, .. });
        let arrives = matches!(
            node.kind,
            NodeKind::Card { stack: None, .. }
                | NodeKind::Note { index: Some(_) }
                | NodeKind::View { .. }
                | NodeKind::Shape { .. }
        );
        NodeMoment {
            arrival: arrives.then(|| self.moments.arrival(&node.id, now)).flatten(),
            lift: if active_card {
                self.moments.lift(&node.id, now)
            } else {
                0.0
            },
            ring: active_card.then(|| self.moments.ring(&node.id, now)).flatten(),
            news: if active_card { self.news(&node.id, now) } else { None },
        }
    }

    /// MOTION-035, MOTION-044: how far through its glow a card is that a
    /// finished child's pulse just reached up a lineage edge, or a message
    /// from another session reached along the line between them.
    fn news(&self, card: &str, now: Instant) -> Option<f32> {
        let travel = moments::PULSE_TRAVEL_MS;
        let finished = self
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Lineage && e.source == card)
            .filter_map(|e| self.moments.pulse(&e.target, now));
        let messages = self
            .moments
            .messages(now)
            .filter(|(m, _)| {
                m.to == card
                    && self
                        .edges
                        .iter()
                        .any(|e| e.kind != EdgeKind::View && e.joins(&m.from, &m.to))
            })
            .map(|(_, ms)| ms);
        finished
            .chain(messages)
            .filter(|ms| *ms >= travel)
            .map(|ms| ((ms - travel) / (moments::PULSE_MS - travel)) as f32)
            .reduce(f32::min)
    }

    /// A node's box on screen, with its contents.
    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &mut self,
        node: &FlowNode,
        drawn: Rect,
        snapshot: &BoardSnapshot,
        board: &BoardCx,
        renderers: &Rc<dyn NodeRenderers>,
        cache: NodeCache,
        moment: NodeMoment,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
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
            NodeKind::Archive { count } => {
                let view = self.node_view(&id, renderers, cx);
                let changed = view.update(cx, |v, _| {
                    v.set_archive(
                        ArchiveNode {
                            node: state,
                            count: *count,
                        },
                        board,
                        cache.visible,
                    )
                });
                node_view::element(&view, changed || !cache.partial)
            }
            NodeKind::Pad => {
                let view = self.node_view(&id, renderers, cx);
                let changed = view.update(cx, |v, _| v.set_pad(PadNode { node: state }, board, cache.visible));
                node_view::element(&view, changed || !cache.partial)
            }
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
                let view = self.node_view(&id, renderers, cx);
                let changed = view.update(cx, |v, _| {
                    v.set_note(
                        NoteNode {
                            node: state,
                            note,
                            draft: index.is_none(),
                        },
                        board,
                        cache.visible,
                    )
                });
                node_view::element(&view, changed || !cache.partial)
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
                let view = self.node_view(&id, renderers, cx);
                let changed = view.update(cx, |v, _| {
                    v.set_card(
                        CardNode {
                            node: state,
                            card,
                            matched: matched.as_deref(),
                            stickers: &stickers,
                            stack: *stack,
                        },
                        board,
                        cache.visible,
                    )
                });
                node_view::element(&view, changed || !cache.partial)
            }
            NodeKind::Shape { index } => {
                let shape = &snapshot.shapes[*index];
                let image = shape.image.as_deref().and_then(|name| self.image_path(name, cx));
                renderers.shape(
                    ShapeNode {
                        node: state,
                        shape,
                        image,
                        drawing: match moment.arrival {
                            Some(Arrival::Drawing(p)) => Some(p),
                            _ => None,
                        },
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
        let content = zoomed(z, content).stepped_glyphs(self.zooming);

        // Frames take the pointer by their header, and their edges to
        // resize (FRAME-013); inside, it's the board's.
        if let NodeKind::Section { .. } = node.kind {
            let zones = if self.read_only() {
                Vec::new()
            } else {
                self.frame_resize_zones(&id, w, h, cx)
            };
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
                .children(zones)
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
            .children(moment.ring.map(|t| done_ring(t, w, h, z)))
            .children(moment.news.map(|t| news_glow(t, w, h, z)))
            .child(content);
        let wrapper = moment.apply(wrapper, &node.kind, z);
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

    /// A node's cached view (`node_view`), made the first frame it's drawn.
    fn node_view(
        &mut self,
        id: &str,
        renderers: &Rc<dyn NodeRenderers>,
        cx: &mut Context<'_, Self>,
    ) -> Entity<NodeView> {
        let view = self
            .node_views
            .entry(id.to_owned())
            .or_insert_with(|| cx.new(|_| NodeView::default()))
            .clone();
        view.update(cx, |v, _| v.set_renderers(renderers));
        view
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
        let now = Instant::now();
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
                let mut pulses = Vec::new();
                if e.kind == EdgeKind::Lineage
                    && let Some(ms) = self.moments.pulse(&e.target, now)
                {
                    pulses.push(Pulse { ms, backwards: true });
                }
                // MOTION-044: messages between its ends run along it; the line
                // between two that just started talking draws in behind the first.
                let mut span = (0.0, 1.0);
                if e.kind != EdgeKind::View {
                    for (m, ms) in self.moments.messages(now).filter(|(m, _)| e.joins(&m.from, &m.to)) {
                        let backwards = m.from == e.target;
                        pulses.push(Pulse { ms, backwards });
                        if m.opens && e.kind == EdgeKind::Talk && ms < moments::PULSE_TRAVEL_MS {
                            let head = f64::from(motion::ease_in_out((ms / moments::PULSE_TRAVEL_MS) as f32));
                            span = if backwards { (1.0 - head, 1.0) } else { (0.0, head) };
                        }
                    }
                }
                let (color, width, dash, round) = match e.kind {
                    EdgeKind::View => (rgba_mix(t.accent, 0.7), 2.0 * z, Some((6.0 * z, 4.0 * z, 1000.0)), true),
                    EdgeKind::Lineage if e.animated => {
                        (t.status_working, 1.5 * z, Some((5.0 * z, 5.0 * z, 500.0)), false)
                    }
                    EdgeKind::Lineage => (t.border_strong, 1.5 * z, None, false),
                    // BOARD-184: dotted, holding still.
                    EdgeKind::Talk => (t.border_strong, 1.5 * z, Some((1.0 * z, 4.5 * z, 0.0)), true),
                };
                Some(EdgePaint {
                    pts,
                    color,
                    width,
                    dash,
                    round,
                    span,
                    pulses,
                })
            })
            .collect()
    }
}

/// Past this many cards the board draws only what's in view.
const VISIBLE_ONLY_ABOVE: usize = 150;

/// Part of a Bézier is on the canvas (its control points hold the curve).
fn on_canvas(pts: &[Point; 4], w: f64, h: f64) -> bool {
    let (x0, x1) = pts
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y0, y1) = pts
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    x1 >= 0.0 && y1 >= 0.0 && x0 <= w && y0 <= h
}

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
    /// Dash on, off, and the ms one period of `dashdraw` takes (0: still).
    dash: Option<(f64, f64, f64)>,
    round: bool,
    /// The part of the curve drawn, 0…1 along it (MOTION-044 draws one in).
    span: (f64, f64),
    /// MOTION-035, MOTION-044: lights running along it.
    pulses: Vec<Pulse>,
}

/// A light running along an edge.
#[derive(Clone, Copy)]
struct Pulse {
    /// Since it set off.
    ms: f64,
    /// From the edge's end to its start: a finished child's news going up
    /// to its parent, or a message from the target.
    backwards: bool,
}

fn bezier(p: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point {
        x: a * p[0].x + b * p[1].x + c * p[2].x + d * p[3].x,
        y: a * p[0].y + b * p[1].y + c * p[2].y + d * p[3].y,
    }
}

/// The curve from `from` to `to` (0…1 along it) as a line through points.
fn flatten(p: &[Point; 4], from: f64, to: f64) -> Vec<Point> {
    const STEPS: f64 = 96.0;
    let steps = ((STEPS * (to - from)).ceil() as usize).max(1);
    (0..=steps)
        .map(|i| bezier(p, from + (to - from) * i as f64 / steps as f64))
        .collect()
}

/// A line's length.
fn length(pts: &[Point]) -> f64 {
    pts.windows(2)
        .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
        .sum()
}

/// Paints an edge; `phase` (0…1) is how far the dashes have marched.
fn paint_edge(edge: &EdgePaint, origin: gpui::Point<Pixels>, phase: f64, window: &mut Window) {
    let at = |p: Point| point(origin.x + px(p.x as f32), origin.y + px(p.y as f32));
    let width = px(edge.width as f32);
    let (from, to) = edge.span;
    if to <= from {
        return;
    }
    let Some((on, off, _)) = edge.dash else {
        let mut path = PathBuilder::stroke(width);
        if (from, to) == (0.0, 1.0) {
            path.move_to(at(edge.pts[0]));
            path.cubic_bezier_to(at(edge.pts[3]), at(edge.pts[1]), at(edge.pts[2]));
        } else {
            let pts = flatten(&edge.pts, from, to);
            path.move_to(at(pts[0]));
            for p in &pts[1..] {
                path.line_to(at(*p));
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, edge.color);
        }
        return;
    };
    // Flatten, then cut into dashes along the length: `stroke-dashoffset`
    // runs 10 → 0 (in flow units, scaled) over a period. Dashes keep their
    // places along the whole curve while only part of it is drawn.
    let pts = flatten(&edge.pts, from, to);
    let period = on + off;
    let offset = period * (1.0 - phase);
    let mut path = PathBuilder::stroke(width);
    let mut ends: Vec<Point> = Vec::new();
    let mut s = if from > 0.0 {
        length(&flatten(&edge.pts, 0.0, from))
    } else {
        0.0
    };
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

/// MOTION-035, MOTION-044: a light running along an edge in Claude's
/// colours, trailing a fading tail, then going out at the far end (whose
/// card glows): up a lineage edge from the session that finished (its end)
/// to its parent (its start), or from a session to one it messaged.
fn paint_pulse(edge: &EdgePaint, origin: gpui::Point<Pixels>, pulse: Pulse, zoom: f64, window: &mut Window) {
    let Pulse { ms, backwards } = pulse;
    let along = |t: f64| if backwards { 1.0 - t } else { t };
    let at = |p: Point| point(origin.x + px(p.x as f32), origin.y + px(p.y as f32));
    let dot = |c: Point, r: f64, color: gpui::Rgba, window: &mut Window| {
        let b = Bounds::new(
            at(Point { x: c.x - r, y: c.y - r }),
            size(px((2.0 * r) as f32), px((2.0 * r) as f32)),
        );
        window.paint_quad(fill(b, color).corner_radii(px(r as f32)));
    };
    let hue = ai::hue(ms / 1200.0);
    let travel = moments::PULSE_TRAVEL_MS;
    if ms < travel {
        let head = f64::from(motion::ease_in_out((ms / travel) as f32));
        for k in (0..8).rev() {
            let t = (head - f64::from(k) * 0.025).max(0.0);
            let share = 1.0 - f64::from(k) / 8.0;
            let c = bezier(&edge.pts, along(t));
            dot(
                c,
                (1.0 + 2.5 * share) * zoom,
                hue.with_alpha(0.55 * share as f32),
                window,
            );
        }
        let c = bezier(&edge.pts, along(head));
        dot(c, 8.0 * zoom, hue.with_alpha(0.22), window);
        dot(c, 3.5 * zoom, hue, window);
    } else {
        let t = ((ms - travel) / (moments::PULSE_MS - travel)).clamp(0.0, 1.0);
        let e = f64::from(motion::ease_out(t as f32));
        let end = if backwards { edge.pts[0] } else { edge.pts[3] };
        dot(end, 3.5 * zoom * (1.0 - e), hue, window);
    }
}

trait WithAlpha {
    fn with_alpha(self, a: f32) -> Self;
}

impl WithAlpha for gpui::Rgba {
    fn with_alpha(self, a: f32) -> Self {
        gpui::Rgba { a, ..self }
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
    // One quad whose shader draws every dot, so a frame costs the same at
    // any zoom instead of a quad per dot (tens of thousands zoomed out).
    window.paint_dot_grid(
        bounds,
        point(bounds.origin.x + px(x0 as f32), bounds.origin.y + px(y0 as f32)),
        px(gap as f32),
        px(r as f32),
        color,
    );
}

impl Render for BoardCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        if self.board.is_none() {
            return div().size_full().bg(t.bg_canvas).into_any_element();
        }
        // SRV-182: before the viewport steps, so a fresh move flies this frame.
        self.follow_step(cx);
        if self.step_animation(cx) {
            window.request_animation_frame();
        }
        if self.auto_pan_step(cx) {
            window.request_animation_frame();
        }
        // Spec 15: moments run at the display's rate, and only while they last.
        self.moments.tick(Instant::now());
        if self.moments.active() {
            window.request_animation_frame();
        }
        self.init_viewport(cx);
        // SRV-180: the viewport may have moved since the server last heard.
        self.share_presence(cx);
        self.take_open_request(cx);
        if let Some(id) = self.pending_link_label.take() {
            self.ink_open_link_label(id, window, cx);
        }

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
        // INKUI-025: ink goes over every node but the open archive panel's.
        let mut raised = Vec::new();
        let mut gliding = false;
        // Lent to the nodes for this loop rather than cloned for each one.
        let Some(snapshot) = self.board.take() else {
            return div().size_full().bg(t.bg_canvas).into_any_element();
        };
        let node_views: HashSet<EntityId> = self.node_views.values().map(Entity::entity_id).collect();
        // Others' pointers moving draw nothing on the nodes (SRV-181).
        let peers = self.state.read(cx).peers.entity_id();
        let partial = node_view::partial_frame(&node_views, &[peers], window, cx);
        for node in &nodes {
            let (drawn, moving) = self.drawn(node, now, cx);
            gliding |= moving;
            // MOTION-020: what Claude hasn't drawn yet isn't there.
            let moment = self.node_moment(node, now);
            if moment.arrival == Some(Arrival::Waiting) {
                continue;
            }
            let cache = NodeCache {
                partial,
                visible: overlaps(drawn, view),
            };
            let el = self.render_node(node, drawn, &snapshot, &board_cx, &renderers, cache, moment, window, cx);
            if node.z_drawn() < 0 {
                below.push(el);
            } else if node.z_drawn() >= super::overlays::INK_Z_BELOW {
                raised.push(el);
            } else {
                above.push(el);
            }
        }
        self.board = Some(snapshot);
        if gliding {
            window.request_animation_frame();
        }
        self.glides.retain(|id, _| nodes.iter().any(|n| &n.id == id));
        self.node_views.retain(|id, _| nodes.iter().any(|n| &n.id == id));

        let edge_paths = self.edge_paths(&t);
        let frozen = crate::parity::frozen(cx);
        // Marching dashes move on the ambient clock, while one is in view.
        let (cw, ch) = self.canvas_size();
        let marching = edge_paths
            .iter()
            .any(|e| e.dash.is_some_and(|(_, _, period)| period > 0.0) && on_canvas(&e.pts, cw, ch));
        if marching && !frozen {
            crate::components::ambient::request_frame(window, cx);
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
                        Some((_, _, period)) if !frozen && period > 0.0 => (elapsed % period) / period,
                        _ => 0.0,
                    };
                    paint_edge(edge, bounds.origin, phase, window);
                    for pulse in &edge.pulses {
                        paint_pulse(edge, bounds.origin, *pulse, vp.zoom, window);
                    }
                }
            },
        )
        .absolute()
        .size_full();

        let group_outlines = self.render_group_outlines(&t);
        let selection = self.render_selection_rect(&t, cx);
        let marquee = self.render_marquee(&t);
        let pill = self.render_zoom_pill(window, cx);
        let minimap = self.render_minimap(cx);
        // Its keys and bar aren't there on a read-only board (SRV-028).
        let hint = (empty && !self.read_only()).then(|| self.render_hint(window, cx));
        // BOARD-138: the toast is stacked above the prompt bar (spec 11).
        // SRV-028: nothing to add to a read-only board.
        let read_only = self.read_only();
        // A field or terminal on it lets go of the keyboard as it turns read-only.
        if read_only && !self.read_only_seen {
            window.focus(&self.focus, cx);
        }
        self.read_only_seen = read_only;
        let bar = (!read_only).then(|| self.render_bar(window, cx));
        let search = self.render_search(window, cx);
        let merge = self.render_merge(window, cx);
        let confirm = self.render_confirm(window, cx);
        let ink_input = self.render_ink_input(cx);
        let ink_layer = self.render_ink_layer(window, cx);
        // MOTION-024: Claude's pens, over what they draw.
        let pens = self.render_pens(Instant::now());
        let ink_hitbox = self.render_ink_hitbox(cx);
        // SRV-181, SRV-182: others' pointers, and whom this window follows.
        let cursors = self.render_cursors(window, cx);
        let following = self.render_following(cx);
        // SHAPE-008: pictures dropped from Finder go up where they land,
        // over the nodes too (which hide the board's own box from a drop).
        let drop_layer = div()
            .id("board-drop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_drop::<gpui::ExternalPaths>(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                let at = window.mouse_position();
                let at = this.bounds.contains(&at).then(|| this.to_flow(at));
                this.add_images(paths.paths().to_vec(), at, cx);
            }));
        let ink_dock = (!read_only).then(|| self.render_ink_dock(window, cx));
        let bubble = if read_only {
            Vec::new()
        } else {
            self.render_bubble(window, cx)
        };
        let read_only_cover = self.render_read_only_cover(cx);
        let reconnecting = self.render_reconnecting(cx);
        // Spec 04: views' webviews kept in step, their large window and the New view dialog.
        let views_layer = crate::views::board_layer(self, window, cx);

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
            // BOARD-178: Edit › Copy and Paste, with the board's keys.
            .on_action(cx.listener(|this, _: &gpui::component::input::Copy, _, cx| {
                if this.cover(false, cx).shortcuts() {
                    this.copy_notes(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &gpui::component::input::Paste, _, cx| {
                if this.cover(false, cx).shortcuts() {
                    this.paste_notes(cx);
                }
            }))
            .child(ink_input)
            .child(background)
            .children(below)
            .child(edges_layer)
            .children(above)
            .child(ink_layer)
            .children(raised)
            .children(pens)
            .children(group_outlines)
            .children(selection)
            .children(marquee)
            .children(cursors)
            .child(ink_hitbox)
            .child(drop_layer)
            .children(hint)
            .children(ink_dock)
            .children(read_only_cover)
            .children(following)
            .child(pill)
            .child(minimap)
            .children(bar)
            .children(bubble)
            .children(search)
            .children(merge)
            .children(confirm)
            .children(views_layer)
            .children(reconnecting)
            .into_any_element()
    }
}
