//! One session's terminal: alacritty's grid behind xterm.js 6's behavior, as
//! registry.ts creates it (13 px SF Mono aside, which is the UI's): 10 000
//! lines of scrollback, Unicode 11 widths, the fixed dark theme.
//!
//! The model never talks to a session. What it would send (query answers,
//! focus reports, keys, pastes) is returned to the caller, which writes it to
//! the session as registry.ts's `onData` does (TERM-025).

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Handler as _, Processor};
use nodus_domain::SessionKind;

use crate::handler::{CursorKind, MouseEncoding, MouseProtocol, NoSyncBuffering, XtermHandler, XtermState};
use crate::keys::Platform;
use crate::queries::{Scanner, SideSequence};

/// registry.ts: `scrollback: 10000` (TERM-074).
pub const SCROLLBACK: usize = 10_000;

/// xterm's default `wordSeparator`: double-click stops at these.
pub const WORD_SEPARATORS: &str = " ()[]{}',\"`";

/// xterm's smallest terminal (`MINIMUM_COLS`, `MINIMUM_ROWS`).
pub const MIN_COLS: usize = 2;
pub const MIN_ROWS: usize = 1;

/// xterm's size before the first fit.
pub const DEFAULT_COLS: usize = 80;
pub const DEFAULT_ROWS: usize = 24;

#[derive(Debug, Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// The modes a program has turned on that the app acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// DECCKM (mode 1): arrows send `ESC O`.
    pub application_cursor: bool,
    /// Mode 2004: pastes are bracketed (TERM-030, TERM-035).
    pub bracketed_paste: bool,
    /// Mode 1004: focus changes are reported (TERM-077).
    pub focus_reporting: bool,
    /// The alternate screen (47/1047/1049) is showing.
    pub alternate_screen: bool,
    pub mouse_protocol: MouseProtocol,
    pub mouse_encoding: MouseEncoding,
    /// Mode 2026: drawing is held until the program ends the update (TERM-082).
    pub synchronized_output: bool,
    /// DECTCEM (mode 25).
    pub show_cursor: bool,
}

pub struct Terminal {
    pub(crate) term: Term<VoidListener>,
    parser: Processor<NoSyncBuffering>,
    pub(crate) xterm: XtermState,
    scanner: Scanner,
    pub(crate) platform: Platform,
    pub(crate) kind: SessionKind,
    pub(crate) focused: bool,
    /// xterm's `_unprocessedDeadKey`: the key after a dead key comes as text.
    pub(crate) dead_key: bool,
    /// The selection's columns (its lines ride on `term.selection`).
    pub(crate) selection_columns: Option<crate::selection::Columns>,
    pub(crate) wheel: crate::pointer::WheelState,
    /// Bumped on every change a drawing could show.
    generation: u64,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("kind", &self.kind)
            .field("cols", &self.cols())
            .field("rows", &self.rows())
            .finish_non_exhaustive()
    }
}

