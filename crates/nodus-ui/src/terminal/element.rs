//! The terminal's GPUI element: draws `nodus-terminal`'s snapshot as
//! xterm.js 6's WebGL renderer drew it in 0.9.4. Cells sit on a fixed grid
//! of device pixels (each glyph placed at its cell, whatever the font's
//! advance); backgrounds, the selection, bold-as-bright, dim and inverse are
//! already resolved in the snapshot (TERM-058, TERM-079); box drawing,
//! blocks and powerline symbols are shapes (TERM-080); the cursor
//! (TERM-076), the IME's composition (TERM-055), a hovered link's underline
//! (TERM-066) and the scrollbar (TERM-075) go on top.

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, Bounds, ContentMask, Corners, DispatchPhase, Element, ElementId, ElementInputHandler, ElementTransform,
    Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayerEffects, LayoutId,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Rgba, Style, TextRun, Window, fill, point, px, relative,
    size,
};
use nodus_terminal::palette::{self, Rgb};
use nodus_terminal::{CursorKind, Snapshot, Underline};

use super::glyphs::{self, Cmd, Prim};
use super::host::{Geometry, SCROLLBAR_W, TerminalHost};
use super::metrics::{self, TermMetrics};
use crate::parity;

/// `--term-bg`.
pub const TERM_BG: Rgb = palette::theme::BACKGROUND;

pub fn rgb(c: Rgb) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    }
    .into()
}

fn rgba(c: palette::Rgba) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: c.alpha(),
    }
    .into()
}

/// The element for a host's terminal.
pub struct TerminalElement {
    host: Entity<TerminalHost>,
}

