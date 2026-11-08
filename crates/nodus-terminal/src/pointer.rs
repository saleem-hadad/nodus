//! The pointer over a terminal, as xterm.js 6 handles it: mouse reports for
//! programs that ask for them (`CoreMouseService`, TERM-063, TERM-071), the
//! wheel (TERM-069…TERM-072), ⌥/Alt-click moving the cursor (`MoveToCell.ts`,
//! TERM-062), and where a pointer lands in the grid (`Mouse.ts`, TERM-083).
//!
//! Positions are in the terminal's own unscaled CSS pixels, relative to the
//! top-left of its grid: the UI maps a pointer on a zoomed board back to them
//! first (TERM-083).

use crate::handler::{MouseEncoding, MouseProtocol};
use crate::terminal::Terminal;

/// `CoreMouseButton`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    None = 3,
    Wheel = 4,
}

/// `CoreMouseAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// A release, or the wheel turned up.
    Up = 0,
    /// A press, or the wheel turned down.
    Down = 1,
    Move = 32,
}

/// One mouse event for the program (`ICoreMouseEvent`): a 0-based cell and
/// pixel position, as `getMouseReportCoords` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub col: usize,
    pub row: usize,
    pub x: usize,
    pub y: usize,
    pub button: MouseButton,
    pub action: MouseAction,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A wheel turn: pixels (trackpads and most mice on a Mac) or lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WheelDelta {
    Pixels(f64),
    Lines(f64),
    Pages(f64),
}

/// The modifiers held during a wheel turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WheelModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// What a wheel turn did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WheelOutcome {
    /// Send this to the session as typed input: a mouse report, or the
    /// alternate screen's arrow key (TERM-070, TERM-071).
    Send(String),
    /// The view scrolled through the history (TERM-069).
    Scrolled,
    /// The terminal took the wheel and nothing changed.
    Consumed,
    /// Not the terminal's: let it go on (the board pans, TERM-072, TERM-105).
    Ignored,
}

/// xterm's `fastScrollSensitivity`.
pub const FAST_SCROLL_SENSITIVITY: f64 = 5.0;
/// VS Code's scrollable element: 50 px per wheel line.
const SCROLL_WHEEL_SENSITIVITY: f64 = 50.0;

/// The wheel state a terminal keeps between turns.
#[derive(Debug, Clone, Default)]
pub(crate) struct WheelState {
    /// `CoreMouseService._wheelPartialScroll`.
    partial: f64,
    /// The scrollable element's `scrollTop` and the view it was synced at.
    scroll_top: Option<(f64, usize)>,
    /// The last reported event (`_lastEvent`), for debouncing moves.
    last_report: Option<MouseEvent>,
}

impl MouseProtocol {
    /// Whether the program wants this kind of event at all (`events`).
    fn wants(self, button: MouseButton, action: MouseAction) -> bool {
        match self {
            MouseProtocol::None => false,
            MouseProtocol::X10 => action == MouseAction::Down && button != MouseButton::Wheel,
            MouseProtocol::Vt200 => action != MouseAction::Move,
            MouseProtocol::Drag => !(action == MouseAction::Move && button == MouseButton::None),
            MouseProtocol::Any => true,
        }
    }
}

/// `eventCode` for the default and SGR encodings.
fn event_code(e: &MouseEvent, sgr: bool) -> u32 {
    let mut code = if e.ctrl { 16 } else { 0 } | if e.shift { 4 } else { 0 } | if e.alt { 8 } else { 0 };
    if e.button == MouseButton::Wheel {
        code |= 64 | e.action as u32;
    } else {
        code |= e.button as u32 & 3;
        if e.action == MouseAction::Move {
            code |= 32;
        } else if e.action == MouseAction::Up && !sgr {
            code |= MouseButton::None as u32;
        }
    }
    code
}

impl Terminal {
    /// Whether a program has mouse reporting on, which turns local selection
    /// off (TERM-063).
    pub fn mouse_events_active(&self) -> bool {
        self.xterm.mouse_protocol != MouseProtocol::None
    }

    /// `shouldForceSelection`: off a Mac Shift selects locally even while a
    /// program reports the mouse; on a Mac nothing does
    /// (`macOptionClickForcesSelection: false`).
    pub fn should_force_selection(&self, shift: bool) -> bool {
        !self.platform.is_mac() && shift
    }

