//! How a card's process is started: the exact `claude` command line, or a
//! login shell for a terminal, and the options each way of starting one
//! (create, open, send) gives it. Pure assembly: what other domains own (the
//! hook settings, the board tools' MCP config, an action run's or a view
//! brain's arguments) comes in as inputs.
//!
//! Ports `commandFor`, `startSession`'s initial status and size, and
//! `sendPrompt` from `src/main/pty.ts`, and `isTakeover` and the spawn
//! options of `createSession`, `session:open` and `sendToSession` from
//! `src/main/ipc.ts`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use nodus_domain::{BoardNode, SessionKind, SessionModel, SessionStatus};
use nodus_platform::env::EnvMap;
use nodus_platform::{ClaudeNotFound, Env, ShellCommand};
use regex::Regex;

/// The size a pseudo-terminal starts at when none is given (SESS-128).
pub const DEFAULT_COLS: f64 = 120.0;
pub const DEFAULT_ROWS: f64 = 36.0;
/// How long after a prompt is pasted into a live session Return is sent (SESS-144).
pub const SEND_RETURN_DELAY: Duration = Duration::from_millis(200);

/// How much Claude Code asks before acting (SESS-122).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    AcceptEdits,
    /// Approves routine steps by itself and blocks risky ones: action runs,
    /// `autoMode` cards and views' brains.
    Auto,
}

impl PermissionMode {
    /// The `--permission-mode` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
        }
    }
}

/// What starts a card's process (pty.ts's `SpawnOptions`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpawnOptions {
    pub session_id: String,
    pub project_id: String,
    /// The project folder.
    pub cwd: String,
    /// Claude when none. A terminal ignores resume, name, first prompt,
    /// permission mode and extra arguments.
    pub kind: Option<SessionKind>,
    /// Resume an existing transcript instead of starting a new session with this id.
    pub resume: bool,
    pub name: Option<String>,
    pub initial_prompt: Option<String>,
    /// `acceptEdits` when none.
    pub permission_mode: Option<PermissionMode>,
    /// The card's model, a `SessionModel` alias; Claude Code's default when none.
    pub model: Option<String>,
    /// More `claude` options, such as an action run's system prompt.
    pub extra_args: Option<Vec<String>>,
    pub cols: Option<f64>,
    pub rows: Option<f64>,
    /// Its own token, on a server (SRV-143), which the host makes its hook
    /// commands send. None in the app.
    pub session_token: Option<String>,
}

/// What a process is started as: a program, its arguments and its whole
/// environment. On Windows the host joins the arguments into one command
/// line (`windowsCommandLine`, 02), so JSON arguments go as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub file: PathBuf,
    pub args: Vec<String>,
    pub env: EnvMap,
}

/// The parts of the login-shell environment (01) a launch reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEnv {
    /// The `claude` binary, if it was found.
    pub claude: Option<PathBuf>,
    /// `spawnEnv()`.
    pub spawn_env: EnvMap,
    /// Git for Windows' bash, which hooks were written for; none off Windows.
    pub git_bash: Option<PathBuf>,
    /// Whether the user's own environment sets `CLAUDE_CODE_GIT_BASH_PATH`.
    pub git_bash_set: bool,
    /// A terminal card's shell.
    pub terminal_shell: ShellCommand,
}

impl LaunchEnv {
    pub fn from_env(env: &Env) -> Self {
        Self {
            claude: env.claude_path().map(Path::to_owned),
            spawn_env: env.spawn_env(),
            git_bash: env.git_bash().map(Path::to_owned),
            git_bash_set: env.var("CLAUDE_CODE_GIT_BASH_PATH").is_some_and(|v| !v.is_empty()),
            terminal_shell: env.terminal_shell(),
        }
    }
}

/// What the status machine (02) gives every Claude session.
#[derive(Debug, Clone, Copy)]
pub struct Hooks<'a> {
    /// `hookSettings()`: the `--settings` JSON.
    pub settings: &'a str,
    /// `hookEnv(sessionId)`: `NODUS_PORT`, `NODUS_TOKEN`, `NODUS_SESSION` and
    /// maybe `NODUS_HOOK_CONFIG`.
    pub env: &'a EnvMap,
}

