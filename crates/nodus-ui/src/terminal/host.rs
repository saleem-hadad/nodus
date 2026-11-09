//! A place a session's terminal is shown: the session window's body or an
//! expanded card's (`useAttachedTerminal.ts` plus xterm's own DOM handling).
//! It attaches the session's one terminal to itself (starting or resuming
//! its process when needed), keeps it fitted, tracks the loading/ready/error
//! phase, and turns keys, text input, the pointer and the wheel into what
//! xterm.js did with them.

use std::future::Future;
use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollDelta, ScrollEvent, SharedString, Styled, Subscription,
    Task, UTF16Selection, Window, div, point, px,
};
use nodus_domain::{Event, SessionKind};
use nodus_terminal::host::{AttachPhase, Attempt, Phase, WheelGestures, WheelRoute};
use nodus_terminal::links::{self, Link, LinkKind, LinkPoint};
use nodus_terminal::pointer::{
    self, MouseAction, MouseButton as TermButton, MouseEvent, WheelDelta, WheelModifiers, WheelOutcome,
};
use nodus_terminal::{BufferPoint, ContainerId, KeyAction, Registry, Snapshot};

use super::element::TerminalElement;
use super::keymap;
use super::metrics::TermMetrics;
use super::store::{Terminals, TerminalsEvent};
use crate::parity;

/// xterm's `ALT_CLICK_MOVE_CURSOR_TIME`.
const ALT_CLICK_MS: u128 = 500;
/// The cursor blinks this long on and off (TERM-076).
pub const BLINK: Duration = Duration::from_millis(600);
/// A synchronized update the program never ends is drawn after this long (TERM-082).
const SYNC_TIMEOUT: Duration = Duration::from_millis(1000);
/// Dragging a selection past an edge scrolls every 50 ms, up to 15 lines (TERM-056).
const DRAG_SCROLL_EVERY: Duration = Duration::from_millis(50);
const DRAG_SCROLL_THRESHOLD: f64 = 50.0;
const DRAG_SCROLL_MAX_SPEED: f64 = 15.0;
/// The scrollbar's strip (`DEFAULT_SCROLL_BAR_WIDTH`, TERM-075).
pub const SCROLLBAR_W: f32 = 14.0;

/// How a host behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostConfig {
    /// Focus the terminal when it is attached (the session window), TERM-090.
    pub focus_on_attach: bool,
    /// A card on the board: its wheel gestures are shared with the board (TERM-104).
    pub card: bool,
}

/// What a host tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// The phase or its error changed.
    Phase,
}

/// Where the grid was drawn last, for mapping the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// The element's box, in its own (unscaled) layout space.
    pub bounds: Bounds<Pixels>,
    /// The grid's top-left, snapped to device pixels.
    pub origin: Point<Pixels>,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cols: usize,
    pub rows: usize,
    /// A card on a zoomed board: drawn scaled by `zoom` about `at` (TERM-083).
    pub zoom: Option<(Point<Pixels>, f32)>,
}

impl Geometry {
    /// A window position mapped back into the terminal's unscaled space (`unscalePointer`).
    pub fn unscale(&self, p: Point<Pixels>) -> Point<Pixels> {
        match self.zoom {
            Some((at, zoom)) if zoom != 1.0 && zoom > 0.0 => at + (p - at) / zoom,
            _ => p,
        }
    }

    /// The position relative to the grid's top-left, in CSS pixels.
    pub fn local(&self, p: Point<Pixels>) -> (f64, f64) {
        let p = self.unscale(p);
        (f64::from(p.x - self.origin.x), f64::from(p.y - self.origin.y))
    }

    /// The grid's size on the page (`css.canvas`).
    pub fn grid_size(&self) -> (f64, f64) {
        (
            f64::from(self.cols as f32 * self.cell_w),
            f64::from(self.rows as f32 * self.cell_h),
        )
    }

    /// Whether the (unscaled) point is over the scrollbar strip.
    pub fn over_scrollbar(&self, p: Point<Pixels>) -> bool {
        let p = self.unscale(p);
        p.x >= self.bounds.right() - px(SCROLLBAR_W) && self.bounds.contains(&p)
    }

    /// Whether the window position is over the element at all.
    pub fn contains(&self, p: Point<Pixels>) -> bool {
        self.bounds.contains(&self.unscale(p))
    }
}

/// A selection being dragged.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// Where it started (an end-exclusive selection point).
    anchor: BufferPoint,
    /// Word or line selection extends by words or lines.
    unit: DragUnit,
    column: bool,
    /// The edge auto-scroll amount (lines per tick), xterm's `_dragScrollAmount`.
    scroll: i32,
    /// The last pointer position, to keep extending while auto-scrolling.
    last: Point<Pixels>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragUnit {
    Char,
    Word,
    Line,
}

/// A mouse-down that may become an ⌥-click (TERM-062).
#[derive(Debug, Clone, Copy)]
struct Press {
    at: Instant,
    alt: bool,
}

