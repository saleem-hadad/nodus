//! Spellcheck underlines in text fields (spec 01 PLAT-137): Chromium's
//! default, kept by every field that doesn't say `spellCheck={false}` (the
//! action dialog's, the capture box). Misspelled words, as the OS's
//! spellchecker finds them (`nodus_platform::spell`), get a dotted red line
//! under them once the field has had the keyboard, except the word the
//! caret is still at the end of, as Chromium waits for the word to end.

use std::ops::Range;

use gpui::base::input::{InputBaseState, InputModeKind};
use gpui::{
    App, Bounds, ElementId, Entity, EntityInputHandler as _, Focusable as _, IntoElement, Pixels, Styled, Window,
    canvas, point, px, size,
};

/// What a field's text was last checked as.
#[derive(Default)]
struct Spelling {
    text: String,
    misspelled: Vec<Range<usize>>,
    /// Chromium checks a field once it has had the keyboard.
    focused_once: bool,
}

/// The dots' colour: AppKit's system red.
const DOT: u32 = 0xff3b30ff;

/// Draws `field`'s spellcheck underlines; hang it anywhere in the field's
/// frame. `font_size` and `line_height` are the field's, for the baseline.
pub fn underlines<M: InputModeKind + 'static>(
    id: ElementId,
    field: &Entity<InputBaseState<M>>,
    font_size: Pixels,
    line_height: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<M> {
    let memo = window.use_keyed_state(id, cx, |_, _| Spelling::default());
    let field = field.clone();
    canvas(
        |_, _, _| (),
        move |_, (), window, cx| {
            let focused = field.read(cx).focus_handle(cx).is_focused(window);
            let (text, caret) = {
                let f = field.read(cx);
                let text = f.value().to_string();
                let caret = nodus_domain::js::utf16_len(text.get(..f.cursor()).unwrap_or(&text));
                (text, caret)
            };
            let misspelled = memo.update(cx, |m, _| {
                m.focused_once |= focused;
                if !m.focused_once {
                    return Vec::new();
                }
                if m.text != text {
                    m.misspelled = nodus_platform::spell::misspelled(&text);
                    m.text = text.clone();
                }
                m.misspelled.clone()
            });
            let Some(text_bounds) = field.read(cx).text_bounds() else {
                return;
            };
            for range in misspelled {
                if focused && range.end == caret {
                    continue;
                }
                let line = field.update(cx, |f, cx| f.bounds_for_range(range, text_bounds, window, cx));
                if let Some(line) = line {
                    paint_dots(line, font_size, line_height, window);
                }
            }
        },
    )
    .absolute()
    .size_0()
}

/// A row of round dots under a word: 2px across, 3px apart, just under the
/// baseline of a line `line_height` tall whose text is `font_size`.
fn paint_dots(word: Bounds<Pixels>, font_size: Pixels, line_height: Pixels, window: &mut Window) {
    let line_top = word.bottom() - line_height;
    // The system font's ascent is about 0.95em, in a content box 1.2em tall
    // centred in the line.
    let baseline = line_top + (line_height - font_size * 1.2) / 2. + font_size * 0.95;
    let d = px(2.);
    let mut x = word.left();
    while x + d <= word.right() {
        window.paint_quad(
            gpui::fill(Bounds::new(point(x, baseline + px(1.5)), size(d, d)), gpui::rgba(DOT)).corner_radii(px(1.)),
        );
        x += px(3.);
    }
}
