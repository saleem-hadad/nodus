//! Laying a chart out at a width: its data grouped and stacked, its scales
//! and ticks, and every bar, line, dot, label and tooltip region placed.
//!
//! The look: thin marks (bars at most 24 px thick, rounded only at their
//! data end, 2 px apart where they touch), 2 px lines, hairline grid lines,
//! quiet axis text, a legend once there are two series or more, and nothing
//! outside the chart's bounds.

use std::collections::HashMap;

use crate::format;
use crate::scale;
use crate::scene::{Anchor, Hover, Item, Rect, Rgba, Scene, Text, TipLine, VisualTheme, Weight};
use crate::spec::{Chart, Domain, FieldDef, FieldType, MarkKind, Row, Sort, Stack};
use crate::time;

/// A chart's title.
pub const TITLE_SIZE: f32 = 13.0;
const TITLE_H: f32 = 18.0;
/// Tick labels, the legend and axis titles.
pub const SMALL_SIZE: f32 = 11.0;
const SMALL_H: f32 = 16.0;
/// Between tick labels and the plot.
const LABEL_GAP: f32 = 8.0;
/// The thickest a bar gets; the rest of its band is air.
const BAR_MAX: f32 = 24.0;
/// Between marks that touch: stacked segments, bars in a group, wedges.
const GAP: f32 = 2.0;
/// A bar's rounded data end.
const END_RADIUS: f32 = 4.0;
const LINE_W: f32 = 2.0;
const DOT_R: f32 = 3.5;
/// The surface-coloured ring around dots on a line.
const RING: f32 = 1.5;
/// A dot's hover target, square.
const DOT_HIT: f32 = 24.0;
/// The longest a legend entry or category label gets before it's cut.
const LABEL_MAX: f32 = 160.0;

/// Measures text: `(text, font size) -> width`.
pub type Measure<'a> = &'a dyn Fn(&str, f32) -> f32;

impl Chart {
    /// The chart laid out `width` wide: its height follows from the spec's
    /// plot height and what goes around the plot (title, legend, labels).
    pub fn layout(&self, width: f32, theme: &VisualTheme, measure: &dyn Fn(&str, f32) -> f32) -> Scene {
        let width = width.max(40.0);
        let mut cx = Ctx {
            width,
            theme,
            measure,
            items: Vec::new(),
            hovers: Vec::new(),
        };
        let mut top = 0.0;
        if let Some(title) = self.title.as_deref().filter(|t| !t.trim().is_empty()) {
            let text = cx.fit(title, TITLE_SIZE, Weight::Semibold, width);
            cx.text(
                0.0,
                TITLE_H / 2.0,
                text,
                TITLE_SIZE,
                theme.label_text,
                Anchor::Start,
                Weight::Semibold,
            );
            top = TITLE_H + 8.0;
        }
        let height = match self.mark.kind {
            MarkKind::Arc => arc(self, &mut cx, top),
            k if k.is_dot() => dots(self, &mut cx, top),
            _ => series(self, &mut cx, top),
        };
        let mut scene = Scene {
            width,
            height: height.ceil(),
            items: cx.items,
            hovers: cx.hovers,
        };
        confine(&mut scene, measure);
        scene
    }
}

struct Ctx<'a> {
    width: f32,
    theme: &'a VisualTheme,
    measure: Measure<'a>,
    items: Vec<Item>,
    hovers: Vec<Hover>,
}

impl Ctx<'_> {
    fn text_w(&self, text: &str, size: f32, weight: Weight) -> f32 {
        let w = (self.measure)(text, size);
        // The measure is for regular text; semibold runs a little wider.
        if weight == Weight::Semibold { w * 1.05 } else { w }
    }

    /// `text`, cut with an ellipsis to fit `max`.
    fn fit(&self, text: &str, size: f32, weight: Weight, max: f32) -> String {
        fit(text, max, &|t| self.text_w(t, size, weight))
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, x: f32, y: f32, text: String, size: f32, color: Rgba, anchor: Anchor, weight: Weight) {
        if text.is_empty() {
            return;
        }
        self.items.push(Item::Text(Text {
            x,
            y,
            text,
            size,
            color,
            anchor,
            weight,
            tabular: false,
        }));
    }

    /// A tick label: tabular figures, in the axis colour.
    fn tick(&mut self, x: f32, y: f32, text: String, anchor: Anchor) {
        if text.is_empty() {
            return;
        }
        self.items.push(Item::Text(Text {
            x,
            y,
            text,
            size: SMALL_SIZE,
            color: self.theme.axis_text,
            anchor,
            weight: Weight::Regular,
            tabular: true,
        }));
    }

    fn rect(&mut self, rect: Rect, radius: [f32; 4], color: Rgba) {
        if rect.w > 0.0 && rect.h > 0.0 {
            self.items.push(Item::Rect { rect, radius, color });
        }
    }

    /// A horizontal hairline.
    fn hline(&mut self, x: f32, y: f32, w: f32, color: Rgba) {
        self.rect(Rect::new(x, y - 0.5, w, 1.0), [0.0; 4], color);
    }

    /// A vertical hairline.
    fn vline(&mut self, x: f32, y: f32, h: f32, color: Rgba) {
        self.rect(Rect::new(x - 0.5, y, 1.0, h), [0.0; 4], color);
    }
}

/// `text`, cut with an ellipsis so `width(text) <= max`.
fn fit(text: &str, max: f32, width: &dyn Fn(&str) -> f32) -> String {
    if width(text) <= max {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>().trim_end().to_owned() + "…";
        if width(&candidate) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return String::new();
    }
    chars[..lo].iter().collect::<String>().trim_end().to_owned() + "…"
}

// -- Ordering categories -----------------------------------------------------------

/// Orders categories found in data order, as a channel's sort and domain
/// say. `totals` gives each category's measure, for sorting by it.
fn order(found: Vec<String>, ch: Option<&FieldDef>, totals: &dyn Fn(&str) -> f64) -> Vec<String> {
    let Some(ch) = ch else { return found };
    let mut cats = match &ch.domain {
        Some(Domain::Values(list)) => list.clone(),
        _ => found,
    };
    match &ch.sort {
        Sort::Data => {}
        Sort::Ascending => cats.sort_by(|a, b| natural(a, b)),
        Sort::Descending => cats.sort_by(|a, b| natural(b, a)),
        Sort::Values(list) => {
            let rank = |c: &String| list.iter().position(|v| v == c).unwrap_or(usize::MAX);
            cats.sort_by_key(rank);
        }
        Sort::ByMeasure { descending } => {
            cats.sort_by(|a, b| {
                let (ta, tb) = (totals(a), totals(b));
                if *descending {
                    tb.total_cmp(&ta)
                } else {
                    ta.total_cmp(&tb)
                }
            });
        }
    }
    cats
}

/// Numbers by value, other text case-insensitively.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        _ => a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
    }
}

/// Distinct values in the order first seen, and each value's index.
#[derive(Default)]
struct Index {
    names: Vec<String>,
    at: HashMap<String, usize>,
}

impl Index {
    fn of(&mut self, name: String) -> usize {
        if let Some(&i) = self.at.get(&name) {
            return i;
        }
        let i = self.names.len();
        self.at.insert(name.clone(), i);
        self.names.push(name);
        i
    }
}

/// Maps old indices to new ones after reordering; `None` for names dropped.
fn remap(old: &[String], new: &[String]) -> Vec<Option<usize>> {
    let at: HashMap<&str, usize> = new.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    old.iter().map(|n| at.get(n.as_str()).copied()).collect()
}

// -- Axes ----------------------------------------------------------------------

/// What an axis maps: categories, or a span of numbers or times.
#[derive(Debug, Clone)]
enum Axis {
    Band(Vec<String>),
    Num {
        lo: f64,
        hi: f64,
        ticks: Vec<f64>,
        labels: Vec<String>,
    },
}

impl Axis {
    /// Where `v` (a category's index, or a value) falls between `a` and `b`.
    fn map(&self, v: f64, a: f32, b: f32) -> f32 {
        match self {
            Axis::Band(cats) => {
                let n = cats.len().max(1) as f32;
                a + (b - a) * ((v as f32 + 0.5) / n)
            }
            Axis::Num { lo, hi, .. } => {
                let t = if hi > lo { ((v - lo) / (hi - lo)) as f32 } else { 0.5 };
                a + (b - a) * t
            }
        }
    }

    /// One band's length.
    fn step(&self, a: f32, b: f32) -> f32 {
        match self {
            Axis::Band(cats) => (b - a).abs() / cats.len().max(1) as f32,
            Axis::Num { .. } => 0.0,
        }
    }

    fn contains(&self, v: f64) -> bool {
        match self {
            Axis::Band(cats) => v >= 0.0 && (v as usize) < cats.len(),
            Axis::Num { lo, hi, .. } => v >= *lo - 1e-9 * (hi - lo).abs() && v <= *hi + 1e-9 * (hi - lo).abs(),
        }
    }
}

/// How to make a numeric axis's ticks.
#[derive(Debug, Clone, Copy)]
struct NumSpec {
    lo: f64,
    hi: f64,
    /// Stretch the domain to round ticks.
    nice: bool,
    /// Whole-number ticks only.
    integer: bool,
    time: bool,
    /// Labels as percentages (a normalized stack).
    percent: bool,
}

impl NumSpec {
    fn axis(&self, count: usize) -> Axis {
        if self.time {
            let t = time::ticks(self.lo, self.hi, count);
            let labels = t.ticks.iter().map(|&v| time::tick_label(v, t.unit)).collect();
            return Axis::Num {
                lo: self.lo,
                hi: self.hi,
                ticks: t.ticks,
                labels,
            };
        }
        let t = scale::linear(self.lo, self.hi, count, self.nice, self.integer);
        let labels = if self.percent {
            format::percent_ticks(&t.ticks, t.step)
        } else {
            format::tick_labels(&t.ticks, t.step)
        };
        Axis::Num {
            lo: t.min,
            hi: t.max,
            ticks: t.ticks,
            labels,
        }
    }
}

/// An axis to lay out: what it maps, its title, whether it shows.
#[derive(Debug, Clone)]
enum AxisIn {
    Band(Vec<String>),
    Num(NumSpec),
}

struct AxisSide {
    input: AxisIn,
    title: Option<String>,
    show: bool,
}

/// The plot and its axes once placed.
struct Frame {
    plot: Rect,
    x: Axis,
    y: Axis,
    /// The chart's height down to the bottom of the x axis.
    bottom: f32,
}

impl Frame {
    fn x(&self, v: f64) -> f32 {
        self.x.map(v, self.plot.x, self.plot.right())
    }

    fn y(&self, v: f64) -> f32 {
        self.y.map(v, self.plot.bottom(), self.plot.y)
    }
}

