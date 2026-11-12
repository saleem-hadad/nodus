//! Text laid out as Chromium lays out the board's paragraphs: wrapped at the
//! line-break opportunities of UAX #14, a word too long for its line broken
//! anywhere (`overflow-wrap: anywhere`), spaces at the end of a line hanging
//! past it, `letter-spacing` after every character, whitespace collapsed
//! (`white-space: normal`) or kept (`pre-wrap`), clamped to a number of lines
//! with an ellipsis (`-webkit-line-clamp`), or one line cut with `…`
//! (`text-overflow: ellipsis`).
//!
//! GPUI's own text wraps like a code editor (indents, no hanging spaces) and
//! has no letter spacing, so card titles and previews broke in different
//! places; this element shapes the whole text once and places every glyph
//! itself.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, Font, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, ShapedLine, SharedString, Size, Style, TextRun, UnderlineStyle,
    Window, point, px, relative,
};
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

/// How whitespace is treated (`white-space`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Space {
    /// `normal`: runs of whitespace are one space, newlines included.
    Normal,
    /// `pre-wrap`: spaces and newlines kept, lines still wrap.
    PreWrap,
    /// `nowrap`: one line, whitespace collapsed.
    NoWrap,
}

/// A line of the laid-out text: its characters `start..end` (spaces that
/// hang past its end excluded), and whether it ends in `…`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    pub start: usize,
    pub end: usize,
    pub ellipsis: bool,
}

/// Lays out characters whose advances (letter spacing included) are
/// `advances` into lines no wider than `width`.
///
/// `breaks` are the characters a line may start at (from UAX #14), with
/// whether the break is forced; `graphemes` the grapheme boundaries, where a
/// word too long for a line is cut. `hang` tells which characters hang at the
/// end of a line. With `clamp`, the last line kept ends in an ellipsis
/// `ellipsis` wide, cutting characters until it fits.
#[allow(clippy::too_many_arguments)]
pub fn break_lines(
    advances: &[f32],
    breaks: &[(usize, bool)],
    graphemes: &[usize],
    hang: &dyn Fn(usize) -> bool,
    width: f32,
    wrap: bool,
    clamp: Option<usize>,
    ellipsis: f32,
) -> Vec<LineSpan> {
    const EPS: f32 = 1.0 / 64.0;
    let n = advances.len();
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f32);
    for a in advances {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + a);
    }
    let w = |a: usize, b: usize| prefix[b] - prefix[a];
    let trim = |a: usize, mut b: usize| {
        while b > a && hang(b - 1) {
            b -= 1;
        }
        b
    };
    let mut lines = Vec::new();
    let full = |lines: &Vec<LineSpan>| clamp.is_some_and(|c| lines.len() > c);

    if !wrap {
        let mut start = 0;
        for &(b, forced) in breaks {
            if forced || b == n {
                lines.push(LineSpan {
                    start,
                    end: trim(start, b),
                    ellipsis: false,
                });
                start = b;
            }
        }
        if lines.is_empty() {
            lines.push(LineSpan {
                start: 0,
                end: 0,
                ellipsis: false,
            });
        }
    } else {
        let mut start = 0;
        let mut fit: Option<usize> = None;
        let mut i = 0;
        while i < breaks.len() && !full(&lines) {
            let (b, forced) = breaks[i];
            if b <= start {
                i += 1;
                continue;
            }
            let e = trim(start, b);
            if w(start, e) <= width + EPS {
                if forced {
                    lines.push(LineSpan {
                        start,
                        end: e,
                        ellipsis: false,
                    });
                    start = b;
                    fit = None;
                } else {
                    fit = Some(b);
                }
                i += 1;
                continue;
            }
            if let Some(f) = fit.take() {
                lines.push(LineSpan {
                    start,
                    end: trim(start, f),
                    ellipsis: false,
                });
                start = f;
                continue;
            }
            // A word wider than a whole line: cut it at the last grapheme
            // that fits, at least one grapheme in.
            let mut cut = graphemes
                .iter()
                .copied()
                .filter(|&g| g > start && g <= e)
                .take_while(|&g| w(start, g) <= width + EPS)
                .last()
                .unwrap_or_else(|| graphemes.iter().copied().find(|&g| g > start).unwrap_or(e));
            if cut <= start {
                cut = e.max(start + 1);
            }
            lines.push(LineSpan {
                start,
                end: cut,
                ellipsis: false,
            });
            start = cut;
        }
        if lines.is_empty() {
            lines.push(LineSpan {
                start: 0,
                end: trim(0, n),
                ellipsis: false,
            });
        }
    }

    let cut_to_fit = |line: &mut LineSpan| {
        let s = line.start;
        let mut e = trim(s, line.end);
        while e > s && w(s, e) + ellipsis > width + EPS {
            e = graphemes.iter().copied().rfind(|&g| g >= s && g < e).unwrap_or(s);
            e = trim(s, e);
        }
        line.end = e;
        line.ellipsis = true;
    };
    if let Some(c) = clamp
        && lines.len() > c
    {
        lines.truncate(c.max(1));
        if let Some(last) = lines.last_mut() {
            cut_to_fit(last);
        }
    } else if !wrap && clamp == Some(1) {
        // `text-overflow: ellipsis` on one line: cut only when it overflows.
        if let Some(line) = lines.first_mut()
            && w(line.start, trim(line.start, line.end)) > width + EPS
        {
            cut_to_fit(line);
        }
    }
    lines
}

