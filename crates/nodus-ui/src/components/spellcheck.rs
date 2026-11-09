//! Spell checking in text fields, as Chromium did it in 0.9.4 (spec 01 PLAT-137;
//! BAR-005, the capture field, notes, the merge dialog's instruction, the
//! action dialog): misspelt words get a row of red dots, checked by the
//! system's checker (`nodus_platform::spell`).
//!
//! When, as Electron 44.5.1 showed it (a hidden page typed into through
//! the input method):
//! - The text is checked shortly after an edit (Blink's "hot mode"), unless
//!   the caret sits in the word the last keystroke changed: while a word is
//!   being typed nothing is judged, so `trimms` gets its dots once a space
//!   follows it or the caret moves away (an arrow, a click).
//! - An edit drops the dots on the word it touches; the others stay put.
//! - Text the field was given rather than typed (a note's, a restored
//!   draft) isn't judged until it's edited: no dots appeared there, even
//!   focused and left idle.
//! - A multi-line field keeps its dots when it loses the focus; a
//!   single-line one (`<input>`) drops them (Blink's
//!   `DidEndEditingOnTextField`).
//! - No suggestions menu: 0.9.4 never handled `context-menu` (PLAT-138).
//!
//! How they look, measured from Electron 44.5.1 at 2×: round dots about
//! 2px across, one every 4px from half a pixel left of the word's edge
//! (the first one cut at that edge), the last starting before the word
//! ends, centred 2px below the baseline, `#ff3b30` at 75% on any
//! background.

use std::ops::Range;
use std::time::Duration;

use gpui::component::input::{InputEvent, InputState, TextareaState};
use gpui::{
    App, AppContext as _, Bounds, ContentMask, Context, Entity, EntityInputHandler as _, IntoElement, Pixels, Rgba,
    Styled as _, Subscription, Task, Window, canvas, fill, point, px, size,
};
use nodus_domain::js;

/// The dots' colour: `#ff3b30` (macOS's red) at 75% in Chromium. GPUI's
/// 2px round quad covers its edge pixels more than Skia's slightly smaller
/// circle does, so it's drawn at 68%: the same ink per dot, the centre a
/// shade lighter.
const DOT: Rgba = Rgba {
    r: 1.0,
    g: 59.0 / 255.0,
    b: 48.0 / 255.0,
    a: 0.68,
};
/// After an edit, before the text is checked.
const HOT_DELAY: Duration = Duration::from_millis(50);

/// A field's text, caret (bytes) and whether its selection is collapsed.
type Snapshot = (String, usize, bool);

/// A field's spelling state: the misspelt words, as byte ranges of `text`.
pub struct Spellcheck {
    text: String,
    markers: Vec<Range<usize>>,
    caret: usize,
    /// The last change was a keystroke at the caret: the word there is
    /// being typed and isn't judged yet.
    typing: bool,
    single_line: bool,
    hot: Option<Task<()>>,
    /// Asks the field to draw again when the dots change.
    redraw: Box<dyn Fn(&mut App)>,
    /// What the field's text and caret are now.
    read: Box<dyn Fn(&App) -> Snapshot>,
    _subscriptions: Vec<Subscription>,
}

