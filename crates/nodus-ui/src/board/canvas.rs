//! The board canvas (`BoardCanvas` in `BoardView.tsx`): one project's
//! pannable, zoomable board, with React Flow's mechanics re-implemented
//! (spec 09, Canvas mechanics). This file holds its state, the scene
//! rebuild, the viewport and its persistence, toasts, confirms and the
//! archive stack's state; `input.rs` routes the pointer and keys,
//! `render.rs` draws it, `commands.rs` and `menus.rs` are what the board
//! does.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Bounds, Context, Entity, FocusHandle, Pixels, Point as GPoint, Subscription, Task};
use nodus_board::geometry::{Point, Size};
use nodus_domain::{BoardNote, BoardSnapshot, NodePatch, Rect, ViewPatch, Viewport};
use tokio::sync::oneshot;

use super::edges::{self, Edge};
use super::fuzzy::fuzzy_match;
use super::layout::{self, card_title, stack_rank, stack_zone};
use super::nodes::{ArchiveStack, BoardActions, BoardCx, BoardPointer};
use super::scene::{self, BuildInput, FlowNode, NodeKind, StackView};
use super::viewport::{self, Flight as VpFlight};
use crate::components::context_menu::{Menu, MenuItem, MenuLayer};
use crate::state::{AppState, StateEvent};

/// How long the viewport waits after a move ends before it's saved (BOARD-…).
pub const SAVE_VIEWPORT_MS: u64 = 400;
/// How long a toast stays (BOARD-139).
pub const TOAST_MS: u64 = 4200;
/// How long a name from Claude takes to land in a section's header.
pub const NAME_LANDING_MS: u64 = 1200;
/// A title click waits this long for a double-click (BOARD-043).
pub const TITLE_CLICK_MS: u64 = 240;
/// `HOME_FREE_SPACE` and `HOME_ARCHIVE_ROWS_H` (BOARD-032).
const HOME_FREE_SPACE: f64 = 760.0;
const HOME_ARCHIVE_ROWS_H: f64 = 64.0 + 3.0 * 240.0;

/// A question the board is waiting on (`ConfirmRequest`).
pub struct Confirm {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub danger: bool,
    pub serial: u64,
    reply: Option<oneshot::Sender<bool>>,
}

/// An animated viewport move under way.
pub(crate) struct ViewportAnim {
    pub flight: VpFlight,
    pub start: Instant,
    pub duration: Duration,
}

/// What the pointer is doing (React Flow's d3-drag, d3-zoom and pane
/// handlers, merged into one state machine).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Gesture {
    Idle,
    /// Left pressed on a node (or on the selection rectangle, `node: None`);
    /// a drag starts past 1px (`nodeDragThreshold`).
    Press {
        node: Option<String>,
        start: GPoint<Pixels>,
        /// The pointer has moved at all: no click (`nodeClickDistance = 0`).
        moved: bool,
        /// On the node's drag handle (section header, archive header, view head).
        handle: bool,
        /// On a card's title text (`.card-title`).
        title: bool,
        click_count: usize,
        shift: bool,
        meta: bool,
        ctrl: bool,
        multi: bool,
    },
    /// Nodes being dragged.
    Drag(DragState),
    /// Left pressed on the pane; a marquee starts past 1px (`paneClickDistance`).
    PanePress {
        start: GPoint<Pixels>,
        click_count: usize,
    },
    /// The marquee, from a flow point to the pointer.
    Marquee {
        from: Point,
        pointer: GPoint<Pixels>,
    },
    /// Middle-drag or Space-drag pan.
    Pan {
        last: GPoint<Pixels>,
    },
    /// Dragging in the minimap.
    MinimapPan {
        last: GPoint<Pixels>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragState {
    /// The grabbed node; None when the selection rectangle is dragged.
    pub node: Option<String>,
    /// Dragged nodes in document order, each with the pointer's offset from
    /// its corner (flow units).
    pub items: Vec<(String, Point)>,
    /// The pointer, flow units (`lastPos`).
    pub last: Point,
    /// The pointer, window coordinates, for the auto-pan.
    pub pointer: GPoint<Pixels>,
}

/// The section being dragged by its header, and the cards travelling with it.
#[derive(Debug, Clone)]
pub(crate) struct SectionDrag {
    pub last: Point,
    pub members: Vec<String>,
}

/// A node's drawn box gliding to where it should be.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Glide {
    pub from: Rect,
    pub to: Rect,
    pub start: Instant,
    pub delay: Duration,
    pub duration: Duration,
    pub ease: fn(f32) -> f32,
}

impl Glide {
    pub fn at(&self, now: Instant) -> Rect {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed < self.delay {
            return self.from;
        }
        let t = (elapsed - self.delay).as_secs_f64() / self.duration.as_secs_f64().max(1e-9);
        if t >= 1.0 {
            return self.to;
        }
        let e = f64::from((self.ease)(t as f32));
        let mix = |a: f64, b: f64| a + (b - a) * e;
        Rect {
            x: mix(self.from.x, self.to.x),
            y: mix(self.from.y, self.to.y),
            w: mix(self.from.w, self.to.w),
            h: mix(self.from.h, self.to.h),
        }
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.delay + self.duration
    }
}