impl TerminalElement {
    pub fn new(host: Entity<TerminalHost>) -> Self {
        Self { host }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// A run of cells in one font, shaped together; each glyph is then placed
/// at the cell it came from.
struct GlyphRun {
    row: usize,
    layout: Arc<gpui::LineLayout>,
    /// The byte offset each cell's text starts at in the run, and its column.
    starts: Vec<(usize, usize)>,
}

/// What prepaint works out for paint.
pub struct Prepaint {
    metrics: TermMetrics,
    geometry: Geometry,
    snapshot: Option<Arc<Snapshot>>,
    runs: Vec<GlyphRun>,
    hitbox: Hitbox,
    cursor_on: bool,
    link: Option<nodus_terminal::links::Link>,
    marked: Option<(String, Arc<gpui::LineLayout>)>,
    scrollbar: Option<(Bounds<Pixels>, Hsla)>,
    history: usize,
    display_offset: usize,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let m = metrics::metrics(window);
        let dpr = m.dpr;
        // TERM-083: on a zoomed board the card's box is scaled; the terminal
        // is laid out (and fitted) at 1× and its picture scaled.
        let outer = bounds;
        let scale = self.host.read(cx).scale;
        let scaled = scale != 1.0;
        let bounds = if scaled {
            Bounds::new(outer.origin, size(outer.size.width / scale, outer.size.height / scale))
        } else {
            outer
        };
        self.host
            .update(cx, |host, cx| host.layout(bounds.size, &m, window, cx));
        // The canvas sits on whole device pixels.
        let snap = |v: Pixels| px((f32::from(v) * dpr).round() / dpr);
        let origin = point(snap(bounds.origin.x), snap(bounds.origin.y));

        let (snapshot, geometry, cursor_on, link, marked) = self.host.update(cx, |host, cx| {
            let fresh = host
                .registry
                .with_terminal(&host.session_id, |t| t.snapshot())
                .map(Arc::new);
            // TERM-082: keep the last drawing while the program holds it.
            let snapshot = match fresh {
                Some(s) if s.hold && host.shown.is_some() => {
                    host.hold_started(cx);
                    host.shown.clone()
                }
                Some(s) => {
                    host.hold_ended();
                    host.shown = Some(s.clone());
                    Some(s)
                }
                None => {
                    host.shown = None;
                    None
                }
            };
            let (cols, rows) = snapshot.as_ref().map_or((0, 0), |s| (s.cols, s.rows));
            let geometry = Geometry {
                bounds,
                origin,
                cell_w: m.cell_w(),
                cell_h: m.cell_h(),
                cols,
                rows,
                zoom: scaled.then_some((bounds.origin, scale)),
            };
            host.geometry = Some(geometry);
            (
                snapshot,
                geometry,
                host.blink_on || parity::frozen(cx),
                host.hovered_link.clone(),
                host.marked.clone(),
            )
        });

        // Glyph runs, row by row, one per font.
        let mut runs = Vec::new();
        if let Some(s) = &snapshot {
            let ts = window.text_system().clone();
            for (row, cells) in s.cells.iter().enumerate() {
                for bold in [false, true] {
                    for italic in [false, true] {
                        let mut text = String::new();
                        let mut starts = Vec::new();
                        for (col, cell) in cells.iter().enumerate() {
                            if cell.bold != bold || cell.italic != italic || cell.invisible {
                                continue;
                            }
                            if cell.text.is_empty() || cell.text == " " {
                                continue;
                            }
                            let mut chars = cell.text.chars();
                            if let (Some(c), None) = (chars.next(), chars.next())
                                && glyphs::is_custom(c)
                            {
                                continue;
                            }
                            starts.push((text.len(), col));
                            text.push_str(&cell.text);
                        }
                        if text.is_empty() {
                            continue;
                        }
                        let run = TextRun {
                            len: text.len(),
                            font: metrics::cell_font(bold, italic),
                            color: gpui::white(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let layout = ts.layout_line(&text, TermMetrics::font_size(), &[run], None);
                        runs.push(GlyphRun { row, layout, starts });
                    }
                }
            }
        }
        let marked = marked.map(|text| {
            let run = TextRun {
                len: text.len(),
                font: metrics::term_font(),
                color: gpui::white(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let layout = window
                .text_system()
                .layout_line(&text, TermMetrics::font_size(), &[run], None);
            (text, layout)
        });

        // TERM-075: the slider, while scrolling, hovered or dragged.
        let (history, display_offset) = snapshot.as_ref().map_or((0, 0), |s| (s.history_size, s.display_offset));
        let scrollbar = self.host.update(cx, |host, _| {
            let track = Bounds::new(
                point(bounds.right() - px(SCROLLBAR_W), bounds.origin.y),
                size(px(SCROLLBAR_W), bounds.size.height),
            );
            let (top, h) = host.slider(f32::from(bounds.size.height))?;
            let opacity = scrollbar_opacity(&host.scrollbar, Instant::now());
            if opacity <= 0.0 {
                return None;
            }
            let color = if host.scrollbar.grab.is_some() {
                palette::theme::SCROLLBAR_SLIDER_ACTIVE_BACKGROUND
            } else if host.scrollbar.hovered {
                palette::theme::SCROLLBAR_SLIDER_HOVER_BACKGROUND
            } else {
                palette::theme::SCROLLBAR_SLIDER_BACKGROUND
            };
            let mut color = rgba(color);
            color.a *= opacity;
            Some((
                Bounds::new(
                    point(track.origin.x, track.origin.y + px(top)),
                    size(track.size.width, px(h)),
                ),
                color,
            ))
        });
        if scrollbar.is_some() && self.host.read(cx).scrollbar.grab.is_none() {
            // Keep fading.
            window.request_animation_frame();
        }

        let hitbox = window.insert_hitbox(outer, HitboxBehavior::Normal);
        Prepaint {
            metrics: m,
            geometry,
            snapshot,
            runs,
            hitbox,
            cursor_on,
            link,
            marked,
            scrollbar,
            history,
            display_offset,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        // The text input and the pointer's shape.
        let focus = self.host.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.host.clone()), cx);
        let mouse = window.mouse_position();
        let style = self.host.read(cx).cursor_style(mouse);
        window.set_cursor_style(style, &p.hitbox);
        // A drag keeps following the pointer outside the terminal.
        if self.host.read(cx).is_dragging() {
            let host = self.host.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    host.update(cx, |h, cx| h.mouse_moved_while_dragging(e.position, cx));
                }
            });
            let host = self.host.clone();
            window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    host.update(cx, |h, cx| h.on_mouse_up(e, window, cx));
                }
            });
        }

        let g = p.geometry;
        match g.zoom {
            None => draw(p, window, point(px(0.), px(0.))),
            // TERM-083: drawn at 1× into a layer, then scaled into the card.
            Some((at, scale)) => {
                let viewport = window.viewport_size();
                for (tile, stage) in stage_tiles(g.bounds.size, at, viewport) {
                    // The tile's top-left in the terminal's own space.
                    let local = tile.origin;
                    let shift = point(
                        stage.x - local.x - g.bounds.origin.x,
                        stage.y - local.y - g.bounds.origin.y,
                    );
                    let t = at - stage + point(local.x * scale, local.y * scale);
                    let effects = LayerEffects {
                        transform: Some(
                            ElementTransform::default()
                                .origin(0., 0.)
                                .translate(t.x, t.y)
                                .scale(scale),
                        ),
                        ..LayerEffects::default()
                    };
                    let stage_bounds = Bounds::new(stage, tile.size);
                    window.paint_effect_layer(stage_bounds, Corners::default(), &effects, |window| {
                        window.with_content_mask(Some(ContentMask { bounds: stage_bounds }), |window| {
                            draw(p, window, shift);
                        });
                    });
                }
            }
        }
    }
}

