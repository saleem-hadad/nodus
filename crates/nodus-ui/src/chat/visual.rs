//! A visual in the chat (spec 16, VIS): charts, stat tiles, tables and
//! Markdown, laid out top to bottom in a card, and side by side where the
//! spec says `columns`.
//!
//! Charts are laid out by `nodus_visual` at the width they're given and
//! painted here as quads, paths and shaped text; hovering one shows the
//! tooltip of the region under the pointer. Everything is sized in px
//! times `scale`, so an expanded card on a zoomed board draws it scaled.

use std::cell::Cell;
use std::rc::Rc;

use gpui::base::text::{TextView, TextViewStyle};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Corners, Element, ElementId, FontWeight, GlobalElementId, HighlightStyle,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseMoveEvent, ParentElement, PathBuilder,
    PathStyle, Pixels, Point, SharedString, Style, StyleRefinement, Styled, TextAlign, TextRun, Window, div, fill,
    point, px, relative, rems, size,
};
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};
use nodus_visual::spec::{Chart, Part, PartKind, Stat, Table, Tone, Visual};
use nodus_visual::{Anchor, Hover, Item, Scene, VisualTheme, Weight};

use crate::theme::tokens::{Tokens, rgba};
use crate::theme::{self, fonts};

/// A visual, `scale` times its natural size, in a card as wide as its
/// container.
pub fn visual_element(id: impl Into<ElementId>, visual: Rc<Visual>, scale: f32, cx: &App) -> AnyElement {
    let id: ElementId = id.into();
    let t = theme::tokens(cx);
    let s = scale;
    let title = visual.title.clone().filter(|t| !t.trim().is_empty());
    let root = Path {
        visual,
        indices: Vec::new(),
    };
    let parts = root
        .visual
        .rows
        .iter()
        .enumerate()
        .map(|(i, part)| part_element(&child(&id, "row", i), part, root.deeper(i), s, &t))
        .collect::<Vec<_>>();
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(16. * s))
        .p(px(16. * s))
        .rounded(px(14. * s))
        .bg(t.bg_elevated)
        .border_1()
        .border_color(t.border)
        .shadow(t.sm())
        .font_family(fonts::UI)
        .text_color(t.text_primary)
        .when_some(title, |d, title| {
            d.child(
                div()
                    .text_size(px(15. * s))
                    .line_height(px(20. * s))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
        })
        .children(parts)
        .into_any_element()
}

/// The theme a chart paints with, from the app's tokens: the palette for
/// the appearance, the text and lines in the app's own greys, and the card
/// behind it.
pub fn chart_theme(t: &Tokens) -> VisualTheme {
    let base = if t.dark {
        VisualTheme::dark()
    } else {
        VisualTheme::light()
    };
    let c = |c: gpui::Rgba| nodus_visual::Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    };
    VisualTheme {
        // Tertiary text is too faint for small numbers; secondary, a step down.
        axis_text: c(t.text_secondary).alpha(0.88),
        label_text: c(t.text_primary).alpha(0.92),
        grid: c(t.separator),
        baseline: c(t.border_strong),
        background: c(t.bg_elevated),
        hover: if t.dark {
            nodus_visual::Rgba::hex(0xffffff).alpha(0.16)
        } else {
            nodus_visual::Rgba::hex(0xffffff).alpha(0.28)
        },
        ..base
    }
}

/// An id for the `i`th `name` under `parent`.
fn child(parent: &ElementId, name: &str, i: usize) -> ElementId {
    ElementId::NamedChild(std::sync::Arc::new(parent.clone()), format!("{name}-{i}").into())
}

/// Where a part sits in its visual: its row, then its column at each level.
#[derive(Clone)]
struct Path {
    visual: Rc<Visual>,
    indices: Vec<usize>,
}

impl Path {
    fn part(&self) -> Option<&Part> {
        let (first, rest) = self.indices.split_first()?;
        let mut part = self.visual.rows.get(*first)?;
        for &i in rest {
            let PartKind::Columns(cols) = &part.kind else {
                return None;
            };
            part = cols.get(i)?;
        }
        Some(part)
    }

    fn chart(&self) -> Option<&Chart> {
        match &self.part()?.kind {
            PartKind::Chart(c) => Some(c),
            _ => None,
        }
    }

    fn deeper(&self, i: usize) -> Self {
        let mut indices = self.indices.clone();
        indices.push(i);
        Self {
            visual: self.visual.clone(),
            indices,
        }
    }
}

