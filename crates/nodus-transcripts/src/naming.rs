//! Names a new section from the sessions grouped under it (naming.ts).
//!
//! A quick headless claude call: each session's title and opening prompt go
//! in, a few words come out. Thinking, tools and MCP servers are off, and it
//! runs outside the repo, so it answers in a couple of seconds.

use std::sync::LazyLock;
use std::time::Duration;

use nodus_domain::js::{self, slice_utf16, utf16_len};
use regex::Regex;

use crate::claude::{self, ClaudeCall, RunError};

/// The call is stopped after this long (SESS-113).
pub const TIMEOUT: Duration = Duration::from_millis(30_000);
const MAX_PROMPT_CHARS: usize = 280;
const MAX_NAME_CHARS: usize = 48;

pub const SYSTEM_PROMPT: &str = "You name groups of Claude Code sessions on a board. Reply with the group's name only: 1 to 4 words in Title Case that say what the sessions have in common. No quotes, no trailing punctuation, no preamble.";

/// JavaScript's `\s`, for patterns.
const SPACE: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

static LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("(?i)^(?:name|title){SPACE}*:{SPACE}*")).expect("a valid pattern"));
static MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new("[*_`#\"“”]").expect("a valid pattern"));
static QUOTES: LazyLock<Regex> = LazyLock::new(|| Regex::new("^'+|'+$").expect("a valid pattern"));
static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new("[.!?:;,]+$").expect("a valid pattern"));

/// A session the section is named after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamingSource {
    pub title: String,
    pub first_prompt: Option<String>,
}

/// Why naming failed, as the user is told.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NamingError(pub String);

/// Spaces collapsed, and cut to `max` UTF-16 units with an ellipsis, without
/// trimming before it (SESS-111).
pub fn clip(s: &str, max: usize) -> String {
    let flat = js::collapse_spaces(s);
    let flat = js::trim(&flat);
    if utf16_len(flat) > max {
        format!("{}…", slice_utf16(flat, 0, max - 1))
    } else {
        flat.to_owned()
    }
}

/// What Claude is asked (SESS-110).
pub fn build_prompt(sources: &[NamingSource]) -> String {
    let lines: Vec<String> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let n = i + 1;
            let prompt = s
                .first_prompt
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| clip(p, MAX_PROMPT_CHARS))
                .filter(|p| !p.is_empty() && *p != s.title);
            match prompt {
                Some(prompt) => format!("{n}. {}\n   Opening prompt: {prompt}", s.title),
                None => format!("{n}. {}", s.title),
            }
        })
        .collect();
    format!("Name this group of sessions:\n\n{}", lines.join("\n"))
}

/// The model's reply as a name: first line, without quotes, markdown or a
/// trailing full stop (SESS-112).
pub fn clean_name(reply: &str) -> String {
    let line = js::trim(reply).split('\n').next().unwrap_or_default();
    let name = LABEL.replace(line, "");
    let name = MARKS.replace_all(&name, "");
    let name = QUOTES.replace_all(&name, "");
    let name = TRAILING.replace(&name, "");
    let name = js::collapse_spaces(&name);
    let name = js::trim(&name);
    if utf16_len(name) > MAX_NAME_CHARS {
        js::trim_end(&slice_utf16(name, 0, MAX_NAME_CHARS)).to_owned()
    } else {
        name.to_owned()
    }
}