/// One project's board. BOARD-004: the shell drops it when the project
/// changes, and with it every timer it holds (title click, stack scroll, the
/// viewport save: a move in its last 400ms is lost, as in 0.9.4) and its
/// subscriptions to the store and the menu; the keys reach it only while the
/// shell holds it.
pub struct BoardCanvas {
    pub state: Entity<AppState>,
    pub menus: Entity<MenuLayer>,
    pub project_id: String,
    pub(crate) focus: FocusHandle,
    /// The snapshot as the store has it, and as the board draws it (runs and
    /// view brains removed, BOARD-003).
    pub(crate) source: Option<BoardSnapshot>,
    pub(crate) board: Option<BoardSnapshot>,
    pub(crate) nodes: Vec<FlowNode>,
    pub(crate) edges: Vec<Edge>,

    pub(crate) viewport: Viewport,
    pub(crate) anim: Option<ViewportAnim>,
    /// The canvas element, in window coordinates.
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) initialized: bool,
    pub(crate) moving: bool,
    /// The zoom changed during this move: node text paints from stepped
    /// glyph sizes until it ends (`units::Zoomed`).
    pub(crate) zooming: bool,
    pub(crate) move_end: Option<Task<()>>,
    pub(crate) save_timer: Option<Task<()>>,

    pub(crate) gesture: Gesture,
    pub(crate) space: bool,
    pub(crate) pointer: Option<GPoint<Pixels>>,
    /// The post-marquee selection rectangle (`nodesSelectionActive`).
    pub(crate) selection_rect: bool,
    pub(crate) title_pressed: Rc<Cell<bool>>,
    pub(crate) title_click: Option<Task<()>>,

    // Nodes that keep their live position while the store changes.
    pub(crate) busy: HashSet<String>,
    pub(crate) drag_start: HashMap<String, Point>,
    pub(crate) archive_busy: bool,
    pub(crate) last_archive_pos: Option<Point>,
    pub(crate) section_drag: Option<SectionDrag>,
    pub(crate) held: HashMap<String, Rect>,
    pub(crate) raised: HashSet<String>,
    pub(crate) glides: HashMap<String, Glide>,
    /// Cards and notes, each its own cached view (`node_view`).
    pub(crate) node_views: HashMap<String, Entity<super::node_view::NodeView>>,

    pub(crate) renaming: Option<String>,
    pub(crate) archive_hot: bool,
    pub(crate) hot_section: Option<String>,
    pub(crate) naming: HashMap<String, Option<String>>,
    pub(crate) moving_section: bool,
    pub(crate) moving_archive: bool,

    /// The merge dialog's sources and the focus a search gave it (spec 11).
    pub merge: Option<Vec<super::bar::MergeSource>>,
    pub merge_focus: Option<String>,
    /// ⌘K search is up, and its query, kept between openings (spec 11).
    pub search_open: bool,
    pub search_query: String,
    /// The New view dialog is up, for a view centred on this point or in
    /// view (spec 04).
    pub view_dialog: Option<Option<Point>>,

    /// The card a search jump flashes, since when (BOARD-036).
    pub(crate) flash: Option<(String, Instant)>,
    pub(crate) flash_timer: Option<Task<()>>,

    pub(crate) toast: Option<(String, u64)>,
    toast_seq: u64,
    toast_timer: Option<Task<()>>,
    pub(crate) confirm: Option<Confirm>,
    confirm_serial: u64,

    // The archive stack.
    pub(crate) stack_open_flag: bool,
    pub(crate) stack_query: String,
    pub(crate) stack_hover: bool,
    pub(crate) stack_dismissed: bool,
    pub(crate) stack_raised: bool,
    raise_timer: Option<Task<()>>,
    pub(crate) stack_scroll: f64,
    pub(crate) stack_scrolling: bool,
    pub(crate) stack_scroll_timer: Option<Task<()>>,
    pub(crate) flights: HashMap<String, scene::Flight>,
    flight_timer: Option<Task<()>>,

    pub(crate) draft_id: String,
    pub(crate) starting_notes: HashSet<String>,
    pub(crate) last_in_modal: Option<String>,
    pub(crate) run_ids: HashSet<String>,
    pub(crate) view_starts: HashMap<String, f64>,
    pub(crate) section_field: Option<(String, Entity<gpui::component::input::InputState>)>,
    pub(crate) section_field_sub: Option<Subscription>,
    pub(crate) clock: Instant,
    /// The prompt bar, search, the merge dialog and the ink (spec 11).
    pub(crate) overlays: super::overlays::Overlays,
    /// A link whose label the menu asked to write, opened when next drawn.
    pub(crate) pending_link_label: Option<String>,
    /// The board was read-only when last drawn (SRV-028).
    pub(crate) read_only_seen: bool,
    _subscriptions: Vec<Subscription>,
}

