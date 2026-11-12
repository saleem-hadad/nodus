//! Claude's guess at what a floating capture is and where it goes. A port of
//! `src/main/captureGuess.ts`.
//!
//! A quick headless claude call, like naming a section: Haiku with thinking,
//! tools and MCP servers off, outside any repository. It reads the projects,
//! their recent sessions, the app in front and what was typed, and answers in
//! a JSON schema. The panel asks again each time the typing pauses, and a new
//! question stops the one before it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nodus_domain::js;
use nodus_domain::json::number_to_string;
use nodus_domain::{CaptureAttachment, CaptureGuess, CaptureKind, CaptureTargets, SessionStatus};
use nodus_platform::ClaudeNotFound;
use nodus_platform::env::EnvMap;
use nodus_platform::process::Capture;
use nodus_platform::runner::{LocalRunner, Program, RunRequest, Runner};
use serde_json::Value;
use tokio::sync::Notify;

/// A guess that hasn't finished by then is stopped, and is no guess (CAP-070).
pub const TIMEOUT: Duration = Duration::from_millis(30_000);
/// How much of the text Claude reads, in UTF-16 units (CAP-075).
pub const MAX_TEXT_CHARS: usize = 4000;
/// Recent sessions listed for each project (CAP-075).
pub const SESSIONS_PER_PROJECT: usize = 8;
/// The reason shown to the user is cut to this (CAP-077).
pub const MAX_REASON_CHARS: usize = 120;

/// The system prompt, verbatim.
pub const SYSTEM_PROMPT: &str = "You route quick captures on Nodus, a board of Claude Code sessions for each of the user's projects. In another app, the user opened a small panel and typed something. Decide what it becomes and which project it goes to:
- session: a new Claude Code session in the project's repository that does what the text asks. The default for work to be done.
- into: the text is sent to a session that already exists, as the user's next message to it. Only when the text clearly carries on that session's work: it follows up on it (\"also…\", \"and add tests for it\"), replies to it, or names it. Never a session that is waiting for the user.
- note: a sticky note on the project's board, and nothing runs. For ideas, reminders and things to do later or to ask someone, not work to start now (\"remember to…\", \"idea:\", \"todo:\", \"ask finance about…\").
- view: a live page on the board that Claude builds and keeps up to date. Only when the text asks for something to watch or show: a dashboard, a table, a chart, a list or the status of something.

Take the project the text names (a project, a repository, a file or one of its sessions), else the one the browser tab or the app in front points to, else the default project.

Answer with kind, project (an id from the list), session (an id from the list, only for into; otherwise \"\"), and reason: one short sentence for the user, under 80 characters, saying why. For example: \"It follows on from “Fix checkout total rounding”.\" or \"It's an idea to keep, not work to start now.\"";

/// `JSON.stringify(SCHEMA)`: what Claude answers in.
pub const SCHEMA: &str = r#"{"type":"object","properties":{"kind":{"type":"string","enum":["session","into","note","view"]},"project":{"type":"string"},"session":{"type":"string"},"reason":{"type":"string"}},"required":["kind","project","session","reason"],"additionalProperties":false}"#;

/// `JSON.stringify({ alwaysThinkingEnabled: false })`: thinking off.
pub const SETTINGS: &str = r#"{"alwaysThinkingEnabled":false}"#;

/// What the guess goes on besides the text.
#[derive(Debug, Clone, Copy)]
pub struct GuessInput<'a> {
    pub text: &'a str,
    pub attachments: &'a [CaptureAttachment],
    /// The app in front when the panel opened.
    pub app: Option<&'a str>,
    pub targets: &'a CaptureTargets,
}

/// How long ago, for the prompt: `just now`, `25 min ago`, `3 h ago` (under
/// 48 hours), `4 days ago` (CAP-076).
pub fn ago(ms: f64) -> String {
    let min = js::round(ms / 60_000.0);
    if min < 1.0 {
        return "just now".into();
    }
    if min < 60.0 {
        return format!("{} min ago", number_to_string(min));
    }
    let h = js::round(min / 60.0);
    if h < 48.0 {
        format!("{} h ago", number_to_string(h))
    } else {
        format!("{} days ago", number_to_string(js::round(h / 24.0)))
    }
}

/// On one line: every run of spaces is one space, trimmed, and cut to `max`
/// UTF-16 units with `…` (CAP-077).
pub fn flat(s: &str, max: usize) -> String {
    let collapsed = js::collapse_spaces(s);
    let one = js::trim(&collapsed);
    if js::utf16_len(one) > max {
        format!("{}…", js::slice_utf16(one, 0, max.saturating_sub(1)))
    } else {
        one.to_owned()
    }
}

/// How a session is doing, in words (CAP-078).
pub fn status_word(status: SessionStatus, live: bool) -> &'static str {
    match status {
        SessionStatus::Waiting => "waiting for the user",
        SessionStatus::Working => "working",
        SessionStatus::Done if live => "idle",
        SessionStatus::Done => "stopped",
    }
}

