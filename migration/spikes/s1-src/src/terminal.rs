//! Terminal: `$SHELL -l` on a portable-pty PTY, parsed by alacritty_terminal, drawn by our own
//! GPUI element (no code from Zed's GPL terminal_view).
//!
//! Threading: one reader thread per PTY parses bytes into the `Term` under a FairMutex and pings
//! the UI over a bounded(1) channel (coalesces bursts); the UI task just `cx.notify()`s, and GPUI
//! draws at most once per display frame. Painting locks the Term briefly in prepaint.
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, CursorShape, NamedColor, Processor, Rgb, StdSyncHandler};
use gpui::*;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::theme::rgba_hex;
use crate::zoom;

// ---------------------------------------------------------------------------------------------
// Session: PTY + Term
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Listener {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        match event {
            // Replies the terminal owes the program (DA, DSR/cursor position, ...).
            TermEvent::PtyWrite(s) => {
                let _ = self.writer.lock().unwrap().write_all(s.as_bytes());
            }
            TermEvent::Title(t) => *self.title.lock().unwrap() = Some(t),
            TermEvent::ResetTitle => *self.title.lock().unwrap() = None,
            _ => {}
        }
    }
}

pub struct Session {
    pub term: Arc<FairMutex<Term<Listener>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    pub size: Mutex<(u16, u16)>,
    pub title: Arc<Mutex<Option<String>>>,
    pub bytes_read: Arc<AtomicU64>,
    pub resizes: AtomicU64,
    pub last_dims: Mutex<(f32, f32)>,
}

/// A throwaway working directory, so demos never list the user's own files.
pub fn demo_cwd() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("s1-term-demo");
    let _ = std::fs::create_dir_all(dir.join("src"));
    let _ = std::fs::create_dir_all(dir.join("docs"));
    for (name, body) in [
        ("README.md", "# demo\n"),
        ("notes.txt", "hello from the S1 spike\n"),
        ("run.sh", "#!/bin/sh\necho hi\n"),
        ("src/main.rs", "fn main() { println!(\"hi\"); }\n"),
    ] {
        let p = dir.join(name);
        if !p.exists() {
            let _ = std::fs::write(&p, body);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755));
    }
    dir
}

