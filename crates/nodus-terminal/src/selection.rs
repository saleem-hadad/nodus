//! Selection as xterm.js 6 keeps it (`SelectionService`, `SelectionModel`):
//! a start and an end-exclusive cell (the end may sit one past the last
//! column), word selection with xterm's separators that follows wrapped lines
//! (TERM-056), line selection of a whole wrapped line, select all (TERM-041),
//! and the copied text (TERM-059): each row cut at its last written cell,
//! wrapped rows joined, NBSP made a space, rows joined by `\n` on a Mac and
//! `\r\n` on Windows.
//!
//! Lines are alacritty's: 0 is the top of the screen, negative lines are the
//! history. To follow the content as output scrolls, the selection's lines
//! ride on an alacritty selection, which alacritty moves with the grid; the
//! columns are kept here.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};

use crate::handler::WRITTEN;
use crate::terminal::{Terminal, WORD_SEPARATORS};

/// A cell in the buffer. `line` 0 is the top of the screen; the history is
/// above it, at negative lines. `col` may equal the column count at an
/// exclusive end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferPoint {
    pub line: i32,
    pub col: usize,
}

impl BufferPoint {
    pub const fn new(line: i32, col: usize) -> Self {
        Self { line, col }
    }
}

/// The columns of a selection; its lines ride on `term.selection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Columns {
    start: usize,
    end: usize,
    column_mode: bool,
    select_all: bool,
}

/// A cell as xterm reads it: the characters it holds (`""` when it holds
/// none) and its width (0 for the right half of a wide character).
pub(crate) fn cell_content(cell: &Cell) -> (String, usize) {
    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        return (String::new(), 0);
    }
    if !cell.flags.contains(WRITTEN) || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
        return (String::new(), 1);
    }
    let mut s = String::new();
    // A tab stop alacritty marked over a written space is still that space.
    s.push(if cell.c == '\t' { ' ' } else { cell.c });
    if let Some(zw) = cell.zerowidth() {
        s.extend(zw.iter());
    }
    let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
    (s, width)
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

impl Terminal {
    fn line_in_buffer(&self, line: i32) -> bool {
        line >= self.term.topmost_line().0 && line <= self.term.bottommost_line().0
    }

    fn row_cells(&self, line: i32) -> &[Cell] {
        &self.term.grid()[Line(line)][..]
    }

    /// `line.isWrapped`: whether `line` continues the line above it.
    pub fn is_wrapped(&self, line: i32) -> bool {
        if line <= self.term.topmost_line().0 || !self.line_in_buffer(line) {
            return false;
        }
        let above = self.row_cells(line - 1);
        above.last().is_some_and(|c| c.flags.contains(Flags::WRAPLINE))
    }

    /// `getTrimmedLength`: just past the last cell holding a character.
    fn trimmed_length(&self, line: i32) -> usize {
        let cells = self.row_cells(line);
        for (i, cell) in cells.iter().enumerate().rev() {
            let (chars, width) = cell_content(cell);
            if !chars.is_empty() {
                return i + width;
            }
        }
        0
    }

    /// `translateBufferLineToString(line, trimRight, startCol, endCol)`.
    pub fn line_text(&self, line: i32, trim_right: bool, start: usize, end: Option<usize>) -> String {
        if !self.line_in_buffer(line) {
            return String::new();
        }
        let cells = self.row_cells(line);
        let mut end = end.unwrap_or(cells.len());
        if trim_right {
            end = end.min(self.trimmed_length(line));
        }
        let mut out = String::new();
        let mut col = start;
        while col < end {
            let (chars, width) = cells.get(col).map(cell_content).unwrap_or_default();
            if chars.is_empty() {
                out.push(' ');
            } else {
                out.push_str(&chars);
            }
            col += width.max(1);
        }
        out
    }

    /// `getWrappedRangeForLine`: the first and last line of the wrapped line holding `line`.
    pub fn wrapped_range(&self, line: i32) -> (i32, i32) {
        let mut first = line;
        let mut last = line;
        while first > self.term.topmost_line().0 && self.is_wrapped(first) {
            first -= 1;
        }
        while last < self.term.bottommost_line().0 && self.is_wrapped(last + 1) {
            last += 1;
        }
        (first, last)
    }

