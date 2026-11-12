//! `src/main/captureGuess.test.ts`, case by case, then the rest of the
//! guess: the prompt's edges, and the process run against a stand-in `claude`.

use nodus_domain::{
    CaptureProject, CaptureSession, FileAttachment, FolderColor, ImageAttachment, TabAttachment, TextAttachment,
};
use serde_json::json;

use super::*;

const NOW: f64 = 1_800_000_000_000.0;

fn session(project: &str, id: &str, title: &str, status: SessionStatus, ago_ms: f64) -> CaptureSession {
    CaptureSession {
        project_id: project.into(),
        session_id: id.into(),
        title: title.into(),
        status,
        live: true,
        updated_at: NOW - ago_ms,
    }
}

fn targets() -> CaptureTargets {
    CaptureTargets {
        projects: vec![
            CaptureProject {
                id: "p-store".into(),
                name: "storefront".into(),
                color: FolderColor::Blue,
            },
            CaptureProject {
                id: "p-api".into(),
                name: "api".into(),
                color: FolderColor::Purple,
            },
        ],
        sessions: vec![
            session(
                "p-store",
                "s-round",
                "Fix checkout total rounding",
                SessionStatus::Done,
                25.0 * 60_000.0,
            ),
            session(
                "p-api",
                "s-uuid",
                "Migrate orders to UUIDs",
                SessionStatus::Waiting,
                2.0 * 60_000.0,
            ),
            session("p-api", "s-stripe", "Bump Stripe SDK", SessionStatus::Working, 0.0),
        ],
        recents: vec![],
        default_project_id: Some("p-store".into()),
    }
}

fn guess(kind: CaptureKind, project: &str, session: Option<&str>, reason: &str) -> CaptureGuess {
    CaptureGuess {
        kind,
        project_id: project.into(),
        session_id: session.map(str::to_owned),
        reason: reason.into(),
    }
}

#[test]
fn build_guess_prompt_lists_projects_with_their_sessions_the_screen_and_the_text() {
    let targets = targets();
    let attachments = [
        CaptureAttachment::Tab(TabAttachment {
            app: "Safari".into(),
            title: "Refund fails #482".into(),
            url: "https://example.com/acme/storefront/issues/482".into(),
        }),
        CaptureAttachment::File(FileAttachment {
            path: "/x/checkout.log".into(),
            name: "checkout.log".into(),
            size: 3,
        }),
    ];
    let prompt = build_guess_prompt(
        &GuessInput {
            text: "also cover the coupon path",
            app: Some("Safari"),
            attachments: &attachments,
            targets: &targets,
        },
        NOW,
    );
    for line in [
        "Projects (default: storefront, p-store):",
        "- project p-store: storefront",
        "  - session s-round: “Fix checkout total rounding”, idle, active 25 min ago",
        "  - session s-uuid: “Migrate orders to UUIDs”, waiting for the user, active 2 min ago",
        "The app in front: Safari",
        "Its browser tab: Refund fails #482 (https://example.com/acme/storefront/issues/482)",
        "A file is attached: checkout.log",
    ] {
        assert!(prompt.contains(line), "missing {line:?} in\n{prompt}");
    }
    assert!(prompt.ends_with("The capture:\n\"\"\"\nalso cover the coupon path\n\"\"\""));
    // The whole of it, as the spec's example lays it out.
    assert_eq!(
        prompt,
        "Projects (default: storefront, p-store):\n\
         - project p-store: storefront\n  \
         - session s-round: “Fix checkout total rounding”, idle, active 25 min ago\n\
         - project p-api: api\n  \
         - session s-uuid: “Migrate orders to UUIDs”, waiting for the user, active 2 min ago\n  \
         - session s-stripe: “Bump Stripe SDK”, working, active just now\n\
         \n\
         The app in front: Safari\n\
         Its browser tab: Refund fails #482 (https://example.com/acme/storefront/issues/482)\n\
         A file is attached: checkout.log\n\
         \n\
         The capture:\n\"\"\"\nalso cover the coupon path\n\"\"\""
    );
}