    /// `triggerMouseEvent`: the report to send for `e`, if the program's
    /// protocol wants it. The default (X10-style) encoding goes out as
    /// binary data, which 0.9.4 never forwards (registry.ts only listens to
    /// `onData`), so only SGR and SGR-pixel reports reach a program.
    pub fn mouse_report(&mut self, mut e: MouseEvent) -> Option<String> {
        if e.col >= self.cols() || e.row >= self.rows() {
            return None;
        }
        if (e.button == MouseButton::Wheel && e.action == MouseAction::Move)
            || (e.button == MouseButton::None && e.action != MouseAction::Move)
        {
            return None;
        }
        e.col += 1;
        e.row += 1;
        let encoding = self.xterm.mouse_encoding;
        if e.action == MouseAction::Move
            && let Some(last) = self.wheel.last_report
        {
            let same_place = if encoding == MouseEncoding::SgrPixels {
                (last.x, last.y) == (e.x, e.y)
            } else {
                (last.col, last.row) == (e.col, e.row)
            };
            if same_place
                && last.button == e.button
                && last.action == e.action
                && (last.ctrl, last.alt, last.shift) == (e.ctrl, e.alt, e.shift)
            {
                return None;
            }
        }
        let protocol = self.xterm.mouse_protocol;
        if !protocol.wants(e.button, e.action) {
            return None;
        }
        if protocol == MouseProtocol::X10 {
            e.ctrl = false;
            e.alt = false;
            e.shift = false;
        }
        self.wheel.last_report = Some(e);
        let final_char = if e.action == MouseAction::Up && e.button != MouseButton::Wheel {
            'm'
        } else {
            'M'
        };
        let report = match encoding {
            MouseEncoding::Default => return None,
            MouseEncoding::Sgr => format!("\x1b[<{};{};{}{final_char}", event_code(&e, true), e.col, e.row),
            MouseEncoding::SgrPixels => format!("\x1b[<{};{};{}{final_char}", event_code(&e, true), e.x, e.y),
        };
        self.before_user_input();
        Some(report)
    }

    /// `consumeWheelEvent`: whole lines for a mouse report or the alternate
    /// screen's arrows. Pixels count in cells (×0.3 for a trackpad's small
    /// deltas), Alt or Ctrl multiply by 5, Shift gives nothing.
    pub fn wheel_lines(&mut self, delta: WheelDelta, mods: WheelModifiers, cell_height: f64) -> i32 {
        let raw = match delta {
            WheelDelta::Pixels(d) | WheelDelta::Lines(d) | WheelDelta::Pages(d) => d,
        };
        if raw == 0.0 || mods.shift || cell_height <= 0.0 {
            return 0;
        }
        let mut amount = raw
            * if mods.alt || mods.ctrl {
                FAST_SCROLL_SENSITIVITY
            } else {
                1.0
            };
        match delta {
            WheelDelta::Pixels(_) => {
                amount /= cell_height;
                if raw.abs() < 50.0 {
                    amount *= 0.3;
                }
                self.wheel.partial += amount;
                let whole = self.wheel.partial.abs().floor() * self.wheel.partial.signum();
                self.wheel.partial %= 1.0;
                whole as i32
            }
            WheelDelta::Lines(_) => amount as i32,
            WheelDelta::Pages(_) => (amount * self.rows() as f64) as i32,
        }
    }

    /// A wheel turn over the terminal at cell (`col`, `row`), pixel (`x`,
    /// `y`). `default_prevented`: something else already handled it (the
    /// board's gesture, TERM-072). `cell_height` is the cell's CSS height.
    #[allow(clippy::too_many_arguments)]
    pub fn wheel(
        &mut self,
        delta: WheelDelta,
        mods: WheelModifiers,
        default_prevented: bool,
        cell_height: f64,
        col: usize,
        row: usize,
        x: usize,
        y: usize,
    ) -> WheelOutcome {
        if default_prevented {
            return WheelOutcome::Ignored;
        }
        let raw = match delta {
            WheelDelta::Pixels(d) | WheelDelta::Lines(d) | WheelDelta::Pages(d) => d,
        };
        // A program reporting the wheel gets it (TERM-071).
        if self.xterm.mouse_protocol.wants(MouseButton::Wheel, MouseAction::Down) {
            if raw == 0.0 {
                return WheelOutcome::Consumed;
            }
            if self.wheel_lines(delta, mods, cell_height) == 0 {
                return WheelOutcome::Consumed;
            }
            let action = if raw < 0.0 { MouseAction::Up } else { MouseAction::Down };
            let e = MouseEvent {
                col,
                row,
                x,
                y,
                button: MouseButton::Wheel,
                action,
                ctrl: mods.ctrl,
                alt: mods.alt,
                shift: mods.shift,
            };
            return self.mouse_report(e).map_or(WheelOutcome::Consumed, WheelOutcome::Send);
        }
        // The alternate screen has no history: one arrow per turn (TERM-070).
        if self.modes().alternate_screen {
            if raw == 0.0 {
                return WheelOutcome::Ignored;
            }
            if self.wheel_lines(delta, mods, cell_height) == 0 {
                return WheelOutcome::Consumed;
            }
            let prefix = if self.modes().application_cursor { 'O' } else { '[' };
            let arrow = if raw < 0.0 { 'A' } else { 'B' };
            self.before_user_input();
            return WheelOutcome::Send(format!("\x1b{prefix}{arrow}"));
        }
        self.scroll_wheel(delta, mods, cell_height)
    }