/// Where each part of a scaled terminal is drawn before it's scaled: a layer
/// holds only what fits in the window, so a terminal bigger than the window
/// at 1× is drawn in window-sized tiles. Each is the tile's box in the
/// terminal (from its top-left) and where it's drawn, as near to `at` as fits.
pub fn stage_tiles(
    terminal: gpui::Size<Pixels>,
    at: Point<Pixels>,
    viewport: gpui::Size<Pixels>,
) -> Vec<(Bounds<Pixels>, Point<Pixels>)> {
    let (w, h) = (f32::from(terminal.width), f32::from(terminal.height));
    let (vw, vh) = (
        f32::from(viewport.width).floor().max(1.0),
        f32::from(viewport.height).floor().max(1.0),
    );
    let mut tiles = Vec::new();
    let mut y = 0.0;
    while y < h {
        let th = (h - y).min(vh);
        let mut x = 0.0;
        while x < w {
            let tw = (w - x).min(vw);
            let sx = (f32::from(at.x) + x).round().clamp(0.0, (vw - tw).max(0.0));
            let sy = (f32::from(at.y) + y).round().clamp(0.0, (vh - th).max(0.0));
            tiles.push((
                Bounds::new(point(px(x), px(y)), size(px(tw), px(th))),
                point(px(sx), px(sy)),
            ));
            x += vw;
        }
        y += vh;
    }
    tiles
}

