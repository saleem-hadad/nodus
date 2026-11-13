//! The visual grammar Claude writes, read into types.
//!
//! A visual is one chart (an object with `"mark"`) or a composition:
//! `{"title"?, "rows": [part, …]}`, drawn top to bottom. A part is a chart,
//! a row of stat tiles, a table, Markdown, or parts side by side
//! (`"columns"`). Charts are a subset of Vega-Lite: one mark, inline data,
//! and the channels x, y, color, theta, xOffset and tooltip.
//!
//! Errors say what is wrong and where (`rows[1].chart.encoding.y: field "ms"
//! isn't in data.values`), because Claude reads them and fixes its JSON.
//! Anything outside the grammar that would change what the data means
//! (`layer`, `transform`, `bin`, `data.url`…) is refused by name; styling
//! the layout decides for itself (`width`, `config`, axis label angles) is
//! ignored.

use std::fmt::Display;

use serde_json::{Map, Value};

use crate::format;
use crate::time;

/// One row of a chart's data.
pub type Row = Map<String, Value>;

/// Charts a visual can hold at most this many rows of data each.
pub const MAX_ROWS: usize = 5000;
/// Series a chart can tell apart: one per palette colour.
pub const MAX_SERIES: usize = 8;
/// Categories along one axis.
pub const MAX_CATEGORIES: usize = 100;
/// A chart's plot height when the spec gives none.
pub const DEFAULT_HEIGHT: f32 = 220.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Visual {
    pub title: Option<String>,
    pub rows: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub title: Option<String>,
    pub kind: PartKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartKind {
    Chart(Box<Chart>),
    Stats(Vec<Stat>),
    Table(Table),
    /// Markdown.
    Text(String),
    /// Parts side by side.
    Columns(Vec<Part>),
}

/// A stat tile: a big value, what it is, and a note under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stat {
    pub label: String,
    pub value: String,
    pub note: Option<String>,
    pub tone: Tone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Bad,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Header labels; empty when the table has no header row.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

impl Table {
    /// How many columns the widest row has.
    pub fn width(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .chain([self.columns.len()])
            .max()
            .unwrap_or(0)
    }

    /// Whether column `i` holds numbers (and blanks) only, to right-align it.
    pub fn numeric(&self, i: usize) -> bool {
        let mut any = false;
        for row in &self.rows {
            match row.get(i) {
                Some(c) if c.number => any = true,
                Some(c) if !c.text.is_empty() => return false,
                _ => {}
            }
        }
        any
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    /// Written as a JSON number.
    pub number: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub title: Option<String>,
    /// The plot's height; `None` lets the layout choose (220, or by the
    /// number of bars for horizontal bars).
    pub height: Option<f32>,
    pub mark: Mark,
    pub data: Vec<Row>,
    pub encoding: Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    /// Dots on a line's points.
    pub point: bool,
    /// A donut's hole, in px.
    pub inner_radius: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Bar,
    Line,
    Area,
    /// A hollow dot.
    Point,
    Circle,
    Square,
    Arc,
}

impl MarkKind {
    fn name(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Line => "line",
            Self::Area => "area",
            Self::Point => "point",
            Self::Circle => "circle",
            Self::Square => "square",
            Self::Arc => "arc",
        }
    }

    /// Dots of some kind, which neither stack nor sum duplicates.
    pub fn is_dot(self) -> bool {
        matches!(self, Self::Point | Self::Circle | Self::Square)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Encoding {
    pub x: Option<FieldDef>,
    pub y: Option<FieldDef>,
    pub color: Option<FieldDef>,
    pub theta: Option<FieldDef>,
    pub x_offset: Option<FieldDef>,
    pub tooltip: Option<Vec<FieldDef>>,
}

/// A channel: which field, of what type, and how to show it.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    /// `None` only for `"aggregate": "count"`.
    pub field: Option<String>,
    pub kind: FieldType,
    pub aggregate: Option<Aggregate>,
    pub title: Option<String>,
    pub sort: Sort,
    pub axis: AxisSpec,
    /// `"legend": null` hides a color channel's legend.
    pub legend: bool,
    pub domain: Option<Domain>,
    pub zero: Option<bool>,
    /// `None` leaves stacking to the mark.
    pub stack: Option<Stack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Quantitative,
    Nominal,
    Ordinal,
    Temporal,
}

impl FieldType {
    pub fn is_discrete(self) -> bool {
        matches!(self, Self::Nominal | Self::Ordinal)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Quantitative => "quantitative",
            Self::Nominal => "nominal",
            Self::Ordinal => "ordinal",
            Self::Temporal => "temporal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aggregate {
    Sum,
    Mean,
    Count,
    Min,
    Max,
    Median,
}

impl Aggregate {
    /// The aggregate of some values (`Count` counts them).
    pub fn apply(self, values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        match self {
            Self::Sum => values.iter().sum(),
            Self::Count => values.len() as f64,
            Self::Mean => values.iter().sum::<f64>() / values.len() as f64,
            Self::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
            Self::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Self::Median => {
                let mut v = values.to_vec();
                v.sort_by(f64::total_cmp);
                let mid = v.len() / 2;
                if v.len().is_multiple_of(2) {
                    (v[mid - 1] + v[mid]) / 2.0
                } else {
                    v[mid]
                }
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sum => "Sum",
            Self::Mean => "Mean",
            Self::Count => "Count",
            Self::Min => "Min",
            Self::Max => "Max",
            Self::Median => "Median",
        }
    }
}

/// The order of a channel's categories.
#[derive(Debug, Clone, PartialEq)]
pub enum Sort {
    /// As they first appear in the data.
    Data,
    Ascending,
    Descending,
    /// These first, in this order; the rest after, as they appear.
    Values(Vec<String>),
    /// By the chart's measure (`"-y"`, or `{"op": "sum", "order": …}`).
    ByMeasure {
        descending: bool,
    },
}

/// Whether a channel's axis shows, and its title.
#[derive(Debug, Clone, PartialEq)]
pub enum AxisSpec {
    /// Shown, titled after the field.
    Auto,
    /// `"axis": null`: no labels and no title.
    Hidden,
    /// `"axis": {"title": null}`: labels without a title.
    Untitled,
    Titled(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Domain {
    /// A quantitative or temporal axis's ends (times in epoch ms).
    Range(f64, f64),
    /// A discrete axis's categories, in order.
    Values(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stack {
    Zero,
    Normalize,
    Off,
}

impl FieldDef {
    /// The axis or legend title: the axis's own, the channel's `title`, or
    /// the field (with its aggregate: "Mean of ms", "Count").
    pub fn auto_title(&self) -> String {
        if let Some(t) = &self.title {
            return t.clone();
        }
        match (&self.aggregate, &self.field) {
            (Some(Aggregate::Count), _) | (_, None) => "Count".into(),
            (Some(a @ (Aggregate::Mean | Aggregate::Min | Aggregate::Max | Aggregate::Median)), Some(f)) => {
                format!("{} of {f}", a.label())
            }
            (_, Some(f)) => f.clone(),
        }
    }

    /// The axis title to show, if any.
    pub fn axis_title(&self) -> Option<String> {
        match &self.axis {
            AxisSpec::Auto => Some(self.auto_title()),
            AxisSpec::Titled(t) => Some(t.clone()),
            AxisSpec::Hidden | AxisSpec::Untitled => None,
        }
    }

    pub fn shows_axis(&self) -> bool {
        self.axis != AxisSpec::Hidden
    }

    /// This channel's value in a row, as text (a category).
    pub fn text(&self, row: &Row) -> Option<String> {
        let v = self.value(row)?;
        Some(match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format::plain),
            Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        })
    }

    /// This channel's raw value in a row; `None` when missing or null.
    pub fn value<'a>(&self, row: &'a Row) -> Option<&'a Value> {
        row.get(self.field.as_deref()?).filter(|v| !v.is_null())
    }

    /// This channel's value in a row as a number: a number, or a string
    /// that reads as one; a time for a temporal channel.
    pub fn number(&self, row: &Row) -> Option<f64> {
        let v = self.value(row)?;
        if self.kind == FieldType::Temporal {
            return time::parse(v);
        }
        number_of(v)
    }
}

/// A JSON number, or a string that reads as one.
pub fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

// -- Errors ------------------------------------------------------------------

/// `path.key`, or `key` at the top.
fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn index(path: &str, i: usize) -> String {
    format!("{path}[{i}]")
}

/// `path: message`, or the message alone at the top.
fn fail<T>(path: &str, message: impl Display) -> Result<T, String> {
    Err(if path.is_empty() {
        message.to_string()
    } else {
        format!("{path}: {message}")
    })
}

/// A JSON value in an error message, cut short.
fn show(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 40 {
        let cut: String = s.chars().take(37).collect();
        format!("{cut}…")
    } else {
        s
    }
}

/// A list for a message: `a, b or c`.
fn either(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

fn get_string(obj: &Map<String, Value>, key: &str, path: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => fail(&join(path, key), format!("must be a string, not {}", show(other))),
    }
}

fn get_bool(obj: &Map<String, Value>, key: &str, path: &str) -> Result<Option<bool>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => fail(&join(path, key), format!("must be true or false, not {}", show(other))),
    }
}

/// Vega-Lite's views that hold several charts.
const MULTI_VIEW: [&str; 7] = ["layer", "hconcat", "vconcat", "concat", "facet", "repeat", "spec"];

fn multi_view_error<T>(path: &str, key: &str) -> Result<T, String> {
    fail(
        &join(path, key),
        "isn't supported: put each chart in its own part of \"rows\", or side by side in {\"columns\": [...]}",
    )
}

// -- Visuals and parts ---------------------------------------------------------

const PART_KEYS: [&str; 5] = ["chart", "stats", "table", "text", "columns"];

/// Reads a visual: one chart, or a composition of parts.
pub fn parse_visual(spec: &Value) -> Result<Visual, String> {
    let Some(obj) = spec.as_object() else {
        return fail(
            "",
            format!(
                "a visual is a JSON object: one chart with \"mark\", or {{\"rows\": [...]}} (got {})",
                show(spec)
            ),
        );
    };
    if obj.contains_key("mark") {
        let chart = parse_chart(spec, "")?;
        return Ok(Visual {
            title: None,
            rows: vec![Part {
                title: None,
                kind: PartKind::Chart(Box::new(chart)),
            }],
        });
    }
    if let Some(rows) = obj.get("rows") {
        for key in obj.keys() {
            if key == "rows" || key == "title" || key == "$schema" {
                continue;
            }
            if MULTI_VIEW.contains(&key.as_str()) {
                return multi_view_error("", key);
            }
            return fail(
                key,
                "isn't a key of a visual: a composition has \"title\" and \"rows\" (put charts, stats, tables and text in the rows)",
            );
        }
        let title = get_string(obj, "title", "")?;
        let Some(rows) = rows.as_array() else {
            return fail("rows", "must be an array of parts");
        };
        if rows.is_empty() {
            return fail("rows", "is empty: give at least one part");
        }
        let rows = rows
            .iter()
            .enumerate()
            .map(|(i, p)| parse_part(p, &index("rows", i)))
            .collect::<Result<_, _>>()?;
        return Ok(Visual { title, rows });
    }
    if let Some(key) = MULTI_VIEW.iter().find(|k| obj.contains_key(**k)) {
        return multi_view_error("", key);
    }
    // A lone part: a table, stats or text on its own.
    if PART_KEYS.iter().any(|k| obj.contains_key(*k)) {
        let part = parse_part(spec, "")?;
        return Ok(Visual {
            title: None,
            rows: vec![part],
        });
    }
    fail(
        "",
        "a visual needs \"mark\" (one chart) or \"rows\" (charts, stats, tables and text laid out together)",
    )
}

fn parse_part(v: &Value, path: &str) -> Result<Part, String> {
    let Some(obj) = v.as_object() else {
        return fail(
            path,
            format!(
                "a part is an object with one of \"chart\", \"stats\", \"table\", \"text\" or \"columns\" (got {})",
                show(v)
            ),
        );
    };
    // A chart written straight into the rows.
    if obj.contains_key("mark") {
        return Ok(Part {
            title: None,
            kind: PartKind::Chart(Box::new(parse_chart(v, path)?)),
        });
    }
    for key in obj.keys() {
        if key != "title" && !PART_KEYS.contains(&key.as_str()) {
            if MULTI_VIEW.contains(&key.as_str()) {
                return multi_view_error(path, key);
            }
            return fail(
                &join(path, key),
                "isn't a part key (a part has \"title\" and one of chart, stats, table, text or columns)",
            );
        }
    }
    let present: Vec<&str> = PART_KEYS.iter().copied().filter(|k| obj.contains_key(*k)).collect();
    let key = match present.as_slice() {
        [one] => *one,
        [] => {
            return fail(
                path,
                "a part needs one of \"chart\", \"stats\", \"table\", \"text\" or \"columns\"",
            );
        }
        [a, b, ..] => {
            return fail(
                path,
                format!("a part holds one thing, not both \"{a}\" and \"{b}\": give each its own part"),
            );
        }
    };
    let title = get_string(obj, "title", path)?;
    let inner = &obj[key];
    let at = join(path, key);
    let kind = match key {
        "chart" => PartKind::Chart(Box::new(parse_chart(inner, &at)?)),
        "stats" => PartKind::Stats(parse_stats(inner, &at)?),
        "table" => PartKind::Table(parse_table(inner, &at)?),
        "text" => match inner {
            Value::String(s) => PartKind::Text(s.clone()),
            other => return fail(&at, format!("must be a Markdown string, not {}", show(other))),
        },
        _ => {
            let Some(items) = inner.as_array().filter(|a| !a.is_empty()) else {
                return fail(&at, "must be a non-empty array of parts to sit side by side");
            };
            PartKind::Columns(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, p)| parse_part(p, &index(&at, i)))
                    .collect::<Result<_, _>>()?,
            )
        }
    };
    Ok(Part { title, kind })
}

