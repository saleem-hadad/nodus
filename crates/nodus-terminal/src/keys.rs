//! Keys to bytes, exactly as 0.9.4 sends them: registry.ts's custom key
//! handler first (⇧Enter for Claude, ⌘K/⌘C/⌘A on a Mac, Windows Terminal's
//! Ctrl rules elsewhere), then xterm.js 6's `_keyDown` with
//! `evaluateKeyboardEvent` (`common/input/Keyboard.ts`), `macOptionIsMeta: false`.
//!
//! Keys are described the way a browser does (`KeyboardEvent.key`, `.code`,
//! `.keyCode`), because that is what xterm's rules are written against. The
//! UI turns its key events into [`KeyEvent`]s; [`KeyEvent::from_parts`] derives
//! the key code as Chromium does.

use nodus_domain::{MenuCommand, SessionKind};

use crate::terminal::Terminal;

/// The OS the app runs on, for xterm's `isMac`/`isWindows` and registry.ts's `isMac`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    /// The platform this build runs on.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub const fn is_mac(self) -> bool {
        matches!(self, Self::Mac)
    }

    pub const fn is_windows(self) -> bool {
        matches!(self, Self::Windows)
    }
}

/// Modifier keys held, as `KeyboardEvent` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    /// ⌥ on a Mac.
    pub alt: bool,
    pub ctrl: bool,
    /// ⌘ on a Mac, the Windows key elsewhere.
    pub meta: bool,
    /// `getModifierState('AltGraph')` (AltGr on Windows; Chromium also sets ctrl and alt).
    pub alt_graph: bool,
}

/// A key press, described as a browser's `keydown` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    /// `KeyboardEvent.key`: `"a"`, `"A"`, `"é"`, `"Enter"`, `"ArrowLeft"`,
    /// `"Dead"`, `"Process"`…
    pub key: String,
    /// `KeyboardEvent.code`, the physical key: `"KeyA"`, `"Digit1"`, `"Enter"`… (may be empty).
    pub code: String,
    /// `KeyboardEvent.keyCode`.
    pub key_code: u32,
    pub modifiers: Modifiers,
    /// Auto-repeat. xterm treats repeats as presses.
    pub repeat: bool,
    /// An IME composition is in progress (`isComposing`, key code 229).
    pub composing: bool,
}

impl KeyEvent {
    /// A key press with the key code Chromium would give it.
    pub fn from_parts(key: &str, code: &str, modifiers: Modifiers) -> Self {
        Self {
            key: key.to_owned(),
            code: code.to_owned(),
            key_code: key_code(key, code),
            modifiers,
            repeat: false,
            composing: false,
        }
    }

    /// The same press reporting `key_code` (when the platform gives one).
    pub fn with_key_code(mut self, key_code: u32) -> Self {
        self.key_code = key_code;
        self
    }

    fn key_char_len(&self) -> usize {
        // `ev.key.length` counts UTF-16 units.
        self.key.encode_utf16().count()
    }
}

/// The key code Chromium gives a `keydown` (`KeyboardEvent.keyCode`): named
/// keys by name, Latin letters by the letter typed (so AZERTY's A is 65),
/// anything else by the physical key (non-Latin layouts, ⌥ and AltGr
/// characters, digits, punctuation), and a US symbol by its key when the
/// physical key isn't known; 0 when nothing fits.
pub fn key_code(key: &str, code: &str) -> u32 {
    if let Some(n) = named_key_code(key) {
        return n;
    }
    if let Some(n) = code.strip_prefix("Numpad").and_then(numpad_key_code) {
        return n;
    }
    if key == "Process" {
        return 229;
    }
    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    if let Some(c) = single.filter(char::is_ascii_alphabetic) {
        return c.to_ascii_uppercase() as u32;
    }
    match physical_key_code(code) {
        0 => single.and_then(us_symbol_key_code).unwrap_or(0),
        n => n,
    }
}

fn named_key_code(key: &str) -> Option<u32> {
    Some(match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" | "AltGraph" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "Meta" => 91,
        "ContextMenu" => 93,
        "NumLock" => 144,
        "ScrollLock" => 145,
        _ => {
            let n: u32 = key.strip_prefix('F')?.parse().ok()?;
            if (1..=24).contains(&n) {
                111 + n
            } else {
                return None;
            }
        }
    })
}

