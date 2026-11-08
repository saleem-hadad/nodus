//! Character widths as xterm.js 6 measures them with `@xterm/addon-unicode11`
//! active (TERM-081): Unicode 11's wide and combining tables, everything else
//! one cell. alacritty_terminal measures with the current Unicode
//! (`unicode-width`), so [`crate::handler`] writes each character at the width
//! this module gives, whatever alacritty would have used.

use crate::unicode11_tables::{BMP_COMBINING, BMP_WIDE, HIGH_COMBINING, HIGH_WIDE};

fn in_table(cp: u32, table: &[(u32, u32)]) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `UnicodeV11.wcwidth`: 0, 1 or 2 cells.
pub fn wcwidth(c: char) -> usize {
    let cp = c as u32;
    if cp < 32 {
        return 0;
    }
    if cp < 127 {
        return 1;
    }
    if cp < 0x10000 {
        // The table starts at 1, controls are 0, combining marks 0, then the
        // wide ranges are written over it.
        if (0x7f..0xa0).contains(&cp) {
            return 0;
        }
        if in_table(cp, &BMP_WIDE) {
            return 2;
        }
        if in_table(cp, &BMP_COMBINING) {
            return 0;
        }
        return 1;
    }
    if in_table(cp, &HIGH_COMBINING) {
        return 0;
    }
    if in_table(cp, &HIGH_WIDE) {
        return 2;
    }
    1
}

/// The width alacritty_terminal would give the character on its own
/// (`None` for the controls it drops).
pub fn alacritty_width(c: char) -> Option<usize> {
    unicode_width::UnicodeWidthChar::width(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_081_unicode_11_widths() {
        assert_eq!(wcwidth('a'), 1);
        assert_eq!(wcwidth('\u{7f}'), 0);
        assert_eq!(wcwidth('\u{301}'), 0);
        assert_eq!(wcwidth('中'), 2);
        assert_eq!(wcwidth('😀'), 2);
        // U+2764 HEAVY BLACK HEART is narrow; its VS16 joins it with no width.
        assert_eq!(wcwidth('\u{2764}'), 1);
        assert_eq!(wcwidth('\u{fe0f}'), 0);
        assert_eq!(wcwidth('\u{200b}'), 0);
        // 🥲 came in Unicode 13: one cell to xterm's v11 table, two to alacritty.
        assert_eq!(wcwidth('\u{1f972}'), 1);
        assert_eq!(alacritty_width('\u{1f972}'), Some(2));
        // And the other way round.
        assert_eq!(wcwidth('\u{1f93b}'), 2);
        assert_eq!(alacritty_width('\u{1f93b}'), Some(1));
        // Box drawing and the spinner's ✻ are one cell.
        assert_eq!(wcwidth('─'), 1);
        // As is Claude Code's ⏺ bullet.
        assert_eq!(wcwidth('⏺'), 1);
        assert_eq!(wcwidth('✻'), 1);
    }
}
