//! GPUI keystrokes as the browser `keydown` events xterm.js's rules are
//! written against (`KeyboardEvent.key`, `.code`, modifiers), so
//! `nodus-terminal`'s encoder sees what Chromium gave 0.9.4 (TERM-050).
//!
//! GPUI names a key by the character on it in a US-like layout (`key`) and
//! the character it types (`key_char`); Chromium's `key` is the typed
//! character for printable keys, a name for the others, and `code` is the
//! physical key, which GPUI's `key` stands in for.

use gpui::Keystroke;
use nodus_terminal::{KeyEvent, Modifiers};

/// Named keys: GPUI's name → `KeyboardEvent.key` and `.code`.
fn named(key: &str) -> Option<(&'static str, &'static str)> {
    Some(match key {
        "enter" => ("Enter", "Enter"),
        "backspace" => ("Backspace", "Backspace"),
        "tab" => ("Tab", "Tab"),
        "escape" => ("Escape", "Escape"),
        "space" => (" ", "Space"),
        "up" => ("ArrowUp", "ArrowUp"),
        "down" => ("ArrowDown", "ArrowDown"),
        "left" => ("ArrowLeft", "ArrowLeft"),
        "right" => ("ArrowRight", "ArrowRight"),
        "pageup" => ("PageUp", "PageUp"),
        "pagedown" => ("PageDown", "PageDown"),
        "home" => ("Home", "Home"),
        "end" => ("End", "End"),
        "delete" => ("Delete", "Delete"),
        "insert" => ("Insert", "Insert"),
        "f1" => ("F1", "F1"),
        "f2" => ("F2", "F2"),
        "f3" => ("F3", "F3"),
        "f4" => ("F4", "F4"),
        "f5" => ("F5", "F5"),
        "f6" => ("F6", "F6"),
        "f7" => ("F7", "F7"),
        "f8" => ("F8", "F8"),
        "f9" => ("F9", "F9"),
        "f10" => ("F10", "F10"),
        "f11" => ("F11", "F11"),
        "f12" => ("F12", "F12"),
        "shift" => ("Shift", "ShiftLeft"),
        "control" => ("Control", "ControlLeft"),
        "alt" => ("Alt", "AltLeft"),
        "platform" => ("Meta", "MetaLeft"),
        _ => return None,
    })
}

/// The US key a character is on, and whether it needs Shift there.
fn us_key(c: char) -> Option<(&'static str, bool)> {
    Some(match c {
        '0' => ("Digit0", false),
        '1' => ("Digit1", false),
        '2' => ("Digit2", false),
        '3' => ("Digit3", false),
        '4' => ("Digit4", false),
        '5' => ("Digit5", false),
        '6' => ("Digit6", false),
        '7' => ("Digit7", false),
        '8' => ("Digit8", false),
        '9' => ("Digit9", false),
        ')' => ("Digit0", true),
        '!' => ("Digit1", true),
        '@' => ("Digit2", true),
        '#' => ("Digit3", true),
        '$' => ("Digit4", true),
        '%' => ("Digit5", true),
        '^' => ("Digit6", true),
        '&' => ("Digit7", true),
        '*' => ("Digit8", true),
        '(' => ("Digit9", true),
        '-' => ("Minus", false),
        '_' => ("Minus", true),
        '=' => ("Equal", false),
        '+' => ("Equal", true),
        '[' => ("BracketLeft", false),
        '{' => ("BracketLeft", true),
        ']' => ("BracketRight", false),
        '}' => ("BracketRight", true),
        '\\' => ("Backslash", false),
        '|' => ("Backslash", true),
        ';' => ("Semicolon", false),
        ':' => ("Semicolon", true),
        '\'' => ("Quote", false),
        '"' => ("Quote", true),
        ',' => ("Comma", false),
        '<' => ("Comma", true),
        '.' => ("Period", false),
        '>' => ("Period", true),
        '/' => ("Slash", false),
        '?' => ("Slash", true),
        '`' => ("Backquote", false),
        '~' => ("Backquote", true),
        _ => return None,
    })
}

