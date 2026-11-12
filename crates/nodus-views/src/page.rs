//! The page a view's frame loads: the HTML Claude wrote, with what every view
//! gets put in front of it. That's a content security policy with no network,
//! the board's colours as CSS variables, and `window.nodus`, the page's only
//! way to the board. The frame is sandboxed without same-origin, so the page
//! can't reach the board's window either; nodus posts messages to it, and the
//! board answers them for this view alone.
//!
//! Ports `src/main/viewPage.ts`. The theme, the bridge script and the guide
//! are kept beside this file exactly as viewPage.ts has them.

use nodus_domain::{js, json};
use serde::Serialize;
use serde_json::{Map, Value};

/// The scheme views are served on: `nodus-view://<view id>/` (VIEW-018).
pub const VIEW_SCHEME: &str = "nodus-view";

/// Inline scripts and styles only. Nothing loads from anywhere, not even
/// another view, and the page can't send anything anywhere: the board is its
/// only way out (VIEW-025).
pub const VIEW_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'";

/// How a view works, for its brain and for any session that puts one up
/// (VIEW-121). On Windows it says commands run in PowerShell.
#[cfg(not(windows))]
pub const VIEW_GUIDE: &str = concat!(include_str!("page/guide-1.txt"), include_str!("page/guide-2.txt"));
#[cfg(windows)]
pub const VIEW_GUIDE: &str = concat!(
    include_str!("page/guide-1.txt"),
    " This is Windows, and each command line runs in PowerShell, so write it for PowerShell.",
    include_str!("page/guide-2.txt")
);

/// The board's colours (tokens.css), light then dark. A view's page is its
/// panel on the board, so its ground is the panel's (VIEW-026).
pub const THEME: &str = include_str!("page/theme.css");

/// `window.nodus`, with `__BOOT__` where what the page starts with goes
/// (VIEW-031 to VIEW-043).
pub const BRIDGE: &str = include_str!("page/bridge.js");

/// What a view without a page yet would show, were its frame loaded anyway (VIEW-021).
pub const EMPTY_PAGE: &str = "<!doctype html><html><head></head><body></body></html>";

/// What the page starts with, so it can draw before asking the board for
/// anything. What's None is left out, as `undefined` is.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBoot {
    pub view_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_sessions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Map<String, Value>>,
}

/// JSON that's safe inside a `<script>`: no `</script>`, and no line
/// separators that end a JS string. `None` is `undefined`, written `null`
/// (VIEW-024).
pub fn script_json<T: Serialize + ?Sized>(value: Option<&T>) -> String {
    let text = value
        .and_then(|v| json::to_string(v).ok())
        .unwrap_or_else(|| "null".into());
    text.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Where `/<name(\s[^>]*)?>/i` first matches in `html`: the end of the tag.
/// `lower` is `html` with ASCII letters lowercased, so offsets are the same.
fn find_tag(html: &str, lower: &str, name: &str) -> Option<usize> {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(at) = lower[from..].find(&open).map(|i| from + i) {
        let after = at + open.len();
        match html[after..].chars().next() {
            Some('>') => return Some(after + 1),
            // `\s[^>]*>`: through the next `>`, if there is one.
            Some(c) if js::is_space(c) => return html[after..].find('>').map(|i| after + i + 1),
            _ => from = after,
        }
    }
    None
}

/// Where `/<!doctype[^>]*>/i` first matches: the end of the doctype.
fn find_doctype(lower: &str) -> Option<usize> {
    let at = lower.find("<!doctype")?;
    lower[at..].find('>').map(|i| at + i + 1)
}

/// The page as served: what every view gets, at the top of its `<head>`, then
/// the HTML Claude wrote. Put in front of the doctype it would switch the page
/// into quirks mode, so it goes after it (VIEW-022, VIEW-023).
pub fn build_view_page(html: &str, boot: &ViewBoot) -> String {
    // A function as the replacement in the TypeScript, so `$&` in the boot
    // stays as it is; only the first `__BOOT__` is replaced.
    let bridge = BRIDGE.replacen("__BOOT__", &script_json(Some(boot)), 1);
    let injected = format!(
        "<meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{VIEW_CSP}\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><style>{THEME}</style><script>{bridge}</script>"
    );
    let insert_after = |end: usize, text: &str| format!("{}{text}{}", &html[..end], &html[end..]);

    let lower = html.to_ascii_lowercase();
    let doctype = find_doctype(&lower);
    // Without one the page would render in quirks mode.
    let standards = |page: String| {
        if doctype.is_some() {
            page
        } else {
            format!("<!doctype html>{page}")
        }
    };
    if let Some(end) = find_tag(html, &lower, "head") {
        return standards(insert_after(end, &injected));
    }
    if let Some(end) = find_tag(html, &lower, "html") {
        return standards(insert_after(end, &format!("<head>{injected}</head>")));
    }
    if let Some(end) = doctype {
        return insert_after(end, &format!("<head>{injected}</head>"));
    }
    format!("<!doctype html><html><head>{injected}</head><body>{html}</body></html>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tags_as_the_regexes_do() {
        let at = |html: &str, name: &str| find_tag(html, &html.to_ascii_lowercase(), name);
        assert_eq!(at("<head>", "head"), Some(6));
        assert_eq!(at("<HEAD lang=x>", "head"), Some(13));
        assert_eq!(at("<header><head>", "head"), Some(14));
        assert_eq!(at("<head", "head"), None);
        assert_eq!(at("<head x", "head"), None);
        assert_eq!(at("<head\u{a0}x>", "head"), Some(9));
        assert_eq!(find_doctype("x<!doctype html>"), Some(16));
        assert_eq!(find_doctype("<!doctype html"), None);
    }
}