impl BoardCanvas {
    pub fn new(
        state: Entity<AppState>,
        menus: Entity<MenuLayer>,
        project_id: String,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| this.sync(cx)),
            cx.subscribe(&state, |this, _, e: &StateEvent, cx| match e {
                StateEvent::BoardMenu(command) => this.on_menu(*command, cx),
                StateEvent::RunFinished(run) => this.run_finished(run, cx),
                StateEvent::Toast(message) => this.show_toast(message.clone(), cx),
                StateEvent::SettingsChanged => {}
            }),
        ];
        let mut canvas = Self {
            state,
            menus,
            project_id,
            focus: cx.focus_handle(),
            source: None,
            board: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            viewport: viewport::initial(None),
            anim: None,
            bounds: Bounds::default(),
            initialized: false,
            moving: false,
            zooming: false,
            move_end: None,
            save_timer: None,
            gesture: Gesture::Idle,
            space: false,
            pointer: None,
            selection_rect: false,
            title_pressed: Rc::new(Cell::new(false)),
            title_click: None,
            busy: HashSet::new(),
            drag_start: HashMap::new(),
            archive_busy: false,
            last_archive_pos: None,
            section_drag: None,
            held: HashMap::new(),
            raised: HashSet::new(),
            glides: HashMap::new(),
            node_views: HashMap::new(),
            renaming: None,
            archive_hot: false,
            hot_section: None,
            naming: HashMap::new(),
            moving_section: false,
            moving_archive: false,
            merge: None,
            merge_focus: None,
            search_open: false,
            search_query: String::new(),
            view_dialog: None,
            flash: None,
            flash_timer: None,
            toast: None,
            toast_seq: 0,
            toast_timer: None,
            confirm: None,
            confirm_serial: 0,
            stack_open_flag: false,
            stack_query: String::new(),
            stack_hover: false,
            stack_dismissed: false,
            stack_raised: false,
            raise_timer: None,
            stack_scroll: 0.0,
            stack_scrolling: false,
            stack_scroll_timer: None,
            flights: HashMap::new(),
            flight_timer: None,
            draft_id: uuid::Uuid::new_v4().to_string(),
            starting_notes: HashSet::new(),
            last_in_modal: None,
            run_ids: HashSet::new(),
            view_starts: HashMap::new(),
            section_field: None,
            section_field_sub: None,
            clock: Instant::now(),
            overlays: super::overlays::Overlays::default(),
            pending_link_label: None,
            read_only_seen: false,
            _subscriptions: subscriptions,
        };
        canvas.sync(cx);
        canvas
    }

    /// Milliseconds on the canvas's monotonic clock (`performance.now()`).
    pub(crate) fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64() * 1000.0
    }

    /// The board as drawn.
    pub fn board(&self) -> Option<&BoardSnapshot> {
        self.board.as_ref()
    }

    /// A server board loaded while its server was away: read-only until
    /// the server is back and the board reloads (SRV-028).
    pub fn read_only(&self) -> bool {
        self.board.as_ref().is_some_and(|b| b.offline)
    }

    /// Takes in a new snapshot from the store (BOARD-005, BOARD-003), and what
    /// follows the store: the terminal modal closing on a card (BOARD-103)
    /// and runs taken over (BOARD-105).
    pub(crate) fn sync(&mut self, cx: &mut Context<'_, Self>) {
        let (snapshot, in_modal) = {
            let s = self.state.read(cx);
            let snapshot = s.board.clone().filter(|b| b.project.id == self.project_id);
            (snapshot, s.terminal_session_id.clone())
        };
        if snapshot != self.source {
            let first = self.source.is_none();
            self.source = snapshot.clone();
            self.board = snapshot.map(|mut b| {
                b.cards.retain(|c| c.node.run_of.is_none() && c.node.view_of.is_none());
                b
            });
            if first && let Some(board) = &self.board {
                self.viewport = viewport::initial(board.viewport);
            }
            self.rebuild(cx);
            self.follow_sections(cx);
            self.join_taken_over_runs(cx);
        }
        if in_modal != self.last_in_modal {
            let closed = std::mem::replace(&mut self.last_in_modal, in_modal);
            if let Some(closed) = closed
                && self.last_in_modal.as_ref() != Some(&closed)
            {
                self.expand_after_modal(&closed, cx);
            }
        }
        cx.notify();
    }

    /// BOARD-082: a section's stored corner follows its cards.
    fn follow_sections(&mut self, cx: &mut Context<'_, Self>) {
        let Some(board) = &self.board else { return };
        let moved: Vec<_> = layout::section_views(&board.sections, &board.cards)
            .into_iter()
            .filter(|g| !g.members.is_empty() && (g.section.x != g.frame.x || g.section.y != g.frame.y))
            .map(|g| nodus_domain::BoardSection {
                x: g.frame.x,
                y: g.frame.y,
                ..g.section
            })
            .collect();
        if moved.is_empty() {
            return;
        }
        let state = self.state.clone();
        cx.defer(move |cx| {
            state.update(cx, |s, cx| {
                for section in moved {
                    s.save_section(section, cx);
                }
            });
        });
    }

    /// The stack's results (BOARD-068): every archived card in pile order,
    /// or the fuzzy matches of the query, best first, the weak ones dropped.
    pub(crate) fn archived_sorted(&self) -> Vec<usize> {
        let Some(board) = &self.board else { return Vec::new() };
        let mut archived: Vec<usize> = (0..board.cards.len())
            .filter(|&i| board.cards[i].node.archived)
            .collect();
        archived.sort_by(|&a, &b| {
            stack_rank(&board.cards[b])
                .partial_cmp(&stack_rank(&board.cards[a]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        archived
    }

    /// BOARD-067: open only with something archived.
    pub fn stack_open(&self) -> bool {
        self.stack_open_flag && !self.archived_sorted().is_empty()
    }

    pub(crate) fn stack_results(&self) -> Vec<(String, Option<Vec<usize>>)> {
        if !self.stack_open() {
            return Vec::new();
        }
        let Some(board) = &self.board else { return Vec::new() };
        let sorted = self.archived_sorted();
        let q = nodus_domain::js::trim(&self.stack_query);
        if q.is_empty() {
            return sorted
                .into_iter()
                .map(|i| (board.cards[i].node.session_id.clone(), None))
                .collect();
        }
        let mut hits: Vec<(usize, usize, super::fuzzy::FuzzyMatch)> = sorted
            .iter()
            .enumerate()
            .filter_map(|(rank, &i)| fuzzy_match(q, &card_title(&board.cards[i])).map(|m| (i, rank, m)))
            .collect();
        hits.sort_by(|a, b| {
            b.2.score
                .partial_cmp(&a.2.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        let floor = hits.first().map_or(0.0, |h| h.2.score * 0.5);
        hits.into_iter()
            .filter(|h| h.2.score >= floor)
            .map(|(i, _, m)| (board.cards[i].node.session_id.clone(), Some(m.indices)))
            .collect()
    }

    /// The stack as laid out now.
    pub(crate) fn stack_view(&self) -> StackView {
        let Some(board) = &self.board else {
            return StackView::default();
        };
        let open = self.stack_open();
        let results = self.stack_results();
        let scroll_max = layout::stack_scroll_max(results.len());
        let pile = self
            .archived_sorted()
            .into_iter()
            .map(|i| board.cards[i].node.session_id.clone())
            .filter(|id| !results.iter().any(|(r, _)| r == id))
            .collect();
        StackView {
            zone: stack_zone(board.archive, open.then_some(results.len())),
            pile,
            open,
            raised: open || self.stack_raised,
            // BOARD-078: the scroll as far as these results go.
            scroll: self.stack_scroll.min(scroll_max),
            scrollable: scroll_max > 0.0,
            results,
        }
    }

    /// The archive as drawn, and the dock: it and the pad (BOARD-069).
    pub(crate) fn zone(&self) -> Rect {
        self.stack_view().zone
    }

    pub(crate) fn dock(&self) -> Rect {
        let Some(board) = &self.board else {
            return Rect::default();
        };
        layout::bounds_of(&[self.zone(), layout::note_pad_rect(board.archive)])
    }

    /// `buildNodes` again from the snapshot, keeping local state; schedules
    /// the next rebuild while cards fly (BOARD-024).
    pub(crate) fn rebuild(&mut self, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.clone() else {
            self.nodes.clear();
            self.edges.clear();
            return;
        };
        let stack = self.stack_view();
        let now = self.now();
        let prev = std::mem::take(&mut self.nodes);
        self.nodes = scene::build_nodes(BuildInput {
            board: &board,
            prev: &prev,
            busy: &self.busy,
            archive_busy: self.archive_busy,
            held: &self.held,
            stack: &stack,
            flights: &mut self.flights,
            now,
            draft_id: &self.draft_id,
        });
        for node in &mut self.nodes {
            if self.raised.contains(&node.id) {
                node.z = 1400;
            }
        }
        self.edges = edges::edges(&board);
        self.sync_selection(cx);
        self.flight_timer = scene::next_landing(&self.flights).map(|until| {
            let wait = (until - now).max(0.0) + 20.0;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(wait as u64)).await;
                let _ = this.update(cx, |c, cx| {
                    c.rebuild(cx);
                    cx.notify();
                });
            })
        });
        cx.notify();
    }

    /// The store's selection = the selected session cards (store sync).
    pub(crate) fn sync_selection(&mut self, cx: &mut Context<'_, Self>) {
        let ids: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| n.is_card() && n.selected)
            .map(|n| n.id.clone())
            .collect();
        let state = self.state.clone();
        if state.read(cx).selection != ids {
            cx.defer(move |cx| state.update(cx, |s, cx| s.set_selection(ids, cx)));
        }
    }

    pub(crate) fn node(&self, id: &str) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub(crate) fn node_mut(&mut self, id: &str) -> Option<&mut FlowNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub(crate) fn selected_ids(&self, kind: fn(&NodeKind) -> bool) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|n| n.selected && kind(&n.kind))
            .map(|n| n.id.clone())
            .collect()
    }

    pub(crate) fn selected_cards(&self) -> Vec<String> {
        self.selected_ids(|k| matches!(k, NodeKind::Card { .. }))
    }

    pub(crate) fn selected_notes(&self) -> Vec<String> {
        self.selected_ids(|k| matches!(k, NodeKind::Note { index: Some(_) }))
    }

    pub(crate) fn selected_views(&self) -> Vec<String> {
        self.selected_ids(|k| matches!(k, NodeKind::View { .. }))
    }

    // ---- viewport -----------------------------------------------------------

    /// The canvas's size in pixels.
    pub(crate) fn canvas_size(&self) -> (f64, f64) {
        (
            f64::from(f32::from(self.bounds.size.width)),
            f64::from(f32::from(self.bounds.size.height)),
        )
    }

    /// A window point in canvas pixels.
    pub(crate) fn local(&self, p: GPoint<Pixels>) -> (f64, f64) {
        (
            f64::from(f32::from(p.x - self.bounds.origin.x)),
            f64::from(f32::from(p.y - self.bounds.origin.y)),
        )
    }

    /// `screenToFlowPosition`.
    pub(crate) fn to_flow(&self, p: GPoint<Pixels>) -> Point {
        let (x, y) = self.local(p);
        let (fx, fy) = viewport::to_flow(self.viewport, x, y);
        Point { x: fx, y: fy }
    }

    /// Sets the viewport at once; a move starts and ends (`onMoveStart`,
    /// `onMoveEnd` after `end_delay`).
    pub(crate) fn set_viewport(&mut self, vp: Viewport, end_delay: Option<u64>, cx: &mut Context<'_, Self>) {
        self.anim = None;
        self.zooming |= vp.zoom != self.viewport.zoom;
        self.viewport = vp;
        self.move_started(cx);
        if let Some(delay) = end_delay {
            self.move_ends_in(delay, cx);
        }
        cx.notify();
    }

    /// Animates the viewport to `to` over `duration_ms`, as d3-zoom's
    /// transition does; 0 is instant.
    pub(crate) fn fly_to(&mut self, to: Viewport, duration_ms: f64, cx: &mut Context<'_, Self>) {
        if duration_ms <= 0.0 || !duration_ms.is_finite() {
            self.set_viewport(to, Some(0), cx);
            return;
        }
        let (w, h) = self.canvas_size();
        self.anim = Some(ViewportAnim {
            flight: VpFlight::new(self.viewport, to, w, h),
            start: Instant::now(),
            duration: crate::theme::motion::duration(cx, Duration::from_millis(duration_ms as u64)),
        });
        self.move_started(cx);
        cx.notify();
    }

    /// Advances an animated move; called every frame while one runs.
    pub(crate) fn step_animation(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(anim) = &self.anim else { return false };
        let t = anim.start.elapsed().as_secs_f64() / anim.duration.as_secs_f64().max(1e-9);
        let vp = anim.flight.at(t);
        self.zooming |= vp.zoom != self.viewport.zoom;
        self.viewport = vp;
        if t >= 1.0 {
            self.anim = None;
            self.move_ends_in(0, cx);
            return false;
        }
        true
    }

    /// `onMoveStart`: closes the context menu.
    pub(crate) fn move_started(&mut self, cx: &mut Context<'_, Self>) {
        if !self.moving {
            self.moving = true;
            self.close_menu(cx);
        }
        self.move_end = None;
    }

    /// `onMoveEnd` after `delay` ms: the viewport is saved 400ms after the
    /// last one (BOARD-…, persistence).
    pub(crate) fn move_ends_in(&mut self, delay: u64, cx: &mut Context<'_, Self>) {
        self.move_end = Some(cx.spawn(async move |this, cx| {
            if delay > 0 {
                cx.background_executor().timer(Duration::from_millis(delay)).await;
            }
            let _ = this.update(cx, |c, cx| {
                c.moving = false;
                // Drawn again, even after a pan with no zoom: views' pages
                // let the pointer through to the board while it moves, and
                // only take it back in the next frame. The text is drawn
                // crisp again at the zoom it stopped at.
                c.zooming = false;
                cx.notify();
                c.schedule_viewport_save(cx);
            });
        }));
    }

    fn schedule_viewport_save(&mut self, cx: &mut Context<'_, Self>) {
        let vp = self.viewport;
        let project_id = self.project_id.clone();
        let nodus = self.state.read(cx).nodus.clone();
        let rt = crate::board::core_rt(cx);
        self.save_timer = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SAVE_VIEWPORT_MS))
                .await;
            crate::board::on_core(&rt, {
                let (nodus, project_id, vp) = (nodus.clone(), project_id.clone(), vp);
                async move { nodus.board().save_viewport(&project_id, vp).await }
            })
            .await;
        }));
    }

    /// `zoomIn` / `zoomOut` / `zoomTo` about the canvas centre.
    pub fn zoom_by(&mut self, factor: f64, duration_ms: f64, cx: &mut Context<'_, Self>) {
        let (w, h) = self.canvas_size();
        let to = viewport::zoom_about(self.viewport, self.target_zoom() * factor, w / 2.0, h / 2.0);
        self.fly_to(to, duration_ms, cx);
    }

    pub fn zoom_to(&mut self, zoom: f64, duration_ms: f64, cx: &mut Context<'_, Self>) {
        let (w, h) = self.canvas_size();
        let to = viewport::zoom_about(self.viewport, zoom, w / 2.0, h / 2.0);
        self.fly_to(to, duration_ms, cx);
    }

    fn target_zoom(&self) -> f64 {
        self.viewport.zoom
    }

    /// `fitView` over these nodes (or all drawn ones), as React Flow does.
    pub(crate) fn fit_nodes(
        &mut self,
        ids: Option<&[String]>,
        padding: f64,
        max_zoom: f64,
        duration_ms: f64,
        cx: &mut Context<'_, Self>,
    ) {
        let rects: Vec<Rect> = self
            .nodes
            .iter()
            .filter(|n| !n.hidden && ids.is_none_or(|ids| ids.contains(&n.id)))
            .map(FlowNode::rect)
            .collect();
        if rects.is_empty() {
            return;
        }
        self.fit_bounds(layout::bounds_of(&rects), padding, max_zoom, duration_ms, cx);
    }

    pub(crate) fn fit_bounds(
        &mut self,
        bounds: Rect,
        padding: f64,
        max_zoom: f64,
        duration_ms: f64,
        cx: &mut Context<'_, Self>,
    ) {
        let (w, h) = self.canvas_size();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let to = viewport::for_bounds(bounds, w, h, viewport::MIN_ZOOM, max_zoom, padding);
        self.fly_to(to, duration_ms, cx);
    }

    /// `fitActive` (BOARD-032): the active cards, free notes, views and the
    /// sections holding cards; or with none, the archive and room right of it.
    pub fn fit_active(&mut self, duration_ms: f64, cx: &mut Context<'_, Self>) {
        let Some(board) = &self.board else { return };
        let mut ids: Vec<String> = board
            .cards
            .iter()
            .filter(|c| !c.node.archived)
            .map(|c| c.node.session_id.clone())
            .collect();
        let notes: Vec<String> = board
            .notes
            .iter()
            .filter(|n| n.session_id.is_none())
            .map(|n| n.id.clone())
            .collect();
        let views: Vec<String> = board.views.iter().map(|v| v.id.clone()).collect();
        if !ids.is_empty() || !notes.is_empty() || !views.is_empty() {
            let frames = layout::section_views(&board.sections, &board.cards)
                .into_iter()
                .filter(|g| !g.members.is_empty())
                .map(|g| g.section.id);
            ids.extend(frames);
            ids.extend(notes);
            ids.extend(views);
            self.fit_nodes(Some(&ids), 0.2, 1.0, duration_ms, cx);
            return;
        }
        let a = self.dock();
        let bounds = Rect {
            x: a.x,
            y: a.y,
            w: a.w + HOME_FREE_SPACE,
            h: a.h.min(HOME_ARCHIVE_ROWS_H),
        };
        self.fit_bounds(bounds, 0.06, viewport::MAX_ZOOM, duration_ms, cx);
    }

    /// `ensureVisible` (BOARD-034): pans (zoom kept) so a rect at `p` of
    /// `size` is on screen, centred; a side longer than the view lines up
    /// 24px from its start.
    pub fn ensure_visible(&mut self, p: Point, size: Size, cx: &mut Context<'_, Self>) {
        let (w, h) = self.canvas_size();
        if w <= 0.0 {
            return;
        }
        let Viewport { x, y, zoom } = self.viewport;
        let view = Rect {
            x: -x / zoom,
            y: -y / zoom,
            w: w / zoom,
            h: h / zoom,
        };
        let inside =
            p.x >= view.x && p.y >= view.y && p.x + size.w <= view.x + view.w && p.y + size.h <= view.y + view.h;
        if inside {
            return;
        }
        let margin = 24.0 / zoom;
        let mid = |start: f64, len: f64, view_len: f64| {
            if len <= view_len {
                start + len / 2.0
            } else {
                start + view_len / 2.0 - margin
            }
        };
        let to = viewport::centred_on(mid(p.x, size.w, view.w), mid(p.y, size.h, view.h), zoom, w, h);
        self.fly_to(to, 320.0, cx);
    }

    /// `viewportCentre` (BOARD-035).
    pub fn viewport_centre(&self, size: Size) -> Point {
        let (w, h) = self.canvas_size();
        if w <= 0.0 {
            return Point { x: 0.0, y: 0.0 };
        }
        let Viewport { x, y, zoom } = self.viewport;
        Point {
            x: (w / 2.0 - x) / zoom - size.w / 2.0,
            y: (h / 2.0 - y) / zoom - size.h / 2.0,
        }
    }

    /// `onInit`: a board with no saved viewport fits its active cards, at once.
    pub(crate) fn init_viewport(&mut self, cx: &mut Context<'_, Self>) {
        if self.initialized || self.bounds.size.width <= gpui::px(0.) || self.board.is_none() {
            return;
        }
        self.initialized = true;
        if self.board.as_ref().is_some_and(|b| b.viewport.is_none()) {
            // React Flow's fitView is instant here and doesn't count as a move.
            self.fit_active(0.0, cx);
        }
    }

    /// The minimap's flow units per pixel (`viewScale`).
    pub(crate) fn minimap_view_scale(&self) -> f64 {
        let (w, h) = self.canvas_size();
        let vp = self.viewport;
        let view = Rect {
            x: -vp.x / vp.zoom,
            y: -vp.y / vp.zoom,
            w: w / vp.zoom,
            h: h / vp.zoom,
        };
        let mut rects: Vec<Rect> = self.nodes.iter().filter(|n| !n.hidden).map(FlowNode::rect).collect();
        rects.push(view);
        let b = layout::bounds_of(&rects);
        (b.w / 176.0).max(b.h / 118.0)
    }

    /// A press in the minimap starts a pan (`XYMinimap`, pannable).
    pub(crate) fn minimap_press(&mut self, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        self.gesture = Gesture::MinimapPan { last: at };
        self.move_started(cx);
    }

    /// Dragging in the minimap moves the view, relative, never jumping.
    pub(crate) fn minimap_drag(&mut self, last: GPoint<Pixels>, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        let dx = f64::from(f32::from(at.x - last.x));
        let dy = f64::from(f32::from(at.y - last.y));
        let zoom = self.viewport.zoom;
        let scale = self.minimap_view_scale() * zoom.max(zoom.ln());
        self.viewport.x -= dx * scale;
        self.viewport.y -= dy * scale;
        cx.notify();
    }

    /// The wheel over the minimap zooms about the canvas centre.
    pub(crate) fn minimap_wheel(&mut self, e: &gpui::ScrollEvent, cx: &mut Context<'_, Self>) {
        let dy = match e.delta {
            gpui::ScrollDelta::Pixels(d) => -f64::from(f32::from(d.y)),
            gpui::ScrollDelta::Lines(l) => -f64::from(l.y) * 40.0,
        };
        let mac = crate::platform::Platform::CURRENT.is_mac();
        let exp = viewport::wheel_zoom_exponent(dy, 0, e.modifiers.control && mac);
        let (w, h) = self.canvas_size();
        let vp = viewport::zoom_about(self.viewport, self.viewport.zoom * 2f64.powf(exp), w / 2.0, h / 2.0);
        self.set_viewport(vp, Some(150), cx);
    }

    // ---- toasts, confirms, menus --------------------------------------------

    /// BOARD-139: one at a time; each clears itself after 4200ms unless
    /// replaced.
    pub fn show_toast(&mut self, message: String, cx: &mut Context<'_, Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some((message, seq));
        self.toast_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(TOAST_MS)).await;
            let _ = this.update(cx, |c, cx| {
                if c.toast.as_ref().is_some_and(|(_, s)| *s == seq) {
                    c.toast = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// BOARD-140: asks, and answers through the receiver (true = confirmed).
    pub fn confirm(
        &mut self,
        title: String,
        message: String,
        confirm_label: &str,
        danger: bool,
        cx: &mut Context<'_, Self>,
    ) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.confirm_serial += 1;
        self.confirm = Some(Confirm {
            title,
            message,
            confirm_label: confirm_label.to_owned(),
            danger,
            serial: self.confirm_serial,
            reply: Some(tx),
        });
        cx.notify();
        rx
    }

    /// Answers the open confirm.
    pub fn resolve_confirm(&mut self, ok: bool, cx: &mut Context<'_, Self>) {
        if let Some(mut confirm) = self.confirm.take()
            && let Some(reply) = confirm.reply.take()
        {
            let _ = reply.send(ok);
        }
        cx.notify();
    }

    /// Opens a context menu at a window point, replacing any open one (BOARD-172).
    pub(crate) fn open_menu(&mut self, at: GPoint<Pixels>, items: Vec<MenuItem>, cx: &mut Context<'_, Self>) {
        self.menus.update(cx, |m, cx| m.open(Menu::at(at, items), cx));
    }

    pub(crate) fn close_menu(&mut self, cx: &mut Context<'_, Self>) {
        let menus = self.menus.clone();
        if menus.read(cx).is_open() {
            cx.defer(move |cx| {
                let windows = cx.windows();
                if let Some(window) = windows.first() {
                    let _ = window.update(cx, |_, window, cx| menus.update(cx, |m, cx| m.close(window, cx)));
                }
            });
        }
    }

    /// BOARD-142: an overlay that blocks board shortcuts and menu commands:
    /// a context menu, the confirm, the merge dialog, the New view dialog or
    /// search (the actions rail's dialogs and menu join with spec 05's UI).
    pub fn overlay_open(&self, cx: &App) -> bool {
        self.menus.read(cx).is_open()
            || self.confirm.is_some()
            || self.merge.is_some()
            || self.view_dialog.is_some()
            || self.search_open
            // ACT-053: the actions rail's dialog, menu or delete confirmation.
            || crate::actions::overlay_open(cx)
            // SRV-028: a read-only board takes no shortcut or menu command.
            || self.read_only()
    }

    // ---- the archive stack ----------------------------------------------------

    /// BOARD-071.
    pub fn open_stack(&mut self, cx: &mut Context<'_, Self>) {
        self.stack_dismissed = false;
        self.set_stack_open(true, cx);
    }

    /// BOARD-071.
    pub fn close_stack(&mut self, cx: &mut Context<'_, Self>) {
        self.stack_query.clear();
        self.stack_scroll = 0.0;
        self.stack_dismissed = true;
        self.set_stack_open(false, cx);
    }

    /// BOARD-070: raised while open and 700ms after.
    fn set_stack_open(&mut self, open: bool, cx: &mut Context<'_, Self>) {
        let was = self.stack_open();
        self.stack_open_flag = open;
        let now_open = self.stack_open();
        if now_open {
            self.stack_raised = true;
            self.raise_timer = None;
        } else if was {
            self.raise_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(scene::FLIGHT_MS as u64))
                    .await;
                let _ = this.update(cx, |c, cx| {
                    c.stack_raised = false;
                    c.rebuild(cx);
                });
            }));
        }
        self.rebuild(cx);
    }

    /// BOARD-072.
    pub fn set_stack_query(&mut self, query: String, cx: &mut Context<'_, Self>) {
        let open = !nodus_domain::js::trim(&query).is_empty();
        self.stack_query = query;
        self.stack_scroll = 0.0;
        if open {
            self.set_stack_open(true, cx);
        } else {
            self.rebuild(cx);
        }
    }

    /// BOARD-073: the first result's transcript.
    pub fn submit_stack(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((id, _)) = self.stack_results().into_iter().next() {
            self.state.update(cx, |s, cx| s.open_preview(&id, cx));
        }
    }

    /// The archive context for its node (BOARD-079).
    pub(crate) fn archive_stack(&self) -> ArchiveStack {
        let results = self.stack_results().len();
        let scroll_max = layout::stack_scroll_max(results);
        ArchiveStack {
            open: self.stack_open(),
            hover: self.stack_hover && !self.stack_dismissed,
            query: self.stack_query.clone().into(),
            result_count: results,
            scroll: self.stack_scroll.min(scroll_max) as f32,
            scroll_max: scroll_max as f32,
        }
    }

    /// What the board tells its nodes this frame.
    pub(crate) fn board_cx(&self, cx: &Context<'_, Self>) -> BoardCx {
        let stack = self.archive_stack();
        BoardCx {
            project_id: self.project_id.clone().into(),
            zoom: self.viewport.zoom as f32,
            renaming: self.renaming.clone().map(Into::into),
            archive_hot: self.archive_hot,
            section_hot: self.hot_section.clone().map(Into::into),
            naming: Rc::new(self.naming.clone()),
            // BOARD-075.
            stack_lift: !stack.open && (stack.hover || self.archive_hot),
            flash: self.flash.clone().map(|(id, at)| (id.into(), at)),
            stack,
            moving_archive: self.moving_archive,
            moving_section: self.moving_section,
            scrolling_stack: self.stack_scrolling,
            ink_tool: self.overlays.ink.tool != super::overlays::ink::InkTool::Select,
            actions: BoardActions {
                canvas: cx.entity().downgrade(),
                title_pressed: self.title_pressed.clone(),
            },
            pointer: BoardPointer {
                dragging: matches!(self.gesture, Gesture::Drag(_)) || self.section_drag.is_some(),
                marquee: self.marquee_rect(),
                selection_rect: self.selection_rect,
                moving: self.moving
                    || self.anim.is_some()
                    || matches!(self.gesture, Gesture::Pan { .. } | Gesture::MinimapPan { .. }),
            },
        }
    }

    // ---- what nodes ask for (nodes.rs) ------------------------------------------

    /// BOARD-148.
    pub fn set_renaming(&mut self, id: Option<String>, cx: &mut Context<'_, Self>) {
        self.renaming = id;
        cx.notify();
    }

    /// BOARD-149.
    pub fn rename_card(&mut self, session_id: &str, title: &str, cx: &mut Context<'_, Self>) {
        self.patch_cards(
            vec![NodePatch {
                session_id: session_id.to_owned(),
                title: Some(title.to_owned()),
                ..NodePatch::default()
            }],
            cx,
        );
    }

    pub fn rename_section(&mut self, section_id: &str, name: &str, cx: &mut Context<'_, Self>) {
        let Some(section) = self
            .board
            .as_ref()
            .and_then(|b| b.sections.iter().find(|s| s.id == section_id).cloned())
        else {
            return;
        };
        let name = name.to_owned();
        self.state.update(cx, |s, cx| {
            s.save_section(nodus_domain::BoardSection { name, ..section }, cx);
        });
    }

    pub fn rename_view(&mut self, view_id: &str, title: &str, cx: &mut Context<'_, Self>) {
        self.patch_view(
            ViewPatch {
                id: view_id.to_owned(),
                x: None,
                y: None,
                w: None,
                h: None,
                title: Some(title.to_owned()),
                starts_sessions: None,
            },
            cx,
        );
    }

    pub fn patch_cards(&mut self, patches: Vec<NodePatch>, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| s.patch_cards(patches, cx));
    }

    pub fn save_note(&mut self, note: BoardNote, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| s.save_note(note, cx));
    }

    pub fn remove_notes(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| s.remove_notes(ids, cx));
    }

    pub fn patch_view(&mut self, patch: ViewPatch, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| s.patch_view(patch, cx));
    }

    /// A click inside a node that its contents took (a view's tools and
    /// buttons): React Flow's `handleNodeClick` still selected the node, or
    /// with a multi-select key toggled it.
    pub fn click_select(&mut self, id: &str, multi: bool, cx: &mut Context<'_, Self>) {
        self.selection_rect = false;
        let Some(selected) = self.node(id).filter(|n| n.selectable).map(|n| n.selected) else {
            return;
        };
        if !selected {
            for n in &mut self.nodes {
                if n.id == id {
                    n.selected = true;
                } else if !multi {
                    n.selected = false;
                }
            }
        } else if multi && let Some(n) = self.node_mut(id) {
            n.selected = false;
        }
        self.sync_selection(cx);
        cx.notify();
    }

    /// A node's live box while it's resized.
    pub fn hold_rect(&mut self, id: String, rect: Option<Rect>, cx: &mut Context<'_, Self>) {
        match rect {
            Some(rect) => {
                self.held.insert(id, rect);
            }
            None => {
                self.held.remove(&id);
            }
        }
        self.rebuild(cx);
    }

    /// z 1400 while a card's agent fan is out.
    pub fn raise(&mut self, id: String, raised: bool, cx: &mut Context<'_, Self>) {
        if raised {
            self.raised.insert(id);
        } else {
            self.raised.remove(&id);
        }
        self.rebuild(cx);
    }
}

/// A new canvas for a project's board.
pub fn new_canvas(
    state: Entity<AppState>,
    menus: Entity<MenuLayer>,
    project_id: String,
    cx: &mut App,
) -> Entity<BoardCanvas> {
    cx.new(|cx| BoardCanvas::new(state, menus, project_id, cx))
}
