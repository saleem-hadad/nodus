//! Windows command lines. A Windows process gets one command line, not an
//! argument list, so the host joins the arguments the way programs built with
//! the Microsoft C runtime split them back up, and puts the program in front
//! the way node-pty 1.1.0 does. (HOST-051, Data & formats §13)

/// JavaScript's `\s`: the characters a regex without the `u` flag counts as
/// whitespace.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Joins a program's arguments into a Windows command line, quoted the way
/// programs built with the Microsoft C runtime split it back up; node-pty puts
/// the program in front. Its own joiner leaves an argument that starts and
/// ends with a quote unquoted, so a prompt such as "fix the tests" would
/// arrive in pieces. (`windowsCommandLine` in hostProtocol.ts)
pub fn windows_command_line<S: AsRef<str>>(args: &[S]) -> String {
    args.iter().map(|arg| quote(arg.as_ref())).collect::<Vec<_>>().join(" ")
}

fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c == '"' || is_js_space(c)) {
        return arg.to_owned();
    }
    // Backslashes only need doubling before a quote, including the closing one:
    // arg.replace(/(\\*)"/g, '$1$1\\"').replace(/(\\+)$/, '$1$1').
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2));
                out.push_str("\\\"");
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The whole command line node-pty 1.1.0 hands `CreateProcessW` for `file`
/// and a command line of arguments (`argsToCommandLine(file, args)` with
/// `args` a string): the program quoted its way, a space, then the arguments;
/// with no arguments, just `file`, exactly as given.
pub fn windows_spawn_command_line(file: &str, args: &str) -> String {
    if args.is_empty() {
        return file.to_owned();
    }
    format!("{} {args}", node_pty_quote(file))
}

/// node-pty's own quoting of one argument (`argsToCommandLine(file, [])`):
/// wrapped in quotes only when it is empty, or holds a space or a tab, is
/// longer than one character and isn't already wrapped in a pair of quotes;
/// a quote inside becomes 2n+1 backslashes and the quote; when wrapped,
/// trailing backslashes are doubled.
fn node_pty_quote(arg: &str) -> String {
    let units: Vec<u16> = arg.encode_utf16().collect();
    let starts_quoted = units.first() == Some(&u16::from(b'"'));
    let ends_quoted = units.last() == Some(&u16::from(b'"'));
    let lopsided = starts_quoted != ends_quoted;
    let no_enclosing = !starts_quoted && !ends_quoted;
    let wrap =
        arg.is_empty() || ((arg.contains(' ') || arg.contains('\t')) && units.len() > 1 && (lopsided || no_enclosing));

    let mut out = String::with_capacity(arg.len() + 2);
    if wrap {
        out.push('"');
    }
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    if wrap {
        out.push_str(&"\\".repeat(backslashes * 2));
        out.push('"');
    } else {
        out.push_str(&"\\".repeat(backslashes));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits a command line back up the way the Microsoft C runtime does.
    fn split_command_line(line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        let mut args = Vec::new();
        let mut arg: Option<String> = None;
        let mut quoted = false;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let mut n = 0;
                while i < chars.len() && chars[i] == '\\' {
                    n += 1;
                    i += 1;
                }
                let current = arg.get_or_insert_with(String::new);
                if i < chars.len() && chars[i] == '"' {
                    // 2n backslashes and a quote: n backslashes, and the quote opens or closes; 2n+1: a literal quote.
                    current.push_str(&"\\".repeat(n >> 1));
                    if n % 2 == 1 {
                        current.push('"');
                    } else {
                        quoted = !quoted;
                    }
                } else {
                    current.push_str(&"\\".repeat(n));
                    i -= 1;
                }
            } else if c == '"' {
                arg.get_or_insert_with(String::new);
                quoted = !quoted;
            } else if (c == ' ' || c == '\t') && !quoted {
                if let Some(done) = arg.take() {
                    args.push(done);
                }
            } else {
                arg.get_or_insert_with(String::new).push(c);
            }
            i += 1;
        }
        if let Some(done) = arg {
            args.push(done);
        }
        args
    }

    // hostProtocol.test.ts › windowsCommandLine

    #[test]
    fn quotes_only_what_needs_it() {
        assert_eq!(
            windows_command_line(&["--resume", "abc", "two words", ""]),
            r#"--resume abc "two words" """#
        );
    }

    #[test]
    fn comes_back_as_the_same_arguments_whatever_they_hold() {
        let settings = r#"{"hooks":{"Stop":[{"command":"curl -K \"$NODUS_HOOK_CONFIG\" >/dev/null 2>&1; exit 0"}]}}"#;
        let args = [
            "--settings",
            settings,
            "--",
            "\"fix the tests\"",
            "C:\\Program Files\\Nodus\\",
            "ends in two \\\\",
            "a\\\"b",
            "tab\there",
            "line one\nline two",
            "",
            "plain",
        ];
        assert_eq!(split_command_line(&windows_command_line(&args)), args);
    }

    // Data & formats §13 vectors.

    #[test]
    fn matches_the_spec_vectors() {
        let cases = [
            ("\"fix the tests\"", r#""\"fix the tests\"""#),
            ("C:\\Program Files\\Nodus\\", r#""C:\Program Files\Nodus\\""#),
            ("ends in two \\\\", r#""ends in two \\\\""#),
            ("a\\\"b", r#""a\\\"b""#),
            ("x\u{a0}y", "\"x\u{a0}y\""),
            ("\u{feff}", "\"\u{feff}\""),
        ];
        for (arg, quoted) in cases {
            assert_eq!(windows_command_line(&[arg]), quoted, "{arg:?}");
        }
    }

    #[test]
    fn puts_the_program_in_front_as_node_pty_does() {
        assert_eq!(
            windows_spawn_command_line("C:\\Program Files\\nodejs\\claude.exe", "--resume abc"),
            r#""C:\Program Files\nodejs\claude.exe" --resume abc"#
        );
        assert_eq!(
            windows_spawn_command_line("C:\\bin\\claude.exe", "-x"),
            r"C:\bin\claude.exe -x"
        );
        // Already wrapped in quotes: left as it is.
        assert_eq!(
            windows_spawn_command_line("\"C:\\a b\\c.exe\"", "-x"),
            r#"\"C:\a b\c.exe\" -x"#
        );
        // Lopsided quotes get wrapped, and the quote inside escaped.
        assert_eq!(windows_spawn_command_line("\"C:\\a b", "-x"), r#""\"C:\a b" -x"#);
        // A trailing backslash, wrapped: doubled.
        assert_eq!(windows_spawn_command_line("C:\\a b\\", "-x"), r#""C:\a b\\" -x"#);
        // No arguments: the program exactly as given, even with a space in it.
        assert_eq!(
            windows_spawn_command_line("C:\\Program Files\\PowerShell\\7\\pwsh.exe", ""),
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe"
        );
        assert_eq!(windows_spawn_command_line("", "-x"), "\"\" -x");
    }
}