/// The text a paragraph shows, after whitespace processing: its characters,
/// and the byte each came from in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub text: String,
    pub chars: Vec<char>,
    pub source: Vec<usize>,
}

fn is_css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// Applies `white-space`: collapses whitespace for `normal`/`nowrap` (and
/// drops it at the start), keeps it for `pre-wrap`.
pub fn prepare(text: &str, space: Space) -> Prepared {
    let mut out = Prepared {
        text: String::with_capacity(text.len()),
        chars: Vec::new(),
        source: Vec::new(),
    };
    let mut pending_space: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match space {
            Space::PreWrap => {
                // CR LF is one line break.
                if c == '\r' && text[i + 1..].starts_with('\n') {
                    continue;
                }
                let c = if c == '\r' { '\n' } else { c };
                out.text.push(c);
                out.chars.push(c);
                out.source.push(i);
            }
            Space::Normal | Space::NoWrap => {
                if is_css_space(c) {
                    if pending_space.is_none() && !out.chars.is_empty() {
                        pending_space = Some(i);
                    }
                    continue;
                }
                if let Some(at) = pending_space.take() {
                    out.text.push(' ');
                    out.chars.push(' ');
                    out.source.push(at);
                }
                out.text.push(c);
                out.chars.push(c);
                out.source.push(i);
            }
        }
    }
    if let Some(at) = pending_space {
        out.text.push(' ');
        out.chars.push(' ');
        out.source.push(at);
    }
    out
}

/// Where lines may start (character indices), with whether they must.
pub fn break_opportunities(prepared: &Prepared) -> Vec<(usize, bool)> {
    let mut byte_to_char = vec![0usize; prepared.text.len() + 1];
    let mut ci = 0;
    for (bi, _) in prepared.text.char_indices() {
        byte_to_char[bi] = ci;
        ci += 1;
    }
    byte_to_char[prepared.text.len()] = ci;
    let chars = &prepared.chars;
    let mut out: Vec<(usize, bool)> = linebreaks(&prepared.text)
        .map(|(b, op)| (byte_to_char[b], op == BreakOpportunity::Mandatory))
        // Chromium keeps `src/cart/store.ts` and `/v2/legacy` whole: no
        // break after a slash that a word follows (UAX #14 allows one).
        .filter(|&(k, forced)| {
            forced || k == 0 || k >= chars.len() || !(chars[k - 1] == '/' && !is_css_space(chars[k]))
        })
        .collect();
    // Chromium breaks after any space, even before a `/` (UAX #14 says no).
    for k in 1..chars.len() {
        if chars[k - 1] == ' ' && !is_css_space(chars[k]) && !out.iter().any(|&(b, _)| b == k) {
            out.push((k, false));
        }
    }
    out.sort_by_key(|&(b, _)| b);
    if out.last().is_none_or(|&(b, _)| b != ci) {
        out.push((ci, true));
    }
    out
}

