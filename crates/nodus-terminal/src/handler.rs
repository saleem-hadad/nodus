//! What sits between the VT parser and alacritty's `Term`: everything is passed
//! on to the `Term` except where xterm.js 6 behaves differently, so a program
//! sees the terminal it saw in 0.9.4:
//!
//! - queries are answered as xterm answers them, and only those (TERM-078):
//!   DA1 `\x1b[?1;2c`, DA2 `\x1b[>0;276;0c`, DSR/CPR, DECRQM from xterm's mode
//!   table, OSC 4/10/11/12 in `rgb:rrrr/gggg/bbbb` ended by `ESC \`. Window ops
//!   (`windowOptions: {}`), the kitty keyboard protocol, modifyOtherKeys and
//!   OSC 52 get no answer;
//! - characters take xterm's Unicode 11 widths (TERM-081, see [`crate::unicode11`]);
//! - modes alacritty lacks are kept here: X10 and SGR-pixels mouse reporting,
//!   alternate screens 47/1047, reverse wraparound, synchronized output 2026
//!   (parsed at once, as xterm does; only drawing waits, TERM-082), DECSCUSR and
//!   mode 12 as xterm's cursor options;
//! - `CSI 2 J` erases the screen in place instead of pushing it into the
//!   scrollback (`scrollOnEraseInDisplay: false`);
//! - bells and titles are ignored.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::cursor_icon::CursorIcon;
use alacritty_terminal::vte::ansi::{
    self, Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, LineClearMode, ModifyOtherKeys, PrivateMode, Rgb as VteRgb, ScpCharPath, ScpUpdateMode,
    StandardCharset, TabulationClearMode,
};

use crate::palette::{Colors, rgb_string};
use crate::unicode11;

/// Marks a cell a program wrote a character into, as xterm's cells "have
/// content" (a written space does, an erased cell doesn't). Bit 15 of
/// alacritty's cell flags is unused; the cursor template carries it into every
/// written cell, and erasing resets cells without it.
pub(crate) const WRITTEN: Flags = Flags::from_bits_retain(0x8000);

/// xterm's mouse tracking protocol (`CoreMouseService.activeProtocol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseProtocol {
    #[default]
    None,
    /// Mode 9: presses only.
    X10,
    /// Mode 1000: presses and releases.
    Vt200,
    /// Mode 1002: and drags.
    Drag,
    /// Mode 1003: and every move.
    Any,
}

/// xterm's mouse report encoding (`CoreMouseService.activeEncoding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseEncoding {
    #[default]
    Default,
    /// Mode 1006.
    Sgr,
    /// Mode 1016.
    SgrPixels,
}

/// The cursor shapes xterm draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorKind {
    #[default]
    Block,
    Underline,
    Bar,
}

/// State xterm keeps that alacritty doesn't, or keeps differently.
#[derive(Debug, Clone)]
pub(crate) struct XtermState {
    pub mouse_protocol: MouseProtocol,
    pub mouse_encoding: MouseEncoding,
    /// Mode 2026 (`decPrivateModes.synchronizedOutput`).
    pub synchronized_output: bool,
    /// Mode 45.
    pub reverse_wraparound: bool,
    /// The `cursorBlink` option (true in registry.ts); mode 12 changes it.
    pub option_cursor_blink: bool,
    /// DECSCUSR's shape and blink (`decPrivateModes.cursorStyle/cursorBlink`), until `CSI 0 SP q`.
    pub decscusr: Option<(CursorKind, bool)>,
    /// The cursor isn't drawn until the terminal is focused, a key is pressed
    /// or the screen buffer switches (`coreService.isCursorInitialized`).
    pub cursor_initialized: bool,
    /// DECSTBM, as xterm reports it to DECRQSS (0-based, inclusive).
    pub scroll_top: usize,
    pub scroll_bottom: usize,
    /// Data the terminal sends the program by itself (query answers).
    pub replies: Vec<String>,
}