/// The question Claude is asked (CAP-075, CAP-079): the projects with their
/// recent sessions, what came from the screen, and the text.
pub fn build_guess_prompt(input: &GuessInput<'_>, now: f64) -> String {
    let targets = input.targets;
    let fallback = targets
        .projects
        .iter()
        .find(|p| Some(&p.id) == targets.default_project_id.as_ref());
    let projects: Vec<String> = targets
        .projects
        .iter()
        .map(|p| {
            let sessions: Vec<String> = targets
                .sessions
                .iter()
                .filter(|s| s.project_id == p.id)
                .take(SESSIONS_PER_PROJECT)
                .map(|s| {
                    format!(
                        "  - session {}: “{}”, {}, active {}",
                        s.session_id,
                        flat(&s.title, 80),
                        status_word(s.status, s.live),
                        ago(now - s.updated_at)
                    )
                })
                .collect();
            let mut lines = vec![format!("- project {}: {}", p.id, p.name)];
            if sessions.is_empty() {
                lines.push("  (no sessions)".into());
            } else {
                lines.extend(sessions);
            }
            lines.join("\n")
        })
        .collect();

    let mut screen = Vec::new();
    if let Some(app) = input.app {
        screen.push(format!("The app in front: {app}"));
    }
    for attachment in input.attachments {
        screen.push(match attachment {
            CaptureAttachment::Tab(tab) => format!("Its browser tab: {} ({})", flat(&tab.title, 120), tab.url),
            CaptureAttachment::Image(image) if image.copied == Some(true) => {
                "An image the user copied is attached.".into()
            }
            CaptureAttachment::Image(_) => "A screenshot is attached.".into(),
            CaptureAttachment::File(file) => format!("A file is attached: {}", file.name),
            CaptureAttachment::Text(text) => format!("Copied text is attached: {}", flat(&text.text, 200)),
        });
    }

    let header = match fallback {
        Some(p) => format!("Projects (default: {}, {}):", p.name, p.id),
        None => "Projects:".into(),
    };
    let screen = if screen.is_empty() {
        String::new()
    } else {
        format!("\n{}", screen.join("\n"))
    };
    let text = js::slice_utf16(js::trim(input.text), 0, MAX_TEXT_CHARS);
    [
        header,
        projects.join("\n"),
        screen,
        format!("\nThe capture:\n\"\"\"\n{text}\n\"\"\""),
    ]
    .join("\n")
}