fn part_element(id: &ElementId, part: &Part, at: Path, s: f32, t: &Tokens) -> AnyElement {
    let body = match &part.kind {
        PartKind::Chart(_) => ChartElement {
            id: id.clone(),
            at,
            scale: s,
            theme: chart_theme(t),
        }
        .into_any_element(),
        PartKind::Stats(stats) => stats_element(stats, s, t),
        PartKind::Table(table) => table_element(table, s, t),
        PartKind::Text(md) => markdown(child(id, "text", 0), md, s, t),
        PartKind::Columns(cols) => div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap(px(16. * s))
            .children(cols.iter().enumerate().map(|(i, p)| {
                div()
                    .flex_1()
                    .flex_basis(px(0.))
                    // Narrower than this, columns wrap to rows.
                    .min_w(px(220. * s))
                    .child(part_element(&child(id, "col", i), p, at.deeper(i), s, t))
            }))
            .into_any_element(),
    };
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(8. * s))
        .when_some(part.title.clone().filter(|t| !t.trim().is_empty()), |d, title| {
            d.child(
                div()
                    .text_size(px(12. * s))
                    .line_height(px(16. * s))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_secondary)
                    .child(title),
            )
        })
        .child(body)
        .into_any_element()
}

/// Stat tiles in a row that wraps: the value large, what it is above it,
/// and its note below in the tone's colour.
fn stats_element(stats: &[Stat], s: f32, t: &Tokens) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .gap(px(8. * s))
        .children(stats.iter().map(|stat| {
            let tone = match stat.tone {
                Tone::Good => Some(t.diff_add),
                Tone::Bad => Some(t.diff_del),
                Tone::Neutral => None,
            };
            div()
                .flex_1()
                .flex_basis(px(0.))
                .min_w(px(116. * s))
                .flex()
                .flex_col()
                .gap(px(2. * s))
                .px(px(12. * s))
                .py(px(10. * s))
                .rounded(px(10. * s))
                .bg(t.bg_sunken)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6. * s))
                        .text_size(px(11.5 * s))
                        .line_height(px(15. * s))
                        .text_color(t.text_secondary)
                        // A tone with no note to carry it shows as a dot.
                        .when(stat.note.is_none(), |d| {
                            d.when_some(tone, |d, c| {
                                d.child(div().flex_none().size(px(6. * s)).rounded_full().bg(c))
                            })
                        })
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(stat.label.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(22. * s))
                        .line_height(px(28. * s))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text_primary)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(stat.value.clone()),
                )
                .when_some(stat.note.clone(), |d, note| {
                    d.child(
                        div()
                            .text_size(px(11.5 * s))
                            .line_height(px(15. * s))
                            .text_color(tone.unwrap_or(t.text_tertiary))
                            .child(note),
                    )
                })
        }))
        .into_any_element()
}

/// A table: a header row on a sunken band, hairlines between rows, numbers
/// right-aligned in tabular figures. Columns share the width by how much
/// they hold.
fn table_element(table: &Table, s: f32, t: &Tokens) -> AnyElement {
    let n = table.width();
    let numeric: Vec<bool> = (0..n).map(|i| table.numeric(i)).collect();
    let weight: Vec<f32> = (0..n)
        .map(|i| {
            let header = table.columns.get(i).map_or(0, |c| c.chars().count());
            let widest = table
                .rows
                .iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.text.chars().count())
                .max()
                .unwrap_or(0);
            (header.max(widest) as f32).clamp(4., 36.)
        })
        .collect();
    let cell = |i: usize, text: String, head: bool| {
        div()
            .flex_grow(weight[i])
            .flex_shrink(1.)
            .flex_basis(px(0.))
            .min_w(px(36. * s))
            .px(px(10. * s))
            .py(px(if head { 6. } else { 7. } * s))
            .when(numeric[i], |d| {
                d.flex()
                    .justify_end()
                    .whitespace_nowrap()
                    .font_features(fonts::tabular())
            })
            .child(text)
    };
    let header = (!table.columns.is_empty()).then(|| {
        div()
            .flex()
            .bg(t.bg_sunken)
            // The table's clip is square: round the band's own top corners.
            .rounded_t(px((10. * s - 1.).max(0.)))
            .border_b_1()
            .border_color(t.border)
            .text_size(px(11.5 * s))
            .line_height(px(15. * s))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.text_secondary)
            .children((0..n).map(|i| cell(i, table.columns.get(i).cloned().unwrap_or_default(), true)))
    });
    let last = table.rows.len().saturating_sub(1);
    div()
        .w_full()
        .rounded(px(10. * s))
        .border_1()
        .border_color(t.border)
        .overflow_hidden()
        .text_size(px(12.5 * s))
        .line_height(px(17. * s))
        .children(header)
        .children(table.rows.iter().enumerate().map(|(r, row)| {
            div()
                .flex()
                .when(r != last, |d| d.border_b_1().border_color(t.separator))
                .children((0..n).map(|i| cell(i, row.get(i).map(|c| c.text.clone()).unwrap_or_default(), false)))
        }))
        .into_any_element()
}

