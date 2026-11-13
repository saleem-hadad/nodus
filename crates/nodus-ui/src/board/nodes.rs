//! The node plug-in interface: how the canvas (spec 09) hands each node's
//! contents to whoever draws them — session cards, expanded cards, the
//! archive frame, notes and the note pad (spec 10, `board/cards/`), and views
//! (spec 04). The canvas owns everything React Flow did: where a node is, its
//! size, z-order, selection, dragging, clicks, double-clicks, context menus
//! and the wheel. A renderer draws what's inside the node's box.
//!
//! # Drawing
//!
//! A renderer returns the node's contents for its box. The canvas places the
//! box on screen and draws the contents inside [`super::units::Zoomed`], so
//! lengths written with [`super::units::fu`] are flow units that scale with
//! the board, as everything inside a React Flow node does. Contents may
//! overflow the box (shadows, a sticker, the agent deck, resize handles);
//! nothing clips them. Keep per-node state with
//! `window.use_keyed_state((node.id.clone(), "…"), cx, …)`.
//!
//! # Input
//!
//! The canvas handles a press, click, double-click, right-click and wheel on
//! the node's box after the contents had their turn (GPUI bubbles from the
//! innermost element out):
//!
//! - a control that must not start a drag, a click, a selection or a menu
//!   stops the mouse-down's propagation (`cx.stop_propagation()`): 0.9.4's
//!   `nodrag` class, and `onPointerDown={e => e.stopPropagation()}`;
//! - an element that scrolls itself stops the scroll event's propagation:
//!   `nowheel`;
//! - a card's title text calls [`BoardActions::press_title`] in its own
//!   mouse-down (without stopping it), so the canvas knows the click landed
//!   on `.card-title` (BOARD-043, BOARD-047).
//!
//! Selection, dragging and opening follow BOARD-038…BOARD-066; renderers only
//! read [`NodeState`].
//!
//! # Installing
//!
//! `board/cards/` implements [`NodeRenderers`] and installs it once at
//! start-up with [`install`] (from [`super::init`]). Until then, and for any
//! method it doesn't override, the canvas draws a plain placeholder box.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Global, IntoElement, ParentElement, Point, SharedString, Size, Styled, WeakEntity, Window, div,
};
use nodus_domain::{BoardNote, BoardShape, BoardView, NodePatch, Rect, SessionCard, ViewPatch};

use super::canvas::BoardCanvas;
use super::units::fu;
use crate::theme;

/// What the canvas tells every node it draws.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeState {
    pub id: SharedString,
    /// The node's box, in flow units (before the zoom).
    pub size: Size<f32>,
    /// React Flow's `selected`.
    pub selected: bool,
    /// React Flow's `dragging`: the node is being dragged right now.
    pub dragging: bool,
    /// The board's zoom.
    pub zoom: f32,
}

/// Where an archived card sits in the archive stack (BOARD-015).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StackPlace {
    /// In the folded pile, `depth` cards from the top: `stack-d<min(depth, 3)>`.
    Pile { depth: usize },
    /// Fanned out in the open panel at `slot`: `is-result`, `--stack-i =
    /// min(slot, 4)`.
    Result { slot: usize },
}

/// How an archived card is drawn: the classes and style variables 0.9.4 set
/// on its node (BOARD-015). The canvas places the card and clips a result
/// crossing the panel's edges; the card draws its depth (`stack-d*`), its
/// deal-out and its rise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StackStyle {
    pub place: StackPlace,
    /// Flying out of the stack (`stack-enter`), from this offset in flow
    /// units (`--fly-x`, `--fly-y`).
    pub rise: Option<Point<f32>>,
    /// The open panel's results scroll with the wheel (`nowheel`).
    pub nowheel: bool,
}

/// A session card, compact or expanded, active or archived.
pub struct CardNode<'a> {
    pub node: NodeState,
    pub card: &'a SessionCard,
    /// The title's characters the archive search matched.
    pub matched: Option<&'a [usize]>,
    /// The notes stuck on it (none while archived, BOARD-018).
    pub stickers: &'a [BoardNote],
    /// Set for archived cards.
    pub stack: Option<StackStyle>,
}

