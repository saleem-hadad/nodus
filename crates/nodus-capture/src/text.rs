//! Floating capture: how what the user typed, and what they brought from
//! their screen, is written out for Claude and for the board. A port of
//! `src/shared/capture.ts`.

use nodus_domain::js;
use nodus_domain::{CaptureAttachment, CaptureKind, MAX_COPIED_CHARS};

/// `isCaptureKind`: the kind a string names (`session`, `into`, `note`,
/// `view`), if it names one. Tab cycles through `CaptureKind::ALL` in order.
pub fn capture_kind(value: &str) -> Option<CaptureKind> {
    CaptureKind::parse(value)
}

/// The longest title [`title_from`] makes by default.
pub const TITLE_MAX: usize = 48;

/// A short name for a capture: its first sentence, or as much of it as fits
/// (CAP-136). `max` counts UTF-16 units, as JavaScript does.
pub fn title_from(text: &str, max: usize) -> String {
    let line = js::trim(text).split('\n').next().unwrap_or_default();
    let sentence = first_sentence(line).unwrap_or(line);
    let sentence = js::trim(sentence.trim_end_matches(['.', '!', '?']));
    if js::utf16_len(sentence) <= max {
        return sentence.to_owned();
    }
    let cut = js::slice_utf16(sentence, 0, max.saturating_sub(1));
    // `word > max / 2`, with JavaScript's fractional half.
    let word = cut.rfind(' ').map(|byte| js::byte_to_utf16(&cut, byte));
    let kept = match word {
        Some(word) if word as f64 > max as f64 / 2.0 => js::slice_utf16(&cut, 0, word),
        _ => cut,
    };
    format!("{}…", js::trim_end(&kept))
}

/// `/^(.+?[.!?])(\s|$)/.exec(line)?.[1]`: the shortest start of the line that
/// ends in `.`, `!` or `?` followed by a space or the end. `.` doesn't match a
/// line break, so the sentence can't run past one.
fn first_sentence(line: &str) -> Option<&str> {
    let mut chars = line.char_indices().peekable();
    let mut seen_one = false;
    while let Some((at, c)) = chars.next() {
        if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            return None;
        }
        if seen_one && matches!(c, '.' | '!' | '?') {
            let end = at + c.len_utf8();
            match chars.peek() {
                None => return Some(&line[..end]),
                Some((_, next)) if js::is_space(*next) => return Some(&line[..end]),
                _ => {}
            }
        }
        seen_one = true;
    }
    None
}

/// A fence longer than any run of backticks in the text, so it can't close
/// early: at least three (CAP-139).
pub fn fence(text: &str) -> String {
    let mut longest = 2;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let ticks = "`".repeat(longest + 1);
    format!("{ticks}\n{text}\n{ticks}")
}

/// What came from the screen, as lines for Claude to read: the browser tab,
/// screenshots and files by path (Claude reads them itself), and copied text
/// in full. Empty when nothing came along (CAP-137).
pub fn describe_attachments(attachments: &[CaptureAttachment]) -> String {
    let mut lines = Vec::new();
    let mut copied = Vec::new();
    for attachment in attachments {
        match attachment {
            CaptureAttachment::Tab(tab) => lines.push(if tab.title.is_empty() {
                format!("- The {} tab I had open: {}", tab.app, tab.url)
            } else {
                format!("- The {} tab I had open: {} ({})", tab.app, tab.title, tab.url)
            }),
            CaptureAttachment::Image(image) => {
                let what = if image.copied == Some(true) {
                    "An image I copied"
                } else {
                    "A screenshot"
                };
                lines.push(format!("- {what}: {}", image.path));
            }
            CaptureAttachment::File(file) => lines.push(format!("- A file: {}", file.path)),
            CaptureAttachment::Text(text) => {
                let text = js::slice_utf16(js::trim(&text.text), 0, MAX_COPIED_CHARS);
                if !text.is_empty() {
                    copied.push(text);
                }
            }
        }
    }
    if lines.is_empty() && copied.is_empty() {
        return String::new();
    }
    let mut parts = vec!["From my screen as I wrote this:".to_owned()];
    if !lines.is_empty() {
        parts.push(lines.join("\n"));
    }
    for text in copied {
        parts.push(format!("Text I copied:\n{}", fence(&text)));
    }
    parts.join("\n\n")
}

/// The text, followed by what came from the screen (CAP-138).
pub fn with_screen(text: &str, attachments: &[CaptureAttachment]) -> String {
    let screen = describe_attachments(attachments);
    if screen.is_empty() {
        js::trim(text).to_owned()
    } else {
        format!("{}\n\n{screen}", js::trim(text))
    }
}

#[cfg(test)]
mod tests {
    //! `src/shared/capture.test.ts`, case by case, then more.

    use nodus_domain::{FileAttachment, ImageAttachment, TabAttachment, TextAttachment};

    use super::*;

    fn title(text: &str) -> String {
        title_from(text, TITLE_MAX)
    }

    fn text(text: &str) -> CaptureAttachment {
        CaptureAttachment::Text(TextAttachment { text: text.into() })
    }

    fn file(path: &str, name: &str) -> CaptureAttachment {
        CaptureAttachment::File(FileAttachment {
            path: path.into(),
            name: name.into(),
            size: 1,
        })
    }

    #[test]
    fn title_from_takes_the_first_sentence() {
        assert_eq!(
            title("Refunds fail with a coupon. Write a test, then fix it"),
            "Refunds fail with a coupon"
        );
    }

    #[test]
    fn title_from_takes_the_first_line() {
        assert_eq!(title("Bump Stripe\nand run the tests"), "Bump Stripe");
    }

