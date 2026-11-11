//! Ink on the board (`InkLayer.tsx`, `ink.css`; spec 11 INKUI): pen lines,
//! arrows, rectangles and text labels drawn over everything, and the dock of tools that
//! draws them, above the zoom controls. While a tool is out, the board's own
//! pointer handling stands aside; the pointer tool selects, moves and deletes
//! ink. The geometry and the undo history are `nodus_board::ink_draw`.
//!
//! Input is taken ahead of the board: the listeners are registered before
//! anything else on the canvas (the window's capture phase runs them in
//! paint order), and the layer's hitbox, inserted above the nodes and under
//! the HUD, tells whether a press is on the board's surface.

use std::rc::Rc;

use gpui::Focusable as _;
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, CursorStyle, DispatchPhase, Entity, FontWeight, Hitbox,
    HitboxBehavior, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, PathBuilder, PathStyle, Pixels, Point as GPoint, Rgba, SharedString, StatefulInteractiveElement,
    Styled, Subscription, TextRun, Window, WindowTextSystem, canvas, div, point, px,
};
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};
use nodus_board::ink_draw::{
    self, HIT_PX, InkHistory, LABEL_FONT, LABEL_LINE, MIN_ARROW_PX, MIN_STEP_PX, MOVE_PX, PEN_WIDTH, Pt, Seg, distance,
    flatten, points_of, simplify, size_at, stroke_at, stroke_box, strokes_along, translate,
};
use nodus_domain::{InkColor, InkKind, InkStroke, js};

use super::icons::{OvIcon, ov_icon};
use crate::board::canvas::BoardCanvas;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::platform::keys;
use crate::theme::tokens::{Tokens, inset_shadow};
use crate::theme::{self, fonts, motion, tokens::DUR_FAST};

/// The ink tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InkTool {
    #[default]
    Select,
    Pen,
    Arrow,
    Rect,
    Text,
    Eraser,
}

/// The dock's tools: tool, name, its key.
const TOOLS: [(InkTool, &str, Option<&str>); 6] = [
    (InkTool::Select, "Pointer", Some("Esc")),
    (InkTool::Pen, "Pen", Some("P")),
    (InkTool::Arrow, "Arrow", Some("A")),
    (InkTool::Rect, "Rectangle", Some("R")),
    (InkTool::Text, "Text", None),
    (InkTool::Eraser, "Eraser", Some("X")),
];

/// INKUI-002, INKUI-027: the board's single-key shortcuts for the tools.
pub fn tool_for_key(key: &str) -> Option<InkTool> {
    match key {
        "p" => Some(InkTool::Pen),
        "a" => Some(InkTool::Arrow),
        "r" => Some(InkTool::Rect),
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

/// INKUI-004: where the colour picked last is kept, app-wide and across
/// restarts (`localStorage`, brought over from 0.9.4's).
pub const COLOR_KEY: &str = "nodus.inkColor";

/// The colour picked last; red when none was, or what's stored isn't one.
pub(crate) fn stored_color(nodus: &nodus_core::Nodus) -> InkColor {
    nodus
        .local_storage()
        .get(COLOR_KEY)
        .and_then(|c| InkColor::parse(&c))
        .unwrap_or(InkColor::Red)
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
    Draw {
        base: InkStroke,
        pts: Vec<Pt>,
        zoom: f64,
    },
    Erase {
        gone: Vec<String>,
        last: Pt,
        tolerance: f64,
    },
    Move {
        id: String,
        start: Pt,
        dx: f64,
        dy: f64,
        moved: bool,
        zoom: f64,
    },
}

/// The ink layer's state, per board mount.
#[derive(Default)]
pub(crate) struct Ink {
    pub tool: InkTool,
    /// The current colour, read from storage the first time the dock is drawn.
    color: Option<InkColor>,
    pub selected: Option<String>,
    gesture: Option<InkGesture>,
    label: Option<LabelDraft>,
    pub palette: bool,
    history: InkHistory,
    /// The layer's hitbox: a press is on the board when it's hovered.
    hitbox: Option<Hitbox>,
    /// INKUI-016: the pointer tool is over a stroke (`.ink-hover`).
    hover: bool,
    /// The colour button and its palette, for the outside press.
    color_area: Vec<Bounds<Pixels>>,
    label_key: u64,
}

impl Ink {
    pub fn color(&self, _: &App) -> InkColor {
        self.color.unwrap_or(InkColor::Red)
    }

    /// Lets go of a gesture under way (the window lost the pointer).
    pub fn drop_gesture(&mut self) {
        self.gesture = None;
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

fn measurer(window: &Window) -> impl Fn(&str, f64) -> f64 + use<> {
    let ts: Rc<WindowTextSystem> = window.text_system().clone();
    move |text: &str, size: f64| label_width(&ts, text, size)
}

// The bubble's cursor is drawn the eraser's way.
pub(crate) mod cursor;
mod dock;
mod draw;
mod input;
mod ops;
// What views draw over their live pages (spec 04, D3).
mod overlay;
pub use overlay::{InkMark, InkScene};
