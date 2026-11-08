//! Links in the terminal, as 0.9.4 finds and opens them:
//!
//! - web links (`@xterm/addon-web-links`, TERM-065): the addon's regex over
//!   the hovered line joined with its wrapped neighbours, kept only if
//!   `new URL()` parses the text and the text starts with the parsed origin;
//! - OSC 8 hyperlinks (xterm's `OscLinkProvider`, TERM-068): runs of cells
//!   carrying the same http(s) URI, ahead of web links where both apply.
//!
//! Ranges are xterm's: 1-based, `y` counting buffer lines from the top of the
//! history, the end inclusive (and, as in xterm, `x: 0` on the next line when
//! a web link ends on the last column).

use std::sync::LazyLock;

use regex::Regex;

use crate::terminal::Terminal;

/// `WebLinksAddon.ts`'s `strictUrlRegex`, with JavaScript's `\s` spelled out
/// (it includes U+FEFF and not U+0085, unlike Rust's).
const JS_SPACE: &str =
    r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

static STRICT_URL: LazyLock<Regex> = LazyLock::new(|| {
    let pattern =
        format!(r#"(https?|HTTPS?):[/]{{2}}[^{JS_SPACE}"'!*(){{}}|\\\^<>`]*[^{JS_SPACE}"':,.!?{{}}|\\\^~\[\]`()<>]"#);
    Regex::new(&pattern).expect("the web-links regex")
});

/// A 1-based cell, as xterm's link ranges give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkPoint {
    pub x: usize,
    pub y: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A URL in the text: opens in the default browser (TERM-067).
    Web,
    /// An OSC 8 hyperlink: xterm asks first (TERM-068).
    Osc8,
}

/// A link under the pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// The URL: the matched text, or the OSC 8 URI.
    pub text: String,
    pub start: LinkPoint,
    pub end: LinkPoint,
}

impl Link {
    /// `Linkifier._linkAtPosition`.
    pub fn contains(&self, p: LinkPoint, cols: usize) -> bool {
        let lower = self.start.y * cols + self.start.x;
        let upper = self.end.y * cols + self.end.x;
        let current = p.y * cols + p.x;
        lower <= current && current <= upper
    }
}

/// xterm's `isUrl`: `new URL(text)` parses, and the text starts (ignoring
/// case) with `protocol//[user[:password]@]host`.
pub fn is_url(text: &str) -> bool {
    let Ok(url) = url::Url::parse(text) else {
        return false;
    };
    let host = match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_owned(),
        (None, _) => String::new(),
    };
    let protocol = format!("{}:", url.scheme());
    let base = match (url.username(), url.password()) {
        (user, Some(pass)) if !user.is_empty() && !pass.is_empty() => format!("{protocol}//{user}:{pass}@{host}"),
        (user, _) if !user.is_empty() => format!("{protocol}//{user}@{host}"),
        _ => format!("{protocol}//{host}"),
    };
    text.to_lowercase().starts_with(&base.to_lowercase())
}

