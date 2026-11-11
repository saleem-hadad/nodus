//! The words of a report: the title and description drafted from what the
//! user wrote, the GitHub username that gets them notified, the footer, and
//! the personal details taken out before anything is shown for review.

use std::process::Command;

/// The longest drafted title, in characters (RPT-021).
pub const TITLE_CHARS: usize = 80;

/// The line that puts the reporter on the issue (RPT-022). The server keeps
/// this one mention and neutralizes every other.
pub const REPORTED_BY: &str = "Reported by @";

/// What a GitHub username can't be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidUsername;

/// RPT-005: GitHub's rule for usernames: letters, digits and single hyphens,
/// not at either end, at most 39. A leading `@` is allowed and dropped;
/// blank is no username.
pub fn github_username(raw: &str) -> Result<Option<String>, InvalidUsername> {
    let name = raw.trim();
    let name = name.strip_prefix('@').unwrap_or(name);
    if name.is_empty() {
        return Ok(None);
    }
    let valid = name.len() <= 39
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if valid {
        Ok(Some(name.to_owned()))
    } else {
        Err(InvalidUsername)
    }
}

/// RPT-021: the first line that isn't blank, spaces collapsed, cut to
/// [`TITLE_CHARS`] with an ellipsis; `Problem report` when there's nothing.
pub fn draft_title(message: &str) -> String {
    let line = message.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return "Problem report".to_owned();
    }
    if line.chars().count() <= TITLE_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(TITLE_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// RPT-022: the description as the user wrote it, under the reporter's line
/// when there is one.
pub fn draft_body(message: &str, reporter: Option<&str>) -> String {
    let message = message.trim();
    match reporter {
        Some(name) => format!("{REPORTED_BY}{name}\n\n{message}"),
        None => message.to_owned(),
    }
}

/// RPT-023: the line the server puts under the description, as the review
/// shows it: `Sent from Nodus feedback · 1.0.2 · macOS 27.2 · arm64`.
pub fn footer(version: &str, os: &str, arch: &str) -> String {
    let mut parts = vec!["Sent from Nodus feedback"];
    parts.extend([version, os, arch].into_iter().filter(|p| !p.is_empty()));
    parts.join(" · ")
}

/// Personal details to take out of what the user wrote before it's shown for
/// review (RPT-024): the home folder becomes `~`, the user's account name and
/// the computer's names become placeholders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scrub {
    home: Option<String>,
    /// Each word, with what replaces it; longest first.
    words: Vec<(String, &'static str)>,
}

/// Names shorter than this stay: they'd hit ordinary words.
const MIN_WORD: usize = 3;

impl Scrub {
    pub fn new(home: Option<String>, user: Option<String>, computers: Vec<String>) -> Self {
        let home = home
            .map(|h| h.trim_end_matches(['/', '\\']).to_owned())
            .filter(|h| h.chars().count() > 1);
        let mut words: Vec<(String, &'static str)> = Vec::new();
        let mut push = |word: String, with: &'static str| {
            let word = word.trim().to_owned();
            if word.chars().count() >= MIN_WORD && !words.iter().any(|(w, _)| w.eq_ignore_ascii_case(&word)) {
                words.push((word, with));
            }
        };
        if let Some(user) = user {
            push(user, "<user>");
        }
        for computer in computers {
            push(computer, "<computer>");
        }
        // A computer name can hold the user's name ("Ada's MacBook Pro"):
        // the longer one goes first.
        words.sort_by_key(|(w, _)| std::cmp::Reverse(w.chars().count()));
        Self { home, words }
    }

    /// This computer's: `HOME` (`USERPROFILE` on Windows), the account name,
    /// and the computer's names. Runs `scutil` on a Mac, so call it off the
    /// UI thread.
    pub fn system() -> Self {
        let home = std::env::var("HOME")
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok())
            .filter(|h| !h.is_empty());
        let user = std::env::var("USER")
            .ok()
            .or_else(|| std::env::var("USERNAME").ok())
            .filter(|u| !u.is_empty());
        Self::new(home, user, computer_names())
    }

    /// `text` with the details taken out.
    pub fn apply(&self, text: &str) -> String {
        let mut out = match &self.home {
            Some(home) => replace_home(text, home),
            None => text.to_owned(),
        };
        for (word, with) in &self.words {
            out = replace_word(&out, word, with);
        }
        out
    }
}

/// The computer's names: macOS's ComputerName and LocalHostName, or
/// Windows's `COMPUTERNAME`, and the host name either way (without
/// `.local`).
fn computer_names() -> Vec<String> {
    let mut names = Vec::new();
    #[cfg(target_os = "macos")]
    for key in ["ComputerName", "LocalHostName"] {
        if let Ok(out) = Command::new("/usr/sbin/scutil").args(["--get", key]).output()
            && out.status.success()
        {
            names.push(String::from_utf8_lossy(&out.stdout).trim().to_owned());
        }
    }
    if let Ok(name) = std::env::var("COMPUTERNAME") {
        names.push(name);
    }
    if let Ok(out) = Command::new("hostname").output()
        && out.status.success()
    {
        let host = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        names.push(host.trim_end_matches(".local").to_owned());
    }
    names.retain(|n| !n.is_empty());
    names
}

