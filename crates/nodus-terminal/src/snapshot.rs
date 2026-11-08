//! What a drawing of the terminal needs, resolved, so the GPUI element in
//! `nodus-ui` draws it without knowing alacritty or xterm's colour rules:
//! each visible cell's characters, width and final colours (selection,
//! inverse, bold-as-bright and dim applied, TERM-058, TERM-079), and the
//! cursor (TERM-076).

use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;

use crate::handler::CursorKind;
use crate::palette::{CellAttrs, CellColor, Colors, Rgb, resolve_cell_colors, theme};
use crate::selection::{BufferPoint, cell_content};
use crate::terminal::Terminal;

/// The underline a program asked for (SGR 4, `4:n`, 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// One cell, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSnapshot {
    /// The characters in the cell (a base character and any combining
    /// marks); empty for a blank cell and for the right half of a wide one.
    pub text: String,
    /// 1, 2 for a wide character, 0 for the right half of one.
    pub width: u8,
    /// The glyph's colour; draw it at [`theme::DIM_OPACITY`] when `dim`.
    pub fg: Rgb,
    pub dim: bool,
    /// The cell's background; nothing to draw when `bg_is_default`.
    pub bg: Rgb,
    pub bg_is_default: bool,
    /// Weight 700.
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    /// SGR 58, else the glyph's colour.
    pub underline_color: Rgb,
    pub strikethrough: bool,
    /// SGR 8: the background is drawn, the glyph isn't.
    pub invisible: bool,
    pub selected: bool,
    /// The OSC 8 hyperlink the cell belongs to, if any.
    pub hyperlink: Option<String>,
}

/// The cursor, when it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnapshot {
    /// Row within the view and column.
    pub row: usize,
    pub col: usize,
    /// Two cells over a wide character.
    pub wide: bool,
    /// Its shape while focused; unfocused it is always a 1 device-px outline
    /// of the cell (`cursorInactiveStyle: 'outline'`).
    pub kind: CursorKind,
    pub focused: bool,
    /// Blinks 600 ms on / 600 ms off while focused, restarting visible on
    /// every move or output.
    pub blink: bool,
    pub color: Rgb,
    /// The glyph under a focused block cursor.
    pub accent: Rgb,
}

/// The visible screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    /// `rows` rows of `cols` cells.
    pub cells: Vec<Vec<CellSnapshot>>,
    pub cursor: Option<CursorSnapshot>,
    /// The terminal's background (OSC 11 can change it).
    pub background: Rgb,
    /// Lines scrolled up from the bottom, and lines of history: the scrollbar.
    pub display_offset: usize,
    pub history_size: usize,
    /// Mode 2026 is on: keep showing the previous frame (TERM-082).
    pub hold: bool,
    /// See [`Terminal::generation`].
    pub generation: u64,
}

impl Terminal {
    /// The visible screen as it should be drawn now.
    pub fn snapshot(&self) -> Snapshot {
        let cols = self.cols();
        let rows = self.rows();
        let offset = self.display_offset() as i32;
        let colors = Colors::with_overrides(self.term.colors());
        let has_selection = self.has_selection();
        let mut cells = Vec::with_capacity(rows);
        for row in 0..rows {
            let line = row as i32 - offset;
            let grid_row = &self.term.grid()[Line(line)];
            let mut out = Vec::with_capacity(cols);
            for col in 0..cols {
                let cell = &grid_row[Column(col)];
                let (text, width) = cell_content(cell);
                let flags = cell.flags;
                let attrs = CellAttrs {
                    bold: flags.contains(Flags::BOLD),
                    dim: flags.contains(Flags::DIM),
                    inverse: flags.contains(Flags::INVERSE),
                };
                let selected = has_selection && self.is_selected(BufferPoint::new(line, col));
                let glyph = text.chars().next().unwrap_or(' ');
                let resolved = resolve_cell_colors(
                    CellColor::from_alacritty(cell.fg),
                    CellColor::from_alacritty(cell.bg),
                    attrs,
                    glyph,
                    selected.then_some(self.focused),
                    &colors,
                );
                let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
                    Underline::Double
                } else if flags.contains(Flags::UNDERCURL) {
                    Underline::Curly
                } else if flags.contains(Flags::DOTTED_UNDERLINE) {
                    Underline::Dotted
                } else if flags.contains(Flags::DASHED_UNDERLINE) {
                    Underline::Dashed
                } else if flags.contains(Flags::UNDERLINE) {
                    Underline::Single
                } else {
                    Underline::None
                };
                let underline_color = match cell.underline_color().map(CellColor::from_alacritty) {
                    // Bold brightens an ANSI 0-7 underline colour too.
                    Some(CellColor::Indexed(i)) if attrs.bold && i < 8 => colors.ansi[usize::from(i) + 8],
                    Some(CellColor::Indexed(i)) => colors.ansi[usize::from(i)],
                    Some(CellColor::Rgb(c)) => c,
                    Some(CellColor::Default) | None => resolved.fg,
                };
                out.push(CellSnapshot {
                    text,
                    width: width as u8,
                    fg: resolved.fg,
                    dim: resolved.dim,
                    bg: resolved.bg,
                    bg_is_default: resolved.bg_is_default,
                    bold: attrs.bold,
                    italic: flags.contains(Flags::ITALIC),
                    underline,
                    underline_color,
                    strikethrough: flags.contains(Flags::STRIKEOUT),
                    invisible: flags.contains(Flags::HIDDEN),
                    selected,
                    hyperlink: cell.hyperlink().map(|h| h.uri().to_owned()),
                });
            }
            cells.push(out);
        }