#[test]
fn read_guess_takes_a_valid_answer() {
    let raw = json!({ "kind": "into", "project": "p-store", "session": "s-round", "reason": "It follows on." });
    assert_eq!(
        read_guess(&raw, &targets()),
        Some(guess(CaptureKind::Into, "p-store", Some("s-round"), "It follows on."))
    );
}

#[test]
fn read_guess_falls_back_to_the_default_project_for_one_it_made_up() {
    let raw = json!({ "kind": "note", "project": "nope", "session": "", "reason": "An idea." });
    let got = read_guess(&raw, &targets()).unwrap();
    assert_eq!((got.kind, got.project_id.as_str()), (CaptureKind::Note, "p-store"));
}

#[test]
fn read_guess_starts_a_session_instead_of_sending_into_one_that_is_waiting_or_missing() {
    let raw = json!({ "kind": "into", "project": "p-api", "session": "s-uuid", "reason": "" });
    let got = read_guess(&raw, &targets()).unwrap();
    assert_eq!(
        (got.kind, got.project_id.as_str(), got.session_id),
        (CaptureKind::Session, "p-api", None)
    );
    let raw = json!({ "kind": "into", "project": "p-api", "session": "gone", "reason": "" });
    assert_eq!(read_guess(&raw, &targets()).unwrap().kind, CaptureKind::Session);
}

#[test]
fn read_guess_takes_the_sessions_own_project() {
    let raw = json!({ "kind": "into", "project": "p-store", "session": "s-stripe", "reason": "" });
    assert_eq!(read_guess(&raw, &targets()).unwrap().project_id, "p-api");
}

#[test]
fn read_guess_turns_down_nonsense() {
    assert_eq!(read_guess(&Value::Null, &targets()), None);
    let raw = json!({ "kind": "party", "project": "p-api", "session": "", "reason": 7 });
    assert_eq!(
        read_guess(&raw, &targets()),
        Some(guess(CaptureKind::Session, "p-api", None, ""))
    );
}

/// CAP-080: no project anywhere is no guess; an array reads as an object with
/// no fields; the reason is flattened and cut.
#[test]
fn read_guess_at_its_edges() {
    let mut none = targets();
    none.default_project_id = None;
    assert_eq!(read_guess(&json!({ "kind": "note", "project": "nope" }), &none), None);
    // Even a good session doesn't help when there's no project to start from.
    assert_eq!(
        read_guess(&json!({ "kind": "into", "session": "s-round" }), &none),
        None
    );
    assert_eq!(
        read_guess(&json!(["note"]), &targets()),
        Some(guess(CaptureKind::Session, "p-store", None, ""))
    );
    assert_eq!(read_guess(&json!("note"), &targets()), None);
    assert_eq!(read_guess(&json!(7), &targets()), None);
    let long = format!("  It   follows\n{}", "x".repeat(200));
    let got = read_guess(
        &json!({ "kind": "view", "project": "p-api", "reason": long }),
        &targets(),
    )
    .unwrap();
    assert_eq!(js::utf16_len(&got.reason), 120);
    assert!(got.reason.starts_with("It follows x") && got.reason.ends_with('…'));
}

/// CAP-076
#[test]
fn ages_read_as_the_prompt_words_them() {
    assert_eq!(ago(0.0), "just now");
    assert_eq!(ago(29_999.0), "just now");
    assert_eq!(ago(30_000.0), "1 min ago");
    assert_eq!(ago(-5.0 * 60_000.0), "just now");
    assert_eq!(ago(59.0 * 60_000.0), "59 min ago");
    assert_eq!(ago(59.5 * 60_000.0), "1 h ago");
    assert_eq!(ago(90.0 * 60_000.0), "2 h ago");
    assert_eq!(ago(47.0 * 3_600_000.0), "47 h ago");
    assert_eq!(ago(48.0 * 3_600_000.0), "2 days ago");
    assert_eq!(ago(30.0 * 86_400_000.0), "30 days ago");
}