fn parse_stats(v: &Value, path: &str) -> Result<Vec<Stat>, String> {
    let Some(items) = v.as_array().filter(|a| !a.is_empty()) else {
        return fail(
            path,
            "must be a non-empty array of {\"label\": \"…\", \"value\": \"…\" or a number}",
        );
    };
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let at = index(path, i);
        let Some(obj) = item.as_object() else {
            return fail(&at, "a stat is an object with \"label\" and \"value\"");
        };
        for key in obj.keys() {
            if !["label", "value", "note", "tone"].contains(&key.as_str()) {
                return fail(&join(&at, key), "isn't a stat key (use label, value, note or tone)");
            }
        }
        let Some(label) = get_string(obj, "label", &at)? else {
            return fail(&join(&at, "label"), "is missing: say what the value is");
        };
        let value = match obj.get("value") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.as_f64().map_or_else(|| n.to_string(), stat_number),
            Some(other) => {
                return fail(
                    &join(&at, "value"),
                    format!("must be a string or a number, not {}", show(other)),
                );
            }
            None => return fail(&join(&at, "value"), "is missing"),
        };
        let note = get_string(obj, "note", &at)?;
        let tone = match obj.get("tone") {
            None | Some(Value::Null) => Tone::Neutral,
            Some(Value::String(s)) if s == "neutral" => Tone::Neutral,
            Some(Value::String(s)) if s == "good" => Tone::Good,
            Some(Value::String(s)) if s == "bad" => Tone::Bad,
            Some(other) => {
                return fail(
                    &join(&at, "tone"),
                    format!("{} isn't a tone (use \"good\", \"bad\" or \"neutral\")", show(other)),
                );
            }
        };
        out.push(Stat {
            label,
            value,
            note,
            tone,
        });
    }
    Ok(out)
}

