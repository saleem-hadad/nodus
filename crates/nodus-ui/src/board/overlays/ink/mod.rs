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
use nodus_board::drawing::BoardHistory;
use nodus_board::ink_draw::{
    self, HIT_PX, LABEL_FONT, LABEL_LINE, MIN_ARROW_PX, MIN_STEP_PX, MOVE_PX, PEN_WIDTH, Pt, Seg, distance, flatten,
    points_of, simplify, size_at, stroke_at, stroke_box, strokes_along, translate,
};
use nodus_domain::{InkColor, InkKind, InkStroke, LinkEnd, ShapeKind, js};

use super::icons::{OvIcon, ov_icon};
use crate::board::canvas::BoardCanvas;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::platform::keys;
use crate::theme::tokens::{Tokens, inset_shadow};
use crate::theme::{self, fonts, motion, tokens::DUR_FAST};

/// The dock's tools: the pointer, the two that ask Claude on the spot
/// (BUBBLE), the ink's pen and eraser, and the tools that put shapes and
/// links on the board (spec 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InkTool {
    #[default]
    Select,
    /// A click opens the bubble there (BUBBLE-001).
    Bubble,
    /// A click starts speaking there, the next one sends it (BUBBLE-040).
    Mic,
    Pen,
    /// A rectangle, ellipse or diamond, of the kind picked (SHAPE-002).
    Shape,
    /// A link between things on the board, or a loose arrow (LINK-010).
    Arrow,
    /// Words on their own, a shape (SHAPE-004).
    Text,
    /// The emoji picked, stamped where clicked (SHAPE-005).
    Sticker,
    Eraser,
}

/// The dock's tools: tool, name, its key.
const TOOLS: [(InkTool, &str, Option<&str>); 9] = [
    (InkTool::Select, "Pointer", Some("Esc")),
    (InkTool::Bubble, "Ask Claude here", Some("B")),
    (InkTool::Mic, "Speak to Claude here", Some("M")),
    (InkTool::Pen, "Pen", Some("P")),
    (InkTool::Shape, "Shape", Some("R")),
    (InkTool::Arrow, "Arrow", Some("A")),
    (InkTool::Text, "Text", None),
    (InkTool::Sticker, "Sticker", Some("S")),
    (InkTool::Eraser, "Eraser", Some("X")),
];

/// The shapes the Shape tool draws, in the picker's order.
pub const SHAPE_KINDS: [(ShapeKind, &str); 3] = [
    (ShapeKind::Rect, "Rectangle"),
    (ShapeKind::Ellipse, "Ellipse"),
    (ShapeKind::Diamond, "Diamond"),
];

/// The stickers the picker offers.
pub const STICKERS: [&str; 16] = [
    "👍", "❤️", "🔥", "✅", "❌", "⭐", "🎉", "💡", "⚠️", "🚀", "👀", "❓", "🐛", "📌", "⏳", "😀",
];

/// INKUI-002, INKUI-027, BUBBLE-001, BUBBLE-040, SHAPE-002: the board's
/// single-key shortcuts for the tools.
pub fn tool_for_key(key: &str) -> Option<InkTool> {
    match key {
        "b" => Some(InkTool::Bubble),
        "m" => Some(InkTool::Mic),
        "p" => Some(InkTool::Pen),
        "a" => Some(InkTool::Arrow),
        "r" => Some(InkTool::Shape),
        "s" => Some(InkTool::Sticker),
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
    /// A shape being drawn from corner to corner (SHAPE-002).
    Shape {
        kind: ShapeKind,
        start: Pt,
        end: Pt,
        square: bool,
        zoom: f64,
    },
    /// A link being drawn from what was pressed to the pointer, and what it
    /// would hold on to there (LINK-010).
    Link {
        from: LinkEnd,
        start: Pt,
        end: Pt,
        target: Option<String>,
        zoom: f64,
    },
    /// The selected link's end being taken somewhere else (LINK-014).
    Reattach {
        id: String,
        /// The end with the head.
        head: bool,
        end: Pt,
        target: Option<String>,
    },
    Draw {
        base: InkStroke,
        pts: Vec<Pt>,
        zoom: f64,
    },
    Erase {
        gone: Vec<String>,
        /// Shapes and links it passed over (SHAPE-021, LINK-015).
        shapes: Vec<String>,
        links: Vec<String>,
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
    /// The selected link (LINK-012).
    pub link: Option<String>,
    gesture: Option<InkGesture>,
    label: Option<LabelDraft>,
    /// A link's label being written (LINK-020).
    link_label: Option<LinkLabelDraft>,
    pub palette: bool,
    /// The Shape tool's picker or the Sticker tool's is out.
    pub picker: Option<InkTool>,
    /// What the Shape tool draws (a rectangle at first), and the Sticker
    /// tool stamps.
    pub shape_kind: Option<ShapeKind>,
    pub sticker: Option<&'static str>,
    pub(crate) history: BoardHistory,
    /// The layer's hitbox: a press is on the board when it's hovered.
    hitbox: Option<Hitbox>,
    /// INKUI-016: the pointer tool is over a stroke (`.ink-hover`).
    hover: bool,
    /// The pointer tool is over a link (LINK-012).
    hover_link: bool,
    /// The colour button and its palette, for the outside press.
    color_area: Vec<Bounds<Pixels>>,
    /// The picker, for the outside press.
    picker_area: Vec<Bounds<Pixels>>,
    /// The dock, whose buttons work their pickers themselves.
    dock_area: Option<Bounds<Pixels>>,
    label_key: u64,
}

/// A link's label being written, at the link's middle.
pub(crate) struct LinkLabelDraft {
    id: String,
    input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl Ink {
    pub fn color(&self, _: &App) -> InkColor {
        self.color.unwrap_or(InkColor::Red)
    }

    pub fn shape_kind(&self) -> ShapeKind {
        self.shape_kind.unwrap_or(ShapeKind::Rect)
    }

    pub fn sticker(&self) -> &'static str {
        self.sticker.unwrap_or(STICKERS[0])
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
mod links;
mod ops;
// What views draw over their live pages (spec 04, D3).
mod overlay;
pub(crate) use links::shape_path;
pub use overlay::{InkMark, InkScene};