/// Where a link opens: http(s) URLs, as the browser resolves them, go to
/// the default browser (`setWindowOpenHandler` in src/main/index.ts); never
/// inside the app (TERM-067).
pub fn browser_url(text: &str) -> Option<String> {
    let url = url::Url::parse(text).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

/// What xterm's default OSC 8 handler asks before following a link
/// (`OscLinkProvider.ts`, `linkHandler: null`). In 0.9.4 its `window.open()`
/// then reaches the window-open handler with no URL, which opens nothing.
pub fn osc8_confirm_text(uri: &str) -> String {
    format!("Do you want to navigate to {uri}?\n\nWARNING: This link could potentially be dangerous")
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

impl Terminal {
    /// The alacritty line of xterm buffer line `index` (0-based from the top of the history).
    fn line_of_index(&self, index: i64) -> Option<i32> {
        let total = (self.history_size() + self.rows()) as i64;
        (0..total)
            .contains(&index)
            .then(|| (index - self.history_size() as i64) as i32)
    }

    /// `_getWindowedLineStrings`: the line and its wrapped neighbours, each
    /// trimmed, stopping at a line holding a space or after 2048 characters.
    fn windowed_lines(&self, index: i64) -> (Vec<String>, i64) {
        let mut lines = Vec::new();
        let mut top = index;
        let Some(line) = self.line_of_index(index) else {
            return (lines, top);
        };
        let current = self.line_text(line, true, 0, None);
        if self.is_wrapped(line) && !current.starts_with(' ') {
            let mut length = 0;
            loop {
                top -= 1;
                let Some(l) = self.line_of_index(top).filter(|_| length < 2048) else {
                    break;
                };
                let content = self.line_text(l, true, 0, None);
                length += utf16_len(&content);
                let stop = !self.is_wrapped(l) || content.contains(' ');
                lines.push(content);
                if stop {
                    break;
                }
            }
            lines.reverse();
        }
        lines.push(current);
        let mut bottom = index;
        let mut length = 0;
        loop {
            bottom += 1;
            let Some(l) = self
                .line_of_index(bottom)
                .filter(|l| self.is_wrapped(*l) && length < 2048)
            else {
                break;
            };
            let content = self.line_text(l, true, 0, None);
            length += utf16_len(&content);
            let stop = content.contains(' ');
            lines.push(content);
            if stop {
                break;
            }
        }
        (lines, top)
    }

    /// `_mapStrIdx`: a string index back to a 0-based [line index, column].
    fn map_str_idx(&self, mut index: i64, start_col: usize, mut string_index: i64) -> Option<(i64, usize)> {
        let cols = self.cols();
        let mut start = start_col;
        while string_index != 0 {
            let line = self.line_of_index(index)?;
            for i in start..cols {
                let (chars, width) = self.cell(line, i);
                if width != 0 {
                    let len = utf16_len(&chars) as i64;
                    string_index -= if len == 0 { 1 } else { len };
                    // A wide character wrapped early leaves an empty last cell.
                    if i == cols - 1
                        && chars.is_empty()
                        && let Some(next) = self.line_of_index(index + 1)
                        && self.is_wrapped(next)
                        && self.cell(next, 0).1 == 2
                    {
                        string_index += 1;
                    }
                }
                if string_index < 0 {
                    return Some((index, i));
                }
            }
            index += 1;
            start = 0;
        }
        Some((index, start))
    }

    /// The web links found from buffer line `y` (1-based, as xterm's
    /// `provideLinks(y)`).
    pub fn web_links(&self, y: usize) -> Vec<Link> {
        let (lines, top) = self.windowed_lines(y as i64 - 1);
        let text = lines.concat();
        let mut links = Vec::new();
        for m in STRICT_URL.find_iter(&text) {
            let matched = m.as_str();
            if !is_url(matched) {
                continue;
            }
            let at = utf16_len(&text[..m.start()]) as i64;
            let Some((start_y, start_x)) = self.map_str_idx(top, 0, at) else {
                continue;
            };
            let Some((end_y, end_x)) = self.map_str_idx(start_y, start_x, utf16_len(matched) as i64) else {
                continue;
            };
            links.push(Link {
                kind: LinkKind::Web,
                text: matched.to_owned(),
                start: LinkPoint {
                    x: start_x + 1,
                    y: start_y as usize + 1,
                },
                end: LinkPoint {
                    x: end_x,
                    y: end_y as usize + 1,
                },
            });
        }
        links
    }

    /// The OSC 8 links on buffer line `y` (1-based), as `OscLinkProvider.provideLinks`
    /// finds them: runs of written cells with the same link, http and https only.
    pub fn osc8_links(&self, y: usize) -> Vec<Link> {
        use alacritty_terminal::index::{Column, Line};
        let Some(line) = self.line_of_index(y as i64 - 1) else {
            return Vec::new();
        };
        let row = &self.term.grid()[Line(line)];
        let length = self.trimmed_length(line);
        let mut links = Vec::new();
        let mut current: Option<(usize, alacritty_terminal::term::cell::Hyperlink)> = None;
        for x in 0..length {
            let has_content = !self.cell(line, x).0.is_empty();
            if current.is_none() && !has_content {
                continue;
            }
            let link = row[Column(x)].hyperlink();
            let mut finish = false;
            match (&current, &link) {
                (None, Some(l)) => {
                    current = Some((x, l.clone()));
                    continue;
                }
                (Some((_, open)), Some(l)) => finish = open != l,
                (Some(_), None) => finish = true,
                (None, None) => {}
            }
            if finish || (current.is_some() && x == length - 1) {
                if let Some((start, open)) = &current {
                    let uri = open.uri().to_owned();
                    if url::Url::parse(&uri).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
                        let end = x + usize::from(!finish && x == length - 1);
                        links.push(Link {
                            kind: LinkKind::Osc8,
                            text: uri,
                            start: LinkPoint { x: start + 1, y },
                            end: LinkPoint { x: end, y },
                        });
                    }
                }
                // Clear the link, or start the one that begins right here.
                current = link.map(|l| (x, l));
            }
        }
        links
    }

    /// The link at a 1-based cell, OSC 8 first, as xterm's linkifier picks it
    /// (TERM-066).
    pub fn link_at(&self, p: LinkPoint) -> Option<Link> {
        let cols = self.cols();
        self.osc8_links(p.y)
            .into_iter()
            .find(|l| l.contains(p, cols))
            .or_else(|| self.web_links(p.y).into_iter().find(|l| l.contains(p, cols)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_065_is_url_checks_the_origin() {
        assert!(is_url("https://example.com/x"));
        assert!(is_url("HTTPS://UPPER.EXAMPLE/Path"));
        assert!(is_url("https://user:pass@e.example:8080/p"));
        assert!(!is_url("https://exa mple"));
        assert!(!is_url("https://ü.example/päth"));
    }

    #[test]
    fn term_067_links_open_http_only_in_the_browser() {
        assert_eq!(
            browser_url("HTTPS://UPPER.EXAMPLE/Path").as_deref(),
            Some("https://upper.example/Path")
        );
        assert_eq!(browser_url("ftp://f.example"), None);
    }

    #[test]
    fn term_068_osc8_confirm_text() {
        assert_eq!(
            osc8_confirm_text("https://example.com"),
            "Do you want to navigate to https://example.com?\n\nWARNING: This link could potentially be dangerous"
        );
    }
}
