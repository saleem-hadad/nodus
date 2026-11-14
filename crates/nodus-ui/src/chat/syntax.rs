//! Colours for the code in Claude's replies (CHAT-047): a small lexer that
//! knows the comments, strings, keywords and calls of the common languages,
//! coloured in the chat's palette rather than by a full grammar. A block
//! without a language, or in one it doesn't know, stays plain: that's
//! usually a command's output or prose, where keywords would be noise.

use std::ops::Range;
use std::sync::{Arc, OnceLock};

use gpui::base::text::CodeBlock;
use gpui::{FontStyle, HighlightStyle, Rgba};

use crate::theme::tokens::Tokens;

/// What a span of code is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Keyword,
    Str,
    Call,
    Comment,
    Added,
    Removed,
}

const KEYWORDS: &[&str] = &[
    "and",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "crate",
    "def",
    "default",
    "defer",
    "del",
    "do",
    "done",
    "dyn",
    "elif",
    "else",
    "enum",
    "esac",
    "except",
    "export",
    "extends",
    "extern",
    "false",
    "False",
    "fi",
    "final",
    "finally",
    "fn",
    "for",
    "from",
    "func",
    "function",
    "go",
    "if",
    "impl",
    "implements",
    "import",
    "in",
    "interface",
    "is",
    "lambda",
    "let",
    "local",
    "loop",
    "match",
    "mod",
    "module",
    "move",
    "mut",
    "new",
    "nil",
    "None",
    "not",
    "null",
    "or",
    "override",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "raise",
    "readonly",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "switch",
    "then",
    "this",
    "throw",
    "throws",
    "trait",
    "true",
    "True",
    "try",
    "type",
    "typeof",
    "undefined",
    "unsafe",
    "use",
    "val",
    "var",
    "void",
    "when",
    "where",
    "while",
    "with",
    "yield",
];

const SQL_KEYWORDS: &[&str] = &[
    "alter",
    "and",
    "as",
    "asc",
    "by",
    "create",
    "default",
    "delete",
    "desc",
    "distinct",
    "drop",
    "from",
    "group",
    "having",
    "in",
    "index",
    "inner",
    "insert",
    "into",
    "is",
    "join",
    "key",
    "left",
    "limit",
    "not",
    "null",
    "on",
    "or",
    "order",
    "outer",
    "primary",
    "references",
    "right",
    "select",
    "set",
    "table",
    "union",
    "update",
    "values",
    "where",
];

/// How a language writes its comments and strings.
struct Lang {
    slash: bool,
    hash: bool,
    dash: bool,
    /// `'` opens a string (not in Rust, where it's mostly a lifetime).
    quote: bool,
    sql: bool,
}

fn lang(name: &str) -> Option<Lang> {
    let name = name.to_ascii_lowercase();
    let c_like = Lang {
        slash: true,
        hash: false,
        dash: false,
        quote: true,
        sql: false,
    };
    let hashed = Lang {
        slash: false,
        hash: true,
        dash: false,
        quote: true,
        sql: false,
    };
    Some(match name.as_str() {
        "rust" | "rs" => Lang { quote: false, ..c_like },
        "js" | "javascript" | "jsx" | "mjs" | "ts" | "typescript" | "tsx" | "go" | "golang" | "java" | "kotlin"
        | "kt" | "swift" | "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "cs" | "csharp" | "php" | "scala" | "dart"
        | "zig" | "json" | "jsonc" | "json5" | "css" | "scss" | "less" | "graphql" | "proto" | "groovy" => c_like,
        "python" | "py" | "sh" | "bash" | "zsh" | "shell" | "fish" | "toml" | "yaml" | "yml" | "ruby" | "rb" | "r"
        | "perl" | "dockerfile" | "makefile" | "make" | "ini" | "nix" | "elixir" | "ex" | "exs" | "powershell"
        | "ps1" => hashed,
        "sql" => Lang {
            slash: false,
            dash: true,
            sql: true,
            ..hashed
        },
        "lua" | "haskell" | "hs" | "elm" => Lang {
            hash: false,
            dash: true,
            ..hashed
        },
        _ => return None,
    })
}