    // --- The selection ---------------------------------------------------

    fn set_selection(&mut self, start: BufferPoint, end: BufferPoint, column_mode: bool, select_all: bool) {
        // The carrier spans whole lines so alacritty never trims it to nothing.
        let last = self.term.last_column();
        let mut carrier = Selection::new(
            SelectionType::Simple,
            Point::new(Line(start.line), Column(0)),
            Side::Left,
        );
        carrier.update(Point::new(Line(end.line.max(start.line)), last), Side::Right);
        self.term.selection = Some(carrier);
        self.selection_columns = Some(Columns {
            start: start.col,
            end: end.col,
            column_mode,
            select_all,
        });
        self.touch();
    }

    /// xterm's `finalSelectionStart` and `finalSelectionEnd`, ordered, and
    /// whether it is a column selection.
    pub fn selection_range(&self) -> Option<(BufferPoint, BufferPoint, bool)> {
        let cols = self.selection_columns?;
        // alacritty drops the carrier where xterm drops the selection (a
        // screen switch, a resize).
        self.term.selection.as_ref()?;
        if cols.select_all {
            return Some((
                BufferPoint::new(self.term.topmost_line().0, 0),
                BufferPoint::new(self.term.bottommost_line().0, self.cols()),
                false,
            ));
        }
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        let start = BufferPoint::new(range.start.line.0, cols.start);
        let end = BufferPoint::new(range.end.line.0, cols.end);
        Some((start, end, cols.column_mode))
    }

    /// `hasSelection`: a selection that isn't empty.
    pub fn has_selection(&self) -> bool {
        self.selection_range().is_some_and(|(s, e, _)| s != e)
    }

    pub fn clear_selection(&mut self) {
        let had = self.term.selection.take().is_some();
        if self.selection_columns.take().is_some() || had {
            self.touch();
        }
    }

    /// Selects the whole buffer, history included (TERM-041).
    pub fn select_all(&mut self) {
        let start = BufferPoint::new(self.term.topmost_line().0, 0);
        let end = BufferPoint::new(self.term.bottommost_line().0, self.cols());
        self.set_selection(start, end, false, true);
    }

    /// Selects from `start` to `end` (exclusive), in whichever order they come;
    /// `column_mode` selects the same columns on every line (⌥/Alt-drag).
    pub fn select_range(&mut self, a: BufferPoint, b: BufferPoint, column_mode: bool) {
        let (start, end) = if (a.line, a.col) <= (b.line, b.col) {
            (a, b)
        } else {
            (b, a)
        };
        self.set_selection(start, end, column_mode, false);
    }

    /// Selects the (wrapped) line holding `line` (triple-click).
    pub fn select_line_at(&mut self, line: i32) {
        if !self.line_in_buffer(line) {
            return;
        }
        let (first, last) = self.wrapped_range(line);
        self.set_selection(
            BufferPoint::new(first, 0),
            BufferPoint::new(last, self.cols()),
            false,
            false,
        );
    }

    /// Selects the word at `p` (double-click: whitespace too; right-click on a
    /// Mac: not whitespace alone). False when there's no word there.
    pub fn select_word_at(&mut self, p: BufferPoint, allow_whitespace_only: bool) -> bool {
        let Some((start, end)) = self.word_range_at(p, allow_whitespace_only) else {
            return false;
        };
        self.set_selection(start, end, false, false);
        true
    }

    /// Whether `p` is inside the selection (`isCellSelected`).
    pub fn is_selected(&self, p: BufferPoint) -> bool {
        let Some((start, end, column)) = self.selection_range() else {
            return false;
        };
        if start == end {
            return false;
        }
        if column {
            let (lo, hi) = (start.col.min(end.col), start.col.max(end.col));
            return p.line >= start.line && p.line <= end.line && p.col >= lo && p.col < hi;
        }
        if p.line < start.line || p.line > end.line {
            return false;
        }
        let from = if p.line == start.line { start.col } else { 0 };
        let to = if p.line == end.line { end.col } else { self.cols() };
        p.col >= from && p.col < to
    }