/// The arguments after `claude`, verbatim (CAP-069).
pub fn guess_args(input: &GuessInput<'_>, now: f64) -> Vec<String> {
    [
        "-p",
        "--model",
        "haiku",
        "--settings",
        SETTINGS,
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--json-schema",
        SCHEMA,
        "--output-format",
        "json",
        "--system-prompt",
        SYSTEM_PROMPT,
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([build_guess_prompt(input, now)])
    .collect()
}

/// Claude's answer, checked against what exists: a project it made up falls
/// back to the default, and "into" a session that isn't there, or is waiting
/// on the user, becomes a new session (CAP-080).
pub fn read_guess(raw: &Value, targets: &CaptureTargets) -> Option<CaptureGuess> {
    // `!raw || typeof raw !== 'object'`: an array is an object with none of the fields.
    let fields = match raw {
        Value::Object(fields) => Some(fields),
        Value::Array(_) => None,
        _ => return None,
    };
    let field = |name: &str| fields.and_then(|f| f.get(name));
    let named = field("project")
        .and_then(Value::as_str)
        .filter(|id| targets.projects.iter().any(|p| p.id == *id));
    let project_id = named
        .map(str::to_owned)
        .or_else(|| targets.default_project_id.clone())
        .filter(|id| !id.is_empty())?;
    let mut kind = field("kind")
        .and_then(Value::as_str)
        .and_then(CaptureKind::parse)
        .unwrap_or(CaptureKind::Session);
    let mut session = None;
    if kind == CaptureKind::Into {
        let target = field("session")
            .and_then(Value::as_str)
            .and_then(|id| targets.sessions.iter().find(|s| s.session_id == id));
        match target {
            Some(s) if s.status != SessionStatus::Waiting => session = Some(s),
            _ => kind = CaptureKind::Session,
        }
    }
    let reason = field("reason")
        .and_then(Value::as_str)
        .map(|r| flat(r, MAX_REASON_CHARS))
        .unwrap_or_default();
    Some(CaptureGuess {
        kind,
        // A session belongs to its own project, whatever project was named.
        project_id: session.map_or(project_id, |s| s.project_id.clone()),
        session_id: session.map(|s| s.session_id.clone()),
        reason,
    })
}

/// JavaScript's truthiness, for `parsed.is_error`.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// What `claude -p` printed, read as the guess (CAP-072): JSON, not an
/// error, with the schema's answer in `structured_output`.
pub fn read_reply(stdout: &str, targets: &CaptureTargets) -> Option<CaptureGuess> {
    let parsed: Value = nodus_domain::json::from_str(js::trim(stdout)).ok()?;
    // `null.is_error` throws, which is no guess too.
    if parsed.is_null() || truthy(parsed.get("is_error")) {
        return None;
    }
    read_guess(parsed.get("structured_output").unwrap_or(&Value::Null), targets)
}

/// Where `claude` is, the environment it runs in (domain 01's
/// `requireClaude` and `spawnEnv`) and what runs it (SRV-005).
pub trait ClaudeEnv: Send + Sync {
    /// The `claude` to run, or why there's none.
    fn claude(&self) -> Result<Program, ClaudeNotFound>;
    /// The login-shell environment claude processes start in.
    fn spawn_env(&self) -> EnvMap;
    /// What starts it.
    fn runner(&self) -> Arc<dyn Runner>;
}

/// The `claude` this environment found, started here.
impl ClaudeEnv for nodus_platform::Env {
    fn claude(&self) -> Result<Program, ClaudeNotFound> {
        LocalRunner.claude(self)
    }

    fn spawn_env(&self) -> EnvMap {
        nodus_platform::Env::spawn_env(self)
    }

    fn runner(&self) -> Arc<dyn Runner> {
        Arc::new(LocalRunner)
    }
}

/// The guess running now, which a new one or the panel hiding stops.
struct Running {
    id: u64,
    cancelled: Arc<Notify>,
}

/// Asks Claude for its guesses, one at a time (CAP-068…CAP-074, CAP-176).
pub struct Guesser {
    env: Arc<dyn ClaudeEnv>,
    running: Mutex<Option<Running>>,
    next_id: AtomicU64,
    timeout: Duration,
}

impl Guesser {
    pub fn new(env: Arc<dyn ClaudeEnv>) -> Self {
        Self {
            env,
            running: Mutex::new(None),
            next_id: AtomicU64::new(0),
            timeout: TIMEOUT,
        }
    }

    /// The same, giving up after `timeout` rather than 30 s (tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `cancelGuess`: stops a guess still in progress (SIGKILL, as its run
    /// is dropped); it resolves `None` at once (CAP-074). Called before every
    /// new guess and whenever the panel hides.
    pub fn cancel(&self) {
        let Some(running) = self.lock().take() else {
            return;
        };
        running.cancelled.notify_one();
    }

    /// Whether a guess is running.
    pub fn is_running(&self) -> bool {
        self.lock().is_some()
    }

    /// `guessCapture`: asks Claude, `None` if it can't say or a newer guess
    /// (or the panel hiding) stopped this one. Fails only when Claude Code
    /// isn't installed (CAP-068).
    pub async fn guess(&self, input: GuessInput<'_>, now: f64) -> Result<Option<CaptureGuess>, ClaudeNotFound> {
        self.cancel();
        if js::trim(input.text).is_empty() || input.targets.projects.is_empty() {
            return Ok(None);
        }
        let stdout = self.ask(guess_args(&input, now)).await?;
        Ok(stdout.and_then(|stdout| read_reply(&stdout, input.targets)))
    }

    /// Runs `claude` with `args` and gives what it printed: `None` when it
    /// couldn't start, ran out of time, or a newer question (or [`cancel`](Self::cancel))
    /// stopped it. Fails only when Claude Code isn't installed.
    pub async fn ask(&self, args: Vec<String>) -> Result<Option<String>, ClaudeNotFound> {
        self.cancel();
        let claude = self.env.claude()?;
        // CAP-069: stdin closed, or `claude -p` waits for piped input.
        let request = RunRequest::new(claude)
            .args(args)
            .cwd(std::env::temp_dir())
            .env(self.env.spawn_env())
            .stdout(Capture::All)
            .stderr(Capture::First(0));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(Notify::new());
        *self.lock() = Some(Running {
            id,
            cancelled: cancelled.clone(),
        });
        // A run that's dropped is stopped with SIGKILL, with what it started.
        let answer = tokio::select! {
            output = self.env.runner().run(request) => match output {
                Ok(output) => Some(output.stdout_text()),
                Err(err) => {
                    // CAP-071
                    tracing::error!(%err, "[capture] guess failed");
                    None
                }
            },
            // CAP-070
            () = tokio::time::sleep(self.timeout) => None,
            // CAP-074
            () = cancelled.notified() => None,
        };
        // CAP-073: it's no longer the running guess, if it still was.
        let mut running = self.lock();
        if running.as_ref().is_some_and(|r| r.id == id) {
            *running = None;
        }
        Ok(answer)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
