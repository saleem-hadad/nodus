//! The pointer, the wheel and the keyboard on the board: React Flow's
//! d3-drag, d3-zoom and pane handlers, and BoardView.tsx's node and pane
//! handlers and key listener (spec 09, Canvas mechanics; BOARD-038…BOARD-066,
//! BOARD-074…BOARD-077, BOARD-156…BOARD-171).

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::base::TextSelection;
use gpui::{
    Context, KeyDownEvent, KeyUpEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PinchEvent, Pixels, Point as GPoint, ScrollDelta, ScrollEvent, Window,
};
use nodus_board::geometry::Point;
use nodus_domain::{MenuCommand, NodePatch, Rect, SessionKind, js};

use super::canvas::{BoardCanvas, DragState, Gesture, SectionDrag};
use super::layout::{
    ARCHIVE_ID, NOTE_SIZE, NOTEPAD_ID, SECTION_HEADER_H, card_size, contains_point, is_expanded, is_in_archive,
    is_on_pad, is_terminal, pad_sheet_spot, section_at, section_views,
};
use super::nodes::StackPlace;
use super::scene::{FlowNode, NodeKind, paint_order};
use super::viewport;
use crate::platform::Platform;

/// `nodeDragThreshold` and `paneClickDistance`, screen px.
const DRAG_THRESHOLD: f32 = 1.0;
/// The archive frame's drag handle (`.archive-header`).
pub(crate) const ARCHIVE_HEADER_H: f64 = 52.0;
/// A view's drag handle (`.view-head`): the caption above its page.
pub(crate) const VIEW_HEAD_H: f64 = crate::views::card::CAPTION_H as f64;

/// `multiSelectionKeyCode = ['Meta', 'Shift']`: one of them held alone.
pub(crate) fn multi_select(m: &Modifiers) -> bool {
    (m.shift ^ m.platform) && !m.control && !m.alt
}

fn distance(a: GPoint<Pixels>, b: GPoint<Pixels>) -> f32 {
    let dx = f32::from(a.x - b.x);
    let dy = f32::from(a.y - b.y);
    (dx * dx + dy * dy).sqrt()
}

/// What is over the board when a key or a menu command comes: the session
/// window (`modal`), the transcript preview, the agents viewer, the diff, one
/// of the board's overlays, or a field (a terminal too) with the keyboard.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Cover {
    pub modal: bool,
    pub preview: bool,
    pub agents: bool,
    pub diff: bool,
    pub overlay: bool,
    pub typing: bool,
}

impl Cover {
    /// BOARD-157, MODAL-031: ⌘K opens search, from the preview too (it
    /// closes, TRANS-024), but not over the session window, the agents
    /// viewer, the diff or an overlay, nor from a field.
    pub(crate) fn search_key(self) -> bool {
        !(self.typing || self.modal || self.agents || self.diff || self.overlay)
    }

    /// BOARD-158, MODAL-031: the board's other shortcuts, with nothing over it.
    pub(crate) fn shortcuts(self) -> bool {
        !(self.modal || self.preview || self.agents || self.diff || self.overlay || self.typing)
    }

    /// BOARD-171, MODAL-031: the menu's board commands, but not under an
    /// overlay, the agents viewer or the diff; Search closes the session
    /// window and the preview.
    pub(crate) fn menu(self) -> bool {
        !(self.overlay || self.agents || self.diff)
    }
}

/// Whether the last scroll the app saw came from a mouse's wheel. A page's
/// relayed wheel (VIEW-047) can't say, since WebKit reports every wheel in
/// pixels.
static LAST_NOTCHED: AtomicBool = AtomicBool::new(false);

/// Notes what made a scroll the app saw: a mouse's wheel (`notched`), or a
/// trackpad or Magic Mouse.
pub(crate) fn saw_scroll(notched: bool) {
    LAST_NOTCHED.store(notched, Ordering::Relaxed);
}

/// The DOM wheel delta (`deltaX`, `deltaY`, `deltaMode` 0) Chromium gave
/// for a GPUI scroll.
fn dom_delta(delta: ScrollDelta) -> (f64, f64) {
    match delta {
        ScrollDelta::Pixels(d) => (-f64::from(f32::from(d.x)), -f64::from(f32::from(d.y))),
        ScrollDelta::Lines(l) => {
            // Chromium: 40px a Cocoa tick on a Mac, 100px a notch (3 lines) on Windows.
            let per_line = if Platform::CURRENT.is_mac() { 40.0 } else { 100.0 / 3.0 };
            (-f64::from(l.x) * per_line, -f64::from(l.y) * per_line)
        }
    }
}

impl BoardCanvas {
    /// The node drawn on top at a flow point, by the canvas's pointer
    /// rules: sections only by their header, pile cards under the top one
    /// click-through, hidden nodes never, edges never.
    pub(crate) fn node_at(&self, p: Point) -> Option<&FlowNode> {
        paint_order(&self.nodes)
            .into_iter()
            .rev()
            .map(|i| &self.nodes[i])
            .find(|n| {
                let r = n.rect();
                match &n.kind {
                    NodeKind::Section { .. } => contains_point(
                        Rect {
                            h: SECTION_HEADER_H,
                            ..r
                        },
                        p,
                    ),
                    NodeKind::Card {
                        stack:
                            Some(super::nodes::StackStyle {
                                place: StackPlace::Pile { depth },
                                ..
                            }),
                        ..
                    } if *depth >= 1 => false,
                    _ => contains_point(r, p),
                }
            })
    }

    /// Whether a press on a node at a window point is on its drag handle.
    pub(crate) fn on_handle(&self, node: &FlowNode, at: GPoint<Pixels>) -> bool {
        let p = self.to_flow(at);
        match node.kind {
            NodeKind::Archive { .. } => p.y - node.position.y <= ARCHIVE_HEADER_H,
            NodeKind::Section { .. } => p.y - node.position.y <= SECTION_HEADER_H,
            NodeKind::View { .. } => p.y - node.position.y <= VIEW_HEAD_H,
            _ => true,
        }
    }

    fn focus_board(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.focus(&self.focus, cx);
    }

    // ---- presses --------------------------------------------------------------------