/// A text field spell checking can follow: gpui-component's multi-line
/// and single-line fields.
pub trait SpellField: Clone + 'static {
    /// The text, the caret (bytes) and whether the selection is collapsed.
    fn snapshot(&self, cx: &App) -> (String, usize, bool);
    /// Where the text was laid out last, the field's frame and its line height.
    fn geometry(&self, cx: &App) -> Option<(Bounds<Pixels>, Bounds<Pixels>, Pixels)>;
    /// Where the UTF-16 `range` sits (one line, the line's height tall).
    fn word_bounds(
        &self,
        range: Range<usize>,
        text: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>>;
    fn notify(&self, cx: &mut App);
    fn watch(&self, window: &mut Window, cx: &mut Context<'_, Spellcheck>) -> Subscription;
}

macro_rules! spell_field {
    ($state:ty) => {
        impl SpellField for Entity<$state> {
            fn snapshot(&self, cx: &App) -> (String, usize, bool) {
                let state = self.read(cx);
                let selected = state.selected_range();
                (
                    state.value().to_string(),
                    state.cursor(),
                    selected.start == selected.end,
                )
            }
            fn geometry(&self, cx: &App) -> Option<(Bounds<Pixels>, Bounds<Pixels>, Pixels)> {
                let state = self.read(cx);
                Some((state.text_bounds()?, state.input_bounds(), state.line_height()?))
            }
            fn word_bounds(
                &self,
                range: Range<usize>,
                text: Bounds<Pixels>,
                window: &mut Window,
                cx: &mut App,
            ) -> Option<Bounds<Pixels>> {
                self.update(cx, |f, cx| f.bounds_for_range(range, text, window, cx))
            }
            fn notify(&self, cx: &mut App) {
                self.update(cx, |_, cx| cx.notify());
            }
            fn watch(&self, window: &mut Window, cx: &mut Context<'_, Spellcheck>) -> Subscription {
                cx.subscribe_in(
                    self,
                    window,
                    |this: &mut Spellcheck, _, event: &InputEvent, _, cx| match event {
                        InputEvent::Change => this.changed(cx),
                        InputEvent::Blur => this.blurred(cx),
                        InputEvent::Focus | InputEvent::PressEnter { .. } => {}
                    },
                )
            }
        }
    };
}

spell_field!(TextareaState);
spell_field!(InputState);

/// Spell checking for `field`; `single_line` for an `<input>`, whose dots
/// go when it loses the focus. Draw them with [`underlines`].
pub fn spellcheck<F: SpellField>(
    field: &F,
    single_line: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Spellcheck> {
    let field = field.clone();
    cx.new(|cx| {
        let subscription = field.watch(window, cx);
        let (text, caret, _) = field.snapshot(cx);
        let (read, redraw) = (field.clone(), field.clone());
        Spellcheck {
            text,
            markers: Vec::new(),
            caret,
            typing: false,
            single_line,
            hot: None,
            redraw: Box::new(move |cx| redraw.notify(cx)),
            read: Box::new(move |cx| read.snapshot(cx)),
            _subscriptions: vec![subscription],
        }
    })
}

impl Spellcheck {
    /// The misspelt words now, as UTF-16 ranges of the field's text.
    pub fn markers_utf16(&self) -> Vec<Range<usize>> {
        self.markers
            .iter()
            .map(|r| js::byte_to_utf16(&self.text, r.start)..js::byte_to_utf16(&self.text, r.end))
            .collect()
    }

    fn changed(&mut self, cx: &mut Context<'_, Self>) {
        let (text, caret, collapsed) = (self.read)(cx);
        if text == self.text {
            return;
        }
        let edit = edited(&self.text, &text);
        self.markers = shift_markers(&self.markers, &edit, text.len());
        // A keystroke at the caret: the word there is being typed.
        self.typing = collapsed && caret == edit.new.end;
        self.text = text;
        self.caret = caret;
        self.schedule_hot(cx);
        (self.redraw)(cx);
    }

    /// A single-line field's dots go when it loses the focus.
    fn blurred(&mut self, cx: &mut Context<'_, Self>) {
        if self.single_line && !self.markers.is_empty() {
            self.markers.clear();
            (self.redraw)(cx);
        }
    }

    /// The caret moved without an edit: the word it left is judged now.
    fn sync_caret(&mut self, cx: &mut Context<'_, Self>) {
        let (text, caret, _) = (self.read)(cx);
        if text != self.text || caret == self.caret {
            return;
        }
        self.caret = caret;
        if self.typing {
            self.typing = false;
            self.schedule_hot(cx);
        }
    }

    fn schedule_hot(&mut self, cx: &mut Context<'_, Self>) {
        self.hot = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOT_DELAY).await;
            this.update(cx, Spellcheck::check).ok();
        }));
    }

    fn check(&mut self, cx: &mut Context<'_, Self>) {
        // A word is being typed: nothing is judged until it's finished.
        if self.typing && word_at(&self.text, self.caret).is_some() {
            return;
        }
        let markers: Vec<Range<usize>> = nodus_platform::spell::misspelled(&self.text)
            .into_iter()
            .map(|r| js::utf16_to_byte(&self.text, r.start)..js::utf16_to_byte(&self.text, r.end))
            .collect();
        if markers != self.markers {
            self.markers = markers;
            (self.redraw)(cx);
        }
    }
}

/// What an edit changed: `old` in the old text became `new` in the new one
/// (byte ranges), found as what's left between the common start and end.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edit {
    old: Range<usize>,
    new: Range<usize>,
}

fn edited(old: &str, new: &str) -> Edit {
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);
    // Keep the prefix on a char boundary of both, then match from the end.
    let prefix = (0..=prefix)
        .rev()
        .find(|&i| old.is_char_boundary(i) && new.is_char_boundary(i))
        .unwrap_or(0);
    let max_suffix = (old.len() - prefix).min(new.len() - prefix);
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .scan(0, |sum, n| {
            *sum += n;
            Some(*sum)
        })
        .take_while(|&s| s <= max_suffix)
        .last()
        .unwrap_or(0);
    Edit {
        old: prefix..old.len() - suffix,
        new: prefix..new.len() - suffix,
    }
}

/// The markers after an edit: those touching the edited stretch go (the
/// word they marked changed), those after it move with the text.
fn shift_markers(markers: &[Range<usize>], edit: &Edit, new_len: usize) -> Vec<Range<usize>> {
    markers
        .iter()
        .filter_map(|m| {
            if m.end < edit.old.start {
                Some(m.clone())
            } else if m.start > edit.old.end {
                let start = m.start - edit.old.end + edit.new.end;
                let end = m.end - edit.old.end + edit.new.end;
                (end <= new_len).then_some(start..end)
            } else {
                None
            }
        })
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’'
}