/// The scrollbar's state (TERM-075).
#[derive(Debug, Clone, Copy, Default)]
pub struct ScrollbarState {
    pub hovered: bool,
    /// Grabbed: the pointer's offset from the slider's top.
    pub grab: Option<f32>,
    /// When it last scrolled.
    pub active_at: Option<Instant>,
}

/// One session's terminal shown here.
pub struct TerminalHost {
    pub session_id: SharedString,
    pub project_id: SharedString,
    pub kind: SessionKind,
    config: HostConfig,
    terminals: Entity<Terminals>,
    pub(crate) registry: Arc<Registry>,
    container: ContainerId,
    pub(crate) focus: FocusHandle,
    pub phase: AttachPhase,
    attempt: Option<Attempt>,
    enabled: bool,
    attached: bool,
    fit: Option<(usize, usize)>,
    pub(crate) geometry: Option<Geometry>,
    /// The board's zoom when the host is an expanded card's (1 in the
    /// session window): the terminal is laid out and drawn at 1× and its
    /// picture scaled, as 0.9.4's canvas was under the board's transform
    /// (TERM-083).
    pub(crate) scale: f32,
    /// Who a wheel gesture over a card's terminal belongs to, when the card
    /// decides that itself (TERM-104…TERM-106); else [`BoardWheel`].
    wheel_gate: Option<WheelGate>,
    /// Focus asked for before the terminal was attached here (a card's wake, TERM-095).
    focus_pending: bool,
    /// What was drawn last, kept while a synchronized update holds (TERM-082).
    pub(crate) shown: Option<Arc<Snapshot>>,
    hold_timer: Option<Task<()>>,
    pub(crate) blink_on: bool,
    blink_task: Option<Task<()>>,
    drag: Option<Drag>,
    drag_task: Option<Task<()>>,
    press: Option<Press>,
    pub(crate) hovered_link: Option<Link>,
    pressed_link: Option<Link>,
    pub(crate) scrollbar: ScrollbarState,
    pub(crate) marked: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<HostEvent> for TerminalHost {}

impl Focusable for TerminalHost {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

type AttachFuture = Pin<Box<dyn Future<Output = Result<(), String>>>>;

/// Says whether a wheel event over a card's terminal scrolls it (true) or
/// belongs to the board's gesture (false); called with whether Ctrl is held.
pub type WheelGate = std::rc::Rc<dyn Fn(bool, &mut App) -> bool>;

/// The browser's wheel gestures on the board, shared by every card (TERM-104).
#[derive(Default)]
pub struct BoardWheel(pub WheelGestures);

impl gpui::Global for BoardWheel {}

/// Milliseconds on a monotonic clock, for wheel gestures.
pub fn now_ms() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// The board saw a wheel event that no card's terminal took: it belongs to
/// the board's gesture (TERM-105). The canvas calls this for every wheel
/// event it handles.
pub fn board_wheel(ctrl: bool, cx: &mut App) {
    cx.default_global::<BoardWheel>().0.route(now_ms(), false, ctrl);
}

impl TerminalHost {
    pub fn new(
        session_id: impl Into<SharedString>,
        project_id: impl Into<SharedString>,
        kind: SessionKind,
        config: HostConfig,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let session_id = session_id.into();
        let terminals = Terminals::global(cx);
        let (registry, container) = terminals.update(cx, |t, _| (t.registry.clone(), t.new_container()));
        let focus = cx.focus_handle();
        let mine = session_id.clone();
        let subscriptions = vec![
            cx.subscribe(&terminals, move |this: &mut Self, _, event, cx| match event {
                TerminalsEvent::Output(id) if *id == mine => {
                    // TERM-088: the first output after the attach.
                    if let Some(attempt) = this.attempt {
                        let before = this.phase.clone();
                        this.phase.output(attempt);
                        if this.phase != before {
                            cx.emit(HostEvent::Phase);
                        }
                    }
                    this.restart_blink(cx);
                    cx.notify();
                }
                TerminalsEvent::Changed(id) if *id == mine => cx.notify(),
                _ => {}
            }),
            // TERM-077: focus and blur reach the program while it asks.
            cx.on_focus(&focus, window, |this, _, cx| {
                this.registry.focus(&this.session_id);
                this.restart_blink(cx);
                cx.notify();
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                this.registry.blur(&this.session_id);
                cx.notify();
            }),
            cx.on_release(|this, _| this.detach_now()),
        ];
        let phase = AttachPhase::new(registry.has_output(&session_id), registry.needs_open(&session_id));
        Self {
            session_id,
            project_id: project_id.into(),
            kind,
            config,
            terminals,
            registry,
            container,
            focus,
            phase,
            attempt: None,
            enabled: false,
            attached: false,
            fit: None,
            geometry: None,
            scale: 1.0,
            wheel_gate: None,
            focus_pending: false,
            shown: None,
            hold_timer: None,
            blink_on: true,
            blink_task: None,
            drag: None,
            drag_task: None,
            press: None,
            hovered_link: None,
            pressed_link: None,
            scrollbar: ScrollbarState::default(),
            marked: None,
            _subscriptions: subscriptions,
        }
    }