    /// The normal screen: VS Code's scrollable element moves `scrollTop` by
    /// 50 px a line (a pixel delta is 1/40 of a line, so 1.25 × the pixels),
    /// ×5 with Alt, at once (no smooth scrolling), and the view follows to
    /// the nearest row (TERM-069). Off a Mac Shift turns it horizontal, which
    /// a terminal doesn't scroll.
    fn scroll_wheel(&mut self, delta: WheelDelta, mods: WheelModifiers, cell_height: f64) -> WheelOutcome {
        if cell_height <= 0.0 {
            return WheelOutcome::Ignored;
        }
        let mut lines = match delta {
            WheelDelta::Pixels(d) => -d / 40.0,
            WheelDelta::Lines(d) | WheelDelta::Pages(d) => -d,
        };
        if lines == 0.0 || (!self.platform.is_mac() && mods.shift) {
            return WheelOutcome::Ignored;
        }
        if mods.alt {
            lines *= FAST_SCROLL_SENSITIVITY;
        }
        let history = self.history_size();
        let ydisp = history - self.display_offset();
        // Start from where the view is unless the last turn left a fraction of a row.
        let top = match self.wheel.scroll_top {
            Some((top, at)) if at == ydisp => top,
            _ => ydisp as f64 * cell_height,
        };
        let step = SCROLL_WHEEL_SENSITIVITY * lines;
        let step = if step < 0.0 { step.floor() } else { step.ceil() };
        let max = history as f64 * cell_height;
        let desired = (top - step).clamp(0.0, max);
        if desired == top {
            return WheelOutcome::Ignored;
        }
        let row = nodus_domain::js::round(desired / cell_height) as usize;
        self.wheel.scroll_top = Some((desired, row));
        if row != ydisp {
            self.scroll_to_offset(history - row.min(history));
        }
        WheelOutcome::Scrolled
    }

    /// `moveToCellSequence`: the arrow keys that take the cursor to cell
    /// (`col`, `row`) of the view (⌥/Alt-click, TERM-062). Nothing while the
    /// view is scrolled up.
    pub fn move_to_cell_sequence(&mut self, col: usize, row: usize) -> Option<String> {
        if self.display_offset() != 0 {
            return None;
        }
        let app = self.modes().application_cursor;
        let seq = |d: char| format!("\x1b{}{d}", if app { 'O' } else { '[' });
        let (start_x, start_y) = self.xterm_cursor();
        let (start_x, start_y) = (start_x as i64, start_y as i64);
        let (target_x, target_y) = (col as i64, row as i64);
        let cols = self.cols() as i64;
        let repeat = |n: i64, s: String| s.repeat(n.max(0) as usize);
        let out = if !self.modes().alternate_screen {
            // The normal screen only moves along the line.
            if start_y == target_y {
                let d = if start_x > target_x { 'D' } else { 'C' };
                repeat((start_x - target_x).abs(), seq(d))
            } else {
                let d = if start_y > target_y { 'D' } else { 'C' };
                let rows = (start_y - target_y).abs();
                let from_end = cols - if start_y > target_y { target_x } else { start_x };
                let from_start = (if start_y > target_y { start_x } else { target_x }) - 1;
                repeat(from_end + (rows - 1) * cols + 1 + from_start, seq(d))
            }
        } else {
            let mv = MoveToCell { term: self, app };
            mv.reset_starting_row(start_x, start_y, target_y)
                + &mv.move_to_requested_row(start_y, target_y)
                + &mv.move_to_requested_col(start_x, start_y, target_x, target_y)
        };
        self.before_user_input();
        Some(out)
    }
}

