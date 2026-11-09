//! Ink on the board (`InkLayer.tsx`, `ink.css`; spec 11 INKUI): pen lines,
//! arrows and text labels drawn over everything, and the dock of tools that
//! draws them, above the zoom controls. While a tool is out, the board's own
//! pointer handling stands aside; the pointer tool selects, moves and deletes
//! ink. The geometry and the undo history are `nodus_board::ink_draw`.
//!
//! Input is taken ahead of the board: the listeners are registered before
//! anything else on the canvas (the window's capture phase runs them in
//! paint order), and the layer's hitbox, inserted above the nodes and under
//! the HUD, tells whether a press is on the board's surface.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, CursorStyle, DispatchPhase, Entity, FontWeight, Global,
    Hitbox, HitboxBehavior, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, PathBuilder, PathStyle, Pixels, Point as GPoint, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TextRun, Window, WindowTextSystem, canvas, div, point, px,
};
use gpui::component::input::{Input, InputEvent, InputState};
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};
use nodus_board::ink_draw::{
    self, ARROW_HEAD, HIT_PX, InkHistory, LABEL_FONT, LABEL_LINE, MIN_ARROW_PX, MIN_STEP_PX, MOVE_PX, PEN_WIDTH, Pt,
    Seg, distance, flatten, points_of, simplify, size_at, stroke_at, stroke_box, strokes_along, translate,
};
use nodus_domain::{InkColor, InkKind, InkStroke, js};

use super::icons::{OvIcon, ov_icon};
use crate::board::canvas::BoardCanvas;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::platform::keys;
use crate::theme::tokens::{Tokens, inset_shadow, ring};
use crate::theme::{self, fonts, motion, tokens::DUR_FAST};

/// The ink tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InkTool {
    #[default]
    Select,
    Pen,
    Arrow,
    Text,
    Eraser,
}

/// The dock's tools: tool, name, its key.
const TOOLS: [(InkTool, &str, Option<&str>); 5] = [
    (InkTool::Select, "Pointer", Some("Esc")),
    (InkTool::Pen, "Pen", Some("P")),
    (InkTool::Arrow, "Arrow", Some("A")),
    (InkTool::Text, "Text", None),
    (InkTool::Eraser, "Eraser", Some("X")),
];

/// INKUI-002: the board's single-key shortcuts for the tools.
pub fn tool_for_key(key: &str) -> Option<InkTool> {
    match key {
        "p" => Some(InkTool::Pen),
        "a" => Some(InkTool::Arrow),
        "x" => Some(InkTool::Eraser),
        _ => None,
    }
}

pub fn color_name(color: InkColor) -> &'static str {
    match color {
        InkColor::Graphite => "Graphite",
        InkColor::Red => "Red",
        InkColor::Orange => "Orange",
        InkColor::Green => "Green",
        InkColor::Blue => "Blue",
        InkColor::Purple => "Purple",
    }
}

/// `var(--ink-<colour>)`: graphite flips with the theme.
pub fn ink_rgba(t: &Tokens, color: InkColor) -> Rgba {
    match color {
        InkColor::Graphite => t.ink_graphite,
        InkColor::Red => t.ink_red,
        InkColor::Orange => t.ink_orange,
        InkColor::Green => t.ink_green,
        InkColor::Blue => t.ink_blue,
        InkColor::Purple => t.ink_purple,
    }
}

/// INKUI-004: the colour picked last, for every board while the app runs
/// (`nodus.inkColor`); red until one is picked.
struct InkColorMemory(InkColor);

impl Global for InkColorMemory {}

fn remembered_color(cx: &App) -> InkColor {
    cx.try_global::<InkColorMemory>().map_or(InkColor::Red, |m| m.0)
}

/// A label being typed: a new one, or one already on the board (`id`).
pub(crate) struct LabelDraft {
    id: Option<String>,
    x: f64,
    y: f64,
    size: f64,
    color: InkColor,
    input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

enum InkGesture {
    Draw { base: InkStroke, pts: Vec<Pt>, zoom: f64 },
    Erase { gone: Vec<String>, last: Pt, tolerance: f64 },
    Move { id: String, start: Pt, dx: f64, dy: f64, moved: bool, zoom: f64 },
}

/// The ink layer's state, per board mount.
#[derive(Default)]
pub(crate) struct Ink {
    pub tool: InkTool,
    /// Picked on this board; the remembered colour until then.
    color: Option<InkColor>,
    pub selected: Option<String>,
    gesture: Option<InkGesture>,
    label: Option<LabelDraft>,
    pub palette: bool,
    history: InkHistory,
    /// The layer's hitbox: a press is on the board when it's hovered.
    hitbox: Option<Hitbox>,
    /// The colour button and its palette, for the outside press.
    color_area: Vec<Bounds<Pixels>>,
    label_key: u64,
}

impl Ink {
    pub fn color(&self, cx: &App) -> InkColor {
        self.color.unwrap_or_else(|| remembered_color(cx))
    }