/// The board tools (04), when the connected session host relays them (its
/// welcome said `mcp: true`, 02).
#[derive(Debug, Clone, Copy)]
pub struct BoardTools<'a> {
    /// `BOARD_MCP_CONFIG`, one argument.
    pub mcp_config: &'a str,
    /// `BOARD_TOOL_RULES`, one argument each.
    pub allowed_tools: &'a [String],
}

/// The command line for a card's process: a login shell for a terminal, like
/// Terminal.app's, or `claude` (SESS-121…127).
pub fn command_for(
    opts: &SpawnOptions,
    env: &LaunchEnv,
    hooks: Hooks<'_>,
    board_tools: Option<BoardTools<'_>>,
) -> Result<Command, ClaudeNotFound> {
    // SESS-127: no hooks, no board tools.
    if opts.kind == Some(SessionKind::Terminal) {
        return Ok(Command {
            file: env.terminal_shell.file.clone(),
            args: env.terminal_shell.args.clone(),
            env: env.spawn_env.clone(),
        });
    }

    // SESS-121
    let mut args = if opts.resume {
        vec!["--resume".to_owned(), opts.session_id.clone()]
    } else {
        vec!["--session-id".to_owned(), opts.session_id.clone()]
    };
    // SESS-125: only session:create names a session.
    if let Some(name) = opts.name.as_deref().filter(|n| !n.is_empty()) {
        args.extend(["--name".to_owned(), name.to_owned()]);
    }
    // SESS-122
    let mode = opts.permission_mode.unwrap_or(PermissionMode::AcceptEdits);
    args.extend(["--permission-mode".to_owned(), mode.as_str().to_owned()]);
    // The model the card was started on, on every launch: a resume doesn't keep it.
    if let Some(model) = opts.model.as_deref().and_then(SessionModel::parse) {
        args.extend(["--model".to_owned(), model.as_str().to_owned()]);
    }
    // SESS-123: the board tools, allowed outright so they never stop to ask.
    // Both lists are followed by another option, so neither swallows the prompt.
    if let Some(tools) = board_tools {
        args.extend([
            "--mcp-config".to_owned(),
            tools.mcp_config.to_owned(),
            "--allowedTools".to_owned(),
        ]);
        args.extend(tools.allowed_tools.iter().cloned());
    }
    // SESS-132, SESS-133
    if let Some(extra) = &opts.extra_args {
        args.extend(extra.iter().cloned());
    }
    args.extend(["--settings".to_owned(), hooks.settings.to_owned()]);
    // SESS-124: after `--`, so a prompt that starts with a dash isn't read as an option.
    if let Some(prompt) = opts.initial_prompt.as_deref().filter(|p| !p.is_empty()) {
        args.extend(["--".to_owned(), prompt.to_owned()]);
    }
    // SESS-126: on Windows, the Git Bash its hooks were written for, should claude look elsewhere.
    let mut process_env = env.spawn_env.clone();
    if let Some(bash) = &env.git_bash
        && !env.git_bash_set
    {
        process_env.insert("CLAUDE_CODE_GIT_BASH_PATH".into(), bash.to_string_lossy().into_owned());
    }
    process_env.extend(hooks.env.iter().map(|(k, v)| (k.clone(), v.clone())));
    let file = env.claude.clone().ok_or(ClaudeNotFound)?;
    Ok(Command {
        file,
        args,
        env: process_env,
    })
}

/// With a first prompt the turn starts at once, so the card shows working
/// straight away (SESS-129).
pub fn initial_status(opts: &SpawnOptions) -> SessionStatus {
    if opts.initial_prompt.as_deref().is_some_and(|p| !p.is_empty()) {
        SessionStatus::Working
    } else {
        SessionStatus::Done
    }
}

/// The pseudo-terminal's size: 120 × 36 unless given, at least 20 × 5 (SESS-128).
pub fn pty_size(opts: &SpawnOptions) -> (f64, f64) {
    (
        opts.cols.unwrap_or(DEFAULT_COLS).max(20.0),
        opts.rows.unwrap_or(DEFAULT_ROWS).max(5.0),
    )
}

/// An action run's prompt and arguments (`runArgs(action)`, 05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRun<'a> {
    pub prompt: &'a str,
    pub args: Vec<String>,
}