/// Markdown, in the chat's type, scaled.
fn markdown(id: ElementId, md: &str, s: f32, t: &Tokens) -> AnyElement {
    let style = TextViewStyle::default()
        .with_dark(t.dark)
        .with_foreground(t.text_primary.into())
        .with_muted_foreground(t.text_secondary.into())
        .with_link(t.accent.into())
        .with_border(t.border.into())
        .with_code_background(t.bg_sunken.into())
        // Inline code keeps a light wash on the light theme's: name it for both.
        .with_inline_code(HighlightStyle {
            background_color: Some(if t.dark { t.bg_pressed } else { t.bg_hover }.into()),
            ..HighlightStyle::default()
        })
        .with_selection(t.bg_selected.into())
        .with_paragraph_gap(rems(0.6 * s))
        .with_heading(move |level| {
            let size = match level {
                1 => 17.,
                2 => 15.,
                _ => 13.5,
            };
            StyleRefinement::default()
                .text_size(px(size * s))
                .font_weight(FontWeight::SEMIBOLD)
        });
    div()
        .w_full()
        .text_size(px(13. * s))
        .line_height(px(19. * s))
        .text_color(t.text_primary)
        .child(TextView::markdown(id, SharedString::from(md.to_owned())).style(style))
        .into_any_element()
}

// -- Charts ----------------------------------------------------------------------

/// A chart, laid out at the width its container gives it.
struct ChartElement {
    id: ElementId,
    at: Path,
    scale: f32,
    theme: VisualTheme,
}

impl IntoElement for ChartElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// What a chart's paint needs: its scene and where the pointer can hover.
pub struct ChartPaint {
    scene: Scene,
    hitbox: Hitbox,
}

/// The pointer over a chart, in window px, kept between frames.
type Pointer = Rc<Cell<Option<Point<Pixels>>>>;

/// Lays a chart out `width` px wide (scaled), measuring text as the
/// window's text system sets it.
fn lay_out(chart: &Chart, width: f32, scale: f32, theme: &VisualTheme, window: &Window) -> Scene {
    let ts = window.text_system().clone();
    let measure = move |text: &str, size: f32| -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let run = TextRun {
            len: text.len(),
            font: gpui::font(fonts::UI),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            ts.shape_line(SharedString::from(text.to_owned()), px(size), &[run], None)
                .width,
        )
    };
    chart.layout(width / scale, theme, &measure)
}

impl Element for ChartElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<ChartPaint>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.min_size.width = px(0.).into();
        let (at, scale, theme) = (self.at.clone(), self.scale, self.theme.clone());
        let id = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known
                .width
                .or(match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    _ => None,
                })
                .map_or(480. * scale, f32::from);
            let height = at
                .chart()
                .map_or(0., |c| lay_out(c, width, scale, &theme, window).height * scale);
            size(px(width), px(height))
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Option<ChartPaint> {
        let chart = self.at.chart()?;
        let scene = lay_out(chart, f32::from(bounds.size.width), self.scale, &self.theme, window);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Some(ChartPaint { scene, hitbox })
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Option<ChartPaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(ChartPaint { scene, hitbox }) = prepaint.take() else {
            return;
        };
        let s = self.scale;
        let origin = bounds.origin;
        paint_items(&scene.items, origin, s, window, cx);

        let Some(id) = id else { return };
        let pointer: Pointer = window.with_element_state(id, |state: Option<Pointer>, _| {
            let state = state.unwrap_or_default();
            (state.clone(), state)
        });
        // VIS-030: the region under the pointer lights up, and its tooltip
        // shows beside the pointer.
        if let Some(p) = pointer.get().filter(|_| hitbox.is_hovered(window)) {
            let (x, y) = (f32::from(p.x - origin.x) / s, f32::from(p.y - origin.y) / s);
            if let Some(hover) = scene.hover_at(x, y) {
                paint_items(&hover.highlight, origin, s, window, cx);
                let t = theme::tokens(cx);
                paint_tooltip(hover, p, bounds, s, &t, window, cx);
            }
        }
        let hovers: Vec<_> = scene.hovers.iter().map(|h| h.rect).collect();
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            let inside = hitbox.is_hovered(window);
            let before = pointer.get();
            let now = inside.then_some(e.position);
            if before == now {
                return;
            }
            pointer.set(now);
            // Redraw while over a region (the tooltip follows the pointer),
            // and once on the way out.
            let local = |p: Point<Pixels>| (f32::from(p.x - origin.x) / s, f32::from(p.y - origin.y) / s);
            let over = |p: Option<Point<Pixels>>| {
                p.is_some_and(|p| {
                    let (x, y) = local(p);
                    hovers.iter().any(|r| r.contains(x, y))
                })
            };
            if over(before) || over(now) {
                window.refresh();
            }
        });
    }
}

