//! The board: an infinite canvas with pan / zoom-around-pointer, draggable cards, rubber-band
//! selection, a terminal card and a webview card.
//!
//! Transform approach: board state is `pan` (screen offset of the world origin, board-local px)
//! and `zoom`. Cards are absolutely positioned divs at `pan + pos * zoom` with size `size * zoom`;
//! their content is wrapped in `Zoomed` (rem = 16px * zoom) and styled in rems, so text, padding,
//! radii scale. The terminal element reads the rem scale. Input is handled at the board root,
//! drags use window-level mouse listeners registered during paint (so they keep tracking outside
//! the hovered hitbox).
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::terminal::TerminalView;
use crate::theme::tokens;
use crate::webview::WebCard;
use crate::zoom::zoomed;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CardKind {
    Note,
    Session,
    Terminal,
    View,
    Sticker,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub id: usize,
    pub kind: CardKind,
    pub title: SharedString,
    pub body: SharedString,
    pub pos: Point<f32>,
    pub size: Size<f32>,
}

#[derive(Clone)]
pub enum Drag {
    Cards { start: Point<Pixels>, origins: Vec<(usize, Point<f32>)> },
    Rubber { start: Point<Pixels>, current: Point<Pixels>, base: BTreeSet<usize> },
}

#[derive(Default)]
pub struct FrameStats {
    pub stamps: Vec<Instant>,
    pub paint_cost: Vec<Duration>,
    pub recording: bool,
}

impl FrameStats {
    pub fn summary(&self) -> String {
        if self.stamps.len() < 2 {
            return "no frames".into();
        }
        let gaps: Vec<f64> = self.stamps.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.).collect();
        let total = (*self.stamps.last().unwrap() - self.stamps[0]).as_secs_f64();
        let fps = (self.stamps.len() - 1) as f64 / total;
        let mut sorted = gaps.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = sorted[sorted.len() / 2];
        let p95 = sorted[(sorted.len() as f64 * 0.95) as usize - if sorted.len() > 20 { 1 } else { 0 }];
        let max = *sorted.last().unwrap();
        let over = gaps.iter().filter(|g| **g > 25.0).count();
        let mut cost: Vec<f64> = self.paint_cost.iter().map(|d| d.as_secs_f64() * 1000.).collect();
        cost.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let cost_p50 = cost.get(cost.len() / 2).copied().unwrap_or(0.);
        let cost_max = cost.last().copied().unwrap_or(0.);
        format!(
            "frames={} fps={fps:.1} gap_p50={p50:.2}ms gap_p95={p95:.2}ms gap_max={max:.2}ms gaps>25ms={over} render_p50={cost_p50:.2}ms render_max={cost_max:.2}ms",
            self.stamps.len()
        )
    }
}

pub struct Board {
    pub pan: Point<f32>,
    pub zoom: f32,
    pub cards: Vec<Card>,
    pub selected: BTreeSet<usize>,
    pub drag: Option<Drag>,
    pub viewport: Bounds<Pixels>,
    pub focus: FocusHandle,
    pub term: Entity<TerminalView>,
    pub web: Option<Rc<WebCard>>,
    pub ipc_log: Vec<String>,
    pub frames: FrameStats,
    pub animate: Option<Box<dyn FnMut(&mut Board, u32) -> bool>>,
    anim_tick: u32,
    render_started: Option<Instant>,
    /// An ink tool is out: views must show their snapshot so ink draws over them.
    pub ink_mode: bool,
    snap_tx: smol::channel::Sender<()>,
    _ipc_task: Option<Task<()>>,
    _snap_task: Task<()>,
}

pub const TERM_CARD: usize = 3;
pub const VIEW_CARD: usize = 4;
pub const STICKER_CARD: usize = 6;