/// A stat's number: grouped from five digits on (12,345), as written below.
fn stat_number(v: f64) -> String {
    if v.abs() >= 10_000.0 {
        format::full(v)
    } else {
        format::plain(v)
    }
}

fn parse_table(v: &Value, path: &str) -> Result<Table, String> {
    let Some(obj) = v.as_object() else {
        return fail(path, "a table is an object with \"rows\" (and \"columns\")");
    };
    for key in obj.keys() {
        if key != "columns" && key != "rows" {
            return fail(&join(path, key), "isn't a table key (use columns and rows)");
        }
    }
    let rows_at = join(path, "rows");
    let Some(rows) = obj.get("rows").and_then(Value::as_array) else {
        return fail(
            &rows_at,
            "is missing: give an array of rows (arrays of cells, or objects)",
        );
    };
    let mut columns: Option<Vec<String>> = None;
    if let Some(c) = obj.get("columns").filter(|c| !c.is_null()) {
        let Some(list) = c.as_array() else {
            return fail(&join(path, "columns"), "must be an array of column names");
        };
        columns = Some(
            list.iter()
                .map(|c| match c {
                    Value::String(s) => s.clone(),
                    other => cell(other).text,
                })
                .collect(),
        );
    }
    let objects = rows.first().is_some_and(Value::is_object);
    let columns = match columns {
        Some(c) => c,
        None if objects => rows[0]
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let at = index(&rows_at, i);
        match (row, objects) {
            (Value::Object(o), true) => out.push(
                columns
                    .iter()
                    .map(|c| o.get(c).map_or_else(Cell::blank, cell))
                    .collect(),
            ),
            (Value::Array(cells), false) => {
                if !columns.is_empty() && cells.len() > columns.len() {
                    return fail(
                        &at,
                        format!("has {} cells but the table has {} columns", cells.len(), columns.len()),
                    );
                }
                out.push(cells.iter().map(cell).collect());
            }
            _ => {
                return fail(
                    &at,
                    if objects {
                        "must be an object like the first row (rows are all arrays or all objects)"
                    } else {
                        "must be an array of cells like the first row (rows are all arrays or all objects)"
                    },
                );
            }
        }
    }
    Ok(Table { columns, rows: out })
}

impl Cell {
    fn blank() -> Self {
        Self {
            text: String::new(),
            number: false,
        }
    }
}

fn cell(v: &Value) -> Cell {
    match v {
        Value::Null => Cell::blank(),
        Value::String(s) => Cell {
            text: s.clone(),
            number: false,
        },
        Value::Number(n) => Cell {
            text: n.as_f64().map_or_else(|| n.to_string(), format::plain),
            number: true,
        },
        Value::Bool(b) => Cell {
            text: b.to_string(),
            number: false,
        },
        other => Cell {
            text: other.to_string(),
            number: false,
        },
    }
}

// -- Charts --------------------------------------------------------------------

/// Keys a chart may have that the layout decides for itself.
const IGNORED_CHART_KEYS: [&str; 9] = [
    "$schema",
    "description",
    "width",
    "autosize",
    "padding",
    "background",
    "config",
    "name",
    "view",
];

fn parse_chart(v: &Value, path: &str) -> Result<Chart, String> {
    let Some(obj) = v.as_object() else {
        return fail(path, "a chart is an object with \"mark\", \"data\" and \"encoding\"");
    };
    for key in obj.keys() {
        match key.as_str() {
            "mark" | "data" | "encoding" | "title" | "height" => {}
            k if IGNORED_CHART_KEYS.contains(&k) => {}
            k if MULTI_VIEW.contains(&k) => return multi_view_error(path, k),
            "transform" => {
                return fail(
                    &join(path, key),
                    "isn't supported: compute the values yourself and put the results in data.values",
                );
            }
            "selection" | "params" => {
                return fail(
                    &join(path, key),
                    "isn't supported: charts here show tooltips, nothing more",
                );
            }
            "projection" => return fail(&join(path, key), "isn't supported: maps can't be drawn"),
            "resolve" => return fail(&join(path, key), "isn't supported (it's for layers, which aren't)"),
            _ => {
                return fail(
                    &join(path, key),
                    "isn't a chart key (use mark, data, encoding, title or height)",
                );
            }
        }
    }
    let title = match obj.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(t)) => get_string(t, "text", &join(path, "title"))?,
        Some(other) => return fail(&join(path, "title"), format!("must be a string, not {}", show(other))),
    };
    let height = match obj.get("height") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_f64().map(|h| (h as f32).clamp(100.0, 600.0)),
        Some(other) => {
            return fail(
                &join(path, "height"),
                format!("must be a number of px, 100 to 600 (got {})", show(other)),
            );
        }
    };
    let mark = parse_mark(obj.get("mark"), &join(path, "mark"))?;
    let data = parse_data(obj.get("data"), &join(path, "data"))?;
    let encoding = parse_encoding(obj.get("encoding"), &join(path, "encoding"), &data, mark.kind)?;
    let chart = Chart {
        title,
        height,
        mark,
        data,
        encoding,
    };
    check_chart(&chart, path)?;
    Ok(chart)
}

const MARKS: [&str; 7] = ["bar", "line", "area", "point", "circle", "square", "arc"];

/// Mark properties that only style it; the layout has its own.
const IGNORED_MARK_KEYS: [&str; 22] = [
    "tooltip",
    "opacity",
    "color",
    "fill",
    "stroke",
    "strokeWidth",
    "strokeCap",
    "strokeJoin",
    "strokeOpacity",
    "fillOpacity",
    "cornerRadius",
    "cornerRadiusEnd",
    "filled",
    "size",
    "interpolate",
    "outerRadius",
    "padAngle",
    "orient",
    "thickness",
    "binSpacing",
    "clip",
    "line",
];