/// What `session:create` starts a new card with (SESS-119): a run's prompt,
/// auto mode and arguments, or the window's first prompt, trimmed.
pub fn for_create(
    project_id: &str,
    cwd: &str,
    node: &BoardNode,
    action: Option<ActionRun<'_>>,
    initial_prompt: Option<&str>,
) -> SpawnOptions {
    let terminal = node.kind == Some(SessionKind::Terminal);
    let first_prompt = match &action {
        Some(action) => Some(action.prompt.to_owned()),
        None => initial_prompt
            .map(nodus_domain::js::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned),
    };
    SpawnOptions {
        session_id: node.session_id.clone(),
        project_id: project_id.to_owned(),
        cwd: cwd.to_owned(),
        kind: node.kind,
        resume: false,
        name: if terminal { None } else { node.title.clone() },
        initial_prompt: if terminal { None } else { first_prompt },
        permission_mode: (action.is_some() || node.auto_mode == Some(true)).then_some(PermissionMode::Auto),
        model: node.model().map(str::to_owned),
        extra_args: action.map(|a| a.args),
        cols: None,
        rows: None,
        session_token: None,
    }
}

/// What `session:open` starts a stopped session with (SESS-136, SESS-137): a
/// fresh shell for a terminal, a resume when there's a transcript, auto mode
/// for an `autoMode` card, a view brain's arguments (`brainArgs`, 04) for a
/// brain, and the terminal's columns and rows. The card may not be on the board.
pub fn for_open(
    project_id: &str,
    cwd: &str,
    session_id: &str,
    node: Option<&BoardNode>,
    has_transcript: bool,
    brain_args: Option<Vec<String>>,
    (cols, rows): (f64, f64),
) -> SpawnOptions {
    SpawnOptions {
        session_id: session_id.to_owned(),
        project_id: project_id.to_owned(),
        cwd: cwd.to_owned(),
        kind: node.and_then(|n| n.kind),
        resume: has_transcript,
        permission_mode: node
            .is_some_and(|n| n.auto_mode == Some(true))
            .then_some(PermissionMode::Auto),
        model: node.and_then(BoardNode::model).map(str::to_owned),
        extra_args: brain_args.filter(|_| node.is_some_and(is_brain)),
        cols: Some(cols),
        rows: Some(rows),
        ..SpawnOptions::default()
    }
}

/// What `session:send` resumes a stopped session with: the prompt, trimmed,
/// as its first (SESS-145).
pub fn for_send(
    project_id: &str,
    cwd: &str,
    node: &BoardNode,
    has_transcript: bool,
    prompt: &str,
    brain_args: Option<Vec<String>>,
) -> SpawnOptions {
    SpawnOptions {
        session_id: node.session_id.clone(),
        project_id: project_id.to_owned(),
        cwd: cwd.to_owned(),
        resume: has_transcript,
        initial_prompt: Some(nodus_domain::js::trim(prompt).to_owned()),
        permission_mode: (node.auto_mode == Some(true)).then_some(PermissionMode::Auto),
        model: node.model().map(str::to_owned),
        extra_args: brain_args.filter(|_| is_brain(node)),
        ..SpawnOptions::default()
    }
}

fn is_brain(node: &BoardNode) -> bool {
    node.view_of.as_deref().is_some_and(|v| !v.is_empty())
}

/// A prompt typed into a live session as a paste, so its lines don't send it
/// early; Return follows after [`SEND_RETURN_DELAY`] (SESS-144).
pub fn bracketed_paste(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~")
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static STRING_SEQUENCE: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)\x1b[P\]^_].*?(?:\x07|\x1b\\)"));
static CSI_SEQUENCE: LazyLock<Regex> = LazyLock::new(|| regex(r"\x1b\[[0-?]*[ -/]*[@-~]"));
static ESCAPE_PAIR: LazyLock<Regex> = LazyLock::new(|| regex(r"\x1b[@-_]"));
static TYPED: LazyLock<Regex> = LazyLock::new(|| regex(r"[^\x00-\x1f\x7f]|\r"));