    /// Lets go of a gesture under way (the window lost the pointer).
    pub fn drop_gesture(&mut self) {
        self.gesture = None;
    }

    /// Whether a label is being typed in this field.
    pub fn label_focused(&self, window: &Window, cx: &App) -> bool {
        self.label
            .as_ref()
            .is_some_and(|l| l.input.read(cx).focus_handle(cx).is_focused(window))
    }
}

/// `labelWidth`: the label's text set in the display font, weight 600, at
/// `16 × size` px, in board units.
pub fn label_width(text_system: &WindowTextSystem, text: &str, size: f64) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let font = gpui::Font {
        weight: FontWeight::SEMIBOLD,
        ..gpui::font(fonts::DISPLAY)
    };
    let run = TextRun {
        len: text.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = text_system.shape_line(
        SharedString::from(text.to_owned()),
        px((LABEL_FONT * size) as f32),
        &[run],
        None,
    );
    f64::from(f32::from(shaped.width))
}

fn measurer(window: &Window) -> impl Fn(&str, f64) -> f64 {
    let ts: Arc<WindowTextSystem> = window.text_system().clone();
    move |text: &str, size: f64| label_width(&ts, text, size)
}

impl BoardCanvas {
    // ---- the strokes ------------------------------------------------------------------

    /// The board's ink as it stands (`strokesRef.current`).
    pub(crate) fn ink_strokes(&self) -> Vec<InkStroke> {
        self.board.as_ref().map(|b| b.ink.clone()).unwrap_or_default()
    }

    /// INKUI-024: the ink changes at once on the board and is saved.
    fn set_ink(&mut self, next: Vec<InkStroke>, cx: &mut Context<'_, Self>) {
        if let Some(b) = self.board.as_mut() {
            b.ink = next.clone();
        }
        if let Some(s) = self.source.as_mut() {
            s.ink = next.clone();
        }
        self.state.update(cx, |s, cx| s.save_ink(next, cx));
        cx.notify();
    }

    /// A change ⌘Z can take back (`commit`).
    fn ink_commit(&mut self, next: Vec<InkStroke>, cx: &mut Context<'_, Self>) {
        let current = self.ink_strokes();
        self.overlays.ink.history.commit(current);
        self.set_ink(next, cx);
    }

    // ---- the keys and the dock --------------------------------------------------------

    /// INKUI-003: choosing a tool finishes a label; any tool but the pointer
    /// lets go of the selected stroke.
    pub(crate) fn ink_set_tool(&mut self, tool: InkTool, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        self.ink_finish_label(window, cx);
        self.overlays.ink.tool = tool;
        if tool != InkTool::Select {
            self.overlays.ink.selected = None;
        }
        cx.notify();
    }

