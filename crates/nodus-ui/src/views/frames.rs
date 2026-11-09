//! Views' frames (spec 04 VIEW-027…030, decision D3): a webview for every
//! view with a page while its board is open, on screen or not, so pages load
//! and page checks get their `loaded` (R13); one more for the view open
//! large. Each frame follows its card's body every frame, and shows a
//! snapshot GPUI draws in its place whenever GPUI must draw over it or move
//! it (`swap.rs`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyWindowHandle, App, Bounds, Context, Corners, EntityId, Global, Hitbox, HitboxBehavior, IntoElement, Pixels,
    RenderImage, Styled, Window, canvas, px,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::webview::{Incoming, PageWebview};
use crate::board::canvas::BoardCanvas;

/// Which frame of a view: its card's, or the large window's (a view can
/// have both at once, VIEW-029).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Card,
    Large,
}

/// One frame and how it's shown.
pub struct Frame {
    pub id: u64,
    pub project_id: String,
    pub view_id: String,
    pub slot: Slot,
    /// The version the frame registered with, which its `loaded` reports
    /// (VIEW-046).
    pub version: u32,
    pub title: String,
    pub web: PageWebview,
    // How it's shown.
    /// The page shows (and takes the pointer); else GPUI draws `snapshot`.
    pub live: bool,
    pub snapshot: Option<Arc<RenderImage>>,
    pub snapshot_at: Option<Instant>,
    /// The page may have changed since the snapshot.
    pub dirty: bool,
    pub pending: bool,
    /// Drawn this frame (its card's probe painted).
    pub drawn: bool,
    /// What the last draw asked for.
    pub geometry: Option<Geometry>,
    /// The probe's hitbox in the frame just drawn.
    pub probe: Option<Hitbox>,
    /// Something GPUI drew covers the page (last check).
    pub occluded: bool,
    /// The page should be behind its snapshot (last check).
    pub want: bool,
    /// A snapshot failed: the next try waits until then, longer after each
    /// failure in a row (`failed`).
    pub retry_at: Option<Instant>,
    pub failed: u32,
    /// The ink its overlay shows over the live page (`ink.rs`).
    pub ink: Option<super::ink::PageInk>,
    /// The pointer is over the page (the shell says, VIEW-128's hover).
    pub hovered: bool,
    /// One of the card's own tooltips is up over the page.
    pub tooltip: bool,
    pub zoom_changed: Option<Instant>,
    /// When the body last changed size: the page lays out again first.
    pub resized: Option<Instant>,
}

/// Where a frame was drawn, and what for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// The body, in window coordinates.
    pub document: Bounds<Pixels>,
    /// What of it may show.
    pub clip: Bounds<Pixels>,
    pub zoom: f32,
    pub radius: f32,
    /// Its own reasons to be a snapshot: dragged, resized, an ink tool out,
    /// the marquee over it, parity shots.
    pub forced: bool,
    /// The pointer goes to the board (VIEW-141).
    pub passthrough: bool,
    /// The resize handles' reach into the page.
    pub edge: f32,
}

/// Every frame, for the open board.
pub struct Frames {
    pub project_id: Option<String>,
    canvas: Option<EntityId>,
    pub window: Option<AnyWindowHandle>,
    pub frames: Vec<Frame>,
    next_id: u64,
    pub tx: UnboundedSender<Incoming>,
    rx: Option<UnboundedReceiver<Incoming>>,
    /// The view open large (VIEW-147).
    pub large: Option<String>,
    dark: Option<bool>,
    pub check_scheduled: bool,
    pub settle_scheduled: bool,
    /// The board's ink this frame, for the overlays over live pages.
    pub ink: Option<super::ink::BoardInk>,
}

#[derive(Clone)]
struct FramesGlobal(Rc<RefCell<Frames>>);

impl Global for FramesGlobal {}

/// The frames, made on first use.
pub fn frames(cx: &mut App) -> Rc<RefCell<Frames>> {
    if let Some(g) = cx.try_global::<FramesGlobal>() {
        return g.0.clone();
    }
    let (tx, rx) = unbounded_channel();
    let frames = Rc::new(RefCell::new(Frames {
        project_id: None,
        canvas: None,
        window: None,
        frames: Vec::new(),
        next_id: 1,
        tx,
        rx: Some(rx),
        large: None,
        dark: None,
        check_scheduled: false,
        settle_scheduled: false,
        ink: None,
    }));
    cx.set_global(FramesGlobal(frames.clone()));
    frames
}

