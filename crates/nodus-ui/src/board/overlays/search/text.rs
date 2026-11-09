//! Highlighted text (`Highlight.tsx`, SRCH-019, SRCH-020) and the rows'
//! relative times (`timeAgo`, CARD-041).

use gpui::{HighlightStyle, Hsla, Pixels, StyledText, UnderlineStyle, px};
use nodus_domain::js;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `.search-row mark { border-radius: 3px }`.
pub(crate) const MARK_RADIUS: Pixels = px(3.);
/// `.search-title mark { text-underline-offset: 3px }`.
pub(crate) const UNDERLINE_OFFSET: Pixels = px(3.);

/// How a match is marked: a background (snippets) or an underline (titles).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mark {
    Background(Hsla),
    Underline(Hsla),
}

/// `[start, end)` ranges in UTF-16 units as byte ranges of `text`; ranges
/// past its end are cut there (a surrogate pair split by a range keeps
/// whole characters).
pub(crate) fn byte_ranges(text: &str, ranges: &[[u32; 2]]) -> Vec<std::ops::Range<usize>> {
    let len = js::utf16_len(text);
    ranges
        .iter()
        .filter_map(|&[start, end]| {
            let (start, end) = ((start as usize).min(len), (end as usize).min(len));
            (start < end).then(|| js::utf16_to_byte(text, start)..js::utf16_to_byte(text, end))
        })
        .filter(|r| r.start < r.end)
        .collect()
}

/// The text with its ranges marked; no ranges, plain text. The element
/// holding it sets how marks look (`search.css`): a snippet's
/// `inline_background_radius(MARK_RADIUS)`, a title's
/// `underline_offset(UNDERLINE_OFFSET)`.
pub(crate) fn highlighted(text: &str, ranges: &[[u32; 2]], mark: Mark) -> StyledText {
    let style = match mark {
        Mark::Background(color) => HighlightStyle {
            background_color: Some(color),
            ..HighlightStyle::default()
        },
        Mark::Underline(color) => HighlightStyle {
            underline: Some(UnderlineStyle {
                // `text-decoration: underline 2px …`, but 0.9.4 drew it one
                // CSS pixel thick (two device rows in both goldens).
                thickness: px(1.),
                color: Some(color),
                wavy: false,
            }),
            ..HighlightStyle::default()
        },
    };
    let highlights: Vec<_> = byte_ranges(text, ranges).into_iter().map(|r| (r, style)).collect();
    StyledText::new(text.to_owned()).with_highlights(highlights)
}

/// `timeAgo`: `just now`, `Nm ago`, `Nh ago`, `Nd ago`, else `Oct 3` (or
/// `Oct 3, 2025` over 300 days back) in local time.
pub(crate) fn time_ago(ts: f64, now: f64) -> String {
    time_ago_with(ts, now, |ts| {
        let l = crate::terminal::text::local(ts);
        (l.year, l.month, l.day)
    })
}

fn time_ago_with(ts: f64, now: f64, date: impl Fn(f64) -> (i32, u32, u32)) -> String {
    if ts == 0.0 || ts.is_nan() {
        return String::new();
    }
    let diff = (now - ts).max(0.0);
    if diff < MINUTE {
        return "just now".into();
    }
    if diff < HOUR {
        return format!("{}m ago", (diff / MINUTE).floor());
    }
    if diff < DAY {
        return format!("{}h ago", (diff / HOUR).floor());
    }
    if diff < 7.0 * DAY {
        return format!("{}d ago", (diff / DAY).floor());
    }
    let (y, m, d) = date(ts);
    let month = MONTHS[(m as usize).min(11)];
    if now - ts > 300.0 * DAY {
        format!("{month} {d}, {y}")
    } else {
        format!("{month} {d}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_utf16_ranges_to_bytes() {
        // "é" is one unit and two bytes; the emoji two units and four bytes.
        let text = "é😀ab";
        assert_eq!(byte_ranges(text, &[[0, 1], [3, 5]]), vec![0..2, 6..8]);
        assert_eq!(byte_ranges(text, &[[4, 99]]), vec![7..8]);
        assert!(byte_ranges("abc", &[]).is_empty());
    }

    #[test]
    fn says_how_long_ago() {
        let now = 1_000.0 * DAY;
        let date = |_: f64| (2025, 9, 3);
        assert_eq!(time_ago_with(now - 10_000.0, now, date), "just now");
        assert_eq!(time_ago_with(now - 5.0 * MINUTE, now, date), "5m ago");
        assert_eq!(time_ago_with(now - 3.0 * HOUR, now, date), "3h ago");
        assert_eq!(time_ago_with(now - 2.0 * DAY, now, date), "2d ago");
        assert_eq!(time_ago_with(now - 20.0 * DAY, now, date), "Oct 3");
        assert_eq!(time_ago_with(now - 400.0 * DAY, now, date), "Oct 3, 2025");
        assert_eq!(time_ago_with(0.0, now, date), "");
    }
}