fn color(c: nodus_visual::Rgba) -> gpui::Rgba {
    gpui::Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

/// Paints scene items at `origin`, `s` times their size.
fn paint_items(items: &[Item], origin: Point<Pixels>, s: f32, window: &mut Window, cx: &mut App) {
    let at = |x: f32, y: f32| point(origin.x + px(x * s), origin.y + px(y * s));
    for item in items {
        match item {
            Item::Rect { rect, radius, color: c } => {
                let bounds = Bounds::new(at(rect.x, rect.y), size(px(rect.w * s), px(rect.h * s)));
                let r = |v: f32| px(v * s);
                window.paint_quad(fill(bounds, color(*c)).corner_radii(Corners {
                    top_left: r(radius[0]),
                    top_right: r(radius[1]),
                    bottom_right: r(radius[2]),
                    bottom_left: r(radius[3]),
                }));
            }
            Item::Line {
                points,
                width,
                color: c,
            } => {
                if points.len() < 2 {
                    continue;
                }
                let w = width * s;
                let options = StrokeOptions::default()
                    .with_line_width(w)
                    .with_line_cap(LineCap::Round)
                    .with_line_join(LineJoin::Round);
                let mut path = PathBuilder::stroke(px(w)).with_style(PathStyle::Stroke(options));
                path.move_to(at(points[0][0], points[0][1]));
                for p in &points[1..] {
                    path.line_to(at(p[0], p[1]));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color(*c));
                }
            }
            Item::Polygon { points, color: c } => {
                if points.len() < 3 {
                    continue;
                }
                let mut path = PathBuilder::fill();
                path.move_to(at(points[0][0], points[0][1]));
                for p in &points[1..] {
                    path.line_to(at(p[0], p[1]));
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, color(*c));
                }
            }
            Item::Circle {
                x,
                y,
                r,
                fill: inner,
                stroke,
            } => {
                let disc = |radius: f32| {
                    Bounds::new(
                        point(origin.x + px((x - radius) * s), origin.y + px((y - radius) * s)),
                        size(px(2. * radius * s), px(2. * radius * s)),
                    )
                };
                match (inner, stroke) {
                    (Some(f), Some((w, ring))) => {
                        window.paint_quad(fill(disc(r + w), color(*ring)).corner_radii(px((r + w) * s)));
                        window.paint_quad(fill(disc(*r), color(*f)).corner_radii(px(r * s)));
                    }
                    (Some(f), None) => window.paint_quad(fill(disc(*r), color(*f)).corner_radii(px(r * s))),
                    (None, Some((w, ring))) => {
                        let outer = r + w / 2.;
                        window.paint_quad(
                            fill(disc(outer), gpui::transparent_black())
                                .corner_radii(px(outer * s))
                                .border_widths(px(w * s))
                                .border_color(color(*ring)),
                        );
                    }
                    (None, None) => {}
                }
            }
            Item::Text(text) => {
                paint_text(
                    &text.text,
                    at(text.x, text.y),
                    text.size * s,
                    text.weight,
                    text.tabular,
                    text.anchor,
                    color(text.color),
                    window,
                    cx,
                );
            }
        }
    }
}