/// Grapheme boundaries (character indices), the end included.
pub fn grapheme_bounds(prepared: &Prepared) -> Vec<usize> {
    let mut out = Vec::new();
    let mut ci = 0;
    for g in prepared.text.graphemes(true) {
        ci += g.chars().count();
        out.push(ci);
    }
    out
}

/// A paragraph of text (see the module docs).
pub struct Para {
    cfg: Rc<Config>,
}

struct Config {
    text: SharedString,
    font: Font,
    size: Pixels,
    line_height: Pixels,
    spacing: Pixels,
    color: Hsla,
    marks: Vec<(Range<usize>, Hsla)>,
    clamp: Option<usize>,
    space: Space,
    center: bool,
    grow: bool,
    fill: bool,
    max_width: Option<Pixels>,
    underline: Option<Hsla>,
    max_lines_height: bool,
    /// The board's zoom: the text is laid out at `size / zoom` (its CSS
    /// size) and drawn scaled, as Chromium draws a scaled DOM.
    zoom: f32,
}

/// One line of text at the board's scale: `size` and `line_height` in CSS
/// px, laid out at that size and drawn `z` times as big (so it measures as
/// Chromium measures it), as wide as its text.
pub fn label(
    text: impl Into<SharedString>,
    font: Font,
    size: f32,
    line_height: f32,
    color: impl Into<Hsla>,
    z: f32,
) -> Para {
    para(text, font, px(size * z), px(line_height * z), color)
        .zoom(z)
        .space(Space::NoWrap)
        .hug()
}

/// A paragraph of `text` in `font` at `size`, lines `line_height` apart.
pub fn para(
    text: impl Into<SharedString>,
    font: Font,
    size: Pixels,
    line_height: Pixels,
    color: impl Into<Hsla>,
) -> Para {
    Para {
        cfg: Rc::new(Config {
            text: text.into(),
            font,
            size,
            line_height,
            spacing: px(0.),
            color: color.into(),
            marks: Vec::new(),
            clamp: None,
            space: Space::Normal,
            center: false,
            grow: false,
            fill: true,
            max_width: None,
            underline: None,
            max_lines_height: false,
            zoom: 1.0,
        }),
    }
}

/// A font of `family` at `weight`, italic or not.
pub fn font(family: &str, weight: FontWeight, italic: bool) -> Font {
    let mut f = gpui::font(SharedString::from(family.to_owned()));
    f.weight = weight;
    if italic {
        f.style = FontStyle::Italic;
    }
    f
}

impl Para {
    fn cfg(&mut self) -> &mut Config {
        Rc::get_mut(&mut self.cfg).expect("configured before layout")
    }
    /// `letter-spacing`.
    pub fn spacing(mut self, spacing: Pixels) -> Self {
        self.cfg().spacing = spacing;
        self
    }
    /// Colours byte ranges of the text differently.
    pub fn marks(mut self, marks: Vec<(Range<usize>, Hsla)>) -> Self {
        self.cfg().marks = marks;
        self
    }
    /// At most `lines` lines, the last ending in `…` when there's more.
    pub fn clamp(mut self, lines: usize) -> Self {
        self.cfg().clamp = Some(lines);
        self
    }
    pub fn space(mut self, space: Space) -> Self {
        self.cfg().space = space;
        self
    }
    /// One line cut with `…` when it overflows (`nowrap` + `text-overflow: ellipsis`).
    pub fn ellipsis(mut self) -> Self {
        let c = self.cfg();
        c.space = Space::NoWrap;
        c.clamp = Some(1);
        self
    }
    /// `text-align: center`.
    pub fn center(mut self) -> Self {
        self.cfg().center = true;
        self
    }
    /// `flex: 1; min-width: 0` in a row.
    pub fn grow(mut self) -> Self {
        self.cfg().grow = true;
        self
    }
    /// As wide as its text (an inline box), rather than its container.
    pub fn hug(mut self) -> Self {
        self.cfg().fill = false;
        self
    }
    pub fn max_width(mut self, width: Pixels) -> Self {
        self.cfg().max_width = Some(width);
        self
    }
    pub fn underline(mut self, color: Option<Hsla>) -> Self {
        self.cfg().underline = color;
        self
    }
    /// The sizes given are scaled by `zoom`: lay out at the CSS size.
    pub fn zoom(mut self, zoom: f32) -> Self {
        self.cfg().zoom = zoom.max(0.01);
        self
    }
}

