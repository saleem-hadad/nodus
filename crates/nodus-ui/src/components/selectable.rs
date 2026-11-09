//! Text the user can select and copy outside fields: `global.css` makes
//! the page `user-select: none` and opts some text back in (`.selectable`,
//! `.diff-text`, `.view-approve-cmd`). GPUI draws text that can't be
//! selected, so these do it: a press and drag selects, a double click
//! selects a word, a triple click the line, ⇧-click extends, ⌘C / Ctrl+C
//! (and Edit › Copy) copies, Select All takes the whole block, and a press
//! anywhere else clears it, as Chromium's page selection does.
//!
//! A [`TextSelection`] is one selection shared by a block of texts (the
//! diff's lines, a transcript's messages), so a drag runs across them and a
//! copy joins them with line breaks; [`selection_scope`] is the element
//! around the block, and each text in it is a [`selectable_part`]. A lone
//! text is [`selectable`].

use std::collections::BTreeMap;
use std::ops::Range;

use gpui::component::input::{Copy, SelectAll};
use gpui::{
    App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Div, ElementId, Entity, FocusHandle, HighlightStyle, Hsla,
    InteractiveElement, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, SharedString, Styled,
    StyledText, TextLayout, Window, canvas, div,
};

/// Where a selection end is: which text of the block, and the byte offset in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPoint {
    pub part: usize,
    pub offset: usize,
}

/// The selection a block of texts shares.
pub struct TextSelection {
    focus: FocusHandle,
    anchor: Option<TextPoint>,
    head: Option<TextPoint>,
    dragging: bool,
    /// Every text of the block drawn so far, for copying across texts that
    /// have scrolled away.
    texts: BTreeMap<usize, SharedString>,
    /// The texts drawn in the last frame, in the order they were drawn.
    visible: Vec<(usize, TextLayout)>,
    /// What goes between two texts' pieces in a copy: a line break for
    /// blocks of lines, nothing for spans in one line.
    separator: &'static str,
}

impl TextSelection {
    pub fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            anchor: None,
            head: None,
            dragging: false,
            texts: BTreeMap::new(),
            visible: Vec::new(),
            separator: "\n",
        }
    }

    /// A selection over spans of one line, copied without line breaks.
    pub fn inline(mut self) -> Self {
        self.separator = "";
        self
    }

    /// The selection's ends, first one first; none when nothing is selected.
    pub fn ends(&self) -> Option<(TextPoint, TextPoint)> {
        let (a, h) = (self.anchor?, self.head?);
        (a != h).then(|| if a <= h { (a, h) } else { (h, a) })
    }

    /// What's selected of text `part` (`len` bytes long).
    pub fn range_in(&self, part: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ends()?;
        if part < start.part || part > end.part {
            return None;
        }
        let from = if part == start.part { start.offset.min(len) } else { 0 };
        let to = if part == end.part { end.offset.min(len) } else { len };
        (from < to).then_some(from..to)
    }

    /// The selected text: the parts' pieces joined with line breaks.
    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.ends() else {
            return String::new();
        };
        let mut pieces = Vec::new();
        for (&part, text) in self.texts.range(start.part..=end.part) {
            if let Some(range) = self.range_in(part, text.len()) {
                pieces.push(text.get(range).unwrap_or_default().to_owned());
            } else if part > start.part && part < end.part {
                pieces.push(String::new());
            }
        }
        pieces.join(self.separator)
    }

    /// Selects from `anchor` to `head`.
    pub fn select(&mut self, anchor: TextPoint, head: TextPoint) {
        self.anchor = Some(anchor);
        self.head = Some(head);
    }

    /// Selects nothing.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.dragging = false;
    }

    /// Selects every text of the block.
    pub fn select_all(&mut self) {
        let first = self.texts.keys().next().copied();
        let last = self.texts.iter().next_back().map(|(&p, t)| (p, t.len()));
        if let (Some(first), Some((last, len))) = (first, last) {
            self.select(
                TextPoint { part: first, offset: 0 },
                TextPoint {
                    part: last,
                    offset: len,
                },
            );
        }
    }

    /// The point under `position`, among the texts drawn last: inside a
    /// text, its nearest character boundary; above or between texts, the
    /// start of the next; below them all, the end of the last.
    fn point_at(&self, position: Point<Pixels>) -> Option<TextPoint> {
        // The texts on the pointer's row: the last one starting left of it.
        let on_row = self
            .visible
            .iter()
            .filter(|(_, layout)| {
                let b: Bounds<Pixels> = layout.bounds();
                b.top() <= position.y && position.y < b.bottom()
            })
            .fold(None, |found: Option<&(usize, TextLayout)>, candidate| match found {
                Some(_) if candidate.1.bounds().left() > position.x => found,
                _ => Some(candidate),
            });
        if let Some((part, layout)) = on_row {
            return Some(TextPoint {
                part: *part,
                offset: index_at(layout, position),
            });
        }
        // Between rows: the start of the next text, or the end of the last.
        let mut last = None;
        for (part, layout) in &self.visible {
            if position.y < layout.bounds().top() {
                return Some(TextPoint { part: *part, offset: 0 });
            }
            last = Some((*part, layout.len()));
        }
        last.map(|(part, len)| TextPoint { part, offset: len })
    }
}