impl Board {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let term = cx.new(TerminalView::new);
        let card = |id, kind, title: &str, body: &str, x, y, w, h| Card {
            id,
            kind,
            title: SharedString::from(title.to_string()),
            body: SharedString::from(body.to_string()),
            pos: point(x, y),
            size: size(w, h),
        };
        let cards = vec![
            card(0, CardKind::Session, "Fix the flaky test", "claude · working\nRunning pnpm test…", 40., 40., 288., 216.),
            card(1, CardKind::Session, "Landing page copy", "claude · done\nUpdated hero + badges.", 360., 40., 288., 216.),
            card(2, CardKind::Note, "Note", "Remember: ship the Windows\ninstaller after notarization.", 40., 300., 220., 160.),
            card(TERM_CARD, CardKind::Terminal, "zsh — terminal card", "", 680., 40., 560., 360.),
            card(VIEW_CARD, CardKind::View, "View · demo", "", 290., 300., 360., 260.),
            // Overlaps the view card's bottom-right corner on purpose (z-order test).
            card(5, CardKind::Note, "Overlapping note", "I am drawn after the view card.\nIs the webview on top of me?", 560., 480., 240., 140.),
            card(STICKER_CARD, CardKind::Sticker, "Sticker", "Tilted 3°\nGPU-rotated SVG layers\ndrag me: I tilt more", 40., 520., 200., 120.),
        ];

        let (web, ipc_task) = match WebCard::new(window) {
            Ok((web, rx)) => {
                let web = Rc::new(web);
                let task = cx.spawn(async move |this, cx| {
                    while let Ok(msg) = rx.recv().await {
                        let ok = this.update(cx, |board, cx| {
                            board.on_ipc(msg, cx);
                        });
                        if ok.is_err() {
                            break;
                        }
                    }
                });
                (Some(web), Some(task))
            }
            Err(e) => {
                log::error!("webview failed: {e:#}");
                (None, None)
            }
        };

        let (snap_tx, snap_rx) = smol::channel::unbounded::<()>();
        let snap_task = cx.spawn(async move |this, cx| {
            while snap_rx.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            pan: point(0., 0.),
            zoom: 1.0,
            cards,
            selected: BTreeSet::new(),
            drag: None,
            viewport: Bounds::default(),
            focus: cx.focus_handle(),
            term,
            web,
            ipc_log: Vec::new(),
            frames: FrameStats::default(),
            animate: None,
            anim_tick: 0,
            render_started: None,
            ink_mode: false,
            snap_tx,
            _ipc_task: ipc_task,
            _snap_task: snap_task,
        }
    }