/// Places the plot below `top`, `plot_h` tall, with the y axis's labels on
/// the left and its title above, and the x axis's labels and title below.
/// Draws grid lines (horizontal, or vertical with `grid_x`) and labels.
fn frame(cx: &mut Ctx<'_>, top: f32, plot_h: f32, xs: &AxisSide, ys: &AxisSide, grid_x: bool) -> Frame {
    let width = cx.width;
    let mut top = top;
    if let Some(title) = ys.title.as_deref().filter(|_| ys.show) {
        let text = cx.fit(title, SMALL_SIZE, Weight::Regular, width);
        cx.text(
            0.0,
            top + SMALL_H / 2.0,
            text,
            SMALL_SIZE,
            cx.theme.axis_text,
            Anchor::Start,
            Weight::Regular,
        );
        top += SMALL_H + 2.0;
    }
    // Room for the top tick label, which centres on the top grid line.
    let plot_top = top + 7.0;

    // The y axis and its label column.
    let y_count = ((plot_h / 44.0).round() as usize).clamp(2, 6);
    let (y_axis, y_labels): (Axis, Vec<String>) = match &ys.input {
        AxisIn::Band(cats) => (Axis::Band(cats.clone()), cats.clone()),
        AxisIn::Num(spec) => {
            let axis = spec.axis(y_count);
            let labels = match &axis {
                Axis::Num { labels, .. } => labels.clone(),
                Axis::Band(_) => Vec::new(),
            };
            (axis, labels)
        }
    };
    let label_col = if ys.show {
        let widest = y_labels
            .iter()
            .map(|l| cx.text_w(l, SMALL_SIZE, Weight::Regular))
            .fold(0.0, f32::max);
        widest.min(LABEL_MAX).min(width * 0.38)
    } else {
        0.0
    };
    let mut plot_left = if ys.show && label_col > 0.0 {
        label_col + LABEL_GAP
    } else {
        0.0
    };

    // The x axis: as many ticks as fit without their labels touching.
    let mut right_pad = 2.0;
    let x_axis = match &xs.input {
        AxisIn::Band(cats) => Axis::Band(cats.clone()),
        AxisIn::Num(spec) => {
            let mut count = (((width - plot_left) / 80.0).round() as usize).clamp(2, 8);
            loop {
                let axis = spec.axis(count);
                let Axis::Num { lo, hi, ticks, labels } = &axis else {
                    unreachable!()
                };
                let widths: Vec<f32> = labels
                    .iter()
                    .map(|l| cx.text_w(l, SMALL_SIZE, Weight::Regular))
                    .collect();
                let first = widths.first().copied().unwrap_or(0.0);
                let last = widths.last().copied().unwrap_or(0.0);
                let left = plot_left.max(if xs.show { first / 2.0 } else { 0.0 });
                let pad = if xs.show { (last / 2.0).max(6.0) } else { 6.0 };
                let plot_w = (width - left - pad).max(10.0);
                let at = |v: f64| left + plot_w * (if hi > lo { ((v - lo) / (hi - lo)) as f32 } else { 0.5 });
                let fits = ticks
                    .windows(2)
                    .zip(widths.windows(2))
                    .all(|(t, w)| at(t[1]) - at(t[0]) >= (w[0] + w[1]) / 2.0 + 12.0);
                if fits || count <= 2 || !xs.show {
                    plot_left = left;
                    right_pad = pad;
                    break axis;
                }
                count -= 1;
            }
        }
    };
    let plot_w = (width - plot_left - right_pad).max(10.0);
    let plot = Rect::new(plot_left, plot_top, plot_w, plot_h);
    let f = Frame {
        plot,
        x: x_axis,
        y: y_axis,
        bottom: 0.0,
    };

    // Grid lines, and the zero line (or the bottom one) a step darker.
    let theme = cx.theme.clone();
    if let Axis::Num { lo, hi, ticks, .. } = if grid_x { &f.x } else { &f.y } {
        let zero_in = *lo <= 0.0 && *hi >= 0.0;
        for (i, &t) in ticks.iter().enumerate() {
            let base = if zero_in { t == 0.0 } else { i == 0 };
            let color = if base { theme.baseline } else { theme.grid };
            if grid_x {
                cx.vline(f.x(t), plot.y, plot.h, color);
            } else {
                cx.hline(plot.x, f.y(t), plot.w, color);
            }
        }
    } else if grid_x {
        cx.vline(plot.x, plot.y, plot.h, theme.baseline);
    } else {
        cx.hline(plot.x, plot.bottom(), plot.w, theme.baseline);
    }

    // The y labels.
    if ys.show {
        let lx = plot.x - LABEL_GAP;
        match &f.y {
            Axis::Band(cats) => {
                let step = f.y.step(plot.y, plot.bottom());
                let every = ((SMALL_H / step.max(0.1)).ceil() as usize).max(1);
                for (i, c) in cats.iter().enumerate().filter(|(i, _)| i % every == 0) {
                    let text = cx.fit(c, SMALL_SIZE, Weight::Regular, label_col);
                    let y = f.y(i as f64);
                    cx.tick(lx, y, text, Anchor::End);
                }
            }
            Axis::Num { ticks, labels, .. } => {
                for (t, l) in ticks.iter().zip(labels) {
                    let y = f.y(*t);
                    cx.tick(lx, y, l.clone(), Anchor::End);
                }
            }
        }
    }

    // The x labels, thinned so they never touch.
    let mut bottom = plot.bottom();
    if xs.show {
        let ly = plot.bottom() + 6.0 + SMALL_H / 2.0;
        match &f.x {
            Axis::Band(cats) => {
                let step = f.x.step(plot.x, plot.right());
                let widths: Vec<f32> = cats
                    .iter()
                    .map(|c| cx.text_w(c, SMALL_SIZE, Weight::Regular).min(LABEL_MAX))
                    .collect();
                let widest = widths.iter().copied().fold(0.0, f32::max);
                let every = (((widest + 10.0) / step.max(0.1)).ceil() as usize).max(1);
                let room = (step * every as f32 - 10.0).max(step - 4.0).min(LABEL_MAX);
                for (i, c) in cats.iter().enumerate().filter(|(i, _)| i % every == 0) {
                    let text = cx.fit(c, SMALL_SIZE, Weight::Regular, room);
                    cx.tick(f.x(i as f64), ly, text, Anchor::Middle);
                }
            }
            Axis::Num { ticks, labels, .. } => {
                // Already as many as fit; if two still touch, every other one.
                let widths: Vec<f32> = labels
                    .iter()
                    .map(|l| cx.text_w(l, SMALL_SIZE, Weight::Regular))
                    .collect();
                let mut every = 1;
                while every < ticks.len()
                    && !ticks
                        .iter()
                        .step_by(every)
                        .zip(widths.iter().step_by(every))
                        .collect::<Vec<_>>()
                        .windows(2)
                        .all(|p| f.x(*p[1].0) - f.x(*p[0].0) >= (p[0].1 + p[1].1) / 2.0 + 8.0)
                {
                    every += 1;
                }
                for (t, l) in ticks.iter().zip(labels).step_by(every) {
                    cx.tick(f.x(*t), ly, l.clone(), Anchor::Middle);
                }
            }
        }
        bottom = plot.bottom() + 6.0 + SMALL_H;
        if let Some(title) = &xs.title {
            let text = cx.fit(title, SMALL_SIZE, Weight::Regular, width);
            let x = plot.x + plot.w / 2.0;
            cx.text(
                x,
                bottom + 4.0 + SMALL_H / 2.0,
                text,
                SMALL_SIZE,
                theme.axis_text,
                Anchor::Middle,
                Weight::Regular,
            );
            bottom += 4.0 + SMALL_H;
        }
    }
    Frame {
        bottom: bottom + 2.0,
        ..f
    }
}

// -- The legend -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Square,
    Line,
    Dot,
}

fn key_for(mark: MarkKind) -> Key {
    match mark {
        MarkKind::Line => Key::Line,
        k if k.is_dot() => Key::Dot,
        _ => Key::Square,
    }
}

/// A swatch for a series, its left edge at `x`, centred on `y`.
fn swatch(cx: &mut Ctx<'_>, key: Key, x: f32, y: f32, color: Rgba) {
    match key {
        Key::Square => cx.rect(Rect::new(x, y - 4.0, 8.0, 8.0), [2.0; 4], color),
        Key::Line => cx.rect(Rect::new(x, y - 1.0, 12.0, 2.0), [1.0; 4], color),
        Key::Dot => cx.items.push(Item::Circle {
            x: x + 4.0,
            y,
            r: 4.0,
            fill: Some(color),
            stroke: None,
        }),
    }
}

fn key_w(key: Key) -> f32 {
    if key == Key::Line { 12.0 } else { 8.0 }
}

/// The legend in rows across the top, wrapping at the chart's width.
/// Returns the height it took.
fn legend(cx: &mut Ctx<'_>, top: f32, entries: &[(String, Rgba)], key: Key) -> f32 {
    if entries.is_empty() {
        return 0.0;
    }
    let mut x = 0.0;
    let mut y = top + SMALL_H / 2.0;
    for (name, color) in entries {
        let text = cx.fit(name, SMALL_SIZE, Weight::Regular, LABEL_MAX);
        let w = key_w(key) + 6.0 + cx.text_w(&text, SMALL_SIZE, Weight::Regular);
        if x > 0.0 && x + w > cx.width {
            x = 0.0;
            y += SMALL_H + 2.0;
        }
        swatch(cx, key, x, y, *color);
        let label = cx.theme.label_text;
        cx.text(
            x + key_w(key) + 6.0,
            y,
            text,
            SMALL_SIZE,
            label,
            Anchor::Start,
            Weight::Regular,
        );
        x += w + 14.0;
    }
    y + SMALL_H / 2.0 - top
}

// -- Bars, lines and areas -------------------------------------------------------

/// One bar, segment or point of a line: a domain value (a category's index
/// or a number), its series and group, and its aggregated value.
#[derive(Debug, Clone)]
struct Mk {
    d: f64,
    s: usize,
    g: usize,
    value: f64,
    /// Where it starts and ends on the measure axis (stacked), or 0 and
    /// the value.
    lo: f64,
    hi: f64,
    /// The data rows it sums.
    rows: Vec<usize>,
    /// Nearest the baseline in its stack, and furthest from it.
    base: bool,
    end: bool,
}

/// A bar, line or area chart's data, grouped, ordered and stacked.
struct Prepared<'a> {
    dom: &'a FieldDef,
    meas: &'a FieldDef,
    discrete: bool,
    cats: Vec<String>,
    /// Distinct domain values, ascending, when it's continuous.
    values: Vec<f64>,
    series: Vec<String>,
    subs: Vec<String>,
    marks: Vec<Mk>,
    stacked: bool,
    normalize: bool,
}