/// The alternate-screen half of `MoveToCell.ts`, where rows count from the
/// top of the screen.
struct MoveToCell<'a> {
    term: &'a Terminal,
    app: bool,
}

impl MoveToCell<'_> {
    fn seq(&self, d: char) -> String {
        format!("\x1b{}{d}", if self.app { 'O' } else { '[' })
    }

    fn wrapped(&self, row: i64) -> bool {
        row >= 0 && self.term.is_wrapped(row as i32)
    }

    fn wrapped_rows_for_row(&self, mut row: i64) -> i64 {
        let mut count = 0;
        let rows = self.term.rows() as i64;
        let mut wraps = self.wrapped(row);
        while wraps && row >= 0 && row < rows {
            count += 1;
            row -= 1;
            wraps = self.wrapped(row);
        }
        count
    }

    fn vertical(start_y: i64, target_y: i64) -> char {
        if start_y > target_y { 'A' } else { 'B' }
    }

    fn wrapped_rows_count(&self, start_y: i64, target_y: i64) -> i64 {
        let start = start_y - self.wrapped_rows_for_row(start_y);
        let end = target_y - self.wrapped_rows_for_row(target_y);
        let dir = if Self::vertical(start_y, target_y) == 'A' {
            -1
        } else {
            1
        };
        (0..(start - end).abs())
            .filter(|i| self.wrapped(start + dir * i))
            .count() as i64
    }

    fn move_to_requested_row(&self, start_y: i64, target_y: i64) -> String {
        let start = start_y - self.wrapped_rows_for_row(start_y);
        let end = target_y - self.wrapped_rows_for_row(target_y);
        let n = (start - end).abs() - self.wrapped_rows_count(start_y, target_y);
        self.seq(Self::vertical(start_y, target_y)).repeat(n.max(0) as usize)
    }

    /// `bufferLine`: the text walked over between two cells.
    fn buffer_line(&self, mut start_col: i64, start_row: i64, end_col: i64, end_row: i64, forward: bool) -> String {
        let cols = self.term.cols() as i64;
        let lines = self.term.rows() as i64;
        let text = |row: i64, from: i64, to: i64| {
            if row < 0 || row >= lines {
                return String::new();
            }
            self.term
                .line_text(row as i32, false, from.max(0) as usize, Some(to.max(0) as usize))
        };
        let (mut col, mut row) = (start_col, start_row);
        let mut out = String::new();
        while (col != end_col || row != end_row) && row >= 0 && row < lines {
            col += if forward { 1 } else { -1 };
            if forward && col > cols - 1 {
                out += &text(row, start_col, col);
                col = 0;
                start_col = 0;
                row += 1;
            } else if !forward && col < 0 {
                out += &text(row, 0, start_col + 1);
                col = cols - 1;
                start_col = col;
                row -= 1;
            }
        }
        out + &text(row, start_col, col)
    }

    fn reset_starting_row(&self, start_x: i64, start_y: i64, target_y: i64) -> String {
        if self.move_to_requested_row(start_y, target_y).is_empty() {
            return String::new();
        }
        let back = self.buffer_line(
            start_x,
            start_y,
            start_x,
            start_y - self.wrapped_rows_for_row(start_y),
            false,
        );
        self.seq('D').repeat(nodus_domain::js::utf16_len(&back))
    }

    fn horizontal(&self, start_x: i64, start_y: i64, target_x: i64, target_y: i64) -> char {
        // xterm passes targetX as the row here; kept as it is.
        let start_row = if self.move_to_requested_row(target_x, target_y).is_empty() {
            start_y
        } else {
            target_y - self.wrapped_rows_for_row(target_y)
        };
        if (start_x < target_x && start_row <= target_y) || (start_x >= target_x && start_row < target_y) {
            'C'
        } else {
            'D'
        }
    }

    fn move_to_requested_col(&self, start_x: i64, start_y: i64, target_x: i64, target_y: i64) -> String {
        let start_row = if self.move_to_requested_row(start_y, target_y).is_empty() {
            start_y
        } else {
            target_y - self.wrapped_rows_for_row(target_y)
        };
        let d = self.horizontal(start_x, start_y, target_x, target_y);
        let walked = self.buffer_line(start_x, start_row, target_x, target_y, d == 'C');
        self.seq(d).repeat(nodus_domain::js::utf16_len(&walked))
    }
}