    /// Attaches while `enabled`, parks the terminal otherwise (TERM-092).
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<'_, Self>) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        if !enabled {
            self.detach_now();
        }
        cx.notify();
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// `reattach()`: the attach runs again, resuming an exited process (TERM-091).
    pub fn reattach(&mut self, cx: &mut Context<'_, Self>) {
        self.detach_now();
        cx.notify();
    }

    /// Whether the next attach starts or resumes a process (TERM-026).
    pub fn needs_open(&self) -> bool {
        self.registry.needs_open(&self.session_id)
    }

    pub fn is_loading(&self) -> bool {
        self.phase.phase == Phase::Loading
    }

    /// Gives the terminal keyboard focus (TERM-085): nothing while parked.
    pub fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.attached {
            return;
        }
        self.focus_pending = false;
        window.focus(&self.focus, cx);
        self.registry.focus(&self.session_id);
        self.restart_blink(cx);
    }

    /// Focuses the terminal now if it's attached here, else as soon as it is
    /// (an expanded card woken before its terminal was in place, TERM-095).
    pub fn request_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.attached {
            self.focus_terminal(window, cx);
        } else {
            self.focus_pending = true;
        }
    }

    /// The board's zoom for a card's terminal (TERM-083).
    pub fn set_scale(&mut self, scale: f32, cx: &mut Context<'_, Self>) {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        if self.scale != scale {
            self.scale = scale;
            cx.notify();
        }
    }

    /// The card's own wheel arbitration (TERM-104).
    pub fn set_wheel_gate(&mut self, gate: Option<WheelGate>) {
        self.wheel_gate = gate;
    }

    /// Whether this terminal has the keyboard.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// Types `text` in at the prompt (TERM-028).
    pub fn type_in(&self, text: &str, starting: bool) {
        self.registry.type_in(&self.session_id, text, starting);
    }

    /// Parks the terminal from here (TERM-003, TERM-004).
    fn detach_now(&mut self) {
        if self.attached {
            self.registry.detach(&self.session_id, Some(self.container));
        }
        self.attached = false;
        self.phase.cancel();
        self.attempt = None;
        self.marked = None;
        self.drag = None;
        self.drag_task = None;
        self.hovered_link = None;
        self.pressed_link = None;
    }

    /// Layout found the container's size: attach on the first one, refit after (TERM-016, TERM-023).
    pub(crate) fn layout(
        &mut self,
        size: gpui::Size<Pixels>,
        metrics: &TermMetrics,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let fit = nodus_terminal::fit::fit(f64::from(size.width), f64::from(size.height), &metrics.cell);
        if self.enabled && !self.attached {
            self.fit = fit;
            self.start_attach(fit, window, cx);
        } else if self.attached && fit != self.fit {
            self.fit = fit;
            self.registry.refit(&self.session_id, self.container, fit);
        }
    }

    /// The attach effect of `useAttachedTerminal` (TERM-086…TERM-090).
    fn start_attach(&mut self, fit: nodus_terminal::registry::Fit, window: &mut Window, cx: &mut Context<'_, Self>) {
        let needs_open = self.registry.needs_open(&self.session_id);
        let has_output = self.registry.has_output(&self.session_id);
        let attempt = self.phase.attach_started(needs_open, has_output);
        self.attempt = Some(attempt);
        self.attached = true;
        cx.emit(HostEvent::Phase);
        let registry = self.registry.clone();
        let (session, project, container, kind) = (
            self.session_id.to_string(),
            self.project_id.to_string(),
            self.container,
            self.kind,
        );
        let mut attach: AttachFuture =
            Box::pin(async move { registry.attach(&session, &project, container, kind, fit).await });
        // Everything up to its first wait runs now, so the terminal is in
        // place (and can take focus) before this frame is drawn.
        let mut task_cx = std::task::Context::from_waker(std::task::Waker::noop());
        let ready = match attach.as_mut().poll(&mut task_cx) {
            std::task::Poll::Ready(result) => Some(result),
            std::task::Poll::Pending => None,
        };
        match ready {
            Some(result) => self.attach_done(attempt, result, window, cx),
            None => {
                cx.spawn_in(window, async move |this, cx| {
                    let result = attach.await;
                    let _ = this.update_in(cx, |this, window, cx| this.attach_done(attempt, result, window, cx));
                })
                .detach();
            }
        }
        if self.config.focus_on_attach || self.focus_pending {
            // TERM-090: focused right after the attach starts (a card: once
            // when a wake asked for it before the terminal was here).
            self.defer_focus(window, cx);
        }
    }

    fn defer_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let this = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let _ = this.update(cx, |this, cx| this.focus_terminal(window, cx));
        });
    }

    fn attach_done(
        &mut self,
        attempt: Attempt,
        result: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // The open's reply is in: the session's held output can follow.
        self.terminals.read(cx).release(&self.session_id);
        let before = self.phase.clone();
        match result {
            Ok(()) => {
                self.phase.resolved(attempt, self.registry.has_output(&self.session_id));
                // TERM-090: and again when it resolves, unless it moved on.
                if self.config.focus_on_attach && self.attempt == Some(attempt) && self.attached {
                    self.focus_terminal(window, cx);
                }
            }
            Err(message) => self.phase.rejected(attempt, &message),
        }
        if self.phase != before {
            cx.emit(HostEvent::Phase);
        }
        cx.notify();
    }

    /// The cursor shows again and the blink restarts (TERM-076).
    pub(crate) fn restart_blink(&mut self, cx: &mut Context<'_, Self>) {
        self.blink_on = true;
        if parity::frozen(cx) {
            self.blink_task = None;
            return;
        }
        self.blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK).await;
                let go_on = this
                    .update(cx, |this, cx| {
                        this.blink_on = !this.blink_on;
                        cx.notify();
                    })
                    .is_ok();
                if !go_on {
                    break;
                }
            }
        }));
    }

    /// TERM-082: a held drawing is let go after a second.
    pub(crate) fn hold_started(&mut self, cx: &mut Context<'_, Self>) {
        if self.hold_timer.is_some() {
            return;
        }
        self.hold_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SYNC_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                this.hold_timer = None;
                this.registry
                    .with_terminal(&this.session_id, nodus_terminal::Terminal::end_synchronized_output);
                cx.notify();
            });
        }));
    }

    pub(crate) fn hold_ended(&mut self) {
        self.hold_timer = None;
    }

    // ---------------------------------------------------------------------
    // Keys

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let ks = &event.keystroke;
        // ⌘V: the Edit menu's Paste, which lands here (TERM-042).
        if cfg!(target_os = "macos")
            && ks.modifiers.platform
            && !ks.modifiers.control
            && !ks.modifiers.alt
            && ks.key == "v"
        {
            cx.stop_propagation();
            self.paste_clipboard(cx);
            return;
        }
        let mut key = keymap::key_event(ks, event.prefer_character_input, event.is_held);
        key.composing = self.marked.is_some();
        let Some(action) = self.registry.key_down(&self.session_id, &key) else {
            return;
        };
        match action {
            // TERM-049: a key the terminal took never reaches the board or the
            // app; TERM-109: so ⌘K (a local clear) and Ctrl+K (`\x0b`) never open
            // the board's search, which listens for keys, not a binding.
            KeyAction::Send(_) | KeyAction::WriteDirect(_) | KeyAction::Handled => {
                cx.stop_propagation();
                self.restart_blink(cx);
                self.hovered_link = None;
                cx.notify();
            }
            KeyAction::Copy(text) => {
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                cx.notify();
            }
            KeyAction::Paste => {
                cx.stop_propagation();
                self.paste_clipboard(cx);
            }
            KeyAction::Menu(command) => {
                cx.stop_propagation();
                self.terminals.read(cx).nodus.emit(Event::Menu(command));
            }
            // The app's menu and shortcuts, or the platform's text input, have it.
            KeyAction::Propagate | KeyAction::Unhandled | KeyAction::Composing => {}
        }
        let _ = window;
    }

    /// Pastes the clipboard's plain text; none (an image) still pastes the
    /// empty brackets Claude Code reacts to (TERM-035, TERM-044).
    fn paste_clipboard(&mut self, cx: &mut Context<'_, Self>) {
        let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        self.registry.paste(&self.session_id, &text);
        self.restart_blink(cx);
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // The pointer

    fn cell_at(&self, p: Point<Pixels>, selection: bool) -> Option<(usize, usize)> {
        let g = self.geometry?;
        let (x, y) = g.local(p);
        Some(pointer::pointer_cell(
            x,
            y,
            g.cols,
            g.rows,
            f64::from(g.cell_w),
            f64::from(g.cell_h),
            selection,
        ))
    }

    /// The selection point under the pointer: `BufferPoint` of the 1-based cell.
    fn buffer_point(&self, p: Point<Pixels>) -> Option<BufferPoint> {
        let (col, row) = self.cell_at(p, true)?;
        let offset = self.display_offset();
        Some(BufferPoint::new(row as i32 - 1 - offset as i32, col - 1))
    }

    fn display_offset(&self) -> usize {
        self.registry
            .with_terminal(&self.session_id, |t| t.display_offset())
            .unwrap_or(0)
    }

    /// The link under the pointer, if any (TERM-065, TERM-068).
    fn link_at(&self, p: Point<Pixels>) -> Option<Link> {
        let (col, row) = self.cell_at(p, false)?;
        self.registry
            .with_terminal(&self.session_id, |t| {
                let y = t.history_size() - t.display_offset() + row;
                t.link_at(LinkPoint { x: col, y })
            })
            .flatten()
    }

    fn mouse_reporting(&self, shift: bool) -> bool {
        self.registry
            .with_terminal(&self.session_id, |t| {
                t.mouse_events_active() && !t.should_force_selection(shift)
            })
            .unwrap_or(false)
    }

    /// A mouse report for the program (TERM-063, TERM-071).
    fn report(&mut self, p: Point<Pixels>, button: TermButton, action: MouseAction, m: &gpui::Modifiers) -> bool {
        let Some(g) = self.geometry else { return false };
        let (x, y) = g.local(p);
        let (w, h) = g.grid_size();
        let (col, row, px_x, px_y) = pointer::report_position(x, y, w, h, f64::from(g.cell_w), f64::from(g.cell_h));
        let event = MouseEvent {
            col,
            row,
            x: px_x,
            y: px_y,
            button,
            action,
            ctrl: m.control,
            alt: m.alt,
            shift: m.shift,
        };
        let report = self
            .registry
            .with_terminal(&self.session_id, |t| t.mouse_report(event))
            .flatten();
        if let Some(report) = report {
            self.registry.input(&self.session_id, &report);
        }
        true
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(g) = self.geometry else { return };
        if !g.contains(event.position) {
            return;
        }
        // TERM-100, MODAL-008: a press focuses the terminal.
        self.focus_terminal(window, cx);
        if g.over_scrollbar(event.position) && event.button == MouseButton::Left {
            self.scrollbar_press(event.position, cx);
            return;
        }
        if self.mouse_reporting(event.modifiers.shift) {
            let button = match event.button {
                MouseButton::Left => TermButton::Left,
                MouseButton::Middle => TermButton::Middle,
                MouseButton::Right => TermButton::Right,
                _ => return,
            };
            self.report(event.position, button, MouseAction::Down, &event.modifiers);
            return;
        }
        match event.button {
            MouseButton::Left => self.selection_down(event, cx),
            MouseButton::Right => {
                // TERM-061: no menu; a Mac selects the word.
                if let Some(p) = self.buffer_point_cell(event.position) {
                    self.registry.with_terminal(&self.session_id, |t| t.right_click(p));
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// The cell under the pointer (not rounded to an edge), as a buffer point.
    fn buffer_point_cell(&self, p: Point<Pixels>) -> Option<BufferPoint> {
        let (col, row) = self.cell_at(p, false)?;
        let offset = self.display_offset();
        Some(BufferPoint::new(row as i32 - 1 - offset as i32, col - 1))
    }

    /// xterm's `SelectionService._handleMouseDown` (TERM-056).
    fn selection_down(&mut self, event: &MouseDownEvent, cx: &mut Context<'_, Self>) {
        let Some(p) = self.buffer_point(event.position) else {
            return;
        };
        self.press = Some(Press {
            at: Instant::now(),
            alt: event.modifiers.alt,
        });
        self.pressed_link = self.link_at(event.position);
        let id = self.session_id.clone();
        let column = event.modifiers.alt;
        match event.click_count {
            2 => {
                // A word, or the link the pointer is on.
                let cell = self.buffer_point_cell(event.position).unwrap_or(p);
                let link = self.pressed_link.clone();
                self.registry.with_terminal(&id, |t| match link {
                    Some(link) => select_link(t, &link),
                    None => {
                        t.select_word_at(cell, true);
                    }
                });
                self.drag = Some(Drag {
                    anchor: cell,
                    unit: DragUnit::Word,
                    column: false,
                    scroll: 0,
                    last: event.position,
                });
            }
            n if n >= 3 => {
                let cell = self.buffer_point_cell(event.position).unwrap_or(p);
                self.registry.with_terminal(&id, |t| t.select_line_at(cell.line));
                self.drag = Some(Drag {
                    anchor: cell,
                    unit: DragUnit::Line,
                    column: false,
                    scroll: 0,
                    last: event.position,
                });
            }
            _ => {
                let has_selection = self.registry.with_terminal(&id, |t| t.has_selection()).unwrap_or(false);
                if event.modifiers.shift && has_selection {
                    self.registry.with_terminal(&id, |t| t.extend_selection(p));
                    let anchor = self
                        .registry
                        .with_terminal(&id, |t| t.selection_range().map(|(s, _, _)| s))
                        .flatten()
                        .unwrap_or(p);
                    self.drag = Some(Drag {
                        anchor,
                        unit: DragUnit::Char,
                        column,
                        scroll: 0,
                        last: event.position,
                    });
                } else {
                    self.registry
                        .with_terminal(&id, nodus_terminal::Terminal::clear_selection);
                    self.drag = Some(Drag {
                        anchor: p,
                        unit: DragUnit::Char,
                        column,
                        scroll: 0,
                        last: event.position,
                    });
                }
            }
        }
        cx.notify();
    }

    /// The selection follows the pointer (and scrolls past the edges).
    pub(crate) fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let Some(mut drag) = self.drag else { return };
        let Some(g) = self.geometry else { return };
        drag.last = position;
        // xterm's `_getMouseEventScrollAmount`.
        let (_, y) = g.local(position);
        let (_, height) = g.grid_size();
        drag.scroll = if y >= 0.0 && y <= height {
            0
        } else {
            let mut offset = if y > height { y - height } else { y };
            offset = offset.clamp(-DRAG_SCROLL_THRESHOLD, DRAG_SCROLL_THRESHOLD) / DRAG_SCROLL_THRESHOLD;
            (offset.signum() + (offset * (DRAG_SCROLL_MAX_SPEED - 1.0)).round()) as i32
        };
        self.drag = Some(drag);
        self.extend_drag(cx);
        if drag.scroll != 0 && self.drag_task.is_none() {
            self.drag_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(DRAG_SCROLL_EVERY).await;
                    let go_on = this
                        .update(cx, |this, cx| {
                            let Some(drag) = this.drag.filter(|d| d.scroll != 0) else {
                                this.drag_task = None;
                                return false;
                            };
                            this.registry
                                .with_terminal(&this.session_id, |t| t.scroll_lines(drag.scroll));
                            this.extend_drag(cx);
                            true
                        })
                        .unwrap_or(false);
                    if !go_on {
                        break;
                    }
                }
            }));
        }
    }

    fn extend_drag(&mut self, cx: &mut Context<'_, Self>) {
        let Some(drag) = self.drag else { return };
        let Some(g) = self.geometry else { return };
        // Past an edge the selection reaches the edge row.
        let (x, y) = g.local(drag.last);
        let (_, height) = g.grid_size();
        let y = y.clamp(0.0, (height - 1.0).max(0.0));
        let (col, row) = pointer::pointer_cell(x, y, g.cols, g.rows, f64::from(g.cell_w), f64::from(g.cell_h), true);
        let offset = self.display_offset();
        let to = BufferPoint::new(row as i32 - 1 - offset as i32, col - 1);
        let id = self.session_id.clone();
        self.registry.with_terminal(&id, |t| match drag.unit {
            DragUnit::Char => t.select_range(drag.anchor, to, drag.column),
            DragUnit::Word => {
                let cell = BufferPoint::new(to.line, to.col.min(g.cols.saturating_sub(1)));
                let a = t.word_range_at(drag.anchor, true);
                let b = t.word_range_at(cell, true);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        let start = a.0.min(b.0);
                        let end = a.1.max(b.1);
                        t.select_range(start, end, false);
                    }
                    (Some(a), None) => t.select_range(a.0.min(to), a.1.max(to), false),
                    _ => t.select_range(drag.anchor, to, false),
                }
            }
            DragUnit::Line => {
                let (a0, a1) = t.wrapped_range(drag.anchor.line);
                let (b0, b1) = t.wrapped_range(to.line);
                let cols = t.cols();
                t.select_range(
                    BufferPoint::new(a0.min(b0), 0),
                    BufferPoint::new(a1.max(b1), cols),
                    false,
                );
            }
        });
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(g) = self.geometry else { return };
        if let Some(grab) = self.scrollbar.grab {
            self.scrollbar_drag(event.position, grab, cx);
            return;
        }
        let over_bar = g.over_scrollbar(event.position);
        if over_bar != self.scrollbar.hovered {
            self.scrollbar.hovered = over_bar;
            cx.notify();
        }
        if self.drag.is_some() && event.pressed_button == Some(MouseButton::Left) {
            self.drag_to(event.position, cx);
            return;
        }
        if self.mouse_reporting(event.modifiers.shift) {
            let button = match event.pressed_button {
                Some(MouseButton::Left) => TermButton::Left,
                Some(MouseButton::Middle) => TermButton::Middle,
                Some(MouseButton::Right) => TermButton::Right,
                _ => TermButton::None,
            };
            if g.contains(event.position) {
                self.report(event.position, button, MouseAction::Move, &event.modifiers);
            }
            return;
        }
        // TERM-066: hovering a link underlines it.
        let link = if g.contains(event.position) && !over_bar {
            self.link_at(event.position)
        } else {
            None
        };
        if link != self.hovered_link {
            self.hovered_link = link;
            cx.notify();
        }
    }

    pub(crate) fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.scrollbar.grab.take().is_some() {
            cx.notify();
            return;
        }
        let pressed_link = self.pressed_link.take();
        let press = self.press.take();
        let dragging = self.drag.take().is_some();
        self.drag_task = None;
        if self.mouse_reporting(event.modifiers.shift) {
            if self.geometry.is_some_and(|g| g.contains(event.position)) {
                let button = match event.button {
                    MouseButton::Left => TermButton::Left,
                    MouseButton::Middle => TermButton::Middle,
                    MouseButton::Right => TermButton::Right,
                    _ => return,
                };
                self.report(event.position, button, MouseAction::Up, &event.modifiers);
            }
            return;
        }
        if !dragging {
            return;
        }
        // TERM-062: ⌥/Alt-click moves the cursor.
        if let Some(press) = press
            && event.modifiers.alt
            && press.alt
            && press.at.elapsed().as_millis() < ALT_CLICK_MS
            && let Some((col, row)) = self.cell_at(event.position, false)
        {
            let id = self.session_id.clone();
            let sequence = self
                .registry
                .with_terminal(&id, |t| {
                    let selected = t.selection_text();
                    (nodus_domain::js::utf16_len(&selected) <= 1)
                        .then(|| t.move_to_cell_sequence(col - 1, row - 1))
                        .flatten()
                })
                .flatten();
            if let Some(sequence) = sequence {
                self.registry.input(&id, &sequence);
            }
        }
        // TERM-067: released on the link it was pressed on.
        if let Some(link) = pressed_link
            && self.link_at(event.position).as_ref() == Some(&link)
        {
            self.activate_link(&link, window, cx);
        }
        cx.notify();
    }

    fn activate_link(&mut self, link: &Link, window: &mut Window, cx: &mut Context<'_, Self>) {
        match link.kind {
            LinkKind::Web => {
                if let Some(url) = links::browser_url(&link.text) {
                    nodus_platform::shell::open_external(&url);
                }
            }
            LinkKind::Osc8 => {
                // TERM-068: xterm's confirm; 0.9.4's window then opened nothing.
                let text = links::osc8_confirm_text(&link.text);
                // Whatever the answer, nothing opens.
                drop(window.prompt(gpui::PromptLevel::Info, &text, None, &["OK", "Cancel"], cx));
            }
        }
    }

    // ---------------------------------------------------------------------
    // The wheel and the scrollbar

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(g) = self.geometry else { return };
        // TERM-104…TERM-106: on the board, the gesture's first event decides.
        if self.config.card {
            let ours = match self.wheel_gate.clone() {
                Some(gate) => gate(event.modifiers.control, cx),
                None => {
                    cx.default_global::<BoardWheel>()
                        .0
                        .route(now_ms(), true, event.modifiers.control)
                        == WheelRoute::Terminal
                }
            };
            if !ours {
                return;
            }
        }
        let delta = match event.delta {
            ScrollDelta::Pixels(p) => WheelDelta::Pixels(-f64::from(p.y)),
            ScrollDelta::Lines(l) => WheelDelta::Lines(-f64::from(l.y)),
        };
        let mods = WheelModifiers {
            shift: event.modifiers.shift,
            alt: event.modifiers.alt,
            ctrl: event.modifiers.control,
        };
        let (x, y) = g.local(event.position);
        let (w, h) = g.grid_size();
        let (col, row, px_x, px_y) = pointer::report_position(x, y, w, h, f64::from(g.cell_w), f64::from(g.cell_h));
        let outcome = self
            .registry
            .with_terminal(&self.session_id, |t| {
                t.wheel(delta, mods, false, f64::from(g.cell_h), col, row, px_x, px_y)
            })
            .unwrap_or(WheelOutcome::Ignored);
        match outcome {
            WheelOutcome::Send(data) => {
                cx.stop_propagation();
                self.registry.input(&self.session_id, &data);
            }
            WheelOutcome::Scrolled => {
                cx.stop_propagation();
                self.scrollbar.active_at = Some(Instant::now());
                self.hovered_link = None;
                cx.notify();
            }
            WheelOutcome::Consumed => cx.stop_propagation(),
            WheelOutcome::Ignored => {
                // A card's terminal keeps its gesture even with nothing left to scroll.
                if self.config.card {
                    cx.stop_propagation();
                }
            }
        }
    }

    /// The slider's top and height within the strip, if there's anything to scroll.
    pub(crate) fn slider(&self, track_h: f32) -> Option<(f32, f32)> {
        let (history, offset, rows) = self
            .registry
            .with_terminal(&self.session_id, |t| (t.history_size(), t.display_offset(), t.rows()))?;
        if history == 0 {
            return None;
        }
        let total = (history + rows) as f32;
        let h = (track_h * rows as f32 / total).max(20.0).min(track_h);
        let top = (track_h - h) * (history - offset) as f32 / history as f32;
        Some((top, h))
    }

    fn scrollbar_press(&mut self, p: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let Some(g) = self.geometry else { return };
        let track_h = f32::from(g.bounds.size.height);
        let Some((top, h)) = self.slider(track_h) else { return };
        let y = f32::from(g.unscale(p).y - g.bounds.origin.y);
        let grab = if y >= top && y <= top + h { y - top } else { h / 2.0 };
        self.scrollbar.grab = Some(grab);
        self.scrollbar_drag(p, grab, cx);
    }

    fn scrollbar_drag(&mut self, p: Point<Pixels>, grab: f32, cx: &mut Context<'_, Self>) {
        let Some(g) = self.geometry else { return };
        let track_h = f32::from(g.bounds.size.height);
        let Some((_, h)) = self.slider(track_h) else { return };
        let y = f32::from(g.unscale(p).y - g.bounds.origin.y) - grab;
        let share = (y / (track_h - h).max(1.0)).clamp(0.0, 1.0);
        self.registry.with_terminal(&self.session_id, |t| {
            let history = t.history_size();
            let row = (share * history as f32).round() as usize;
            t.scroll_to_offset(history - row.min(history));
        });
        self.scrollbar.active_at = Some(Instant::now());
        cx.notify();
    }

    /// The pointer's shape over the terminal (TERM-064).
    pub(crate) fn cursor_style(&self, position: Point<Pixels>) -> gpui::CursorStyle {
        if self.geometry.is_some_and(|g| g.over_scrollbar(position)) {
            return gpui::CursorStyle::Arrow;
        }
        if self.hovered_link.is_some() {
            return gpui::CursorStyle::PointingHand;
        }
        if self.mouse_reporting(false) {
            return gpui::CursorStyle::Arrow;
        }
        gpui::CursorStyle::IBeam
    }

    /// A drag of the selection or the scrollbar is under way.
    pub(crate) fn is_dragging(&self) -> bool {
        self.drag.is_some() || self.scrollbar.grab.is_some()
    }

    pub(crate) fn mouse_moved_while_dragging(&mut self, position: Point<Pixels>, cx: &mut Context<'_, Self>) {
        if let Some(grab) = self.scrollbar.grab {
            self.scrollbar_drag(position, grab, cx);
        } else {
            self.drag_to(position, cx);
        }
    }

    /// The cursor cell's box in the window, for the IME's candidates (TERM-055).
    fn cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let g = self.geometry?;
        let cursor = self.registry.with_terminal(&self.session_id, |t| {
            let (line, col) = t.cursor_point();
            (line + t.display_offset() as i32, col)
        })?;
        let origin = point(
            g.origin.x + px(cursor.1 as f32 * g.cell_w),
            g.origin.y + px(cursor.0.max(0) as f32 * g.cell_h),
        );
        let bounds = Bounds::new(origin, gpui::size(px(g.cell_w), px(g.cell_h)));
        Some(match g.zoom {
            Some((at, zoom)) if zoom != 1.0 => Bounds::new(
                at + (bounds.origin - at) * zoom,
                gpui::size(bounds.size.width * zoom, bounds.size.height * zoom),
            ),
            _ => bounds,
        })
    }
}