fn numpad_key_code(rest: &str) -> Option<u32> {
    Some(match rest {
        "Multiply" => 106,
        "Add" => 107,
        "Subtract" => 109,
        "Decimal" => 110,
        "Divide" => 111,
        "Enter" => 13,
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => 96 + u32::from(d.as_bytes()[0] - b'0'),
        _ => return None,
    })
}

fn us_symbol_key_code(c: char) -> Option<u32> {
    Some(match c {
        '0'..='9' => 48 + (c as u32 - '0' as u32),
        ')' => 48,
        '!' => 49,
        '@' => 50,
        '#' => 51,
        '$' => 52,
        '%' => 53,
        '^' => 54,
        '&' => 55,
        '*' => 56,
        '(' => 57,
        ';' | ':' => 186,
        '=' | '+' => 187,
        ',' | '<' => 188,
        '-' | '_' => 189,
        '.' | '>' => 190,
        '/' | '?' => 191,
        '`' | '~' => 192,
        '[' | '{' => 219,
        '\\' | '|' => 220,
        ']' | '}' => 221,
        '\'' | '"' => 222,
        ' ' => 32,
        _ => return None,
    })
}

fn physical_key_code(code: &str) -> u32 {
    if let Some(l) = code
        .strip_prefix("Key")
        .filter(|l| l.len() == 1 && l.as_bytes()[0].is_ascii_uppercase())
    {
        return u32::from(l.as_bytes()[0]);
    }
    if let Some(d) = code
        .strip_prefix("Digit")
        .filter(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit())
    {
        return 48 + u32::from(d.as_bytes()[0] - b'0');
    }
    match code {
        "Space" => 32,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        "IntlBackslash" => 226,
        _ => 0,
    }
}

/// `KeyboardResultType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardResultType {
    SendKey,
    PageUp,
    PageDown,
    SelectAll,
}

/// What `evaluateKeyboardEvent` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardResult {
    pub kind: KeyboardResultType,
    pub cancel: bool,
    pub key: Option<String>,
}

const ESC: &str = "\x1b";

/// `KEYCODE_KEY_MAPPINGS`: digits and US punctuation, unshifted and shifted.
fn keycode_key_mapping(key_code: u32) -> Option<[&'static str; 2]> {
    Some(match key_code {
        48 => ["0", ")"],
        49 => ["1", "!"],
        50 => ["2", "@"],
        51 => ["3", "#"],
        52 => ["4", "$"],
        53 => ["5", "%"],
        54 => ["6", "^"],
        55 => ["7", "&"],
        56 => ["8", "*"],
        57 => ["9", "("],
        186 => [";", ":"],
        187 => ["=", "+"],
        188 => [",", "<"],
        189 => ["-", "_"],
        190 => [".", ">"],
        191 => ["/", "?"],
        192 => ["`", "~"],
        219 => ["[", "{"],
        220 => ["\\", "|"],
        221 => ["]", "}"],
        222 => ["'", "\""],
        _ => return None,
    })
}