impl XtermState {
    pub fn new(rows: usize) -> Self {
        Self {
            mouse_protocol: MouseProtocol::None,
            mouse_encoding: MouseEncoding::Default,
            synchronized_output: false,
            reverse_wraparound: false,
            option_cursor_blink: true,
            decscusr: None,
            cursor_initialized: false,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            replies: Vec::new(),
        }
    }

    /// What xterm draws: DECSCUSR's style if set, else the options (block, `cursorBlink`).
    pub fn cursor_style(&self) -> (CursorKind, bool) {
        self.decscusr.unwrap_or((CursorKind::Block, self.option_cursor_blink))
    }
}

/// One pass of the parser over the terminal.
pub(crate) struct XtermHandler<'a> {
    pub term: &'a mut Term<VoidListener>,
    pub st: &'a mut XtermState,
}

/// DECRPM values.
const NOT_RECOGNIZED: u8 = 0;
const SET: u8 = 1;
const RESET: u8 = 2;
const PERMANENTLY_SET: u8 = 3;
const PERMANENTLY_RESET: u8 = 4;

fn b2v(on: bool) -> u8 {
    if on { SET } else { RESET }
}

impl XtermHandler<'_> {
    fn reply(&mut self, s: String) {
        self.st.replies.push(s);
    }

    fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Writes `c` at `width` cells whatever width alacritty would give it.
    fn input_at_width(&mut self, c: char, width: usize) {
        // A cell written with alacritty's width would be the same; only
        // differences go the long way.
        if unicode11::alacritty_width(c) == Some(width) {
            self.term.input(c);
            return;
        }
        if width == 0 {
            // Join the previous cell, as alacritty does for its own zero-width characters.
            let grid = self.term.grid_mut();
            let line = grid.cursor.point.line;
            let mut column = grid.cursor.point.column;
            if !grid.cursor.input_needs_wrap {
                column.0 = column.0.saturating_sub(1);
            }
            if grid[line][column].flags.contains(Flags::WIDE_CHAR_SPACER) {
                column.0 = column.0.saturating_sub(1);
            }
            grid[line][column].push_zerowidth(c);
            return;
        }
        // Write a stand-in of the right width, then put the real character in its cell.
        let stand_in = if width == 2 { '\u{4e00}' } else { '\u{e000}' };
        self.term.input(stand_in);
        let grid = self.term.grid_mut();
        let line = grid.cursor.point.line;
        let after = grid.cursor.point.column.0 + usize::from(grid.cursor.input_needs_wrap);
        if let Some(col) = after.checked_sub(width) {
            let cell = &mut grid[line][Column(col)];
            if cell.c == stand_in {
                cell.c = c;
            }
        }
    }

    fn set_alt_screen(&mut self, on: bool, keep_cursor: bool) {
        let in_alt = self.mode().contains(TermMode::ALT_SCREEN);
        if on && !in_alt {
            self.term.swap_alt();
        } else if !on && in_alt {
            // 47/1047: xterm's normal buffer takes the alternate buffer's cursor.
            let point = self.term.grid().cursor.point;
            self.term.swap_alt();
            if keep_cursor {
                let last_line = Line(self.term.screen_lines() as i32 - 1);
                let last_col = Column(self.term.columns() - 1);
                let grid = self.term.grid_mut();
                grid.cursor.point.line = point.line.min(last_line);
                grid.cursor.point.column = point.column.min(last_col);
                grid.cursor.input_needs_wrap = false;
            }
        }
        self.st.cursor_initialized = true;
    }

    fn set_xterm_private_mode(&mut self, mode: PrivateMode, on: bool) {
        let raw = mode.raw();
        match raw {
            9 => self.st.mouse_protocol = if on { MouseProtocol::X10 } else { MouseProtocol::None },
            1000 => self.st.mouse_protocol = if on { MouseProtocol::Vt200 } else { MouseProtocol::None },
            1002 => self.st.mouse_protocol = if on { MouseProtocol::Drag } else { MouseProtocol::None },
            1003 => self.st.mouse_protocol = if on { MouseProtocol::Any } else { MouseProtocol::None },
            1006 => self.st.mouse_encoding = if on { MouseEncoding::Sgr } else { MouseEncoding::Default },
            1016 => {
                self.st.mouse_encoding = if on {
                    MouseEncoding::SgrPixels
                } else {
                    MouseEncoding::Default
                }
            }
            12 => self.st.option_cursor_blink = on,
            45 => self.st.reverse_wraparound = on,
            2026 => self.st.synchronized_output = on,
            66 => {
                if on {
                    self.term.set_keypad_application_mode();
                } else {
                    self.term.unset_keypad_application_mode();
                }
            }
            _ => {}
        }
        match raw {
            // xterm's own alternate-screen handling.
            47 | 1047 => self.set_alt_screen(on, true),
            1049 => {
                if on {
                    if !self.mode().contains(TermMode::ALT_SCREEN) {
                        self.term.swap_alt();
                    }
                    self.st.cursor_initialized = true;
                } else {
                    self.set_alt_screen(false, false);
                }
            }
            1048 => {
                if on {
                    self.term.save_cursor_position();
                } else {
                    self.term.restore_cursor_position();
                }
            }
            // xterm doesn't do UTF-8 mouse (1005), DECCOLM without `setWinLines`
            // (3), urgency hints or alternate scroll; mode 12 is the option above.
            3 | 12 | 1005 | 1007 | 1042 => {}
            // Mouse modes live above; alacritty needn't track them.
            9 | 1000 | 1002 | 1003 | 1006 | 1016 => {}
            _ => {
                if on {
                    self.term.set_private_mode(mode);
                } else {
                    self.term.unset_private_mode(mode);
                }
            }
        }
    }

    /// `InputHandler.requestMode` for DEC private modes.
    fn private_mode_state(&self, p: u16) -> u8 {
        let m = self.mode();
        let st = &self.st;
        match p {
            1 => b2v(m.contains(TermMode::APP_CURSOR)),
            // DECCOLM only with `windowOptions.setWinLines`.
            3 => NOT_RECOGNIZED,
            6 => b2v(m.contains(TermMode::ORIGIN)),
            7 => b2v(m.contains(TermMode::LINE_WRAP)),
            8 => PERMANENTLY_SET,
            9 => b2v(st.mouse_protocol == MouseProtocol::X10),
            12 => b2v(st.option_cursor_blink),
            25 => b2v(m.contains(TermMode::SHOW_CURSOR)),
            45 => b2v(st.reverse_wraparound),
            66 => b2v(m.contains(TermMode::APP_KEYPAD)),
            67 => PERMANENTLY_RESET,
            1000 => b2v(st.mouse_protocol == MouseProtocol::Vt200),
            1002 => b2v(st.mouse_protocol == MouseProtocol::Drag),
            1003 => b2v(st.mouse_protocol == MouseProtocol::Any),
            1004 => b2v(m.contains(TermMode::FOCUS_IN_OUT)),
            1005 | 1015 => PERMANENTLY_RESET,
            1006 => b2v(st.mouse_encoding == MouseEncoding::Sgr),
            1016 => b2v(st.mouse_encoding == MouseEncoding::SgrPixels),
            1048 => SET,
            47 | 1047 | 1049 => b2v(m.contains(TermMode::ALT_SCREEN)),
            2004 => b2v(m.contains(TermMode::BRACKETED_PASTE)),
            2026 => b2v(st.synchronized_output),
            _ => NOT_RECOGNIZED,
        }
    }

    /// The cursor position xterm reports: one-based, and one past the last
    /// column while a wrap is pending (xterm's `x` is `cols` then).
    pub(crate) fn cursor_report(&self) -> (usize, usize) {
        let grid = self.term.grid();
        let y = grid.cursor.point.line.0.max(0) as usize + 1;
        let x = grid.cursor.point.column.0 + 1 + usize::from(grid.cursor.input_needs_wrap);
        (y, x)
    }
}