impl Session {
    pub fn spawn(cols: u16, rows: u16) -> anyhow::Result<(Arc<Session>, smol::channel::Receiver<()>)> {
        let pty = native_pty_system();
        let pair = pty.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.arg("-l");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.cwd(demo_cwd());
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(pair.master.take_writer()?));
        let title = Arc::new(Mutex::new(None));
        let listener = Listener { writer: writer.clone(), title: title.clone() };
        let term = Arc::new(FairMutex::new(Term::new(
            Config { scrolling_history: 10_000, ..Config::default() },
            &TermSize::new(cols as usize, rows as usize),
            listener,
        )));
        let (tx, rx) = smol::channel::bounded::<()>(1);
        let bytes_read = Arc::new(AtomicU64::new(0));
        {
            let term = term.clone();
            let bytes_read = bytes_read.clone();
            std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
                let mut processor: Processor<StdSyncHandler> = Processor::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            bytes_read.fetch_add(n as u64, Ordering::Relaxed);
                            {
                                let mut t = term.lock();
                                processor.advance(&mut *t, &buf[..n]);
                            }
                            let _ = tx.try_send(());
                        }
                    }
                }
                let _ = tx.try_send(());
            })?;
        }
        Ok((
            Arc::new(Session {
                term,
                writer,
                master: Mutex::new(pair.master),
                child: Mutex::new(child),
                size: Mutex::new((cols, rows)),
                title,
                bytes_read,
                resizes: AtomicU64::new(0),
                last_dims: Mutex::new((0., 0.)),
            }),
            rx,
        ))
    }

    pub fn write(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: f32, cell_h: f32) {
        let mut size = self.size.lock().unwrap();
        if *size == (cols, rows) {
            return;
        }
        *size = (cols, rows);
        self.resizes.fetch_add(1, Ordering::Relaxed);
        let _ = self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: (cols as f32 * cell_w) as u16,
            pixel_height: (rows as f32 * cell_h) as u16,
        });
        self.term.lock().resize(TermSize::new(cols as usize, rows as usize));
    }

    /// Plain text of the visible screen (for autopilot assertions).
    pub fn screen_text(&self) -> String {
        let term = self.term.lock();
        let content = term.renderable_content();
        let rows = term.screen_lines();
        let cols = term.columns();
        let mut lines = vec![vec![' '; cols]; rows];
        for item in content.display_iter {
            let row = item.point.line.0 + content.display_offset as i32;
            if row >= 0 && (row as usize) < rows && item.point.column.0 < cols {
                lines[row as usize][item.point.column.0] = item.cell.c;
            }
        }
        lines.into_iter().map(|l| l.into_iter().collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    pub fn kill(&self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

// ---------------------------------------------------------------------------------------------
// Colors (Nodus' xterm theme)
// ---------------------------------------------------------------------------------------------

const ANSI: [u32; 16] = [
    0x3a3a3c, 0xff6b62, 0x5fd068, 0xf5c451, 0x5aa9ff, 0xd68cf7, 0x5ad1e0, 0xd8d8dc, // normal
    0x6e6e73, 0xff8a80, 0x7fe08a, 0xffd978, 0x80bdff, 0xe2a8fb, 0x80e2ee, 0xf5f5f7, // bright
];
const FG: u32 = 0xe6e6e6;
const BG: u32 = 0x1e1e20;

fn rgb(c: Rgb) -> Hsla {
    Rgba { r: c.r as f32 / 255., g: c.g as f32 / 255., b: c.b as f32 / 255., a: 1. }.into()
}

fn indexed(i: u8) -> Hsla {
    match i {
        0..=15 => rgba_hex(ANSI[i as usize], 1.),
        16..=231 => {
            let i = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { x * 40 + 55 };
            rgb(Rgb { r: v(i / 36), g: v((i / 6) % 6), b: v(i % 6) })
        }
        _ => {
            let g = (i - 232) * 10 + 8;
            rgb(Rgb { r: g, g, b: g })
        }
    }
}

fn resolve(color: AColor, colors: &alacritty_terminal::term::color::Colors, bold: bool, is_fg: bool) -> Hsla {
    match color {
        AColor::Spec(c) => rgb(c),
        AColor::Indexed(i) => {
            let i = if bold && is_fg && i < 8 { i + 8 } else { i };
            colors[i as usize].map(rgb).unwrap_or_else(|| indexed(i))
        }
        AColor::Named(n) => {
            let idx = n as usize;
            if let Some(c) = colors[idx] {
                return rgb(c);
            }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => rgba_hex(FG, 1.),
                NamedColor::DimForeground => rgba_hex(FG, 0.66),
                NamedColor::Background => rgba_hex(BG, 1.),
                NamedColor::Cursor => rgba_hex(FG, 1.),
                _ if idx < 16 => {
                    // xterm.js drawBoldTextInBrightColors (default on).
                    let idx = if bold && is_fg && idx < 8 { idx + 8 } else { idx };
                    rgba_hex(ANSI[idx], 1.)
                }
                _ if idx < 256 => indexed(idx as u8),
                _ => rgba_hex(FG, 1.),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Keys -> bytes
// ---------------------------------------------------------------------------------------------

pub fn keystroke_to_bytes(ks: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if m.platform {
        return None; // ⌘-shortcuts belong to the app
    }
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modcode = 1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.control as u8);
    let csi_mod = |final_: &str, app: &str| -> Vec<u8> {
        if modcode > 1 {
            format!("\x1b[1;{modcode}{final_}").into_bytes()
        } else if app_cursor {
            app.as_bytes().to_vec()
        } else {
            format!("\x1b[{final_}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modcode > 1 { format!("\x1b[{n};{modcode}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let bytes = match ks.key.as_str() {
        "enter" => b"\r".to_vec(),
        "backspace" => if m.control { vec![0x08] } else if m.alt { b"\x1b\x7f".to_vec() } else { vec![0x7f] },
        "tab" => if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
        "escape" => b"\x1b".to_vec(),
        "up" => csi_mod("A", "\x1bOA"),
        "down" => csi_mod("B", "\x1bOB"),
        "right" => csi_mod("C", "\x1bOC"),
        "left" => csi_mod("D", "\x1bOD"),
        "home" => csi_mod("H", "\x1bOH"),
        "end" => csi_mod("F", "\x1bOF"),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "delete" => tilde(3),
        "insert" => tilde(2),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        key if m.control && key.chars().count() == 1 => {
            let c = key.chars().next().unwrap().to_ascii_lowercase();
            match c {
                'a'..='z' => vec![c as u8 - b'a' + 1],
                '[' => vec![0x1b],
                '\\' => vec![0x1c],
                ']' => vec![0x1d],
                '^' | '6' => vec![0x1e],
                '_' | '-' => vec![0x1f],
                '@' | '2' => vec![0],
                _ => return None,
            }
        }
        _ => {
            // macOptionIsMeta: false in Nodus -> option composes (key_char is e.g. "ß").
            let text = ks.key_char.clone().or_else(|| (ks.key == "space").then(|| " ".to_string()))?;
            text.into_bytes()
        }
    };
    Some(bytes)
}

// ---------------------------------------------------------------------------------------------
// View + element
// ---------------------------------------------------------------------------------------------

pub struct TerminalView {
    pub session: Arc<Session>,
    pub focus: FocusHandle,
    pub frames: u64,
    _pump: Task<()>,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (session, rx) = Session::spawn(80, 24).expect("spawn pty");
        let pump = cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self { session, focus: cx.focus_handle(), frames: 0, _pump: pump }
    }

    pub fn type_text(&self, s: &str) {
        self.session.write(s.as_bytes());
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames += 1;
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .bg(rgba_hex(BG, 1.))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                let ks = &ev.keystroke;
                if ks.modifiers.platform && ks.key == "v" {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        let bracketed = this.session.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
                        let mut bytes = Vec::new();
                        if bracketed { bytes.extend_from_slice(b"\x1b[200~"); }
                        bytes.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
                        if bracketed { bytes.extend_from_slice(b"\x1b[201~"); }
                        this.session.write(&bytes);
                    }
                    cx.stop_propagation();
                    return;
                }
                let mode = *this.session.term.lock().mode();
                if let Some(bytes) = keystroke_to_bytes(ks, mode) {
                    // Typing snaps back to the bottom like xterm.
                    this.session.term.lock().scroll_display(Scroll::Bottom);
                    this.session.write(&bytes);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, window, cx| {
                window.focus(&this.focus, cx);
            }))
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, window, cx| {
                // Inside the board, the board owns the wheel (pan/zoom); a panel terminal scrolls.
                if zoom::current_scale(window) != 1.0 && ev.modifiers.control {
                    return;
                }
                let line_h = 16.0 * zoom::current_scale(window);
                let dy = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y) / line_h,
                    ScrollDelta::Lines(l) => l.y * 3.0,
                };
                let lines = dy.round() as i32;
                if lines == 0 {
                    return;
                }
                let mut term = this.session.term.lock();
                if term.mode().contains(TermMode::ALT_SCREEN) {
                    // alternate scroll mode: arrows to the app (vim/less/top)
                    drop(term);
                    let seq: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                    for _ in 0..lines.unsigned_abs().min(10) {
                        this.session.write(seq);
                    }
                } else {
                    term.scroll_display(Scroll::Delta(lines));
                    drop(term);
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .child(TerminalElement { session: self.session.clone(), focused })
    }
}

pub struct TerminalElement {
    session: Arc<Session>,
    focused: bool,
}

pub struct TermLayout {
    bg: Hsla,
    bg_rects: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, CursorShape, Option<(Point<Pixels>, ShapedLine)>)>,
    line_h: Pixels,
}

pub const BASE_FONT: f32 = 13.0;
pub const LINE_HEIGHT: f32 = 1.2;
pub const PAD: f32 = 6.0;

pub fn mono_font() -> Font {
    // Nodus' xterm uses 'SF Mono', ui-monospace, Menlo. "SF Mono" isn't a public family name for
    // CoreText outside Terminal.app's bundle; GPUI resolves it via font-kit if present, else Menlo.
    // main.rs loads /System/Library/Fonts/SFNSMono.ttf, which registers the family ".SF NS Mono".
    static SF: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::path::Path::new("/System/Library/Fonts/SFNSMono.ttf").exists());
    let default = if *SF { ".SF NS Mono" } else { "Menlo" };
    font(std::env::var("S1_TERM_FONT").unwrap_or_else(|_| default.into()))
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TermLayout;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) -> TermLayout {
        let scale = zoom::current_scale(window);
        let font_base = px(BASE_FONT);
        let ts = window.text_system().clone();
        let regular = mono_font();
        let font_id = ts.resolve_font(&regular);
        let cell_w_base = ts.advance(font_id, font_base, 'm').map(|s| s.width).unwrap_or(px(7.8));
        let line_h_base = (BASE_FONT * LINE_HEIGHT).round();

        // Grid size is computed in *unscaled* units so zooming the board never resizes the PTY.
        let w = f32::from(bounds.size.width) / scale - 2. * PAD;
        let h = f32::from(bounds.size.height) / scale - 2. * PAD;
        // Layout snaps to device pixels and some decorations are fixed px, so `bounds / scale`
        // wobbles by a pixel or so as the board zooms. Only a real size change (> 1.5 unscaled px)
        // may recompute the grid; otherwise zooming would resize the PTY (SIGWINCH storms).
        let changed = {
            let mut last = self.session.last_dims.lock().unwrap();
            let changed = (w - last.0).abs() > 1.5 || (h - last.1).abs() > 1.5;
            if changed {
                *last = (w, h);
            }
            changed
        };
        if changed {
            let cols = ((w / f32::from(cell_w_base)).floor() as i32).clamp(2, 1000) as u16;
            let rows = ((h / line_h_base).floor() as i32).clamp(1, 500) as u16;
            self.session.resize(cols, rows, f32::from(cell_w_base), line_h_base);
        }

        let font_size = font_base * scale;
        let cell_w = cell_w_base * scale;
        let line_h = px(line_h_base * scale);
        let origin = bounds.origin + point(px(PAD * scale), px(PAD * scale));

        let term = self.session.term.lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let default_bg = rgba_hex(BG, 1.);
        let mut bg_rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
        let mut lines = Vec::new();

        struct Seg {
            col: usize,
            text: String,
            color: Hsla,
            bold: bool,
            italic: bool,
            underline: bool,
            strike: bool,
            wide: bool,
        }
        let mut row_segs: Vec<Vec<Seg>> = (0..term.screen_lines()).map(|_| Vec::new()).collect();
        let mut cursor_cell: Option<(char, Hsla, Hsla, bool)> = None;
        let cursor_point = content.cursor.point;
        let cursor_shape = content.cursor.shape;
        let display_offset = content.display_offset as i32;

        for item in content.display_iter {
            let row = item.point.line.0 + display_offset;
            if row < 0 || row as usize >= row_segs.len() {
                continue;
            }
            let col = item.point.column.0;
            let cell = item.cell;
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let bold = flags.contains(Flags::BOLD);
            let mut fg = resolve(cell.fg, colors, bold, true);
            let mut bg = resolve(cell.bg, colors, false, false);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::DIM) {
                fg.a *= 0.66;
            }
            let wide = flags.contains(Flags::WIDE_CHAR);
            let ncells = if wide { 2 } else { 1 };
            let x = origin.x + cell_w * col as f32;
            let y = origin.y + line_h * row as f32;
            if bg != default_bg {
                let r = Bounds::new(point(x, y), size(cell_w * ncells as f32, line_h));
                match bg_rects.last_mut() {
                    Some((last, c)) if *c == bg && last.origin.y == y && (last.right() - x).abs() < px(0.5) => {
                        last.size.width += r.size.width;
                    }
                    _ => bg_rects.push((r, bg)),
                }
            }
            let mut ch = if flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
            if ch == '\t' {
                ch = ' ';
            }
            if item.point.line == cursor_point.line && item.point.column == cursor_point.column {
                cursor_cell = Some((ch, fg, bg, wide));
            }
            let mut text = String::new();
            text.push(ch);
            if let Some(zw) = cell.zerowidth() {
                text.extend(zw.iter());
            }
            let italic = flags.contains(Flags::ITALIC);
            let underline = flags.intersects(Flags::ALL_UNDERLINES);
            let strike = flags.contains(Flags::STRIKEOUT);
            let segs = &mut row_segs[row as usize];
            match segs.last_mut() {
                Some(s)
                    if !wide
                        && !s.wide
                        && s.color == fg
                        && s.bold == bold
                        && s.italic == italic
                        && s.underline == underline
                        && s.strike == strike
                        && s.col + s.text.chars().filter(|c| !is_zero_width(*c)).count() == col =>
                {
                    s.text.push_str(&text)
                }
                _ => segs.push(Seg { col, text, color: fg, bold, italic, underline, strike, wide }),
            }
        }

        for (row, segs) in row_segs.into_iter().enumerate() {
            for s in segs {
                if s.text.trim().is_empty() && !s.underline && !s.strike {
                    continue;
                }
                let mut f = regular.clone();
                if s.bold {
                    f.weight = FontWeight::BOLD;
                }
                if s.italic {
                    f.style = FontStyle::Italic;
                }
                let run = TextRun {
                    len: s.text.len(),
                    font: f,
                    color: s.color,
                    background_color: None,
                    underline: s.underline.then(|| UnderlineStyle { thickness: px(1.), color: Some(s.color), wavy: false }),
                    strikethrough: s.strike.then(|| StrikethroughStyle { thickness: px(1.), color: Some(s.color) }),
                };
                let width = if s.wide { cell_w * 2. } else { cell_w };
                let shaped = ts.shape_line(s.text.into(), font_size, &[run], Some(width));
                lines.push((point(origin.x + cell_w * s.col as f32, origin.y + line_h * row as f32), shaped));
            }
        }

        let show_cursor = term.mode().contains(TermMode::SHOW_CURSOR) && display_offset == 0;
        let cursor = if show_cursor && cursor_shape != CursorShape::Hidden {
            let row = cursor_point.line.0 + display_offset;
            let x = origin.x + cell_w * cursor_point.column.0 as f32;
            let y = origin.y + line_h * row as f32;
            let (ch, _fg, _bg, wide) = cursor_cell.unwrap_or((' ', rgba_hex(FG, 1.), default_bg, false));
            let w = if wide { cell_w * 2. } else { cell_w };
            let shape = if self.focused { cursor_shape } else { CursorShape::HollowBlock };
            let glyph = (shape == CursorShape::Block && ch != ' ').then(|| {
                let run = TextRun {
                    len: ch.len_utf8(),
                    font: regular.clone(),
                    color: default_bg,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                (point(x, y), ts.shape_line(ch.to_string().into(), font_size, &[run], Some(w)))
            });
            Some((Bounds::new(point(x, y), size(w, line_h)), shape, glyph))
        } else {
            None
        };

        TermLayout { bg: default_bg, bg_rects, lines, cursor, line_h }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        layout: &mut TermLayout,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, layout.bg));
            for (r, c) in &layout.bg_rects {
                window.paint_quad(fill(*r, *c));
            }
            for (origin, line) in &layout.lines {
                let _ = line.paint(*origin, layout.line_h, TextAlign::Left, None, window, cx);
            }
            if let Some((r, shape, glyph)) = &layout.cursor {
                let fgc = rgba_hex(FG, 1.);
                let scale = f32::from(layout.line_h) / (BASE_FONT * LINE_HEIGHT).round();
                match shape {
                    CursorShape::Block => {
                        window.paint_quad(fill(*r, fgc));
                        if let Some((o, l)) = glyph {
                            let _ = l.paint(*o, layout.line_h, TextAlign::Left, None, window, cx);
                        }
                    }
                    CursorShape::Beam => {
                        window.paint_quad(fill(Bounds::new(r.origin, size(px(2. * scale.max(0.5)), r.size.height)), fgc));
                    }
                    CursorShape::Underline => {
                        let h = px(2. * scale.max(0.5));
                        window.paint_quad(fill(Bounds::new(point(r.origin.x, r.bottom() - h), size(r.size.width, h)), fgc));
                    }
                    _ => {
                        window.paint_quad(outline(*r, fgc, BorderStyle::Solid));
                    }
                }
            }
        });
    }
}

fn is_zero_width(c: char) -> bool {
    matches!(c as u32, 0x200B..=0x200F | 0x0300..=0x036F | 0xFE00..=0xFE0F)
}