/// A port of xterm.js 6's `evaluateKeyboardEvent` (TERM-050), line for line.
pub fn evaluate_keyboard_event(
    ev: &KeyEvent,
    application_cursor: bool,
    is_mac: bool,
    mac_option_is_meta: bool,
) -> KeyboardResult {
    let m = ev.modifiers;
    let mut result = KeyboardResult {
        kind: KeyboardResultType::SendKey,
        cancel: false,
        key: None,
    };
    let modifiers = u32::from(m.shift) | (u32::from(m.alt) << 1) | (u32::from(m.ctrl) << 2) | (u32::from(m.meta) << 3);
    let set = |r: &mut KeyboardResult, s: String| r.key = Some(s);
    // `CSI 1 ; mod X`, or `ESC O X` in application cursor mode, or `CSI X`.
    let cursor = |r: &mut KeyboardResult, x: char| {
        r.key = Some(if modifiers != 0 {
            format!("{ESC}[1;{}{x}", modifiers + 1)
        } else if application_cursor {
            format!("{ESC}O{x}")
        } else {
            format!("{ESC}[{x}")
        });
    };
    let fkey_ss3 = |r: &mut KeyboardResult, x: char| {
        r.key = Some(if modifiers != 0 {
            format!("{ESC}[1;{}{x}", modifiers + 1)
        } else {
            format!("{ESC}O{x}")
        });
    };
    let tilde = |r: &mut KeyboardResult, n: u32| {
        r.key = Some(if modifiers != 0 {
            format!("{ESC}[{n};{}~", modifiers + 1)
        } else {
            format!("{ESC}[{n}~")
        });
    };
    match ev.key_code {
        0 => {
            let x = match ev.key.as_str() {
                "UIKeyInputUpArrow" => Some('A'),
                "UIKeyInputLeftArrow" => Some('D'),
                "UIKeyInputRightArrow" => Some('C'),
                "UIKeyInputDownArrow" => Some('B'),
                _ => None,
            };
            if let Some(x) = x {
                set(
                    &mut result,
                    if application_cursor {
                        format!("{ESC}O{x}")
                    } else {
                        format!("{ESC}[{x}")
                    },
                );
            }
        }
        8 => {
            let mut key = if m.ctrl { "\x08".to_owned() } else { "\x7f".to_owned() };
            if m.alt {
                key = format!("{ESC}{key}");
            }
            set(&mut result, key);
        }
        9 => {
            if m.shift {
                set(&mut result, format!("{ESC}[Z"));
            } else {
                set(&mut result, "\t".to_owned());
                result.cancel = true;
            }
        }
        13 => {
            set(&mut result, if m.alt { format!("{ESC}\r") } else { "\r".to_owned() });
            result.cancel = true;
        }
        27 => {
            set(&mut result, if m.alt { format!("{ESC}{ESC}") } else { ESC.to_owned() });
            result.cancel = true;
        }
        37..=40 => {
            if !m.meta {
                let x = match ev.key_code {
                    37 => 'D',
                    39 => 'C',
                    38 => 'A',
                    _ => 'B',
                };
                cursor(&mut result, x);
            }
        }
        45 => {
            if !m.shift && !m.ctrl {
                set(&mut result, format!("{ESC}[2~"));
            }
        }
        46 => tilde(&mut result, 3),
        36 => cursor(&mut result, 'H'),
        35 => cursor(&mut result, 'F'),
        33 | 34 => {
            let n = if ev.key_code == 33 { 5 } else { 6 };
            if m.shift {
                result.kind = if ev.key_code == 33 {
                    KeyboardResultType::PageUp
                } else {
                    KeyboardResultType::PageDown
                };
            } else if m.ctrl {
                set(&mut result, format!("{ESC}[{n};{}~", modifiers + 1));
            } else {
                set(&mut result, format!("{ESC}[{n}~"));
            }
        }
        112 => fkey_ss3(&mut result, 'P'),
        113 => fkey_ss3(&mut result, 'Q'),
        114 => fkey_ss3(&mut result, 'R'),
        115 => fkey_ss3(&mut result, 'S'),
        116 => tilde(&mut result, 15),
        117 => tilde(&mut result, 17),
        118 => tilde(&mut result, 18),
        119 => tilde(&mut result, 19),
        120 => tilde(&mut result, 20),
        121 => tilde(&mut result, 21),
        122 => tilde(&mut result, 23),
        123 => tilde(&mut result, 24),
        kc => {
            if m.ctrl && !m.shift && !m.alt && !m.meta {
                // a-z, space, and the control characters on digits and brackets.
                let c = match kc {
                    65..=90 => Some(kc - 64),
                    32 => Some(0),
                    51..=55 => Some(kc - 51 + 27),
                    56 => Some(0x7f),
                    219 => Some(0x1b),
                    220 => Some(0x1c),
                    221 => Some(0x1d),
                    _ => None,
                };
                if let Some(c) = c {
                    set(&mut result, char::from_u32(c).unwrap_or_default().to_string());
                }
            } else if (!is_mac || mac_option_is_meta) && m.alt && !m.meta {
                // On a Mac without macOptionIsMeta, ⌥ is a third-level shift.
                let mapped = keycode_key_mapping(kc).map(|k| k[usize::from(m.shift)]);
                if let Some(k) = mapped {
                    set(&mut result, format!("{ESC}{k}"));
                } else if (65..=90).contains(&kc) {
                    let code = if m.ctrl { kc - 64 } else { kc + 32 };
                    let mut s = char::from_u32(code).unwrap_or_default().to_string();
                    if m.shift {
                        s = s.to_uppercase();
                    }
                    set(&mut result, format!("{ESC}{s}"));
                } else if kc == 32 {
                    set(&mut result, format!("{ESC}{}", if m.ctrl { "\0" } else { " " }));
                } else if ev.key == "Dead" && ev.code.starts_with("Key") {
                    // Alt makes a dead key of some US letters (N, E, U…).
                    let mut s = ev.code.get(3..4).unwrap_or_default().to_owned();
                    if !m.shift {
                        s = s.to_lowercase();
                    }
                    set(&mut result, format!("{ESC}{s}"));
                    result.cancel = true;
                }
            } else if is_mac && !m.alt && !m.ctrl && !m.shift && m.meta {
                if kc == 65 {
                    result.kind = KeyboardResultType::SelectAll;
                }
            } else if !ev.key.is_empty() && !m.ctrl && !m.alt && !m.meta && kc >= 48 && ev.key_char_len() == 1 {
                // Keys that make a single character.
                set(&mut result, ev.key.clone());
            } else if !ev.key.is_empty() && m.ctrl {
                if ev.key == "_" {
                    set(&mut result, "\x1f".to_owned());
                }
                if ev.key == "@" {
                    set(&mut result, "\0".to_owned());
                }
            }
        }
    }
    result
}