impl Terminal {
    /// A terminal of `cols` × `rows` for a session of `kind`, behaving as
    /// xterm does on `platform`.
    pub fn new(kind: SessionKind, platform: Platform, cols: usize, rows: usize) -> Self {
        let size = Size {
            cols: cols.max(MIN_COLS),
            rows: rows.max(MIN_ROWS),
        };
        let config = Config {
            scrolling_history: SCROLLBACK,
            semantic_escape_chars: WORD_SEPARATORS.to_owned(),
            kitty_keyboard: false,
            osc52: alacritty_terminal::term::Osc52::Disabled,
            // cursorStyle 'block', cursorBlink true.
            default_cursor_style: CursorStyle {
                shape: CursorShape::Block,
                blinking: true,
            },
            ..Config::default()
        };
        let term = Term::new(config, &size, VoidListener);
        Self {
            term,
            parser: Processor::new(),
            xterm: XtermState::new(size.rows),
            scanner: Scanner::default(),
            platform,
            kind,
            focused: false,
            dead_key: false,
            selection_columns: None,
            wheel: Default::default(),
            generation: 0,
        }
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn cols(&self) -> usize {
        self.term.columns()
    }

    pub fn rows(&self) -> usize {
        self.term.screen_lines()
    }

    /// Changes whenever something a drawing shows may have changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Parses output from the program (TERM-007, TERM-010) and returns what
    /// the terminal answers, in order: each string is one write to the session.
    pub fn feed(&mut self, data: &str) -> Vec<String> {
        if data.is_empty() {
            return Vec::new();
        }
        let bytes = data.as_bytes();
        let found = self.scanner.scan(bytes);
        let mut start = 0;
        for (end, seq) in found {
            self.advance(&bytes[start..end]);
            self.side_sequence(seq);
            start = end;
        }
        self.advance(&bytes[start..]);
        self.touch();
        std::mem::take(&mut self.xterm.replies)
    }

    fn advance(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut handler = XtermHandler {
            term: &mut self.term,
            st: &mut self.xterm,
        };
        self.parser.advance(&mut handler, bytes);
    }

    fn side_sequence(&mut self, seq: SideSequence) {
        match seq {
            SideSequence::CursorPositionPrivate => {
                let handler = XtermHandler {
                    term: &mut self.term,
                    st: &mut self.xterm,
                };
                let (y, x) = handler.cursor_report();
                self.xterm.replies.push(format!("\x1b[?{y};{x}R"));
            }
            SideSequence::RequestStatusString(data) => {
                // `InputHandler.requestStatusString`.
                let reply = match data.as_str() {
                    "\"q" => "P1$r0\"q".to_owned(),
                    "\"p" => "P1$r61;1\"p".to_owned(),
                    "r" => format!("P1$r{};{}r", self.xterm.scroll_top + 1, self.xterm.scroll_bottom + 1),
                    "m" => "P1$r0m".to_owned(),
                    // From the options: block (2), less one when blinking.
                    " q" => format!("P1$r{} q", 2 - u8::from(self.xterm.option_cursor_blink)),
                    _ => "P0$r".to_owned(),
                };
                self.xterm.replies.push(format!("\x1b{reply}\x1b\\"));
            }
            SideSequence::SoftReset => self.soft_reset(),
        }
    }

    /// DECSTR (`InputHandler.softReset`): cursor shown, full scroll region,
    /// attributes and modes reset, saved cursor at the origin; the cursor stays.
    fn soft_reset(&mut self) {
        use alacritty_terminal::vte::ansi::{Attr, CharsetIndex, StandardCharset};
        let point = self.term.grid().cursor.point;
        let rows = self.rows();
        self.term.set_scrolling_region(1, None);
        let grid = self.term.grid_mut();
        grid.cursor.point = point;
        grid.cursor.input_needs_wrap = false;
        self.term.terminal_attribute(Attr::Reset);
        for index in [CharsetIndex::G0, CharsetIndex::G1, CharsetIndex::G2, CharsetIndex::G3] {
            self.term.configure_charset(index, StandardCharset::Ascii);
        }
        self.term.set_active_charset(CharsetIndex::G0);
        // `coreService.reset()`: every DEC private mode xterm keeps there.
        for (mode, on) in [
            (1u16, false),
            (6, false),
            (7, true),
            (25, true),
            (66, false),
            (1004, false),
            (2004, false),
            (2026, false),
            (45, false),
        ] {
            let mut handler = XtermHandler {
                term: &mut self.term,
                st: &mut self.xterm,
            };
            let mode = private_mode(mode);
            if on {
                handler.set_private_mode(mode);
            } else {
                handler.unset_private_mode(mode);
            }
        }
        self.term.unset_mode(ansi_mode_insert());
        self.term.unset_keypad_application_mode();
        self.xterm.decscusr = None;
        self.xterm.scroll_top = 0;
        self.xterm.scroll_bottom = rows - 1;
        // The saved cursor goes to the origin.
        let point = self.term.grid().cursor.point;
        self.term.grid_mut().cursor.point = alacritty_terminal::index::Point::new(Line(0), Column(0));
        self.term.save_cursor_position();
        self.term.grid_mut().cursor.point = point;
    }

    /// The modes in force.
    pub fn modes(&self) -> Modes {
        let m = *self.term.mode();
        Modes {
            application_cursor: m.contains(TermMode::APP_CURSOR),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            focus_reporting: m.contains(TermMode::FOCUS_IN_OUT),
            alternate_screen: m.contains(TermMode::ALT_SCREEN),
            mouse_protocol: self.xterm.mouse_protocol,
            mouse_encoding: self.xterm.mouse_encoding,
            synchronized_output: self.xterm.synchronized_output,
            show_cursor: m.contains(TermMode::SHOW_CURSOR),
        }
    }

    /// Ends a synchronized update the program never ended: xterm draws again
    /// after `SYNCHRONIZED_OUTPUT_TIMEOUT_MS` (1000 ms) of held drawing.
    pub fn end_synchronized_output(&mut self) {
        if self.xterm.synchronized_output {
            self.xterm.synchronized_output = false;
            self.touch();
        }
    }

    /// Resizes the grid as `term.resize(cols, rows)` does; false if the size
    /// didn't change. A change of rows clears the selection (TERM-057).
    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        let cols = cols.max(MIN_COLS);
        let rows = rows.max(MIN_ROWS);
        if cols == self.cols() && rows == self.rows() {
            return false;
        }
        let rows_changed = rows != self.rows();
        self.term.resize(Size { cols, rows });
        if rows_changed {
            self.clear_selection();
        }
        self.xterm.scroll_top = 0;
        self.xterm.scroll_bottom = rows - 1;
        self.touch();
        true
    }