/// What a paragraph's layout depends on: its text and how it's set, and the
/// width it wraps in (CSS px). Colours, marks and alignment only paint.
#[derive(Clone, PartialEq, Eq, Hash)]
struct LayoutKey {
    text: SharedString,
    font: Font,
    size: u32,
    spacing: u32,
    space: Space,
    clamp: Option<usize>,
    width: u32,
}

/// Laid-out paragraphs, for the frames that draw them again unchanged
/// (a board redraws every card's title and every note each frame it draws):
/// shaping, line breaking and grapheme bounds ran three times per paragraph
/// per frame. Two generations, so what's still drawn survives a turnover.
#[derive(Default)]
struct LayoutCache {
    current: HashMap<LayoutKey, Rc<Layout>>,
    previous: HashMap<LayoutKey, Rc<Layout>>,
}

const LAYOUT_CACHE_GENERATION: usize = 2048;

thread_local! {
    static LAYOUTS: RefCell<LayoutCache> = RefCell::default();
}

impl Config {
    /// The layout at `width` (CSS px), from the cache when this paragraph
    /// was laid out at that width before.
    fn layout(&self, width: f32, window: &mut Window) -> Rc<Layout> {
        let key = LayoutKey {
            text: self.text.clone(),
            font: self.font.clone(),
            size: f32::from(self.css_size()).to_bits(),
            spacing: (f32::from(self.spacing) / self.zoom).to_bits(),
            space: self.space,
            clamp: self.clamp,
            width: width.to_bits(),
        };
        let hit = LAYOUTS.with(|c| {
            let mut c = c.borrow_mut();
            if let Some(l) = c.current.get(&key) {
                return Some(l.clone());
            }
            let l = c.previous.remove(&key)?;
            c.current.insert(key.clone(), l.clone());
            Some(l)
        });
        if let Some(l) = hit {
            return l;
        }
        let l = Rc::new(self.lay_out(width, window));
        LAYOUTS.with(|c| {
            let mut c = c.borrow_mut();
            if c.current.len() >= LAYOUT_CACHE_GENERATION {
                c.previous = std::mem::take(&mut c.current);
            }
            c.current.insert(key, l.clone());
        });
        l
    }

    fn prepared(&self) -> Prepared {
        prepare(&self.text, self.space)
    }

    /// The CSS font size.
    fn css_size(&self) -> Pixels {
        self.size / self.zoom
    }