/// `getCoords`: the 1-based cell under a pointer at (`x`, `y`) CSS pixels
/// from the grid's top-left. For a selection the column rounds to the
/// nearer cell edge and may be one past the last column.
pub fn pointer_cell(
    x: f64,
    y: f64,
    cols: usize,
    rows: usize,
    cell_width: f64,
    cell_height: f64,
    selection: bool,
) -> (usize, usize) {
    let col = ((x + if selection { cell_width / 2.0 } else { 0.0 }) / cell_width).ceil();
    let row = (y / cell_height).ceil();
    let max_col = cols as f64 + if selection { 1.0 } else { 0.0 };
    (col.clamp(1.0, max_col) as usize, row.clamp(1.0, rows as f64) as usize)
}

/// `getMouseReportCoords`: the 0-based cell and whole-pixel position a mouse
/// report gives, clamped to the grid's `width` × `height` CSS pixels.
pub fn report_position(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    cell_width: f64,
    cell_height: f64,
) -> (usize, usize, usize, usize) {
    let x = x.clamp(0.0, (width - 1.0).max(0.0));
    let y = y.clamp(0.0, (height - 1.0).max(0.0));
    (
        (x / cell_width).floor() as usize,
        (y / cell_height).floor() as usize,
        x.floor() as usize,
        y.floor() as usize,
    )
}

#[cfg(test)]
mod tests {
    use nodus_domain::SessionKind;

    use super::*;
    use crate::keys::Platform;

    fn term(platform: Platform, text: &str) -> Terminal {
        let mut t = Terminal::new(SessionKind::Terminal, platform, 20, 5);
        t.feed(text);
        t
    }