    #[test]
    fn title_from_cuts_a_long_sentence_at_a_word() {
        let cut = title("Look into why the checkout total is rounded down when the customer has a coupon applied");
        assert!(js::utf16_len(&cut) <= 48);
        assert_eq!(cut, "Look into why the checkout total is rounded…");
    }

    #[test]
    fn title_from_doesnt_take_a_dot_inside_a_word_as_the_end_of_a_sentence() {
        assert_eq!(title("Fix README.md links"), "Fix README.md links");
    }

    /// CAP-136: the edges of the sentence rule and the cut.
    #[test]
    fn title_from_follows_the_regex_at_its_edges() {
        // One character before the stop is needed; `?!` at the end all go.
        assert_eq!(title("Why?! Really"), "Why");
        assert_eq!(title("."), "");
        assert_eq!(title("Done."), "Done");
        // A stop followed by a non-breaking space ends a sentence (JavaScript's \s).
        assert_eq!(title("Fix it.\u{A0}Then ship"), "Fix it");
        // A carriage return stops the search: no sentence, so the whole line.
        assert_eq!(title("Fix it\r. Then"), "Fix it\r. Then");
        // Trimmed first, so a leading blank line doesn't make the title empty.
        assert_eq!(title("\n\n  Ship it  \nlater"), "Ship it");
        // A space past the middle: cut there. None: cut at 47. Both marked.
        // (Checked against the TypeScript under Node.)
        let long = "a".repeat(30) + " " + &"b".repeat(30);
        assert_eq!(title(&long), "a".repeat(30) + "…");
        let unbroken = "x".repeat(60);
        assert_eq!(title(&unbroken), "x".repeat(47) + "…");
        // A space at 24 isn't past the middle (24 > 24 is false).
        let at_half = "a".repeat(24) + " " + &"b".repeat(40);
        assert_eq!(title(&at_half), "a".repeat(24) + " " + &"b".repeat(22) + "…");
        // Lengths are UTF-16 units: 24 emoji are 48 units and fit.
        let emoji = "😀".repeat(24);
        assert_eq!(title(&emoji), emoji);
    }

    #[test]
    fn describe_attachments_is_empty_with_nothing_attached() {
        assert_eq!(describe_attachments(&[]), "");
        assert_eq!(describe_attachments(&[text("   ")]), "");
    }

    #[test]
    fn describe_attachments_lists_the_tab_images_and_files_then_the_copied_text() {
        let out = describe_attachments(&[
            text("TypeError: amount.minus"),
            CaptureAttachment::Tab(TabAttachment {
                app: "Safari".into(),
                title: "Refund fails #482".into(),
                url: "https://example.com/acme/storefront/issues/482".into(),
            }),
            CaptureAttachment::Image(ImageAttachment {
                path: "/tmp/shot.png".into(),
                thumb: String::new(),
                copied: None,
            }),
            CaptureAttachment::Image(ImageAttachment {
                path: "/tmp/copied.png".into(),
                thumb: String::new(),
                copied: Some(true),
            }),
            file("/logs/checkout.log", "checkout.log"),
        ]);
        assert_eq!(
            out,
            [
                "From my screen as I wrote this:",
                "",
                "- The Safari tab I had open: Refund fails #482 (https://example.com/acme/storefront/issues/482)",
                "- A screenshot: /tmp/shot.png",
                "- An image I copied: /tmp/copied.png",
                "- A file: /logs/checkout.log",
                "",
                "Text I copied:",
                "```",
                "TypeError: amount.minus",
                "```",
            ]
            .join("\n")
        );
    }

    #[test]
    fn describe_attachments_fences_copied_text_with_more_backticks_than_it_holds() {
        let out = describe_attachments(&[text("```js\nx()\n```")]);
        assert!(out.contains("````\n```js\nx()\n```\n````"), "{out}");
    }

    /// CAP-137: a tab with no title, several copied texts, only copied text.
    #[test]
    fn describe_attachments_words_the_rest() {
        let out = describe_attachments(&[
            CaptureAttachment::Tab(TabAttachment {
                app: "Arc".into(),
                title: String::new(),
                url: "https://example.com/".into(),
            }),
            text("  one  "),
            text("two"),
        ]);
        assert_eq!(
            out,
            "From my screen as I wrote this:\n\n- The Arc tab I had open: https://example.com/\n\nText I copied:\n```\none\n```\n\nText I copied:\n```\ntwo\n```"
        );
        assert_eq!(
            describe_attachments(&[text("x")]),
            "From my screen as I wrote this:\n\nText I copied:\n```\nx\n```"
        );
    }

    /// CAP-139
    #[test]
    fn fences_are_at_least_three_backticks() {
        assert_eq!(fence("plain"), "```\nplain\n```");
        assert_eq!(fence("a `b` c"), "```\na `b` c\n```");
        assert_eq!(fence("`````"), "``````\n`````\n``````");
    }

    #[test]
    fn with_screen_leaves_text_alone_with_nothing_attached() {
        assert_eq!(with_screen("  Fix it  ", &[]), "Fix it");
    }

    #[test]
    fn with_screen_puts_the_screen_after_the_text() {
        assert_eq!(
            with_screen("Fix it", &[file("/a.log", "a.log")]),
            "Fix it\n\nFrom my screen as I wrote this:\n\n- A file: /a.log"
        );
    }

    #[test]
    fn kinds_are_named_and_ordered_as_the_typescript_has_them() {
        let names: Vec<&str> = CaptureKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names, ["session", "into", "note", "view"]);
        assert_eq!(capture_kind("into"), Some(CaptureKind::Into));
        assert_eq!(capture_kind("party"), None);
    }
}