    /// `selectionText` (TERM-059).
    pub fn selection_text(&self) -> String {
        let Some((start, end, column)) = self.selection_range() else {
            return String::new();
        };
        let mut rows: Vec<String> = Vec::new();
        if column {
            if start.col == end.col {
                return String::new();
            }
            let (lo, hi) = (start.col.min(end.col), start.col.max(end.col));
            for line in start.line..=end.line {
                rows.push(self.line_text(line, true, lo, Some(hi)));
            }
        } else {
            let first_end = (start.line == end.line).then_some(end.col);
            rows.push(self.line_text(start.line, true, start.col, first_end));
            for line in start.line + 1..end.line {
                let text = self.line_text(line, true, 0, None);
                if self.is_wrapped(line) {
                    if let Some(last) = rows.last_mut() {
                        last.push_str(&text);
                    }
                } else {
                    rows.push(text);
                }
            }
            if start.line != end.line {
                let text = self.line_text(end.line, true, 0, Some(end.col));
                if self.is_wrapped(end.line) {
                    if let Some(last) = rows.last_mut() {
                        last.push_str(&text);
                    }
                } else {
                    rows.push(text);
                }
            }
        }
        let sep = if self.platform.is_windows() { "\r\n" } else { "\n" };
        rows.iter()
            .map(|r| r.replace('\u{a0}', " "))
            .collect::<Vec<_>>()
            .join(sep)
    }

    // --- Words (`_getWordAt`) -------------------------------------------

    fn cell_at(&self, line: i32, col: usize) -> (String, usize) {
        self.row_cells(line).get(col).map(cell_content).unwrap_or_default()
    }

    fn code_point_at(&self, line: i32, col: usize) -> u32 {
        self.cell_at(line, col).0.chars().next().map_or(0, |c| c as u32)
    }

    /// `_isCharWordSeparator`: wide halves never are; an empty cell is (its
    /// `""` is found in any string).
    fn is_separator(&self, line: i32, col: usize) -> bool {
        let (chars, width) = self.cell_at(line, col);
        width != 0 && WORD_SEPARATORS.contains(chars.as_str())
    }

    /// `_convertViewportColToCharacterIndex`.
    fn char_index(&self, line: i32, col: usize) -> i64 {
        let mut index = col as i64;
        for i in 0..=col {
            let (chars, width) = self.cell_at(line, i);
            let len = utf16_len(&chars);
            if width == 0 {
                index -= 1;
            } else if len > 1 && col != i {
                index += len as i64 - 1;
            }
        }
        index
    }