    fn click(col: usize, row: usize, button: MouseButton, action: MouseAction) -> MouseEvent {
        MouseEvent {
            col,
            row,
            x: col * 8,
            y: row * 18,
            button,
            action,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    #[test]
    fn term_071_sgr_reports() {
        let mut t = term(Platform::Mac, "\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            t.mouse_report(click(2, 3, MouseButton::Left, MouseAction::Down))
                .as_deref(),
            Some("\x1b[<0;3;4M")
        );
        assert_eq!(
            t.mouse_report(click(2, 3, MouseButton::Left, MouseAction::Up))
                .as_deref(),
            Some("\x1b[<0;3;4m")
        );
        // 1000 doesn't report moves; 1002 reports drags; 1003 everything.
        assert_eq!(t.mouse_report(click(4, 3, MouseButton::Left, MouseAction::Move)), None);
        t.feed("\x1b[?1002h");
        assert_eq!(
            t.mouse_report(click(4, 3, MouseButton::Left, MouseAction::Move))
                .as_deref(),
            Some("\x1b[<32;5;4M")
        );
        assert_eq!(
            t.mouse_report(click(4, 3, MouseButton::Left, MouseAction::Move)),
            None,
            "debounced"
        );
        assert_eq!(t.mouse_report(click(4, 3, MouseButton::None, MouseAction::Move)), None);
        t.feed("\x1b[?1003h");
        assert_eq!(
            t.mouse_report(click(5, 3, MouseButton::None, MouseAction::Move))
                .as_deref(),
            Some("\x1b[<35;6;4M")
        );
        let wheel = MouseEvent {
            ctrl: true,
            ..click(0, 0, MouseButton::Wheel, MouseAction::Up)
        };
        assert_eq!(t.mouse_report(wheel).as_deref(), Some("\x1b[<80;1;1M"));
        t.feed("\x1b[?1016h");
        assert_eq!(
            t.mouse_report(click(1, 1, MouseButton::Right, MouseAction::Down))
                .as_deref(),
            Some("\x1b[<2;8;18M")
        );
    }

    #[test]
    fn term_071_x10_encoding_never_reaches_the_program() {
        // xterm sends it as binary data, which registry.ts doesn't forward.
        let mut t = term(Platform::Mac, "\x1b[?1000h");
        assert!(t.mouse_events_active());
        assert_eq!(t.mouse_report(click(0, 0, MouseButton::Left, MouseAction::Down)), None);
    }

    #[test]
    fn term_063_shift_forces_selection_off_a_mac_only() {
        assert!(term(Platform::Windows, "").should_force_selection(true));
        assert!(!term(Platform::Mac, "").should_force_selection(true));
    }

    #[test]
    fn term_070_alternate_screen_wheel_sends_one_arrow() {
        let mut t = term(Platform::Mac, "\x1b[?1049h");
        let none = WheelModifiers::default();
        assert_eq!(
            t.wheel(WheelDelta::Lines(3.0), none, false, 18.0, 0, 0, 0, 0),
            WheelOutcome::Send("\x1b[B".into())
        );
        t.feed("\x1b[?1h");
        assert_eq!(
            t.wheel(WheelDelta::Lines(-3.0), none, false, 18.0, 0, 0, 0, 0),
            WheelOutcome::Send("\x1bOA".into())
        );
        // A trackpad's small pixel deltas add up (×0.3) before an arrow is sent.
        assert_eq!(
            t.wheel(WheelDelta::Pixels(10.0), none, false, 18.0, 0, 0, 0, 0),
            WheelOutcome::Consumed
        );
        let mut sent = 0;
        for _ in 0..10 {
            if let WheelOutcome::Send(_) = t.wheel(WheelDelta::Pixels(10.0), none, false, 18.0, 0, 0, 0, 0) {
                sent += 1;
            }
        }
        assert_eq!(sent, 1);
        assert_eq!(
            t.wheel(WheelDelta::Lines(3.0), none, true, 18.0, 0, 0, 0, 0),
            WheelOutcome::Ignored
        );
    }

    #[test]
    fn term_069_normal_screen_wheel_scrolls_the_history() {
        let mut t = Terminal::new(SessionKind::Terminal, Platform::Mac, 20, 5);
        for i in 0..40 {
            t.feed(&format!("line {i}\r\n"));
        }
        let none = WheelModifiers::default();
        // 1.25 × 37 px = 47 px up, 2.5 rows of 18.5 px: rounds to 3.
        assert_eq!(
            t.wheel(WheelDelta::Pixels(-37.0), none, false, 18.5, 0, 0, 0, 0),
            WheelOutcome::Scrolled
        );
        assert_eq!(t.display_offset(), 3);
        // Alt ×5: one line is 250 px, 14 rows.
        let alt = WheelModifiers { alt: true, ..none };
        t.wheel(WheelDelta::Lines(-1.0), alt, false, 18.5, 0, 0, 0, 0);
        assert_eq!(t.display_offset(), 16);
        // Off a Mac Shift turns it horizontal.
        let mut w = Terminal::new(SessionKind::Terminal, Platform::Windows, 20, 5);
        w.feed(&"x\r\n".repeat(20));
        let shift = WheelModifiers { shift: true, ..none };
        assert_eq!(
            w.wheel(WheelDelta::Lines(-1.0), shift, false, 18.5, 0, 0, 0, 0),
            WheelOutcome::Ignored
        );
    }

    #[test]
    fn term_062_alt_click_moves_along_the_line() {
        let mut t = term(Platform::Mac, "hello world");
        assert_eq!(
            t.move_to_cell_sequence(3, 0).as_deref(),
            Some("\x1b[D".repeat(8).as_str())
        );
        assert_eq!(
            t.move_to_cell_sequence(13, 0).as_deref(),
            Some("\x1b[C".repeat(2).as_str())
        );
        t.feed("\r\nab");
        // One row up: the rest of that row, then to the column.
        assert_eq!(
            t.move_to_cell_sequence(5, 0).as_deref(),
            Some("\x1b[D".repeat(20 - 5 + 1 + 1).as_str())
        );
    }

    #[test]
    fn term_062_alt_click_in_the_alternate_screen_moves_rows() {
        let mut t = term(Platform::Mac, "\x1b[?1049h\x1b[3;5H");
        assert_eq!(t.move_to_cell_sequence(4, 0).as_deref(), Some("\x1b[A\x1b[A"));
    }

    #[test]
    fn term_083_pointer_cells() {
        assert_eq!(pointer_cell(0.0, 0.0, 80, 24, 8.0, 18.5, false), (1, 1));
        assert_eq!(pointer_cell(9.0, 19.0, 80, 24, 8.0, 18.5, false), (2, 2));
        assert_eq!(pointer_cell(13.0, 1.0, 80, 24, 8.0, 18.5, true), (3, 1));
        assert_eq!(pointer_cell(10_000.0, 10_000.0, 80, 24, 8.0, 18.5, true), (81, 24));
        assert_eq!(report_position(17.5, 40.0, 640.0, 444.0, 8.0, 18.5), (2, 2, 17, 40));
    }
}