/// The word the caret is at the end of, or inside.
fn word_at(text: &str, caret: usize) -> Option<Range<usize>> {
    if caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }
    let start = text[..caret]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(caret, |(i, _)| i);
    let end = text[caret..]
        .char_indices()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(caret, |(i, c)| caret + i + c.len_utf8());
    (start < end).then_some(start..end)
}

/// The dots under the misspelt words of `field`, drawn over it: put this
/// right after the field, in the same parent (whose text style gives the
/// font; `font_size` is the field's). `scale` is the zoom the field is
/// drawn at (a note on a zoomed board), 1 elsewhere.
pub fn underlines<F: SpellField>(
    spell: &Entity<Spellcheck>,
    field: &F,
    font_size: Pixels,
    scale: f32,
) -> impl IntoElement {
    let spell = spell.clone();
    let field = field.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            spell.update(cx, Spellcheck::sync_caret);
            let markers = spell.read(cx).markers_utf16();
            if markers.is_empty() {
                return;
            }
            let Some((text_bounds, clip, line_height)) = field.geometry(cx) else {
                return;
            };
            let style = window.text_style();
            let font_id = window.text_system().resolve_font(&style.font());
            let ascent = window.text_system().ascent(font_id, font_size);
            // GPUI's descent is the font's descender, negative.
            let descent = window.text_system().descent(font_id, font_size).abs();
            let device = window.scale_factor();
            for range in markers {
                let Some(word) = field.word_bounds(range, text_bounds, window, cx) else {
                    continue;
                };
                let top = word.top();
                let baseline = top + gpui::chromium_baseline_offset(top, line_height, ascent, descent, device);
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: clip.intersect(&Bounds::from_corners(
                            point(word.left(), clip.top()),
                            point(word.right() + px(4. * scale), clip.bottom()),
                        )),
                    }),
                    |window| paint_dots(window, word.left(), word.size.width, baseline, scale),
                );
            }
        },
    )
    .absolute()
    .size_full()
}

/// The dots for a word `width` wide from `left`, on `baseline`: 2px round
/// in a cell 1px below the baseline.
fn paint_dots(window: &mut Window, left: Pixels, width: Pixels, baseline: Pixels, scale: f32) {
    let d = px(2. * scale);
    for x in dot_lefts(f32::from(left), f32::from(width), scale) {
        window.paint_quad(
            fill(
                Bounds {
                    origin: point(px(x), baseline + px(scale)),
                    size: size(d, d),
                },
                DOT,
            )
            .corner_radii(d / 2.),
        );
    }
}

/// Where each dot starts: every 4px from half a pixel left of the word,
/// while a dot starts before the word ends.
fn dot_lefts(left: f32, width: f32, scale: f32) -> Vec<f32> {
    let period = 4. * scale;
    let first = left - 0.5 * scale;
    (0..)
        .map(|k| first + period * k as f32)
        .take_while(|&x| x < left + width)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_is_what_changed_between_the_common_ends() {
        assert_eq!(edited("helo world", "hello world"), Edit { old: 3..3, new: 3..4 });
        assert_eq!(edited("abc", "abc "), Edit { old: 3..3, new: 3..4 });
        assert_eq!(edited("abc d", "abd"), Edit { old: 2..4, new: 2..2 });
        assert_eq!(edited("aaa", "aa"), Edit { old: 2..3, new: 2..2 });
        assert_eq!(edited("é🙂x", "é🙂yx"), Edit { old: 6..6, new: 6..7 });
    }

    #[test]
    fn markers_touching_an_edit_go_and_later_ones_move() {
        let markers = vec![0..4, 5..9, 12..16];
        // "teh coupn and feild" with "coupn" edited at its end.
        let edit = Edit { old: 9..9, new: 9..10 };
        assert_eq!(shift_markers(&markers, &edit, 21), vec![0..4, 13..17]);
    }

    #[test]
    fn the_word_at_the_caret() {
        assert_eq!(word_at("Fix coupn", 9), Some(4..9));
        assert_eq!(word_at("Fix coupn ", 10), None);
        assert_eq!(word_at("Fix coupn", 6), Some(4..9));
        assert_eq!(word_at("don't", 5), Some(0..5));
    }

    #[test]
    fn dots_start_half_a_pixel_left_and_stop_inside_the_word() {
        // A 36.5px word (13px "coupn" in Electron): ten dots, 4px apart.
        let lefts = dot_lefts(41.0, 36.5, 1.0);
        assert_eq!(lefts.len(), 10);
        assert_eq!(lefts[0], 40.5);
        assert_eq!(lefts[9], 76.5);
        assert_eq!(dot_lefts(10.0, 6.0, 2.0), vec![9.0]);
        assert_eq!(dot_lefts(10.0, 9.0, 2.0), vec![9.0, 17.0]);
    }
}