/// The character boundary nearest `position` in a laid-out text.
fn index_at(layout: &TextLayout, position: Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(ix) | Err(ix) => ix,
    }
}

/// A block's selection, kept across frames under `id`.
pub fn text_selection(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<TextSelection> {
    window.use_keyed_state(id.into(), cx, |_, cx| TextSelection::new(cx))
}

/// The key context of a block of selectable text, which [`bind_keys`] gives
/// Copy and Select All.
const KEY_CONTEXT: &str = "TextSelection";

/// ⌘C / Ctrl+C copies a block's selection and ⌘A / Ctrl+A selects all of
/// it, once the block has the keyboard (a text in it was pressed).
pub fn bind_keys(cx: &mut App) {
    let (copy, all) = if cfg!(target_os = "macos") {
        ("cmd-c", "cmd-a")
    } else {
        ("ctrl-c", "ctrl-a")
    };
    cx.bind_keys([
        gpui::KeyBinding::new(copy, Copy, Some(KEY_CONTEXT)),
        gpui::KeyBinding::new(all, SelectAll, Some(KEY_CONTEXT)),
    ]);
}

/// The colour selected text sits on: the text fields' (gpui-component's theme).
fn selection_color(cx: &App) -> Hsla {
    gpui::component::theme::Theme::global(cx).selection
}

/// The element around a block of selectable texts: it takes the keyboard
/// once a text is pressed (so ⌘C and Edit › Copy reach it), clears the
/// selection on a press anywhere else, and follows a drag past the texts.
pub fn selection_scope(selection: &Entity<TextSelection>, cx: &App) -> gpui::Stateful<Div> {
    let s = selection.read(cx);
    let focus = s.focus.clone();
    let dragging = s.dragging;
    let id = ElementId::from(SharedString::from(format!("text-selection-{}", selection.entity_id())));
    let copy = selection.clone();
    let all = selection.clone();
    let press = selection.clone();
    let out = selection.clone();
    let frame = selection.clone();
    div()
        .id(id)
        .key_context(KEY_CONTEXT)
        .track_focus(&focus)
        // A press anywhere in the block clears it first; a text's own press
        // then starts a new one.
        .capture_any_mouse_down(move |e, _, cx| {
            press.update(cx, |s, cx| {
                // ⇧-click extends what's there.
                if s.ends().is_some() && !e.modifiers.shift {
                    s.clear();
                    cx.notify();
                }
            });
        })
        .on_mouse_down_out(move |_, _, cx| {
            out.update(cx, |s, cx| {
                if s.ends().is_some() || s.dragging {
                    s.clear();
                    cx.notify();
                }
            });
        })
        .on_action(move |_: &Copy, _, cx| {
            let text = copy.read(cx).selected_text();
            if text.is_empty() {
                cx.propagate();
            } else {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        })
        .on_action(move |_: &SelectAll, _, cx| {
            all.update(cx, |s, cx| {
                s.select_all();
                cx.notify();
            });
        })
        .child(
            // First of the block's children: forgets last frame's texts
            // before they're drawn again, and while a drag runs, follows the
            // pointer anywhere in the window.
            canvas(
                {
                    let frame = frame.clone();
                    move |_, _, cx| frame.update(cx, |s, _| s.visible.clear())
                },
                move |_, (), window, _| {
                    if !dragging {
                        return;
                    }
                    let follow = frame.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        follow.update(cx, |s, cx| {
                            if !s.dragging {
                                return;
                            }
                            if e.pressed_button != Some(MouseButton::Left) {
                                s.dragging = false;
                                return;
                            }
                            if let Some(point) = s.point_at(e.position)
                                && s.head != Some(point)
                            {
                                s.head = Some(point);
                                cx.notify();
                            }
                        });
                    });
                    let release = frame.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            release.update(cx, |s, _| s.dragging = false);
                        }
                    });
                },
            )
            .absolute()
            .size_0(),
        )
}