    fn shape(&self, text: &str, window: &mut Window) -> ShapedLine {
        // Shaping can't take a newline; a forced break never shows one.
        let shown: String = text.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
        let run = TextRun {
            len: shown.len(),
            font: self.font.clone(),
            color: self.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(SharedString::from(shown), self.css_size(), &[run], None)
    }

    /// Each character's x in the shaped line (CSS px), less `tracking` per
    /// character, without letter spacing, and the end.
    fn xs(prepared: &Prepared, line: &ShapedLine, tracking: f32) -> Vec<f32> {
        let mut glyphs: Vec<(usize, f32)> = line
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| (g.index, f32::from(g.position.x))))
            .collect();
        glyphs.sort_by_key(|g| g.0);
        let mut xs = Vec::with_capacity(prepared.chars.len() + 1);
        let mut byte = 0;
        let mut gi = 0;
        for c in &prepared.chars {
            while gi < glyphs.len() && glyphs[gi].0 < byte {
                gi += 1;
            }
            xs.push(glyphs.get(gi).map_or(f32::from(line.width), |g| g.1));
            byte += c.len_utf8();
        }
        xs.push(f32::from(line.width));
        for (k, x) in xs.iter_mut().enumerate() {
            *x -= tracking * k as f32;
        }
        // A character with no glyph of its own (inside a cluster) sits where
        // the next one starts.
        for k in (0..xs.len().saturating_sub(1)).rev() {
            if xs[k] > xs[k + 1] {
                xs[k] = xs[k + 1];
            }
        }
        xs
    }

    fn lay_out(&self, width: f32, window: &mut Window) -> Layout {
        let prepared = self.prepared();
        let shaped = self.shape(&prepared.text, window);
        // Shaped at the CSS size, CoreText's advances are Chromium's.
        let tracking = 0.0;
        let xs = Self::xs(&prepared, &shaped, tracking);
        let spacing = f32::from(self.spacing) / self.zoom;
        let advances: Vec<f32> = (0..prepared.chars.len())
            .map(|k| {
                if prepared.chars[k] == '\n' {
                    0.0
                } else {
                    xs[k + 1] - xs[k] + spacing
                }
            })
            .collect();
        let breaks = break_opportunities(&prepared);
        let graphemes = grapheme_bounds(&prepared);
        let ellipsis_line = self.shape("…", window);
        let ellipsis_w = f32::from(ellipsis_line.width) - tracking + spacing;
        let chars = prepared.chars.clone();
        let space = self.space;
        let hang = move |k: usize| match chars[k] {
            ' ' | '\t' => true,
            '\n' => space == Space::PreWrap,
            _ => false,
        };
        let lines = break_lines(
            &advances,
            &breaks,
            &graphemes,
            &hang,
            width,
            self.space != Space::NoWrap,
            self.clamp,
            ellipsis_w,
        );
        Layout {
            prepared,
            shaped,
            xs,
            advances,
            lines,
            ellipsis: ellipsis_line,
            ellipsis_w,
            tracking,
        }
    }

    fn natural_width(&self, window: &mut Window) -> f32 {
        self.layout(f32::INFINITY, window).widest()
    }
}

pub struct Layout {
    prepared: Prepared,
    shaped: ShapedLine,
    xs: Vec<f32>,
    advances: Vec<f32>,
    lines: Vec<LineSpan>,
    ellipsis: ShapedLine,
    ellipsis_w: f32,
    tracking: f32,
}

impl Layout {
    fn line_width(&self, line: &LineSpan) -> f32 {
        let w: f32 = self.advances[line.start..line.end].iter().sum();
        w + if line.ellipsis { self.ellipsis_w } else { 0.0 }
    }

    fn widest(&self) -> f32 {
        self.lines.iter().map(|l| self.line_width(l)).fold(0.0, f32::max)
    }
}