/// The frames if there are any, without making them.
pub fn existing(cx: &App) -> Option<Rc<RefCell<Frames>>> {
    cx.try_global::<FramesGlobal>().map(|g| g.0.clone())
}

impl Frames {
    pub fn get(&self, view_id: &str, slot: Slot) -> Option<&Frame> {
        self.frames.iter().find(|f| f.view_id == view_id && f.slot == slot)
    }

    pub fn get_mut(&mut self, view_id: &str, slot: Slot) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| f.view_id == view_id && f.slot == slot)
    }

    pub fn by_id(&self, id: u64) -> Option<&Frame> {
        self.frames.iter().find(|f| f.id == id)
    }

    pub fn by_id_mut(&mut self, id: u64) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| f.id == id)
    }

    /// What the shells post, for the relay to read (once).
    pub fn take_rx(&mut self) -> Option<UnboundedReceiver<Incoming>> {
        self.rx.take()
    }

    /// Drops every frame (the board closed).
    pub fn clear(&mut self) {
        self.frames.clear();
        self.large = None;
        self.project_id = None;
        self.canvas = None;
        self.ink = None;
    }
}

/// What a card or the large window shows in its body this frame. (Its
/// snapshot, while it's behind one, is the probe's to paint: whether it is
/// is decided as the frame paints.)
#[derive(Clone, Default)]
pub struct Shown {
    /// There's a frame for it.
    pub framed: bool,
    /// The pointer is over the page.
    pub hovered: bool,
}

/// How a view's frame shows this frame.
pub fn shown(view_id: &str, slot: Slot, cx: &App) -> Shown {
    let Some(frames) = existing(cx) else {
        return Shown::default();
    };
    let frames = frames.borrow();
    frames.get(view_id, slot).map_or_else(Shown::default, |f| Shown {
        framed: true,
        hovered: f.hovered,
    })
}

/// Keeps a frame for every view with a page on the open board (VIEW-030:
/// `version > 0`), and the large window's while it's open; brings their
/// versions, titles and backgrounds up to date (VIEW-027, VIEW-029).
pub fn sync_board(canvas: &mut BoardCanvas, window: &mut Window, cx: &mut Context<'_, BoardCanvas>) {
    let rc = frames(cx);
    let entity = cx.entity_id();
    let first = {
        let mut f = rc.borrow_mut();
        let first = f.canvas != Some(entity);
        if first || f.project_id.as_deref() != Some(canvas.project_id.as_str()) {
            f.clear();
            f.project_id = Some(canvas.project_id.clone());
            f.canvas = Some(entity);
        }
        f.window = Some(window.window_handle());
        first
    };
    if first {
        // BOARD-004: the board's frames go with it.
        let drop_rc = rc.clone();
        cx.on_release(move |_, _| {
            let mut f = drop_rc.borrow_mut();
            if f.canvas == Some(entity) {
                f.clear();
            }
        })
        .detach();
        super::relay::start(&rc, cx);
        // The git status pages hear is the window's (the title bar's model),
        // not a read of their own.
        let git = crate::git::GitModel::get(&canvas.state, window, cx);
        let status = git.read(cx).status.clone();
        super::relay::board_mounted(status.as_ref(), cx);
        cx.observe(&git, |_, git, cx| {
            let status = git.read(cx).status.clone();
            super::relay::git_changed(status.as_ref(), cx);
        })
        .detach();
    }
    let Some(board) = canvas.board().cloned() else { return };
    let t = crate::theme::tokens(cx);
    let nodus = cx.global::<crate::shell::window::Core>().0.clone();
    let mut f = rc.borrow_mut();
    let theme_changed = f.dark != Some(t.dark);
    f.dark = Some(t.dark);
    // Frames for views that went, or lost their page, go.
    let large = f.large.clone();
    f.frames.retain(|fr| {
        board.views.iter().any(|v| {
            v.id == fr.view_id && v.version > 0 && (fr.slot == Slot::Card || large.as_deref() == Some(v.id.as_str()))
        })
    });
    if let Some(id) = &large
        && !board.views.iter().any(|v| &v.id == id)
    {
        f.large = None;
    }
    let mut wanted: Vec<(&nodus_domain::BoardView, Slot)> = board
        .views
        .iter()
        .filter(|v| v.version > 0)
        .map(|v| (v, Slot::Card))
        .collect();
    if let Some(v) = large.as_ref().and_then(|id| board.views.iter().find(|v| &v.id == id))
        && v.version > 0
    {
        wanted.push((v, Slot::Large));
    }
    for (view, slot) in wanted {
        match f.get_mut(&view.id, slot) {
            Some(fr) => {
                if fr.version != view.version || fr.title != view.title || theme_changed {
                    fr.web.update(view.version, &view.title, t.bg_elevated);
                    if theme_changed {
                        fr.web.set_dark(t.dark);
                    }
                    fr.version = view.version;
                    fr.title.clone_from(&view.title);
                    fr.dirty = true;
                }
            }
            None => {
                let id = f.next_id;
                f.next_id += 1;
                let made = PageWebview::new(
                    window,
                    id,
                    &view.id,
                    view.version,
                    &view.title,
                    t.bg_elevated,
                    nodus.clone(),
                    f.tx.clone(),
                );
                match made {
                    Ok(web) => f.frames.push(Frame {
                        web: {
                            web.set_dark(t.dark);
                            web
                        },
                        id,
                        project_id: canvas.project_id.clone(),
                        view_id: view.id.clone(),
                        slot,
                        version: view.version,
                        title: view.title.clone(),
                        live: true,
                        snapshot: None,
                        snapshot_at: None,
                        dirty: true,
                        pending: false,
                        drawn: false,
                        geometry: None,
                        probe: None,
                        occluded: false,
                        want: false,
                        retry_at: None,
                        failed: 0,
                        ink: None,
                        hovered: false,
                        tooltip: false,
                        zoom_changed: None,
                        resized: None,
                    }),
                    Err(err) => tracing::warn!("[views] couldn't make a webview: {err}"),
                }
            }
        }
    }
    // Drawn again below; a frame no card paints this time hides.
    for fr in &mut f.frames {
        fr.drawn = false;
    }
}