fn prepare<'a>(chart: &'a Chart) -> Prepared<'a> {
    let e = &chart.encoding;
    let horizontal = chart.horizontal();
    let (dom, meas) = match (&e.x, &e.y) {
        (Some(x), Some(y)) if horizontal => (y, x),
        (Some(x), Some(y)) => (x, y),
        _ => unreachable!("checked when parsed"),
    };
    let series_ch = chart.series_channel();
    let sub_ch = e.x_offset.as_ref();
    let discrete = dom.kind.is_discrete();
    let mark = chart.mark.kind;
    let mode = meas.stack.unwrap_or(match mark {
        MarkKind::Bar | MarkKind::Area => Stack::Zero,
        _ => Stack::Off,
    });
    let count = meas.aggregate == Some(crate::spec::Aggregate::Count);

    let (mut cat_ix, mut ser_ix, mut sub_ix) = (Index::default(), Index::default(), Index::default());
    // A mark per (domain value, series, group), and the values it sums.
    let mut marks: Vec<Mk> = Vec::new();
    let mut piles: Vec<Vec<f64>> = Vec::new();
    let mut group_at: HashMap<(u64, usize, usize), usize> = HashMap::new();
    for (i, row) in chart.data.iter().enumerate() {
        let d = if discrete {
            match dom.text(row) {
                Some(t) => cat_ix.of(t) as f64,
                None => continue,
            }
        } else {
            match dom.number(row) {
                Some(v) => v,
                None => continue,
            }
        };
        let s = match series_ch {
            Some(c) => match c.text(row) {
                Some(t) => ser_ix.of(t),
                None => continue,
            },
            None => 0,
        };
        let g = match sub_ch {
            Some(c) => match c.text(row) {
                Some(t) => sub_ix.of(t),
                None => continue,
            },
            None => 0,
        };
        let v = if count {
            1.0
        } else {
            match meas.number(row) {
                Some(v) => v,
                None => continue,
            }
        };
        let key = (d.to_bits(), s, g);
        let at = *group_at.entry(key).or_insert_with(|| {
            marks.push(Mk {
                d,
                s,
                g,
                value: 0.0,
                lo: 0.0,
                hi: 0.0,
                rows: Vec::new(),
                base: true,
                end: true,
            });
            piles.push(Vec::new());
            marks.len() - 1
        });
        piles[at].push(v);
        marks[at].rows.push(i);
    }
    let aggregate = meas.aggregate.unwrap_or(crate::spec::Aggregate::Sum);
    for (m, pile) in marks.iter_mut().zip(&piles) {
        m.value = aggregate.apply(pile);
        m.hi = m.value;
    }

    // Order categories, series and groups, and renumber the marks.
    let total_by = |key: &dyn Fn(&Mk) -> Option<usize>, n: usize| {
        let mut t = vec![0.0; n];
        for m in &marks {
            if let Some(k) = key(m) {
                t[k] += m.value;
            }
        }
        t
    };
    let cat_totals = total_by(&|m| discrete.then_some(m.d as usize), cat_ix.names.len());
    let ser_totals = total_by(&|m| Some(m.s), ser_ix.names.len().max(1));
    let cats = order(cat_ix.names.clone(), Some(dom), &|c| {
        cat_ix.at.get(c).map_or(0.0, |&i| cat_totals[i])
    });
    let series = order(ser_ix.names.clone(), series_ch, &|c| {
        ser_ix.at.get(c).map_or(0.0, |&i| ser_totals[i])
    });
    let subs = order(sub_ix.names.clone(), sub_ch, &|_| 0.0);
    let (cat_map, ser_map, sub_map) = (
        remap(&cat_ix.names, &cats),
        remap(&ser_ix.names, &series),
        remap(&sub_ix.names, &subs),
    );
    marks.retain_mut(|m| {
        if discrete {
            let Some(d) = cat_map[m.d as usize] else { return false };
            m.d = d as f64;
        }
        if series_ch.is_some() {
            let Some(s) = ser_map[m.s] else { return false };
            m.s = s;
        }
        if sub_ch.is_some() {
            let Some(g) = sub_map[m.g] else { return false };
            m.g = g;
        }
        true
    });

    let has_series = series_ch.is_some();
    let stacked = has_series && mode != Stack::Off && !(mark == MarkKind::Line && meas.stack.is_none());
    let normalize = mode == Stack::Normalize && (stacked || !has_series);
    // Areas stack across every domain value, so each series needs one.
    if stacked && mark == MarkKind::Area {
        let mut have: HashMap<(u64, usize), ()> = HashMap::new();
        let mut ds: Vec<f64> = Vec::new();
        for m in &marks {
            have.insert((m.d.to_bits(), m.s), ());
            if !ds.iter().any(|d| d.to_bits() == m.d.to_bits()) {
                ds.push(m.d);
            }
        }
        for &d in &ds {
            for s in 0..series.len() {
                if !have.contains_key(&(d.to_bits(), s)) {
                    marks.push(Mk {
                        d,
                        s,
                        g: 0,
                        value: 0.0,
                        lo: 0.0,
                        hi: 0.0,
                        rows: Vec::new(),
                        base: true,
                        end: true,
                    });
                }
            }
        }
    }
    marks.sort_by(|a, b| a.d.total_cmp(&b.d).then(a.g.cmp(&b.g)).then(a.s.cmp(&b.s)));
    if stacked || normalize {
        stack(&mut marks, stacked, normalize);
    }

    let mut values: Vec<f64> = if discrete {
        Vec::new()
    } else {
        marks.iter().map(|m| m.d).collect()
    };
    values.sort_by(f64::total_cmp);
    values.dedup();
    Prepared {
        dom,
        meas,
        discrete,
        cats,
        values,
        series,
        subs,
        marks,
        stacked,
        normalize,
    }
}

/// Stacks marks that share a domain value and group, series in order:
/// positive values up from zero and negative ones down. Normalized, each
/// stack runs 0 to 1.
fn stack(marks: &mut [Mk], stacked: bool, normalize: bool) {
    let mut start = 0;
    while start < marks.len() {
        let key = (marks[start].d.to_bits(), marks[start].g);
        let mut end = start;
        while end < marks.len() && (marks[end].d.to_bits(), marks[end].g) == key {
            end += 1;
        }
        let group = &mut marks[start..end];
        let total: f64 = group.iter().map(|m| m.value.abs()).sum();
        let scale = if normalize && total > 0.0 { 1.0 / total } else { 1.0 };
        let (mut up, mut down) = (0.0, 0.0);
        let (mut first_up, mut first_down) = (None, None);
        let (mut last_up, mut last_down) = (None, None);
        for (i, m) in group.iter_mut().enumerate() {
            let v = m.value * scale;
            if !stacked {
                (m.lo, m.hi) = (0.0, v);
                continue;
            }
            if v >= 0.0 {
                (m.lo, m.hi) = (up, up + v);
                up += v;
                if v > 0.0 {
                    first_up.get_or_insert(i);
                    last_up = Some(i);
                }
            } else {
                (m.lo, m.hi) = (down + v, down);
                down += v;
                first_down.get_or_insert(i);
                last_down = Some(i);
            }
        }
        for (i, m) in group.iter_mut().enumerate() {
            if stacked {
                m.base = Some(i) == first_up || Some(i) == first_down;
                m.end = Some(i) == last_up || Some(i) == last_down;
            }
        }
        start = end;
    }
}

/// A domain value as a tooltip shows it.
fn dom_label(p: &Prepared<'_>, d: f64, with_time: bool) -> String {
    if p.discrete {
        p.cats.get(d as usize).cloned().unwrap_or_default()
    } else if p.dom.kind == FieldType::Temporal {
        time::full(d, with_time)
    } else {
        format::full(d)
    }
}

/// A mark's value as a tooltip shows it.
fn value_label(p: &Prepared<'_>, m: &Mk) -> String {
    if p.normalize {
        format!("{} · {}", format::percent(m.hi - m.lo), format::full(m.value))
    } else {
        format::full(m.value)
    }
}

/// The tooltip lines for one mark: the encoding's tooltip fields, or the
/// series (or measure) and value.
fn mark_tip(chart: &Chart, p: &Prepared<'_>, m: &Mk, color: Option<Rgba>) -> Vec<TipLine> {
    if let Some(fields) = chart.encoding.tooltip.as_ref().filter(|f| !f.is_empty()) {
        return tip_fields(&chart.data, fields, &m.rows, p.meas, m.value);
    }
    let label = if p.series.is_empty() {
        p.meas.auto_title()
    } else {
        p.series.get(m.s).cloned().unwrap_or_default()
    };
    vec![TipLine {
        label,
        value: value_label(p, m),
        color,
    }]
}

/// Lines for the encoding's tooltip fields over some rows. The measure's
/// field shows the mark's own (aggregated) value.
fn tip_fields(data: &[Row], fields: &[FieldDef], rows: &[usize], meas: &FieldDef, value: f64) -> Vec<TipLine> {
    let first = rows.first().and_then(|&i| data.get(i));
    fields
        .iter()
        .map(|f| {
            let label = f.title.clone().unwrap_or_else(|| f.auto_title());
            let value = if let Some(a) = f.aggregate {
                let vals: Vec<f64> = rows.iter().filter_map(|&i| f.number(&data[i])).collect();
                let n = if a == crate::spec::Aggregate::Count {
                    rows.len() as f64
                } else {
                    a.apply(&vals)
                };
                format::full(n)
            } else if f.field.is_some() && f.field == meas.field && rows.len() > 1 {
                format::full(value)
            } else {
                first.map_or_else(String::new, |r| field_text(f, r))
            };
            TipLine {
                label,
                value,
                color: None,
            }
        })
        .collect()
}

/// A row's value for a field, as a tooltip shows it.
fn field_text(f: &FieldDef, row: &Row) -> String {
    match f.kind {
        FieldType::Temporal => f.number(row).map_or_else(
            || f.text(row).unwrap_or_default(),
            |t| time::full(t, time::has_time([t])),
        ),
        FieldType::Quantitative => f
            .number(row)
            .map_or_else(|| f.text(row).unwrap_or_default(), format::full),
        _ => f.text(row).unwrap_or_default(),
    }
}

/// The default plot height.
fn plot_height(chart: &Chart, bands: usize, horizontal: bool) -> f32 {
    match chart.height {
        Some(h) => h,
        None if horizontal => (bands as f32 * 26.0).clamp(100.0, 600.0),
        None => crate::spec::DEFAULT_HEIGHT,
    }
}