/// What the terminal did with a key press, and what the UI does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Send this to the session as typed input (registry.ts's `onData`:
    /// dropped once the process has exited). The view is already back at the
    /// bottom and the selection gone (TERM-054). Stop the event.
    Send(String),
    /// ⇧Enter in a Claude terminal (TERM-036): write this straight to the
    /// session, even if the process has exited. Stop the event.
    WriteDirect(String),
    /// Put this text on the clipboard (TERM-040, TERM-043). Stop the event.
    Copy(String),
    /// Paste the clipboard's plain text with [`Terminal::paste`] (TERM-044). Stop the event.
    Paste,
    /// Run this menu command (TERM-045). Stop the event.
    Menu(MenuCommand),
    /// Done here (⌘K, select all, ⇧PgUp…); nothing to send. Stop the event.
    Handled,
    /// Not the terminal's: let the app's menu and shortcuts have it, and type
    /// nothing (TERM-038, TERM-042, TERM-045).
    Propagate,
    /// xterm let it through: whatever character the platform's text input
    /// makes of it (⌥ compositions, dead keys, AltGr, ⇧ letters) arrives
    /// through [`Terminal::input_text`].
    Unhandled,
    /// An IME composition is in progress: the input method has the key (TERM-055).
    Composing,
}

/// registry.ts's `shortcutKey`: a shortcut's letter in lower case on any
/// layout, read from the key code when `key` isn't a Latin letter (TERM-046).
pub fn shortcut_key(ev: &KeyEvent) -> String {
    let latin = {
        let mut cs = ev.key.chars();
        matches!((cs.next(), cs.next()), (Some(c), None) if c.is_ascii_alphabetic())
    };
    if latin || ev.key_code < 65 || ev.key_code > 90 {
        return ev.key.to_lowercase();
    }
    char::from_u32(ev.key_code)
        .unwrap_or_default()
        .to_lowercase()
        .to_string()
}

fn is_lower_latin_letter(s: &str) -> bool {
    let mut cs = s.chars();
    matches!((cs.next(), cs.next()), (Some(c), None) if c.is_ascii_lowercase())
}

impl Terminal {
    /// A key went down while the terminal has focus. Applies what the key does
    /// to the terminal itself and says what the UI does with it.
    pub fn key_down(&mut self, ev: &KeyEvent) -> KeyAction {
        if let Some(action) = self.custom_key_handler(ev) {
            return action;
        }
        self.xterm_key_down(ev)
    }