impl Handler for XtermHandler<'_> {
    fn set_title(&mut self, _: Option<String>) {
        // Parsed, unused: the app never shows a terminal's title.
    }

    fn set_cursor_style(&mut self, style: Option<CursorStyle>) {
        // DECSCUSR. vte has already turned `CSI Ps SP q` into a shape and blink.
        self.st.decscusr = style.map(|s| {
            let kind = match s.shape {
                CursorShape::Underline => CursorKind::Underline,
                CursorShape::Beam => CursorKind::Bar,
                _ => CursorKind::Block,
            };
            (kind, s.blinking)
        });
    }

    fn set_cursor_shape(&mut self, _shape: CursorShape) {
        // OSC 50 CursorShape=…: xterm.js has no OSC 50.
    }

    fn input(&mut self, c: char) {
        // C1 controls that came as UTF-8: alacritty drops them, and so does
        // xterm (its parser executes them rather than printing them).
        if unicode11::alacritty_width(c).is_none() {
            return;
        }
        self.term.grid_mut().cursor.template.flags.insert(WRITTEN);
        self.input_at_width(c, unicode11::wcwidth(c));
    }

    fn goto(&mut self, line: i32, col: usize) {
        self.term.goto(line, col);
    }

    fn goto_line(&mut self, line: i32) {
        self.term.goto_line(line);
    }

    fn goto_col(&mut self, col: usize) {
        self.term.goto_col(col);
    }

    fn insert_blank(&mut self, count: usize) {
        self.term.insert_blank(count);
    }

    fn move_up(&mut self, rows: usize) {
        self.term.move_up(rows);
    }

    fn move_down(&mut self, rows: usize) {
        self.term.move_down(rows);
    }

    fn identify_terminal(&mut self, intermediate: Option<char>) {
        // `termName` is 'xterm'.
        match intermediate {
            None => self.reply("\x1b[?1;2c".to_owned()),
            Some('>') => self.reply("\x1b[>0;276;0c".to_owned()),
            _ => {}
        }
    }

    fn device_status(&mut self, arg: usize) {
        match arg {
            5 => self.reply("\x1b[0n".to_owned()),
            6 => {
                let (y, x) = self.cursor_report();
                self.reply(format!("\x1b[{y};{x}R"));
            }
            _ => {}
        }
    }

    fn move_forward(&mut self, cols: usize) {
        self.term.move_forward(cols);
    }

    fn move_backward(&mut self, cols: usize) {
        self.term.move_backward(cols);
    }

    fn move_down_and_cr(&mut self, rows: usize) {
        self.term.move_down_and_cr(rows);
    }

    fn move_up_and_cr(&mut self, rows: usize) {
        self.term.move_up_and_cr(rows);
    }

    fn put_tab(&mut self, count: u16) {
        self.term.put_tab(count);
    }

    fn backspace(&mut self) {
        self.term.backspace();
    }

    fn carriage_return(&mut self) {
        self.term.carriage_return();
    }

    fn linefeed(&mut self) {
        self.term.linefeed();
    }

    fn bell(&mut self) {
        // No sound and no flash in 0.9.4.
    }

    fn substitute(&mut self) {
        self.term.substitute();
    }

    fn newline(&mut self) {
        self.term.newline();
    }

    fn set_horizontal_tabstop(&mut self) {
        self.term.set_horizontal_tabstop();
    }

    fn scroll_up(&mut self, rows: usize) {
        self.term.scroll_up(rows);
    }

    fn scroll_down(&mut self, rows: usize) {
        self.term.scroll_down(rows);
    }

    fn insert_blank_lines(&mut self, rows: usize) {
        self.term.insert_blank_lines(rows);
    }

    fn delete_lines(&mut self, rows: usize) {
        self.term.delete_lines(rows);
    }

    fn erase_chars(&mut self, count: usize) {
        self.term.erase_chars(count);
    }

    fn delete_chars(&mut self, count: usize) {
        self.term.delete_chars(count);
    }

    fn move_backward_tabs(&mut self, count: u16) {
        self.term.move_backward_tabs(count);
    }

    fn move_forward_tabs(&mut self, count: u16) {
        self.term.move_forward_tabs(count);
    }

    fn save_cursor_position(&mut self) {
        self.term.save_cursor_position();
    }

    fn restore_cursor_position(&mut self) {
        self.term.restore_cursor_position();
    }

    fn clear_line(&mut self, mode: LineClearMode) {
        // xterm never drops the selection for erasing under it.
        let selection = self.term.selection.clone();
        self.term.clear_line(mode);
        self.term.selection = selection;
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        // xterm keeps the selection through erases; only clearing the
        // history can take it (when it was up there).
        let selection = match mode {
            ClearMode::Saved => None,
            _ => Some(self.term.selection.clone()),
        };
        match mode {
            // `scrollOnEraseInDisplay: false`: erase the screen where it is,
            // instead of scrolling it into the scrollback as alacritty does.
            ClearMode::All => self.term.grid_mut().reset_region(..),
            ClearMode::Above => {
                let line = self.term.grid().cursor.point.line;
                self.term.clear_screen(ClearMode::Above);
                // alacritty leaves the top line when the cursor is on the
                // second; xterm erases every line above the cursor.
                if line.0 == 1 {
                    self.term.grid_mut().reset_region(..line);
                }
            }
            mode => self.term.clear_screen(mode),
        }
        if let Some(selection) = selection {
            self.term.selection = selection;
        }
    }

    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        self.term.clear_tabs(mode);
    }

    fn set_tabs(&mut self, interval: u16) {
        self.term.set_tabs(interval);
    }

    fn reset_state(&mut self) {
        // RIS (`ESC c`).
        self.term.reset_state();
        let rows = self.term.screen_lines();
        // Options (mode 12's `cursorBlink`) and whether the cursor was ever
        // shown outlive a reset, as in xterm.
        let replies = std::mem::take(&mut self.st.replies);
        let initialized = self.st.cursor_initialized;
        let blink = self.st.option_cursor_blink;
        *self.st = XtermState::new(rows);
        self.st.replies = replies;
        self.st.cursor_initialized = initialized;
        self.st.option_cursor_blink = blink;
    }

    fn reverse_index(&mut self) {
        self.term.reverse_index();
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        self.term.terminal_attribute(attr);
    }

    fn set_mode(&mut self, mode: ansi::Mode) {
        self.term.set_mode(mode);
    }

    fn unset_mode(&mut self, mode: ansi::Mode) {
        self.term.unset_mode(mode);
    }

    fn report_mode(&mut self, mode: ansi::Mode) {
        let p = mode.raw();
        let v = match p {
            2 => PERMANENTLY_RESET,
            4 => b2v(self.mode().contains(TermMode::INSERT)),
            12 => PERMANENTLY_SET,
            // `convertEol` is false.
            20 => RESET,
            _ => NOT_RECOGNIZED,
        };
        self.reply(format!("\x1b[{p};{v}$y"));
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        self.set_xterm_private_mode(mode, true);
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        self.set_xterm_private_mode(mode, false);
    }

    fn report_private_mode(&mut self, mode: PrivateMode) {
        let p = mode.raw();
        let v = self.private_mode_state(p);
        self.reply(format!("\x1b[?{p};{v}$y"));
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        let rows = self.term.screen_lines();
        let bottom_1 = bottom.unwrap_or(rows);
        // xterm: `top < bottom` within the screen, else ignored.
        let (t, b) = (top.max(1) - 1, bottom_1.min(rows).max(1) - 1);
        if t < b {
            self.st.scroll_top = t;
            self.st.scroll_bottom = b;
        }
        self.term.set_scrolling_region(top, bottom);
    }

    fn set_keypad_application_mode(&mut self) {
        self.term.set_keypad_application_mode();
    }

    fn unset_keypad_application_mode(&mut self) {
        self.term.unset_keypad_application_mode();
    }

    fn set_active_charset(&mut self, index: CharsetIndex) {
        self.term.set_active_charset(index);
    }

    fn configure_charset(&mut self, index: CharsetIndex, charset: StandardCharset) {
        self.term.configure_charset(index, charset);
    }

    fn set_color(&mut self, index: usize, color: VteRgb) {
        self.term.set_color(index, color);
    }

    fn dynamic_color_sequence(&mut self, _prefix: String, index: usize, _terminator: &str) {
        // A query: xterm answers with the colour in force, always ended by ST.
        use alacritty_terminal::vte::ansi::NamedColor;
        let colors = Colors::with_overrides(self.term.colors());
        let (ident, color) = match index {
            i if i < 256 => (format!("4;{i}"), colors.ansi[i]),
            i if i == NamedColor::Foreground as usize => ("10".to_owned(), colors.foreground),
            i if i == NamedColor::Background as usize => ("11".to_owned(), colors.background),
            i if i == NamedColor::Cursor as usize => ("12".to_owned(), colors.cursor),
            _ => return,
        };
        self.reply(format!("\x1b]{ident};{}\x1b\\", rgb_string(color)));
    }

    fn reset_color(&mut self, index: usize) {
        self.term.reset_color(index);
    }

    fn clipboard_store(&mut self, _: u8, _: &[u8]) {
        // No OSC 52: 0.9.4 has no clipboard addon.
    }

    fn clipboard_load(&mut self, _: u8, _: &str) {}

    fn decaln(&mut self) {
        self.term.decaln();
        // xterm's `E`s are content.
        let rows = self.term.screen_lines();
        let grid = self.term.grid_mut();
        for line in 0..rows {
            for cell in &mut grid[Line(line as i32)][..] {
                cell.flags.insert(WRITTEN);
            }
        }
    }

    fn push_title(&mut self) {
        // `CSI 22 t`: window ops are off (`windowOptions: {}`).
    }

    fn pop_title(&mut self) {}

    fn text_area_size_pixels(&mut self) {
        // `CSI 14 t`: no reply with `windowOptions: {}`.
    }

    fn text_area_size_chars(&mut self) {
        // `CSI 18 t`: no reply with `windowOptions: {}`.
    }

    fn set_hyperlink(&mut self, hyperlink: Option<Hyperlink>) {
        self.term.set_hyperlink(hyperlink);
    }

    fn set_mouse_cursor_icon(&mut self, _: CursorIcon) {
        // OSC 22: xterm.js doesn't have it.
    }

    fn report_keyboard_mode(&mut self) {
        // The kitty keyboard protocol is off: `CSI ? u` gets no answer.
    }

    fn push_keyboard_mode(&mut self, _mode: KeyboardModes) {}

    fn pop_keyboard_modes(&mut self, _to_pop: u16) {}

    fn set_keyboard_mode(&mut self, _mode: KeyboardModes, _behavior: KeyboardModesApplyBehavior) {}

    fn set_modify_other_keys(&mut self, _mode: ModifyOtherKeys) {
        // xterm.js has no modifyOtherKeys.
    }

    fn report_modify_other_keys(&mut self) {}

    fn set_scp(&mut self, _char_path: ScpCharPath, _update_mode: ScpUpdateMode) {}
}

/// Turns vte's synchronized-update buffering off: xterm parses everything as
/// it arrives and only holds back drawing (TERM-082), which [`XtermState`]
/// tracks from mode 2026 itself.
#[derive(Debug, Default)]
pub(crate) struct NoSyncBuffering;

impl ansi::Timeout for NoSyncBuffering {
    fn set_timeout(&mut self, _: std::time::Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}