/// Bars, lines and areas.
fn series(chart: &Chart, cx: &mut Ctx<'_>, top: f32) -> f32 {
    let p = prepare(chart);
    let mark = chart.mark.kind;
    let horizontal = chart.horizontal();
    let theme = cx.theme.clone();
    let color_of = |s: usize| theme.series(s);
    // Series with no series channel: grouped bars take their colours from
    // the groups.
    let grouped_colors = p.series.is_empty() && !p.subs.is_empty();
    let mut top = top;
    let show_legend = chart.series_channel().is_some_and(|c| c.legend);
    if show_legend && p.series.len() >= 2 {
        let entries: Vec<(String, Rgba)> = p
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), color_of(i)))
            .collect();
        top += legend(cx, top, &entries, key_for(mark)) + 8.0;
    } else if grouped_colors && p.subs.len() >= 2 {
        let entries: Vec<(String, Rgba)> = p
            .subs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), color_of(i)))
            .collect();
        top += legend(cx, top, &entries, Key::Square) + 8.0;
    }

    // The measure axis's span.
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for m in &p.marks {
        let (a, b) = if p.stacked || p.normalize {
            (m.lo, m.hi)
        } else {
            (m.value, m.value)
        };
        lo = lo.min(a);
        hi = hi.max(b);
    }
    if !lo.is_finite() {
        (lo, hi) = (0.0, 1.0);
    }
    let mut nice = true;
    if p.normalize {
        (lo, hi) = (lo.min(0.0), hi.max(1.0));
    }
    let zero = p.meas.zero.unwrap_or(true);
    if zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    if let Some(Domain::Range(a, b)) = p.meas.domain {
        (lo, hi, nice) = (a, b, false);
    }
    let meas_spec = NumSpec {
        lo,
        hi,
        nice,
        integer: p.meas.aggregate == Some(crate::spec::Aggregate::Count),
        time: false,
        percent: p.normalize,
    };

    // The domain axis.
    let is_bar = mark == MarkKind::Bar;
    let dom_in = if p.discrete {
        AxisIn::Band(p.cats.clone())
    } else {
        let (mut a, mut b) = (
            p.values.first().copied().unwrap_or(0.0),
            p.values.last().copied().unwrap_or(1.0),
        );
        if p.dom.zero == Some(true) {
            a = a.min(0.0);
            b = b.max(0.0);
        }
        if is_bar {
            // Half a bar's room past the first and last bars.
            let gap = min_gap(&p.values).unwrap_or(if p.dom.kind == FieldType::Temporal {
                time::DAY
            } else {
                1.0
            });
            a -= gap / 2.0;
            b += gap / 2.0;
        }
        if let Some(Domain::Range(x, y)) = p.dom.domain {
            (a, b) = (x, y);
        }
        AxisIn::Num(NumSpec {
            lo: a,
            hi: b,
            nice: false,
            integer: p.values.iter().all(|v| v.fract() == 0.0),
            time: p.dom.kind == FieldType::Temporal,
            percent: false,
        })
    };
    let dom_side = AxisSide {
        input: dom_in,
        title: p.dom.axis_title(),
        show: p.dom.shows_axis(),
    };
    let meas_side = AxisSide {
        input: AxisIn::Num(meas_spec),
        title: p.meas.axis_title(),
        show: p.meas.shows_axis(),
    };
    let bands = if p.discrete { p.cats.len() } else { p.values.len() };
    let plot_h = plot_height(chart, bands, horizontal);
    let f = if horizontal {
        frame(cx, top, plot_h, &meas_side, &dom_side, true)
    } else {
        frame(cx, top, plot_h, &dom_side, &meas_side, false)
    };

    let with_time = p.dom.kind == FieldType::Temporal && time::has_time(p.values.iter().copied());
    let tips = chart.encoding.tooltip.as_ref().is_none_or(|t| !t.is_empty());
    match mark {
        MarkKind::Bar => bars(cx, chart, &p, &f, horizontal, with_time, tips),
        _ => lines(cx, chart, &p, &f, with_time, tips),
    }
    f.bottom
}

/// The smallest step between neighbouring values.
fn min_gap(values: &[f64]) -> Option<f64> {
    values
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|g| *g > 0.0)
        .min_by(f64::total_cmp)
}

/// Where a mark's band is along the domain axis, in px: its start and
/// thickness.
fn bar_span(p: &Prepared<'_>, f: &Frame, horizontal: bool, m: &Mk) -> (f32, f32) {
    let (axis, a, b) = if horizontal {
        (&f.y, f.plot.y, f.plot.bottom())
    } else {
        (&f.x, f.plot.x, f.plot.right())
    };
    let centre = axis.map(m.d, a, b);
    let room = if p.discrete {
        axis.step(a, b)
    } else {
        // Continuous: as much room as the closest neighbours leave.
        min_gap(&p.values).map_or((b - a).abs() * 0.5, |g| {
            (axis.map(p.values[0] + g, a, b) - axis.map(p.values[0], a, b)).abs()
        })
    };
    let groups = p.subs.len().max(1);
    let grouped = groups > 1 || (!p.stacked && p.series.len() > 1);
    if !grouped {
        let w = (room * 0.62).clamp(1.0, BAR_MAX);
        return (centre - w / 2.0, w);
    }
    let k = if groups > 1 { groups } else { p.series.len() };
    let slot = if groups > 1 { m.g } else { m.s };
    let w = ((room * 0.84 - GAP * (k - 1) as f32) / k as f32).clamp(1.0, BAR_MAX);
    let total = w * k as f32 + GAP * (k - 1) as f32;
    (centre - total / 2.0 + slot as f32 * (w + GAP), w)
}

#[allow(clippy::too_many_arguments)]
fn bars(cx: &mut Ctx<'_>, chart: &Chart, p: &Prepared<'_>, f: &Frame, horizontal: bool, with_time: bool, tips: bool) {
    let theme = cx.theme.clone();
    let plot = f.plot;
    let colored_by_group = p.series.is_empty() && !p.subs.is_empty();
    let to_px = |v: f64| {
        if horizontal {
            f.x(v).clamp(plot.x, plot.right())
        } else {
            f.y(v).clamp(plot.y, plot.bottom())
        }
    };
    // Bars grow from zero, or from the end of the axis nearest it.
    let base_px = to_px(0.0);
    let stacked_view = p.stacked && p.series.len() > 1;
    for m in &p.marks {
        if (!p.discrete && !f_contains(f, horizontal, m.d)) || (m.hi - m.lo).abs() <= f64::EPSILON && m.value == 0.0 {
            continue;
        }
        let color = if colored_by_group {
            theme.series(m.g)
        } else {
            theme.series(m.s)
        };
        let (start, w) = bar_span(p, f, horizontal, m);
        let (from, to) = if p.stacked || p.normalize {
            (to_px(m.lo), to_px(m.hi))
        } else {
            (base_px, to_px(m.value))
        };
        let positive = if p.stacked || p.normalize {
            m.hi > 0.0
        } else {
            m.value >= 0.0
        };
        // Gaps between segments: each one above the first gives up 2 px
        // on its baseline side.
        let inset = if stacked_view && !m.base { GAP } else { 0.0 };
        let r = if m.end { END_RADIUS } else { 0.0 };
        let (rect, radius) = if horizontal {
            let (mut x0, mut x1) = (from.min(to), from.max(to));
            if positive {
                x0 += inset;
            } else {
                x1 -= inset;
            }
            let rect = Rect::new(x0, start, (x1 - x0).max(0.0), w);
            let r = r.min(w / 2.0).min(rect.w);
            (rect, if positive { [0.0, r, r, 0.0] } else { [r, 0.0, 0.0, r] })
        } else {
            let (mut y0, mut y1) = (from.min(to), from.max(to));
            if positive {
                y1 -= inset;
            } else {
                y0 += inset;
            }
            let rect = Rect::new(start, y0, w, (y1 - y0).max(0.0));
            let r = r.min(w / 2.0).min(rect.h);
            (rect, if positive { [r, r, 0.0, 0.0] } else { [0.0, 0.0, r, r] })
        };
        // A bar too short to see still shows as a sliver.
        let rect = if horizontal && rect.w < 1.0 {
            Rect { w: 1.0, ..rect }
        } else if !horizontal && rect.h < 1.0 {
            Rect {
                y: if positive { rect.y - (1.0 - rect.h) } else { rect.y },
                h: 1.0,
                ..rect
            }
        } else {
            rect
        };
        cx.rect(rect, radius, color);
        if !tips {
            continue;
        }
        // The hover target: the whole column for a lone bar, the segment
        // (and to the end of the plot for the top one) in a stack.
        let target = if stacked_view {
            let mut t = Rect::new(rect.x - 2.0, rect.y - 1.0, rect.w + 4.0, rect.h + 2.0);
            if horizontal {
                t = Rect::new(rect.x - 1.0, rect.y - 2.0, rect.w + 2.0, rect.h + 4.0);
                if m.end && positive {
                    t.w = plot.right() - t.x;
                }
            } else if m.end && positive {
                t.h = t.bottom() - plot.y;
                t.y = plot.y;
            }
            t
        } else if horizontal {
            Rect::new(plot.x, start - GAP / 2.0, plot.w, w + GAP)
        } else {
            Rect::new(start - GAP / 2.0, plot.y, w + GAP, plot.h)
        };
        let mut title = dom_label(p, m.d, with_time);
        // A group that isn't the series says which it is.
        let sub_field = chart.encoding.x_offset.as_ref().and_then(|c| c.field.as_ref());
        let series_field = chart.series_channel().and_then(|c| c.field.as_ref());
        if !p.subs.is_empty() && !colored_by_group && sub_field != series_field {
            title = format!("{title} · {}", p.subs.get(m.g).cloned().unwrap_or_default());
        }
        let lines = mark_tip(chart, p, m, Some(color));
        cx.hovers.push(Hover {
            rect: target,
            shape: None,
            title,
            lines: if colored_by_group && chart.encoding.tooltip.is_none() {
                vec![TipLine {
                    label: p.subs.get(m.g).cloned().unwrap_or_default(),
                    value: value_label(p, m),
                    color: Some(color),
                }]
            } else {
                lines
            },
            highlight: vec![Item::Rect {
                rect,
                radius,
                color: theme.hover,
            }],
        });
    }
}

fn f_contains(f: &Frame, horizontal: bool, d: f64) -> bool {
    if horizontal { f.y.contains(d) } else { f.x.contains(d) }
}

