//! The loupe (spec 11, LOUPE): a round lens at the pointer that shows the
//! board under it up close, so a zoomed-out board can be read without
//! zooming in. The dock's Loupe tool (L) puts it out, and L held shows it
//! only while the key is down. A press dives in: the board zooms to the
//! lens's zoom about the pointer.
//!
//! The lens draws the board a second time at its own zoom, from the same
//! data and in the nodes' looks, without anything a node does: no cached
//! node views, terminals, chats or pages, and no hitboxes of the lens's
//! own (the ink's hitbox above it takes the presses). Notes, shapes,
//! tiles, goals, the pad and frames are drawn by their own renderers; a
//! card shows its title and its preview (an expanded one too, for its
//! terminal or chat can only be drawn once), a view its page's picture.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Corners, Entity, EntityId, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Pixels, Point as GPoint, RenderImage, Rgba, Styled, Window, canvas, div, px, size,
};
use nodus_domain::{BoardSnapshot, BoardView, SessionCard, SessionStatus, Viewport};

use super::canvas::BoardCanvas;
use super::cards::Z;
use super::cards::para::{font, para};
use super::cards::present::{self, PreviewKind, is_expanded, is_terminal, preview};
use super::nodes::{BoardCx, GoalNode, NodeRenderers, NodeState, NoteNode, PadNode, ShapeNode, TileNode, renderers};
use super::overlays::INK_Z_BELOW;
use super::overlays::ink::InkTool;
use super::render::{overlaps, paint_dots, paint_edge};
use super::scene::{FlowNode, NodeKind, paint_order};
use super::section::SectionProps;
use super::units::zoomed;
use crate::theme::{self, fonts, motion, tokens::Tokens};

/// The glass across, in screen pixels, inside its bezel (LOUPE-003).
const LENS: f32 = 250.0;
/// The bezel round the glass.
const BEZEL: f32 = 5.0;
/// LOUPE-004: the lens shows the board at three times its zoom, kept
/// between 75% and 125%.
const FACTOR: f64 = 3.0;
const MIN_ZOOM: f64 = 0.75;
const MAX_ZOOM: f64 = 1.25;
/// LOUPE-002: L down longer than this is held, not tapped.
const HOLD: Duration = Duration::from_millis(250);
/// LOUPE-007: how long diving in takes.
const DIVE_MS: u64 = 400;
/// LOUPE-003: how long the lens takes to come up.
const APPEAR: Duration = Duration::from_millis(140);
/// A view's caption and its page's corners (spec 04).
const VIEW_RADIUS: f32 = 14.0;
/// LOUPE-006: a page's picture in the lens is taken again this often while
/// the lens is over it, for pages whose data moves.
const PICTURE_FRESH: Duration = Duration::from_secs(2);