fn parse_mark(v: Option<&Value>, path: &str) -> Result<Mark, String> {
    let kind_of = |name: &str| -> Result<MarkKind, String> {
        Ok(match name {
            "bar" => MarkKind::Bar,
            "line" => MarkKind::Line,
            "area" => MarkKind::Area,
            "point" => MarkKind::Point,
            "circle" => MarkKind::Circle,
            "square" => MarkKind::Square,
            "arc" => MarkKind::Arc,
            other => {
                return fail(
                    path,
                    format!("\"{other}\" isn't supported (use bar, line, area, point or arc)"),
                );
            }
        })
    };
    match v {
        None | Some(Value::Null) => fail(path, format!("is missing: one of {}", either(&MARKS))),
        Some(Value::String(s)) => Ok(Mark {
            kind: kind_of(s)?,
            point: false,
            inner_radius: None,
        }),
        Some(Value::Object(o)) => {
            for key in o.keys() {
                if !["type", "point", "innerRadius"].contains(&key.as_str())
                    && !IGNORED_MARK_KEYS.contains(&key.as_str())
                {
                    return fail(
                        &join(path, key),
                        "isn't supported (a mark object has type, point and innerRadius)",
                    );
                }
            }
            let Some(name) = get_string(o, "type", path)? else {
                return fail(&join(path, "type"), format!("is missing: one of {}", either(&MARKS)));
            };
            let point = match o.get("point") {
                None | Some(Value::Null | Value::Bool(false)) => false,
                Some(Value::Bool(true) | Value::Object(_)) => true,
                Some(Value::String(s)) if s == "transparent" => false,
                Some(other) => {
                    return fail(
                        &join(path, "point"),
                        format!("must be true or false, not {}", show(other)),
                    );
                }
            };
            let inner_radius = match o.get("innerRadius") {
                None | Some(Value::Null) => None,
                Some(Value::Number(n)) => n.as_f64().map(|r| r.max(0.0) as f32),
                Some(other) => {
                    return fail(
                        &join(path, "innerRadius"),
                        format!("must be a number of px, not {}", show(other)),
                    );
                }
            };
            Ok(Mark {
                kind: kind_of(&name)?,
                point,
                inner_radius,
            })
        }
        Some(other) => fail(path, format!("must be a mark name like \"bar\", not {}", show(other))),
    }
}

fn parse_data(v: Option<&Value>, path: &str) -> Result<Vec<Row>, String> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return fail(path, "is missing: give the rows inline as {\"values\": [{...}, ...]}");
    };
    for key in obj.keys() {
        match key.as_str() {
            "values" => {}
            "url" => return fail(&join(path, key), "isn't supported: put the rows in data.values"),
            "format" => {}
            _ => {
                return fail(&join(path, key), "isn't supported: give the rows inline in data.values");
            }
        }
    }
    let at = join(path, "values");
    let Some(values) = obj.get("values") else {
        return fail(
            &at,
            "is missing: give an array of rows such as {\"day\": \"Mon\", \"ms\": 120}",
        );
    };
    let Some(values) = values.as_array() else {
        return fail(&at, "must be an array of rows such as {\"day\": \"Mon\", \"ms\": 120}");
    };
    if values.is_empty() {
        return fail(&at, "is empty: give at least one row");
    }
    if values.len() > MAX_ROWS {
        return fail(
            &at,
            format!(
                "has {} rows; at most {MAX_ROWS} are drawn, so aggregate first",
                values.len()
            ),
        );
    }
    values
        .iter()
        .enumerate()
        .map(|(i, r)| match r {
            Value::Object(o) => Ok(o.clone()),
            other => fail(
                &index(&at, i),
                format!(
                    "each row must be an object such as {{\"day\": \"Mon\", \"ms\": 120}}, not {}",
                    show(other)
                ),
            ),
        })
        .collect()
}

/// The channels a chart can use.
const CHANNELS: [&str; 6] = ["x", "y", "color", "theta", "xOffset", "tooltip"];

fn parse_encoding(v: Option<&Value>, path: &str, data: &[Row], mark: MarkKind) -> Result<Encoding, String> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return fail(
            path,
            if mark == MarkKind::Arc {
                "is missing: an arc needs {\"theta\": {...}, \"color\": {...}}"
            } else {
                "is missing: give at least {\"x\": {...}, \"y\": {...}}"
            },
        );
    };
    for key in obj.keys() {
        match key.as_str() {
            k if CHANNELS.contains(&k) => {}
            "row" | "column" | "facet" => {
                return fail(
                    &join(path, key),
                    "isn't supported: draw one chart per group, each in its own part of \"rows\" or \"columns\"",
                );
            }
            "fill" | "stroke" => return fail(&join(path, key), "isn't supported: use color"),
            _ => {
                return fail(
                    &join(path, key),
                    format!("isn't a supported channel (use {})", either(&CHANNELS)),
                );
            }
        }
    }
    let channel = |name: &str, default: Option<FieldType>| -> Result<Option<FieldDef>, String> {
        match obj.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => parse_field(v, &join(path, name), data, default).map(Some),
        }
    };
    // Colours and offsets split series, so they're categories unless told.
    let x = channel("x", None)?;
    let y = channel("y", None)?;
    let color = channel("color", Some(FieldType::Nominal))?;
    let theta = channel("theta", Some(FieldType::Quantitative))?;
    let x_offset = channel("xOffset", Some(FieldType::Nominal))?;
    let tooltip = match obj.get("tooltip") {
        None => None,
        Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .enumerate()
                .map(|(i, t)| parse_field(t, &index(&join(path, "tooltip"), i), data, None))
                .collect::<Result<_, _>>()?,
        ),
        Some(Value::Object(o)) if o.contains_key("content") => None,
        Some(t) => Some(vec![parse_field(t, &join(path, "tooltip"), data, None)?]),
    };
    Ok(Encoding {
        x,
        y,
        color,
        theta,
        x_offset,
        tooltip,
    })
}

const FIELD_KEYS: [&str; 10] = [
    "field",
    "type",
    "aggregate",
    "title",
    "sort",
    "axis",
    "scale",
    "stack",
    "legend",
    "format",
];

