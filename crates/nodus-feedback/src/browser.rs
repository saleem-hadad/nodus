//! The way round a server that can't be reached (RPT-040…RPT-043): GitHub's
//! new-issue page, filled in, opened in the browser. Nothing is posted until
//! the user presses Create there.

/// Where a new issue starts.
pub const NEW_ISSUE_URL: &str = "https://github.com/saleem-hadad/nodus/issues/new";

/// GitHub turns longer new-issue links away (HTTP 414).
pub const MAX_URL_LENGTH: usize = 8_000;

/// What stands in for a description too long for a link (RPT-042).
pub const ON_CLIPBOARD: &str = "_The description is on the clipboard: paste it here._";

/// The page to open, and the text for the clipboard when the description
/// didn't fit in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineIssue {
    pub url: String,
    pub clipboard: Option<String>,
}

/// RPT-041, RPT-042: the new-issue link with `title`, and `body` with the
/// footer under it as the server would have put it. A description that would
/// make the link too long goes to the clipboard instead, and the page says to
/// paste it.
pub fn offline_issue(title: &str, body: &str, footer: &str) -> OfflineIssue {
    let full = format!("{}\n\n---\n<sub>{footer}</sub>", body.trim());
    let url = issue_url(title, &full);
    if url.len() <= MAX_URL_LENGTH {
        return OfflineIssue { url, clipboard: None };
    }
    let short = format!("{ON_CLIPBOARD}\n\n---\n<sub>{footer}</sub>");
    OfflineIssue {
        url: issue_url(title, &short),
        clipboard: Some(full),
    }
}

fn issue_url(title: &str, body: &str) -> String {
    format!(
        "{NEW_ISSUE_URL}?title={}&body={}",
        form_encode(title),
        form_encode(body)
    )
}

/// `URLSearchParams` form encoding: space is `+`, letters, digits and `*-._`
/// stay, the rest is UTF-8 percent-encoded in upper case.
pub fn form_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encodes_as_urlsearchparams_does() {
        assert_eq!(form_encode("a b*-._~?é·"), "a+b*-._%7E%3F%C3%A9%C2%B7");
    }

    #[test]
    fn fills_in_the_new_issue_page() {
        let issue = offline_issue(
            "It broke",
            "Reported by @ada\n\nDetails\n",
            "Sent from Nodus feedback · 1.0.2",
        );
        assert_eq!(
            issue.url,
            "https://github.com/saleem-hadad/nodus/issues/new?title=It+broke&body=Reported+by+%40ada%0A%0ADetails%0A%0A---%0A%3Csub%3ESent+from+Nodus+feedback+%C2%B7+1.0.2%3C%2Fsub%3E"
        );
        assert_eq!(issue.clipboard, None);
    }

    #[test]
    fn puts_a_long_description_on_the_clipboard() {
        let body = "x ".repeat(5_000);
        let issue = offline_issue("Long", &body, "Sent from Nodus feedback");
        assert!(issue.url.len() <= MAX_URL_LENGTH);
        assert!(issue.url.contains("clipboard"));
        let copied = issue.clipboard.unwrap();
        assert!(copied.starts_with("x x x"));
        assert!(copied.ends_with("<sub>Sent from Nodus feedback</sub>"));
    }
}