/// The zoom the lens shows the board at, for the board's `zoom`.
pub fn lens_zoom(zoom: f64) -> f64 {
    (zoom * FACTOR).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Whether the lens has anything to show at the board's `zoom`: it reads
/// closer than the board does.
fn magnifies(zoom: f64) -> bool {
    lens_zoom(zoom) > zoom * 1.02
}

/// Notified as the pointer moves, so only the lens draws again
/// (`node_view::partial_frame`).
pub(crate) struct LoupeTick;

/// A page's picture for the lens (LOUPE-006), drawn at the lens's zoom.
struct LensPicture {
    /// The board's zoom and the lens's when it was taken.
    zooms: (f64, f64),
    at: Instant,
    image: Arc<RenderImage>,
}

/// The loupe's state on the canvas.
pub(crate) struct Loupe {
    tick: Entity<LoupeTick>,
    /// L is down: since when, and the tool out before it.
    key: Option<(Instant, InkTool)>,
    /// The lens has been up since.
    shown_at: Option<Instant>,
    /// Pages' pictures at the lens's zoom, by view, and those asked for.
    pictures: HashMap<String, LensPicture>,
    asked: HashSet<String>,
}

impl Loupe {
    pub fn new(cx: &mut Context<'_, BoardCanvas>) -> Self {
        Self {
            tick: cx.new(|_| LoupeTick),
            key: None,
            shown_at: None,
            pictures: HashMap::new(),
            asked: HashSet::new(),
        }
    }

    pub fn tick_id(&self) -> EntityId {
        self.tick.entity_id()
    }
}

impl BoardCanvas {
    /// LOUPE-002: L goes down: the loupe comes out at once.
    pub(crate) fn loupe_key_down(&mut self, cx: &mut Context<'_, Self>) {
        if self.loupe.key.is_some() {
            return;
        }
        let was = self.overlays.ink.tool;
        self.loupe.key = Some((Instant::now(), was));
        if was != InkTool::Loupe {
            self.ink_set_tool(InkTool::Loupe, None, cx);
        }
    }

    /// LOUPE-002: L comes up. A tap leaves the loupe out, or puts it down
    /// if it was out already; held, it goes back to the tool before.
    pub(crate) fn loupe_key_up(&mut self, cx: &mut Context<'_, Self>) {
        let Some((at, was)) = self.loupe.key.take() else {
            return;
        };
        let to = match (at.elapsed() < HOLD, was) {
            (true, InkTool::Loupe) => InkTool::Select,
            (true, _) => InkTool::Loupe,
            (false, was) => was,
        };
        // A tool picked meanwhile stays.
        if self.overlays.ink.tool == InkTool::Loupe && to != InkTool::Loupe {
            self.ink_set_tool(to, None, cx);
        }
    }

    /// LOUPE-002: the window let go of the keys with L held.
    pub(crate) fn loupe_release(&mut self) {
        if let Some((_, was)) = self.loupe.key.take()
            && self.overlays.ink.tool == InkTool::Loupe
        {
            self.overlays.ink.tool = was;
        }
    }

    /// LOUPE-003: the pointer moved; the lens follows it.
    pub(crate) fn loupe_moved(&mut self, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.tool == InkTool::Loupe {
            self.loupe.tick.update(cx, |_, cx| cx.notify());
        }
    }

    /// LOUPE-007: a press dives in: the board zooms to the lens's zoom about
    /// the pointer, and the loupe is put down.
    pub(crate) fn loupe_press(&mut self, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        let zoom = self.viewport.zoom;
        if !magnifies(zoom) {
            return;
        }
        let (x, y) = self.local(at);
        self.loupe.shown_at = None;
        self.fly_about(x, y, lens_zoom(zoom), DIVE_MS, cx);
        self.ink_set_tool(InkTool::Select, None, cx);
    }

    /// LOUPE-003…LOUPE-006: the lens, while the loupe is out and the pointer
    /// is on the board.
    pub(crate) fn render_loupe(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        // Its notifications draw the window again.
        let _ = self.loupe.tick.read(cx);
        let zoom = self.viewport.zoom;
        let pointer = self.pointer.filter(|p| {
            self.overlays.ink.tool == InkTool::Loupe
                && self.bounds.contains(p)
                && self.overlays.ink.on_board(window)
                && self.anim.is_none()
                && !self.space
                && magnifies(zoom)
        });
        let Some(pointer) = pointer else {
            self.loupe.shown_at = None;
            if self.overlays.ink.tool != InkTool::Loupe {
                self.loupe_drop_pictures(cx);
            }
            return None;
        };
        let t = theme::tokens(cx);
        let now = Instant::now();
        let since = *self.loupe.shown_at.get_or_insert(now);
        let appear = motion::duration(cx, APPEAR).as_secs_f32();
        let k = if appear > 0.0 {
            (now.duration_since(since).as_secs_f32() / appear).min(1.0)
        } else {
            1.0
        };
        if k < 1.0 {
            window.request_animation_frame();
        }

        let m = lens_zoom(zoom);
        let d = LENS + 2.0 * BEZEL;
        let c = f64::from(d / 2.0);
        let (x, y) = self.local(pointer);
        let f = self.to_flow(pointer);
        // The lens's own viewport: the pointer's board point at its centre.
        let vp = Viewport {
            x: c - f.x * m,
            y: c - f.y * m,
            zoom: m,
        };
        let seen = nodus_domain::Rect {
            x: f.x - c / m,
            y: f.y - c / m,
            w: 2.0 * c / m,
            h: 2.0 * c / m,
        };

        let mut below = Vec::new();
        let mut above = Vec::new();
        if let Some(snapshot) = self.board.take() {
            let mut board = self.board_cx(cx);
            board.zoom = m as f32;
            board.renaming = None;
            board.flash = None;
            let renderers = renderers(cx);
            let order = paint_order(&self.nodes);
            let nodes: Vec<FlowNode> = order
                .iter()
                .map(|&i| &self.nodes[i])
                .filter(|n| overlaps(n.rect(), seen) && n.z_drawn() < INK_Z_BELOW)
                .cloned()
                .collect();
            for node in &nodes {
                let Some(el) = self.lens_node(node, vp, &snapshot, &board, &renderers, &t, window, cx) else {
                    continue;
                };
                if node.z_drawn() < 0 {
                    below.push(el);
                } else {
                    above.push(el);
                }
            }
            self.board = Some(snapshot);
        }
        // Edges and ink are drawn from the canvas's viewport: the lens's, for now.
        let saved = self.viewport;
        self.viewport = vp;
        let edge_paths = self.edge_paths(&t);
        let ink = self.render_ink_layer(window, cx);
        self.viewport = saved;
        let label = self.lens_label(f, &t);

        let dot = t.canvas_dot;
        let content = div()
            .absolute()
            .left_0()
            .top_0()
            .size(px(d))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint_dots(bounds, vp, dot, window),
                )
                .absolute()
                .size_full(),
            )
            .children(below)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        for edge in &edge_paths {
                            paint_edge(edge, bounds.origin, 0.0, window);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(above)
            .child(ink);

        let e = motion::ease_out(k);
        let pill = |el: gpui::Div| {
            el.h(px(20.))
                .px(px(8.))
                .rounded_full()
                .bg(t.bg_elevated)
                .shadow(vec![theme::tokens::ring(1.0, t.border)])
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text_secondary)
                .whitespace_nowrap()
        };
        let label = label.map(|(text, color)| {
            div()
                .absolute()
                .top(px(BEZEL + 14.))
                .left_0()
                .w(px(d))
                .flex()
                .justify_center()
                .child(
                    pill(div())
                        .max_w(px(d * 0.62))
                        .overflow_hidden()
                        .when_some(color, |el, color| {
                            el.child(div().flex_none().size(px(7.)).rounded_full().bg(color))
                        })
                        .child(div().overflow_hidden().text_ellipsis().child(text)),
                )
        });
        let badge = div()
            .absolute()
            .bottom(px(BEZEL + 12.))
            .left_0()
            .w(px(d))
            .flex()
            .justify_center()
            .child(pill(div()).child(format!("{}%", (m * 100.0).round())));

        Some(
            div()
                .id("board-loupe")
                .absolute()
                .left(px(x as f32 - d / 2.0))
                .top(px(y as f32 - d / 2.0))
                .size(px(d))
                .opacity(e)
                // The shadow under the glass; the glass clips to its round edge.
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .size(px(d))
                        .rounded_full()
                        .bg(t.bg_elevated)
                        .shadow(t.lg()),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .size(px(d))
                        .rounded_full()
                        .overflow_hidden()
                        .clip_rounded()
                        .bg(t.bg_canvas)
                        .child(content),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .size(px(d))
                        .rounded_full()
                        .border(px(BEZEL))
                        .border_color(t.bg_elevated),
                )
                .children(label)
                .child(badge)
                .into_any_element(),
        )
    }

    /// One node in the lens, at the lens's viewport `vp`.
    #[allow(clippy::too_many_arguments)]
    fn lens_node(
        &mut self,
        node: &FlowNode,
        vp: Viewport,
        snapshot: &BoardSnapshot,
        board: &BoardCx,
        renderers: &std::rc::Rc<dyn NodeRenderers>,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let r = node.rect();
        let z = vp.zoom as f32;
        let (left, top) = ((vp.x + r.x * vp.zoom) as f32, (vp.y + r.y * vp.zoom) as f32);
        let (w, h) = ((r.w * vp.zoom) as f32, (r.h * vp.zoom) as f32);
        let state = NodeState {
            id: node.id.clone().into(),
            size: size(r.w as f32, r.h as f32),
            selected: false,
            dragging: false,
            zoom: z,
        };
        let content: AnyElement = match &node.kind {
            NodeKind::Card { index, .. } => {
                let card = &snapshot.cards[*index];
                // An expanded card's chat, its last rows drawn again at the
                // lens's zoom.
                let chat = (is_expanded(card) && !is_terminal(card))
                    .then(|| crate::terminal::card::card_chat(&card.node.session_id, cx))
                    .flatten();
                match chat {
                    Some(chat) => {
                        let count = ((r.h / 40.0).ceil() as usize).clamp(4, 24);
                        let rows = chat.update(cx, |c, cx| c.lens_rows(z, count, window, cx));
                        lens_chat_card(card, rows, (r.w as f32, r.h as f32), Z(z), t)
                    }
                    None => lens_card(
                        card,
                        (r.w as f32, r.h as f32),
                        Z(z),
                        Some(snapshot.project.repo_path.as_str()),
                        crate::parity::now_ms(cx),
                        t,
                    ),
                }
            }
            NodeKind::Note { index: Some(i) } => renderers.note(
                NoteNode {
                    node: state,
                    note: &snapshot.notes[*i],
                    draft: false,
                },
                board,
                window,
                cx,
            ),
            // The pad's loose top sheet is the pad's own picture.
            NodeKind::Note { index: None } => return None,
            NodeKind::Pad => renderers.pad(PadNode { node: state }, board, window, cx),
            NodeKind::Archive { count } => lens_archive(*count, (r.w as f32, r.h as f32), Z(z), t),
            NodeKind::View { index } => {
                let view = &snapshot.views[*index];
                let sharp = self.lens_picture(&view.id, vp.zoom, cx);
                lens_view(view, sharp, (r.w as f32, r.h as f32), Z(z), t, cx)
            }
            NodeKind::Shape { index } => {
                let shape = &snapshot.shapes[*index];
                let image = shape.image.as_deref().and_then(|name| self.image_path(name, cx));
                renderers.shape(
                    ShapeNode {
                        node: state,
                        shape,
                        image,
                        drawing: None,
                    },
                    board,
                    window,
                    cx,
                )
            }
            NodeKind::Tile { index } => {
                let tile = &snapshot.tiles[*index];
                let action = snapshot.project.actions.iter().flatten().find(|a| a.id == tile.id);
                let running = snapshot
                    .cards
                    .iter()
                    .find(|c| !c.node.archived && c.node.run_of.as_deref() == Some(tile.id.as_str()))
                    .map(|c| c.status);
                renderers.tile(
                    TileNode {
                        node: state,
                        tile,
                        action,
                        running,
                    },
                    board,
                    window,
                    cx,
                )
            }
            NodeKind::Goal { index } => {
                let goal = &snapshot.goals[*index];
                let running = snapshot
                    .cards
                    .iter()
                    .find(|c| c.node.session_id == goal.session_id && c.live)
                    .map(|c| c.status);
                renderers.goal(
                    GoalNode {
                        node: state,
                        goal,
                        running,
                    },
                    board,
                    window,
                    cx,
                )
            }
            NodeKind::Section { index, count, status } => {
                // A frame being renamed has its one field on the board.
                if self.renaming.as_deref() == Some(node.id.as_str()) {
                    return None;
                }
                let props = SectionProps {
                    section: snapshot.sections[*index].clone(),
                    count: *count,
                    status: *status,
                    hot: false,
                    zoom: z,
                    naming: None,
                };
                self.render_section(&props, window, cx)
            }
        };
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .child(zoomed(z, content))
                .into_any_element(),
        )
    }

    /// LOUPE-006: the page's picture at the lens's zoom `lens`, once WebKit
    /// has drawn it; asked for when there's none for these zooms, or it's
    /// older than [`PICTURE_FRESH`].
    fn lens_picture(&mut self, view_id: &str, lens: f64, cx: &mut Context<'_, Self>) -> Option<Arc<RenderImage>> {
        let zooms = (self.viewport.zoom, lens);
        let held = self.loupe.pictures.get(view_id).filter(|p| p.zooms == zooms);
        let current = held.map(|p| p.image.clone());
        let stale = held.is_none_or(|p| p.at.elapsed() > PICTURE_FRESH);
        if !stale || self.loupe.asked.contains(view_id) {
            return current;
        }
        let frames = crate::views::frames::existing(cx)?;
        let frames = frames.borrow();
        let Some(frame) = frames.get(view_id, crate::views::frames::Slot::Card) else {
            return current;
        };
        self.loupe.asked.insert(view_id.to_owned());
        let me = cx.entity().downgrade();
        let async_cx = cx.to_async();
        let id = view_id.to_owned();
        frame.web.snapshot_scaled(lens / zooms.0, move |image| {
            // WebKit's completion runs from the run loop, outside GPUI: hop in.
            async_cx.update(|cx| {
                let _ = me.update(cx, |c, cx| {
                    c.loupe.asked.remove(&id);
                    let Some(image) = image else { return };
                    if c.overlays.ink.tool != InkTool::Loupe {
                        cx.drop_image(image, None);
                        return;
                    }
                    let picture = LensPicture {
                        zooms,
                        at: Instant::now(),
                        image,
                    };
                    if let Some(old) = c.loupe.pictures.insert(id, picture) {
                        cx.drop_image(old.image, None);
                    }
                    c.loupe.tick.update(cx, |_, cx| cx.notify());
                });
            });
        });
        current
    }

    /// The loupe put down: its pictures go from the atlas.
    fn loupe_drop_pictures(&mut self, cx: &mut Context<'_, Self>) {
        for (_, picture) in self.loupe.pictures.drain() {
            cx.drop_image(picture.image, None);
        }
    }

    /// LOUPE-005: what the lens is over, for its label: the topmost node
    /// under the board point `f`, and its session's status colour.
    fn lens_label(&self, f: nodus_board::geometry::Point, t: &Tokens) -> Option<(String, Option<Rgba>)> {
        let board = self.board.as_ref()?;
        let order = paint_order(&self.nodes);
        let node = order.iter().rev().map(|&i| &self.nodes[i]).find(|n| {
            let r = n.rect();
            f.x >= r.x && f.x <= r.x + r.w && f.y >= r.y && f.y <= r.y + r.h
        })?;
        let words = |text: &str| {
            let first: Vec<&str> = text.split_whitespace().take(6).collect();
            let more = text.split_whitespace().count() > first.len();
            format!("{}{}", first.join(" "), if more { "…" } else { "" })
        };
        let label = match &node.kind {
            NodeKind::Card { index, .. } => {
                let card = &board.cards[*index];
                let color = (!is_terminal(card)).then(|| t.status(card.status));
                (present::title(card), color)
            }
            NodeKind::Note { index: Some(i) } => (words(&board.notes[*i].text), None),
            NodeKind::View { index } => (board.views[*index].title.clone(), None),
            NodeKind::Shape { index } => {
                let shape = &board.shapes[*index];
                if shape.image.is_some() {
                    ("Picture".to_owned(), None)
                } else {
                    (words(&shape.text), None)
                }
            }
            NodeKind::Tile { index } => {
                let tile = &board.tiles[*index];
                let action = board.project.actions.iter().flatten().find(|a| a.id == tile.id);
                (action.map_or_else(String::new, |a| a.name.clone()), None)
            }
            NodeKind::Goal { index } => (board.goals[*index].title.clone(), None),
            NodeKind::Section { index, .. } => (board.sections[*index].name.clone(), None),
            NodeKind::Archive { .. } => ("Archive".to_owned(), None),
            NodeKind::Pad | NodeKind::Note { index: None } => ("Notes".to_owned(), None),
        };
        (!label.0.trim().is_empty()).then_some(label)
    }
}