    /// `_getWordAt`: the word's start column (negative when it begins on a
    /// line above) and its length in cells.
    fn word_at(&self, col: usize, line: i32, allow_ws: bool, above: bool, below: bool) -> Option<(i64, i64)> {
        let cols = self.cols();
        if col >= cols || !self.line_in_buffer(line) {
            return None;
        }
        let text: Vec<u16> = self.line_text(line, false, 0, None).encode_utf16().collect();
        let mut start_index = self.char_index(line, col);
        let mut end_index = start_index;
        let char_offset = col as i64 - start_index;
        let (mut left_wide, mut right_wide, mut left_long, mut right_long) = (0i64, 0i64, 0i64, 0i64);
        let at = |i: i64| usize::try_from(i).ok().and_then(|i| text.get(i).copied());

        if at(start_index) == Some(u16::from(b' ')) {
            while start_index > 0 && at(start_index - 1) == Some(u16::from(b' ')) {
                start_index -= 1;
            }
            while (end_index as usize) < text.len() && at(end_index + 1) == Some(u16::from(b' ')) {
                end_index += 1;
            }
        } else {
            let mut start_col = col as i64;
            let mut end_col = col as i64;
            if self.cell_at(line, start_col as usize).1 == 0 {
                left_wide += 1;
                start_col -= 1;
            }
            if self.cell_at(line, end_col as usize).1 == 2 {
                right_wide += 1;
                end_col += 1;
            }
            let len = utf16_len(&self.cell_at(line, end_col as usize).0) as i64;
            if len > 1 {
                right_long += len - 1;
                end_index += len - 1;
            }
            while start_col > 0 && start_index > 0 && !self.is_separator(line, (start_col - 1) as usize) {
                let (chars, width) = self.cell_at(line, (start_col - 1) as usize);
                let len = utf16_len(&chars) as i64;
                if width == 0 {
                    left_wide += 1;
                    start_col -= 1;
                } else if len > 1 {
                    left_long += len - 1;
                    start_index -= len - 1;
                }
                start_index -= 1;
                start_col -= 1;
            }
            while (end_col as usize) < cols
                && ((end_index + 1) as usize) < text.len()
                && !self.is_separator(line, (end_col + 1) as usize)
            {
                let (chars, width) = self.cell_at(line, (end_col + 1) as usize);
                let len = utf16_len(&chars) as i64;
                if width == 2 {
                    right_wide += 1;
                    end_col += 1;
                } else if len > 1 {
                    right_long += len - 1;
                    end_index += len - 1;
                }
                end_index += 1;
                end_col += 1;
            }
        }
        end_index += 1;

        let mut start = start_index + char_offset - left_wide + left_long;
        let mut length = (cols as i64).min(end_index - start_index + left_wide + right_wide - left_long - right_long);

        if !allow_ws {
            let from = start_index.max(0) as usize;
            let to = (end_index.max(0) as usize).min(text.len());
            let slice = String::from_utf16_lossy(text.get(from..to).unwrap_or_default());
            if nodus_domain::js::trim(&slice).is_empty() {
                return None;
            }
        }

        if above
            && start == 0
            && self.code_point_at(line, 0) != 32
            && self.is_wrapped(line)
            && self.line_in_buffer(line - 1)
            && self.code_point_at(line - 1, cols - 1) != 32
            && let Some((prev_start, _)) = self.word_at(cols - 1, line - 1, false, true, false)
        {
            let offset = cols as i64 - prev_start;
            start -= offset;
            length += offset;
        }
        if below
            && start + length == cols as i64
            && self.code_point_at(line, cols - 1) != 32
            && self.line_in_buffer(line + 1)
            && self.is_wrapped(line + 1)
            && self.code_point_at(line + 1, 0) != 32
            && let Some((_, next_len)) = self.word_at(0, line + 1, false, false, true)
        {
            length += next_len;
        }
        Some((start, length))
    }

    /// The word at `p` as a range (`_selectWordAt` then `finalSelectionEnd`).
    pub fn word_range_at(&self, p: BufferPoint, allow_whitespace_only: bool) -> Option<(BufferPoint, BufferPoint)> {
        let (mut start, length) = self.word_at(p.col, p.line, allow_whitespace_only, true, true)?;
        let cols = self.cols() as i64;
        let mut line = p.line;
        while start < 0 {
            start += cols;
            line -= 1;
        }
        let begin = BufferPoint::new(line, start as usize);
        let plus = start + length;
        let end = if plus > cols {
            if plus % cols == 0 {
                BufferPoint::new(line + (plus / cols) as i32 - 1, cols as usize)
            } else {
                BufferPoint::new(line + (plus / cols) as i32, (plus % cols) as usize)
            }
        } else {
            BufferPoint::new(line, plus as usize)
        };
        Some((begin, end))
    }

    /// `isClickInSelection` for a cell.
    pub fn is_in_selection(&self, p: BufferPoint) -> bool {
        self.is_selected(p)
    }
}

#[cfg(test)]
mod tests {
    use nodus_domain::SessionKind;

    use super::*;
    use crate::keys::Platform;

    fn term(cols: usize, rows: usize, text: &str) -> Terminal {
        let mut t = Terminal::new(SessionKind::Terminal, Platform::Mac, cols, rows);
        t.feed(text);
        t
    }