    fn on_ipc(&mut self, msg: String, cx: &mut Context<Self>) {
        log::info!("ipc from page: {msg}");
        println!("S1 IPC {msg}");
        self.ipc_log.push(msg.clone());
        // The page changed: the next obstruction must capture a fresh snapshot.
        if let Some(web) = &self.web {
            *web.snapshot.borrow_mut() = None;
            web.showing_snapshot.set(false);
        }
        // Rust -> JS reply.
        if msg.contains("\"ping\"") {
            if let Some(web) = &self.web {
                let n = msg.split("\"n\":").nth(1).and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next()).unwrap_or("0").to_string();
                let t = msg.split("\"t\":").nth(1).and_then(|s| s.split(|c: char| c == '}' || c == ',').next()).unwrap_or("0").to_string();
                let js = format!("window.__nodusReply({{\"from\":\"rust\",\"n\":{n},\"t\":{t},\"zoom\":{:.2}}})", self.zoom);
                let _ = web.webview.evaluate_script(&js);
            }
        }
        cx.notify();
    }

    /// Anything GPUI draws over the view card (a later card overlapping it, the view itself being
    /// dragged, an ink tool) means the live native view must give way to its snapshot.
    pub fn view_obstructed(&self) -> bool {
        let Some(i) = self.cards.iter().position(|c| c.id == VIEW_CARD) else { return false };
        let v = &self.cards[i];
        let overlaps = self.cards[i + 1..].iter().any(|c| {
            c.pos.x < v.pos.x + v.size.width && c.pos.x + c.size.width > v.pos.x && c.pos.y < v.pos.y + v.size.height && c.pos.y + c.size.height > v.pos.y
        });
        let dragging_view = matches!(&self.drag, Some(Drag::Cards { origins, .. }) if origins.iter().any(|(id, _)| *id == VIEW_CARD));
        overlaps || dragging_view || self.ink_mode
    }

    fn sync_webview_mode(&mut self) {
        let obstructed = self.view_obstructed();
        let Some(web) = self.web.clone() else { return };
        // While the board itself is being dragged/panned, the pointer must reach GPUI even when it
        // passes over the page.
        web.set_passthrough(self.drag.is_some() || self.animate.is_some());
        if obstructed {
            if web.snapshot.borrow().is_some() {
                web.showing_snapshot.set(true);
            } else {
                let w2 = web.clone();
                let tx = self.snap_tx.clone();
                let t0 = Instant::now();
                web.request_snapshot(move |img| {
                    println!("S1 SNAPSHOT_LATENCY {:.1}ms ok={}", t0.elapsed().as_secs_f64() * 1000., img.is_some());
                    w2.snapshot_pending.set(false);
                    *w2.snapshot.borrow_mut() = img;
                    let _ = tx.try_send(());
                });
            }
        } else if web.showing_snapshot.get() {
            web.showing_snapshot.set(false);
            // Stale once the page is live again; the next obstruction captures a fresh one.
            *web.snapshot.borrow_mut() = None;
        }
    }

    // ---- coordinates ----
    pub fn to_local(&self, p: Point<Pixels>) -> Point<f32> {
        point(f32::from(p.x - self.viewport.origin.x), f32::from(p.y - self.viewport.origin.y))
    }
    pub fn to_world(&self, p: Point<Pixels>) -> Point<f32> {
        let l = self.to_local(p);
        point((l.x - self.pan.x) / self.zoom, (l.y - self.pan.y) / self.zoom)
    }
    pub fn card_screen_bounds(&self, c: &Card) -> Bounds<Pixels> {
        Bounds::new(
            point(
                self.viewport.origin.x + px(self.pan.x + c.pos.x * self.zoom),
                self.viewport.origin.y + px(self.pan.y + c.pos.y * self.zoom),
            ),
            size(px(c.size.width * self.zoom), px(c.size.height * self.zoom)),
        )
    }

    pub fn zoom_at(&mut self, p: Point<Pixels>, factor: f32) {
        let local = self.to_local(p);
        let new_zoom = (self.zoom * factor).clamp(0.2, 3.0);
        let k = new_zoom / self.zoom;
        self.pan = point(local.x - (local.x - self.pan.x) * k, local.y - (local.y - self.pan.y) * k);
        self.zoom = new_zoom;
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let zoom_mod = ev.modifiers.control || ev.modifiers.platform;
        match ev.delta {
            ScrollDelta::Pixels(d) => {
                if zoom_mod {
                    self.zoom_at(ev.position, (-f32::from(d.y) * 0.01).exp());
                } else {
                    self.pan.x += f32::from(d.x);
                    self.pan.y += f32::from(d.y);
                }
            }
            ScrollDelta::Lines(l) => {
                if zoom_mod {
                    self.zoom_at(ev.position, (-l.y * 0.1).exp());
                } else {
                    self.pan.x += l.x * 40.;
                    self.pan.y += l.y * 40.;
                }
            }
        }
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_at(ev.position, 1.0 + ev.delta);
        cx.notify();
    }

    pub fn start_card_drag(&mut self, id: usize, pos: Point<Pixels>, shift: bool, cx: &mut Context<Self>) {
        if shift {
            if !self.selected.insert(id) {
                self.selected.remove(&id);
            }
        } else if !self.selected.contains(&id) {
            self.selected.clear();
            self.selected.insert(id);
        }
        // bring to front
        if let Some(i) = self.cards.iter().position(|c| c.id == id) {
            let c = self.cards.remove(i);
            self.cards.push(c);
        }
        let origins = self.cards.iter().filter(|c| self.selected.contains(&c.id)).map(|c| (c.id, c.pos)).collect();
        self.drag = Some(Drag::Cards { start: pos, origins });
        cx.notify();
    }

    fn on_bg_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let base = if ev.modifiers.shift { self.selected.clone() } else { BTreeSet::new() };
        self.selected = base.clone();
        self.drag = Some(Drag::Rubber { start: ev.position, current: ev.position, base });
        cx.notify();
    }

    pub fn drag_move(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        match self.drag.clone() {
            Some(Drag::Cards { start, origins }) => {
                let dx = f32::from(pos.x - start.x) / self.zoom;
                let dy = f32::from(pos.y - start.y) / self.zoom;
                for (id, o) in origins {
                    if let Some(c) = self.cards.iter_mut().find(|c| c.id == id) {
                        c.pos = point(o.x + dx, o.y + dy);
                    }
                }
            }
            Some(Drag::Rubber { start, base, .. }) => {
                let a = self.to_world(start);
                let b = self.to_world(pos);
                let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
                let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
                let mut sel = base.clone();
                for c in &self.cards {
                    let hit = c.pos.x < x1 && c.pos.x + c.size.width > x0 && c.pos.y < y1 && c.pos.y + c.size.height > y0;
                    if hit {
                        sel.insert(c.id);
                    }
                }
                self.selected = sel;
                self.drag = Some(Drag::Rubber { start, current: pos, base });
            }
            None => return,
        }
        cx.notify();
    }

    pub fn drag_end(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    fn render_sticker(&self, c: &Card, cx: &mut Context<Self>) -> AnyElement {
        let t = tokens(cx);
        let z = self.zoom;
        let id = c.id;
        let lines: Vec<&str> = c.body.split('\n').collect();
        let layers = std::rc::Rc::new(crate::rotated::sticker_layers(c.size.width, c.size.height, &lines));
        let dragging = matches!(&self.drag, Some(Drag::Cards { origins, .. }) if origins.iter().any(|(i, _)| *i == id));
        let paper = rgba(0xffd60aff).into();
        let colors = (t.shadow_b.opacity(0.55), paper, rgba(0x000000d9).into());
        let tilt_from = if dragging { 3.0 } else { 7.0 };
        let tilt_to = if dragging { 7.0 } else { 3.0 };
        // Nodus --ease-stack: a damped spring as CSS linear() with 29 stops.
        let ease = crate::rotated::css_linear(vec![
            0., 0.05, 0.1697, 0.3218, 0.4803, 0.6281, 0.7553, 0.8579, 0.9355, 0.9902, 1.0254, 1.0452, 1.0534, 1.0536, 1.0487,
            1.0411, 1.0326, 1.0242, 1.0166, 1.0104, 1.0055, 1.0019, 0.9995, 0.998, 0.9973, 0.9971, 0.9972, 0.9975, 1.,
        ]);
        let fade = crate::rotated::cubic_bezier(0.2, 0.8, 0.2, 1.0);
        div()
            .id(("card", id))
            .absolute()
            .left(px(self.pan.x + c.pos.x * z))
            .top(px(self.pan.y + c.pos.y * z))
            .w(px(c.size.width * z))
            .h(px(c.size.height * z))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.start_card_drag(id, ev.position, ev.modifiers.shift, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .size_full()
                    .child(div().size_full().with_animation(
                        if dragging { "tilt-on" } else { "tilt-off" },
                        Animation::new(std::time::Duration::from_millis(420)).with_easing(ease),
                        move |el, delta| {
                            let angle = tilt_from + (tilt_to - tilt_from) * delta;
                            let layers = layers.clone();
                            el.child(
                                canvas(|_, _, _| {}, move |bounds, _, window, cx| {
                                    crate::rotated::paint_sticker(&layers, bounds, z, angle, colors, window, cx);
                                })
                                .size_full(),
                            )
                        },
                    ))
                    // Per-element opacity, eased with CSS cubic-bezier(0.2, 0.8, 0.2, 1).
                    .with_animation("fade-in", Animation::new(std::time::Duration::from_millis(600)).with_easing(fade), |el, d| el.opacity(d)),
            )
            .into_any_element()
    }

    fn render_card(&self, c: &Card, cx: &mut Context<Self>) -> AnyElement {
        if c.kind == CardKind::Sticker {
            return self.render_sticker(c, cx);
        }
        let t = tokens(cx);
        let z = self.zoom;
        let selected = self.selected.contains(&c.id);
        let id = c.id;
        let note = c.kind == CardKind::Note;
        let bg = if note { t.note_paper } else { t.bg_elevated };
        let ink = if note { t.note_ink } else { t.text_primary };

        let header = div()
            .id(("hdr", id))
            .flex()
            .items_center()
            .gap(rems(0.5))
            .h(rems(2.0))
            .px(rems(0.75))
            .flex_shrink_0()
            .text_size(rems(0.8125))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ink)
            .when(!note, |d| d.border_b_1().border_color(t.border))
            .cursor_grab()
            .child(div().size(rems(0.5)).rounded_full().bg(match c.kind {
                CardKind::Session if c.body.contains("working") => rgba(0xff9500ff).into(),
                CardKind::Session => t.accent,
                CardKind::Terminal => rgba(0x28cd41ff).into(),
                CardKind::View => rgba(0xaf52deff).into(),
                CardKind::Note | CardKind::Sticker => transparent_black(),
            }))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(c.title.clone()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.start_card_drag(id, ev.position, ev.modifiers.shift, cx);
                    cx.stop_propagation();
                }),
            );

        let body: AnyElement = match c.kind {
            CardKind::Terminal => div()
                .flex_1()
                .overflow_hidden()
                .rounded_b(rems(0.625))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if !this.selected.contains(&id) || ev.modifiers.shift {
                        this.selected.clear();
                        this.selected.insert(id);
                    }
                    cx.notify();
                    // no stop_propagation: let the terminal view focus itself; but don't start a rubber band:
                }))
                .child(self.term.clone())
                .into_any_element(),
            CardKind::View => {
                let web = self.web.clone();
                let zoom = z;
                let snapshot = self
                    .web
                    .as_ref()
                    .filter(|w| w.showing_snapshot.get())
                    .and_then(|w| w.snapshot.borrow().clone());
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .rounded_b(rems(0.625))
                    .bg(t.bg_elevated)
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                // Follow the card: position the native view during prepaint.
                                if let Some(web) = &web {
                                    let clip = window.content_mask().bounds;
                                    web.place(bounds, clip, zoom, 10. * zoom);
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(snapshot.map(|img| {
                        // GPUI-drawn stand-in: everything GPUI paints later (notes, menus, ink) is over it.
                        gpui::img(ImageSource::Render(img)).absolute().size_full().rounded_b(rems(0.625))
                    }))
                    .into_any_element()
            }
            _ => div()
                .flex_1()
                .p(rems(0.75))
                .text_size(rems(0.8125))
                .line_height(rems(1.25))
                .text_color(if note { t.note_ink } else { t.text_secondary })
                .children(c.body.split('\n').map(|l| div().child(SharedString::from(l.to_string()))))
                .into_any_element(),
        };

        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .rounded(rems(0.625))
            .bg(bg)
            // Border in rems (scales with zoom) and the same width when selected: the terminal grid
            // is derived from the card's inner size, so neither zoom nor selection may change it.
            .border(rems(1. / 16.))
            .border_color(if selected { t.accent } else { t.border })
            .child(header)
            .child(body);

        let shadow = vec![
            BoxShadow { color: t.shadow_a, offset: point(px(0.), px(1. * z)), blur_radius: px(2. * z), spread_radius: px(0.), inset: false },
            BoxShadow {
                color: t.shadow_b.opacity(if selected { 0.9 } else { 0.6 }),
                offset: point(px(0.), px(8. * z)),
                blur_radius: px(18. * z),
                spread_radius: px(-6. * z),
                inset: false,
            },
        ];

        div()
            .id(("card", id))
            .absolute()
            .left(px(self.pan.x + c.pos.x * z))
            .top(px(self.pan.y + c.pos.y * z))
            .w(px(c.size.width * z))
            .h(px(c.size.height * z))
            .rounded(px(10. * z))
            .shadow(shadow)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(zoomed(z, content))
            .into_any_element()
    }
}