    /// registry.ts's `attachCustomKeyEventHandler` callback for a keydown:
    /// `Some` where it returns false.
    fn custom_key_handler(&mut self, ev: &KeyEvent) -> Option<KeyAction> {
        let m = ev.modifiers;
        // ⇧Enter is a newline in Claude's prompt (TERM-036); a shell gets Enter (TERM-037).
        if self.kind == SessionKind::Claude && ev.key == "Enter" && m.shift && !m.meta && !m.ctrl && !m.alt {
            return Some(KeyAction::WriteDirect("\x1b\r".to_owned()));
        }
        let mac = self.platform.is_mac();
        if mac && m.meta && !m.ctrl && !m.alt {
            // ⌘ keys never reach the program (TERM-038).
            let key = ev.key.to_lowercase();
            return Some(match key.as_str() {
                "k" => {
                    self.clear();
                    KeyAction::Handled
                }
                "c" if self.has_selection() => KeyAction::Copy(self.selection_text()),
                "a" => {
                    self.select_all();
                    KeyAction::Handled
                }
                // ⌘V, ⌘W, ⌘N, ⌘C with nothing selected…: the app menu (TERM-042).
                _ => KeyAction::Propagate,
            });
        }
        // Off a Mac, Ctrl is the terminal's own key, as in Windows Terminal (TERM-043…047).
        if !mac && m.ctrl && !m.alt && !m.meta {
            let key = shortcut_key(ev);
            if key == "c" && (m.shift || self.has_selection()) {
                let text = self.has_selection().then(|| self.selection_text());
                // So the next Ctrl+C stops what's running.
                self.clear_selection();
                return Some(match text {
                    Some(t) => KeyAction::Copy(t),
                    None => KeyAction::Handled,
                });
            }
            if key == "v" {
                return Some(KeyAction::Paste);
            }
            if m.shift && is_lower_latin_letter(&key) {
                return Some(match key.as_str() {
                    "a" => {
                        self.select_all();
                        KeyAction::Handled
                    }
                    "w" => KeyAction::Menu(MenuCommand::Close),
                    "t" => KeyAction::Menu(MenuCommand::NewTerminal),
                    // Ctrl+Shift+N, Ctrl+Shift+F… go on to the app.
                    _ => KeyAction::Propagate,
                });
            }
        }
        None
    }

    /// `_isThirdLevelShift` for a keydown.
    fn is_third_level_shift(&self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        let third = (self.platform.is_mac() && m.alt && !m.ctrl && !m.meta)
            || (self.platform.is_windows() && m.alt && m.ctrl && !m.meta)
            || (self.platform.is_windows() && m.alt_graph);
        third && (ev.key_code == 0 || ev.key_code > 47)
    }