    /// INKUI-023 (BOARD-160): back to the pointer, or lets go of the
    /// selected stroke. False when there was nothing to do.
    pub(crate) fn ink_escape(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.overlays.ink.tool != InkTool::Select {
            self.ink_set_tool(InkTool::Select, None, cx);
            return true;
        }
        if self.overlays.ink.selected.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// INKUI-022 (BOARD-163): ⌫ deletes the selected stroke.
    pub(crate) fn ink_delete_selected(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(id) = self.overlays.ink.selected.take() else {
            return false;
        };
        let next = self.ink_strokes().into_iter().filter(|s| s.id != id).collect();
        self.ink_commit(next, cx);
        true
    }

    /// INKUI-021 (BOARD-159): ⌘Z. False when there's nothing to undo.
    pub(crate) fn ink_undo(&mut self, cx: &mut Context<'_, Self>) -> bool {
        self.ink_finish_label(None, cx);
        let current = self.ink_strokes();
        let Some(prev) = self.overlays.ink.history.undo(current) else {
            return false;
        };
        self.overlays.ink.selected = None;
        self.set_ink(prev, cx);
        true
    }

    /// INKUI-021: ⇧⌘Z.
    pub(crate) fn ink_redo(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let current = self.ink_strokes();
        let Some(next) = self.overlays.ink.history.redo(current) else {
            return false;
        };
        self.overlays.ink.selected = None;
        self.set_ink(next, cx);
        true
    }

    /// INKUI-004: a colour from the palette; a selected stroke takes it too.
    fn ink_pick_color(&mut self, color: InkColor, cx: &mut Context<'_, Self>) {
        self.overlays.ink.color = Some(color);
        self.overlays.ink.palette = false;
        cx.set_global(InkColorMemory(color));
        let current = self.ink_strokes();
        if let Some(id) = self.overlays.ink.selected.clone()
            && current.iter().any(|s| s.id == id && s.color != color)
        {
            let next = current
                .into_iter()
                .map(|s| if s.id == id { InkStroke { color, ..s } } else { s })
                .collect();
            self.ink_commit(next, cx);
        }
        cx.notify();
    }

    /// INKUI-005.
    fn ink_clear_all(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ink_finish_label(Some(window), cx);
        if self.ink_strokes().is_empty() {
            return;
        }
        self.overlays.ink.selected = None;
        self.ink_commit(Vec::new(), cx);
        self.show_toast(format!("Cleared the ink. Press {} to bring it back.", keys("⌘Z")), cx);
    }

    /// A gesture the system took away (INKUI-012), or the window let go.
    pub(crate) fn ink_cancel(&mut self, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.gesture.take().is_some() {
            cx.notify();
        }
    }

    // ---- labels -----------------------------------------------------------------------

    /// INKUI-013, INKUI-014: puts a label being typed on the board (or
    /// takes an edited one off, emptied).
    fn ink_finish_label(&mut self, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        let Some(draft) = self.overlays.ink.label.take() else {
            return;
        };
        if let Some(window) = window
            && draft.input.read(cx).focus_handle(cx).is_focused(window)
        {
            window.focus(&self.focus, cx);
        }
        let text = js::trim(&draft.input.read(cx).value()).to_owned();
        let current = self.ink_strokes();
        match &draft.id {
            Some(id) => {
                let Some(old) = current.iter().find(|s| &s.id == id) else {
                    cx.notify();
                    return;
                };
                if old.text.as_deref() == Some(text.as_str()) {
                    cx.notify();
                    return;
                }
                let next = if text.is_empty() {
                    current.into_iter().filter(|s| &s.id != id).collect()
                } else {
                    current
                        .into_iter()
                        .map(|s| {
                            if &s.id == id {
                                InkStroke {
                                    text: Some(text.clone()),
                                    ..s
                                }
                            } else {
                                s
                            }
                        })
                        .collect()
                };
                self.ink_commit(next, cx);
            }
            None if !text.is_empty() => {
                let mut next = current;
                next.push(InkStroke {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: InkKind::Text,
                    color: draft.color,
                    points: flatten(&[[draft.x, draft.y]]),
                    size: draft.size,
                    text: Some(text),
                    extra: serde_json::Map::new(),
                });
                self.ink_commit(next, cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// Esc in a label: a new one stays off the board, an edited one as it was.
    pub(crate) fn ink_discard_label(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.label.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Return in a label.
    pub(crate) fn ink_commit_label(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ink_finish_label(Some(window), cx);
    }

    pub(crate) fn ink_label_focused(&self, window: &Window, cx: &App) -> bool {
        self.overlays.ink.label_focused(window, cx)
    }

    /// Whether an input method is composing in the label.
    pub(crate) fn ink_label_composing(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        use gpui::EntityInputHandler as _;
        let Some(label) = &self.overlays.ink.label else { return false };
        label.input.update(cx, |s, cx| s.marked_text_range(window, cx)).is_some()
    }

    fn ink_open_label(
        &mut self,
        id: Option<String>,
        at: Pt,
        size: f64,
        color: InkColor,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = text.to_owned();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Label").context_menu(false);
            state.set_value(text, window, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Blur => this.ink_finish_label(Some(window), cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
        ];
        self.overlays.ink.label_key += 1;
        self.overlays.ink.label = Some(LabelDraft {
            id,
            x: at[0],
            y: at[1],
            size,
            color,
            input: input.clone(),
            _subscriptions: subscriptions,
        });
        // Focused once drawn, with the caret at the end.
        window.on_next_frame(move |window, cx| {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                let end = s.value().len();
                s.set_cursor_position(end, window, cx);
            });
        });
        cx.notify();
    }

    // ---- the pointer ------------------------------------------------------------------

    /// The pointer's place in board units.
    fn ink_point(&self, p: GPoint<Pixels>) -> Pt {
        let f = self.to_flow(p);
        [f.x, f.y]
    }

    /// Whether `p` is on the label being typed.
    fn on_label_input(&self, p: GPoint<Pixels>, window: &Window, cx: &App) -> bool {
        let Some(label) = &self.overlays.ink.label else { return false };
        let text = label.input.read(cx).value();
        let w = label_width(window.text_system(), if text.is_empty() { "Label" } else { &text }, label.size).max(48.0);
        let q = self.ink_point(p);
        let pad = 3.0;
        q[0] >= label.x - pad
            && q[0] <= label.x + w + pad
            && q[1] >= label.y - pad
            && q[1] <= label.y + LABEL_LINE * label.size + pad
    }

    /// INKUI-007: the press is the ink's: nothing under it sees it, a
    /// label being typed is finished, and the keyboard comes back to the board.
    fn ink_take(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        cx.stop_propagation();
        window.prevent_default();
        self.ink_finish_label(Some(window), cx);
        if !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        }
    }

    /// INKUI-006…INKUI-015, INKUI-019, INKUI-020: a press, seen before
    /// anything on the board.
    pub(crate) fn ink_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        // INKUI-004: the palette closes on a press anywhere else.
        if self.overlays.ink.palette && !self.overlays.ink.color_area.iter().any(|b| b.contains(&e.position)) {
            self.overlays.ink.palette = false;
            cx.notify();
        }
        if e.button != MouseButton::Left || self.space {
            return;
        }
        let on_board = self.overlays.ink.hitbox.as_ref().is_some_and(|h| h.is_hovered(window));
        if !on_board || self.on_label_input(e.position, window, cx) {
            return;
        }
        let p = self.ink_point(e.position);
        let zoom = self.viewport.zoom;
        let tool = self.overlays.ink.tool;
        if tool == InkTool::Select {
            let measure = measurer(window);
            let strokes = self.ink_strokes();
            let Some(hit) = stroke_at(&strokes, p, HIT_PX / zoom, &measure).cloned() else {
                if self.overlays.ink.selected.take().is_some() {
                    cx.notify();
                }
                return;
            };
            self.ink_take(window, cx);
            // INKUI-014: a double-click on a label edits it in place.
            if e.click_count >= 2 && hit.kind == InkKind::Text {
                self.overlays.ink.selected = None;
                let at = points_of(&hit)[0];
                let text = hit.text.clone().unwrap_or_default();
                self.ink_open_label(Some(hit.id.clone()), at, hit.size, hit.color, &text, window, cx);
                return;
            }
            // INKUI-015: selected, the cards let go, and it follows the pointer.
            self.overlays.ink.selected = Some(hit.id.clone());
            self.clear_selection(cx);
            self.overlays.ink.gesture = Some(InkGesture::Move {
                id: hit.id,
                start: p,
                dx: 0.0,
                dy: 0.0,
                moved: false,
                zoom,
            });
            cx.notify();
            return;
        }
        self.ink_take(window, cx);
        let color = self.overlays.ink.color(cx);
        match tool {
            InkTool::Pen | InkTool::Arrow => {
                self.overlays.ink.gesture = Some(InkGesture::Draw {
                    base: InkStroke {
                        id: uuid::Uuid::new_v4().to_string(),
                        kind: if tool == InkTool::Pen { InkKind::Pen } else { InkKind::Arrow },
                        color,
                        points: Vec::new(),
                        size: size_at(zoom),
                        text: None,
                        extra: serde_json::Map::new(),
                    },
                    pts: vec![p],
                    zoom,
                });
            }
            InkTool::Eraser => {
                let tolerance = HIT_PX / zoom;
                self.overlays.ink.gesture = Some(InkGesture::Erase {
                    gone: Vec::new(),
                    last: p,
                    tolerance,
                });
                self.ink_erase_along(p, p, window, cx);
            }
            InkTool::Text => {
                let size = size_at(zoom);
                // The click lands mid-line, where the caret shows.
                let at = [p[0], p[1] - LABEL_LINE * size / 2.0];
                self.ink_open_label(None, at, size, color, "", window, cx);
            }
            InkTool::Select => {}
        }
        cx.notify();
    }

    /// INKUI-011: the strokes the eraser passed over from `a` to `b`.
    fn ink_erase_along(&mut self, a: Pt, b: Pt, window: &Window, cx: &mut Context<'_, Self>) {
        let strokes = self.ink_strokes();
        let measure = measurer(window);
        let Some(InkGesture::Erase { gone, tolerance, .. }) = &mut self.overlays.ink.gesture else {
            return;
        };
        let left: Vec<InkStroke> = strokes.into_iter().filter(|s| !gone.contains(&s.id)).collect();
        let hit = strokes_along(&left, a, b, *tolerance, &measure);
        if !hit.is_empty() {
            gone.extend(hit);
            cx.notify();
        }
    }

    /// The pointer moving while the ink holds it (INKUI-007).
    pub(crate) fn ink_mouse_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.gesture.is_none() {
            return;
        }
        cx.stop_propagation();
        let q = self.ink_point(e.position);
        match &mut self.overlays.ink.gesture {
            Some(InkGesture::Draw { base, pts, zoom }) => {
                if base.kind == InkKind::Arrow {
                    if pts.len() < 2 {
                        pts.push(q);
                    } else {
                        pts[1] = q;
                    }
                } else if pts.last().is_some_and(|&last| distance(q, last) * *zoom >= MIN_STEP_PX) {
                    pts.push(q);
                }
            }
            Some(InkGesture::Erase { last, .. }) => {
                let from = std::mem::replace(last, q);
                self.ink_erase_along(from, q, window, cx);
            }
            Some(InkGesture::Move {
                start,
                dx,
                dy,
                moved,
                zoom,
                ..
            }) => {
                *dx = q[0] - start[0];
                *dy = q[1] - start[1];
                if !*moved && dx.hypot(*dy) * *zoom >= MOVE_PX {
                    *moved = true;
                }
            }
            None => {}
        }
        cx.notify();
    }

    /// The pointer lifting: the gesture's change goes in (INKUI-008…INKUI-015).
    pub(crate) fn ink_mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<'_, Self>) {
        let Some(gesture) = self.overlays.ink.gesture.take() else {
            return;
        };
        cx.stop_propagation();
        let current = self.ink_strokes();
        match gesture {
            InkGesture::Draw { base, pts, zoom } => {
                if base.kind == InkKind::Arrow && (pts.len() < 2 || distance(pts[0], pts[1]) * zoom < MIN_ARROW_PX) {
                    cx.notify();
                    return;
                }
                let kept = if base.kind == InkKind::Pen {
                    simplify(&pts, 0.5 / zoom)
                } else {
                    pts
                };
                let mut next = current;
                next.push(InkStroke {
                    points: flatten(&kept),
                    ..base
                });
                self.ink_commit(next, cx);
            }
            InkGesture::Erase { gone, .. } => {
                if gone.is_empty() {
                    cx.notify();
                    return;
                }
                if self.overlays.ink.selected.as_ref().is_some_and(|s| gone.contains(s)) {
                    self.overlays.ink.selected = None;
                }
                let next = current.into_iter().filter(|s| !gone.contains(&s.id)).collect();
                self.ink_commit(next, cx);
            }
            InkGesture::Move {
                id, dx, dy, moved, ..
            } => {
                if !moved {
                    cx.notify();
                    return;
                }
                let next = current
                    .into_iter()
                    .map(|s| if s.id == id { translate(&s, dx, dy) } else { s })
                    .collect();
                self.ink_commit(next, cx);
            }
        }
        cx.notify();
    }

    // ---- drawing ----------------------------------------------------------------------

    /// The listeners that take the pointer ahead of the board: drawn first,
    /// so the window's capture phase runs them before any other.
    pub(crate) fn render_ink_input(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let me = cx.entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.ink_mouse_down(e, window, cx));
                    }
                });
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.ink_mouse_move(e, window, cx));
                    }
                });
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.ink_mouse_up(e, cx));
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    /// INKUI-025, INKUI-026: lines in stored order, the live stroke, the
    /// selection outline, then labels and the label being typed; above every
    /// card, under the open archive panel.
    pub(crate) fn render_ink_layer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let ink = &self.overlays.ink;
        let mut shown = self.ink_strokes();
        if let Some(InkGesture::Move {
            id,
            dx,
            dy,
            moved: true,
            ..
        }) = &ink.gesture
        {
            for s in &mut shown {
                if &s.id == id {
                    *s = translate(s, *dx, *dy);
                }
            }
        }
        let erasing: Vec<String> = match &ink.gesture {
            Some(InkGesture::Erase { gone, .. }) => gone.clone(),
            _ => Vec::new(),
        };
        let live = match &ink.gesture {
            Some(InkGesture::Draw { base, pts, .. }) => Some(InkStroke {
                points: flatten(pts),
                ..base.clone()
            }),
            _ => None,
        };
        let editing = ink.label.as_ref().and_then(|l| l.id.clone());
        let measure = measurer(window);
        let picked = ink
            .selected
            .as_ref()
            .and_then(|id| shown.iter().find(|s| &s.id == id))
            .map(|s| (stroke_box(s, &measure), s.size));
        let tool = ink.tool;
        let vp = self.viewport;
        let z = vp.zoom;

        let lines: Vec<(Vec<Seg>, f64, Rgba)> = shown
            .iter()
            .filter(|s| s.kind != InkKind::Text)
            .chain(live.iter())
            .map(|s| {
                let mut color = ink_rgba(&t, s.color);
                if erasing.contains(&s.id) {
                    color.a *= 0.25;
                }
                (ink_draw::stroke_path(s), PEN_WIDTH * s.size, color)
            })
            .collect();
        let accent = t.accent;
        let me = cx.entity();
        let layer = canvas(
            move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, cx| {
                me.update(cx, |c, _| c.overlays.ink.hitbox = Some(hitbox.clone()));
                // INKUI-017: the tool's cursor wins over the cards' own.
                match tool {
                    InkTool::Pen | InkTool::Arrow => window.set_cursor_style(CursorStyle::Crosshair, &hitbox),
                    InkTool::Text => window.set_cursor_style(CursorStyle::IBeam, &hitbox),
                    InkTool::Eraser => window.set_cursor_style(CursorStyle::Crosshair, &hitbox),
                    InkTool::Select => {}
                }
                let at = |p: Pt| {
                    point(
                        bounds.origin.x + px((vp.x + p[0] * z) as f32),
                        bounds.origin.y + px((vp.y + p[1] * z) as f32),
                    )
                };
                for (segs, width, color) in &lines {
                    paint_line(segs, (width * z) as f32, *color, &at, window);
                }
                if let Some((b, size)) = picked {
                    let pad = 6.0 * size;
                    let r = (4.0 * size * z) as f32;
                    let origin = at([b.x - pad, b.y - pad]);
                    let (w, h) = (((b.w + 2.0 * pad) * z) as f32, ((b.h + 2.0 * pad) * z) as f32);
                    paint_dashed_rect(origin, w, h, r, 1.5, (5.0, 4.0), accent, window);
                }
            },
        )
        .absolute()
        .size_full();

        let labels: Vec<AnyElement> = shown
            .iter()
            .filter(|s| s.kind == InkKind::Text && Some(&s.id) != editing.as_ref())
            .map(|s| {
                let mut color = ink_rgba(&t, s.color);
                if erasing.contains(&s.id) {
                    color.a *= 0.25;
                }
                label_element(
                    (vp.x + s.points[0] * z) as f32,
                    (vp.y + s.points.get(1).copied().unwrap_or(0.0) * z) as f32,
                    (s.size * z) as f32,
                    color,
                )
                .child(s.text.clone().unwrap_or_default())
                .into_any_element()
            })
            .collect();

        let input = self.render_label_input(&t, window, cx);
        div()
            .absolute()
            .size_full()
            .child(layer)
            .children(labels)
            .children(input)
            .into_any_element()
    }

    /// The label being typed (`.ink-label-input`): in the label's font and
    /// colour, as wide as its text (at least 48 units), a dashed accent
    /// outline 3 units outside it.
    fn render_label_input(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let label = self.overlays.ink.label.as_ref()?;
        let vp = self.viewport;
        let z = vp.zoom as f32;
        let size = label.size as f32;
        let color = ink_rgba(t, label.color);
        let text = label.input.read(cx).value().to_string();
        let measured = label_width(
            window.text_system(),
            if text.is_empty() { "Label" } else { &text },
            label.size,
        ) as f32;
        let w = measured.max(48.0) * z + 2.0;
        let h = 20.0 * size * z;
        let (left, top) = ((vp.x as f32) + label.x as f32 * z, (vp.y as f32) + label.y as f32 * z);
        let off = 3.0 * z;
        let outline = 1.5 * z;
        let accent = t.accent;
        let ring = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                paint_dashed_rect(
                    point(o.x - px(off), o.y - px(off)),
                    w + 2.0 * off,
                    h + 2.0 * off,
                    (2.0 + 3.0) * z,
                    outline,
                    (3.0 * outline, 3.0 * outline),
                    accent,
                    window,
                );
            },
        )
        .absolute()
        .size_full();
        let placeholder = text.is_empty().then(|| {
            label_element(0.0, 0.0, size * z, Rgba { a: color.a * 0.4, ..color }).child("Label")
        });
        Some(
            div()
                .id(("ink-label-input", self.overlays.ink.label_key))
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .occlude()
                .font_family(fonts::DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(LABEL_FONT as f32 * size * z))
                .line_height(px(h))
                .text_color(color)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(ring)
                .children(placeholder)
                .child(
                    Input::new(&label.input)
                        .appearance(false)
                        .px(px(0.))
                        .py(px(0.))
                        .h(px(h))
                        .text_size(px(LABEL_FONT as f32 * size * z))
                        .line_height(px(h)),
                )
                .into_any_element(),
        )
    }

    /// INKUI-001: the dock, 16px from the left, its bottom 58px up.
    pub(crate) fn render_ink_dock(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let tool = self.overlays.ink.tool;
        let color = self.overlays.ink.color(cx);
        let palette = self.overlays.ink.palette;
        let empty = self.ink_strokes().is_empty();
        let me = cx.entity().downgrade();

        let mut children: Vec<AnyElement> = Vec::new();
        for (each, name, key) in TOOLS {
            let me = me.clone();
            let glyph = match each {
                InkTool::Select => ov_icon(OvIcon::MousePointer2, 15., 2., Rgba::default()),
                InkTool::Pen => icon(Icon::Pencil, 15., 2., Rgba::default()),
                InkTool::Arrow => ov_icon(OvIcon::MoveUpRight, 15., 2., Rgba::default()),
                InkTool::Text => ov_icon(OvIcon::Type, 15., 2., Rgba::default()),
                InkTool::Eraser => ov_icon(OvIcon::Eraser, 15., 2., Rgba::default()),
            };
            let title = match key {
                Some(k) => format!("{name} ({k})"),
                None => name.to_owned(),
            };
            children.push(tool_button(
                ("ink-tool", each as usize),
                &t,
                tool == each,
                false,
                title,
                move |el| el.child(glyph),
                move |window, cx| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, |c, cx| c.ink_set_tool(each, Some(window), cx));
                    }
                },
                window,
                cx,
            ));
        }
        children.push(div().w(px(16.)).h(px(1.)).my(px(3.)).bg(t.separator).into_any_element());

        // The colour button and its palette.
        let toggle = me.clone();
        let dot = ink_rgba(&t, color);
        let color_button = tool_button(
            "ink-color",
            &t,
            false,
            false,
            "Ink colour".to_owned(),
            move |el| {
                el.child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(dot)
                        .shadow(vec![inset_shadow(0., 0., 0., 0.5, crate::theme::tokens::rgba(0, 0, 0, 0.25))]),
                )
            },
            move |_, cx| {
                if let Some(c) = toggle.upgrade() {
                    c.update(cx, |c, cx| {
                        c.overlays.ink.palette = !c.overlays.ink.palette;
                        cx.notify();
                    });
                }
            },
            window,
            cx,
        );
        let palette_el = palette.then(|| self.render_ink_palette(&t, color, window, cx));
        let area = me.clone();
        children.push(
            div()
                .relative()
                .child(color_button)
                .children(palette_el)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, cx| {
                            if let Some(c) = area.upgrade() {
                                c.update(cx, |c, _| {
                                    c.overlays.ink.color_area.clear();
                                    c.overlays.ink.color_area.push(bounds);
                                    // The palette: to the right, 168×34 around the button's middle.
                                    c.overlays.ink.color_area.push(Bounds::new(
                                        point(bounds.origin.x + px(38.), bounds.origin.y - px(3.)),
                                        gpui::size(px(168.), px(34.)),
                                    ));
                                });
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .into_any_element(),
        );

        let clear = me.clone();
        children.push(tool_button(
            "ink-clear",
            &t,
            false,
            empty,
            "Clear all ink".to_owned(),
            move |el| el.child(icon(Icon::Trash2, 15., 2., Rgba::default())),
            move |window, cx| {
                if let Some(c) = clear.upgrade() {
                    c.update(cx, |c, cx| c.ink_clear_all(window, cx));
                }
            },
            window,
            cx,
        ));

        div()
            .id("ink-dock")
            .absolute()
            .left(px(16.))
            .bottom(px(58.))
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(11.))
            .map(|d| frosted(d, super::super::hud::rgba_mix(t.bg_elevated, 0.86), 20., 1.6))
            .shadow(t.md())
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                // Pressing the dock never moves the keyboard.
                window.prevent_default();
                cx.stop_propagation();
            })
            .children(children)
            .into_any_element()
    }

    /// INKUI-004: the palette, to the right of the colour button.
    fn render_ink_palette(&mut self, t: &Tokens, current: InkColor, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let rise = enter(window, cx, "ink-palette-in", DUR_FAST, motion::ease_out);
        let me = cx.entity().downgrade();
        let swatches = InkColor::ALL.iter().map(|&c| {
            let me = me.clone();
            let mut shadows = Vec::new();
            if c == current {
                shadows.push(ring(3.5, t.accent));
                shadows.push(ring(2., t.bg_elevated));
            }
            shadows.push(inset_shadow(0., 0., 0., 0.5, crate::theme::tokens::rgba(0, 0, 0, 0.25)));
            div()
                .id(("ink-swatch", c as usize))
                .size(px(18.))
                .flex_none()
                .rounded_full()
                .bg(ink_rgba(t, c))
                .shadow(shadows)
                .tooltip(native_tooltip(color_name(c).into()))
                .on_click(move |_, _, cx| {
                    if let Some(canvas) = me.upgrade() {
                        canvas.update(cx, |canvas, cx| canvas.ink_pick_color(c, cx));
                    }
                })
        });
        div()
            .absolute()
            .left(px(38.))
            .top(px(-3.))
            .occlude()
            .flex()
            .gap(px(8.))
            .py(px(8.))
            .px(px(10.))
            .rounded(px(12.))
            .map(|d| frosted(d, super::super::hud::rgba_mix(t.bg_elevated, 0.92), 24., 1.8))
            .shadow(t.lg())
            .group_opacity(rise)
            .translate(px(-4. * (1. - rise)), px(0.))
            .children(swatches)
            .into_any_element()
    }
}