/// A free note, or the pad's top sheet (`draft`).
pub struct NoteNode<'a> {
    pub node: NodeState,
    pub note: &'a BoardNote,
    /// The pad's top sheet, waiting to be peeled off (BOARD-022).
    pub draft: bool,
}

/// A shape (spec 14), and its picture's file when it's a picture.
pub struct ShapeNode<'a> {
    pub node: NodeState,
    pub shape: &'a BoardShape,
    pub image: Option<std::path::PathBuf>,
    /// Claude is drawing it, 0…1 through (MOTION-021).
    pub drawing: Option<f32>,
}

/// The archive frame: the folded stack, or the open panel.
pub struct ArchiveNode {
    pub node: NodeState,
    /// How many cards are archived.
    pub count: usize,
}

/// The note pad under the archive.
pub struct PadNode {
    pub node: NodeState,
}

/// A view.
pub struct ViewNode<'a> {
    pub node: NodeState,
    pub view: &'a BoardView,
}

/// The archive stack's state, for the archive frame (BOARD-079).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArchiveStack {
    /// The search panel is open with results fanned out of the stack.
    pub open: bool,
    /// The pointer is over the stack (and it wasn't dismissed), which shows
    /// the search field.
    pub hover: bool,
    pub query: SharedString,
    pub result_count: usize,
    /// How far the results are scrolled, and how far they can scroll (flow
    /// units).
    pub scroll: f32,
    pub scroll_max: f32,
}

/// What the whole board tells its nodes: 0.9.4's `RenamingContext`,
/// `ArchiveHotContext`, `SectionHotContext`, `SectionNamingContext`,
/// `ArchiveStackContext`, the canvas's classes, and `BoardActions`.
#[derive(Clone, PartialEq)]
pub struct BoardCx {
    pub project_id: SharedString,
    pub zoom: f32,
    /// The card, section, note or view being renamed (or written in), if any.
    pub renaming: Option<SharedString>,
    /// A dragged card is over the archive.
    pub archive_hot: bool,
    /// The section a dragged card would join.
    pub section_hot: Option<SharedString>,
    /// Sections Claude is naming: None while it thinks, then the name it
    /// gave ('' for none) while that lands.
    pub naming: Rc<HashMap<String, Option<String>>>,
    pub stack: ArchiveStack,
    /// `is-moving-archive`, `is-moving-section`, `is-scrolling-stack`,
    /// `stack-lift` on the canvas.
    pub moving_archive: bool,
    pub moving_section: bool,
    pub scrolling_stack: bool,
    pub stack_lift: bool,
    /// The card a search jump flashes, and when the flash started: two
    /// 800ms rounds of `0 0 0 2px accent, 0 0 0 2px→18px accent 55%→0%`,
    /// eased `cubic-bezier(0.2, 0.8, 0.2, 1)` (BOARD-036).
    pub flash: Option<(SharedString, std::time::Instant)>,
    /// An ink tool other than the pointer is out (spec 11 INKUI-018): a
    /// view's live page ignores the pointer then, and shows its snapshot.
    pub ink_tool: bool,
    pub actions: BoardActions,
    /// What the pointer and the viewport are doing, for views' pages, which
    /// let the pointer through and show a snapshot meanwhile (VIEW-141, D3).
    pub pointer: BoardPointer,
}

/// What the pointer and the viewport are doing on the board.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoardPointer {
    /// Nodes (or a section with its cards) are being dragged.
    pub dragging: bool,
    /// The marquee being drawn, in canvas pixels: x, y, w, h.
    pub marquee: Option<(f64, f64, f64, f64)>,
    /// The post-marquee selection rectangle is up.
    pub selection_rect: bool,
    /// The viewport is moving: a wheel, a pinch, a pan or a flight.
    pub moving: bool,
}

/// What nodes can ask of the board (`BoardContext.ts`'s `BoardActions`, and
/// the store actions 0.9.4's nodes called directly).
#[derive(Clone)]
pub struct BoardActions {
    pub(crate) canvas: WeakEntity<BoardCanvas>,
    pub(crate) title_pressed: Rc<Cell<bool>>,
}