/// The frames as the test driver reports them: which views have a webview,
/// whether each is live or behind its snapshot, and why.
pub fn describe(cx: &App) -> serde_json::Value {
    let Some(rc) = existing(cx) else {
        return serde_json::Value::Array(Vec::new());
    };
    let f = rc.borrow();
    serde_json::Value::Array(
        f.frames
            .iter()
            .map(|fr| {
                let g = fr.geometry;
                serde_json::json!({
                    "frame": fr.id,
                    "view": fr.view_id,
                    "slot": if fr.slot == Slot::Card { "card" } else { "large" },
                    "version": fr.version,
                    "live": fr.live,
                    "snapshot": fr.snapshot.is_some(),
                    "occluded": fr.occluded,
                    "want": fr.want,
                    "pending": fr.pending,
                    "ink": fr.ink.as_ref().map(|i| i.marks.len()),
                    "offscreen": fr.web.offscreen(),
                    "drawn": fr.drawn,
                    "hovered": fr.hovered,
                    "keyboard": fr.web.has_keyboard(),
                    "forced": g.map(|g| g.forced),
                    "passthrough": g.map(|g| g.passthrough),
                    "zoom": g.map(|g| g.zoom),
                    "document": g.map(|g| [
                        f32::from(g.document.origin.x),
                        f32::from(g.document.origin.y),
                        f32::from(g.document.size.width),
                        f32::from(g.document.size.height),
                    ]),
                })
            })
            .collect(),
    )
}

/// The body's probe: an invisible box over a frame's body, last in its card,
/// so whatever GPUI hit-tests above it covers the page. Painting it places
/// the frame's webview there, and paints the page's snapshot in its place
/// while it's behind one (decided in that same paint, `swap::placed`).
pub fn probe(
    view_id: String,
    slot: Slot,
    geometry: impl Fn(Bounds<Pixels>, Bounds<Pixels>) -> Geometry + 'static,
) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let g = geometry(bounds, window.content_mask().bounds);
            if let Some(image) = super::swap::placed(&view_id, slot, g, hitbox, window, cx) {
                // `object-fit: fill`, the body's rounded bottom corners.
                let r = px(g.radius);
                let corners = Corners {
                    top_left: Pixels::ZERO,
                    top_right: Pixels::ZERO,
                    bottom_right: r,
                    bottom_left: r,
                };
                let _ = window.paint_image(bounds, bounds, corners, image, 0, false);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
