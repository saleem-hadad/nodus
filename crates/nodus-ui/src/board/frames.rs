//! Frames and groups on the board (spec 17), the canvas's side: what a frame
//! holds as drawn, moving and resizing one, gluing things into groups, and
//! the outline round a selected group. Frames are stored as sections; their
//! header and rename field are `section.rs`.

use gpui::{
    AnyElement, Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, Pixels,
    Point as GPoint, Styled, Window, div, px,
};
use nodus_board::frames::{FRAME_MIN_H, FRAME_MIN_W};
use nodus_board::geometry::{EXPANDED_W, NEW_CARD_H, Point, Size};
use nodus_domain::{BoardGroup, BoardNote, BoardSection, BoardSnapshot, NodePatch, Rect, ViewPatch, js};

use super::canvas::{BoardCanvas, Gesture};
use super::cards::present::{ResizeEdges, resize, rounded};
use super::hud::rgba_mix;
use super::layout::{SECTION_HEADER_H, SECTION_PAD, holds_centre, is_terminal, section_views};
use super::scene::{FlowNode, NodeKind};
use crate::theme::tokens::Tokens;

/// FRAME-005: a new empty frame holds one new session's card.
pub const NEW_FRAME_SIZE: Size = Size {
    w: EXPANDED_W + 2.0 * SECTION_PAD,
    h: NEW_CARD_H + SECTION_HEADER_H + SECTION_PAD,
};
/// How far round its things a selected group's outline runs, in board units.
const GROUP_OUTLINE_PAD: f64 = 8.0;
/// The grab zones along a frame's edges, and at its corners, in screen px.
const EDGE_PX: f32 = 10.0;
const CORNER_PX: f32 = 16.0;

/// A grab zone: the edges it moves, its box (x, y, w, h) and its cursor.
type Zone = (ResizeEdges, (f32, f32, f32, f32), CursorStyle);

/// GROUP-002: what can be glued into a group or framed: an active card, a
/// note on the board, a view or a shape, drawn.
pub(crate) fn groupable(n: &FlowNode) -> bool {
    !n.hidden
        && match n.kind {
            NodeKind::Card { stack, .. } => stack.is_none(),
            NodeKind::Note { index } => index.is_some(),
            NodeKind::View { .. } | NodeKind::Shape { .. } => true,
            NodeKind::Archive { .. } | NodeKind::Pad | NodeKind::Section { .. } => false,
        }
}

/// FRAME-030: whether a thing gives Claude something to name a frame
/// after: a Claude session with a title or a transcript, a note with words,
/// a titled view, a shape with words in it.
pub(crate) fn says_something(board: &BoardSnapshot, id: &str) -> bool {
    if let Some(c) = board.cards.iter().find(|c| c.node.session_id == id) {
        return !is_terminal(c) && (c.node.title.as_ref().is_some_and(|t| !t.is_empty()) || c.meta.is_some());
    }
    if let Some(n) = board.notes.iter().find(|n| n.id == id) {
        return !js::trim(&n.text).is_empty();
    }
    if let Some(v) = board.views.iter().find(|v| v.id == id) {
        return !js::trim(&v.title).is_empty();
    }
    board
        .shapes
        .iter()
        .find(|s| s.id == id)
        .is_some_and(|s| s.kind.has_text() && !js::trim(&s.text).is_empty())
}

impl BoardCanvas {
    // ---- groups ------------------------------------------------------------------------

    fn board_groups(&self) -> &[BoardGroup] {
        self.board.as_ref().map_or(&[], |b| b.groups.as_slice())
    }

    /// The group `id` is glued into, if any.
    pub(crate) fn group_of(&self, id: &str) -> Option<&BoardGroup> {
        self.board_groups().iter().find(|g| g.members.iter().any(|m| m == id))
    }