/// Lines and areas.
fn lines(cx: &mut Ctx<'_>, chart: &Chart, p: &Prepared<'_>, f: &Frame, with_time: bool, tips: bool) {
    let theme = cx.theme.clone();
    let plot = f.plot;
    let mark = chart.mark.kind;
    let n_series = p.series.len().max(1);
    let y_px = |v: f64| f.y(v).clamp(plot.y, plot.bottom());
    let x_px = |d: f64| f.x(d).clamp(plot.x, plot.right());
    let base = match &f.y {
        Axis::Num { lo, hi, .. } => y_px(0.0f64.clamp(*lo, *hi)),
        Axis::Band(_) => plot.bottom(),
    };
    let top_of = |m: &Mk| if p.stacked || p.normalize { m.hi } else { m.value };
    let visible = |m: &&Mk| p.discrete || f.x.contains(m.d);

    // Areas first, so every line sits on top of every fill.
    if mark == MarkKind::Area {
        let alpha = if p.stacked {
            0.34
        } else if n_series > 1 {
            0.12
        } else {
            0.16
        };
        for s in 0..n_series {
            let pts: Vec<&Mk> = p.marks.iter().filter(|m| m.s == s).filter(visible).collect();
            if pts.len() < 2 {
                continue;
            }
            let mut poly: Vec<[f32; 2]> = pts.iter().map(|m| [x_px(m.d), y_px(top_of(m))]).collect();
            for m in pts.iter().rev() {
                let y = if p.stacked || p.normalize { y_px(m.lo) } else { base };
                poly.push([x_px(m.d), y]);
            }
            cx.items.push(Item::Polygon {
                points: poly,
                color: theme.series(s).alpha(alpha),
            });
        }
    }
    let width = if mark == MarkKind::Area && p.stacked {
        1.5
    } else {
        LINE_W
    };
    for s in 0..n_series {
        let pts: Vec<[f32; 2]> = p
            .marks
            .iter()
            .filter(|m| m.s == s)
            .filter(visible)
            .map(|m| [x_px(m.d), y_px(top_of(m))])
            .collect();
        let color = theme.series(s);
        if pts.len() >= 2 {
            cx.items.push(Item::Line {
                points: pts.clone(),
                width,
                color,
            });
        }
        if chart.mark.point || pts.len() == 1 {
            for [x, y] in pts {
                cx.items.push(Item::Circle {
                    x,
                    y,
                    r: DOT_R,
                    fill: Some(color),
                    stroke: Some((RING, theme.background)),
                });
            }
        }
    }
    if !tips {
        return;
    }
    // One hover column per domain value, reading out every series there.
    let ds: Vec<f64> = if p.discrete {
        (0..p.cats.len()).map(|i| i as f64).collect()
    } else {
        p.values.iter().copied().filter(|&d| f.x.contains(d)).collect()
    };
    let xs: Vec<f32> = ds.iter().map(|&d| x_px(d)).collect();
    for (i, &d) in ds.iter().enumerate() {
        let left = if i == 0 { plot.x } else { (xs[i - 1] + xs[i]) / 2.0 };
        let right = if i + 1 == xs.len() {
            plot.right()
        } else {
            (xs[i] + xs[i + 1]) / 2.0
        };
        let here: Vec<&Mk> = p.marks.iter().filter(|m| m.d.to_bits() == d.to_bits()).collect();
        if here.is_empty() {
            continue;
        }
        let x = xs[i];
        let mut highlight = vec![Item::Rect {
            rect: Rect::new(x - 0.5, plot.y, 1.0, plot.h),
            radius: [0.0; 4],
            color: theme.baseline,
        }];
        let mut tip = Vec::new();
        for m in &here {
            let color = theme.series(m.s);
            highlight.push(Item::Circle {
                x,
                y: y_px(top_of(m)),
                r: 4.0,
                fill: Some(color),
                stroke: Some((2.0, theme.background)),
            });
            if m.rows.is_empty() && p.stacked {
                continue;
            }
            if p.series.is_empty() {
                tip.extend(mark_tip(chart, p, m, Some(color)));
            } else {
                tip.push(TipLine {
                    label: p.series.get(m.s).cloned().unwrap_or_default(),
                    value: value_label(p, m),
                    color: Some(color),
                });
            }
        }
        cx.hovers.push(Hover {
            rect: Rect::new(left, plot.y, right - left, plot.h),
            shape: None,
            title: dom_label(p, d, with_time),
            lines: tip,
            highlight,
        });
    }
}

// -- Dots ------------------------------------------------------------------------

/// Points, circles and squares: each axis by its own type.
fn dots(chart: &Chart, cx: &mut Ctx<'_>, top: f32) -> f32 {
    let e = &chart.encoding;
    let (Some(xd), Some(yd)) = (&e.x, &e.y) else { return top };
    let theme = cx.theme.clone();
    let series_ch = chart.series_channel();
    let tips = e.tooltip.as_ref().is_none_or(|t| !t.is_empty());

    // Categories along discrete axes, series, and the dots.
    let (mut xi, mut yi, mut si) = (Index::default(), Index::default(), Index::default());
    struct Dot {
        x: f64,
        y: f64,
        s: usize,
        rows: Vec<usize>,
    }
    let pos = |ch: &FieldDef, row: &Row, ix: &mut Index| -> Option<f64> {
        if ch.kind.is_discrete() {
            ch.text(row).map(|t| ix.of(t) as f64)
        } else if ch.aggregate == Some(crate::spec::Aggregate::Count) {
            Some(1.0)
        } else {
            ch.number(row)
        }
    };
    let aggregated = xd.aggregate.is_some() || yd.aggregate.is_some();
    let mut dots: Vec<Dot> = Vec::new();
    // Aggregated: a pile of values (and their rows) per position and series.
    type Pile = (Vec<f64>, Vec<usize>);
    let mut groups: HashMap<(u64, u64, usize), Pile> = HashMap::new();
    let mut group_order: Vec<(u64, u64, usize)> = Vec::new();
    for (i, row) in chart.data.iter().enumerate() {
        let (Some(x), Some(y)) = (pos(xd, row, &mut xi), pos(yd, row, &mut yi)) else {
            continue;
        };
        let s = match series_ch {
            Some(c) => match c.text(row) {
                Some(t) => si.of(t),
                None => continue,
            },
            None => 0,
        };
        if aggregated {
            // Group by the other axis; the aggregated one's values pile up.
            let (key_x, key_y) = if yd.aggregate.is_some() {
                (x.to_bits(), 0)
            } else {
                (0, y.to_bits())
            };
            let key = (key_x, key_y, s);
            let entry = groups.entry(key).or_insert_with(|| {
                group_order.push(key);
                (Vec::new(), Vec::new())
            });
            entry.0.push(if yd.aggregate.is_some() { y } else { x });
            entry.1.push(i);
        } else {
            dots.push(Dot { x, y, s, rows: vec![i] });
        }
    }
    for key in group_order {
        let (vals, rows) = &groups[&key];
        if let Some(a) = yd.aggregate {
            dots.push(Dot {
                x: f64::from_bits(key.0),
                y: a.apply(vals),
                s: key.2,
                rows: rows.clone(),
            });
        } else if let Some(a) = xd.aggregate {
            dots.push(Dot {
                x: a.apply(vals),
                y: f64::from_bits(key.1),
                s: key.2,
                rows: rows.clone(),
            });
        }
    }
    let series = order(si.names.clone(), series_ch, &|_| 0.0);
    let ser_map = remap(&si.names, &series);
    let xcats = order(xi.names.clone(), Some(xd), &|_| 0.0);
    let ycats = order(yi.names.clone(), Some(yd), &|_| 0.0);
    let (xmap, ymap) = (remap(&xi.names, &xcats), remap(&yi.names, &ycats));
    dots.retain_mut(|d| {
        if xd.kind.is_discrete() {
            let Some(x) = xmap[d.x as usize] else { return false };
            d.x = x as f64;
        }
        if yd.kind.is_discrete() {
            let Some(y) = ymap[d.y as usize] else { return false };
            d.y = y as f64;
        }
        if series_ch.is_some() {
            let Some(s) = ser_map[d.s] else { return false };
            d.s = s;
        }
        true
    });

    let mut top = top;
    if series_ch.is_some_and(|c| c.legend) && series.len() >= 2 {
        let entries: Vec<(String, Rgba)> = series
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), theme.series(i)))
            .collect();
        top += legend(cx, top, &entries, Key::Dot) + 8.0;
    }
    let side = |ch: &FieldDef, cats: &[String], vals: Vec<f64>, zero_default: bool| -> AxisSide {
        let input = if ch.kind.is_discrete() {
            AxisIn::Band(cats.to_vec())
        } else {
            let (mut lo, mut hi) = vals
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
            if !lo.is_finite() {
                (lo, hi) = (0.0, 1.0);
            }
            let time = ch.kind == FieldType::Temporal;
            if !time && ch.zero.unwrap_or(zero_default) {
                lo = lo.min(0.0);
                hi = hi.max(0.0);
            }
            let mut nice = !time;
            if let Some(Domain::Range(a, b)) = ch.domain {
                (lo, hi, nice) = (a, b, false);
            }
            AxisIn::Num(NumSpec {
                lo,
                hi,
                nice,
                integer: vals.iter().all(|v| v.fract() == 0.0),
                time,
                percent: false,
            })
        };
        AxisSide {
            input,
            title: ch.axis_title(),
            show: ch.shows_axis(),
        }
    };
    let xs = side(xd, &xcats, dots.iter().map(|d| d.x).collect(), false);
    let ys = side(yd, &ycats, dots.iter().map(|d| d.y).collect(), true);
    let bands = if yd.kind.is_discrete() { ycats.len() } else { 0 };
    let plot_h = plot_height(chart, bands, yd.kind.is_discrete());
    let f = frame(
        cx,
        top,
        plot_h,
        &xs,
        &ys,
        yd.kind.is_discrete() && !xd.kind.is_discrete(),
    );
    let plot = f.plot;
    let with_time = |ch: &FieldDef| {
        ch.kind == FieldType::Temporal && time::has_time(chart.data.iter().filter_map(|r| ch.number(r)))
    };
    let (x_time, y_time) = (with_time(xd), with_time(yd));
    let show = |ch: &FieldDef, cats: &[String], v: f64, timed: bool| -> String {
        if ch.kind.is_discrete() {
            cats.get(v as usize).cloned().unwrap_or_default()
        } else if ch.kind == FieldType::Temporal {
            time::full(v, timed)
        } else {
            format::full(v)
        }
    };
    for d in &dots {
        if !f.x.contains(d.x) || !f.y.contains(d.y) {
            continue;
        }
        let (x, y) = (f.x(d.x), f.y(d.y));
        let color = theme.series(d.s);
        let item = match chart.mark.kind {
            MarkKind::Square => Item::Rect {
                rect: Rect::new(x - 3.5, y - 3.5, 7.0, 7.0),
                radius: [1.0; 4],
                color,
            },
            MarkKind::Point => Item::Circle {
                x,
                y,
                r: DOT_R,
                fill: None,
                stroke: Some((1.5, color)),
            },
            _ => Item::Circle {
                x,
                y,
                r: DOT_R,
                fill: Some(color),
                stroke: Some((RING, theme.background)),
            },
        };
        cx.items.push(item);
        if !tips {
            continue;
        }
        let lines = if let Some(fields) = e.tooltip.as_ref().filter(|t| !t.is_empty()) {
            tip_fields(&chart.data, fields, &d.rows, yd, d.y)
        } else {
            vec![
                TipLine {
                    label: xd.auto_title(),
                    value: show(xd, &xcats, d.x, x_time),
                    color: None,
                },
                TipLine {
                    label: yd.auto_title(),
                    value: show(yd, &ycats, d.y, y_time),
                    color: None,
                },
            ]
        };
        cx.hovers.push(Hover {
            rect: Rect::new(x - DOT_HIT / 2.0, y - DOT_HIT / 2.0, DOT_HIT, DOT_HIT).intersect(&Rect::new(
                plot.x - 6.0,
                plot.y - 6.0,
                plot.w + 12.0,
                plot.h + 12.0,
            )),
            shape: None,
            title: series.get(d.s).cloned().unwrap_or_default(),
            lines,
            highlight: vec![Item::Circle {
                x,
                y,
                r: 5.0,
                fill: Some(color),
                stroke: Some((2.0, theme.background)),
            }],
        });
    }
    f.bottom
}