/// The grid, its decorations, the cursor, the composition and the slider,
/// moved by `shift` (where a scaled terminal is drawn before it's scaled).
fn draw(p: &Prepaint, window: &mut Window, shift: Point<Pixels>) {
    let m = p.metrics;
    let dpr = m.dpr;
    let d = |v: f32| px(v / dpr);
    let (cw, ch) = (m.device_cell_w(), m.device_cell_h());
    let origin = p.geometry.origin + shift;
    let bounds = Bounds::new(p.geometry.bounds.origin + shift, p.geometry.bounds.size);
    let cell_origin = |row: usize, col: usize| point(origin.x + d(col as f32 * cw), origin.y + d(row as f32 * ch));
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        window.paint_quad(fill(bounds, rgb(TERM_BG)));
        let Some(s) = p.snapshot.clone() else {
            return;
        };
        let grid = Bounds::new(origin, size(d(s.cols as f32 * cw), d(s.rows as f32 * ch)));
        window.paint_quad(fill(grid, rgb(s.background)));

        // The cursor's cell, drawn as a block (TERM-076).
        let cursor = s.cursor.filter(|c| c.row < s.rows && c.col < s.cols);
        let block = cursor.filter(|c| c.focused && c.kind == CursorKind::Block && (p.cursor_on || !c.blink));

        // Backgrounds, merged along each row.
        for (row, cells) in s.cells.iter().enumerate() {
            let mut col = 0;
            while col < cells.len() {
                let cell = &cells[col];
                if cell.bg_is_default {
                    col += 1;
                    continue;
                }
                let start = col;
                let bg = cell.bg;
                while col < cells.len() && !cells[col].bg_is_default && cells[col].bg == bg {
                    col += 1;
                }
                let at = cell_origin(row, start);
                window.paint_quad(fill(
                    Bounds::new(at, size(d((col - start) as f32 * cw), d(ch))),
                    rgb(bg),
                ));
            }
        }
        if let Some(c) = block {
            let w = if c.wide { 2.0 } else { 1.0 };
            window.paint_quad(fill(
                Bounds::new(cell_origin(c.row, c.col), size(d(w * cw), d(ch))),
                rgb(c.color),
            ));
        }

        let glyph_color = |row: usize, col: usize| -> Hsla {
            let cell = &s.cells[row][col];
            if block.is_some_and(|c| c.row == row && c.col == col) {
                return rgb(palette::theme::CURSOR_ACCENT);
            }
            let mut color = rgb(cell.fg);
            if cell.dim {
                color.a *= palette::theme::DIM_OPACITY;
            }
            color
        };

        // Glyphs, each at its cell.
        let size_px = TermMetrics::font_size();
        for run in &p.runs {
            let baseline_y = origin.y + d(run.row as f32 * ch + m.baseline);
            for shaped in &run.layout.runs {
                for glyph in &shaped.glyphs {
                    let i = run.starts.partition_point(|(start, _)| *start <= glyph.index);
                    let Some(&(_, col)) = run.starts.get(i.saturating_sub(1)) else {
                        continue;
                    };
                    let at = point(origin.x + d(col as f32 * cw), baseline_y);
                    if glyph.is_emoji {
                        let _ = window.paint_emoji(at, shaped.font_id, glyph.id, size_px);
                    } else {
                        let color = glyph_color(run.row, col);
                        let _ = window.paint_glyph(at, shaped.font_id, glyph.id, size_px, color);
                    }
                }
            }
        }

        // Custom glyphs, decorations.
        for (row, cells) in s.cells.iter().enumerate() {
            for (col, cell) in cells.iter().enumerate() {
                if cell.width == 0 {
                    continue;
                }
                let at = cell_origin(row, col);
                let color = glyph_color(row, col);
                if !cell.invisible {
                    let mut chars = cell.text.chars();
                    if let (Some(c), None) = (chars.next(), chars.next())
                        && let Some(glyph) = glyphs::custom_glyph(c, cw, ch, dpr, size_px.into())
                    {
                        paint_custom(window, &glyph, at, cw, ch, dpr, color);
                    }
                }
                let span = f32::from(cell.width.max(1));
                if cell.underline != Underline::None {
                    let mut color = rgb(cell.underline_color);
                    if cell.dim {
                        color.a *= palette::theme::DIM_OPACITY;
                    }
                    paint_underline(window, cell.underline, at, span * cw, &m, color);
                }
                if cell.strikethrough {
                    // `floor(charHeight / 2)` less half a pixel, 2 device px at 2×.
                    let lw = (13.0 * dpr / 10.0).floor().max(1.0);
                    let y = m.cell.device_char_top as f32 + (m.cell.device_char_height as f32 / 2.0).floor() - 0.5;
                    window.paint_quad(fill(
                        Bounds::new(point(at.x, at.y + d(y - lw / 2.0)), size(d(span * cw), d(lw))),
                        color,
                    ));
                }
            }
        }

        // TERM-066: a hovered link is underlined in its cells' colour.
        if let Some(link) = &p.link {
            let top = p.history - p.display_offset;
            for row in 0..s.rows {
                for col in 0..s.cols {
                    let lp = nodus_terminal::links::LinkPoint {
                        x: col + 1,
                        y: top + row + 1,
                    };
                    if link.contains(lp, s.cols) {
                        let color = rgb(s.cells[row][col].fg);
                        let at = cell_origin(row, col);
                        let y = (m.cell.device_cell_height - 1.0) as f32;
                        window.paint_quad(fill(Bounds::new(point(at.x, at.y + d(y)), size(d(cw), d(1.0))), color));
                    }
                }
            }
        }

        // The cursor's other shapes, and the unfocused outline (TERM-076).
        if let Some(c) = cursor {
            let w = if c.wide { 2.0 * cw } else { cw };
            let at = cell_origin(c.row, c.col);
            let color = rgb(c.color);
            let rect = |window: &mut Window, x: f32, y: f32, rw: f32, rh: f32| {
                window.paint_quad(fill(
                    Bounds::new(point(at.x + d(x), at.y + d(y)), size(d(rw), d(rh))),
                    color,
                ));
            };
            if !c.focused {
                rect(window, 0.0, 0.0, dpr, ch);
                rect(window, 0.0, ch - dpr, w, dpr);
                rect(window, 0.0, 0.0, w, dpr);
                rect(window, w - dpr, 0.0, dpr, ch);
            } else if p.cursor_on || !c.blink {
                match c.kind {
                    CursorKind::Block => {}
                    CursorKind::Bar => rect(window, 0.0, 0.0, dpr, ch),
                    CursorKind::Underline => rect(window, 0.0, ch - dpr, w, dpr),
                }
            }
        }

        // TERM-055: the composition at the cursor, white on black.
        if let (Some((_, layout)), Some(c)) = (&p.marked, s.cursor.or(cursor)) {
            let at = cell_origin(c.row, c.col);
            let width = layout.width.max(d(cw));
            window.paint_quad(fill(
                Bounds::new(at, size(width, d(ch))),
                rgb(palette::theme::COMPOSITION_BACKGROUND),
            ));
            let baseline = point(at.x, at.y + d(m.baseline));
            for shaped in &layout.runs {
                for glyph in &shaped.glyphs {
                    let gp = point(baseline.x + glyph.position.x, baseline.y);
                    let _ = window.paint_glyph(
                        gp,
                        shaped.font_id,
                        glyph.id,
                        size_px,
                        rgb(palette::theme::COMPOSITION_FOREGROUND),
                    );
                }
            }
        }

        if let Some((slider, color)) = p.scrollbar {
            window.paint_quad(fill(Bounds::new(slider.origin + shift, slider.size), color));
        }
    });
}