/// The spans of `code`, written in `language`, worth colouring.
pub(crate) fn spans(code: &str, language: &str) -> Vec<(Range<usize>, Kind)> {
    if matches!(language.to_ascii_lowercase().as_str(), "diff" | "patch") {
        return diff_spans(code);
    }
    let Some(lang) = lang(language) else {
        return Vec::new();
    };
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let rest = &code[i..];
        let after_space = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let line_end = |from: usize| code[from..].find('\n').map_or(code.len(), |n| from + n);
        if (lang.slash && rest.starts_with("//"))
            || (lang.hash && c == b'#' && after_space)
            || (lang.dash && rest.starts_with("--"))
        {
            let end = line_end(i);
            out.push((i..end, Kind::Comment));
            i = end;
        } else if lang.slash && rest.starts_with("/*") {
            let end = rest[2..].find("*/").map_or(code.len(), |n| i + n + 4);
            out.push((i..end, Kind::Comment));
            i = end;
        } else if c == b'"' || c == b'`' || (c == b'\'' && (lang.quote || is_char_literal(rest))) {
            let end = string_end(code, i);
            out.push((i..end, Kind::Str));
            i = end;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &code[start..i];
            let keyword = if lang.sql {
                SQL_KEYWORDS.contains(&word.to_ascii_lowercase().as_str())
            } else {
                KEYWORDS.contains(&word)
            };
            // A call or a definition: `name(`, or a macro's `name!(`.
            let next = &code[i..];
            let call = next.starts_with('(') || (next.starts_with('!') && next[1..].starts_with(['(', '[', '{']));
            if keyword {
                out.push((start..i, Kind::Keyword));
            } else if call {
                out.push((start..i, Kind::Call));
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Whether `rest` (starting at a `'`) is a Rust char literal: `'x'` or an
/// escape, not a lifetime.
fn is_char_literal(rest: &str) -> bool {
    let mut chars = rest.chars().skip(1);
    match chars.next() {
        Some('\\') => true,
        Some(_) => chars.next() == Some('\''),
        None => false,
    }
}

/// Where the string opened at `start` ends: after its closing quote, at the
/// line's end for an unclosed `"` or `'`, or at the code's end for a `` ` ``.
fn string_end(code: &str, start: usize) -> usize {
    let bytes = code.as_bytes();
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' if quote != b'`' => return i,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    code.len()
}

/// A diff's added and removed lines, and its hunk headers.
fn diff_spans(code: &str) -> Vec<(Range<usize>, Kind)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        let end = at + line.trim_end_matches('\n').len();
        let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            Some(Kind::Comment)
        } else if line.starts_with('+') {
            Some(Kind::Added)
        } else if line.starts_with('-') {
            Some(Kind::Removed)
        } else {
            None
        };
        if let Some(kind) = kind {
            out.push((at..end, kind));
        }
        at += line.len();
    }
    out
}

type Highlighter = Arc<dyn Fn(&CodeBlock) -> Vec<(Range<usize>, HighlightStyle)> + Send + Sync>;

/// The code blocks' highlighter in `t`'s colours: keywords purple, strings
/// green, calls in the accent, comments faint and italic, a diff's lines
/// green and red. One per theme, so blocks keep their highlights from frame
/// to frame.
pub(crate) fn highlighter(t: &Tokens) -> Highlighter {
    static LIGHT: OnceLock<Highlighter> = OnceLock::new();
    static DARK: OnceLock<Highlighter> = OnceLock::new();
    let cell = if t.dark { &DARK } else { &LIGHT };
    let (keyword, string, call, comment, added, removed) = (
        super::look::purple(t),
        t.diff_add,
        t.accent,
        t.text_tertiary,
        t.diff_add,
        t.diff_del,
    );
    cell.get_or_init(move || {
        Arc::new(move |block: &CodeBlock| {
            let code = block.code();
            let lang = block.lang().unwrap_or_default();
            spans(&code, &lang)
                .into_iter()
                .map(|(range, kind)| {
                    let color = match kind {
                        Kind::Keyword => keyword,
                        Kind::Str => string,
                        Kind::Call => call,
                        Kind::Comment => comment,
                        Kind::Added => added,
                        Kind::Removed => removed,
                    };
                    // A highlight keeps the text's alpha, so a faint colour goes on
                    // opaque and fades out to its own alpha.
                    let style = HighlightStyle {
                        color: Some(Rgba { a: 1., ..color }.into()),
                        fade_out: (color.a < 1.).then_some(1. - color.a),
                        font_style: (kind == Kind::Comment).then_some(FontStyle::Italic),
                        ..HighlightStyle::default()
                    };
                    (range, style)
                })
                .collect()
        })
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds<'a>(code: &'a str, lang: &str) -> Vec<(&'a str, Kind)> {
        spans(code, lang).into_iter().map(|(r, k)| (&code[r], k)).collect()
    }

    #[test]
    fn colours_rust() {
        let code = "// say hi\nfn main() { let s = \"hi\"; println!(\"{s}\"); }\nfn f<'a>(x: &'a str) -> char { 'x' }";
        let k = kinds(code, "rust");
        assert_eq!(k[0], ("// say hi", Kind::Comment));
        assert!(k.contains(&("fn", Kind::Keyword)));
        assert!(k.contains(&("main", Kind::Call)));
        assert!(k.contains(&("\"hi\"", Kind::Str)));
        assert!(k.contains(&("println", Kind::Call)));
        // A lifetime isn't a string; a char literal is.
        assert!(k.contains(&("'x'", Kind::Str)));
        assert!(!k.iter().any(|(s, _)| s.starts_with("'a")));
    }

    #[test]
    fn knows_each_languages_comments() {
        assert_eq!(kinds("x = 1  # one", "python"), vec![("# one", Kind::Comment)]);
        assert_eq!(kinds("echo ${#x}", "sh"), vec![]);
        assert_eq!(kinds("SELECT a FROM t -- all", "sql")[0], ("SELECT", Kind::Keyword));
        assert_eq!(kinds("a /* b */ c", "ts"), vec![("/* b */", Kind::Comment)]);
        // A block without a language stays plain.
        assert_eq!(kinds("if it's done, return", ""), vec![]);
    }

    #[test]
    fn colours_a_diffs_lines() {
        let code = "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n same";
        let k = kinds(code, "diff");
        assert_eq!(k[3], ("-old", Kind::Removed));
        assert_eq!(k[4], ("+new", Kind::Added));
        assert_eq!(k.len(), 5);
    }

    #[test]
    fn survives_unclosed_strings_and_wide_letters() {
        assert_eq!(kinds("s = \"open\nnext", "js"), vec![("\"open", Kind::Str)]);
        assert_eq!(
            kinds("é = 'ü' // ç", "js"),
            vec![("'ü'", Kind::Str), ("// ç", Kind::Comment)]
        );
    }
}
