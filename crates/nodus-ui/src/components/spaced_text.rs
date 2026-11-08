//! One line of text with CSS `letter-spacing`, which GPUI's text style lacks.
//! The headings use it: −0.01em on the title bar's project name and group
//! names, −0.02em on the Projects heading. Like CSS, the spacing follows every
//! character, the last one too, and the line ends in an ellipsis when it
//! doesn't fit.

use gpui::{
    App, Bounds, Element, ElementId, Font, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels,
    ShapedLine, SharedString, Size, Style, TextRun, Window, px, relative,
};

/// `text` in `font` at `size`, its characters `spacing` apart beyond their
/// own advance, on a line `line_height` tall.
pub fn spaced_text(
    text: impl Into<SharedString>,
    font: Font,
    size: Pixels,
    spacing: Pixels,
    line_height: Pixels,
    color: impl Into<Hsla>,
) -> SpacedText {
    SpacedText {
        text: text.into(),
        font,
        size,
        spacing,
        line_height,
        color: color.into(),
        shrink: false,
    }
}

pub struct SpacedText {
    text: SharedString,
    font: Font,
    size: Pixels,
    spacing: Pixels,
    line_height: Pixels,
    color: Hsla,
    shrink: bool,
}

impl SpacedText {
    /// Lets the line shrink below its width, cutting it with an ellipsis
    /// (`min-width: 0; text-overflow: ellipsis`).
    pub fn truncate(mut self) -> Self {
        self.shrink = true;
        self
    }

    fn shape(&self, text: &str, window: &mut Window) -> ShapedLine {
        let run = TextRun {
            len: text.len(),
            font: self.font.clone(),
            color: self.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(SharedString::from(text.to_owned()), self.size, &[run], None)
    }

    /// The line's width with the spacing.
    fn width(&self, line: &ShapedLine, text: &str) -> Pixels {
        line.width + self.spacing * text.chars().count() as f32
    }

    /// The text that fits `width`: all of it, or as much as fits before `…`.
    fn fitted(&self, width: Pixels, window: &mut Window) -> (SharedString, ShapedLine) {
        let whole = self.shape(&self.text, window);
        if self.width(&whole, &self.text) <= width + px(0.5) {
            return (self.text.clone(), whole);
        }
        let chars: Vec<char> = self.text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            let line = self.shape(&candidate, window);
            if self.width(&line, &candidate) <= width + px(0.5) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let text: String = chars[..lo].iter().collect::<String>() + "…";
        let line = self.shape(&text, window);
        (text.into(), line)
    }
}

impl IntoElement for SpacedText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub struct Prepaint {
    line: ShapedLine,
    text: SharedString,
}

impl Element for SpacedText {
    type RequestLayoutState = ();
    type PrepaintState = Option<Prepaint>;

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
        let whole = self.shape(&self.text, window);
        let width = self.width(&whole, &self.text);
        let mut style = Style::default();
        style.size.width = width.into();
        style.size.height = self.line_height.into();
        style.flex_shrink = if self.shrink { 1.0 } else { 0.0 };
        if self.shrink {
            style.min_size.width = px(0.).into();
            style.max_size.width = relative(1.).into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Option<Prepaint> {
        let (text, line) = self.fitted(bounds.size.width, window);
        Some(Prepaint { line, text })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Option<Prepaint>,
        window: &mut Window,
        _: &mut App,
    ) {
        let Some(Prepaint { line, text }) = prepaint.take() else {
            return;
        };
        let padding_top = (self.line_height - line.ascent - line.descent) / 2.;
        let baseline = bounds.origin.y + padding_top + line.ascent;
        // Characters before each glyph, to place it with the spacing.
        let offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        for run in &line.runs {
            for glyph in &run.glyphs {
                let before = offsets.partition_point(|&i| i < glyph.index);
                let x = bounds.origin.x + glyph.position.x + self.spacing * before as f32;
                let origin = gpui::point(x, baseline + glyph.position.y);
                if glyph.is_emoji {
                    let _ = window.paint_emoji(origin, run.font_id, glyph.id, line.font_size);
                } else {
                    let _ = window.paint_glyph(origin, run.font_id, glyph.id, line.font_size, self.color);
                }
            }
        }
    }
}

/// The size a spaced line takes, for callers laying out around it.
pub fn measure(text: &str, font: Font, size: Pixels, spacing: Pixels, window: &mut Window) -> Size<Pixels> {
    let t = spaced_text(text.to_owned(), font, size, spacing, size, gpui::black());
    let line = t.shape(text, window);
    Size {
        width: t.width(&line, text),
        height: size,
    }
}