fn parse_field(v: &Value, path: &str, data: &[Row], default: Option<FieldType>) -> Result<FieldDef, String> {
    let Some(obj) = v.as_object() else {
        return fail(
            path,
            format!(
                "must be a field definition like {{\"field\": \"…\", \"type\": \"…\"}}, not {}",
                show(v)
            ),
        );
    };
    for key in obj.keys() {
        match key.as_str() {
            k if FIELD_KEYS.contains(&k) => {}
            "formatType" | "bandPosition" | "band" => {}
            "bin" => {
                return fail(
                    &join(path, key),
                    "isn't supported: count the values into buckets yourself and put one row per bucket in data.values",
                );
            }
            "timeUnit" => {
                return fail(
                    &join(path, key),
                    "isn't supported: group the dates yourself (one row per day, month or year, like \"2026-10\") in data.values",
                );
            }
            "value" | "datum" => {
                return fail(
                    &join(path, key),
                    "isn't supported: a channel encodes a field of data.values (leave color out for one series)",
                );
            }
            "condition" => return fail(&join(path, key), "isn't supported: charts here aren't interactive"),
            "impute" => {
                return fail(
                    &join(path, key),
                    "isn't supported: fill the gaps in data.values yourself",
                );
            }
            _ => {
                return fail(
                    &join(path, key),
                    "isn't supported (a field definition has field, type, aggregate, title, sort, axis, scale and stack)",
                );
            }
        }
    }
    let aggregate = match obj.get("aggregate") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(match s.as_str() {
            "sum" => Aggregate::Sum,
            "mean" | "average" => Aggregate::Mean,
            "count" => Aggregate::Count,
            "min" => Aggregate::Min,
            "max" => Aggregate::Max,
            "median" => Aggregate::Median,
            other => {
                return fail(
                    &join(path, "aggregate"),
                    format!("\"{other}\" isn't supported (use sum, mean, count, min, max or median)"),
                );
            }
        }),
        Some(other) => {
            return fail(
                &join(path, "aggregate"),
                format!("must be a string like \"sum\", not {}", show(other)),
            );
        }
    };
    let field = match obj.get("field") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => return fail(&join(path, "field"), format!("must be a string, not {}", show(other))),
    };
    if field.is_none() && aggregate != Some(Aggregate::Count) {
        return fail(
            &join(path, "field"),
            "is missing: name a field of data.values (only \"aggregate\": \"count\" goes without one)",
        );
    }
    if let Some(f) = &field
        && !data.iter().any(|r| r.contains_key(f))
    {
        let known: Vec<&str> = data
            .first()
            .map(|r| r.keys().map(String::as_str).take(10).collect())
            .unwrap_or_default();
        return fail(
            path,
            format!("field \"{f}\" isn't in data.values (its fields: {})", known.join(", ")),
        );
    }
    let kind = match obj.get("type") {
        None | Some(Value::Null) => {
            if aggregate == Some(Aggregate::Count) {
                FieldType::Quantitative
            } else {
                default.unwrap_or_else(|| infer(data, field.as_deref().unwrap_or_default()))
            }
        }
        Some(Value::String(s)) => match s.as_str() {
            "quantitative" => FieldType::Quantitative,
            "nominal" => FieldType::Nominal,
            "ordinal" => FieldType::Ordinal,
            "temporal" => FieldType::Temporal,
            other => {
                return fail(
                    &join(path, "type"),
                    format!("\"{other}\" isn't a type (use quantitative, nominal, ordinal or temporal)"),
                );
            }
        },
        Some(other) => return fail(&join(path, "type"), format!("must be a string, not {}", show(other))),
    };
    if aggregate.is_some_and(|a| a != Aggregate::Count) && kind != FieldType::Quantitative {
        return fail(
            &join(path, "aggregate"),
            format!(
                "needs a quantitative field, and \"{}\" is {}",
                field.as_deref().unwrap_or(""),
                kind.name()
            ),
        );
    }
    // The values must be what the type says.
    if let Some(f) = &field
        && aggregate != Some(Aggregate::Count)
    {
        let values = data.iter().filter_map(|r| r.get(f)).filter(|v| !v.is_null());
        match kind {
            FieldType::Quantitative => {
                let mut values = values.peekable();
                let first = values.peek().copied().cloned();
                if !values.any(|v| number_of(v).is_some()) {
                    return fail(
                        path,
                        format!(
                            "field \"{f}\" is quantitative but has no numbers (first value: {}); write numbers, or use type \"nominal\"",
                            first.as_ref().map_or_else(|| "none".to_owned(), show)
                        ),
                    );
                }
            }
            FieldType::Temporal => {
                if let Some(bad) = values.clone().find(|v| time::parse(v).is_none()) {
                    return fail(
                        path,
                        format!(
                            "{} isn't a date (use ISO dates like \"2026-10-03\", datetimes like \"2026-10-03T14:00\", or epoch milliseconds)",
                            show(bad)
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    let title = get_string(obj, "title", path)?;
    let sort = parse_sort(obj.get("sort"), &join(path, "sort"))?;
    let axis = match obj.get("axis") {
        None => AxisSpec::Auto,
        Some(Value::Null) => AxisSpec::Hidden,
        Some(Value::Object(a)) => match a.get("title") {
            None => AxisSpec::Auto,
            Some(Value::Null) => AxisSpec::Untitled,
            Some(Value::String(s)) if s.is_empty() => AxisSpec::Untitled,
            Some(Value::String(s)) => AxisSpec::Titled(s.clone()),
            Some(other) => {
                return fail(
                    &join(path, "axis.title"),
                    format!("must be a string or null, not {}", show(other)),
                );
            }
        },
        Some(other) => {
            return fail(
                &join(path, "axis"),
                format!("must be null or {{\"title\": …}}, not {}", show(other)),
            );
        }
    };
    let legend = !matches!(obj.get("legend"), Some(Value::Null));
    let (domain, zero) = parse_scale(obj.get("scale"), &join(path, "scale"), kind)?;
    let stack = match obj.get("stack") {
        None => None,
        Some(Value::Null | Value::Bool(false)) => Some(Stack::Off),
        Some(Value::Bool(true)) => Some(Stack::Zero),
        Some(Value::String(s)) if s == "zero" => Some(Stack::Zero),
        Some(Value::String(s)) if s == "normalize" => Some(Stack::Normalize),
        Some(other) => {
            return fail(
                &join(path, "stack"),
                format!(
                    "{} isn't supported (use true, false, null or \"normalize\")",
                    show(other)
                ),
            );
        }
    };
    Ok(FieldDef {
        field,
        kind,
        aggregate,
        title,
        sort,
        axis,
        legend,
        domain,
        zero,
        stack,
    })
}

/// A field's type from its values: numbers are quantitative, ISO dates
/// temporal, anything else nominal.
fn infer(data: &[Row], field: &str) -> FieldType {
    let mut values = data
        .iter()
        .filter_map(|r| r.get(field))
        .filter(|v| !v.is_null())
        .peekable();
    if values.peek().is_none() {
        return FieldType::Nominal;
    }
    let values: Vec<&Value> = values.collect();
    if values.iter().all(|v| v.is_number()) {
        return FieldType::Quantitative;
    }
    let dated = |v: &&Value| {
        v.as_str()
            .is_some_and(|s| s.contains('-') && time::parse_str(s).is_some())
    };
    if values.iter().all(dated) {
        return FieldType::Temporal;
    }
    FieldType::Nominal
}

fn parse_sort(v: Option<&Value>, path: &str) -> Result<Sort, String> {
    Ok(match v {
        None => Sort::Data,
        Some(Value::Null) => Sort::Data,
        Some(Value::String(s)) => match s.as_str() {
            "ascending" => Sort::Ascending,
            "descending" => Sort::Descending,
            "x" | "y" | "theta" => Sort::ByMeasure { descending: false },
            "-x" | "-y" | "-theta" => Sort::ByMeasure { descending: true },
            other => {
                return fail(
                    path,
                    format!(
                        "\"{other}\" isn't supported (use \"ascending\", \"descending\", \"-y\", null or a list of values)"
                    ),
                );
            }
        },
        Some(Value::Array(items)) => Sort::Values(
            items
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format::plain),
                    other => other.to_string(),
                })
                .collect(),
        ),
        Some(Value::Object(o)) => {
            let descending = match o.get("order") {
                None | Some(Value::Null) => false,
                Some(Value::String(s)) if s == "ascending" => false,
                Some(Value::String(s)) if s == "descending" => true,
                Some(other) => {
                    return fail(
                        &join(path, "order"),
                        format!("{} isn't an order (use \"ascending\" or \"descending\")", show(other)),
                    );
                }
            };
            if o.contains_key("field") || o.contains_key("op") || o.contains_key("encoding") {
                Sort::ByMeasure { descending }
            } else if descending {
                Sort::Descending
            } else {
                Sort::Ascending
            }
        }
        Some(other) => return fail(path, format!("{} isn't a sort order", show(other))),
    })
}

fn parse_scale(v: Option<&Value>, path: &str, kind: FieldType) -> Result<(Option<Domain>, Option<bool>), String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok((None, None));
    };
    let Some(obj) = v.as_object() else {
        return fail(
            path,
            "must be an object such as {\"domain\": [0, 100], \"zero\": false}",
        );
    };
    for key in obj.keys() {
        match key.as_str() {
            "domain" | "zero" => {}
            // Styling the layout decides.
            "nice" | "padding" | "paddingInner" | "paddingOuter" | "round" | "clamp" | "range" | "scheme" => {}
            "type" => match obj.get("type").and_then(Value::as_str) {
                Some("linear" | "time" | "utc" | "band" | "point" | "ordinal") => {}
                other => {
                    let shown = other.map_or_else(|| show(&obj["type"]), |s| format!("\"{s}\""));
                    return fail(
                        &join(path, "type"),
                        format!("{shown} isn't supported (scales are linear)"),
                    );
                }
            },
            _ => return fail(&join(path, key), "isn't supported (a scale has domain and zero)"),
        }
    }
    let zero = get_bool(obj, "zero", path)?;
    let domain = match obj.get("domain") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "unaggregated" => None,
        Some(Value::Array(items)) => match kind {
            FieldType::Quantitative | FieldType::Temporal => {
                let read = |v: &Value| {
                    if kind == FieldType::Temporal {
                        time::parse(v)
                    } else {
                        number_of(v)
                    }
                };
                match items.as_slice() {
                    [a, b] => match (read(a), read(b)) {
                        (Some(a), Some(b)) if a < b => Some(Domain::Range(a, b)),
                        (Some(_), Some(_)) => {
                            return fail(&join(path, "domain"), "must go from a smaller value to a larger one");
                        }
                        _ => {
                            return fail(
                                &join(path, "domain"),
                                if kind == FieldType::Temporal {
                                    "must be two dates, [start, end]"
                                } else {
                                    "must be two numbers, [min, max]"
                                },
                            );
                        }
                    },
                    _ => {
                        return fail(&join(path, "domain"), "must be two values, [min, max]");
                    }
                }
            }
            _ => Some(Domain::Values(
                items
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format::plain),
                        other => other.to_string(),
                    })
                    .collect(),
            )),
        },
        Some(other) => {
            return fail(&join(path, "domain"), format!("must be an array, not {}", show(other)));
        }
    };
    Ok((domain, zero))
}