    #[test]
    fn term_059_trailing_empty_cells_go_but_written_spaces_stay() {
        let mut t = term(10, 3, "ab  \r\nc\x1b[5Cd");
        t.select_range(BufferPoint::new(0, 0), BufferPoint::new(1, 10), false);
        assert_eq!(t.selection_text(), "ab  \nc     d");
        t.feed("\x1b[1;3H\x1b[K");
        assert_eq!(t.selection_text(), "ab\nc     d");
    }

    #[test]
    fn term_059_wrapped_rows_join_and_nbsp_becomes_space() {
        let mut t = term(5, 4, "abcdefgh\r\nx\u{a0}y");
        t.select_all();
        assert_eq!(t.selection_text(), "abcdefgh\nx y\n");
    }

    #[test]
    fn term_059_rows_join_with_crlf_on_windows() {
        let mut t = Terminal::new(SessionKind::Terminal, Platform::Windows, 10, 2);
        t.feed("one\r\ntwo");
        t.select_all();
        assert_eq!(t.selection_text(), "one\r\ntwo");
    }

    #[test]
    fn term_059_an_end_at_column_zero_keeps_the_line_break() {
        let mut t = term(10, 3, "one\r\ntwo");
        t.select_range(BufferPoint::new(0, 0), BufferPoint::new(1, 0), false);
        assert_eq!(t.selection_text(), "one\n");
    }

    #[test]
    fn term_056_column_selection_takes_the_same_columns() {
        let mut t = term(10, 3, "abcdef\r\nghijkl\r\nmn");
        t.select_range(BufferPoint::new(0, 1), BufferPoint::new(2, 3), true);
        assert_eq!(t.selection_text(), "bc\nhi\nn");
    }

    #[test]
    fn term_056_words_stop_at_xterms_separators() {
        let t = term(30, 2, "echo (foo.bar) 'baz' qux");
        let word = |col| {
            let (s, e) = t.word_range_at(BufferPoint::new(0, col), true).unwrap();
            t.line_text(0, false, s.col, Some(e.col))
        };
        assert_eq!(word(1), "echo");
        assert_eq!(word(7), "foo.bar");
        assert_eq!(word(16), "baz");
        assert_eq!(word(4), " ");
        assert!(t.word_range_at(BufferPoint::new(0, 4), false).is_none());
    }

    #[test]
    fn term_056_words_follow_wrapped_lines() {
        let mut t = term(5, 3, "ab cdefgh ij");
        assert!(t.select_word_at(BufferPoint::new(1, 1), true));
        assert_eq!(t.selection_text(), "cdefgh");
        assert!(t.select_word_at(BufferPoint::new(0, 4), true));
        assert_eq!(t.selection_text(), "cdefgh");
    }

    #[test]
    fn term_056_wide_characters_select_whole() {
        let mut t = term(10, 2, "a 中文b c");
        assert!(t.select_word_at(BufferPoint::new(0, 3), true));
        assert_eq!(t.selection_text(), "中文b");
    }

    #[test]
    fn term_056_triple_click_selects_the_wrapped_line() {
        let mut t = term(4, 4, "x\r\nabcdefg\r\ny");
        t.select_line_at(2);
        assert_eq!(t.selection_text(), "abcdefg");
    }

    #[test]
    fn term_057_selection_rides_with_scrolling_output() {
        let mut t = term(10, 3, "one\r\ntwo\r\nthree");
        t.select_range(BufferPoint::new(1, 0), BufferPoint::new(1, 3), false);
        t.feed("\r\nfour\r\nfive");
        assert_eq!(t.selection_range().unwrap().0.line, -1);
        assert_eq!(t.selection_text(), "two");
    }

    #[test]
    fn term_057_erasing_keeps_the_selection_but_a_row_resize_clears_it() {
        let mut t = term(10, 3, "one\r\ntwo");
        t.select_range(BufferPoint::new(0, 0), BufferPoint::new(0, 3), false);
        t.feed("\x1b[2K\x1b[2J");
        assert!(t.has_selection());
        t.resize(10, 4);
        assert!(!t.has_selection());
    }
}