// -- Arcs ------------------------------------------------------------------------

/// Each slice's start and end angle, clockwise from twelve o'clock, in
/// radians. Values that aren't positive get no slice.
pub fn arc_angles(values: &[f64]) -> Vec<(f32, f32)> {
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    let mut at = 0.0f64;
    values
        .iter()
        .map(|&v| {
            let share = if total > 0.0 && v > 0.0 { v / total } else { 0.0 };
            let start = at;
            at += share * std::f64::consts::TAU;
            (start as f32, at as f32)
        })
        .collect()
}

/// A point at `angle` (clockwise from twelve o'clock) and `radius`.
fn polar(cx: f32, cy: f32, radius: f32, angle: f32) -> [f32; 2] {
    [cx + radius * angle.sin(), cy - radius * angle.cos()]
}

/// A slice's outline: along the outer edge, then back along the inner one
/// (or to the centre).
fn wedge(cx: f32, cy: f32, outer: f32, inner: f32, a0: f32, a1: f32) -> Vec<[f32; 2]> {
    let steps = (((a1 - a0) * outer / 3.0).ceil() as usize).clamp(2, 240);
    let mut pts: Vec<[f32; 2]> = (0..=steps)
        .map(|i| polar(cx, cy, outer, a0 + (a1 - a0) * i as f32 / steps as f32))
        .collect();
    if inner > 0.0 {
        pts.extend(
            (0..=steps)
                .rev()
                .map(|i| polar(cx, cy, inner, a0 + (a1 - a0) * i as f32 / steps as f32)),
        );
    } else {
        pts.push([cx, cy]);
    }
    pts
}

/// Pies and donuts, with the legend to the right when there's room, or
/// below.
fn arc(chart: &Chart, cx: &mut Ctx<'_>, top: f32) -> f32 {
    let e = &chart.encoding;
    let (Some(theta), Some(color)) = (&e.theta, &e.color) else {
        return top;
    };
    let theme = cx.theme.clone();
    let mut ix = Index::default();
    let mut sums: Vec<(Vec<f64>, Vec<usize>)> = Vec::new();
    let count = theta.aggregate == Some(crate::spec::Aggregate::Count);
    for (i, row) in chart.data.iter().enumerate() {
        let Some(name) = color.text(row) else { continue };
        let v = if count {
            1.0
        } else {
            match theta.number(row) {
                Some(v) => v,
                None => continue,
            }
        };
        let k = ix.of(name);
        if k == sums.len() {
            sums.push((Vec::new(), Vec::new()));
        }
        sums[k].0.push(v);
        sums[k].1.push(i);
    }
    let aggregate = theta.aggregate.unwrap_or(crate::spec::Aggregate::Sum);
    let values: Vec<f64> = sums.iter().map(|(v, _)| aggregate.apply(v)).collect();
    // Slices go in the color channel's order, or theta's when it sorts.
    let mut names = order(ix.names.clone(), Some(color), &|c| {
        ix.at.get(c).map_or(0.0, |&i| values[i])
    });
    match theta.sort {
        Sort::Descending | Sort::ByMeasure { descending: true } => {
            names.sort_by(|a, b| values[ix.at[b]].total_cmp(&values[ix.at[a]]));
        }
        Sort::Ascending | Sort::ByMeasure { descending: false } => {
            names.sort_by(|a, b| values[ix.at[a]].total_cmp(&values[ix.at[b]]));
        }
        _ => {}
    }
    let names: Vec<String> = names.into_iter().filter(|n| ix.at.contains_key(n)).collect();
    // Colours follow the slice's place in the color channel's own order,
    // so sorting by size doesn't repaint them.
    let color_order = order(ix.names.clone(), Some(color), &|_| 0.0);
    let color_of = |name: &str| theme.series(color_order.iter().position(|n| n == name).unwrap_or(0));
    let vals: Vec<f64> = names.iter().map(|n| values[ix.at[n]]).collect();
    let total: f64 = vals.iter().filter(|v| **v > 0.0).sum();
    let angles = arc_angles(&vals);

    // The legend's widest entry decides where it goes.
    let entries: Vec<(String, String, Rgba)> = names
        .iter()
        .zip(&vals)
        .map(|(n, v)| {
            let share = if total > 0.0 { v.max(0.0) / total } else { 0.0 };
            (n.clone(), format::percent(share), color_of(n))
        })
        .collect();
    let plot_h = chart.height.unwrap_or(crate::spec::DEFAULT_HEIGHT);
    let entry_w = |cx: &Ctx<'_>, (n, pct, _): &(String, String, Rgba)| {
        8.0 + 6.0
            + cx.text_w(n, SMALL_SIZE, Weight::Regular).min(LABEL_MAX)
            + 8.0
            + cx.text_w(pct, SMALL_SIZE, Weight::Regular)
    };
    let legend_w = entries.iter().map(|en| entry_w(cx, en)).fold(0.0, f32::max);
    let legend_h = entries.len() as f32 * (SMALL_H + 4.0);
    let width = cx.width;
    let side = width >= plot_h.min(width) + 32.0 + legend_w && entries.len() <= 12;
    let diameter = if side {
        plot_h.min(width - legend_w - 32.0)
    } else {
        plot_h.min(width)
    }
    .max(40.0);
    let outer = diameter / 2.0 - 2.0;
    let inner = chart.mark.inner_radius.map_or(0.0, |r| r.min(outer - 12.0).max(0.0));
    let (pcx, pcy) = (outer + 2.0, top + diameter / 2.0);

    let tips = e.tooltip.as_ref().is_none_or(|t| !t.is_empty());
    for (i, ((name, v), &(a0, a1))) in names.iter().zip(&vals).zip(&angles).enumerate() {
        if a1 - a0 <= 0.0 {
            continue;
        }
        let points = wedge(pcx, pcy, outer, inner, a0, a1);
        let c = color_of(name);
        cx.items.push(Item::Polygon {
            points: points.clone(),
            color: c,
        });
        if !tips {
            continue;
        }
        let rows = &sums[ix.at[name]].1;
        let lines = if let Some(fields) = e.tooltip.as_ref().filter(|t| !t.is_empty()) {
            tip_fields(&chart.data, fields, rows, theta, *v)
        } else {
            vec![TipLine {
                label: theta.auto_title(),
                value: format!("{} · {}", format::full(*v), entries[i].1),
                color: Some(c),
            }]
        };
        let (minx, maxx, miny, maxy) = points.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY),
            |(a, b, c, d), p| (a.min(p[0]), b.max(p[0]), c.min(p[1]), d.max(p[1])),
        );
        cx.hovers.push(Hover {
            rect: Rect::new(minx, miny, maxx - minx, maxy - miny),
            shape: Some(points.clone()),
            title: name.clone(),
            lines,
            highlight: vec![Item::Polygon {
                points,
                color: theme.hover,
            }],
        });
    }
    // Surface-coloured gaps between slices.
    let live: Vec<&(f32, f32)> = angles.iter().filter(|(a, b)| b > a).collect();
    if live.len() > 1 {
        for &&(a0, _) in &live {
            let from = if inner > 0.0 {
                polar(pcx, pcy, inner - 1.0, a0)
            } else {
                [pcx, pcy]
            };
            cx.items.push(Item::Line {
                points: vec![from, polar(pcx, pcy, outer + 1.0, a0)],
                width: GAP,
                color: theme.background,
            });
        }
    }
    // The total in a donut's hole.
    if inner >= 34.0 && total > 0.0 {
        cx.text(
            pcx,
            pcy - 7.0,
            format::compact(total),
            18.0,
            theme.label_text,
            Anchor::Middle,
            Weight::Semibold,
        );
        cx.text(
            pcx,
            pcy + 11.0,
            "Total".into(),
            SMALL_SIZE,
            theme.axis_text,
            Anchor::Middle,
            Weight::Regular,
        );
    }

    // The legend, with each slice's share.
    let (lx, ly) = if side {
        (diameter + 32.0, top + (diameter - legend_h).max(0.0) / 2.0)
    } else {
        (0.0, top + diameter + 12.0)
    };
    let mut y = ly + SMALL_H / 2.0;
    let mut x = lx;
    for en in &entries {
        let w = entry_w(cx, en);
        if !side && x > 0.0 && x + w > width {
            x = 0.0;
            y += SMALL_H + 4.0;
        }
        let (name, pct, c) = en;
        swatch(cx, Key::Square, x, y, *c);
        let text = cx.fit(name, SMALL_SIZE, Weight::Regular, LABEL_MAX);
        let tx = x + 14.0;
        let name_w = cx.text_w(&text, SMALL_SIZE, Weight::Regular);
        cx.text(
            tx,
            y,
            text,
            SMALL_SIZE,
            theme.label_text,
            Anchor::Start,
            Weight::Regular,
        );
        cx.items.push(Item::Text(Text {
            x: tx + name_w + 8.0,
            y,
            text: pct.clone(),
            size: SMALL_SIZE,
            color: theme.axis_text,
            anchor: Anchor::Start,
            weight: Weight::Regular,
            tabular: true,
        }));
        if side {
            y += SMALL_H + 4.0;
        } else {
            x += w + 16.0;
        }
    }
    let legend_bottom = y + SMALL_H / 2.0;
    if side {
        (top + diameter).max(legend_bottom) + 2.0
    } else {
        legend_bottom + 2.0
    }
}

// -- Keeping inside the bounds ----------------------------------------------------

/// Clips and nudges everything inside 0..width × 0..height: rectangles cut
/// to it, points held in it, and text shifted (or cut) to fit.
fn confine(scene: &mut Scene, measure: Measure<'_>) {
    let bounds = Rect::new(0.0, 0.0, scene.width, scene.height);
    for item in &mut scene.items {
        confine_item(item, &bounds, measure);
    }
    scene
        .items
        .retain(|i| !matches!(i, Item::Rect { rect, .. } if rect.w <= 0.0 || rect.h <= 0.0));
    for hover in &mut scene.hovers {
        hover.rect = hover.rect.intersect(&bounds);
        for item in &mut hover.highlight {
            confine_item(item, &bounds, measure);
        }
    }
    scene.hovers.retain(|h| h.rect.w > 0.0 && h.rect.h > 0.0);
}