impl Focusable for Board {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Board {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_started = Some(Instant::now());
        if let Some(mut f) = self.animate.take() {
            self.anim_tick += 1;
            if f(self, self.anim_tick) {
                self.animate = Some(f);
                window.request_animation_frame();
            } else {
                self.anim_tick = 0;
            }
        }
        self.sync_webview_mode();
        let t = tokens(cx);
        let board = cx.entity();
        let dragging = self.drag.is_some();
        let (pan, zoom) = (self.pan, self.zoom);
        let dot = t.canvas_dot;

        let rubber = match &self.drag {
            Some(Drag::Rubber { start, current, .. }) => {
                let a = self.to_local(*start);
                let b = self.to_local(*current);
                Some(
                    div()
                        .absolute()
                        .left(px(a.x.min(b.x)))
                        .top(px(a.y.min(b.y)))
                        .w(px((a.x - b.x).abs()))
                        .h(px((a.y - b.y).abs()))
                        .bg(t.bg_selected)
                        .border_1()
                        .border_color(t.accent)
                        .rounded(px(2.)),
                )
            }
            _ => None,
        };

        let cards: Vec<AnyElement> = self.cards.clone().iter().map(|c| self.render_card(c, cx)).collect();
        let hud = format!(
            "zoom {:.0}% · pan {:.0},{:.0} · selected {} · ipc {}",
            self.zoom * 100.,
            self.pan.x,
            self.pan.y,
            self.selected.len(),
            self.ipc_log.len()
        );