/// The arguments after `claude`: 14 of them, the empty one after `--tools`
/// included (SESS-109).
pub fn naming_args(sources: &[NamingSource]) -> Vec<String> {
    let settings = nodus_domain::json::to_string(&serde_json::json!({ "alwaysThinkingEnabled": false }))
        .expect("settings serialize");
    [
        "-p",
        "--model",
        "haiku",
        "--settings",
        &settings,
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--system-prompt",
        SYSTEM_PROMPT,
        "--output-format",
        "json",
        &build_prompt(sources),
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Asks `claude` to name a section after `sources`, in the OS temp folder
/// (SESS-109…113). The caller finds `claude`.
pub async fn name_section(claude: ClaudeCall<'_>, sources: &[NamingSource]) -> Result<String, NamingError> {
    let args = naming_args(sources);
    let finished = match claude::run(claude, &args, &std::env::temp_dir(), TIMEOUT).await {
        Ok(finished) => finished,
        Err(RunError::TimedOut) => return Err(NamingError("Naming timed out".into())),
        Err(RunError::Spawn(message)) => return Err(NamingError(message)),
    };
    let parsed = claude::parse_reply(&finished.stdout);
    let name = claude::result_of(parsed.as_ref()).map(clean_name).unwrap_or_default();
    if !name.is_empty() {
        return Ok(name);
    }
    Err(NamingError(claude::failure(parsed.as_ref(), &finished)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(title: &str, first_prompt: Option<&str>) -> NamingSource {
        NamingSource {
            title: title.into(),
            first_prompt: first_prompt.map(Into::into),
        }
    }

    #[test]
    fn asks_with_each_title_and_a_distinct_opening_prompt() {
        let prompt = build_prompt(&[
            source(
                "Fix login",
                Some("The login redirect loops.\n  Check the auth middleware please"),
            ),
            source("Write the README", Some("Write the README")),
            source("Tidy", Some("")),
        ]);
        assert_eq!(
            prompt,
            "Name this group of sessions:\n\n1. Fix login\n   Opening prompt: The login redirect loops. Check the auth middleware please\n2. Write the README\n3. Tidy"
        );
    }

    #[test]
    fn clips_in_utf16_units_without_trimming() {
        assert_eq!(clip("ab cdef", 4), "ab …");
        assert_eq!(clip("  a\n\nb  ", 10), "a b");
        assert_eq!(clip("😀😀", 3), "😀…");
        assert_eq!(utf16_len(&clip(&"x".repeat(300), 280)), 280);
    }

    #[test]
    fn cleans_the_reply_into_a_name() {
        assert_eq!(clean_name("Auth Fixes"), "Auth Fixes");
        assert_eq!(clean_name("  Name: **Auth Fixes**.\nBecause…"), "Auth Fixes");
        assert_eq!(clean_name("title :  `Login`!?"), "Login");
        assert_eq!(clean_name("'Docs'"), "Docs");
        assert_eq!(clean_name("“Release Prep”"), "Release Prep");
        assert_eq!(clean_name("A.B.."), "A.B");
        assert_eq!(clean_name("Big   \t Refactor"), "Big Refactor");
        assert_eq!(clean_name("'''"), "");
        assert_eq!(clean_name(&format!("{} more", "w".repeat(47))), "w".repeat(47));
        assert_eq!(clean_name("Names: x"), "Names: x");
    }

    #[test]
    fn passes_fourteen_arguments() {
        let args = naming_args(&[source("T", None)]);
        assert_eq!(args.len(), 14);
        assert_eq!(
            &args[..7],
            [
                "-p",
                "--model",
                "haiku",
                "--settings",
                r#"{"alwaysThinkingEnabled":false}"#,
                "--tools",
                ""
            ]
        );
        assert_eq!(
            &args[7..13],
            [
                "--strict-mcp-config",
                "--no-session-persistence",
                "--system-prompt",
                SYSTEM_PROMPT,
                "--output-format",
                "json"
            ]
        );
        assert_eq!(args[13], "Name this group of sessions:\n\n1. T");
    }

    #[cfg(unix)]
    mod with_a_stand_in {
        use std::path::Path;

        use nodus_platform::env::EnvMap;
        use nodus_platform::runner::{LocalRunner, Program};

        use super::*;
        use crate::claude::stand_in;

        async fn name_with(claude: &Path, sources: &[NamingSource]) -> Result<String, NamingError> {
            let call = ClaudeCall {
                runner: &LocalRunner,
                program: &Program::from(claude),
                env: &EnvMap::new(),
            };
            name_section(call, sources).await
        }

        #[tokio::test]
        async fn names_the_section_from_claudes_reply() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(
                dir.path(),
                r#"echo '{"type":"result","is_error":false,"result":"Name: Auth Work."}'"#,
            );
            let sources = [source("Fix login", None)];
            let name = name_with(&claude, &sources).await;
            assert_eq!(name, Ok("Auth Work".into()));
            assert_eq!(stand_in::calls(dir.path()), [naming_args(&sources)]);
            let cwd = std::fs::read_to_string(dir.path().join("cwd.log")).unwrap();
            let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
            assert_eq!(std::fs::canonicalize(cwd.trim()).unwrap(), tmp);
        }

        #[tokio::test]
        async fn says_why_it_couldnt() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(dir.path(), r#"echo '{"result":"***"}'; echo 'warn' >&2; exit 1"#);
            let sources = [source("T", None)];
            // A reply that cleans to nothing gives its own text as the reason.
            assert_eq!(name_with(&claude, &sources).await, Err(NamingError("***".into())));
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(dir.path(), "exit 7");
            assert_eq!(
                name_with(&claude, &sources).await,
                Err(NamingError("claude exited with code 7".into()))
            );
        }

        #[tokio::test(start_paused = true)]
        async fn times_out_after_30_seconds() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(dir.path(), "sleep 600");
            let result = name_with(&claude, &[source("T", None)]).await;
            assert_eq!(result, Err(NamingError("Naming timed out".into())));
        }
    }
}