    /// xterm's `_keyDown` after the custom handler let the key through.
    fn xterm_key_down(&mut self, ev: &KeyEvent) -> KeyAction {
        // CompositionHelper: key code 229 belongs to the IME.
        if ev.composing || ev.key_code == 229 {
            self.scroll_to_bottom();
            return KeyAction::Composing;
        }
        if ev.key == "Dead" || ev.key == "AltGraph" {
            self.dead_key = true;
        }
        let result = evaluate_keyboard_event(ev, self.modes().application_cursor, self.platform.is_mac(), false);
        match result.kind {
            KeyboardResultType::PageUp | KeyboardResultType::PageDown => {
                // ⇧PgUp/⇧PgDn scroll a page less a line and send nothing (TERM-053).
                let page = self.rows() as i32 - 1;
                self.scroll_lines(if result.kind == KeyboardResultType::PageUp {
                    -page
                } else {
                    page
                });
                return KeyAction::Handled;
            }
            KeyboardResultType::SelectAll => self.select_all(),
            KeyboardResultType::SendKey => {}
        }
        if self.is_third_level_shift(ev) {
            return KeyAction::Unhandled;
        }
        let Some(key) = result.key else {
            return if result.cancel {
                KeyAction::Handled
            } else {
                KeyAction::Unhandled
            };
        };
        let m = ev.modifiers;
        // A-Z come through keypress (a macOS IME fix): the same character.
        if !m.ctrl && !m.alt && !m.meta && ev.key_char_len() == 1 {
            let c = ev.key.chars().next().unwrap_or_default();
            if c.is_ascii_uppercase() {
                return KeyAction::Unhandled;
            }
        }
        // The key after a dead key comes through the text input, unless xterm
        // already cancelled it (Enter, Tab, Esc…), which then sends nothing.
        if self.dead_key {
            self.dead_key = false;
            return if result.cancel {
                KeyAction::Handled
            } else {
                KeyAction::Unhandled
            };
        }
        self.xterm.cursor_initialized = true;
        self.before_user_input();
        KeyAction::Send(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(key: &str, code: &str, m: Modifiers) -> KeyEvent {
        KeyEvent::from_parts(key, code, m)
    }

    const NONE: Modifiers = Modifiers {
        shift: false,
        alt: false,
        ctrl: false,
        meta: false,
        alt_graph: false,
    };
    const SHIFT: Modifiers = Modifiers { shift: true, ..NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const META: Modifiers = Modifiers { meta: true, ..NONE };

    #[test]
    fn key_codes_as_chromium_gives_them() {
        assert_eq!(key_code("a", "KeyA"), 65);
        assert_eq!(key_code("A", "KeyA"), 65);
        assert_eq!(key_code("ф", "KeyA"), 65);
        assert_eq!(key_code("å", "KeyA"), 65);
        assert_eq!(key_code("!", "Digit1"), 49);
        assert_eq!(key_code("&", "Digit1"), 49);
        assert_eq!(key_code("_", "Minus"), 189);
        assert_eq!(key_code("Enter", "NumpadEnter"), 13);
        assert_eq!(key_code("7", "Numpad7"), 103);
        assert_eq!(key_code("F12", "F12"), 123);
        assert_eq!(key_code("Dead", "KeyE"), 69);
        assert_eq!(key_code("Process", "KeyA"), 229);
        assert_eq!(key_code(" ", "Space"), 32);
    }

    #[test]
    fn term_050_evaluate_matches_the_key_table() {
        let e = |key: &str, code: &str, m: Modifiers, app: bool| {
            evaluate_keyboard_event(&ev(key, code, m), app, false, false).key
        };
        assert_eq!(e("Backspace", "Backspace", NONE, false).as_deref(), Some("\x7f"));
        assert_eq!(e("Backspace", "Backspace", CTRL, false).as_deref(), Some("\x08"));
        assert_eq!(e("Backspace", "Backspace", ALT, false).as_deref(), Some("\x1b\x7f"));
        assert_eq!(e("Tab", "Tab", SHIFT, false).as_deref(), Some("\x1b[Z"));
        assert_eq!(e("ArrowUp", "ArrowUp", NONE, true).as_deref(), Some("\x1bOA"));
        assert_eq!(e("ArrowLeft", "ArrowLeft", SHIFT, true).as_deref(), Some("\x1b[1;2D"));
        assert_eq!(e("Home", "Home", NONE, true).as_deref(), Some("\x1bOH"));
        assert_eq!(e("Delete", "Delete", CTRL, false).as_deref(), Some("\x1b[3;5~"));
        assert_eq!(e("F5", "F5", NONE, false).as_deref(), Some("\x1b[15~"));
        assert_eq!(e("F1", "F1", ALT, false).as_deref(), Some("\x1b[1;3P"));
        assert_eq!(e("c", "KeyC", CTRL, false).as_deref(), Some("\x03"));
        assert_eq!(e("3", "Digit3", CTRL, false).as_deref(), Some("\x1b"));
        assert_eq!(e("a", "KeyA", ALT, false).as_deref(), Some("\x1ba"));
        assert_eq!(
            e(
                "A",
                "KeyA",
                Modifiers {
                    alt: true,
                    shift: true,
                    ..NONE
                },
                false
            )
            .as_deref(),
            Some("\x1bA")
        );
        assert_eq!(e("1", "Digit1", ALT, false).as_deref(), Some("\x1b1"));
        assert_eq!(e("ArrowLeft", "ArrowLeft", META, false), None);
    }
}