/// `KeyboardEvent.code` for GPUI's key name.
fn code_for(key: &str) -> (String, bool) {
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return (String::new(), false);
    };
    if c.is_ascii_alphabetic() {
        return (format!("Key{}", c.to_ascii_uppercase()), c.is_ascii_uppercase());
    }
    match us_key(c) {
        Some((code, shifted)) => (code.to_owned(), shifted),
        None => (String::new(), false),
    }
}

/// The browser's `keydown` for a GPUI keystroke. `alt_graph`: Windows'
/// AltGr (GPUI's `prefer_character_input`); `repeat`: auto-repeat.
pub fn key_event(ks: &Keystroke, alt_graph: bool, repeat: bool) -> KeyEvent {
    let m = &ks.modifiers;
    let (key, code, shifted) = match named(&ks.key) {
        Some((key, code)) => (key.to_owned(), code.to_owned(), false),
        None => {
            let (code, shifted) = code_for(&ks.key);
            // The character typed; with Ctrl or ⌘ there's none, and the key
            // is the letter on it, upper case with Shift.
            let key = match (&ks.key_char, m.control || m.platform) {
                (Some(typed), false) if !typed.is_empty() => typed.clone(),
                _ if m.shift && ks.key.chars().all(|c| c.is_ascii_lowercase()) => ks.key.to_ascii_uppercase(),
                _ => ks.key.clone(),
            };
            (key, code, shifted)
        }
    };
    let modifiers = Modifiers {
        // GPUI drops Shift when it's what turned 1 into !; the browser keeps it.
        shift: m.shift || shifted,
        alt: m.alt,
        ctrl: m.control,
        meta: m.platform,
        alt_graph,
    };
    let mut event = KeyEvent::from_parts(&key, &code, modifiers);
    event.repeat = repeat;
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(spec: &str) -> Keystroke {
        Keystroke::parse(spec).unwrap()
    }

    #[test]
    fn term_050_named_keys_and_letters() {
        let e = key_event(&ks("enter"), false, false);
        assert_eq!((e.key.as_str(), e.code.as_str(), e.key_code), ("Enter", "Enter", 13));
        let e = key_event(&ks("shift-left"), false, false);
        assert_eq!((e.key.as_str(), e.modifiers.shift), ("ArrowLeft", true));
        let mut a = ks("a");
        a.key_char = Some("a".into());
        let e = key_event(&a, false, false);
        assert_eq!((e.key.as_str(), e.code.as_str(), e.key_code), ("a", "KeyA", 65));
        let mut shift_a = ks("shift-a");
        shift_a.key_char = Some("A".into());
        assert_eq!(key_event(&shift_a, false, false).key, "A");
        // Ctrl and ⌘ type nothing: the key is the letter.
        let e = key_event(&ks("ctrl-c"), false, false);
        assert_eq!((e.key.as_str(), e.modifiers.ctrl), ("c", true));
        let e = key_event(&ks("ctrl-shift-c"), false, false);
        assert_eq!(e.key, "C");
        let e = key_event(&ks("cmd-k"), false, false);
        assert_eq!((e.key.as_str(), e.modifiers.meta), ("k", true));
    }

    #[test]
    fn term_050_shifted_symbols_keep_shift() {
        let mut bang = ks("!");
        bang.key_char = Some("!".into());
        let e = key_event(&bang, false, false);
        assert_eq!(
            (e.key.as_str(), e.code.as_str(), e.modifiers.shift),
            ("!", "Digit1", true)
        );
        let e = key_event(&ks("space"), false, true);
        assert_eq!((e.key.as_str(), e.repeat), (" ", true));
    }

    #[test]
    fn term_051_option_compositions_are_the_typed_character() {
        let mut alt_e = ks("alt-e");
        alt_e.key_char = Some("é".into());
        let e = key_event(&alt_e, false, false);
        assert_eq!((e.key.as_str(), e.code.as_str(), e.modifiers.alt), ("é", "KeyE", true));
    }
}
