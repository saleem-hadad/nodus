//! Claude drafting an action from what the user describes (`draftAction`,
//! ACT-019…ACT-027): a quick headless `claude -p` on Sonnet that may read the
//! repository (and only read it) to fit the prompt to it, answering in JSON.

use std::path::Path;
use std::time::Duration;

use nodus_domain::js::trim;
use nodus_domain::{ActionDraft, ActionIcon};
use nodus_platform::env::EnvMap;
use nodus_platform::process::{Signal, SpawnError};
use nodus_platform::runner::{Program, RunRequest, Runner};
use nodus_platform::{ClaudeNotFound, Env};
use serde_json::Value;

use crate::normalize::{MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_PROMPT_CHARS, clip, icon_or_default};

/// How long a draft may take before it's stopped (ACT-024).
pub const DRAFT_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// What the drafting session is told about Nodus and the answer it gives
/// (`--append-system-prompt`, verbatim).
pub const DRAFT_SYSTEM_PROMPT: &str = "You design action buttons for Nodus, a board of Claude Code sessions for one project. Pressing a button starts a fresh Claude Code session in the project's repository on the button's prompt. Nobody watches the session while it works, and it goes away when its turn ends, so the prompt has to stand on its own. It runs in auto mode: Claude Code approves routine steps by itself and blocks risky ones.

Besides the usual tools (reading and editing files, running commands), a run has these board tools:
- list_sessions: the board's sessions, with their status (working, waiting for the user, or done), whether they are archived, their section, Claude's last reply and when they were last active.
- archive_sessions: archives sessions by id. It skips ones that are working or waiting unless forced.
- create_note, update_note and list_notes: write sticky notes on the board, change them, and read the notes there. A note is short plain text, which suits a report the user should find on the board. They take an optional project, from list_projects, to use another project's board.

From the user's description, write:
- name: 1 to 3 words in Title Case, for the button.
- icon: the best fit from the list.
- prompt: the run's instructions, addressed to the session (\"you\"). Say what to do, step by step: what to check first, when to stop without changing anything, and how to handle the likely failures. If the run may need the user to decide something, tell it to ask with AskUserQuestion. Don't add requirements the user didn't ask for.

You can read the repository to fit the prompt to it (for example, to find the real test command), but be quick about it.";

/// The global button's note to the drafter (ACT-022).
const GLOBAL_NOTE: &str = "The button is global: it shows in every project, so the prompt has to work in any repository. Don't rely on this one's commands, paths or tools; tell the run how to find them.";

/// `JSON.stringify(DRAFT_SCHEMA)`: the shape of the answer, with the icons to
/// pick from in `ACTION_ICONS` order.
pub fn draft_schema_json() -> String {
    let icons: Vec<String> = ActionIcon::ALL.iter().map(|i| format!("\"{i}\"")).collect();
    format!(
        r#"{{"type":"object","properties":{{"name":{{"type":"string"}},"icon":{{"type":"string","enum":[{}]}},"prompt":{{"type":"string"}}}},"required":["name","icon","prompt"],"additionalProperties":false}}"#,
        icons.join(",")
    )
}

/// ACT-022: the drafter's prompt, for a description already clipped.
pub fn draft_prompt(description: &str, global: bool) -> String {
    if global {
        format!("Draft a button that does this:\n\n{description}\n\n{GLOBAL_NOTE}")
    } else {
        format!("Draft a button that does this:\n\n{description}")
    }
}

/// ACT-021: `claude`'s arguments, each its own argv element.
pub fn draft_args(description: &str, global: bool) -> Vec<String> {
    let read_only = "Read,Glob,Grep";
    [
        "-p",
        "--model",
        "sonnet",
        // At the default effort Sonnet can think for over a minute before
        // answering, which runs into the timeout while the dialog says "Drafting…".
        "--effort",
        "low",
        "--tools",
        read_only,
        "--allowedTools",
        read_only,
        "--permission-mode",
        "dontAsk",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--json-schema",
    ]
    .iter()
    .map(|&s| s.to_owned())
    .chain([draft_schema_json()])
    .chain(["--output-format", "json", "--append-system-prompt", DRAFT_SYSTEM_PROMPT].map(str::to_owned))
    .chain([draft_prompt(description, global)])
    .collect()
}