/// One line of text, its `anchor` at `at`, centred on `at.y`.
#[allow(clippy::too_many_arguments)]
fn paint_text(
    text: &str,
    at: Point<Pixels>,
    size_px: f32,
    weight: Weight,
    tabular: bool,
    anchor: Anchor,
    color: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let mut font = gpui::font(fonts::UI);
    font.weight = match weight {
        Weight::Regular => FontWeight::NORMAL,
        Weight::Medium => FontWeight::MEDIUM,
        Weight::Semibold => FontWeight::SEMIBOLD,
    };
    if tabular {
        font.features = fonts::tabular();
    }
    let run = TextRun {
        len: text.len(),
        font,
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(SharedString::from(text.to_owned()), px(size_px), &[run], None);
    let w = f32::from(line.width);
    let left = match anchor {
        Anchor::Start => f32::from(at.x),
        Anchor::Middle => f32::from(at.x) - w / 2.,
        Anchor::End => f32::from(at.x) - w,
    };
    let line_h = (size_px * 1.3).round();
    let _ = line.paint(
        point(px(left), at.y - px(line_h / 2.)),
        px(line_h),
        TextAlign::Left,
        None,
        window,
        cx,
    );
    w
}

/// VIS-030: the tooltip, below and right of the pointer, or wherever it
/// fits inside the chart: its title, then a line per value with the
/// series' colour as a short stroke.
fn paint_tooltip(
    hover: &Hover,
    pointer: Point<Pixels>,
    chart: Bounds<Pixels>,
    s: f32,
    t: &Tokens,
    window: &mut Window,
    cx: &mut App,
) {
    if hover.title.is_empty() && hover.lines.is_empty() {
        return;
    }
    let ts = window.text_system().clone();
    let width = |text: &str, size: f32, bold: bool| -> f32 {
        if text.is_empty() {
            return 0.;
        }
        let mut font = gpui::font(fonts::UI);
        if bold {
            font.weight = FontWeight::SEMIBOLD;
        }
        let run = TextRun {
            len: text.len(),
            font,
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            ts.shape_line(SharedString::from(text.to_owned()), px(size), &[run], None)
                .width,
        )
    };
    let (pad, line_h, key) = (9. * s, 17. * s, 10. * s);
    let keyed = hover.lines.iter().any(|l| l.color.is_some());
    let label_w = hover
        .lines
        .iter()
        .map(|l| width(&l.label, 11.5 * s, false))
        .fold(0., f32::max);
    let value_w = hover
        .lines
        .iter()
        .map(|l| width(&l.value, 11.5 * s, true))
        .fold(0., f32::max);
    let row_w = if keyed { key + 6. * s } else { 0. } + label_w + 14. * s + value_w;
    let title_w = width(&hover.title, 12. * s, true);
    let w = row_w.max(title_w) + 2. * pad;
    let title_h = if hover.title.is_empty() { 0. } else { line_h + 2. * s };
    let h = title_h + hover.lines.len() as f32 * line_h + 2. * pad - 2. * s;

    // Below right of the pointer; flipped to the left, or above, to stay
    // inside the chart.
    let (cl, ct) = (f32::from(chart.origin.x), f32::from(chart.origin.y));
    let (cr, cb) = (cl + f32::from(chart.size.width), ct + f32::from(chart.size.height));
    let (px_, py_) = (f32::from(pointer.x), f32::from(pointer.y));
    let gap = 14. * s;
    let mut x = px_ + gap;
    if x + w > cr {
        x = px_ - gap - w;
    }
    let mut y = py_ + gap;
    if y + h > cb {
        y = py_ - gap - h;
    }
    let x = x.clamp(cl, (cr - w).max(cl));
    let y = y.clamp(ct, (cb - h).max(ct));

    let radius = px(8. * s);
    let bounds = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    // A soft edge under the box, then the box.
    let shade = Bounds::new(point(px(x), px(y + 2. * s)), size(px(w), px(h)));
    window.paint_quad(fill(shade, rgba(0, 0, 0, if t.dark { 0.35 } else { 0.08 })).corner_radii(radius));
    let surface = if t.dark {
        theme::tokens::hex(0x38383c)
    } else {
        t.bg_elevated
    };
    window.paint_quad(
        fill(bounds, surface)
            .corner_radii(radius)
            .border_widths(px(s.max(0.5)))
            .border_color(t.border_strong),
    );
    let mut row_y = y + pad + line_h / 2. - 1. * s;
    if !hover.title.is_empty() {
        paint_text(
            &hover.title,
            point(px(x + pad), px(row_y)),
            12. * s,
            Weight::Semibold,
            false,
            Anchor::Start,
            t.text_primary,
            window,
            cx,
        );
        row_y += title_h;
    }
    for line in &hover.lines {
        let mut lx = x + pad;
        if keyed {
            if let Some(c) = line.color {
                let stroke = Bounds::new(point(px(lx), px(row_y - 1. * s)), size(px(key), px(2. * s)));
                window.paint_quad(fill(stroke, color(c)).corner_radii(px(1. * s)));
            }
            lx += key + 6. * s;
        }
        paint_text(
            &line.label,
            point(px(lx), px(row_y)),
            11.5 * s,
            Weight::Regular,
            false,
            Anchor::Start,
            t.text_secondary,
            window,
            cx,
        );
        paint_text(
            &line.value,
            point(px(x + w - pad), px(row_y)),
            11.5 * s,
            Weight::Semibold,
            true,
            Anchor::End,
            t.text_primary,
            window,
            cx,
        );
        row_y += line_h;
    }
}