    /// Lines of history above the screen.
    pub fn history_size(&self) -> usize {
        self.term.history_size()
    }

    /// How many lines the view is scrolled up from the bottom.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// xterm's `scrollLines(amount)`: negative scrolls up into the history.
    pub fn scroll_lines(&mut self, amount: i32) {
        self.term.scroll_display(Scroll::Delta(-amount));
        self.touch();
    }

    pub fn scroll_to_bottom(&mut self) {
        if self.display_offset() != 0 {
            self.term.scroll_display(Scroll::Bottom);
            self.touch();
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.term.scroll_display(Scroll::Top);
        self.touch();
    }

    /// Scrolls so the view starts `offset` lines up from the bottom.
    pub fn scroll_to_offset(&mut self, offset: usize) {
        let delta = offset as i32 - self.display_offset() as i32;
        if delta != 0 {
            self.term.scroll_display(Scroll::Delta(delta));
            self.touch();
        }
    }

    /// What `triggerDataEvent(data, true)` does before sending: the view
    /// returns to the bottom and the selection goes (TERM-054, TERM-057).
    pub(crate) fn before_user_input(&mut self) {
        self.scroll_to_bottom();
        self.clear_selection();
    }

    /// The text input path (`insertText`, a finished IME composition): what
    /// to send the session.
    pub fn input_text(&mut self, text: &str) -> String {
        self.dead_key = false;
        if text.is_empty() {
            return String::new();
        }
        self.before_user_input();
        self.xterm.cursor_initialized = true;
        text.to_owned()
    }

    /// xterm's `paste`: what to send the session for pasting `text`
    /// (TERM-035): line breaks become CR, and the whole is bracketed while
    /// the program has mode 2004 on, even when empty.
    pub fn paste(&mut self, text: &str) -> String {
        let data = crate::paste::prepare(text, self.modes().bracketed_paste);
        self.before_user_input();
        data
    }

    /// The terminal gets keyboard focus: `\x1b[I` while mode 1004 is on (TERM-077).
    pub fn focus(&mut self) -> Option<String> {
        if self.focused {
            return None;
        }
        self.focused = true;
        self.xterm.cursor_initialized = true;
        self.touch();
        self.modes().focus_reporting.then(|| "\x1b[I".to_owned())
    }

    /// It loses focus: `\x1b[O` while mode 1004 is on (TERM-005, TERM-077).
    pub fn blur(&mut self) -> Option<String> {
        if !self.focused {
            return None;
        }
        self.focused = false;
        self.touch();
        self.modes().focus_reporting.then(|| "\x1b[O".to_owned())
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// ⌘K (TERM-039, xterm's `clear()`): the cursor's line becomes the top
    /// line, every other line and the whole history are dropped, and the view
    /// goes to the top. Nothing is sent. Nothing happens when the cursor is
    /// on the first line with no history.
    pub fn clear(&mut self) {
        let history = self.history_size();
        let cursor_line = self.term.grid().cursor.point.line;
        if history == 0 && cursor_line.0 == 0 {
            return;
        }
        let rows = self.rows();
        let cols = self.cols();
        let grid = self.term.grid_mut();
        let row = grid[cursor_line].clone();
        grid.clear_history();
        grid[Line(0)] = row;
        // The kept line no longer runs on into the next.
        grid[Line(0)][Column(cols - 1)].flags.remove(Flags::WRAPLINE);
        let blank = alacritty_terminal::term::cell::Cell::default();
        for line in 1..rows {
            grid[Line(line as i32)].reset(&blank);
        }
        grid.cursor.point.line = Line(0);
        grid.scroll_display(Scroll::Bottom);
        self.clear_selection();
        self.touch();
    }

    /// `term.reset()` (TERM-021): screen, history, modes, colours and
    /// selection back to a new terminal's; the size and kind stay.
    pub fn reset(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        let mut fresh = Self::new(self.kind, self.platform, cols, rows);
        // Options outlive a reset, as does whether the cursor was ever shown.
        fresh.xterm.option_cursor_blink = self.xterm.option_cursor_blink;
        fresh.xterm.cursor_initialized = self.xterm.cursor_initialized;
        fresh.focused = self.focused;
        fresh.generation = self.generation;
        *self = fresh;
        self.touch();
    }

    /// The cursor as xterm draws it: where, which shape, whether it blinks.
    pub fn cursor_style(&self) -> (CursorKind, bool) {
        self.xterm.cursor_style()
    }

    /// Whether the cursor is drawn at all: shown (DECTCEM) and initialised
    /// (focus, a key, or a screen switch, TERM-076).
    pub fn cursor_visible(&self) -> bool {
        self.term.mode().contains(TermMode::SHOW_CURSOR) && self.xterm.cursor_initialized
    }

    /// The cursor as xterm keeps it (`buffer.x`, `buffer.y`): `x` is one past
    /// the last column while a wrap is pending.
    pub fn xterm_cursor(&self) -> (usize, usize) {
        let grid = self.term.grid();
        let x = grid.cursor.point.column.0 + usize::from(grid.cursor.input_needs_wrap);
        (x, grid.cursor.point.line.0.max(0) as usize)
    }

    /// The cursor's cell in grid lines (0 = top of the screen).
    pub fn cursor_point(&self) -> (i32, usize) {
        let p = self.term.grid().cursor.point;
        (p.line.0, p.column.0)
    }
}

fn private_mode(raw: u16) -> alacritty_terminal::vte::ansi::PrivateMode {
    use alacritty_terminal::vte::ansi::{NamedPrivateMode as N, PrivateMode};
    match raw {
        1 => PrivateMode::Named(N::CursorKeys),
        6 => PrivateMode::Named(N::Origin),
        7 => PrivateMode::Named(N::LineWrap),
        25 => PrivateMode::Named(N::ShowCursor),
        1004 => PrivateMode::Named(N::ReportFocusInOut),
        2004 => PrivateMode::Named(N::BracketedPaste),
        2026 => PrivateMode::Named(N::SyncUpdate),
        other => PrivateMode::Unknown(other),
    }
}

fn ansi_mode_insert() -> alacritty_terminal::vte::ansi::Mode {
    alacritty_terminal::vte::ansi::Mode::Named(alacritty_terminal::vte::ansi::NamedMode::Insert)
}