// -- What a chart's encoding must hold, by mark ----------------------------------

impl Chart {
    /// Bars that run across, categories down the left: y is the categories
    /// and x the measure.
    pub fn horizontal(&self) -> bool {
        self.mark.kind == MarkKind::Bar
            && matches!((&self.encoding.x, &self.encoding.y), (Some(x), Some(y))
                if x.kind == FieldType::Quantitative && y.kind.is_discrete())
    }

    /// The colour channel, unless it only repeats the category axis (bars
    /// coloured by the same field as their labels), which adds nothing.
    pub fn series_channel(&self) -> Option<&FieldDef> {
        let color = self.encoding.color.as_ref()?;
        let band = if self.horizontal() {
            self.encoding.y.as_ref()
        } else {
            self.encoding.x.as_ref()
        };
        if self.mark.kind != MarkKind::Arc
            && band.is_some_and(|b| b.kind.is_discrete() && b.field.is_some() && b.field == color.field)
        {
            return None;
        }
        Some(color)
    }
}

fn check_chart(chart: &Chart, path: &str) -> Result<(), String> {
    let e = &chart.encoding;
    let enc = join(path, "encoding");
    let mark = chart.mark.kind;
    if mark == MarkKind::Arc {
        for (name, ch) in [("x", &e.x), ("y", &e.y), ("xOffset", &e.x_offset)] {
            if ch.is_some() {
                return fail(
                    &join(&enc, name),
                    "an arc doesn't use it (use theta for the size and color for the slices)",
                );
            }
        }
        let Some(theta) = &e.theta else {
            return fail(
                &join(&enc, "theta"),
                "is missing: an arc needs theta (the size of each slice)",
            );
        };
        if theta.kind != FieldType::Quantitative {
            return fail(&join(&enc, "theta"), "must be quantitative (the size of each slice)");
        }
        let Some(color) = &e.color else {
            return fail(
                &join(&enc, "color"),
                "is missing: an arc needs color to split the circle into slices",
            );
        };
        check_color(color, &join(&enc, "color"), &chart.data, "slices")?;
        return Ok(());
    }
    if e.theta.is_some() {
        return fail(
            &join(&enc, "theta"),
            format!("only an arc uses theta; a {} uses x and y", mark.name()),
        );
    }
    let Some(x) = &e.x else {
        return fail(
            &join(&enc, "x"),
            format!("is missing: a {} chart needs x and y", mark.name()),
        );
    };
    let Some(y) = &e.y else {
        return fail(
            &join(&enc, "y"),
            format!("is missing: a {} chart needs x and y", mark.name()),
        );
    };
    match mark {
        MarkKind::Bar => {
            let horizontal = x.kind == FieldType::Quantitative && y.kind.is_discrete();
            if !horizontal && y.kind != FieldType::Quantitative {
                return fail(
                    &join(&enc, "y"),
                    if x.kind == FieldType::Quantitative && y.kind == FieldType::Temporal {
                        "dates go on x: put the dates on x and the numbers on y"
                    } else {
                        "a bar needs a quantitative y (or a quantitative x, with categories on y, for bars across)"
                    },
                );
            }
            if let Some(off) = &e.x_offset {
                if horizontal {
                    return fail(
                        &join(&enc, "xOffset"),
                        "grouped bars need the categories on x (bars across can't be grouped)",
                    );
                }
                if !x.kind.is_discrete() {
                    return fail(
                        &join(&enc, "xOffset"),
                        "grouped bars need nominal or ordinal categories on x",
                    );
                }
                if !off.kind.is_discrete() {
                    return fail(
                        &join(&enc, "xOffset"),
                        "must be nominal or ordinal (the bars within a group)",
                    );
                }
            }
        }
        MarkKind::Line | MarkKind::Area if y.kind != FieldType::Quantitative => {
            return fail(
                &join(&enc, "y"),
                format!("a {} needs a quantitative y (got {})", mark.name(), y.kind.name()),
            );
        }
        _ => {}
    }
    if mark != MarkKind::Bar && e.x_offset.is_some() {
        return fail(&join(&enc, "xOffset"), "only bars can be grouped with xOffset");
    }
    if let Some(color) = &e.color
        && chart.series_channel().is_some()
    {
        check_color(color, &join(&enc, "color"), &chart.data, "series")?;
    }
    if let Some(off) = &e.x_offset {
        check_count(off, &join(&enc, "xOffset"), &chart.data, MAX_SERIES, "bars in a group")?;
    }
    for (name, ch) in [("x", x), ("y", y)] {
        if ch.kind.is_discrete() {
            check_count(ch, &join(&enc, name), &chart.data, MAX_CATEGORIES, "categories")?;
        }
    }
    Ok(())
}