fn confine_item(item: &mut Item, b: &Rect, measure: Measure<'_>) {
    let clamp = |p: &mut [f32; 2]| {
        p[0] = p[0].clamp(b.x, b.right());
        p[1] = p[1].clamp(b.y, b.bottom());
    };
    match item {
        Item::Rect { rect, .. } => *rect = rect.intersect(b),
        Item::Line { points, width, .. } => {
            let half = *width / 2.0;
            for p in points {
                p[0] = p[0].clamp(b.x + half, b.right() - half);
                p[1] = p[1].clamp(b.y + half, b.bottom() - half);
            }
        }
        Item::Polygon { points, .. } => points.iter_mut().for_each(clamp),
        Item::Circle { x, y, r, stroke, .. } => {
            let reach = *r + stroke.map_or(0.0, |s| s.0);
            *x = x.clamp(b.x + reach, (b.right() - reach).max(b.x + reach));
            *y = y.clamp(b.y + reach, (b.bottom() - reach).max(b.y + reach));
        }
        Item::Text(t) => {
            let mut w = measure(&t.text, t.size) * if t.weight == Weight::Semibold { 1.05 } else { 1.0 };
            if w > b.w {
                t.text = fit(&t.text, b.w, &|s| measure(s, t.size));
                w = measure(&t.text, t.size);
            }
            let left = match t.anchor {
                Anchor::Start => t.x,
                Anchor::Middle => t.x - w / 2.0,
                Anchor::End => t.x - w,
            };
            let shift = if left < b.x {
                b.x - left
            } else if left + w > b.right() {
                b.right() - (left + w)
            } else {
                0.0
            };
            t.x += shift;
            let half = t.size * 0.6;
            t.y = t.y.clamp(b.y + half, (b.bottom() - half).max(b.y + half));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{PartKind, parse_visual};
    use serde_json::{Value, json};

    /// About the system font's widths.
    fn measure(text: &str, size: f32) -> f32 {
        text.chars().count() as f32 * size * 0.55
    }

    #[allow(clippy::needless_pass_by_value, reason = "tests read best with json! in place")]
    fn chart(spec: Value) -> Chart {
        let v = parse_visual(&spec).unwrap_or_else(|e| panic!("{e}"));
        match v.rows.into_iter().next().map(|p| p.kind) {
            Some(PartKind::Chart(c)) => *c,
            _ => panic!("a chart"),
        }
    }

    fn lay(spec: Value, width: f32) -> Scene {
        chart(spec).layout(width, &VisualTheme::light(), &measure)
    }

    /// Bars (rects) painted in series `i`'s colour, legend swatches left out.
    fn bars_of(scene: &Scene, i: usize) -> Vec<Rect> {
        let c = VisualTheme::light().series(i);
        scene
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Rect { rect, color, radius }
                    if *color == c && rect.w > 2.0 && rect.h > 2.0 && *radius != [2.0; 4] =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect()
    }

    fn texts(scene: &Scene) -> Vec<&Text> {
        scene
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn has_text(scene: &Scene, s: &str) -> bool {
        texts(scene).iter().any(|t| t.text == s)
    }

    /// Every item inside the scene's bounds.
    fn assert_inside(scene: &Scene) {
        let (w, h) = (scene.width + 0.01, scene.height + 0.01);
        let ok = |x: f32, y: f32| x >= -0.01 && y >= -0.01 && x <= w && y <= h;
        for item in &scene.items {
            match item {
                Item::Rect { rect, .. } => assert!(ok(rect.x, rect.y) && ok(rect.right(), rect.bottom()), "{item:?}"),
                Item::Line { points, .. } | Item::Polygon { points, .. } => {
                    assert!(points.iter().all(|p| ok(p[0], p[1])), "{item:?}");
                }
                Item::Circle { x, y, r, .. } => assert!(ok(x - r, y - r) && ok(x + r, y + r), "{item:?}"),
                Item::Text(t) => {
                    let tw = measure(&t.text, t.size);
                    let left = match t.anchor {
                        Anchor::Start => t.x,
                        Anchor::Middle => t.x - tw / 2.0,
                        Anchor::End => t.x - tw,
                    };
                    assert!(ok(left, t.y) && ok(left + tw, t.y), "{t:?} in {w}");
                }
            }
        }
    }

    /// No two tick labels on one line overlap.
    fn assert_no_overlap(scene: &Scene) {
        let mut spans: Vec<(f32, f32, f32)> = texts(scene)
            .iter()
            .filter(|t| t.tabular && t.anchor == Anchor::Middle)
            .map(|t| {
                let w = measure(&t.text, t.size);
                (t.y, t.x - w / 2.0, t.x + w / 2.0)
            })
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        for w in spans.windows(2) {
            if (w[0].0 - w[1].0).abs() < 1.0 {
                assert!(w[0].2 <= w[1].1, "labels overlap: {w:?}");
            }
        }
    }

    fn grouped() -> Value {
        json!({
            "mark": "bar",
            "data": {"values": [
                {"q": "Q1", "team": "iOS", "n": 10}, {"q": "Q1", "team": "Web", "n": 20},
                {"q": "Q2", "team": "iOS", "n": 30}, {"q": "Q2", "team": "Web", "n": 40}
            ]},
            "encoding": {
                "x": {"field": "q", "type": "nominal"},
                "y": {"field": "n", "type": "quantitative"},
                "xOffset": {"field": "team"},
                "color": {"field": "team"}
            }
        })
    }

    #[test]
    fn grouped_bars_sit_side_by_side_in_their_band() {
        let s = lay(grouped(), 400.0);
        let (ios, web) = (bars_of(&s, 0), bars_of(&s, 1));
        assert_eq!((ios.len(), web.len()), (2, 2));
        for (a, b) in ios.iter().zip(&web) {
            // Same width, at most 24, 2 px apart, iOS on the left.
            assert_eq!(a.w, b.w);
            assert!(a.w <= BAR_MAX);
            assert!((b.x - a.right() - GAP).abs() < 0.01, "{a:?} {b:?}");
        }
        // Each pair is centred on its band: Q1's in the left half, Q2's in the right.
        let plot_mid = (ios[0].x + web[1].right()) / 2.0;
        let pair_mid = |a: &Rect, b: &Rect| (a.x + b.right()) / 2.0;
        assert!(pair_mid(&ios[0], &web[0]) < plot_mid && pair_mid(&ios[1], &web[1]) > plot_mid);
        let band = pair_mid(&ios[1], &web[1]) - pair_mid(&ios[0], &web[0]);
        assert!(band > 100.0);
        // Heights follow the values: 40 is four times 10.
        assert!((web[1].h / ios[0].h - 4.0).abs() < 0.05);
        // A legend names both teams.
        assert!(has_text(&s, "iOS") && has_text(&s, "Web"));
        assert_inside(&s);
    }

    #[test]
    fn bars_stack_by_color_without_an_offset() {
        let mut spec = grouped();
        spec["encoding"].as_object_mut().unwrap().shift_remove("xOffset");
        let s = lay(spec, 400.0);
        let (ios, web) = (bars_of(&s, 0), bars_of(&s, 1));
        for (a, b) in ios.iter().zip(&web) {
            // The same column; Web on top, 2 px above iOS.
            assert_eq!(a.x, b.x);
            assert!((a.y - b.bottom() - GAP).abs() < 0.01, "{a:?} {b:?}");
        }
        // Only the top segment has rounded corners.
        let radius = |r: &Rect| {
            s.items.iter().find_map(|it| match it {
                Item::Rect { rect, radius, .. } if rect == r => Some(*radius),
                _ => None,
            })
        };
        assert_eq!(radius(&ios[0]), Some([0.0; 4]));
        assert_eq!(radius(&web[0]).map(|r| r[0]), Some(END_RADIUS));
        // Q2's stack (70) is taller than Q1's (30).
        assert!(ios[1].bottom() - web[1].y > ios[0].bottom() - web[0].y);
        assert_inside(&s);
    }

    #[test]
    fn normalized_stacks_fill_the_plot() {
        let mut spec = grouped();
        spec["encoding"].as_object_mut().unwrap().shift_remove("xOffset");
        spec["encoding"]["y"]["stack"] = json!("normalize");
        let s = lay(spec, 400.0);
        let (ios, web) = (bars_of(&s, 0), bars_of(&s, 1));
        let full = ios[0].bottom() - web[0].y;
        assert!((ios[1].bottom() - web[1].y - full).abs() < 0.5);
        assert!(has_text(&s, "100%") && has_text(&s, "40%"));
        // Q1: iOS is a third of the stack.
        assert!(((ios[0].h + GAP / 2.0) / full - 1.0 / 3.0).abs() < 0.02);
    }

    #[test]
    fn horizontal_bars_grow_from_the_left() {
        let s = lay(
            json!({"mark": "bar", "data": {"values": [{"lang": "Rust", "loc": 1200}, {"lang": "TypeScript", "loc": 300}]},
                "encoding": {"y": {"field": "lang", "type": "nominal"}, "x": {"field": "loc", "type": "quantitative"}}}),
            360.0,
        );
        let bars = bars_of(&s, 0);
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].x, bars[1].x);
        assert!((bars[0].w / bars[1].w - 4.0).abs() < 0.05);
        assert!(bars[0].y < bars[1].y);
        // Category labels end left of the bars.
        let rust = texts(&s).into_iter().find(|t| t.text == "Rust").unwrap();
        assert_eq!(rust.anchor, Anchor::End);
        assert!(rust.x < bars[0].x);
        assert!(has_text(&s, "1.2k"));
        assert_inside(&s);
    }

    #[test]
    fn duplicate_keys_are_summed_and_aggregates_apply() {
        let base = json!({"mark": "bar", "data": {"values": [
            {"k": "a", "v": 1}, {"k": "a", "v": 3}, {"k": "b", "v": 2}
        ]}, "encoding": {"x": {"field": "k"}, "y": {"field": "v"}}});
        let s = lay(base.clone(), 300.0);
        let b = bars_of(&s, 0);
        assert_eq!(b.len(), 2);
        assert!((b[0].h / b[1].h - 2.0).abs() < 0.02, "a sums to 4, b is 2");
        let mut mean = base;
        mean["encoding"]["y"]["aggregate"] = json!("mean");
        let s = lay(mean, 300.0);
        let b = bars_of(&s, 0);
        assert!((b[0].h - b[1].h).abs() < 0.01, "both mean to 2");
        assert!(has_text(&s, "Mean of v"));
        let count = json!({"mark": "bar", "data": {"values": [{"k": "a"}, {"k": "a"}, {"k": "b"}]},
            "encoding": {"x": {"field": "k"}, "y": {"aggregate": "count"}}});
        let s = lay(count, 300.0);
        let b = bars_of(&s, 0);
        assert!((b[0].h / b[1].h - 2.0).abs() < 0.02);
        // Counts tick in whole numbers.
        assert!(!texts(&s).iter().any(|t| t.text.contains('.')));
    }

    #[test]
    fn sort_orders_the_categories() {
        let spec = |sort: Value| {
            json!({"mark": "bar", "data": {"values": [{"k": "b", "v": 1}, {"k": "c", "v": 3}, {"k": "a", "v": 2}]},
                "encoding": {"x": {"field": "k", "sort": sort}, "y": {"field": "v"}}})
        };
        let order = |s: &Scene| {
            let mut ts: Vec<&Text> = texts(s)
                .into_iter()
                .filter(|t| ["a", "b", "c"].contains(&t.text.as_str()))
                .collect();
            ts.sort_by(|a, b| a.x.total_cmp(&b.x));
            ts.iter().map(|t| t.text.clone()).collect::<Vec<_>>()
        };
        assert_eq!(order(&lay(spec(Value::Null), 300.0)), ["b", "c", "a"]);
        assert_eq!(order(&lay(spec(json!("ascending")), 300.0)), ["a", "b", "c"]);
        assert_eq!(order(&lay(spec(json!("descending")), 300.0)), ["c", "b", "a"]);
        assert_eq!(order(&lay(spec(json!("-y")), 300.0)), ["c", "a", "b"]);
        assert_eq!(order(&lay(spec(json!(["a", "c"])), 300.0)), ["a", "c", "b"]);
    }

    #[test]
    fn zero_and_domain() {
        let spec = |scale: Value| {
            json!({"mark": "line", "data": {"values": [{"x": 1, "y": 96}, {"x": 2, "y": 104}]},
                "encoding": {"x": {"field": "x"}, "y": {"field": "y", "scale": scale}}})
        };
        let s = lay(spec(Value::Null), 300.0);
        assert!(has_text(&s, "0"), "the y axis starts at zero");
        let s = lay(spec(json!({"zero": false})), 300.0);
        assert!(!has_text(&s, "0") && has_text(&s, "100"));
        let s = lay(spec(json!({"domain": [90, 110]})), 300.0);
        assert!(has_text(&s, "90") && has_text(&s, "110"));
    }

    #[test]
    fn lines_by_series_with_points() {
        let s = lay(
            json!({"mark": {"type": "line", "point": true}, "data": {"values": [
                {"d": "Mon", "s": "p50", "ms": 10}, {"d": "Tue", "s": "p50", "ms": 12}, {"d": "Wed", "s": "p50", "ms": 11},
                {"d": "Mon", "s": "p95", "ms": 40}, {"d": "Tue", "s": "p95", "ms": 55}, {"d": "Wed", "s": "p95", "ms": 48}
            ]}, "encoding": {"x": {"field": "d"}, "y": {"field": "ms"}, "color": {"field": "s"}}}),
            420.0,
        );
        let lines: Vec<&Item> = s.items.iter().filter(|i| matches!(i, Item::Line { .. })).collect();
        assert_eq!(lines.len(), 2);
        let dots = s.items.iter().filter(|i| matches!(i, Item::Circle { .. })).count();
        assert_eq!(dots, 6);
        // One hover column per day, reading out both series.
        assert_eq!(s.hovers.len(), 3);
        assert_eq!(s.hovers[1].title, "Tue");
        assert_eq!(
            s.hovers[1].lines.iter().map(|l| l.value.as_str()).collect::<Vec<_>>(),
            ["12", "55"]
        );
        let h = &s.hovers[2];
        let found = s
            .hover_at(h.rect.x + h.rect.w / 2.0, h.rect.y + 5.0)
            .map(|h| h.title.as_str());
        assert_eq!(found, Some("Wed"));
        assert_inside(&s);
    }

    #[test]
    fn stacked_areas_close_over_the_series_below() {
        let s = lay(
            json!({"mark": "area", "data": {"values": [
                {"t": "2026-10-01", "k": "a", "v": 1}, {"t": "2026-10-02", "k": "a", "v": 2},
                {"t": "2026-10-01", "k": "b", "v": 3}, {"t": "2026-10-02", "k": "b", "v": 1}
            ]}, "encoding": {"x": {"field": "t", "type": "temporal"}, "y": {"field": "v"}, "color": {"field": "k"}}}),
            400.0,
        );
        let polys: Vec<&Vec<[f32; 2]>> = s
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Polygon { points, .. } => Some(points),
                _ => None,
            })
            .collect();
        assert_eq!(polys.len(), 2);
        // b's bottom edge is a's top edge.
        let a_top = &polys[0][..2];
        let b_bottom: Vec<[f32; 2]> = polys[1][2..].iter().rev().copied().collect();
        assert_eq!(a_top, b_bottom.as_slice());
        assert!(has_text(&s, "Oct 1") && has_text(&s, "Oct 2"));
        assert_inside(&s);
    }

    #[test]
    fn pie_slices_cover_the_circle() {
        let angles = arc_angles(&[1.0, 2.0, 3.0, 0.0, -1.0]);
        let covered: f32 = angles.iter().map(|(a, b)| b - a).sum();
        assert!((covered - std::f32::consts::TAU).abs() < 1e-5);
        assert_eq!(angles[3].0, angles[3].1);
        assert!((angles[1].1 - angles[1].0 - std::f32::consts::TAU / 3.0).abs() < 1e-5);

        let s = lay(
            json!({"mark": {"type": "arc", "innerRadius": 50}, "data": {"values": [
                {"k": "Rust", "v": 60}, {"k": "Swift", "v": 30}, {"k": "Go", "v": 10}
            ]}, "encoding": {"theta": {"field": "v"}, "color": {"field": "k"}}}),
            420.0,
        );
        let polys = s.items.iter().filter(|i| matches!(i, Item::Polygon { .. })).count();
        assert_eq!(polys, 3);
        assert!(has_text(&s, "60%") && has_text(&s, "Total") && has_text(&s, "100"));
        assert_eq!(s.hovers.len(), 3);
        // The hover follows the wedge, not its box.
        let rust = &s.hovers[0];
        assert_eq!(rust.title, "Rust");
        assert!(rust.lines[0].value.ends_with("60%"));
        assert_inside(&s);
    }

    #[test]
    fn many_labels_are_thinned_not_overlapped() {
        let values: Vec<Value> = (0..40).map(|i| json!({"k": format!("Category {i}"), "v": i})).collect();
        let s = lay(
            json!({"mark": "bar", "data": {"values": values}, "encoding": {"x": {"field": "k"}, "y": {"field": "v"}}}),
            360.0,
        );
        let shown = texts(&s).iter().filter(|t| t.text.starts_with("Category")).count();
        assert!(shown > 1 && shown < 40, "{shown}");
        assert_no_overlap(&s);
        assert_inside(&s);

        let days: Vec<Value> = (1..=31)
            .map(|d| json!({"t": format!("2026-10-{d:02}"), "v": d}))
            .collect();
        let s = lay(
            json!({"mark": "line", "data": {"values": days}, "encoding": {"x": {"field": "t", "type": "temporal"}, "y": {"field": "v"}}}),
            300.0,
        );
        assert_no_overlap(&s);
        assert_inside(&s);
    }

    #[test]
    fn axis_null_hides_labels_and_titles() {
        let s = lay(
            json!({"mark": "bar", "data": {"values": [{"k": "a", "v": 5}]},
                "encoding": {"x": {"field": "k", "axis": null}, "y": {"field": "v", "axis": {"title": null}}}}),
            300.0,
        );
        assert!(!has_text(&s, "a") && !has_text(&s, "k") && !has_text(&s, "v"));
        assert!(has_text(&s, "5"));
    }

    #[test]
    fn one_series_has_no_legend() {
        let s = lay(
            json!({"mark": "line", "data": {"values": [{"x": 1, "y": 1, "s": "only"}, {"x": 2, "y": 2, "s": "only"}]},
                "encoding": {"x": {"field": "x"}, "y": {"field": "y"}, "color": {"field": "s"}}}),
            300.0,
        );
        assert!(!has_text(&s, "only"));
    }

    #[test]
    fn hovering_a_bar_reads_its_value() {
        let s = lay(grouped(), 400.0);
        let b = bars_of(&s, 1)[1];
        let h = s.hover_at(b.x + b.w / 2.0, b.y + b.h / 2.0).expect("a hover");
        assert_eq!(h.title, "Q2");
        assert_eq!(h.lines[0].label, "Web");
        assert_eq!(h.lines[0].value, "40");
    }

    #[test]
    fn tooltip_fields_replace_the_default() {
        let s = lay(
            json!({"mark": "point", "data": {"values": [{"x": 1, "y": 2, "who": "Ann", "at": "2026-10-03"}]},
                "encoding": {"x": {"field": "x"}, "y": {"field": "y"},
                    "tooltip": [{"field": "who"}, {"field": "at", "type": "temporal", "title": "When"}]}}),
            300.0,
        );
        let lines = &s.hovers[0].lines;
        assert_eq!(lines[0].label, "who");
        assert_eq!(lines[0].value, "Ann");
        assert_eq!(
            (lines[1].label.as_str(), lines[1].value.as_str()),
            ("When", "Oct 3, 2026")
        );
        assert_inside(&s);
    }

    #[test]
    fn bars_on_a_time_axis_stay_inside() {
        let days: Vec<Value> = (1..=14)
            .map(|d| json!({"t": format!("2026-10-{d:02}"), "v": d * 3}))
            .collect();
        let s = lay(
            json!({"mark": "bar", "title": "Deploys per day", "data": {"values": days},
                "encoding": {"x": {"field": "t", "type": "temporal"}, "y": {"field": "v"}}}),
            320.0,
        );
        let bars = bars_of(&s, 0);
        assert_eq!(bars.len(), 14);
        assert!(bars.windows(2).all(|w| w[1].x - w[0].right() > 1.0), "bars don't touch");
        assert!(has_text(&s, "Deploys per day"));
        assert_inside(&s);
        assert_no_overlap(&s);
    }

    #[test]
    fn a_narrow_chart_keeps_everything_in_bounds() {
        for spec in [
            grouped(),
            json!({"mark": "arc", "title": "A long title that will not fit in a narrow chart at all",
            "data": {"values": [{"k": "A very long category name indeed", "v": 1}, {"k": "b", "v": 2}]},
            "encoding": {"theta": {"field": "v"}, "color": {"field": "k"}}}),
        ] {
            let s = lay(spec, 120.0);
            assert_inside(&s);
        }
    }

    #[test]
    fn labels_are_cut_to_fit() {
        let wide = |t: &str| measure(t, 11.0);
        assert_eq!(fit("short", 100.0, &wide), "short");
        let cut = fit("a rather long label here", 60.0, &wide);
        assert!(cut.ends_with('…') && wide(&cut) <= 60.0, "{cut}");
        assert_eq!(fit("abc", 1.0, &wide), "");
    }
}