/// The slider's opacity: shown while scrolling or hovered, faded out 500 ms
/// later over 800 ms (TERM-075).
pub fn scrollbar_opacity(s: &super::host::ScrollbarState, now: Instant) -> f32 {
    if s.hovered || s.grab.is_some() {
        return 1.0;
    }
    let Some(at) = s.active_at else { return 0.0 };
    let ms = now.duration_since(at).as_secs_f32() * 1000.0;
    if ms < 100.0 {
        ms / 100.0
    } else if ms < 500.0 {
        1.0
    } else {
        (1.0 - (ms - 500.0) / 800.0).max(0.0)
    }
}

/// A custom glyph's shapes in its cell.
fn paint_custom(
    window: &mut Window,
    glyph: &glyphs::CustomGlyph,
    at: Point<Pixels>,
    cw: f32,
    ch: f32,
    dpr: f32,
    color: Hsla,
) {
    let d = |v: f32| px(v / dpr);
    let cell = Bounds::new(at, size(d(cw), d(ch)));
    let draw = |window: &mut Window| {
        for prim in &glyph.prims {
            match prim {
                Prim::Rect { x, y, w, h, alpha } => {
                    let mut c = color;
                    c.a *= alpha;
                    window.paint_quad(fill(
                        Bounds::new(point(at.x + d(*x), at.y + d(*y)), size(d(*w), d(*h))),
                        c,
                    ));
                }
                Prim::Path { cmds, stroke } => {
                    let mut builder = match stroke {
                        Some(w) => PathBuilder::stroke(d(*w)),
                        None => PathBuilder::fill(),
                    };
                    let pt = |x: f32, y: f32| point(at.x + d(x), at.y + d(y));
                    for cmd in cmds {
                        match *cmd {
                            Cmd::Move(x, y) => builder.move_to(pt(x, y)),
                            Cmd::Line(x, y) => builder.line_to(pt(x, y)),
                            Cmd::Cubic(c1, c2, to) => {
                                builder.cubic_bezier_to(pt(to[0], to[1]), pt(c1[0], c1[1]), pt(c2[0], c2[1]));
                            }
                        }
                    }
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, color);
                    }
                }
            }
        }
    };
    if glyph.clip {
        window.with_content_mask(Some(ContentMask { bounds: cell }), draw);
    } else {
        draw(window);
    }
}