    /// A press on a node's box (whatever the contents didn't take).
    pub(crate) fn node_mouse_down(
        &mut self,
        id: &str,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let title = self.title_pressed.replace(false);
        self.pointer = Some(e.position);
        match e.button {
            MouseButton::Middle => {
                // Middle-drag pans from anywhere, over nodes too.
                self.gesture = Gesture::Pan { last: e.position };
                self.move_started(cx);
            }
            MouseButton::Left => {
                // CHAT-009: a press on a chat's text selects it; the card stays put.
                if TextSelection::is_selectable_at(e.position, window, cx) {
                    return;
                }
                self.focus_board(window, cx);
                let Some(node) = self.node(id) else { return };
                let handle = self.on_handle(node, e.position);
                let m = e.modifiers;
                self.gesture = Gesture::Press {
                    node: Some(id.to_owned()),
                    start: e.position,
                    moved: false,
                    handle,
                    title,
                    click_count: e.click_count,
                    shift: m.shift,
                    meta: m.platform,
                    ctrl: m.control,
                    multi: multi_select(&m),
                };
            }
            _ => {}
        }
        cx.notify();
    }

    /// A press on the post-marquee selection rectangle.
    pub(crate) fn selection_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.pointer = Some(e.position);
        match e.button {
            MouseButton::Middle => {
                self.gesture = Gesture::Pan { last: e.position };
                self.move_started(cx);
            }
            MouseButton::Left => {
                self.focus_board(window, cx);
                let m = e.modifiers;
                self.gesture = Gesture::Press {
                    node: None,
                    start: e.position,
                    moved: false,
                    handle: true,
                    title: false,
                    click_count: e.click_count,
                    shift: m.shift,
                    meta: m.platform,
                    ctrl: m.control,
                    multi: multi_select(&m),
                };
            }
            _ => {}
        }
    }

    /// A press on the empty pane (or inside a section's frame).
    pub(crate) fn pane_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        // FRAME-013: a frame's edge under the press took it first.
        if matches!(self.gesture, Gesture::FrameResize { .. }) {
            return;
        }
        self.pointer = Some(e.position);
        match e.button {
            MouseButton::Middle => {
                self.gesture = Gesture::Pan { last: e.position };
                self.move_started(cx);
            }
            MouseButton::Left => {
                self.focus_board(window, cx);
                if self.space {
                    self.gesture = Gesture::Pan { last: e.position };
                    self.move_started(cx);
                } else {
                    self.gesture = Gesture::PanePress {
                        start: e.position,
                        click_count: e.click_count,
                    };
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// BOARD-076: a press anywhere on the board but the archive and its
    /// cards folds the open stack (capture phase).
    pub(crate) fn capture_mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<'_, Self>) {
        if !self.stack_open() || !self.bounds.contains(&e.position) {
            return;
        }
        let p = self.to_flow(e.position);
        let keep = self.node_at(p).is_some_and(|n| {
            n.id == ARCHIVE_ID
                || n.card_index()
                    .and_then(|i| self.board.as_ref().map(|b| b.cards[i].node.archived))
                    .unwrap_or(false)
        });
        if !keep {
            self.close_stack(cx);
        }
    }

    // ---- moves ------------------------------------------------------------------------

    /// Every pointer move over the window.
    pub(crate) fn pointer_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<'_, Self>) {
        self.pointer = Some(e.position);
        // SRV-180: where the pointer is, for the others on a server board.
        self.share_presence(cx);
        // BOARD-074: hovering the stack, with no button down.
        if e.pressed_button.is_none() {
            if self.bounds.contains(&e.position) {
                let over = contains_point(self.zone(), self.to_flow(e.position));
                if over != self.stack_hover || (!over && self.stack_dismissed) {
                    self.stack_hover = over;
                    if !over {
                        self.stack_dismissed = false;
                    }
                    cx.notify();
                }
            } else if self.stack_hover || self.stack_dismissed {
                self.stack_hover = false;
                self.stack_dismissed = false;
                cx.notify();
            }
        }
        match self.gesture.clone() {
            Gesture::Idle => {}
            Gesture::MinimapPan { last } => {
                self.stop_following(cx);
                self.minimap_drag(last, e.position, cx);
                self.gesture = Gesture::MinimapPan { last: e.position };
            }
            Gesture::Pan { last } => {
                self.stop_following(cx);
                let dx = f64::from(f32::from(e.position.x - last.x));
                let dy = f64::from(f32::from(e.position.y - last.y));
                self.viewport.x += dx;
                self.viewport.y += dy;
                self.gesture = Gesture::Pan { last: e.position };
                cx.notify();
            }
            Gesture::PanePress { start, .. } => {
                if distance(start, e.position) > DRAG_THRESHOLD {
                    self.start_marquee(start, e.position, cx);
                }
            }
            Gesture::Marquee { from, .. } => {
                self.gesture = Gesture::Marquee {
                    from,
                    pointer: e.position,
                };
                self.update_marquee(cx);
            }
            Gesture::Press {
                node,
                start,
                moved,
                handle,
                multi,
                ..
            } => {
                let d = distance(start, e.position);
                if d > 0.0
                    && !moved
                    && let Gesture::Press { moved, .. } = &mut self.gesture
                {
                    *moved = true;
                }
                if d > DRAG_THRESHOLD && handle {
                    let draggable = node
                        .as_deref()
                        .is_none_or(|id| self.node(id).is_some_and(|n| n.draggable));
                    if draggable {
                        self.start_drag(node, multi, e.position, cx);
                    }
                }
            }
            Gesture::Drag(mut drag) => {
                drag.pointer = e.position;
                let p = self.to_flow(e.position);
                drag.last = p;
                self.gesture = Gesture::Drag(drag.clone());
                self.move_dragged(&drag, cx);
            }
            Gesture::FrameResize { id, edges, start, rect } => {
                self.frame_resize_move(&id, edges, start, rect, e.position, cx);
            }
        }
    }

    /// The pointer let go anywhere.
    pub(crate) fn pointer_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::Idle);
        match gesture {
            Gesture::Idle => {}
            Gesture::Pan { .. } | Gesture::MinimapPan { .. } => self.move_ends_in(150, cx),
            Gesture::PanePress { click_count, .. } if e.button == MouseButton::Left => {
                self.pane_click(e.position, click_count, cx);
            }
            Gesture::Marquee { .. } => {
                // React Flow swallows the click that ends a marquee.
                if self.nodes.iter().any(|n| n.selected) {
                    self.selection_rect = true;
                }
            }
            Gesture::Press {
                node,
                moved,
                title,
                click_count,
                shift,
                meta,
                ctrl,
                multi,
                ..
            } if e.button == MouseButton::Left => {
                // `nodeClickDistance = 0`: any movement means no click.
                if !moved && let Some(id) = node {
                    self.node_click(&id, click_count, title, shift || meta || ctrl, multi, window, cx);
                    if click_count == 2 {
                        self.node_double_click(&id, title, cx);
                    }
                }
            }
            Gesture::Drag(drag) => self.end_drag(&drag, cx),
            Gesture::FrameResize { id, edges, start, rect } => {
                self.end_frame_resize(&id, edges, start, rect, e.position, cx);
            }
            other => self.gesture = other,
        }
        cx.notify();
    }

    // ---- clicks -----------------------------------------------------------------------

    /// `handleNodeClick`: select it (or add it), or with a multi-select key
    /// held deselect it; what's glued to it comes too (GROUP-010).
    fn handle_node_click(&mut self, id: &str, multi: bool, cx: &mut Context<'_, Self>) {
        self.selection_rect = false;
        let Some(selected) = self.node(id).map(|n| n.selected) else {
            return;
        };
        if !selected {
            let group = self.with_groups(&[id.to_owned()]);
            for n in &mut self.nodes {
                if group.contains(&n.id) {
                    n.selected = n.selectable || n.id == id;
                } else if !multi {
                    n.selected = false;
                }
            }
        } else if multi {
            self.deselect_group_of(id);
        }
        self.sync_selection(cx);
    }

    /// A click on a node: React Flow's selection, then `onNodeClick`
    /// (BOARD-038…BOARD-044).
    #[allow(clippy::too_many_arguments)]
    fn node_click(
        &mut self,
        id: &str,
        click_count: usize,
        title: bool,
        modifier: bool,
        multi: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(node) = self.node(id).cloned() else { return };
        if node.selectable {
            self.handle_node_click(id, multi, cx);
        } else {
            self.selection_rect = false;
        }
        // BOARD-038.
        if node.is_draft() {
            if click_count == 1 {
                self.note_beside_pad(cx);
            }
            return;
        }
        let NodeKind::Card { index, stack, .. } = &node.kind else {
            return;
        };
        let Some(card) = self.board.as_ref().map(|b| b.cards[*index].clone()) else {
            return;
        };
        // BOARD-039: the pile opens.
        if card.node.archived
            && !matches!(
                stack,
                Some(super::nodes::StackStyle {
                    place: StackPlace::Result { .. },
                    ..
                })
            )
        {
            self.open_stack(cx);
            return;
        }
        // BOARD-040, BOARD-041.
        if modifier || self.renaming.as_deref() == Some(id) {
            return;
        }
        self.title_click = None;
        // BOARD-042: a click on an expanded card focuses its terminal, like
        // clicking a window.
        if is_expanded(&card) {
            super::nodes::renderers(cx).focus_terminal(id, window, cx);
            return;
        }
        // BOARD-043.
        if title {
            self.open_after_title_click(id.to_owned(), cx);
        } else {
            self.open_card(id, cx);
        }
    }

    /// `onNodeDoubleClick` (BOARD-045…BOARD-047).
    fn node_double_click(&mut self, id: &str, title: bool, cx: &mut Context<'_, Self>) {
        let Some(node) = self.node(id).cloned() else { return };
        match &node.kind {
            NodeKind::Section { .. } => self.set_renaming(Some(id.to_owned()), cx),
            // BOARD-046: a free note opens for writing; the top sheet doesn't.
            NodeKind::Note { index } => {
                if index.is_some() {
                    self.set_renaming(Some(id.to_owned()), cx);
                }
            }
            // SHAPE-012: a double-click writes in a shape that holds words.
            NodeKind::Shape { index } => {
                if self.board.as_ref().is_some_and(|b| b.shapes[*index].kind.has_text()) {
                    self.set_renaming(Some(id.to_owned()), cx);
                }
            }
            NodeKind::Card { index, stack, .. } => {
                let archived = self.board.as_ref().is_some_and(|b| b.cards[*index].node.archived);
                let result = matches!(
                    stack,
                    Some(super::nodes::StackStyle {
                        place: StackPlace::Result { .. },
                        ..
                    })
                );
                if archived && !result {
                    return;
                }
                if title {
                    self.title_click = None;
                    self.set_renaming(Some(id.to_owned()), cx);
                }
            }
            _ => {}
        }
    }

    /// A click on the pane (BOARD-049): deselects everything, closes the menu;
    /// a double-click starts a session there (BOARD-048).
    fn pane_click(&mut self, at: GPoint<Pixels>, click_count: usize, cx: &mut Context<'_, Self>) {
        self.close_menu(cx);
        self.clear_selection(cx);
        if click_count == 2 {
            let p = self.to_flow(at);
            if !self.in_dock(p) {
                self.create_session_centred(p, SessionKind::Claude, cx);
            }
        }
    }

    // ---- the marquee ----------------------------------------------------------------------

    fn start_marquee(&mut self, start: GPoint<Pixels>, pointer: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        // A marquee always replaces the selection.
        for n in &mut self.nodes {
            n.selected = false;
        }
        self.selection_rect = false;
        self.close_menu(cx);
        self.gesture = Gesture::Marquee {
            from: self.to_flow(start),
            pointer,
        };
        self.update_marquee(cx);
    }

    /// `SelectionMode.Partial`: every selectable node the marquee overlaps.
    pub(crate) fn update_marquee(&mut self, cx: &mut Context<'_, Self>) {
        let Gesture::Marquee { from, pointer } = self.gesture.clone() else {
            return;
        };
        let to = self.to_flow(pointer);
        let (x0, x1) = (from.x.min(to.x), from.x.max(to.x));
        let (y0, y1) = (from.y.min(to.y), from.y.max(to.y));
        for n in &mut self.nodes {
            if !n.selectable || n.hidden {
                continue;
            }
            let r = n.rect();
            let w = (x1.min(r.x + r.w) - x0.max(r.x)).max(0.0);
            let h = (y1.min(r.y + r.h) - y0.max(r.y)).max(0.0);
            n.selected = w * h > 0.0;
        }
        self.select_groups();
        self.sync_selection(cx);
        cx.notify();
    }

    /// The marquee's rect in canvas pixels.
    pub(crate) fn marquee_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let Gesture::Marquee { from, pointer } = &self.gesture else {
            return None;
        };
        let (ax, ay) = viewport::to_screen(self.viewport, from.x, from.y);
        let (bx, by) = self.local(*pointer);
        Some((ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs()))
    }

    // ---- dragging ---------------------------------------------------------------------------

    /// XYDrag's `startDrag`: selection first, then what moves.
    fn start_drag(&mut self, node: Option<String>, multi: bool, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        if let Some(id) = &node {
            let Some(n) = self.node(id).cloned() else { return };
            if !n.selectable && !multi && !n.selected {
                for other in &mut self.nodes {
                    other.selected = false;
                }
            }
            if n.selectable {
                self.handle_node_click(id, multi, cx);
            }
        }
        let p = self.to_flow(at);
        let items: Vec<(String, Point)> = self
            .nodes
            .iter()
            .filter(|n| (n.selected || Some(&n.id) == node.as_ref()) && n.draggable && !n.hidden)
            .map(|n| {
                (
                    n.id.clone(),
                    Point {
                        x: p.x - n.position.x,
                        y: p.y - n.position.y,
                    },
                )
            })
            .collect();
        if items.is_empty() {
            self.gesture = Gesture::Idle;
            return;
        }
        for n in &mut self.nodes {
            if items.iter().any(|(id, _)| id == &n.id) {
                n.dragging = true;
            }
        }
        let lead = node.clone().unwrap_or_else(|| items[0].0.clone());
        let drag = DragState {
            node,
            items,
            last: p,
            pointer: at,
        };
        self.gesture = Gesture::Drag(drag.clone());
        self.on_drag_start(&lead, &drag, cx);
        cx.notify();
    }

    /// Positions the dragged nodes under the pointer (`updateNodes`).
    fn move_dragged(&mut self, drag: &DragState, cx: &mut Context<'_, Self>) {
        let mut changed = false;
        for (id, offset) in &drag.items {
            let to = Point {
                x: drag.last.x - offset.x,
                y: drag.last.y - offset.y,
            };
            if let Some(n) = self.node_mut(id)
                && n.position != to
            {
                n.position = to;
                changed = true;
            }
        }
        if changed {
            let lead = drag.node.clone().unwrap_or_else(|| drag.items[0].0.clone());
            self.on_drag(&lead, drag, cx);
            cx.notify();
        }
    }

    /// The auto-pan near the edges while dragging nodes or a marquee
    /// (`calcAutoPan`, 15px a frame within 40px). True while it pans.
    pub(crate) fn auto_pan_step(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let pointer = match &self.gesture {
            Gesture::Drag(d) => d.pointer,
            Gesture::Marquee { pointer, .. } => *pointer,
            _ => return false,
        };
        let (x, y) = self.local(pointer);
        let (w, h) = self.canvas_size();
        let (dx, dy) = viewport::auto_pan(x, y, w, h);
        if dx == 0.0 && dy == 0.0 {
            return false;
        }
        self.stop_following(cx);
        self.viewport.x += dx;
        self.viewport.y += dy;
        match self.gesture.clone() {
            Gesture::Drag(mut drag) => {
                drag.last.x -= dx / self.viewport.zoom;
                drag.last.y -= dy / self.viewport.zoom;
                self.gesture = Gesture::Drag(drag.clone());
                self.move_dragged(&drag, cx);
            }
            Gesture::Marquee { .. } => self.update_marquee(cx),
            _ => {}
        }
        true
    }

    /// `onNodeDragStart` (BOARD-050, BOARD-051, BOARD-053, BOARD-055).
    fn on_drag_start(&mut self, lead: &str, drag: &DragState, cx: &mut Context<'_, Self>) {
        self.close_menu(cx);
        self.title_click = None;
        let Some(node) = self.node(lead).cloned() else { return };
        if lead == ARCHIVE_ID {
            self.archive_busy = true;
            self.moving_archive = true;
            self.last_archive_pos = Some(node.position);
            if let Some(board) = &self.board {
                for c in board.cards.iter().filter(|c| c.node.archived) {
                    self.busy.insert(c.node.session_id.clone());
                }
            }
            self.busy.insert(NOTEPAD_ID.to_owned());
            self.busy.insert(self.draft_id.clone());
            return;
        }
        // FRAME-012: a frame takes what’s in it along.
        if let NodeKind::Section { .. } = node.kind {
            let members = self.frame_contents(lead);
            self.busy.insert(lead.to_owned());
            for m in &members {
                self.busy.insert(m.clone());
            }
            self.section_drag = Some(SectionDrag {
                last: node.position,
                members,
            });
            self.moving_section = true;
            return;
        }
        self.drag_start.clear();
        for (id, _) in &drag.items {
            if let Some(n) = self.node(id)
                && matches!(
                    n.kind,
                    NodeKind::Card { .. } | NodeKind::Note { .. } | NodeKind::View { .. } | NodeKind::Shape { .. }
                )
            {
                let pos = n.position;
                self.busy.insert(id.clone());
                self.drag_start.insert(id.clone(), pos);
            }
        }
    }

    /// `onNodeDrag` (BOARD-051, BOARD-053, BOARD-056, BOARD-057).
    fn on_drag(&mut self, lead: &str, drag: &DragState, cx: &mut Context<'_, Self>) {
        let Some(node) = self.node(lead).cloned() else { return };
        if lead == ARCHIVE_ID {
            let Some(last) = self.last_archive_pos else { return };
            let (dx, dy) = (node.position.x - last.x, node.position.y - last.y);
            if dx == 0.0 && dy == 0.0 {
                return;
            }
            self.last_archive_pos = Some(node.position);
            let archived: Vec<bool> = self
                .nodes
                .iter()
                .map(|n| {
                    n.card_index()
                        .is_some_and(|i| self.board.as_ref().is_some_and(|b| b.cards[i].node.archived))
                })
                .collect();
            for (n, archived) in self.nodes.iter_mut().zip(archived) {
                if archived || n.id == NOTEPAD_ID || n.is_draft() {
                    n.position.x += dx;
                    n.position.y += dy;
                }
            }
            return;
        }
        if let NodeKind::Section { .. } = node.kind {
            let Some(sd) = self.section_drag.as_mut() else { return };
            let (dx, dy) = (node.position.x - sd.last.x, node.position.y - sd.last.y);
            if dx == 0.0 && dy == 0.0 {
                return;
            }
            sd.last = node.position;
            let members = sd.members.clone();
            for n in &mut self.nodes {
                if members.contains(&n.id) {
                    n.position.x += dx;
                    n.position.y += dy;
                }
            }
            return;
        }
        let zone = self.zone();
        let Some(board) = &self.board else { return };
        let dragged_cards: Vec<&FlowNode> = drag
            .items
            .iter()
            .filter_map(|(id, _)| self.node(id))
            .filter(|n| n.is_card())
            .collect();
        let over_archive = dragged_cards.iter().any(|n| {
            let card = &board.cards[n.card_index().unwrap_or(0)];
            !is_terminal(card) && is_in_archive(n.position, zone, card_size(card))
        });
        // FRAME-015: the frame the grabbed thing would land in lights up.
        let hot = if over_archive || !super::frames::groupable(&node) {
            None
        } else {
            section_at(node.position, &section_views(&board.sections, &board.cards), node.size)
        };
        if self.archive_hot != over_archive || self.hot_section != hot {
            self.archive_hot = over_archive;
            self.hot_section = hot;
            cx.notify();
        }
    }

    /// d3-drag's end: `onNodeDragStop` (BOARD-052…BOARD-066).
    fn end_drag(&mut self, drag: &DragState, cx: &mut Context<'_, Self>) {
        for n in &mut self.nodes {
            n.dragging = false;
        }
        let lead = drag.node.clone().unwrap_or_else(|| drag.items[0].0.clone());
        // BOARD-058.
        self.archive_hot = false;
        self.hot_section = None;
        let Some(node) = self.node(&lead).cloned() else { return };
        let Some(board) = self.board.clone() else { return };

        // BOARD-054, FRAME-014: the frame and what it took along are saved
        // where they landed; it keeps its size.
        if let NodeKind::Section { .. } = node.kind {
            let sd = self.section_drag.take();
            self.moving_section = false;
            let members = sd.map(|s| s.members).unwrap_or_default();
            let mut moved: Vec<(String, Point)> = vec![(lead.clone(), node.position)];
            moved.extend(
                members
                    .iter()
                    .filter_map(|id| self.node(id).map(|n| (id.clone(), n.position))),
            );
            self.move_things(&moved, cx);
            self.busy.remove(&lead);
            for id in &members {
                self.busy.remove(id);
            }
            self.rebuild(cx);
            return;
        }

        // BOARD-052.
        if lead == ARCHIVE_ID {
            let next = Rect {
                x: js::round(node.position.x),
                y: js::round(node.position.y),
                w: board.archive.w,
                h: board.archive.h,
            };
            self.archive_busy = false;
            self.moving_archive = false;
            self.last_archive_pos = None;
            self.busy.clear();
            self.state.update(cx, |s, cx| s.set_archive(next, cx));
            self.rebuild(cx);
            return;
        }

        let pad = super::layout::note_pad_rect(board.archive);
        // BOARD-059: the pad's top sheet.
        if node.is_draft() {
            self.busy.remove(&lead);
            self.drag_start.clear();
            if is_on_pad(node.position, pad, NOTE_SIZE) || is_in_archive(node.position, self.zone(), NOTE_SIZE) {
                let home = pad_sheet_spot(pad);
                if let Some(n) = self.node_mut(&lead) {
                    n.position = home;
                }
            } else {
                // MOTION-010: the sheet was in hand all the way.
                let id = self.draft_id.clone();
                self.peel_note(node.position, cx);
                self.moments.hush(&id);
            }
            return;
        }

        let zone = self.zone();
        let moved: Vec<FlowNode> = drag.items.iter().filter_map(|(id, _)| self.node(id).cloned()).collect();
        let moved_cards: Vec<&FlowNode> = moved.iter().filter(|n| n.is_card()).collect();
        let moved_notes: Vec<&FlowNode> = moved
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Note { index: Some(_) }))
            .collect();
        let moved_views: Vec<&FlowNode> = moved
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::View { .. }))
            .collect();
        let moved_shapes: Vec<&FlowNode> = moved
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Shape { .. }))
            .collect();

        // BOARD-060, BOARD-061: bounces.
        let mut bounced: Vec<(String, Point)> = Vec::new();
        for n in &moved_cards {
            let card = &board.cards[n.card_index().unwrap_or(0)];
            if is_terminal(card) && is_in_archive(n.position, zone, card_size(card)) {
                let back = self.drag_start.get(&n.id).copied().unwrap_or(Point {
                    x: card.node.x,
                    y: card.node.y,
                });
                bounced.push((n.id.clone(), back));
            }
        }
        for n in &moved_notes {
            let NodeKind::Note { index: Some(i) } = n.kind else {
                continue;
            };
            let note = &board.notes[i];
            let size = nodus_board::geometry::note_size(note);
            if is_in_archive(n.position, zone, size) || is_on_pad(n.position, pad, size) {
                let back = self
                    .drag_start
                    .get(&n.id)
                    .copied()
                    .unwrap_or(Point { x: note.x, y: note.y });
                bounced.push((n.id.clone(), back));
            }
        }
        for n in &moved_views {
            let NodeKind::View { index } = n.kind else { continue };
            let view = &board.views[index];
            let size = nodus_board::geometry::view_size(view);
            if is_in_archive(n.position, zone, size) || is_on_pad(n.position, pad, size) {
                let back = self
                    .drag_start
                    .get(&n.id)
                    .copied()
                    .unwrap_or(Point { x: view.x, y: view.y });
                bounced.push((n.id.clone(), back));
            }
        }
        // SHAPE-010: a shape dropped on the archive or the pad goes back.
        for n in &moved_shapes {
            let Some(shape) = n.shape_index().map(|i| &board.shapes[i]) else {
                continue;
            };
            let size = nodus_board::geometry::Size { w: shape.w, h: shape.h };
            if is_in_archive(n.position, zone, size) || is_on_pad(n.position, pad, size) {
                bounced.push((n.id.clone(), Point { x: shape.x, y: shape.y }));
            }
        }
        for (id, back) in &bounced {
            if let Some(n) = self.node_mut(id) {
                n.position = *back;
            }
        }
        let is_bounced = |id: &str| bounced.iter().any(|(b, _)| b == id);

        // BOARD-062, FRAME-010: what lands in a frame is in it; nothing to save but where.
        let patches: Vec<NodePatch> = moved_cards
            .iter()
            .filter(|n| !is_bounced(&n.id))
            .map(|n| {
                let card = &board.cards[n.card_index().unwrap_or(0)];
                // BOARD-063: a result dropped outside the stack is restored where it lands.
                let archived = !is_terminal(card) && is_in_archive(n.position, zone, card_size(card));
                NodePatch {
                    session_id: n.id.clone(),
                    x: Some(js::round(n.position.x)),
                    y: Some(js::round(n.position.y)),
                    archived: Some(archived),
                    ..NodePatch::default()
                }
            })
            .collect();
        let running: Vec<nodus_domain::SessionCard> = patches
            .iter()
            .filter_map(|p| {
                let card = board.cards.iter().find(|c| c.node.session_id == p.session_id)?;
                (p.archived == Some(true)
                    && !card.node.archived
                    && card.live
                    && card.status != nodus_domain::SessionStatus::Done)
                    .then(|| card.clone())
            })
            .collect();
        let notes: Vec<nodus_domain::BoardNote> = moved_notes
            .iter()
            .filter(|n| !is_bounced(&n.id))
            .filter_map(|n| match n.kind {
                NodeKind::Note { index: Some(i) } => Some(nodus_domain::BoardNote {
                    x: js::round(n.position.x),
                    y: js::round(n.position.y),
                    ..board.notes[i].clone()
                }),
                _ => None,
            })
            .collect();
        let views: Vec<nodus_domain::ViewPatch> = moved_views
            .iter()
            .filter(|n| !is_bounced(&n.id))
            .map(|n| nodus_domain::ViewPatch {
                id: n.id.clone(),
                x: Some(js::round(n.position.x)),
                y: Some(js::round(n.position.y)),
                w: None,
                h: None,
                title: None,
                starts_sessions: None,
            })
            .collect();
        let shapes_moved: Vec<(String, Point)> = moved_shapes
            .iter()
            .filter(|n| !is_bounced(&n.id))
            .map(|n| (n.id.clone(), n.position))
            .collect();
        let released: Vec<String> = moved.iter().map(|n| n.id.clone()).collect();

        // BOARD-065: the cards in one patch, then each moved note and view.
        let apply = move |c: &mut BoardCanvas, cx: &mut Context<'_, BoardCanvas>| {
            c.state.update(cx, |s, cx| {
                s.patch_cards(patches, cx);
                for note in notes {
                    s.save_note(note, cx);
                }
                for view in views {
                    s.patch_view(view, cx);
                }
            });
            c.move_shapes(&shapes_moved, cx);
        };
        // BOARD-064: archiving a running session asks first.
        if !running.is_empty() {
            let ask = self.confirm_archive_drop(&running, cx);
            cx.spawn(async move |this, cx| {
                let ok = ask.await.unwrap_or(false);
                let _ = this.update(cx, |c, cx| {
                    if ok {
                        apply(c, cx);
                    } else {
                        let starts = c.drag_start.clone();
                        for n in &mut c.nodes {
                            if let Some(p) = starts.get(&n.id) {
                                n.position = *p;
                            }
                        }
                    }
                    for id in &released {
                        c.busy.remove(id);
                    }
                    c.drag_start.clear();
                    c.rebuild(cx);
                });
            })
            .detach();
            return;
        }
        apply(self, cx);
        for id in &released {
            self.busy.remove(id);
        }
        self.drag_start.clear();
        self.rebuild(cx);
    }

    fn confirm_archive_drop(
        &mut self,
        running: &[nodus_domain::SessionCard],
        cx: &mut Context<'_, Self>,
    ) -> tokio::sync::oneshot::Receiver<bool> {
        let (title, message) = if running.len() == 1 {
            let card = &running[0];
            let doing = if card.status == nodus_domain::SessionStatus::Waiting {
                "waiting for you"
            } else {
                "working"
            };
            (
                "Archive a running session?".to_owned(),
                format!(
                    "“{}” is still {doing}. The Claude process will be stopped.",
                    super::layout::card_title(card)
                ),
            )
        } else {
            (
                format!("Archive {} running sessions?", running.len()),
                "Some of these sessions are still working. Their Claude processes will be stopped.".to_owned(),
            )
        };
        self.confirm(title, message, "Stop & Archive", true, cx)
    }

    // ---- the wheel and pinch -------------------------------------------------------------

    /// A wheel event over the board (the pane or a node that didn't take it).
    pub(crate) fn wheel(&mut self, e: &ScrollEvent, cx: &mut Context<'_, Self>) {
        let notched = matches!(e.delta, ScrollDelta::Lines(_));
        saw_scroll(notched);
        self.scroll(e, notched, cx);
    }

    /// A wheel a view's page didn't use (VIEW-047): a mouse's when the last
    /// scroll the app saw was. A ctrl-wheel is a pinch there (a touchpad's
    /// arrives that way off a Mac), so it zooms as one.
    pub(crate) fn page_wheel(&mut self, e: &ScrollEvent, cx: &mut Context<'_, Self>) {
        let notched = LAST_NOTCHED.load(Ordering::Relaxed) && !e.modifiers.control;
        self.scroll(e, notched, cx);
    }

    /// `notched`: the scroll came from a mouse's wheel, not a trackpad or a
    /// Magic Mouse.
    fn scroll(&mut self, e: &ScrollEvent, notched: bool, cx: &mut Context<'_, Self>) {
        let platform = Platform::CURRENT;
        let (dx, dy) = dom_delta(e.delta);
        let m = e.modifiers;
        let (px, py) = self.local(e.position);
        // BOARD-077: the open panel's results scroll.
        if self.stack_open() && !m.control {
            let max = super::layout::stack_scroll_max(self.stack_results().len());
            if max > 0.0 && contains_point(self.zone(), self.to_flow(e.position)) {
                let next = (self.stack_scroll.min(max) + dy / self.viewport.zoom).clamp(0.0, max);
                self.stack_scroll = next;
                self.stack_scrolling = true;
                self.stack_scroll_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(160))
                        .await;
                    let _ = this.update(cx, |c, cx| {
                        c.stack_scrolling = false;
                        cx.notify();
                    });
                }));
                self.rebuild(cx);
                return;
            }
        }
        let zoom_key = if platform.is_mac() { m.platform } else { m.control };
        let wheel_zooms = notched && !self.space && self.state.read(cx).wheel_zoom();
        let zoom = if wheel_zooms {
            // A mouse's wheel zooms (BOARD-179); with Shift or the zoom key
            // it pans. A Mac's ctrl-wheel stays a pinch.
            (!zoom_key && !m.shift).then(|| {
                if m.control {
                    viewport::wheel_zoom_exponent(dy, 0, true)
                } else {
                    viewport::notch_zoom_exponent(dy, platform.is_mac())
                }
            })
        } else if m.control {
            // A pinch arrives as a ctrl-wheel (×10 on a Mac).
            Some(viewport::wheel_zoom_exponent(dy, 0, platform.is_mac()))
        } else if zoom_key && !self.space {
            Some(viewport::wheel_zoom_exponent(dy, 0, false))
        } else {
            None
        };
        if let Some(exp) = zoom {
            let vp = viewport::zoom_about(self.viewport, self.viewport.zoom * 2f64.powf(exp), px, py);
            self.set_viewport(vp, Some(300), cx);
        } else {
            let (dx, dy) = if !platform.is_mac() && m.shift && dy == 0.0 {
                // GPUI already turned Shift+wheel horizontal off a Mac.
                (0.0, dx)
            } else {
                (dx, dy)
            };
            let vp = viewport::wheel_pan(self.viewport, dx, dy, 0, m.shift, platform.is_mac());
            self.set_viewport(vp, Some(150), cx);
        }
    }

    /// A trackpad pinch: Chromium sent it as a ctrl-wheel of
    /// `deltaY = −100·ln(scale)`.
    pub(crate) fn pinch(&mut self, e: &PinchEvent, cx: &mut Context<'_, Self>) {
        let scale = 1.0 + f64::from(e.delta);
        if scale <= 0.0 {
            return;
        }
        let dy = -100.0 * scale.ln();
        let exp = viewport::wheel_zoom_exponent(dy, 0, Platform::CURRENT.is_mac());
        let (px, py) = self.local(e.position);
        let vp = viewport::zoom_about(self.viewport, self.viewport.zoom * 2f64.powf(exp), px, py);
        self.set_viewport(vp, Some(300), cx);
    }

    // ---- keys ---------------------------------------------------------------------------------

    /// The board's key listener (BOARD-156…BOARD-170). `typing`: focus is in
    /// a text field (or a terminal). True when the key was the board's.
    pub fn on_key_down(&mut self, e: &KeyDownEvent, typing: bool, cx: &mut Context<'_, Self>) -> bool {
        let platform = Platform::CURRENT;
        let m = e.keystroke.modifiers;
        let key = e.keystroke.key.to_lowercase();
        let ch = e
            .keystroke
            .key_char
            .as_deref()
            .map_or_else(|| key.clone(), str::to_lowercase);
        let modk = platform.mod_key(&m);
        let other = platform.other_mod_key(&m);
        // BOARD-141: the confirm's button has the keyboard: Enter or Space
        // confirms, Escape cancels.
        if self.confirm.is_some() {
            match key.as_str() {
                "escape" => self.resolve_confirm(false, cx),
                "enter" | "space" => self.resolve_confirm(true, cx),
                _ => return false,
            }
            return true;
        }
        let cover = self.cover(typing, cx);
        // Space held alone pans (React Flow's panActivationKeyCode).
        if key == "space" && !typing && !m.modified() {
            self.space = true;
        }
        // BOARD-157: ⌘K opens search (spec 11).
        if key == "k" && modk && !m.shift && !m.alt && !other {
            if !cover.search_key() {
                return false;
            }
            if cover.preview {
                self.state.update(cx, |s, cx| {
                    s.preview_session_id = None;
                    cx.notify();
                });
            }
            self.open_search(cx);
            return true;
        }
        // BOARD-158.
        if !cover.shortcuts() {
            return false;
        }
        // BOARD-159: only ink has an undo (spec 11, INKUI-021).
        if key == "z" && modk && !m.alt && !other {
            return if m.shift { self.ink_redo(cx) } else { self.ink_undo(cx) };
        }
        let sel = self.selected_cards();
        let note_sel = self.selected_notes();
        let view_sel = self.selected_views();
        let shape_sel = self.selected_shapes();
        let any = !sel.is_empty() || !note_sel.is_empty() || !view_sel.is_empty() || !shape_sel.is_empty();
        let plain = !m.platform && !m.control && !m.alt;
        match key.as_str() {
            // BOARD-160.
            "escape" => {
                // SRV-182: Esc stops following first.
                if self.stop_following(cx) {
                    return true;
                }
                // INKUI-023: the ink puts its tool down or lets go of its stroke first.
                if self.ink_escape(cx) {
                    return false;
                }
                if self.stack_open() {
                    self.close_stack(cx);
                }
                if any {
                    self.clear_selection(cx);
                }
                false
            }
            // BOARD-161.
            "a" if modk && !m.shift && !m.alt => {
                self.select_all_active(cx);
                true
            }
            // GROUP-001, GROUP-003, FRAME-020: ⌘G glues the selection
            // together, ⇧⌘G takes it apart, ⌥⌘G frames it.
            "g" if modk && !other => {
                let things = self.selected_things();
                if things.is_empty() {
                    return false;
                }
                match (m.shift, m.alt) {
                    (false, false) => self.group_things(&things, cx),
                    (true, false) => self.ungroup_things(&things, cx),
                    (false, true) => self.frame_things(&things, cx),
                    (true, true) => return false,
                }
                true
            }
            // BOARD-178: copy and paste notes.
            "c" if modk && !m.shift && !m.alt && !other => self.copy_notes(cx),
            "v" if modk && !m.shift && !m.alt && !other => self.paste_notes(cx),
            // BOARD-163.
            "backspace" | "delete" if !modk && !m.alt => {
                // INKUI-022: the selected stroke goes first.
                if self.ink_delete_selected(cx) {
                    return true;
                }
                if !any {
                    return false;
                }
                self.delete_notes(&note_sel, cx);
                // SHAPE-020.
                if !shape_sel.is_empty() {
                    self.delete_shapes(&shape_sel, cx);
                }
                self.archive_cards(&sel, cx);
                self.delete_views(&view_sel, cx);
                true
            }
            // BOARD-164.
            "enter" if !modk && !m.shift => {
                if sel.len() == 1 {
                    self.open_card(&sel[0], cx);
                    true
                } else if sel.is_empty() && note_sel.len() == 1 {
                    self.set_renaming(Some(note_sel[0].clone()), cx);
                    true
                } else if sel.is_empty()
                    && note_sel.is_empty()
                    && let [one] = shape_sel.as_slice()
                    && self.shape(one).is_some_and(|s| s.kind.has_text())
                {
                    // SHAPE-012: Return writes in the one selected shape.
                    self.set_renaming(Some(one.clone()), cx);
                    true
                } else {
                    false
                }
            }
            _ if plain => match ch.as_str() {
                // BOARD-165.
                "n" => {
                    self.create_session(None, SessionKind::Claude, cx);
                    true
                }
                "t" => {
                    self.create_session(None, SessionKind::Terminal, cx);
                    true
                }
                // BOARD-166, BOARD-167, BOARD-169: the prompt bar and ink (spec 11).
                // BAR-034: record, or stop and transcribe.
                "v" => {
                    if e.is_held {
                        return false;
                    }
                    self.bar_toggle_voice(cx);
                    true
                }
                // BAR-033: the bar takes the keyboard, with `/` typed.
                "/" => {
                    self.bar_focus_from_board(Some("/".to_owned()), cx);
                    true
                }
                "p" | "a" | "r" | "s" | "x" | "b" | "m" => {
                    if let Some(tool) = super::overlays::ink::tool_for_key(&ch) {
                        self.ink_set_tool(tool, None, cx);
                    }
                    true
                }
                // BOARD-168.
                "e" => {
                    let active: Vec<bool> = self
                        .board
                        .as_ref()
                        .map(|b| {
                            b.cards
                                .iter()
                                .filter(|c| sel.contains(&c.node.session_id) && !c.node.archived)
                                .map(is_expanded)
                                .collect()
                        })
                        .unwrap_or_default();
                    if active.is_empty() {
                        return false;
                    }
                    let expand = active.iter().any(|e| !e);
                    self.expand_cards(&sel, expand, cx);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub fn on_key_up(&mut self, e: &KeyUpEvent, cx: &mut Context<'_, Self>) {
        if e.keystroke.key == "space" && self.space {
            self.space = false;
            cx.notify();
        }
    }

    /// Window blur and any context menu let go of held keys.
    pub fn release_keys(&mut self) {
        self.space = false;
        // INKUI-012, INKUI-020: a gesture the window lost goes.
        self.overlays.ink.drop_gesture();
    }

    /// What is over the board now (`typing`: a field has the keyboard).
    pub(crate) fn cover(&self, typing: bool, cx: &gpui::App) -> Cover {
        let s = self.state.read(cx);
        Cover {
            modal: s.terminal_session_id.is_some(),
            preview: s.preview_session_id.is_some(),
            agents: s.agent_view.is_some(),
            diff: s.diff_open,
            overlay: self.overlay_open(cx),
            typing,
        }
    }

    /// BOARD-171: the app menu's board commands.
    pub fn on_menu(&mut self, command: MenuCommand, cx: &mut Context<'_, Self>) {
        // RPT-002: nothing starts behind the report dialog.
        if !self.cover(false, cx).menu() || self.state.read(cx).report_open {
            return;
        }
        match command {
            MenuCommand::NewSession => self.create_session(None, SessionKind::Claude, cx),
            MenuCommand::NewTerminal => self.create_session(None, SessionKind::Terminal, cx),
            // MODAL-031: over the session window (and the preview), which close.
            MenuCommand::Search => {
                self.state.update(cx, |s, cx| {
                    s.terminal_session_id = None;
                    s.preview_session_id = None;
                    cx.notify();
                });
                self.open_search(cx);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_window_keeps_the_boards_keys_but_not_the_menus_search() {
        // MODAL-031: with the session window up, no shortcut and no ⌘K…
        let modal = Cover {
            modal: true,
            ..Cover::default()
        };
        assert!(!modal.search_key());
        assert!(!modal.shortcuts());
        // …but the menu's Search Sessions… still comes (and closes it).
        assert!(modal.menu());
        // A bare board takes all three.
        let bare = Cover::default();
        assert!(bare.search_key() && bare.shortcuts() && bare.menu());
        // TRANS-024: the preview gives way to ⌘K, not to the other shortcuts.
        let preview = Cover {
            preview: true,
            ..Cover::default()
        };
        assert!(preview.search_key());
        assert!(!preview.shortcuts());
        assert!(preview.menu());
        // An overlay, the agents viewer or the diff take the menu's too.
        for cover in [
            Cover {
                overlay: true,
                ..Cover::default()
            },
            Cover {
                agents: true,
                ..Cover::default()
            },
            Cover {
                diff: true,
                ..Cover::default()
            },
        ] {
            assert!(!cover.search_key() && !cover.shortcuts() && !cover.menu());
        }
        // A field (or a focused terminal) keeps its keys; the menu still works.
        let typing = Cover {
            typing: true,
            ..Cover::default()
        };
        assert!(!typing.search_key() && !typing.shortcuts() && typing.menu());
    }

    #[test]
    fn multi_selects_with_shift_or_meta_alone() {
        let m = |shift, platform, control| Modifiers {
            shift,
            platform,
            control,
            ..Modifiers::default()
        };
        assert!(multi_select(&m(true, false, false)));
        assert!(multi_select(&m(false, true, false)));
        assert!(!multi_select(&m(true, true, false)));
        // Ctrl never adds to the selection, on any platform.
        assert!(!multi_select(&m(false, false, true)));
    }

    #[test]
    fn reads_wheel_deltas_as_chromium_reports_them() {
        let (dx, dy) = dom_delta(ScrollDelta::Pixels(gpui::point(gpui::px(3.), gpui::px(-10.))));
        assert_eq!((dx, dy), (-3.0, 10.0));
    }
}
