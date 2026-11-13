//! A laid-out chart as plain data: shapes and text in the chart's own
//! coordinates (0..width across, 0..height down), in the order they paint,
//! and the regions that show a tooltip. The UI paints it as it is.

/// A colour, each channel 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// `#rrggbb`, opaque.
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// The same colour at `a` of its own alpha.
    pub fn alpha(self, a: f32) -> Self {
        Self { a: self.a * a, ..self }
    }
}

/// What a chart is painted with. Series take the palette in order; text,
/// grid lines and the gaps between marks take the roles.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualTheme {
    /// Series colours, first series first. Never cycled: a chart has at most
    /// as many series as the palette has colours.
    pub palette: Vec<Rgba>,
    /// Tick labels and axis titles.
    pub axis_text: Rgba,
    /// Chart titles, legend labels and the numbers inside a donut.
    pub label_text: Rgba,
    /// Grid lines.
    pub grid: Rgba,
    /// The zero line and a band axis's base line.
    pub baseline: Rgba,
    /// The surface the chart sits on: the gaps between stacked bars and pie
    /// wedges, and the rings around dots, are painted in it.
    pub background: Rgba,
    /// Laid over a hovered bar or wedge.
    pub hover: Rgba,
}

impl VisualTheme {
    /// Eight hues ordered so neighbours stay apart for colour-blind readers.
    pub fn light() -> Self {
        Self {
            palette: [
                0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x008300, 0x4a3aa7, 0xe34948,
            ]
            .map(Rgba::hex)
            .to_vec(),
            axis_text: Rgba::hex(0x000000).alpha(0.5),
            label_text: Rgba::hex(0x000000).alpha(0.78),
            grid: Rgba::hex(0x000000).alpha(0.07),
            baseline: Rgba::hex(0x000000).alpha(0.2),
            background: Rgba::hex(0xffffff),
            hover: Rgba::hex(0xffffff).alpha(0.25),
        }
    }

    /// The same hues, stepped for a dark surface (green a step lighter, to
    /// stay 3:1 against `--bg-elevated`).
    pub fn dark() -> Self {
        Self {
            palette: [
                0x3987e5, 0xd95926, 0x199e70, 0xc98500, 0xd55181, 0x1f9a1f, 0x9085e9, 0xe66767,
            ]
            .map(Rgba::hex)
            .to_vec(),
            axis_text: Rgba::hex(0xffffff).alpha(0.5),
            label_text: Rgba::hex(0xffffff).alpha(0.82),
            grid: Rgba::hex(0xffffff).alpha(0.08),
            baseline: Rgba::hex(0xffffff).alpha(0.22),
            background: Rgba::hex(0x2a2a2d),
            hover: Rgba::hex(0xffffff).alpha(0.18),
        }
    }

    /// Series `i`'s colour.
    pub fn series(&self, i: usize) -> Rgba {
        if self.palette.is_empty() {
            return self.label_text;
        }
        self.palette[i % self.palette.len()]
    }
}

/// A rectangle: its top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    /// The part of this rectangle inside `other`; empty (zero-sized) when
    /// they don't meet.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }
}

/// Where a text item's `x` is: its start, middle or end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
}

/// One line of text. `y` is the middle of the line, so text centres on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub color: Rgba,
    pub anchor: Anchor,
    pub weight: Weight,
    /// Digits all one width, for numbers that line up.
    pub tabular: bool,
}

/// Something to paint.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Corner radii go top-left, top-right, bottom-right, bottom-left.
    Rect {
        rect: Rect,
        radius: [f32; 4],
        color: Rgba,
    },
    /// An open line through the points, round joins and caps.
    Line {
        points: Vec<[f32; 2]>,
        width: f32,
        color: Rgba,
    },
    /// A filled shape: an area under a line, a pie wedge.
    Polygon {
        points: Vec<[f32; 2]>,
        color: Rgba,
    },
    /// A dot: filled, outlined, or both. The outline is drawn outside `r`.
    Circle {
        x: f32,
        y: f32,
        r: f32,
        fill: Option<Rgba>,
        stroke: Option<(f32, Rgba)>,
    },
    Text(Text),
}

/// One line in a tooltip: a value, what it is, and the series colour.
#[derive(Debug, Clone, PartialEq)]
pub struct TipLine {
    pub label: String,
    pub value: String,
    pub color: Option<Rgba>,
}

/// A region that shows a tooltip while the pointer is over it.
#[derive(Debug, Clone, PartialEq)]
pub struct Hover {
    pub rect: Rect,
    /// A finer outline inside `rect` (a pie wedge), when the rectangle alone
    /// would take too much.
    pub shape: Option<Vec<[f32; 2]>>,
    /// The tooltip's first line: the category, date or series.
    pub title: String,
    pub lines: Vec<TipLine>,
    /// What to paint over the chart while hovered: a lightened bar, a guide
    /// line with the points on it.
    pub highlight: Vec<Item>,
}

/// A chart laid out at a width.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    pub hovers: Vec<Hover>,
}

impl Scene {
    /// The hover region under a point, the last laid out first (they're laid
    /// out so the more specific ones come later).
    pub fn hover_at(&self, x: f32, y: f32) -> Option<&Hover> {
        self.hovers
            .iter()
            .rev()
            .find(|h| h.rect.contains(x, y) && h.shape.as_ref().is_none_or(|s| inside(s, x, y)))
    }
}

/// Whether a point is inside a polygon (even-odd).
pub fn inside(polygon: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut odd = false;
    let n = polygon.len();
    for i in 0..n {
        let [xi, yi] = polygon[i];
        let [xj, yj] = polygon[(i + n - 1) % n];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            odd = !odd;
        }
    }
    odd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_alpha() {
        let c = Rgba::hex(0xff8000);
        assert_eq!((c.r, c.g, c.a), (1.0, 128.0 / 255.0, 1.0));
        assert_eq!(c.alpha(0.5).a, 0.5);
    }

    #[test]
    fn rectangles_meet() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            a.intersect(&Rect::new(5.0, 5.0, 10.0, 10.0)),
            Rect::new(5.0, 5.0, 5.0, 5.0)
        );
        assert_eq!(a.intersect(&Rect::new(20.0, 0.0, 5.0, 5.0)).w, 0.0);
        assert!(a.contains(10.0, 0.0));
        assert!(!a.contains(10.1, 0.0));
    }

    #[test]
    fn points_in_a_triangle() {
        let tri = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
        assert!(inside(&tri, 2.0, 2.0));
        assert!(!inside(&tri, 8.0, 8.0));
    }
}
