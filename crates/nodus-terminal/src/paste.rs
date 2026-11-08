//! Pasting and dropping files, as xterm's `paste()` and registry.ts do.

/// xterm's `prepareTextForTerminal` then `bracketTextForPaste` (TERM-035):
/// `\r\n` and `\n` become `\r`, and while the program has bracketed paste on
/// (mode 2004) the text goes between `\x1b[200~` and `\x1b[201~`, even when
/// it is empty (an image-only clipboard pastes empty brackets, which Claude
/// Code reacts to).
pub fn prepare(text: &str, bracketed: bool) -> String {
    let mut out = String::with_capacity(text.len() + 12);
    if bracketed {
        out.push_str("\x1b[200~");
    }
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                out.push('\r');
            }
            '\n' => out.push('\r'),
            c => out.push(c),
        }
    }
    if bracketed {
        out.push_str("\x1b[201~");
    }
    out
}

/// registry.ts `shellQuote` (TERM-033): a path made only of `[\w@%+=:,./-]`
/// (ASCII word characters, as JavaScript's `\w`) stays as it is; anything else
/// goes in single quotes, each `'` written `'\''`.
pub fn shell_quote(path: &str) -> String {
    let safe = !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b));
    if safe {
        return path.to_owned();
    }
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// What dropping files on a terminal pastes (TERM-032): each path quoted,
/// joined by spaces, with a trailing space. Empty paths (files the OS gave no
/// path for) are left out; with none left there is nothing to paste.
pub fn dropped_paths<S: AsRef<str>>(paths: &[S]) -> Option<String> {
    let quoted: Vec<String> = paths
        .iter()
        .map(AsRef::as_ref)
        .filter(|p| !p.is_empty())
        .map(shell_quote)
        .collect();
    if quoted.is_empty() {
        return None;
    }
    Some(quoted.join(" ") + " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_035_line_breaks_become_cr() {
        assert_eq!(prepare("a\nb\r\nc\rd", false), "a\rb\rc\rd");
        assert_eq!(prepare("\r\n\n", false), "\r\r");
    }

    #[test]
    fn term_035_bracketed_when_mode_2004_is_on_even_empty() {
        assert_eq!(prepare("hi\nthere", true), "\x1b[200~hi\rthere\x1b[201~");
        assert_eq!(prepare("", true), "\x1b[200~\x1b[201~");
        assert_eq!(prepare("", false), "");
    }

    #[test]
    fn term_033_shell_quote() {
        assert_eq!(shell_quote("/Users/example/file.txt"), "/Users/example/file.txt");
        assert_eq!(shell_quote("a-b_c@d%e+f=g:h,i.j/k"), "a-b_c@d%e+f=g:h,i.j/k");
        assert_eq!(shell_quote("/tmp/my file.png"), "'/tmp/my file.png'");
        assert_eq!(shell_quote("it's here"), "'it'\\''s here'");
        // JavaScript's \w is ASCII only.
        assert_eq!(shell_quote("/tmp/é.txt"), "'/tmp/é.txt'");
        assert_eq!(
            shell_quote(r"C:\Users\example\file.txt"),
            r"'C:\Users\example\file.txt'"
        );
    }

    #[test]
    fn term_032_dropped_paths_are_quoted_and_spaced() {
        assert_eq!(
            dropped_paths(&["/a/b.png", "/c d/e.txt"]).as_deref(),
            Some("/a/b.png '/c d/e.txt' ")
        );
        assert_eq!(dropped_paths(&["", "/x"]).as_deref(), Some("/x "));
        assert_eq!(dropped_paths::<&str>(&[]), None);
        assert_eq!(dropped_paths(&[""]), None);
    }
}