/// Text `part` of a block: `text` drawn in the inherited text style with
/// `highlights` (sorted, not overlapping) and the selection over them.
pub fn selectable_part(
    selection: &Entity<TextSelection>,
    part: usize,
    text: impl Into<SharedString>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    cx: &mut App,
) -> gpui::Stateful<Div> {
    let text: SharedString = text.into();
    let color = selection_color(cx);
    let range = selection.update(cx, |s, _| {
        s.texts.insert(part, text.clone());
        s.range_in(part, text.len())
    });
    let styled = StyledText::new(text.clone()).with_highlights(with_selection(highlights, range, color));
    let layout = styled.layout().clone();
    let id = ElementId::from(SharedString::from(format!(
        "text-selection-{}-{part}",
        selection.entity_id()
    )));
    let press = selection.clone();
    let seen = selection.clone();
    let seen_layout = layout.clone();
    let text_for_press = text.clone();
    div()
        .id(id)
        .relative()
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |e, window, cx| {
            let offset = index_at(&layout, e.position);
            let at = |offset| TextPoint { part, offset };
            press.update(cx, |s, cx| {
                match e.click_count {
                    1 => {
                        let anchor = if e.modifiers.shift { s.anchor } else { None };
                        s.select(anchor.unwrap_or(at(offset)), at(offset));
                        s.dragging = true;
                    }
                    2 => {
                        let word = word_at(&text_for_press, offset);
                        s.select(at(word.start), at(word.end));
                        s.dragging = false;
                    }
                    _ => {
                        let line = line_at(&text_for_press, offset);
                        s.select(at(line.start), at(line.end));
                        s.dragging = false;
                    }
                }
                window.focus(&s.focus, cx);
                cx.notify();
            });
        })
        .child(styled)
        .child(
            canvas(
                move |_, _, cx| seen.update(cx, |s, _| s.visible.push((part, seen_layout))),
                |_, (), _, _| {},
            )
            .absolute()
            .size_0(),
        )
}