/// A card in the lens: its surface, status, title, preview (CARD-026) and
/// its branch and age at the foot, as the compact card shows them; an
/// expanded card shows as many of the preview's lines as its box holds.
fn lens_card(card: &SessionCard, (w, h): (f32, f32), z: Z, repo: Option<&str>, now_ms: f64, t: &Tokens) -> AnyElement {
    let dot = if is_terminal(card) {
        if card.live { t.status_done } else { t.text_tertiary }
    } else {
        t.status(card.status)
    };
    let pv = preview(card, repo);
    let lines = if is_expanded(card) {
        (((h - 14.0 - 11.0 - 16.0 - 37.0 - 25.0) / 17.4).floor() as usize).max(1)
    } else {
        4
    };
    let text = match pv.kind {
        PreviewKind::Path => para(
            pv.text,
            font(fonts::mono(), FontWeight::NORMAL, false),
            z.px(11.),
            z.px(15.95),
            t.text_secondary,
        )
        .zoom(z.0)
        .clamp(2),
        kind => para(
            pv.text,
            font(fonts::UI, FontWeight::NORMAL, kind == PreviewKind::Prompt),
            z.px(12.),
            z.px(17.4),
            if kind == PreviewKind::Empty {
                t.text_tertiary
            } else {
                t.text_secondary
            },
        )
        .zoom(z.0)
        .clamp(lines),
    };
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(w))
        .h(z.px(h))
        .flex()
        .flex_col()
        .gap(z.px(8.))
        .pt(z.px(14.))
        .px(z.px(16.))
        .pb(z.px(11.))
        .rounded(z.px(14.))
        .bg(t.bg_elevated)
        .shadow(t.sm())
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_start()
                .gap(z.px(9.))
                .child(
                    div()
                        .flex_none()
                        .pt(z.px(5.))
                        .child(div().size(z.px(10.)).rounded_full().bg(dot)),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        para(
                            present::title(card),
                            font(fonts::UI, FontWeight::SEMIBOLD, false),
                            z.px(13.),
                            z.px(18.2),
                            t.text_primary,
                        )
                        .zoom(z.0)
                        .clamp(2),
                    ),
                ),
        )
        .child(div().min_h_0().child(text))
        .child(
            div()
                .flex()
                .items_center()
                .gap(z.px(12.))
                .mt_auto()
                .pt(z.px(9.))
                .border_t(z.px(0.5))
                .border_color(t.separator)
                .h(z.px(9.0 + 0.5 + 15.4))
                .when_some(present::branch(card), |d, branch| {
                    d.child(div().min_w_0().overflow_hidden().child(super::cards::text_tab(
                        z,
                        branch,
                        11.0,
                        FontWeight::NORMAL,
                        t.text_tertiary,
                    )))
                })
                .child(div().ml_auto().flex_none().child(super::cards::text_tab(
                    z,
                    present::time_ago(present::updated_at(card), now_ms),
                    11.0,
                    FontWeight::NORMAL,
                    t.text_tertiary,
                ))),
        )
        .into_any_element()
}