/// The underline styles as xterm's texture atlas draws them: 1 device px
/// (`max(1, floor(fontSize × dpr / 15))`) at the bottom of the character box.
fn paint_underline(window: &mut Window, style: Underline, at: Point<Pixels>, width: f32, m: &TermMetrics, color: Hsla) {
    let dpr = m.dpr;
    let d = |v: f32| px(v / dpr);
    let lw = (13.0 * dpr / 15.0).floor().max(1.0);
    let char_bottom = (m.cell.device_char_top + m.cell.device_char_height) as f32;
    // `yTop`: ceil(charHeight) less half a pixel for an odd width, the line's centre.
    let y_top = char_bottom - if lw as i32 % 2 == 1 { 0.5 } else { 0.0 };
    let line = |window: &mut Window, x: f32, y: f32, w: f32| {
        window.paint_quad(fill(
            Bounds::new(point(at.x + d(x), at.y + d(y - lw / 2.0)), size(d(w), d(lw))),
            color,
        ));
    };
    match style {
        Underline::None => {}
        Underline::Single => line(window, 0.0, y_top, width),
        Underline::Double => {
            line(window, 0.0, y_top, width);
            line(window, 0.0, y_top + 2.0 * lw, width);
        }
        Underline::Dotted => {
            let mut x = 0.0;
            while x < width {
                line(window, x, y_top, lw.min(width - x));
                x += 2.0 * lw;
            }
        }
        Underline::Dashed => {
            let cell = m.device_cell_w();
            let dash = (0.6 * cell).floor();
            let gap = (0.3 * cell).floor();
            let mut x = 0.0;
            while x < width {
                line(window, x, y_top, dash.min(width - x));
                x += dash + gap + (cell - dash - gap);
            }
        }
        Underline::Curly => {
            let cell = m.device_cell_w();
            let mid = y_top + lw;
            let (top, bot) = (y_top, y_top + 2.0 * lw);
            let mut builder = PathBuilder::stroke(d(lw));
            let pt = |x: f32, y: f32| point(at.x + d(x), at.y + d(y));
            let mut x = 0.0;
            builder.move_to(pt(0.0, mid));
            while x < width {
                let half = cell / 2.0;
                builder.cubic_bezier_to(pt(x + half, mid), pt(x, bot), pt(x + half, bot));
                builder.cubic_bezier_to(pt(x + cell, mid), pt(x + half, top), pt(x + cell, top));
                x += cell;
            }
            if let Ok(path) = builder.build() {
                let clip = Bounds::new(at, size(d(width), d(m.device_cell_h())));
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    window.paint_path(path, color)
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::host::ScrollbarState;

    #[test]
    fn term_075_the_slider_fades() {
        let now = Instant::now();
        let mut s = ScrollbarState::default();
        assert_eq!(scrollbar_opacity(&s, now), 0.0);
        s.active_at = Some(now);
        assert_eq!(scrollbar_opacity(&s, now + std::time::Duration::from_millis(300)), 1.0);
        let fading = scrollbar_opacity(&s, now + std::time::Duration::from_millis(900));
        assert!((fading - 0.5).abs() < 1e-3);
        assert_eq!(scrollbar_opacity(&s, now + std::time::Duration::from_millis(1400)), 0.0);
        s.hovered = true;
        assert_eq!(scrollbar_opacity(&s, now + std::time::Duration::from_millis(5000)), 1.0);
    }
}