        let cursor = self
            .cursor_visible()
            .then(|| self.cursor_snapshot(&cells, &colors))
            .flatten();
        Snapshot {
            cols,
            rows,
            cells,
            cursor,
            background: colors.background,
            display_offset: self.display_offset(),
            history_size: self.history_size(),
            hold: self.modes().synchronized_output,
            generation: self.generation(),
        }
    }

    fn cursor_snapshot(&self, cells: &[Vec<CellSnapshot>], colors: &Colors) -> Option<CursorSnapshot> {
        let (line, col) = self.cursor_point();
        let row = usize::try_from(line + self.display_offset() as i32)
            .ok()
            .filter(|r| *r < self.rows())?;
        let (kind, blink) = self.cursor_style();
        let wide = cells.get(row).and_then(|r| r.get(col)).is_some_and(|c| c.width == 2);
        Some(CursorSnapshot {
            row,
            col,
            wide,
            kind,
            focused: self.focused,
            blink: blink && self.focused,
            color: colors.cursor,
            accent: theme::CURSOR_ACCENT,
        })
    }
}

#[cfg(test)]
mod tests {
    use nodus_domain::SessionKind;

    use super::*;
    use crate::keys::Platform;

    fn term(text: &str) -> Terminal {
        let mut t = Terminal::new(SessionKind::Claude, Platform::Mac, 10, 3);
        t.feed(text);
        t
    }

    #[test]
    fn term_079_attributes_reach_the_snapshot() {
        let t = term("\x1b[1;31mA\x1b[0;2mB\x1b[0;3;4:3;58;5;2mC\x1b[0;7mD\x1b[0;8mE\x1b[0;9;38;2;1;2;3mF");
        let s = t.snapshot();
        let c = &s.cells[0];
        assert_eq!((c[0].text.as_str(), c[0].fg, c[0].bold), ("A", theme::ANSI[9], true));
        assert!(c[1].dim);
        assert_eq!(
            (c[2].italic, c[2].underline, c[2].underline_color),
            (true, Underline::Curly, theme::ANSI[2])
        );
        assert_eq!((c[3].fg, c[3].bg), (theme::BACKGROUND, theme::FOREGROUND));
        assert!(c[4].invisible);
        assert_eq!((c[5].strikethrough, c[5].fg), (true, Rgb { r: 1, g: 2, b: 3 }));
        assert!(c[6].text.is_empty() && c[6].bg_is_default);
    }

    #[test]
    fn term_081_wide_characters_take_two_cells() {
        let s = term("中a").snapshot();
        assert_eq!((s.cells[0][0].text.as_str(), s.cells[0][0].width), ("中", 2));
        assert_eq!(s.cells[0][1].width, 0);
        assert_eq!(s.cells[0][2].text, "a");
    }

    #[test]
    fn term_076_cursor_waits_for_focus_and_follows_dectcem_and_decscusr() {
        let mut t = term("ab");
        assert!(t.snapshot().cursor.is_none());
        t.focus();
        let c = t.snapshot().cursor.unwrap();
        assert_eq!(
            (c.row, c.col, c.kind, c.blink, c.focused),
            (0, 2, CursorKind::Block, true, true)
        );
        t.feed("\x1b[6 q");
        assert_eq!(t.snapshot().cursor.unwrap().kind, CursorKind::Bar);
        assert!(!t.snapshot().cursor.unwrap().blink);
        t.feed("\x1b[0 q\x1b[?25l");
        assert!(t.snapshot().cursor.is_none());
        t.feed("\x1b[?25h");
        t.blur();
        let c = t.snapshot().cursor.unwrap();
        assert_eq!((c.kind, c.blink, c.focused), (CursorKind::Block, false, false));
    }

    #[test]
    fn term_076_a_screen_switch_shows_the_cursor_too() {
        let mut t = term("\x1b[?1049h");
        assert!(t.snapshot().cursor.is_some());
        t.feed("\x1b[?1049l");
        assert!(t.snapshot().cursor.is_some());
    }

    #[test]
    fn term_082_synchronized_output_holds_drawing() {
        let mut t = term("\x1b[?2026hx");
        assert!(t.snapshot().hold);
        // Parsed at once, as xterm does.
        assert_eq!(t.snapshot().cells[0][0].text, "x");
        t.feed("\x1b[?2026l");
        assert!(!t.snapshot().hold);
    }

    #[test]
    fn term_084_osc_colours_change_and_restore() {
        let mut t = term("\x1b]11;rgb:00/00/00\x07\x1b]4;1;#010203\x07\x1b[31mx");
        let s = t.snapshot();
        assert_eq!(s.background, Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(s.cells[0][0].fg, Rgb { r: 1, g: 2, b: 3 });
        t.feed("\x1b]111\x07\x1b]104\x07");
        let s = t.snapshot();
        assert_eq!(s.background, theme::BACKGROUND);
        assert_eq!(s.cells[0][0].fg, theme::ANSI[1]);
    }
}