impl IntoElement for Para {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Para {
    type RequestLayoutState = ();
    type PrepaintState = Option<Rc<Layout>>;

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
        if self.cfg.grow {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = px(0.).into();
            style.min_size.width = px(0.).into();
        } else if self.cfg.fill {
            style.size.width = relative(1.).into();
            style.min_size.width = px(0.).into();
        } else {
            style.flex_shrink = 1.0;
            style.min_size.width = px(0.).into();
        }
        if let Some(max) = self.cfg.max_width {
            style.max_size.width = max.into();
        }
        let cfg = self.cfg.clone();
        let _ = cfg.max_lines_height;
        let id = window.request_measured_layout(style, move |known, available, window, _cx| {
            let z = cfg.zoom;
            let width = known.width.map(f32::from).or(match available.width {
                AvailableSpace::Definite(w) => Some(f32::from(w)),
                AvailableSpace::MinContent => Some(0.0),
                AvailableSpace::MaxContent => None,
            });
            let max = cfg.max_width.map_or(f32::INFINITY, f32::from);
            let natural = (cfg.natural_width(window) * z).min(max);
            let width = width.map_or(natural, |w| w.min(max));
            let layout = cfg.layout(width / z, window);
            let lines = layout.lines.len().max(1) as f32;
            let w = if cfg.fill || cfg.grow {
                width
            } else {
                (layout.widest() * z).min(width).ceil()
            };
            // GPUI rounds a measured size up to whole device pixels, where
            // Chromium keeps fractions; rounding to the nearest one here
            // keeps a column of lines (a fan row, a card) from drifting down.
            let scale = window.scale_factor();
            Size {
                width: px(w),
                height: px((f32::from(cfg.line_height) * lines * scale).round() / scale),
            }
        });
        let _ = cx;
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
    ) -> Option<Rc<Layout>> {
        Some(self.cfg.layout(f32::from(bounds.size.width) / self.cfg.zoom, window))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        layout: &mut Option<Rc<Layout>>,
        window: &mut Window,
        _: &mut App,
    ) {
        let Some(layout) = layout.take() else {
            return;
        };
        let cfg = &self.cfg;
        let z = cfg.zoom;
        // Laid out in CSS px; drawn `z` times as big.
        let spacing = f32::from(cfg.spacing) / z;
        let shaped = &layout.shaped;
        let lh = cfg.line_height;
        let ascent = shaped.ascent * z;
        let half_leading = (lh - ascent - shaped.descent * z) / 2.;
        // The colour of each character, from the marks on its source bytes.
        let colors: Vec<Hsla> = layout
            .prepared
            .source
            .iter()
            .map(|&b| {
                cfg.marks
                    .iter()
                    .rev()
                    .find(|(r, _)| r.contains(&b))
                    .map_or(cfg.color, |(_, c)| *c)
            })
            .collect();
        let mut char_of_byte = Vec::with_capacity(layout.prepared.text.len() + 1);
        for (k, c) in layout.prepared.chars.iter().enumerate() {
            for _ in 0..c.len_utf8() {
                char_of_byte.push(k);
            }
        }
        char_of_byte.push(layout.prepared.chars.len());
        for (li, line) in layout.lines.iter().enumerate() {
            let top = bounds.origin.y + lh * li as f32;
            let baseline = top + half_leading + ascent;
            let mut x0 = f32::from(bounds.origin.x);
            if cfg.center {
                x0 += (f32::from(bounds.size.width) - layout.line_width(line) * z) / 2.0;
            }
            let base_x = layout.xs[line.start];
            for run in &shaped.runs {
                for glyph in &run.glyphs {
                    let k = char_of_byte.get(glyph.index).copied().unwrap_or(usize::MAX);
                    if k < line.start || k >= line.end || layout.prepared.chars[k] == '\n' {
                        continue;
                    }
                    let gx = f32::from(glyph.position.x) - layout.tracking * k as f32;
                    let x = x0 + (gx - base_x + spacing * (k - line.start) as f32) * z;
                    let origin = point(px(x), baseline + glyph.position.y * z);
                    if glyph.is_emoji {
                        let _ = window.paint_emoji(origin, run.font_id, glyph.id, cfg.size);
                    } else {
                        let _ = window.paint_glyph(origin, run.font_id, glyph.id, cfg.size, colors[k]);
                    }
                }
            }
            let text_w: f32 = layout.advances[line.start..line.end].iter().sum::<f32>() * z;
            if line.ellipsis {
                let color = if line.end > line.start {
                    colors[line.end - 1]
                } else {
                    cfg.color
                };
                for run in &layout.ellipsis.runs {
                    for glyph in &run.glyphs {
                        let origin = point(
                            px(x0 + text_w + f32::from(glyph.position.x) * z),
                            baseline + glyph.position.y * z,
                        );
                        let _ = window.paint_glyph(origin, run.font_id, glyph.id, cfg.size, color);
                    }
                }
            }
            if let Some(color) = cfg.underline {
                let width = text_w + if line.ellipsis { layout.ellipsis_w * z } else { 0.0 };
                window.paint_underline(
                    point(px(x0), baseline + px(2.)),
                    px(width),
                    &UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: false,
                    },
                );
            }
        }
    }
}