/// An expanded card in the lens: its title bar, the last of its chat (the
/// chat's own rows, newest at the foot as the chat keeps them) and its
/// composer's place.
fn lens_chat_card(card: &SessionCard, rows: Vec<AnyElement>, (w, h): (f32, f32), z: Z, t: &Tokens) -> AnyElement {
    let busy = card.live && matches!(card.status, SessionStatus::Working | SessionStatus::Waiting);
    let placeholder = if !card.live {
        "Message Claude to resume this session…"
    } else if busy {
        "Queue a message for when Claude is done…"
    } else {
        "Message Claude…"
    };
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(w))
        .h(z.px(h))
        .flex()
        .flex_col()
        .rounded(z.px(14.))
        .bg(t.bg_elevated)
        .shadow(t.sm())
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(z.px(9.))
                .h(z.px(40.))
                .pl(z.px(14.))
                .pr(z.px(6.))
                .border_b(z.px(0.5))
                .border_color(t.separator)
                .child(
                    div()
                        .flex_none()
                        .size(z.px(10.))
                        .rounded_full()
                        .bg(t.status(card.status)),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        para(
                            present::title(card),
                            font(fonts::UI, FontWeight::SEMIBOLD, false),
                            z.px(13.),
                            z.px(18.2),
                            t.text_primary,
                        )
                        .zoom(z.0)
                        .clamp(1),
                    ),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .justify_end()
                .children(rows.into_iter().map(|row| div().flex_none().w_full().child(row))),
        )
        .child(
            div()
                .flex_none()
                .mx(z.px(12.))
                .mb(z.px(12.))
                .h(z.px(40.))
                .px(z.px(12.))
                .flex()
                .items_center()
                .rounded(z.px(12.))
                .border(z.px(1.))
                .border_color(t.border)
                .text_size(z.px(13.))
                .text_color(t.text_tertiary)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(placeholder),
        )
        .into_any_element()
}