/// Why a draft failed. The text is what the action dialog shows after
/// `Claude couldn’t draft it: `.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    /// ACT-019.
    #[error("Describe what the button should do")]
    NoDescription,
    /// ACT-020.
    #[error(transparent)]
    NoClaude(#[from] ClaudeNotFound),
    /// ACT-024.
    #[error("Drafting the button took too long")]
    TookTooLong,
    /// ACT-025: `spawn <path> ENOENT` and the like.
    #[error(transparent)]
    Spawn(#[from] SpawnError),
    /// ACT-027: Claude's own reason, its last words on stderr, or its exit code.
    #[error("{0}")]
    Failed(String),
}

/// JavaScript truthiness of a JSON value; `None` is `undefined`.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// ACT-026, ACT-027: what `claude -p --output-format json` printed, and how
/// it ended (`code` is `None` when a signal stopped it).
pub fn read_draft(stdout: &str, stderr: &str, code: Option<i32>) -> Result<ActionDraft, DraftError> {
    // A parse failure is no result.
    let parsed: Option<Value> = nodus_domain::json::from_str(trim(stdout)).ok();
    let object = parsed.as_ref().and_then(Value::as_object);
    let draft = object
        .filter(|o| !truthy(o.get("is_error")))
        .and_then(|o| o.get("structured_output"))
        .and_then(Value::as_object);
    if let Some(draft) = draft {
        let prompt = clip(draft.get("prompt"), MAX_PROMPT_CHARS);
        if !prompt.is_empty() {
            let name = Some(clip(draft.get("name"), MAX_NAME_CHARS))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Action".to_owned());
            return Ok(ActionDraft {
                name,
                icon: icon_or_default(draft.get("icon")),
                prompt,
            });
        }
    }
    let result = object
        .and_then(|o| o.get("result"))
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty());
    let reason = match result {
        Some(result) => result.to_owned(),
        None => {
            let lines: Vec<&str> = trim(stderr).split('\n').collect();
            let last = lines[lines.len().saturating_sub(3)..].join(" ");
            if last.is_empty() {
                let code = code.map_or_else(|| "null".to_owned(), |c| c.to_string());
                format!("claude exited with code {code}")
            } else {
                last
            }
        }
    };
    Err(DraftError::Failed(reason))
}

/// `actions:draft` once the login-shell env is ready (ACT-016): drafts in the
/// project's folder with the user's `claude`, run through `runner` (SRV-005).
pub async fn draft_action(
    runner: &dyn Runner,
    env: &Env,
    repo_path: &Path,
    description: &str,
    global: bool,
) -> Result<ActionDraft, DraftError> {
    draft_with(
        runner,
        runner.claude(env).ok(),
        env.spawn_env(),
        repo_path,
        description,
        global,
    )
    .await
}

