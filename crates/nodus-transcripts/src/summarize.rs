//! Handoff summaries for "new session from context" (summarize.ts).
//!
//! Each session is resumed headlessly as a fork that is never written to
//! disk, and asked to summarize itself. It has the full context, and the
//! original session is left untouched.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use nodus_domain::js;
use nodus_domain::{SummarizePhase, SummarizeProgress, SummaryResult};
use nodus_platform::env::EnvMap;

use crate::claude::{self, RunError};

/// Sessions summarized at once (SESS-169).
pub const CONCURRENCY: usize = 3;
/// A fork is stopped after this long (SESS-171).
pub const TIMEOUT: Duration = Duration::from_millis(5 * 60 * 1000);

pub const SUMMARY_PROMPT: &str = "Write a handoff summary of this session so that a NEW Claude Code session, with no access to this conversation, can pick up the work.

Cover, under short markdown headings:
- Goal: what the user is trying to achieve.
- Key decisions: what was decided and why, including approaches that were rejected.
- Current state: files created or changed (with paths), what works, and what was verified.
- Open issues and next steps: unfinished work, known bugs, and the obvious next actions.
- Gotchas: important commands, conventions, environment details, or pitfalls discovered.

Answer from the conversation alone: do not call tools, run commands, or change files. Leave out environment notices that are not part of the work (for example connector or MCP authorization messages). Be concise but complete, and prefer specifics (paths, names, commands) over generalities. Output only the summary, with no preamble.";

/// A handoff of just the part of a session about `focus`, for a session
/// started from search results (SESS-167).
pub fn focused_summary_prompt(focus: &str) -> String {
    let topic = nodus_domain::json::to_string(&js::collapse_spaces(js::trim(focus))).expect("a string serializes");
    format!(
        "Write a handoff summary of what this session did about {topic}, so that a NEW Claude Code session, with no access to this conversation, can pick up that work.

Cover only what relates to {topic}, under short markdown headings:
- What was asked: what the user wanted done about it.
- Key decisions: what was decided and why, including approaches that were rejected.
- Current state: files created or changed (with paths), what works, and what was verified.
- Open issues and next steps: unfinished work and known bugs.
- Gotchas: commands, conventions, or pitfalls discovered along the way.

If the session only touched on it in passing, say so in a sentence or two instead. Answer from the conversation alone: do not call tools, run commands, or change files. Be concise, and prefer specifics (paths, names, commands) over generalities. Output only the summary, with no preamble."
    )
}

/// A session to summarize, and the title its result goes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummarizeTarget {
    pub session_id: String,
    pub title: String,
}

/// Summarizes each session with `claude`, three at a time, in the project
/// folder `cwd` with `spawn_env` (`spawnEnv()`), reporting each one's
/// progress. Results come back in the order asked; a session that couldn't be
/// summarized has its `error` set (SESS-167…173). The caller finds `claude`
/// first, failing the whole call without it (SESS-174).
pub async fn summarize_sessions(
    claude: &Path,
    spawn_env: &EnvMap,
    cwd: &str,
    targets: &[SummarizeTarget],
    request_id: &str,
    on_progress: &(dyn Fn(SummarizeProgress) + Send + Sync),
    focus: Option<&str>,
) -> Vec<SummaryResult> {
    // SESS-167
    let prompt = match focus {
        Some(focus) if !js::trim(focus).is_empty() => focused_summary_prompt(focus),
        _ => SUMMARY_PROMPT.to_owned(),
    };
    let results = Mutex::new(
        targets
            .iter()
            .map(|t| SummaryResult {
                session_id: t.session_id.clone(),
                title: t.title.clone(),
                summary: String::new(),
                error: None,
            })
            .collect::<Vec<_>>(),
    );
    let progress = |session_id: &str, phase: SummarizePhase, error: Option<String>| {
        on_progress(SummarizeProgress {
            request_id: request_id.to_owned(),
            session_id: session_id.to_owned(),
            phase,
            error,
        });
    };
    // SESS-168
    for t in targets {
        progress(&t.session_id, SummarizePhase::Queued, None);
    }

    // SESS-169: each worker takes the next session as it finishes one.
    let next = AtomicUsize::new(0);
    let worker = || async {
        loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            let Some(target) = targets.get(i) else {
                return;
            };
            progress(&target.session_id, SummarizePhase::Running, None);
            let outcome = summarize_one(claude, Path::new(cwd), spawn_env, &target.session_id, &prompt).await;
            let mut all = results.lock().unwrap_or_else(PoisonError::into_inner);
            match outcome {
                Ok(summary) => {
                    all[i].summary = summary;
                    drop(all);
                    progress(&target.session_id, SummarizePhase::Done, None);
                }
                Err(error) => {
                    all[i].error = Some(error.clone());
                    drop(all);
                    progress(&target.session_id, SummarizePhase::Error, Some(error));
                }
            }
        }
    };
    join_all((0..CONCURRENCY.min(targets.len())).map(|_| worker()).collect()).await;
    // SESS-173
    results.into_inner().unwrap_or_else(PoisonError::into_inner)
}