/// A view in the lens: its caption and its page: the `sharp` picture taken
/// at the lens's zoom, else the board's own (VIEW-141), else its title on
/// the panel before the page has one.
fn lens_view(
    view: &BoardView,
    sharp: Option<Arc<RenderImage>>,
    (w, h): (f32, f32),
    z: Z,
    t: &Tokens,
    cx: &App,
) -> AnyElement {
    let picture = sharp.or_else(|| {
        crate::views::frames::existing(cx).and_then(|frames| {
            frames
                .borrow()
                .get(&view.id, crate::views::frames::Slot::Card)
                .and_then(|f| f.snapshot.clone())
        })
    });
    let caption_h = crate::views::card::CAPTION_H;
    let radius = z.f(VIEW_RADIUS);
    let page = match picture {
        Some(image) => canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let _ = window.paint_image(bounds, bounds, Corners::all(px(radius)), image.clone(), 0, false);
            },
        )
        .size_full()
        .into_any_element(),
        None => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(z.px(12.))
            .text_color(t.text_tertiary)
            .child(view.title.clone())
            .into_any_element(),
    };
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(w))
        .h(z.px(h))
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .h(z.px(caption_h))
                .flex()
                .items_center()
                .px(z.px(4.))
                .text_size(z.px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text_secondary)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(view.title.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .rounded(px(radius))
                .when(view.has_background(), |d| d.bg(t.bg_elevated).shadow(t.sm()))
                .child(page),
        )
        .into_any_element()
}

/// The archive in the lens: its zone and how many it holds.
fn lens_archive(count: usize, (w, h): (f32, f32), z: Z, t: &Tokens) -> AnyElement {
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(w))
        .h(z.px(h))
        .rounded(z.px(16.))
        .bg(t.archive_bg)
        .border(z.px(1.5))
        .border_dashed()
        .border_color(t.archive_border)
        .p(z.px(14.))
        .text_size(z.px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.archive_label)
        .child(format!("Archive · {count}"))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_reads_at_three_times_the_zoom_within_bounds() {
        assert!((lens_zoom(0.28) - 0.84).abs() < 1e-9);
        assert_eq!(lens_zoom(0.1), 0.75);
        assert_eq!(lens_zoom(0.6), 1.25);
        assert!(magnifies(0.1) && magnifies(1.2));
        assert!(!magnifies(1.25) && !magnifies(2.0));
    }
}