fn check_color(color: &FieldDef, path: &str, data: &[Row], what: &str) -> Result<(), String> {
    if !color.kind.is_discrete() {
        return fail(
            path,
            format!(
                "must be nominal or ordinal (it splits the {what}); a {} color isn't supported",
                color.kind.name()
            ),
        );
    }
    check_count(color, path, data, MAX_SERIES, what)
}

fn check_count(ch: &FieldDef, path: &str, data: &[Row], max: usize, what: &str) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for row in data {
        if let Some(t) = ch.text(row) {
            seen.insert(t);
        }
    }
    if seen.len() > max {
        return fail(
            path,
            format!(
                "{} {what} is more than a chart can tell apart (at most {max}): keep the largest and group the rest as \"Other\"",
                seen.len()
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[allow(clippy::needless_pass_by_value, reason = "tests read best with json! in place")]
    fn err(v: Value) -> String {
        parse_visual(&v).expect_err("an error")
    }

    fn bars() -> Value {
        json!({
            "mark": "bar",
            "data": {"values": [{"day": "Mon", "ms": 120}, {"day": "Tue", "ms": 90}]},
            "encoding": {"x": {"field": "day", "type": "nominal"}, "y": {"field": "ms", "type": "quantitative"}}
        })
    }

    #[test]
    fn a_single_chart() {
        let v = parse_visual(&bars()).unwrap();
        assert_eq!(v.rows.len(), 1);
        let PartKind::Chart(chart) = &v.rows[0].kind else {
            panic!("a chart")
        };
        assert_eq!(chart.mark.kind, MarkKind::Bar);
        assert_eq!(chart.data.len(), 2);
        assert_eq!(chart.encoding.y.as_ref().unwrap().kind, FieldType::Quantitative);
        assert_eq!(chart.height, None);
    }

    #[test]
    fn a_composition_of_every_part() {
        let v = parse_visual(&json!({
            "title": "Build health",
            "rows": [
                {"stats": [{"label": "Builds", "value": 1284, "note": "+12% this week", "tone": "good"},
                           {"label": "Failures", "value": "3", "tone": "bad"}]},
                {"title": "Slowest", "table": {"rows": [{"job": "lint", "ms": 1200}, {"job": "test", "ms": 81000.5}]}},
                {"columns": [{"chart": bars()}, {"text": "**Note**: flaky on Mondays"}]},
            ]
        }))
        .unwrap();
        assert_eq!(v.title.as_deref(), Some("Build health"));
        let PartKind::Stats(stats) = &v.rows[0].kind else {
            panic!()
        };
        assert_eq!(stats[0].value, "1284");
        assert_eq!(stats[0].tone, Tone::Good);
        assert_eq!(stats[1].tone, Tone::Bad);
        let PartKind::Table(table) = &v.rows[1].kind else {
            panic!()
        };
        assert_eq!(table.columns, ["job", "ms"]);
        assert_eq!(table.rows[1][1].text, "81000.5");
        assert!(table.numeric(1));
        assert!(!table.numeric(0));
        assert_eq!(v.rows[1].title.as_deref(), Some("Slowest"));
        let PartKind::Columns(cols) = &v.rows[2].kind else {
            panic!()
        };
        assert!(matches!(cols[1].kind, PartKind::Text(_)));
    }

    #[test]
    fn table_columns_follow_the_first_rows_key_order() {
        let v =
            parse_visual(&json!({"table": {"rows": [{"zeta": 1, "alpha": 2, "mid": 3}, {"alpha": 5, "extra": 9}]}}))
                .unwrap();
        let PartKind::Table(t) = &v.rows[0].kind else { panic!() };
        assert_eq!(t.columns, ["zeta", "alpha", "mid"]);
        assert_eq!(
            t.rows[1].iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["", "5", ""]
        );
        let v = parse_visual(&json!({"table": {"columns": ["a", "b"], "rows": [["x", 1], ["y"]]}})).unwrap();
        let PartKind::Table(t) = &v.rows[0].kind else { panic!() };
        assert_eq!(t.rows[1].len(), 1);
        assert_eq!(t.width(), 2);
    }

    #[test]
    fn errors_name_the_place_and_the_fix() {
        let mut spec = bars();
        spec["encoding"]["y"]["field"] = json!("latency");
        assert_eq!(
            err(spec),
            "encoding.y: field \"latency\" isn't in data.values (its fields: day, ms)"
        );

        let spec = json!({"rows": [{"text": "hi"}, {"chart": {
            "mark": "bar", "data": {"values": [{"day": "Mon", "v": 1}]},
            "encoding": {"x": {"field": "day"}, "y": {"field": "ms"}}
        }}]});
        assert_eq!(
            err(spec),
            "rows[1].chart.encoding.y: field \"ms\" isn't in data.values (its fields: day, v)"
        );

        let mut spec = bars();
        spec["mark"] = json!("rule");
        assert_eq!(
            err(spec),
            "mark: \"rule\" isn't supported (use bar, line, area, point or arc)"
        );
        let mut spec = bars();
        spec["mark"] = json!({"type": "tick"});
        assert_eq!(
            err(spec),
            "mark: \"tick\" isn't supported (use bar, line, area, point or arc)"
        );
    }

    #[test]
    fn refuses_what_would_change_the_data() {
        let mut spec = bars();
        spec["layer"] = json!([]);
        assert!(err(spec).starts_with("layer: isn't supported: put each chart in its own part"));
        let mut spec = bars();
        spec["transform"] = json!([{"filter": "datum.ms > 100"}]);
        assert!(err(spec).starts_with("transform: isn't supported: compute the values yourself"));
        assert!(err(json!({"facet": {}, "spec": {}})).starts_with("facet: isn't supported"));
        let mut spec = bars();
        spec["data"] = json!({"url": "data/cars.json"});
        assert_eq!(err(spec), "data.url: isn't supported: put the rows in data.values");
        let mut spec = bars();
        spec["encoding"]["x"]["bin"] = json!(true);
        assert!(err(spec).starts_with("encoding.x.bin: isn't supported: count the values into buckets"));
        let mut spec = bars();
        spec["encoding"]["x"]["timeUnit"] = json!("month");
        assert!(err(spec).starts_with("encoding.x.timeUnit: isn't supported"));
        let mut spec = bars();
        spec["encoding"]["size"] = json!({"field": "ms"});
        assert_eq!(
            err(spec),
            "encoding.size: isn't a supported channel (use x, y, color, theta, xOffset or tooltip)"
        );
        let mut spec = bars();
        spec["encoding"]["y"]["scale"] = json!({"type": "log"});
        assert_eq!(
            err(spec),
            "encoding.y.scale.type: \"log\" isn't supported (scales are linear)"
        );
        let mut spec = bars();
        spec["encoding"]["y"]["aggregate"] = json!("stdev");
        assert_eq!(
            err(spec),
            "encoding.y.aggregate: \"stdev\" isn't supported (use sum, mean, count, min, max or median)"
        );
    }

    #[test]
    fn ignores_styling_the_layout_decides() {
        let mut spec = bars();
        spec["$schema"] = json!("https://vega.github.io/schema/vega-lite/v5.json");
        spec["width"] = json!("container");
        spec["config"] = json!({"axis": {"grid": false}});
        spec["mark"] = json!({"type": "bar", "cornerRadiusEnd": 4, "tooltip": true});
        spec["encoding"]["x"]["axis"] = json!({"labelAngle": -45});
        assert!(parse_visual(&spec).is_ok());
    }

    #[test]
    fn data_errors() {
        let mut spec = bars();
        spec["data"]["values"] = json!([]);
        assert_eq!(err(spec), "data.values: is empty: give at least one row");
        let mut spec = bars();
        spec["data"]["values"] = json!([{"day": "Mon", "ms": 1}, 5]);
        assert!(err(spec).starts_with("data.values[1]: each row must be an object"));
        let mut spec = bars();
        spec["data"]["values"] = json!([{"day": "Mon", "ms": "fast"}]);
        assert_eq!(
            err(spec),
            "encoding.y: field \"ms\" is quantitative but has no numbers (first value: \"fast\"); write numbers, or use type \"nominal\""
        );
        let mut spec = bars();
        spec["encoding"]["x"] = json!({"field": "day", "type": "temporal"});
        assert!(err(spec).starts_with("encoding.x: \"Mon\" isn't a date (use ISO dates"));
        let mut spec = bars();
        spec["encoding"]["x"]["type"] = json!("geo");
        assert_eq!(
            err(spec),
            "encoding.x.type: \"geo\" isn't a type (use quantitative, nominal, ordinal or temporal)"
        );
    }

    #[test]
    fn channel_rules_by_mark() {
        let mut spec = bars();
        spec["encoding"].as_object_mut().unwrap().shift_remove("y");
        assert_eq!(err(spec), "encoding.y: is missing: a bar chart needs x and y");
        let mut spec = bars();
        spec["mark"] = json!("arc");
        assert_eq!(
            err(spec),
            "encoding.x: an arc doesn't use it (use theta for the size and color for the slices)"
        );
        let spec =
            json!({"mark": "arc", "data": {"values": [{"k": "a", "v": 1}]}, "encoding": {"theta": {"field": "v"}}});
        assert_eq!(
            err(spec),
            "encoding.color: is missing: an arc needs color to split the circle into slices"
        );
        let mut spec = bars();
        spec["mark"] = json!("line");
        spec["encoding"]["y"]["type"] = json!("nominal");
        assert_eq!(err(spec), "encoding.y: a line needs a quantitative y (got nominal)");
        let mut spec = bars();
        spec["encoding"]["xOffset"] = json!({"field": "ms", "type": "quantitative"});
        assert_eq!(
            err(spec),
            "encoding.xOffset: must be nominal or ordinal (the bars within a group)"
        );
    }

    #[test]
    fn too_many_series() {
        let values: Vec<Value> = (0..12)
            .map(|i| json!({"d": "a", "s": format!("s{i}"), "v": i}))
            .collect();
        let spec = json!({"mark": "line", "data": {"values": values},
            "encoding": {"x": {"field": "d"}, "y": {"field": "v"}, "color": {"field": "s"}}});
        assert!(err(spec).starts_with("encoding.color: 12 series is more than a chart can tell apart (at most 8)"));
        // Colouring bars by their own category adds nothing, so it isn't counted.
        let values: Vec<Value> = (0..12).map(|i| json!({"d": format!("d{i}"), "v": i})).collect();
        let spec = json!({"mark": "bar", "data": {"values": values},
            "encoding": {"x": {"field": "d"}, "y": {"field": "v"}, "color": {"field": "d"}}});
        assert!(parse_visual(&spec).is_ok());
    }

    #[test]
    fn types_are_inferred_from_the_data() {
        let spec = json!({"mark": "line", "data": {"values": [{"t": "2026-10-01", "v": 1}, {"t": "2026-10-02", "v": 2}]},
            "encoding": {"x": {"field": "t"}, "y": {"field": "v"}, "color": {"field": "v"}}});
        let v = parse_visual(&spec).unwrap();
        let PartKind::Chart(c) = &v.rows[0].kind else { panic!() };
        assert_eq!(c.encoding.x.as_ref().unwrap().kind, FieldType::Temporal);
        assert_eq!(c.encoding.y.as_ref().unwrap().kind, FieldType::Quantitative);
        // Colours split series: categories unless typed.
        assert_eq!(c.encoding.color.as_ref().unwrap().kind, FieldType::Nominal);
    }

    #[test]
    fn field_options() {
        let spec = json!({"mark": {"type": "bar"}, "height": 900, "title": {"text": "Load"},
        "data": {"values": [{"k": "a", "v": 1}]},
        "encoding": {
            "x": {"field": "k", "sort": "-y", "axis": null},
            "y": {"aggregate": "count", "axis": {"title": "Runs"}, "scale": {"domain": [0, 10], "zero": true}, "stack": "normalize"},
            "tooltip": [{"field": "k"}, {"field": "v", "type": "quantitative"}]
        }});
        let v = parse_visual(&spec).unwrap();
        let PartKind::Chart(c) = &v.rows[0].kind else { panic!() };
        assert_eq!(c.height, Some(600.0));
        assert_eq!(c.title.as_deref(), Some("Load"));
        let x = c.encoding.x.as_ref().unwrap();
        assert_eq!(x.sort, Sort::ByMeasure { descending: true });
        assert!(!x.shows_axis());
        let y = c.encoding.y.as_ref().unwrap();
        assert_eq!(y.field, None);
        assert_eq!(y.axis_title().as_deref(), Some("Runs"));
        assert_eq!(y.domain, Some(Domain::Range(0.0, 10.0)));
        assert_eq!(y.stack, Some(Stack::Normalize));
        assert_eq!(c.encoding.tooltip.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn part_errors() {
        assert_eq!(
            err(json!({"rows": [{"chart": bars(), "table": {"rows": []}}]})),
            "rows[0]: a part holds one thing, not both \"chart\" and \"table\": give each its own part"
        );
        assert_eq!(
            err(json!({"rows": [{"title": "x"}]})),
            "rows[0]: a part needs one of \"chart\", \"stats\", \"table\", \"text\" or \"columns\""
        );
        assert_eq!(
            err(json!({"rows": [{"stats": [{"label": "a", "value": true}]}]})),
            "rows[0].stats[0].value: must be a string or a number, not true"
        );
        assert_eq!(
            err(json!({"rows": [{"table": {"columns": ["a"], "rows": [["x", "y"]]}}]})),
            "rows[0].table.rows[0]: has 2 cells but the table has 1 columns"
        );
        assert_eq!(
            err(json!([1, 2])),
            "a visual is a JSON object: one chart with \"mark\", or {\"rows\": [...]} (got [1,2])"
        );
        assert_eq!(err(json!({"rows": []})), "rows: is empty: give at least one part");
    }

    #[test]
    fn aggregates() {
        let v = [3.0, 1.0, 2.0, 10.0];
        assert_eq!(Aggregate::Sum.apply(&v), 16.0);
        assert_eq!(Aggregate::Mean.apply(&v), 4.0);
        assert_eq!(Aggregate::Count.apply(&v), 4.0);
        assert_eq!(Aggregate::Min.apply(&v), 1.0);
        assert_eq!(Aggregate::Max.apply(&v), 10.0);
        assert_eq!(Aggregate::Median.apply(&v), 2.5);
        assert_eq!(Aggregate::Median.apply(&[5.0, 1.0, 3.0]), 3.0);
    }
}