    /// A group's members that are drawn on the board now (an archived card
    /// isn't, nor a note stuck on a card).
    fn drawn_members<'a>(&'a self, group: &'a BoardGroup) -> impl Iterator<Item = &'a String> + 'a {
        group.members.iter().filter(|m| self.node(m).is_some_and(groupable))
    }

    /// GROUP-010: `ids` and whatever is glued to any of them, each once.
    pub(crate) fn with_groups(&self, ids: &[String]) -> Vec<String> {
        let mut out = ids.to_vec();
        for id in ids {
            if let Some(g) = self.group_of(id) {
                for m in self.drawn_members(g) {
                    if !out.contains(m) {
                        out.push(m.clone());
                    }
                }
            }
        }
        out
    }

    /// GROUP-010: the selection takes in whatever is glued to what's in it.
    pub(crate) fn select_groups(&mut self) {
        let selected: Vec<String> = self.nodes.iter().filter(|n| n.selected).map(|n| n.id.clone()).collect();
        let all = self.with_groups(&selected);
        if all.len() == selected.len() {
            return;
        }
        for n in &mut self.nodes {
            if all.contains(&n.id) && n.selectable && !n.hidden {
                n.selected = true;
            }
        }
    }

    /// GROUP-010: a thing let go of with ⇧ or ⌘ takes what's glued to it out
    /// of the selection too.
    pub(crate) fn deselect_group_of(&mut self, id: &str) {
        let all = self.with_groups(&[id.to_owned()]);
        for n in &mut self.nodes {
            if all.contains(&n.id) {
                n.selected = false;
            }
        }
    }

    /// The selected things a group or a frame takes.
    pub(crate) fn selected_things(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|n| n.selected && groupable(n))
            .map(|n| n.id.clone())
            .collect()
    }

    /// The group `ids` are, when they're exactly one group's drawn members.
    pub(crate) fn exactly_one_group(&self, ids: &[String]) -> Option<String> {
        let g = self.group_of(ids.first()?)?;
        let drawn: Vec<&String> = self.drawn_members(g).collect();
        (drawn.len() == ids.len() && ids.iter().all(|id| drawn.contains(&id))).then(|| g.id.clone())
    }

    /// Whether any of `ids` is glued into a group.
    pub(crate) fn any_grouped(&self, ids: &[String]) -> bool {
        ids.iter().any(|id| self.group_of(id).is_some())
    }

    /// GROUP-001: glues these things, and whatever was glued to them,
    /// together as one new group, selected. Fewer than two: nothing.
    pub fn group_things(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let members: Vec<String> = self
            .with_groups(ids)
            .into_iter()
            .filter(|id| self.node(id).is_some_and(groupable))
            .collect();
        if members.len() < 2 || self.exactly_one_group(&members).is_some() {
            return;
        }
        let group = BoardGroup {
            id: uuid::Uuid::new_v4().to_string(),
            members: members.clone(),
            created_at: crate::parity::now_ms(cx),
            extra: serde_json::Map::new(),
        };
        self.state.update(cx, |s, cx| s.save_groups(vec![group], cx));
        for n in &mut self.nodes {
            n.selected = members.contains(&n.id);
        }
        self.selection_rect = false;
        self.sync_selection(cx);
        cx.notify();
    }

    /// GROUP-003: the groups any of these things is in come apart; the
    /// things stay where they are, selected.
    pub fn ungroup_things(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let mut gone: Vec<String> = Vec::new();
        for id in ids {
            if let Some(g) = self.group_of(id)
                && !gone.contains(&g.id)
            {
                gone.push(g.id.clone());
            }
        }
        if gone.is_empty() {
            return;
        }
        self.state.update(cx, |s, cx| s.remove_groups(gone, cx));
        cx.notify();
    }

    /// GROUP-011: the dashed outline round each group something selected
    /// is in, round its things as they're drawn now.
    pub(crate) fn render_group_outlines(&self, t: &Tokens) -> Vec<AnyElement> {
        let vp = self.viewport;
        let mut out = Vec::new();
        for g in self.board_groups() {
            let rects: Vec<Rect> = self
                .drawn_members(g)
                .filter_map(|m| self.node(m))
                .map(FlowNode::rect)
                .collect();
            let selected = g.members.iter().any(|m| self.node(m).is_some_and(|n| n.selected));
            if rects.len() < 2 || !selected {
                continue;
            }
            let b = super::layout::bounds_of(&rects);
            let pad = GROUP_OUTLINE_PAD;
            out.push(
                div()
                    .absolute()
                    .left(px((vp.x + (b.x - pad) * vp.zoom) as f32))
                    .top(px((vp.y + (b.y - pad) * vp.zoom) as f32))
                    .w(px(((b.w + 2.0 * pad) * vp.zoom) as f32))
                    .h(px(((b.h + 2.0 * pad) * vp.zoom) as f32))
                    .rounded(px((10.0 * vp.zoom) as f32))
                    .border(px(1.5))
                    .border_dashed()
                    .border_color(rgba_mix(t.accent, 0.7))
                    .into_any_element(),
            );
        }
        out
    }

    // ---- frames ------------------------------------------------------------------------

    /// FRAME-012: what a frame holds as drawn now: the things whose centre is
    /// in it, the frames wholly inside it, and whatever is glued to them.
    pub(crate) fn frame_contents(&self, frame_id: &str) -> Vec<String> {
        let Some(frame) = self.node(frame_id).map(FlowNode::rect) else {
            return Vec::new();
        };
        let inside: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| n.id != frame_id)
            .filter(|n| match n.kind {
                NodeKind::Section { .. } => {
                    let r = n.rect();
                    r.x >= frame.x && r.y >= frame.y && r.x + r.w <= frame.x + frame.w && r.y + r.h <= frame.y + frame.h
                }
                _ => groupable(n) && holds_centre(frame, n.rect()),
            })
            .map(|n| n.id.clone())
            .collect();
        self.with_groups(&inside)
    }

    /// FRAME-003: a frame saved before frames had a size gets the size it's
    /// drawn at, round its cards, so it stays that size as they move.
    pub(crate) fn settle_frames(&mut self, cx: &mut Context<'_, Self>) {
        if self.read_only() {
            return;
        }
        let Some(board) = &self.board else { return };
        let settled: Vec<BoardSection> = section_views(&board.sections, &board.cards)
            .into_iter()
            .filter(|g| {
                (g.section.w.is_none() || g.section.h.is_none()) && !self.settled_frames.contains(&g.section.id)
            })
            .map(|g| BoardSection {
                x: g.frame.x,
                y: g.frame.y,
                w: Some(g.frame.w),
                h: Some(g.frame.h),
                ..g.section
            })
            .collect();
        if settled.is_empty() {
            return;
        }
        self.settled_frames.extend(settled.iter().map(|s| s.id.clone()));
        let state = self.state.clone();
        cx.defer(move |cx| {
            state.update(cx, |s, cx| {
                for section in settled {
                    s.save_section(section, cx);
                }
            });
        });
    }

    /// Things moved to these spots, saved each by its kind: cards in one
    /// patch, then notes, views, frames, and shapes as one change ⌘Z takes
    /// back (FRAME-014, FRAME-021).
    pub(crate) fn move_things(&mut self, moved: &[(String, Point)], cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.clone() else { return };
        let mut cards = Vec::new();
        let mut notes = Vec::new();
        let mut views = Vec::new();
        let mut frames = Vec::new();
        let mut shapes = Vec::new();
        for (id, to) in moved {
            let Some(n) = self.node(id) else { continue };
            let (x, y) = (js::round(to.x), js::round(to.y));
            match n.kind {
                NodeKind::Card { .. } => cards.push(NodePatch {
                    session_id: id.clone(),
                    x: Some(x),
                    y: Some(y),
                    ..NodePatch::default()
                }),
                NodeKind::Note { index: Some(i) } => notes.push(BoardNote {
                    x,
                    y,
                    ..board.notes[i].clone()
                }),
                NodeKind::View { .. } => views.push(ViewPatch {
                    id: id.clone(),
                    x: Some(x),
                    y: Some(y),
                    w: None,
                    h: None,
                    title: None,
                    starts_sessions: None,
                }),
                NodeKind::Section { index, .. } => frames.push(BoardSection {
                    x,
                    y,
                    w: Some(js::round(n.size.w)),
                    h: Some(js::round(n.size.h)),
                    ..board.sections[index].clone()
                }),
                NodeKind::Shape { .. } => shapes.push((id.clone(), Point { x, y })),
                NodeKind::Archive { .. } | NodeKind::Pad | NodeKind::Note { index: None } => {}
            }
        }
        self.state.update(cx, |s, cx| {
            if !cards.is_empty() {
                s.patch_cards(cards, cx);
            }
            for note in notes {
                s.save_note(note, cx);
            }
            for view in views {
                s.patch_view(view, cx);
            }
            for frame in frames {
                s.save_section(frame, cx);
            }
        });
        if !shapes.is_empty() {
            self.move_shapes(&shapes, cx);
        }
    }

    // ---- resizing a frame (FRAME-013) --------------------------------------------------

    /// The grab zones along a frame's edges and at its corners, over a frame
    /// drawn `w`×`h` screen px.
    pub(crate) fn frame_resize_zones(&self, id: &str, w: f32, h: f32, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let edge = |l: bool, r: bool, t: bool, b: bool| ResizeEdges {
            left: l,
            right: r,
            top: t,
            bottom: b,
        };
        let (e, c) = (EDGE_PX, CORNER_PX);
        // The top edge leaves the header to its drag; corners over edges.
        let zones: [Zone; 8] = [
            (
                edge(true, false, false, false),
                (-e / 2.0, 0.0, e, h),
                CursorStyle::ResizeLeftRight,
            ),
            (
                edge(false, true, false, false),
                (w - e / 2.0, 0.0, e, h),
                CursorStyle::ResizeLeftRight,
            ),
            (
                edge(false, false, true, false),
                (0.0, -e / 2.0, w, e / 2.0),
                CursorStyle::ResizeUpDown,
            ),
            (
                edge(false, false, false, true),
                (0.0, h - e / 2.0, w, e),
                CursorStyle::ResizeUpDown,
            ),
            (
                edge(true, false, true, false),
                (-c / 2.0, -c / 2.0, c, c),
                CursorStyle::ResizeUpLeftDownRight,
            ),
            (
                edge(false, true, true, false),
                (w - c / 2.0, -c / 2.0, c, c),
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                edge(true, false, false, true),
                (-c / 2.0, h - c / 2.0, c, c),
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                edge(false, true, false, true),
                (w - c / 2.0, h - c / 2.0, c, c),
                CursorStyle::ResizeUpLeftDownRight,
            ),
        ];
        zones
            .into_iter()
            .enumerate()
            .map(|(i, (edges, (x, y, zw, zh), cursor))| {
                let id = id.to_owned();
                div()
                    .id(gpui::ElementId::Name(format!("frame-resize-{id}-{i}").into()))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(zw))
                    .h(px(zh))
                    .cursor(cursor)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.start_frame_resize(&id, edges, e.position, window, cx);
                        }),
                    )
                    // No click handler: the zone follows the edge, so the
                    // release lands on it, and the board must see it to end
                    // the resize.
                    .on_scroll(cx.listener(|this, e, _, cx| this.wheel(e, cx)))
                    .into_any_element()
            })
            .collect()
    }

    fn start_frame_resize(
        &mut self,
        id: &str,
        edges: ResizeEdges,
        at: GPoint<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only() {
            return;
        }
        let Some(rect) = self.node(id).map(FlowNode::rect) else {
            return;
        };
        window.focus(&self.focus, cx);
        self.close_menu(cx);
        self.pointer = Some(at);
        self.gesture = Gesture::FrameResize {
            id: id.to_owned(),
            edges,
            start: at,
            rect,
        };
        cx.notify();
    }

    /// The frame's box with the pointer at `at`, never below its smallest.
    fn frame_resized(&self, edges: ResizeEdges, start: GPoint<Pixels>, rect: Rect, at: GPoint<Pixels>) -> Rect {
        let zoom = self.viewport.zoom.max(0.01) as f32;
        let dx = f32::from(at.x - start.x) / zoom;
        let dy = f32::from(at.y - start.y) / zoom;
        let (x, y, w, h) = resize(
            (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
            edges,
            dx,
            dy,
            (FRAME_MIN_W as f32, FRAME_MIN_H as f32),
        );
        Rect {
            x: f64::from(x),
            y: f64::from(y),
            w: f64::from(w),
            h: f64::from(h),
        }
    }

    /// The frame follows the pointer, shown live (`held`).
    pub(crate) fn frame_resize_move(
        &mut self,
        id: &str,
        edges: ResizeEdges,
        start: GPoint<Pixels>,
        rect: Rect,
        at: GPoint<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let next = self.frame_resized(edges, start, rect, at);
        if self.held.get(id) != Some(&next) {
            self.held.insert(id.to_owned(), next);
            self.rebuild(cx);
        }
    }

    /// Let go: the frame keeps its new box, rounded. What's in it doesn't move.
    pub(crate) fn end_frame_resize(
        &mut self,
        id: &str,
        edges: ResizeEdges,
        start: GPoint<Pixels>,
        rect: Rect,
        at: GPoint<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let r = self.frame_resized(edges, start, rect, at);
        let (x, y, w, h) = rounded((r.x as f32, r.y as f32, r.w as f32, r.h as f32));
        let section = self
            .board
            .as_ref()
            .and_then(|b| b.sections.iter().find(|s| s.id == id).cloned());
        if let Some(section) = section
            && (x, y, w, h) != (rect.x, rect.y, rect.w, rect.h)
        {
            self.state.update(cx, |s, cx| {
                s.save_section(
                    BoardSection {
                        x,
                        y,
                        w: Some(w),
                        h: Some(h),
                        ..section
                    },
                    cx,
                );
            });
        }
        self.held.remove(id);
        self.rebuild(cx);
    }
}