/// A label's box (`.ink-label`): display font, weight 600, one line.
fn label_element(left: f32, top: f32, scale: f32, color: Rgba) -> gpui::Div {
    let h = LABEL_LINE as f32 * scale;
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .h(px(h))
        .font_family(fonts::DISPLAY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(LABEL_FONT as f32 * scale))
        .line_height(px(h))
        .text_color(color)
        .whitespace_nowrap()
}

/// `.ink-tool`: 28×28, radius 8; hover, pressed and on looks.
#[allow(clippy::too_many_arguments)]
fn tool_button(
    id: impl Into<gpui::ElementId>,
    t: &Tokens,
    on: bool,
    disabled: bool,
    title: String,
    content: impl FnOnce(gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id: gpui::ElementId = id.into();
    let hovered = window.use_keyed_state((id.clone(), "hover"), cx, |_, _| false);
    let hover = *hovered.read(cx) && !disabled;
    let (bg, fg) = if on {
        (t.bg_selected, t.accent)
    } else if hover {
        (t.bg_hover, t.text_primary)
    } else {
        (crate::components::button::transparent(), t.text_secondary)
    };
    let pressed_bg = t.bg_pressed;
    let el = div()
        .id(id)
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(bg)
        .text_color(fg)
        .tooltip(native_tooltip(title.into()))
        .when(disabled, |d| d.opacity(0.35))
        .when(!disabled, |d| {
            d.on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .when(!on, |d| d.active(move |s| s.bg(pressed_bg)))
            .on_click(move |_, window, cx| on_click(window, cx))
        });
    content(el).into_any_element()
}

/// Strokes a path of segments in window coordinates, round caps and joins.
fn paint_line(segs: &[Seg], width: f32, color: Rgba, at: &dyn Fn(Pt) -> GPoint<Pixels>, window: &mut Window) {
    if segs.is_empty() || width <= 0.0 {
        return;
    }
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    let mut path = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
    for seg in segs {
        match *seg {
            Seg::Move(p) => path.move_to(at(p)),
            Seg::Line(p) => path.line_to(at(p)),
            Seg::Quad(c, p) => path.curve_to(at(p), at(c)),
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// A dashed rounded rectangle (`stroke-dasharray`, non-scaling), the dash
/// starting at the top edge after the corner as SVG's `rect` does.
#[allow(clippy::too_many_arguments)]
fn paint_dashed_rect(
    origin: GPoint<Pixels>,
    w: f32,
    h: f32,
    r: f32,
    width: f32,
    dash: (f32, f32),
    color: Rgba,
    window: &mut Window,
) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (x, y) = (f32::from(origin.x), f32::from(origin.y));
    let options = StrokeOptions::default().with_line_width(width);
    let mut path = PathBuilder::stroke(px(width))
        .with_style(PathStyle::Stroke(options))
        .dash_array(&[px(dash.0), px(dash.1)]);
    let p = |px_: f32, py: f32| point(px(px_), px(py));
    // Quarter circles as cubic curves.
    let k = 0.5523;
    path.move_to(p(x + r, y));
    path.line_to(p(x + w - r, y));
    path.cubic_bezier_to(p(x + w, y + r), p(x + w - r + k * r, y), p(x + w, y + r - k * r));
    path.line_to(p(x + w, y + h - r));
    path.cubic_bezier_to(p(x + w - r, y + h), p(x + w, y + h - r + k * r), p(x + w - r + k * r, y + h));
    path.line_to(p(x + r, y + h));
    path.cubic_bezier_to(p(x, y + h - r), p(x + r - k * r, y + h), p(x, y + h - r + k * r));
    path.line_to(p(x, y + r));
    path.cubic_bezier_to(p(x + r, y), p(x, y + r - k * r), p(x + r - k * r, y));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