/// [`draft_action`] with the `claude` program and `spawnEnv()` given, for
/// tests and other wiring.
pub async fn draft_with(
    runner: &dyn Runner,
    claude: Option<Program>,
    spawn_env: EnvMap,
    repo_path: &Path,
    description: &str,
    global: bool,
) -> Result<ActionDraft, DraftError> {
    // ACT-019, before looking for Claude.
    let what = clip(Some(&Value::String(description.to_owned())), MAX_DESCRIPTION_CHARS);
    if what.is_empty() {
        return Err(DraftError::NoDescription);
    }
    // ACT-020
    let claude = claude.ok_or(ClaudeNotFound)?;
    // ACT-023: stdin closed, otherwise claude -p waits for piped input; no
    // console window on Windows. ACT-024: SIGKILL at the timeout.
    let request = RunRequest::new(claude)
        .args(draft_args(&what, global))
        .cwd(repo_path)
        .env(spawn_env)
        .timeout(DRAFT_TIMEOUT)
        .kill_signal(Signal::KILL);
    let output = runner.run(request).await?;
    if output.timed_out {
        return Err(DraftError::TookTooLong);
    }
    read_draft(&output.stdout_text(), &output.stderr_text(), output.code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn asks_for_the_answer_in_the_exact_shape() {
        assert_eq!(
            draft_schema_json(),
            r#"{"type":"object","properties":{"name":{"type":"string"},"icon":{"type":"string","enum":["zap","refresh-cw","git-merge","git-pull-request","git-commit-horizontal","upload","download","archive","brush-cleaning","eraser","sparkles","wand-sparkles","test-tube","bug","rocket","package","file-text","book-open","list-checks","search","shield-check","hammer","square-terminal","gauge","lightbulb","message-square"]},"prompt":{"type":"string"}},"required":["name","icon","prompt"],"additionalProperties":false}"#
        );
        // The schema as JSON.stringify writes it.
        let reparsed: Value = serde_json::from_str(&draft_schema_json()).unwrap();
        assert_eq!(nodus_domain::json::to_string(&reparsed).unwrap(), draft_schema_json());
    }

    #[test]
    fn builds_the_draft_command_line() {
        // ACT-021, ACT-022
        let args = draft_args("Run the tests", false);
        let head: Vec<&str> = args.iter().take(14).map(String::as_str).collect();
        assert_eq!(
            head,
            [
                "-p",
                "--model",
                "sonnet",
                "--effort",
                "low",
                "--tools",
                "Read,Glob,Grep",
                "--allowedTools",
                "Read,Glob,Grep",
                "--permission-mode",
                "dontAsk",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--json-schema"
            ]
        );
        assert_eq!(args[14], draft_schema_json());
        assert_eq!(&args[15..18], ["--output-format", "json", "--append-system-prompt"]);
        assert_eq!(args[18], DRAFT_SYSTEM_PROMPT);
        assert_eq!(args[19], "Draft a button that does this:\n\nRun the tests");
        assert_eq!(args.len(), 20);
        assert_eq!(
            draft_args("x", true)[19],
            "Draft a button that does this:\n\nx\n\nThe button is global: it shows in every project, so the prompt has to work in any repository. Don't rely on this one's commands, paths or tools; tell the run how to find them."
        );
        assert!(DRAFT_SYSTEM_PROMPT.starts_with("You design action buttons for Nodus,"));
        assert!(DRAFT_SYSTEM_PROMPT.ends_with("but be quick about it."));
        assert_eq!(DRAFT_SYSTEM_PROMPT.matches("\n\n").count(), 3);
    }

    #[test]
    fn reads_claudes_answer() {
        let ok = json!({
            "is_error": false,
            "structured_output": { "name": "  Run Tests ", "icon": "test-tube", "prompt": " Run them. " }
        });
        assert_eq!(
            read_draft(&format!("\n{ok}\n"), "", Some(0)).unwrap(),
            ActionDraft {
                name: "Run Tests".into(),
                icon: "test-tube".into(),
                prompt: "Run them.".into()
            }
        );
        // ACT-026: a bad icon is zap, a missing name is Action.
        let odd = json!({ "structured_output": { "icon": "nope", "prompt": "p" } });
        let draft = read_draft(&odd.to_string(), "", Some(0)).unwrap();
        assert_eq!((draft.name.as_str(), draft.icon.as_str()), ("Action", "zap"));
    }

    #[test]
    fn says_why_a_draft_failed() {
        let reason = |stdout: &str, stderr: &str, code| read_draft(stdout, stderr, code).unwrap_err().to_string();
        // Claude's own reason first, even with a draft, when it's an error.
        let error =
            json!({ "is_error": 1, "result": "Credit balance is too low", "structured_output": { "prompt": "p" } });
        assert_eq!(reason(&error.to_string(), "x", Some(1)), "Credit balance is too low");
        // An empty prompt is no draft.
        let empty = json!({ "structured_output": { "prompt": "  " }, "result": "" });
        assert_eq!(reason(&empty.to_string(), "", Some(0)), "claude exited with code 0");
        // Then the last three lines of stderr, joined with spaces.
        assert_eq!(reason("not json", "one\ntwo\nthree\nfour\n", Some(1)), "two three four");
        assert_eq!(reason("", "  only  ", Some(2)), "only");
        // A non-string result doesn't count.
        assert_eq!(reason(r#"{"result": 5}"#, "", Some(3)), "claude exited with code 3");
        // Killed by a signal: no code.
        assert_eq!(reason("", "", None), "claude exited with code null");
        assert_eq!(reason("[1]", "", Some(0)), "claude exited with code 0");
    }
}
