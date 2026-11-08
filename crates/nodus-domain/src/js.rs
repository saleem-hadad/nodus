//! JavaScript's own rules for strings and numbers, which the TypeScript relies
//! on without saying so. Ports use these instead of Rust's defaults wherever
//! the TypeScript measures, cuts or rounds (migration/plan.md, "JavaScript
//! semantics to keep").

/// `s.length`: the number of UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(start, end)` with non-negative bounds in UTF-16 code units,
/// clamped to the string as JavaScript clamps them.
///
/// JavaScript can cut an astral character (an emoji) in half, leaving a lone
/// surrogate; a Rust string can't hold one, so that half becomes U+FFFD, which
/// is also how the window draws it and how `nodus_domain::json` reads it back.
pub fn slice_utf16(s: &str, start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut at = 0;
    for c in s.chars() {
        let width = c.len_utf16();
        let next = at + width;
        if at >= end {
            break;
        }
        if at >= start && next <= end {
            out.push(c);
        } else if next > start {
            // Half of a surrogate pair falls inside the range.
            out.push(char::REPLACEMENT_CHARACTER);
        }
        at = next;
    }
    out
}

/// The byte offset in `s` of UTF-16 offset `offset`, clamped to the end. An
/// offset inside a surrogate pair maps to the start of that character.
pub fn utf16_to_byte(s: &str, offset: usize) -> usize {
    let mut at = 0;
    for (byte, c) in s.char_indices() {
        if at >= offset {
            return byte;
        }
        at += c.len_utf16();
        if at > offset {
            return byte;
        }
    }
    s.len()
}

/// The UTF-16 offset of byte offset `byte`, which must be a char boundary.
pub fn byte_to_utf16(s: &str, byte: usize) -> usize {
    utf16_len(&s[..byte])
}

/// `Math.round(x)`: halves round toward +∞, so `Math.round(-2.5)` is -2, where
/// Rust's `f64::round` gives -3.
pub fn round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_count_utf16_units() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("é"), 1);
        assert_eq!(utf16_len("😀"), 2);
        assert_eq!(utf16_len("a😀b"), 4);
    }

    #[test]
    fn slices_like_javascript() {
        assert_eq!(slice_utf16("hello", 0, 3), "hel");
        assert_eq!(slice_utf16("hello", 2, 99), "llo");
        assert_eq!(slice_utf16("hello", 4, 2), "");
        assert_eq!(slice_utf16("a😀b", 0, 3), "a😀");
        // Cutting the emoji in half leaves a lone surrogate in JavaScript.
        assert_eq!(slice_utf16("a😀b", 0, 2), "a\u{FFFD}");
        assert_eq!(slice_utf16("a😀b", 2, 4), "\u{FFFD}b");
    }

    #[test]
    fn maps_offsets_between_utf16_and_bytes() {
        let s = "a😀é";
        assert_eq!(utf16_to_byte(s, 0), 0);
        assert_eq!(utf16_to_byte(s, 1), 1);
        assert_eq!(utf16_to_byte(s, 2), 1);
        assert_eq!(utf16_to_byte(s, 3), 5);
        assert_eq!(utf16_to_byte(s, 4), 7);
        assert_eq!(utf16_to_byte(s, 9), 7);
        assert_eq!(byte_to_utf16(s, 5), 3);
        assert_eq!(byte_to_utf16(s, 7), 4);
    }

    #[test]
    fn rounds_halves_up_like_math_round() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(-2.6), -3.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert_eq!(round(1.4), 1.0);
        assert!(round(f64::NAN).is_nan());
        assert_eq!(round(f64::INFINITY), f64::INFINITY);
    }
}