/// Whether terminal input to a run is the user stepping in: an interrupt
/// (Esc, Ctrl+C), or text and Enter unless the run is waiting on them, when
/// keys answer its question. Replies the terminal sends by itself (focus
/// changes, cursor, mouse and colour reports) don't count (SESS-139).
pub fn is_takeover(data: &str, status: SessionStatus) -> bool {
    if data == "\x1b" || data == "\x03" {
        return true;
    }
    if status == SessionStatus::Waiting {
        return false;
    }
    let typed = STRING_SEQUENCE.replace_all(data, "");
    let typed = CSI_SEQUENCE.replace_all(&typed, "");
    let typed = ESCAPE_PAIR.replace_all(&typed, "");
    TYPED.is_match(&typed)
}

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)] // Helpers that build JSON read clearer taking values.
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "00000000-0000-4000-8000-000000000001";
    const SETTINGS: &str = r#"{"hooks":{"Stop":[]},"statusLine":{"type":"command","command":"curl"}}"#;
    const MCP: &str = r#"{"mcpServers":{"nodus":{"type":"http","url":"http://127.0.0.1:${NODUS_PORT}/mcp","headers":{"Authorization":"Bearer ${NODUS_TOKEN}","X-Nodus-Session":"${NODUS_SESSION}"}}}}"#;

    fn rules() -> Vec<String> {
        [
            "create_note",
            "update_note",
            "list_notes",
            "list_projects",
            "create_project",
            "archive_self",
            "list_board",
            "draw_on_board",
            "update_drawing",
            "erase_from_board",
            "create_view",
            "update_view",
            "get_view",
            "list_views",
            "list_sessions",
            "archive_sessions",
        ]
        .iter()
        .map(|t| format!("mcp__nodus__{t}"))
        .collect()
    }

    fn env() -> LaunchEnv {
        LaunchEnv {
            claude: Some("/opt/claude/bin/claude".into()),
            spawn_env: EnvMap::from([
                ("PATH".into(), "/usr/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
                ("NODUS_SESSION".into(), "stale".into()),
            ]),
            git_bash: None,
            git_bash_set: false,
            terminal_shell: ShellCommand {
                file: "/bin/zsh".into(),
                args: vec!["-l".into()],
            },
        }
    }

    fn hook_env() -> EnvMap {
        EnvMap::from([
            ("NODUS_PORT".into(), "52100".into()),
            ("NODUS_TOKEN".into(), "test-token".into()),
            ("NODUS_SESSION".into(), ID.into()),
        ])
    }

    fn argv(opts: &SpawnOptions, tools: bool) -> Vec<String> {
        let rules = rules();
        let tools = tools.then_some(BoardTools {
            mcp_config: MCP,
            allowed_tools: &rules,
        });
        let hook_env = hook_env();
        let hooks = Hooks {
            settings: SETTINGS,
            env: &hook_env,
        };
        command_for(opts, &env(), hooks, tools).unwrap().args
    }

    fn node(v: serde_json::Value) -> BoardNode {
        let mut base = json!({ "sessionId": ID, "x": 0, "y": 0, "archived": false, "createdAt": 0 });
        for (k, val) in v.as_object().unwrap() {
            base[k] = val.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| (*i).to_owned()).collect()
    }

    #[test]
    fn starts_a_new_named_session_with_a_prompt() {
        let opts = for_create(
            "p",
            "/work/repo",
            &node(json!({ "title": "Fix login" })),
            None,
            Some("  -v is broken "),
        );
        let mut expected = s(&[
            "--session-id",
            ID,
            "--name",
            "Fix login",
            "--permission-mode",
            "acceptEdits",
        ]);
        expected.extend(s(&["--mcp-config", MCP, "--allowedTools"]));
        expected.extend(rules());
        expected.extend(s(&["--settings", SETTINGS, "--", "-v is broken"]));
        assert_eq!(argv(&opts, true), expected);
        assert_eq!(initial_status(&opts), SessionStatus::Working);
        assert_eq!(pty_size(&opts), (120.0, 36.0));
    }

    #[test]
    fn starts_and_resumes_on_the_cards_model() {
        let card = node(json!({ "model": "opus" }));
        let created = for_create("p", "/work/repo", &card, None, Some("design it"));
        let opened = for_open("p", "/work/repo", ID, Some(&card), true, None, (80.0, 24.0));
        let sent = for_send("p", "/work/repo", &card, true, "and test it", None);
        for opts in [&created, &opened, &sent] {
            let args = argv(opts, false);
            let at = args.iter().position(|a| a == "--model").expect("a --model");
            assert_eq!(args[at + 1], "opus");
        }
    }

    #[test]
    fn leaves_out_a_model_it_doesnt_know() {
        let opts = for_create("p", "/work/repo", &node(json!({ "model": "-rf" })), None, None);
        assert!(!argv(&opts, false).iter().any(|a| a == "--model"));
    }

    #[test]
    fn resumes_without_a_name() {
        let opts = for_open(
            "p",
            "/work/repo",
            ID,
            Some(&node(json!({ "title": "Fix login" }))),
            true,
            None,
            (80.0, 3.0),
        );
        assert_eq!(
            argv(&opts, false),
            s(&[
                "--resume",
                ID,
                "--permission-mode",
                "acceptEdits",
                "--settings",
                SETTINGS
            ])
        );
        assert_eq!(initial_status(&opts), SessionStatus::Done);
        assert_eq!(pty_size(&opts), (80.0, 5.0));
        // A card never used starts with its own id.
        let fresh = for_open("p", "/work/repo", ID, None, false, None, (100.0, 30.0));
        assert_eq!(&argv(&fresh, false)[..2], s(&["--session-id", ID]));
    }

    #[test]
    fn runs_an_action_in_auto_mode_with_its_system_prompt() {
        let run_prompt =
            "You are running \"Deploy\", an action button the user pressed on the Nodus board for this project.";
        let action = ActionRun {
            prompt: "Deploy to staging",
            args: s(&["--append-system-prompt", run_prompt]),
        };
        let run = node(json!({ "title": "Deploy", "runOf": "deploy", "createdByApp": true }));
        let opts = for_create("p", "/work/repo", &run, Some(action), Some("ignored"));
        let mut expected = s(&["--session-id", ID, "--name", "Deploy", "--permission-mode", "auto"]);
        expected.extend(s(&["--mcp-config", MCP, "--allowedTools"]));
        expected.extend(rules());
        expected.extend(s(&[
            "--append-system-prompt",
            run_prompt,
            "--settings",
            SETTINGS,
            "--",
            "Deploy to staging",
        ]));
        assert_eq!(argv(&opts, true), expected);
    }

    #[test]
    fn reopens_a_taken_over_run_as_an_ordinary_session() {
        // SESS-137: no runOf, no autoMode, so acceptEdits and no system prompt.
        let opts = for_open(
            "p",
            "/r",
            ID,
            Some(&node(json!({ "title": "Deploy" }))),
            true,
            None,
            (120.0, 36.0),
        );
        assert_eq!(opts.permission_mode, None);
        assert_eq!(opts.extra_args, None);
    }

    #[test]
    fn tells_a_views_brain_about_its_view_every_time() {
        let brain = node(json!({ "title": "View: Stats", "viewOf": "v1", "autoMode": true, "createdByApp": true }));
        let args = s(&["--append-system-prompt", "You are the brain of a view"]);
        let open = for_open("p", "/r", ID, Some(&brain), true, Some(args.clone()), (120.0, 36.0));
        let mut expected = s(&["--resume", ID, "--permission-mode", "auto"]);
        expected.extend(args.clone());
        expected.extend(s(&["--settings", SETTINGS]));
        assert_eq!(argv(&open, false), expected);
        let send = for_send("p", "/r", &brain, true, " refresh ", Some(args.clone()));
        let mut expected = s(&["--resume", ID, "--permission-mode", "auto"]);
        expected.extend(args.clone());
        expected.extend(s(&["--settings", SETTINGS, "--", "refresh"]));
        assert_eq!(argv(&send, false), expected);
        // Brain arguments only go to a brain.
        let plain = for_send("p", "/r", &node(json!({})), false, "hi", Some(args));
        assert_eq!(plain.extra_args, None);
    }

    #[test]
    fn leaves_the_board_tools_out_for_an_older_host() {
        let opts = for_create("p", "/r", &node(json!({})), None, None);
        assert_eq!(
            argv(&opts, false),
            s(&[
                "--session-id",
                ID,
                "--permission-mode",
                "acceptEdits",
                "--settings",
                SETTINGS
            ])
        );
    }

    #[test]
    fn gives_claude_the_hook_environment() {
        let hook_env = hook_env();
        let hooks = Hooks {
            settings: SETTINGS,
            env: &hook_env,
        };
        let opts = for_create("p", "/r", &node(json!({ "autoMode": true })), None, Some(""));
        let command = command_for(&opts, &env(), hooks, None).unwrap();
        assert_eq!(command.file, PathBuf::from("/opt/claude/bin/claude"));
        assert_eq!(command.env["NODUS_SESSION"], ID);
        assert_eq!(command.env["NODUS_PORT"], "52100");
        assert_eq!(command.env["PATH"], "/usr/bin");
        assert!(!command.env.contains_key("CLAUDE_CODE_GIT_BASH_PATH"));
        assert_eq!(command.args[2..4], s(&["--permission-mode", "auto"]));
        assert_eq!(initial_status(&opts), SessionStatus::Done);
        // Git Bash on Windows, unless the user set their own.
        let mut windows = env();
        windows.git_bash = Some(r"C:\Program Files\Git\bin\bash.exe".into());
        let command = command_for(&opts, &windows, hooks, None).unwrap();
        assert_eq!(
            command.env["CLAUDE_CODE_GIT_BASH_PATH"],
            r"C:\Program Files\Git\bin\bash.exe"
        );
        windows.git_bash_set = true;
        assert!(
            !command_for(&opts, &windows, hooks, None)
                .unwrap()
                .env
                .contains_key("CLAUDE_CODE_GIT_BASH_PATH")
        );
        // No claude, no session.
        let mut missing = env();
        missing.claude = None;
        assert_eq!(command_for(&opts, &missing, hooks, None), Err(ClaudeNotFound));
    }

    #[test]
    fn starts_a_terminal_as_a_login_shell() {
        let terminal = node(json!({ "kind": "terminal", "title": "zsh", "autoMode": true }));
        let opts = for_create("p", "/r", &terminal, None, Some("ignored"));
        assert_eq!((&opts.name, &opts.initial_prompt), (&None, &None));
        let hook_env = hook_env();
        let hooks = Hooks {
            settings: SETTINGS,
            env: &hook_env,
        };
        let mut missing = env();
        missing.claude = None;
        let command = command_for(&opts, &missing, hooks, None).unwrap();
        assert_eq!(command.file, PathBuf::from("/bin/zsh"));
        assert_eq!(command.args, s(&["-l"]));
        // spawnEnv() alone: no hooks.
        assert_eq!(command.env, env().spawn_env);
        // A terminal reopened gets a fresh shell.
        let open = for_open("p", "/r", ID, Some(&terminal), false, None, (90.0, 20.0));
        assert_eq!(command_for(&open, &missing, hooks, None).unwrap().args, s(&["-l"]));
    }

    #[test]
    fn pastes_a_prompt_into_a_live_session() {
        assert_eq!(
            bracketed_paste("line one\nline two"),
            "\x1b[200~line one\nline two\x1b[201~"
        );
        assert_eq!(SEND_RETURN_DELAY, Duration::from_millis(200));
    }

    #[test]
    fn tells_the_user_stepping_in_from_the_terminal_talking() {
        use SessionStatus::{Done, Waiting, Working};
        // Interrupts always count.
        assert!(is_takeover("\x1b", Waiting));
        assert!(is_takeover("\x03", Working));
        // Typing and Return count, unless the run waits on them.
        assert!(is_takeover("y", Working));
        assert!(is_takeover("\r", Done));
        assert!(!is_takeover("y", Waiting));
        assert!(!is_takeover("\r", Waiting));
        // What the terminal sends by itself doesn't.
        for reply in [
            "\x1b[I",
            "\x1b[O",
            "\x1b[A",
            "\x1b[<0;10;5M",
            "\x1b]11;rgb:1e1e/1e1e/1e1e\x07",
            "\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
            "\x1b[?1;2c",
            "\x1bO",
            "\x7f",
        ] {
            assert!(!is_takeover(reply, Working), "{reply:?}");
        }
        // A bracketed paste of text does, and so does an X10 mouse report.
        assert!(is_takeover("\x1b[200~hello\x1b[201~", Working));
        assert!(is_takeover("\x1b[M !!", Working));
        assert!(is_takeover("é", Working));
    }
}