/// CAP-077
#[test]
fn flat_puts_text_on_one_line_and_cuts_it() {
    assert_eq!(flat("  a \n\t b\u{FEFF} ", 80), "a b");
    assert_eq!(flat("abcdef", 6), "abcdef");
    assert_eq!(flat("abcdefg", 6), "abcde…");
    // UTF-16 units: the emoji is two.
    assert_eq!(flat("ab😀cd", 4), "ab\u{FFFD}…");
}

/// CAP-078
#[test]
fn status_words() {
    assert_eq!(status_word(SessionStatus::Waiting, false), "waiting for the user");
    assert_eq!(status_word(SessionStatus::Working, false), "working");
    assert_eq!(status_word(SessionStatus::Done, true), "idle");
    assert_eq!(status_word(SessionStatus::Done, false), "stopped");
}

/// CAP-075, CAP-079: with nothing from the screen, two blank lines before
/// the capture; no default; eight sessions at most; the text cut to 4000;
/// every kind of attachment worded, paths never sent.
#[test]
fn build_guess_prompt_at_its_edges() {
    let mut many = targets();
    many.default_project_id = Some("gone".into());
    many.sessions = (0..10)
        .map(|i| session("p-api", &format!("s{i}"), "  Long\ntitle ", SessionStatus::Done, 0.0))
        .collect();
    let text = "  ".to_owned() + &"y".repeat(4100);
    let prompt = build_guess_prompt(
        &GuessInput {
            text: &text,
            attachments: &[],
            app: None,
            targets: &many,
        },
        NOW,
    );
    assert!(prompt.starts_with("Projects:\n- project p-store: storefront\n  (no sessions)\n- project p-api: api\n"));
    assert!(prompt.contains("  - session s7: “Long title”, idle, active just now\n\n\nThe capture:\n\"\"\"\n"));
    assert!(!prompt.contains("session s8"));
    assert!(prompt.ends_with(&format!("\n\"\"\"\n{}\n\"\"\"", "y".repeat(4000))));

    let attachments = [
        CaptureAttachment::Image(ImageAttachment {
            path: "/secret/shot.png".into(),
            thumb: String::new(),
            copied: None,
        }),
        CaptureAttachment::Image(ImageAttachment {
            path: "/secret/copied.png".into(),
            thumb: String::new(),
            copied: Some(true),
        }),
        CaptureAttachment::Text(TextAttachment {
            text: "line one\nline two".into(),
        }),
    ];
    let prompt = build_guess_prompt(
        &GuessInput {
            text: "x",
            attachments: &attachments,
            app: None,
            targets: &targets(),
        },
        NOW,
    );
    assert!(prompt.contains(
        "\n\nA screenshot is attached.\nAn image the user copied is attached.\nCopied text is attached: line one line two\n\nThe capture:"
    ));
    assert!(!prompt.contains("/secret"));
}