/// The same board's actions (a node drawn with either draws the same).
impl PartialEq for BoardActions {
    fn eq(&self, other: &Self) -> bool {
        self.canvas.entity_id() == other.canvas.entity_id() && Rc::ptr_eq(&self.title_pressed, &other.title_pressed)
    }
}

impl BoardActions {
    fn with(&self, cx: &mut App, f: impl FnOnce(&mut BoardCanvas, &mut gpui::Context<'_, BoardCanvas>)) {
        if let Some(canvas) = self.canvas.upgrade() {
            canvas.update(cx, f);
        }
    }

    /// The press under way landed on a card's title text (`.card-title`).
    pub fn press_title(&self) {
        self.title_pressed.set(true);
    }

    /// Commits a new title for a card (empty clears the override, BOARD-149).
    pub fn rename(&self, session_id: &str, title: &str, cx: &mut App) {
        let (id, title) = (session_id.to_owned(), title.to_owned());
        self.with(cx, move |c, cx| c.rename_card(&id, &title, cx));
    }

    /// Starts or stops inline renaming of a card, section, note or view (BOARD-148).
    pub fn set_renaming(&self, id: Option<&str>, cx: &mut App) {
        let id = id.map(str::to_owned);
        self.with(cx, move |c, cx| c.set_renaming(id, cx));
    }

    /// Commits a new name for a section (empty leaves it untitled).
    pub fn rename_section(&self, section_id: &str, name: &str, cx: &mut App) {
        let (id, name) = (section_id.to_owned(), name.to_owned());
        self.with(cx, move |c, cx| c.rename_section(&id, &name, cx));
    }

    /// Expands active cards in place, or folds them back (BOARD-150).
    pub fn expand(&self, ids: &[String], expanded: bool, cx: &mut App) {
        let ids = ids.to_vec();
        self.with(cx, move |c, cx| c.expand_cards(&ids, expanded, cx));
    }

    /// Archives active sessions, asking first when one is running (BOARD-110).
    pub fn archive(&self, ids: &[String], cx: &mut App) {
        let ids = ids.to_vec();
        self.with(cx, move |c, cx| c.archive_cards(&ids, cx));
    }

    /// Stops sessions, asking first when one is mid-turn (BOARD-112).
    pub fn stop(&self, ids: &[String], cx: &mut App) {
        let ids = ids.to_vec();
        self.with(cx, move |c, cx| c.stop_cards(&ids, cx));
    }

    /// Saves what's written on a note (BOARD-124).
    pub fn edit_note(&self, note: &BoardNote, text: &str, cx: &mut App) {
        let (note, text) = (note.clone(), text.to_owned());
        self.with(cx, move |c, cx| c.edit_note(note, text, cx));
    }

    /// Puts words in a shape (SHAPE-012).
    pub fn edit_shape(&self, shape_id: &str, text: &str, cx: &mut App) {
        let (id, text) = (shape_id.to_owned(), text.to_owned());
        self.with(cx, move |c, cx| c.set_shape_text(&id, &text, cx));
    }

    /// Gives the keyboard back to the board (a shape's words put down).
    pub fn focus_board(&self, window: &mut Window, cx: &mut App) {
        if let Some(canvas) = self.canvas.upgrade() {
            let focus = canvas.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
    }

    /// A shape resized (SHAPE-011).
    pub fn resize_shape(&self, shape_id: &str, rect: Rect, cx: &mut App) {
        let id = shape_id.to_owned();
        self.with(cx, move |c, cx| c.resize_shape(&id, rect, cx));
    }

    /// Starts a Claude session on a note's text (BOARD-122, BOARD-123).
    pub fn start_note(&self, note_id: &str, cx: &mut App) {
        let id = note_id.to_owned();
        self.with(cx, move |c, cx| c.start_note(id, cx));
    }

    /// Opens a card's session, as clicking it does (BOARD-044).
    pub fn open_session(&self, session_id: &str, cx: &mut App) {
        let id = session_id.to_owned();
        self.with(cx, move |c, cx| c.open_card(&id, cx));
    }

    /// Opens a view's brain (BOARD-126).
    pub fn open_view_session(&self, view_id: &str, cx: &mut App) {
        let id = view_id.to_owned();
        self.with(cx, move |c, cx| c.open_view_session(id, cx));
    }

    /// Commits a new title for a view (BOARD-151, as are `open_session`,
    /// `open_view_session`, `start_note`, `archive`, `stop` and `edit_note`).
    pub fn rename_view(&self, view_id: &str, title: &str, cx: &mut App) {
        let (id, title) = (view_id.to_owned(), title.to_owned());
        self.with(cx, move |c, cx| c.rename_view(&id, &title, cx));
    }

    /// The archive search's query (BOARD-072).
    pub fn set_stack_query(&self, query: &str, cx: &mut App) {
        let query = query.to_owned();
        self.with(cx, move |c, cx| c.set_stack_query(query, cx));
    }

    /// Opens the archive stack (BOARD-071).
    pub fn open_stack(&self, cx: &mut App) {
        self.with(cx, BoardCanvas::open_stack);
    }

    /// Folds the archive stack (BOARD-071).
    pub fn close_stack(&self, cx: &mut App) {
        self.with(cx, BoardCanvas::close_stack);
    }

    /// Opens the first result (Enter in the archive search, BOARD-073).
    pub fn submit_stack(&self, window: &mut Window, cx: &mut App) {
        let canvas = self.canvas.clone();
        // Leave the field first so Escape closes the preview, not the panel.
        window.blur(cx);
        if let Some(canvas) = canvas.upgrade() {
            canvas.update(cx, BoardCanvas::submit_stack);
        }
    }

    /// Saves card changes (`patchCards`): a resized expanded card's `w`/`h`.
    pub fn patch_cards(&self, patches: Vec<NodePatch>, cx: &mut App) {
        self.with(cx, move |c, cx| c.patch_cards(patches, cx));
    }

    /// Saves a note (`saveNote`): a resized or recoloured note.
    pub fn save_note(&self, note: BoardNote, cx: &mut App) {
        self.with(cx, move |c, cx| c.save_note(note, cx));
    }

    /// Deletes notes at once (`removeNotes`, BOARD-121).
    pub fn remove_notes(&self, ids: Vec<String>, cx: &mut App) {
        self.with(cx, move |c, cx| c.remove_notes(ids, cx));
    }

    /// Saves a view change (`patchView`).
    pub fn patch_view(&self, patch: ViewPatch, cx: &mut App) {
        self.with(cx, move |c, cx| c.patch_view(patch, cx));
    }

    /// While a node is resized, its live box (React Flow's `resizing`): the
    /// canvas keeps this rect for it, whatever the store says, until it's
    /// given `None` (BOARD-005, BOARD-009).
    pub fn hold_rect(&self, id: &str, rect: Option<Rect>, cx: &mut App) {
        let id = id.to_owned();
        self.with(cx, move |c, cx| c.hold_rect(id, rect, cx));
    }

    /// Raises a card above everything but the open archive while its agent
    /// fan is out (z 1400, spec 10).
    pub fn raise(&self, id: &str, raised: bool, cx: &mut App) {
        let id = id.to_owned();
        self.with(cx, move |c, cx| c.raise(id, raised, cx));
    }

    /// A click on a control inside a node still selects the node, as React
    /// Flow's click on the node's wrapper did (a view's tools, spec 04).
    pub fn click_select(&self, id: &str, modifiers: &gpui::Modifiers, cx: &mut App) {
        let id = id.to_owned();
        let multi = (modifiers.shift ^ modifiers.platform) && !modifiers.control && !modifiers.alt;
        self.with(cx, move |c, cx| c.click_select(&id, multi, cx));
    }

    /// Shows the board's toast (BOARD-139).
    pub fn toast(&self, message: &str, cx: &mut App) {
        let message = message.to_owned();
        self.with(cx, move |c, cx| c.show_toast(message, cx));
    }
}

/// Draws node contents. Every method has a placeholder, so a renderer can
/// take over one kind of node at a time.
pub trait NodeRenderers: 'static {
    /// `SessionCardNode` / `ExpandedCard` (spec 10).
    fn card(&self, node: CardNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let _ = (board, window);
        placeholder(&node.node, nodus_board_title(node.card), cx)
    }

    /// `NoteNode` (spec 10): a free note or the pad's top sheet.
    fn note(&self, node: NoteNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let _ = (board, window);
        placeholder(&node.node, node.note.text.clone(), cx)
    }

    /// `ArchiveZoneNode` (spec 10).
    fn archive(&self, node: ArchiveNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let _ = (board, window);
        placeholder(&node.node, format!("Archive {}", node.count), cx)
    }

    /// `NotePadNode` (spec 10).
    fn pad(&self, node: PadNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let _ = (board, window);
        placeholder(&node.node, "Notes".to_owned(), cx)
    }

    /// A shape (spec 14).
    fn shape(&self, node: ShapeNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let _ = (board, window);
        placeholder(&node.node, node.shape.text.clone(), cx)
    }

    /// `ViewNode` (spec 04): the views' own renderer ([`ViewRenderer`])
    /// when it's installed.
    fn view(&self, node: ViewNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        if let Some(render) = cx.try_global::<ViewRenderer>().map(|r| r.0.clone()) {
            return render(node, board, window, cx);
        }
        let _ = (board, window);
        placeholder(&node.node, node.view.title.clone(), cx)
    }

    /// Shows a session in its expanded card, starting or resuming it, and
    /// focuses its terminal (`wakeCardTerminal`, XCARD-025): opening an
    /// expanded card (BOARD-044) and expanding cards (BOARD-150, BOARD-168)
    /// call it.
    fn wake(&self, session_id: &str, cx: &mut App) {
        let _ = (session_id, cx);
    }

    /// Focuses an expanded card's terminal, as clicking a window does (the
    /// terminal registry's `focus`, BOARD-042).
    fn focus_terminal(&self, session_id: &str, window: &mut Window, cx: &mut App) {
        let _ = (session_id, window, cx);
    }
}

/// The renderers in use.
#[derive(Clone)]
pub struct Renderers(pub Rc<dyn NodeRenderers>);

impl Global for Renderers {}

/// Draws every node as a placeholder.
pub struct Placeholders;

impl NodeRenderers for Placeholders {}

/// Installs the renderers the canvas draws nodes with.
pub fn install(renderers: impl NodeRenderers, cx: &mut App) {
    cx.set_global(Renderers(Rc::new(renderers)));
}

/// Draws view nodes, whichever [`NodeRenderers`] is installed: `crate::views`
/// (spec 04) sets it at start-up, so the cards' renderers needn't know views.
pub type RenderView = dyn Fn(ViewNode<'_>, &BoardCx, &mut Window, &mut App) -> AnyElement;

/// The view renderer in use.
#[derive(Clone)]
pub struct ViewRenderer(pub Rc<RenderView>);

impl Global for ViewRenderer {}

/// The renderers in use: the installed ones, else placeholders.
pub fn renderers(cx: &App) -> Rc<dyn NodeRenderers> {
    cx.try_global::<Renderers>()
        .map_or_else(|| Rc::new(Placeholders) as Rc<dyn NodeRenderers>, |r| r.0.clone())
}

fn nodus_board_title(card: &SessionCard) -> String {
    super::layout::card_title(card)
}

/// A plain box with a label, until the real node lands.
fn placeholder(node: &NodeState, label: String, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    div()
        .size_full()
        .p(fu(12.))
        .rounded(fu(14.))
        .bg(t.bg_elevated)
        .border(fu(1.))
        .border_color(if node.selected { t.accent } else { t.border })
        .text_size(fu(13.))
        .line_height(fu(18.2))
        .text_color(t.text_secondary)
        .overflow_hidden()
        .when(!label.is_empty(), |d| d.child(label))
        .into_any_element()
}