/// One fork, asked for its summary (SESS-170…172).
async fn summarize_one(
    claude: &Path,
    cwd: &Path,
    env: &EnvMap,
    session_id: &str,
    prompt: &str,
) -> Result<String, String> {
    let args: Vec<String> = [
        "-p",
        "--resume",
        session_id,
        "--fork-session",
        "--no-session-persistence",
        "--output-format",
        "json",
        prompt,
    ]
    .map(str::to_owned)
    .to_vec();
    let finished = match claude::run(claude, &args, cwd, env, TIMEOUT).await {
        Ok(finished) => finished,
        Err(RunError::TimedOut) => return Err("Timed out after 5 minutes".into()),
        Err(RunError::Spawn(message)) => return Err(message),
    };
    let parsed = claude::parse_reply(&finished.stdout);
    match claude::result_of(parsed.as_ref()).map(js::trim) {
        Some(summary) if !summary.is_empty() => Ok(summary.to_owned()),
        _ => Err(claude::failure(parsed.as_ref(), &finished)),
    }
}

/// Runs the workers together on this task, polling them in order, so they
/// report as `Promise.all` over the same workers would.
async fn join_all<F: Future<Output = ()>>(futures: Vec<F>) {
    let mut futures: Vec<Option<Pin<Box<F>>>> = futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for slot in &mut futures {
            if let Some(future) = slot {
                if future.as_mut().poll(cx).is_ready() {
                    *slot = None;
                } else {
                    pending = true;
                }
            }
        }
        if pending { Poll::Pending } else { Poll::Ready(()) }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_the_focus_as_json() {
        let prompt = focused_summary_prompt("  the \"auth\"\n middleware ");
        assert!(prompt.starts_with(
            "Write a handoff summary of what this session did about \"the \\\"auth\\\" middleware\", so that"
        ));
        assert!(prompt.contains("Cover only what relates to \"the \\\"auth\\\" middleware\", under"));
    }

    #[cfg(unix)]
    mod with_a_stand_in {
        use super::*;
        use crate::claude::stand_in;
        use nodus_domain::SummarizePhase::{Done, Error, Queued, Running};

        fn phases(events: &Mutex<Vec<SummarizeProgress>>) -> Vec<(String, SummarizePhase, Option<String>)> {
            events
                .lock()
                .unwrap()
                .iter()
                .map(|p| (p.session_id.clone(), p.phase, p.error.clone()))
                .collect()
        }

        fn targets(ids: &[&str]) -> Vec<SummarizeTarget> {
            ids.iter()
                .map(|id| SummarizeTarget {
                    session_id: (*id).into(),
                    title: format!("Title {id}"),
                })
                .collect()
        }

        #[tokio::test]
        async fn forks_each_session_three_at_a_time() {
            let dir = tempfile::TempDir::new().unwrap();
            // Answers with the session id it resumed; fails for "bad".
            let claude = stand_in::write(
                dir.path(),
                r#"if [ "$3" = bad ]; then echo 'boom' >&2; echo 'last words' >&2; exit 3; fi
printf '{"type":"result","is_error":false,"result":"  Summary of %s  "}' "$3""#,
            );
            let env = EnvMap::new();
            let events = Mutex::new(Vec::new());
            let record = |p: SummarizeProgress| events.lock().unwrap().push(p);
            let cwd = dir.path().to_str().unwrap();
            let results = summarize_sessions(
                &claude,
                &env,
                cwd,
                &targets(&["a", "bad", "c", "d"]),
                "r1",
                &record,
                None,
            )
            .await;
            let summaries: Vec<(&str, &str, Option<&str>)> = results
                .iter()
                .map(|r| (r.session_id.as_str(), r.summary.as_str(), r.error.as_deref()))
                .collect();
            assert_eq!(
                summaries,
                [
                    ("a", "Summary of a", None),
                    ("bad", "", Some("boom last words")),
                    ("c", "Summary of c", None),
                    ("d", "Summary of d", None),
                ]
            );
            assert_eq!(results[0].title, "Title a");
            let seen = phases(&events);
            let queued: Vec<_> = seen
                .iter()
                .take(4)
                .map(|(id, phase, _)| (id.as_str(), *phase))
                .collect();
            assert_eq!(queued, [("a", Queued), ("bad", Queued), ("c", Queued), ("d", Queued)]);
            let running: Vec<_> = seen
                .iter()
                .skip(4)
                .take(3)
                .map(|(id, phase, _)| (id.as_str(), *phase))
                .collect();
            assert_eq!(running, [("a", Running), ("bad", Running), ("c", Running)]);
            assert!(seen.contains(&("bad".into(), Error, Some("boom last words".into()))));
            assert!(seen.contains(&("d".into(), Done, None)));
            assert_eq!(seen.len(), 4 * 3);

            let calls = stand_in::calls(dir.path());
            assert_eq!(calls.len(), 4);
            let a = calls.iter().find(|c| c[2] == "a").unwrap();
            assert_eq!(
                a,
                &[
                    "-p",
                    "--resume",
                    "a",
                    "--fork-session",
                    "--no-session-persistence",
                    "--output-format",
                    "json",
                    SUMMARY_PROMPT
                ]
            );
        }

        #[tokio::test]
        async fn uses_the_focused_prompt_for_a_topic() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(dir.path(), r#"echo '{"result":"ok"}'"#);
            let env = EnvMap::new();
            let cwd = dir.path().to_str().unwrap();
            let ignore = |_: SummarizeProgress| {};
            summarize_sessions(&claude, &env, cwd, &targets(&["a"]), "r", &ignore, Some(" auth ")).await;
            summarize_sessions(&claude, &env, cwd, &targets(&["b"]), "r", &ignore, Some("  ")).await;
            let calls = stand_in::calls(dir.path());
            let prompt_for = |id: &str| calls.iter().find(|c| c[2] == id).unwrap()[7].clone();
            assert_eq!(prompt_for("a"), focused_summary_prompt("auth"));
            assert_eq!(prompt_for("b"), SUMMARY_PROMPT);
        }

        #[tokio::test]
        async fn reports_an_error_reply_and_an_empty_one() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(
                dir.path(),
                r#"if [ "$3" = a ]; then echo '{"is_error":true,"result":"Credit balance is too low"}'; else echo '{"result":"   "}'; exit 0; fi"#,
            );
            let env = EnvMap::new();
            let ignore = |_: SummarizeProgress| {};
            let cwd = dir.path().to_str().unwrap();
            let results = summarize_sessions(&claude, &env, cwd, &targets(&["a", "b"]), "r", &ignore, None).await;
            assert_eq!(results[0].error.as_deref(), Some("Credit balance is too low"));
            // A blank result is no summary, and is itself the reason given.
            assert_eq!(results[1].error.as_deref(), Some("   "));
        }

        #[tokio::test]
        async fn falls_back_to_the_exit_code() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(dir.path(), "exit 0");
            let ignore = |_: SummarizeProgress| {};
            let cwd = dir.path().to_str().unwrap();
            let results = summarize_sessions(&claude, &EnvMap::new(), cwd, &targets(&["a"]), "r", &ignore, None).await;
            assert_eq!(results[0].error.as_deref(), Some("claude exited with code 0"));
        }

        #[tokio::test]
        async fn fails_a_session_whose_claude_wont_start() {
            let ignore = |_: SummarizeProgress| {};
            let missing = Path::new("/nowhere/claude");
            let results = summarize_sessions(missing, &EnvMap::new(), "/", &targets(&["a"]), "r", &ignore, None).await;
            assert_eq!(results[0].error.as_deref(), Some("spawn /nowhere/claude ENOENT"));
        }

        #[tokio::test(start_paused = true)]
        async fn gives_up_after_five_minutes() {
            let dir = tempfile::TempDir::new().unwrap();
            let claude = stand_in::write(dir.path(), "sleep 600");
            let env = EnvMap::new();
            let ignore = |_: SummarizeProgress| {};
            let cwd = dir.path().to_str().unwrap();
            let results = summarize_sessions(&claude, &env, cwd, &targets(&["a"]), "r", &ignore, None).await;
            assert_eq!(results[0].error.as_deref(), Some("Timed out after 5 minutes"));
        }
    }
}