/// CAP-069: the argv, element by element.
#[test]
fn the_arguments_are_verbatim() {
    let targets = targets();
    let input = GuessInput {
        text: "hi",
        attachments: &[],
        app: None,
        targets: &targets,
    };
    let args = guess_args(&input, NOW);
    assert_eq!(args.len(), 16);
    assert_eq!(
        args[..15],
        [
            "-p",
            "--model",
            "haiku",
            "--settings",
            "{\"alwaysThinkingEnabled\":false}",
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
    );
    assert_eq!(args[15], build_guess_prompt(&input, NOW));
    // The schema is JSON.stringify's, which nodus-domain writes the same way.
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    assert_eq!(nodus_domain::json::to_string(&schema).unwrap(), SCHEMA);
    assert_eq!(
        schema["properties"]["kind"]["enum"],
        json!(["session", "into", "note", "view"])
    );
    assert!(SYSTEM_PROMPT.starts_with("You route quick captures on Nodus,"));
    assert!(SYSTEM_PROMPT.ends_with("\"It's an idea to keep, not work to start now.\""));
    assert_eq!(SYSTEM_PROMPT.lines().count(), 9);
}

/// CAP-072
#[test]
fn reads_the_reply() {
    let t = targets();
    let ok = r#" {"is_error":false,"structured_output":{"kind":"note","project":"p-api","session":"","reason":"An idea."}} "#;
    assert_eq!(
        read_reply(ok, &t),
        Some(guess(CaptureKind::Note, "p-api", None, "An idea."))
    );
    assert_eq!(
        read_reply(r#"{"is_error":true,"structured_output":{"kind":"note"}}"#, &t),
        None
    );
    assert_eq!(
        read_reply(r#"{"is_error":1,"structured_output":{"kind":"note"}}"#, &t),
        None
    );
    assert_eq!(
        read_reply(r#"{"structured_output":{"kind":"note"}}"#, &t).unwrap().kind,
        CaptureKind::Note
    );
    assert_eq!(read_reply("not json", &t), None);
    assert_eq!(read_reply("null", &t), None);
    assert_eq!(read_reply(r#"{"result":"hi"}"#, &t), None);
}

// A shell script stands in for `claude`, so these run on macOS and Linux.
#[cfg(unix)]
mod process {
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use super::*;

    struct StandIn {
        claude: Option<PathBuf>,
    }

    impl ClaudeEnv for StandIn {
        fn claude(&self) -> Result<Program, ClaudeNotFound> {
            self.claude.clone().map(Program::from).ok_or(ClaudeNotFound)
        }

        fn runner(&self) -> Arc<dyn Runner> {
            Arc::new(LocalRunner)
        }

        fn spawn_env(&self) -> EnvMap {
            EnvMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("GUESS_ENV".into(), "login".into()),
            ])
        }
    }

    /// A `claude` that logs its arguments (each ended by NUL), its folder and
    /// an environment variable, reads stdin to its end, then runs `body`.
    fn stand_in(dir: &Path, body: &str) -> Guesser {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("claude");
        let text = format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\000' \"$a\" >> '{dir}/argv.log'; done\npwd > '{dir}/cwd.log'\necho \"$GUESS_ENV\" > '{dir}/env.log'\ncat > /dev/null\n{body}\n",
            dir = dir.display(),
        );
        std::fs::write(&script, text).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Guesser::new(Arc::new(StandIn { claude: Some(script) }))
    }

    fn input(targets: &CaptureTargets) -> GuessInput<'_> {
        GuessInput {
            text: "  remember to ask finance  ",
            attachments: &[],
            app: Some("Safari"),
            targets,
        }
    }

    const NOTE: &str = r#"echo '{"type":"result","is_error":false,"structured_output":{"kind":"note","project":"p-api","session":"","reason":"An idea."}}'"#;

    #[tokio::test]
    async fn runs_claude_with_the_exact_arguments_outside_any_repository() {
        let dir = tempfile::tempdir().unwrap();
        let guesser = stand_in(dir.path(), &format!("echo 'noise' >&2; {NOTE}"));
        let targets = targets();
        let got = guesser.guess(input(&targets), NOW).await.unwrap();
        assert_eq!(got, Some(guess(CaptureKind::Note, "p-api", None, "An idea.")));
        let logged = std::fs::read_to_string(dir.path().join("argv.log")).unwrap();
        // Each argument ends in a NUL, so the last piece is empty.
        let argv: Vec<&str> = logged.split('\0').collect();
        assert_eq!(argv[..argv.len() - 1], guess_args(&input(&targets), NOW));
        let cwd = std::fs::read_to_string(dir.path().join("cwd.log")).unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(Path::new(cwd.trim()).canonicalize().unwrap(), temp);
        assert_eq!(std::fs::read_to_string(dir.path().join("env.log")).unwrap(), "login\n");
        assert!(!guesser.is_running());
    }

    /// CAP-072: the exit code doesn't matter; `is_error` and bad JSON are no guess.
    #[tokio::test]
    async fn reads_whatever_it_printed_however_it_exited() {
        let dir = tempfile::tempdir().unwrap();
        let targets = targets();
        let guesser = stand_in(dir.path(), &format!("{NOTE}; exit 3"));
        assert!(guesser.guess(input(&targets), NOW).await.unwrap().is_some());
        let guesser = stand_in(
            dir.path(),
            r#"echo '{"is_error":true,"structured_output":{"kind":"note"}}'"#,
        );
        assert_eq!(guesser.guess(input(&targets), NOW).await.unwrap(), None);
        let guesser = stand_in(dir.path(), "echo 'Error: not logged in'");
        assert_eq!(guesser.guess(input(&targets), NOW).await.unwrap(), None);
    }

    /// CAP-068: nothing to guess from spawns nothing; no Claude Code fails.
    #[tokio::test]
    async fn asks_only_when_there_is_something_to_ask() {
        let dir = tempfile::tempdir().unwrap();
        let guesser = stand_in(dir.path(), NOTE);
        let targets = targets();
        let blank = GuessInput {
            text: " \n ",
            ..input(&targets)
        };
        assert_eq!(guesser.guess(blank, NOW).await.unwrap(), None);
        let empty = CaptureTargets {
            projects: vec![],
            ..targets.clone()
        };
        assert_eq!(guesser.guess(input(&empty), NOW).await.unwrap(), None);
        assert!(!dir.path().join("argv.log").exists());

        let missing = Guesser::new(Arc::new(StandIn { claude: None }));
        assert_eq!(missing.guess(input(&targets), NOW).await, Err(ClaudeNotFound));
        // Blank text is no guess before Claude is looked for.
        let blank = GuessInput {
            text: "",
            ..input(&targets)
        };
        assert_eq!(missing.guess(blank, NOW).await, Ok(None));
    }

    /// CAP-071: a claude that can't start is no guess.
    #[tokio::test]
    async fn a_claude_that_cant_start_is_no_guess() {
        let dir = tempfile::tempdir().unwrap();
        let guesser = Guesser::new(Arc::new(StandIn {
            claude: Some(dir.path().join("nowhere")),
        }));
        let targets = targets();
        assert_eq!(guesser.guess(input(&targets), NOW).await.unwrap(), None);
    }

    /// CAP-070: past the timeout it's stopped, and no guess.
    #[tokio::test]
    async fn gives_up_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let guesser = stand_in(dir.path(), "sleep 30").with_timeout(Duration::from_millis(300));
        let targets = targets();
        let started = Instant::now();
        assert_eq!(guesser.guess(input(&targets), NOW).await.unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!guesser.is_running());
    }

    /// CAP-074, CAP-176: a new guess, or the panel hiding, stops the one
    /// running, which is no guess at once.
    #[tokio::test]
    async fn a_new_guess_stops_the_one_before() {
        let dir = tempfile::tempdir().unwrap();
        let guesser = Arc::new(stand_in(
            dir.path(),
            &format!(
                "if [ -e '{0}/first' ]; then {NOTE}; else touch '{0}/first'; sleep 30; fi",
                dir.path().display()
            ),
        ));
        let targets = Arc::new(targets());
        let first = {
            let (guesser, targets) = (guesser.clone(), targets.clone());
            tokio::spawn(async move { guesser.guess(input(&targets), NOW).await })
        };
        while !dir.path().join("first").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let started = Instant::now();
        let second = guesser.guess(input(&targets), NOW).await.unwrap();
        assert_eq!(first.await.unwrap(), Ok(None));
        assert!(second.is_some());
        assert!(started.elapsed() < Duration::from_secs(5));

        // Hiding the panel: cancel.
        std::fs::remove_file(dir.path().join("first")).unwrap();
        let third = {
            let (guesser, targets) = (guesser.clone(), targets.clone());
            tokio::spawn(async move { guesser.guess(input(&targets), NOW).await })
        };
        while !guesser.is_running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        guesser.cancel();
        assert_eq!(third.await.unwrap(), Ok(None));
        assert!(!guesser.is_running());
        // Cancelling with nothing running does nothing.
        guesser.cancel();
    }
}