/// Selects a link's cells (a double-click on a link, TERM-056).
fn select_link(t: &mut nodus_terminal::Terminal, link: &Link) {
    // Link rows count from the top of the history, 1-based.
    let top = -(t.history_size() as i32);
    let start = BufferPoint::new(top + link.start.y as i32 - 1, link.start.x.saturating_sub(1));
    let end = BufferPoint::new(top + link.end.y as i32 - 1, link.end.x);
    t.select_range(start, end, false);
}

impl EntityInputHandler for TerminalHost {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<UTF16Selection> {
        let at = self.marked.as_deref().map_or(0, nodus_domain::js::utf16_len);
        Some(UTF16Selection {
            range: at..at,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|m| 0..nodus_domain::js::utf16_len(m))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.marked = None;
        cx.notify();
    }

    /// Text typed through the platform: a character, or a finished composition (TERM-055).
    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.marked = None;
        self.registry.input_text(&self.session_id, text);
        self.restart_blink(cx);
        cx.notify();
    }

    /// The composition so far, drawn at the cursor (TERM-055).
    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds()
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let host = cx.entity();
        div()
            .id(("term-host", self.container.0 as usize))
            .key_context("Terminal")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_scroll))
            // The Edit menu's commands land in the focused terminal.
            .on_action(cx.listener(|this, _: &gpui::component::input::Paste, _, cx| this.paste_clipboard(cx)))
            .on_action(cx.listener(|this, _: &gpui::component::input::Copy, _, cx| {
                let text = this
                    .registry
                    .with_terminal(&this.session_id, |t| t.has_selection().then(|| t.selection_text()))
                    .flatten();
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }))
            .on_action(cx.listener(|this, _: &gpui::component::input::SelectAll, _, cx| {
                this.registry
                    .with_terminal(&this.session_id, nodus_terminal::Terminal::select_all);
                cx.notify();
            }))
            // TERM-032: files dropped type their quoted paths; TERM-034: anything else is ignored.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                let paths: Vec<String> = paths.paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
                this.registry.drop_paths(&this.session_id, &paths);
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .when(self.geometry.is_some(), |d| d)
            .child(TerminalElement::new(host))
    }
}