/// A lone selectable text in the inherited text style (`.selectable`).
pub fn selectable(id: impl Into<ElementId>, text: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> Div {
    let selection = text_selection(id, window, cx);
    let part = selectable_part(&selection, 0, text, Vec::new(), cx);
    div().child(selection_scope(&selection, cx).child(part))
}

/// A lone selectable text as an element, for places drawn without the
/// window at hand: [`selectable`] once it's drawn.
#[derive(gpui::IntoElement)]
pub struct SelectableText {
    id: ElementId,
    text: SharedString,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

/// [`SelectableText`]: `text`, selectable, kept under `id`.
pub fn selectable_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> SelectableText {
    SelectableText {
        id: id.into(),
        text: text.into(),
        highlights: Vec::new(),
    }
}

impl SelectableText {
    /// Ranges drawn in their own style under the selection (sorted, not
    /// overlapping): search matches, say.
    pub fn highlights(mut self, highlights: Vec<(Range<usize>, HighlightStyle)>) -> Self {
        self.highlights = highlights;
        self
    }
}

impl gpui::RenderOnce for SelectableText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl gpui::IntoElement {
        let selection = text_selection(self.id, window, cx);
        let part = selectable_part(&selection, 0, self.text, self.highlights, cx);
        div().child(selection_scope(&selection, cx).child(part))
    }
}

/// A selection over spans of one line (a path drawn as folder and name),
/// kept across frames under `id`.
pub fn inline_selection(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<TextSelection> {
    window.use_keyed_state(id.into(), cx, |_, cx| TextSelection::new(cx).inline())
}

/// `highlights` with the selection drawn over them: sorted ranges that
/// don't overlap, as `StyledText::with_highlights` takes them.
pub fn with_selection(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    selection: Option<Range<usize>>,
    color: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(sel) = selection.filter(|r| !r.is_empty()) else {
        return highlights;
    };
    let selected = HighlightStyle {
        background_color: Some(color),
        ..HighlightStyle::default()
    };
    // Every boundary, then each piece between two takes what covers it.
    let mut cuts: Vec<usize> = highlights
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .chain([sel.start, sel.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for pair in cuts.windows(2) {
        let piece = pair[0]..pair[1];
        let base = highlights
            .iter()
            .find(|(r, _)| r.start <= piece.start && piece.end <= r.end)
            .map(|(_, style)| *style);
        let in_selection = sel.start <= piece.start && piece.end <= sel.end;
        let style = match (base, in_selection) {
            (Some(base), true) => Some(base.highlight(selected)),
            (Some(base), false) => Some(base),
            (None, true) => Some(selected),
            (None, false) => None,
        };
        if let Some(style) = style {
            out.push((piece, style));
        }
    }
    out
}

/// The word around byte `ix`, as a double click takes it: a run of letters,
/// digits and `_`, a run of spaces, or one other character.
pub fn word_at(text: &str, ix: usize) -> Range<usize> {
    let ix = floor_boundary(text, ix.min(text.len()));
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    // The character under the pointer, or the one before at the end.
    let Some((at, c)) = text[ix..]
        .char_indices()
        .next()
        .map(|(i, c)| (ix + i, c))
        .or_else(|| text[..ix].char_indices().next_back())
    else {
        return ix..ix;
    };
    let kind = class(c);
    if kind == 2 {
        return at..at + c.len_utf8();
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| class(c) == kind)
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|&(_, c)| class(c) != kind)
        .map_or(text.len(), |(i, _)| at + i);
    start..end
}

/// The line around byte `ix`, as a triple click takes it.
pub fn line_at(text: &str, ix: usize) -> Range<usize> {
    let ix = floor_boundary(text, ix.min(text.len()));
    let start = text[..ix].rfind('\n').map_or(0, |i| i + 1);
    let end = text[ix..].find('\n').map_or(text.len(), |i| ix + i);
    start..end
}

fn floor_boundary(text: &str, mut ix: usize) -> usize {
    while !text.is_char_boundary(ix) {
        ix -= 1;
    }
    ix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_words_spaces_and_lines() {
        let text = "Error: git_status failed  twice\nsecond line";
        assert_eq!(&text[word_at(text, 2)], "Error");
        assert_eq!(&text[word_at(text, 5)], ":");
        assert_eq!(&text[word_at(text, 9)], "git_status");
        assert_eq!(&text[word_at(text, 25)], "  ");
        assert_eq!(&text[word_at(text, text.len())], "line");
        assert_eq!(&text[line_at(text, 3)], "Error: git_status failed  twice");
        assert_eq!(&text[line_at(text, 40)], "second line");
        assert_eq!(word_at("", 0), 0..0);
    }

    #[test]
    fn draws_the_selection_over_the_highlights() {
        let mark = HighlightStyle {
            font_weight: Some(gpui::FontWeight::BOLD),
            ..HighlightStyle::default()
        };
        let color = gpui::black();
        let out = with_selection(vec![(2..6, mark)], Some(4..9), color);
        let ranges: Vec<Range<usize>> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [2..4, 4..6, 6..9]);
        assert_eq!(out[0].1.background_color, None);
        assert_eq!(out[1].1.background_color, Some(color));
        assert_eq!(out[1].1.font_weight, Some(gpui::FontWeight::BOLD));
        assert_eq!(out[2].1.font_weight, None);
        // Nothing selected: the highlights as they were.
        assert_eq!(with_selection(vec![(2..6, mark)], None, color).len(), 1);
    }
}