/// Every `home` that is a whole path start (not `/Users/example` in
/// `/Users/examples`) becomes `~`. Case is ignored on Windows, and either slash
/// matches there.
fn replace_home(text: &str, home: &str) -> String {
    let windows = home.contains('\\') || home.as_bytes().get(1) == Some(&b':');
    let fold = |s: &str| {
        if windows {
            s.to_ascii_lowercase().replace('\\', "/")
        } else {
            s.to_owned()
        }
    };
    // Folding keeps byte offsets: ASCII case and one slash for another.
    let hay = fold(text);
    let needle = fold(home);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(found) = hay[from..].find(&needle) {
        let start = from + found;
        let end = start + needle.len();
        let next = text[end..].chars().next();
        if next.is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))) {
            out.push_str(&text[last..start]);
            out.push('~');
            last = end;
        }
        from = end;
    }
    out.push_str(&text[last..]);
    out
}

/// Every whole-word `word`, any case, becomes `with`.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let lower_text = text.to_lowercase();
    let lower_word = word.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; then match exactly.
    let (hay, needle) = if lower_text.len() == text.len() && lower_word.len() == word.len() {
        (lower_text, lower_word)
    } else {
        (text.to_owned(), word.to_owned())
    };
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(found) = hay[from..].find(&needle) {
        let start = from + found;
        let end = start + needle.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
            out.push_str(&text[last..start]);
            out.push_str(with);
            last = end;
        }
        from = end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_github_usernames() {
        assert_eq!(github_username(""), Ok(None));
        assert_eq!(github_username("   "), Ok(None));
        assert_eq!(github_username("ada-lovelace"), Ok(Some("ada-lovelace".into())));
        assert_eq!(github_username(" @Ada1 "), Ok(Some("Ada1".into())));
        assert_eq!(github_username("-ada"), Err(InvalidUsername));
        assert_eq!(github_username("ada-"), Err(InvalidUsername));
        assert_eq!(github_username("a--b"), Err(InvalidUsername));
        assert_eq!(github_username("ada lovelace"), Err(InvalidUsername));
        assert_eq!(github_username("ada_l"), Err(InvalidUsername));
        assert_eq!(github_username(&"a".repeat(39)), Ok(Some("a".repeat(39))));
        assert_eq!(github_username(&"a".repeat(40)), Err(InvalidUsername));
    }

    #[test]
    fn drafts_the_title_from_the_first_line() {
        assert_eq!(
            draft_title("\n\n  The   section didn't grow \nmore"),
            "The section didn't grow"
        );
        assert_eq!(draft_title("   \n "), "Problem report");
        let long = "word ".repeat(30);
        let title = draft_title(&long);
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with("word…"), "{title}");
        assert_eq!(draft_title(&"é".repeat(80)), "é".repeat(80));
        assert_eq!(draft_title(&"é".repeat(81)).chars().count(), 80);
    }

    #[test]
    fn drafts_the_body_under_the_reporter() {
        assert_eq!(
            draft_body("  It broke.\n", Some("ada")),
            "Reported by @ada\n\nIt broke."
        );
        assert_eq!(draft_body("It broke.", None), "It broke.");
    }

    #[test]
    fn writes_the_footer() {
        assert_eq!(
            footer("1.0.2", "macOS 27.2", "arm64"),
            "Sent from Nodus feedback · 1.0.2 · macOS 27.2 · arm64"
        );
        assert_eq!(footer("1.0.2", "", "x64"), "Sent from Nodus feedback · 1.0.2 · x64");
        assert_eq!(footer("", "", ""), "Sent from Nodus feedback");
    }

    #[test]
    fn takes_the_home_folder_out() {
        let s = Scrub::new(Some("/Users/example/".into()), None, vec![]);
        assert_eq!(
            s.apply("in /Users/example/Code/app and /Users/example"),
            "in ~/Code/app and ~"
        );
        assert_eq!(
            s.apply("not /Users/examples/x or /Users/example.old"),
            "not /Users/examples/x or /Users/example.old"
        );
        let w = Scrub::new(Some(r"C:\Users\Example".into()), None, vec![]);
        assert_eq!(
            w.apply(r"at c:\users\example\code and C:/Users/example/x"),
            r"at ~\code and ~/x"
        );
    }

    #[test]
    fn takes_names_out_as_whole_words() {
        let s = Scrub::new(
            None,
            Some("ada".into()),
            vec!["Ada's MacBook Pro".into(), "Adas-MacBook-Pro".into(), "ab".into()],
        );
        assert_eq!(
            s.apply("ada@Adas-MacBook-Pro ~ % on Ada's MacBook Pro; Adam and canada stay; ab stays"),
            "<user>@<computer> ~ % on <computer>; Adam and canada stay; ab stays"
        );
    }

    #[test]
    fn scrubs_home_before_names() {
        let s = Scrub::new(Some("/Users/example".into()), Some("example".into()), vec![]);
        assert_eq!(s.apply("/Users/example/x by example"), "~/x by <user>");
    }
}