        div()
            .id("board")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(t.bg_canvas)
            .track_focus(&self.focus)
            .on_scroll(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_bg_mouse_down))
            .child(
                canvas(
                    {
                        let board = board.clone();
                        move |bounds, _, cx| {
                            board.update(cx, |b, _| b.viewport = bounds);
                        }
                    },
                    {
                        let board = board.clone();
                        move |bounds, _, window, _cx| {
                            // Dot grid in screen space, 24 world px apart.
                            let step = 24.0 * zoom;
                            if step >= 8.0 {
                                let r = (1.2 * zoom.sqrt()).max(0.8);
                                let ox = pan.x.rem_euclid(step);
                                let oy = pan.y.rem_euclid(step);
                                let w = f32::from(bounds.size.width);
                                let h = f32::from(bounds.size.height);
                                let mut y = oy;
                                while y < h {
                                    let mut x = ox;
                                    while x < w {
                                        window.paint_quad(
                                            fill(Bounds::new(bounds.origin + point(px(x - r), px(y - r)), size(px(2. * r), px(2. * r))), dot)
                                                .corner_radii(px(r)),
                                        );
                                        x += step;
                                    }
                                    y += step;
                                }
                            }
                            if dragging {
                                let b = board.clone();
                                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                                    if phase == DispatchPhase::Bubble {
                                        b.update(cx, |b, cx| b.drag_move(ev.position, cx));
                                    }
                                });
                                let b = board.clone();
                                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                    if phase == DispatchPhase::Bubble {
                                        b.update(cx, |b, cx| b.drag_end(cx));
                                    }
                                });
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(cards)
            .children(rubber)
            .child(
                div()
                    .absolute()
                    .left(px(12.))
                    .bottom(px(10.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .bg(t.bg_toolbar)
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(11.))
                    .text_color(t.text_secondary)
                    .child(hud),
            )
            .child(
                // Frame clock: last thing painted each frame.
                canvas(|_, _, _| {}, move |_, _, _, cx| {
                    board.update(cx, |b, _| {
                        if b.frames.recording {
                            b.frames.stamps.push(Instant::now());
                            if let Some(s) = b.render_started {
                                b.frames.paint_cost.push(s.elapsed());
                            }
                        }
                    });
                })
                .absolute()
                .size_0(),
            )
    }
}