/// Byte ranges of `text` for the UTF-16 indices a fuzzy match marked, merged
/// into runs (CARD-018: each maximal run of matched characters).
pub fn highlight_runs(text: &str, indices: &[usize]) -> Vec<Range<usize>> {
    let hit: std::collections::HashSet<usize> = indices.iter().copied().collect();
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut unit = 0usize;
    for (b, c) in text.char_indices() {
        let on = hit.contains(&unit);
        unit += c.len_utf16();
        if !on {
            continue;
        }
        let end = b + c.len_utf8();
        match runs.last_mut() {
            Some(r) if r.end == b => r.end = end,
            _ => runs.push(b..end),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(text: &str, width: f32, space: Space, clamp: Option<usize>) -> Vec<String> {
        let p = prepare(text, space);
        let advances = vec![1.0; p.chars.len()];
        let breaks = break_opportunities(&p);
        let graphemes = grapheme_bounds(&p);
        let chars = p.chars.clone();
        let hang = move |k: usize| chars[k] == ' ' || (space == Space::PreWrap && chars[k] == '\n');
        break_lines(
            &advances,
            &breaks,
            &graphemes,
            &hang,
            width,
            space != Space::NoWrap,
            clamp,
            1.0,
        )
        .iter()
        .map(|l| {
            let s: String = p.chars[l.start..l.end].iter().collect();
            if l.ellipsis { format!("{s}…") } else { s }
        })
        .collect()
    }

    #[test]
    fn collapses_whitespace_unless_pre_wrap() {
        assert_eq!(prepare("  a \n\t b  ", Space::Normal).text, "a b ");
        assert_eq!(prepare("a\r\nb", Space::PreWrap).text, "a\nb");
        let p = prepare(" x  y", Space::Normal);
        assert_eq!(p.source, [1, 2, 4]);
    }

    #[test]
    fn wraps_at_spaces_and_lets_them_hang() {
        assert_eq!(lay("aaa bbb ccc", 7.0, Space::Normal, None), ["aaa bbb", "ccc"]);
        // The space after "bbb" hangs: it doesn't count against the width.
        assert_eq!(lay("aaa bbb ccc", 3.0, Space::Normal, None), ["aaa", "bbb", "ccc"]);
    }

    #[test]
    fn breaks_a_word_too_long_anywhere() {
        assert_eq!(lay("abcdefgh ij", 3.0, Space::Normal, None), ["abc", "def", "gh", "ij"]);
    }

    #[test]
    fn breaks_paths_as_chromium_does() {
        // Before a slash after a space, never after a slash inside a path.
        assert_eq!(lay("calls /v2/x", 6.0, Space::Normal, None), ["calls", "/v2/x"]);
        // A path too long for a line breaks anywhere, not at its slashes.
        assert_eq!(
            lay("in src/cart/store", 9.0, Space::Normal, None),
            ["in", "src/cart/", "store"]
        );
    }

    #[test]
    fn clamps_with_an_ellipsis_cutting_characters_to_fit() {
        // "yyyy" fits a line, "yyyy…" doesn't: "yyy…".
        assert_eq!(lay("aaa yyyy b", 4.0, Space::Normal, Some(2)), ["aaa", "yyy…"]);
        assert_eq!(
            lay("aaa bb cc dd ee", 6.0, Space::Normal, Some(2)),
            ["aaa bb", "cc dd…"]
        );
        // Fits without being cut: no ellipsis.
        assert_eq!(lay("aaa bb", 6.0, Space::Normal, Some(2)), ["aaa bb"]);
        // The ellipsis follows the last word, not the space after it.
        assert_eq!(lay("aaaa b cc", 6.0, Space::Normal, Some(1)), ["aaaa…"]);
    }

    #[test]
    fn keeps_newlines_in_pre_wrap() {
        assert_eq!(lay("ab\ncd ef", 10.0, Space::PreWrap, None), ["ab", "cd ef"]);
        assert_eq!(lay("ab\n\ncd", 10.0, Space::PreWrap, None), ["ab", "", "cd"]);
    }

    #[test]
    fn cuts_one_line_only_when_it_overflows() {
        assert_eq!(lay("abcdef", 6.0, Space::NoWrap, Some(1)), ["abcdef"]);
        assert_eq!(lay("abcdefg", 6.0, Space::NoWrap, Some(1)), ["abcde…"]);
    }

    #[test]
    fn highlights_runs_of_matched_characters() {
        // CARD-018
        assert_eq!(highlight_runs("Fix rounding", &[0, 1, 2]), vec![0..3]);
        assert_eq!(highlight_runs("abcde", &[0, 2, 3]), [0..1, 2..4]);
        assert!(highlight_runs("abc", &[]).is_empty());
        // UTF-16 indices: the emoji is two units.
        assert_eq!(highlight_runs("😀ab", &[2]), vec![4..5]);
    }
}
