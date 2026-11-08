//! Shortcuts take ⌘ on a Mac and Ctrl elsewhere, and are written the way each
//! platform writes them. A port of `src/renderer/src/platform.ts` (UI-301…UI-305,
//! PLAT-130…PLAT-133).

use gpui::Modifiers;

/// Which platform's look the window has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    /// The platform this runs on (UI-301).
    pub const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Linux
    };

    /// `platform` from `navigator.platform`: `Mac…` is a Mac, `Win…` Windows,
    /// anything else Linux.
    pub fn from_navigator(platform: &str) -> Self {
        if platform.starts_with("Mac") {
            Self::Mac
        } else if platform.starts_with("Win") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub fn is_mac(self) -> bool {
        self == Self::Mac
    }

    /// Whether the shortcut key is held: ⌘ on a Mac, Ctrl elsewhere (UI-302).
    pub fn mod_key(self, m: &Modifiers) -> bool {
        if self.is_mac() { m.platform } else { m.control }
    }

    /// Whether the other of ⌘ and Ctrl is held, which a shortcut mustn't have.
    pub fn other_mod_key(self, m: &Modifiers) -> bool {
        if self.is_mac() { m.control } else { m.platform }
    }

    /// A shortcut as a Mac writes it (⇧⌘F, ⌘↩, ⌫), as this platform writes
    /// it: Ctrl+Shift+F, Ctrl+Enter, Del (UI-303).
    pub fn keys(self, mac: &str) -> String {
        if self.is_mac() {
            return mac.to_owned();
        }
        const MODIFIERS: [&str; 3] = ["Ctrl", "Alt", "Shift"];
        let parts: Vec<String> = mac
            .chars()
            .map(|c| match c {
                '⌘' | '⌃' => "Ctrl".to_owned(),
                '⌥' => "Alt".to_owned(),
                '⇧' => "Shift".to_owned(),
                '↩' => "Enter".to_owned(),
                '⌫' => "Del".to_owned(),
                other => other.to_string(),
            })
            .collect();
        let key: String = parts
            .iter()
            .filter(|p| !MODIFIERS.contains(&p.as_str()))
            .map(String::as_str)
            .collect();
        let mut out: Vec<&str> = MODIFIERS
            .iter()
            .copied()
            .filter(|m| parts.iter().any(|p| p == m))
            .collect();
        if !key.is_empty() {
            out.push(&key);
        }
        out.join("+")
    }
}

/// [`Platform::keys`] for this platform.
pub fn keys(mac: &str) -> String {
    Platform::CURRENT.keys(mac)
}

/// The key of a shortcut such as Ctrl+C, in lower case, whatever the keyboard
/// layout: with Arabic, Russian or Greek, `key` is that layout's letter, so a
/// letter key is read from its key code (UI-304, UI-305).
///
/// GPUI's keystrokes already carry the ASCII letter of a non-Latin layout's
/// key, so the app reads `Keystroke::key`; this is the rule they follow.
pub fn shortcut_key(key: &str, key_code: u32) -> String {
    let is_ascii_letter = key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic();
    if is_ascii_letter || !(65..=90).contains(&key_code) {
        return js_lower(key);
    }
    char::from_u32(key_code).map_or_else(String::new, |c| c.to_ascii_lowercase().to_string())
}

/// `String.prototype.toLowerCase`.
fn js_lower(s: &str) -> String {
    s.to_lowercase()
}

#[cfg(test)]
mod tests {
    //! platform.test.ts
    use super::*;

    fn mods(meta: bool, ctrl: bool) -> Modifiers {
        Modifiers {
            platform: meta,
            control: ctrl,
            ..Modifiers::default()
        }
    }

    #[test]
    fn keys_writes_a_mac_shortcut_as_windows_does() {
        let p = Platform::from_navigator("Win32");
        assert_eq!(p, Platform::Windows);
        assert_eq!(p.keys("⇧⌘F"), "Ctrl+Shift+F");
        assert_eq!(p.keys("⌘↩"), "Ctrl+Enter");
        assert_eq!(p.keys("⌘"), "Ctrl");
        assert_eq!(p.keys("⌫"), "Del");
        assert_eq!(p.keys("↩"), "Enter");
        assert_eq!(p.keys("⌥⌘K"), "Ctrl+Alt+K");
    }

    #[test]
    fn keys_leaves_it_be_on_a_mac() {
        let p = Platform::from_navigator("MacIntel");
        assert_eq!(p, Platform::Mac);
        assert_eq!(p.keys("⇧⌘F"), "⇧⌘F");
    }

    #[test]
    fn mod_key_is_cmd_on_a_mac_and_ctrl_elsewhere() {
        assert!(Platform::Mac.mod_key(&mods(true, false)));
        assert!(!Platform::Windows.mod_key(&mods(true, false)));
        assert!(Platform::Windows.mod_key(&mods(false, true)));
        assert!(Platform::Windows.other_mod_key(&mods(true, false)));
    }

    #[test]
    fn shortcut_key_reads_a_latin_layouts_letter_as_it_is() {
        assert_eq!(shortcut_key("c", 67), "c");
        assert_eq!(shortcut_key("W", 87), "w");
        assert_eq!(shortcut_key("a", 65), "a");
    }

    #[test]
    fn shortcut_key_reads_the_letter_from_the_key_code_on_another_alphabets_layout() {
        assert_eq!(shortcut_key("ر", 86), "v");
        assert_eq!(shortcut_key("с", 67), "c");
    }

    #[test]
    fn shortcut_key_leaves_keys_that_arent_letters_be() {
        assert_eq!(shortcut_key("Enter", 13), "enter");
        assert_eq!(shortcut_key("é", 50), "é");
    }
}
