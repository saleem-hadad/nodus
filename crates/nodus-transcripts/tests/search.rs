//! A port of `src/main/search.test.ts`: ⌘K search over a board's sessions
//! (SESS-175…195), and the query helpers the window highlights with.

#![allow(clippy::needless_pass_by_value)] // Helpers that build JSON lines read clearer taking values.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use nodus_domain::js::slice_utf16;
use nodus_domain::{SearchSnippetKind, SessionSearchHit};
use nodus_transcripts::{SearchTarget, Transcripts, match_ranges, query_terms};
use serde_json::{Value, json};

const AUTH: &str = "00000000-0000-4000-8000-000000000001";
const DOCS: &str = "00000000-0000-4000-8000-000000000002";
const AGENT: &str = "00000000-0000-4000-8000-000000000003";

/// Transcripts under a home of their own, with a clock that ticks a second a line.
struct Fixture {
    _home: tempfile::TempDir,
    repo: String,
    transcripts: Transcripts,
    clock: Cell<f64>,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::TempDir::new().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let fixture = Self {
            repo: repo.to_string_lossy().into_owned(),
            transcripts: Transcripts::new(home.path().to_owned()),
            _home: home,
            clock: Cell::new(nodus_transcripts::date::parse("2026-11-01T10:00:00Z").unwrap()),
        };
        fixture.write_sessions();
        fixture
    }

    fn at(&self) -> String {
        self.clock.set(self.clock.get() + 1000.0);
        nodus_transcripts::date::to_iso_string(self.clock.get())
    }

    fn prompt(&self, text: &str) -> Value {
        json!({ "type": "user", "timestamp": self.at(), "message": { "role": "user", "content": text } })
    }

    fn reply(&self, text: &str) -> Value {
        let timestamp = self.at();
        json!({
            "type": "assistant",
            "timestamp": timestamp,
            "message": { "id": format!("m{}", self.clock.get()), "role": "assistant", "content": [{ "type": "text", "text": text }] }
        })
    }

    fn tool(&self, name: &str, input: Value) -> Value {
        let timestamp = self.at();
        let clock = self.clock.get();
        json!({
            "type": "assistant",
            "timestamp": timestamp,
            "message": { "id": format!("m{clock}"), "role": "assistant", "content": [{ "type": "tool_use", "id": format!("t{clock}"), "name": name, "input": input }] }
        })
    }

    fn result(&self, content: &str) -> Value {
        json!({
            "type": "user",
            "timestamp": self.at(),
            "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t", "content": content }] }
        })
    }

    fn dir(&self) -> PathBuf {
        self.transcripts.transcript_dir(&self.repo)
    }

    fn in_repo(&self, rel: &[&str]) -> String {
        rel.iter()
            .fold(PathBuf::from(&self.repo), |p, part| p.join(part))
            .to_string_lossy()
            .into_owned()
    }

    fn write(&self, session_id: &str, lines: &[Value]) {
        self.write_file(&self.dir().join(format!("{session_id}.jsonl")), lines);
    }

    fn write_file(&self, file: &Path, lines: &[Value]) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let text: String = lines
            .iter()
            .map(|l| format!("{}\n", nodus_domain::json::to_string(l).unwrap()))
            .collect();
        fs::write(file, text).unwrap();
    }

    fn write_sessions(&self) {
        self.write(
            AUTH,
            &[
                self.prompt("The login redirect loops. Check the auth middleware please"),
                self.reply("The session cookie is read before it is set."),
                self.tool(
                    "Edit",
                    json!({
                        "file_path": self.in_repo(&["src", "middleware", "auth.ts"]),
                        "old_string": "const token = req.cookies.session",
                        "new_string": "const token = req.signedCookies.session"
                    }),
                ),
                self.result("export function authMiddleware() { /* the whole file, which mentions README */ }"),
                self.reply("Fixed: the middleware now reads the signed cookie."),
            ],
        );
        self.write(
            DOCS,
            &[
                self.prompt("Write a README for the project"),
                self.tool(
                    "Write",
                    json!({ "file_path": self.in_repo(&["README.md"]), "content": "# Project\n\nAuth is handled elsewhere." }),
                ),
                self.reply("Added the README."),
            ],
        );
        self.write(
            AGENT,
            &[
                self.prompt("Tidy up the repo"),
                self.reply("I asked an agent to look around."),
            ],
        );
        let mut look = self.prompt("Look for dead code (the session’s prompt to its agent)");
        look["isSidechain"] = json!(true);
        let mut grep = self.tool("Bash", json!({ "command": "rg -l signedCookies src" }));
        grep["isSidechain"] = json!(true);
        self.write_file(
            &self.dir().join(AGENT).join("subagents").join("agent-a1.jsonl"),
            &[look, grep],
        );
    }

    fn targets() -> Vec<SearchTarget> {
        [(AUTH, "Fix login"), (DOCS, "Write the README"), (AGENT, "Tidy up")]
            .iter()
            .map(|(id, title)| SearchTarget {
                session_id: (*id).into(),
                title: (*title).into(),
                folder: None,
            })
            .collect()
    }

    async fn search(&self, query: &str) -> Vec<SessionSearchHit> {
        self.transcripts.search(&self.repo, &Self::targets(), query).await
    }
}

fn ids(hits: &[SessionSearchHit]) -> Vec<&str> {
    hits.iter().map(|h| h.session_id.as_str()).collect()
}

fn sep_path(parts: &[&str]) -> String {
    parts.join(if cfg!(windows) { "\\" } else { "/" })
}

#[tokio::test]
async fn finds_the_session_whose_transcript_mentions_every_word_path_matches_included() {
    let f = Fixture::new();
    let hits = f.search("auth middleware").await;
    assert_eq!(ids(&hits), [AUTH]);
    assert_eq!(hits[0].matches, 2);
    let first = &hits[0].snippets[0];
    assert_eq!(first.kind, SearchSnippetKind::Prompt);
    assert!(first.text.contains("auth middleware"), "{}", first.text);
    let second = &hits[0].snippets[1];
    assert_eq!(second.kind, SearchSnippetKind::Tool);
    assert_eq!(second.tool.as_deref(), Some("Edit"));
    assert_eq!(second.text, sep_path(&["src", "middleware", "auth.ts"]));
}

#[tokio::test]
async fn names_the_file_when_the_match_is_in_what_a_tool_wrote() {
    let f = Fixture::new();
    let hits = f.search("signedCookies").await;
    assert_eq!(hits[0].session_id, AUTH);
    let edit = hits[0]
        .snippets
        .iter()
        .find(|s| s.kind == SearchSnippetKind::Tool && s.agent.is_none())
        .unwrap();
    assert_eq!(edit.tool.as_deref(), Some("Edit"));
    assert_eq!(edit.file.as_deref(), Some("auth.ts"));
    assert!(edit.text.contains("req.signedCookies.session"), "{}", edit.text);
    assert!(!edit.text.contains("src/middleware"));
}

#[tokio::test]
async fn searches_subagents_work_but_not_the_prompt_the_session_gave_them_twice() {
    let f = Fixture::new();
    let hits = f.search("signedCookies").await;
    let agent = hits.iter().find(|h| h.session_id == AGENT).unwrap();
    assert_eq!(agent.snippets.len(), 1);
    let s = &agent.snippets[0];
    assert_eq!(
        (s.kind, s.tool.as_deref(), s.agent),
        (SearchSnippetKind::Tool, Some("Bash"), Some(true))
    );
    assert!(f.search("dead code").await.is_empty());
}

#[tokio::test]
async fn leaves_tool_output_out() {
    let f = Fixture::new();
    assert!(f.search("authMiddleware").await.is_empty());
    assert_eq!(ids(&f.search("readme").await), [DOCS]);
}

#[tokio::test]
async fn needs_every_word_somewhere_in_the_session_or_its_title() {
    let f = Fixture::new();
    // "Auth is handled elsewhere" in the README it wrote.
    assert_eq!(ids(&f.search("auth readme").await), [DOCS]);
    assert_eq!(ids(&f.search("login cookie").await), [AUTH]);
}

#[tokio::test]
async fn ranks_the_whole_phrase_above_scattered_words() {
    let f = Fixture::new();
    f.write(DOCS, &[f.prompt("Middleware for auth, maybe later"), f.reply("Noted.")]);
    assert_eq!(ids(&f.search("auth middleware").await), [AUTH, DOCS]);
}

#[tokio::test]
async fn returns_title_only_matches_without_snippets() {
    let f = Fixture::new();
    assert_eq!(f.search("tidy").await[0].session_id, AGENT);
    let lone = SearchTarget {
        session_id: "00000000-0000-4000-8000-00000000000f".into(),
        title: "Tidy notes".into(),
        folder: None,
    };
    let hits = f.transcripts.search(&f.repo, &[lone], "tidy").await;
    assert_eq!(
        hits,
        [SessionSearchHit {
            session_id: "00000000-0000-4000-8000-00000000000f".into(),
            matches: 0,
            snippets: vec![],
        }]
    );
}

#[tokio::test]
async fn reads_what_a_live_session_adds_to_its_transcript() {
    let f = Fixture::new();
    assert!(f.search("websocket").await.is_empty());
    let file = f.dir().join(format!("{DOCS}.jsonl"));
    let line = nodus_domain::json::to_string(&f.prompt("Now add a websocket endpoint")).unwrap();
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str(&format!("{line}\n"));
    fs::write(&file, text).unwrap();
    assert_eq!(ids(&f.search("websocket").await), [DOCS]);
}

#[tokio::test]
async fn cuts_long_text_down_to_a_line_around_the_match() {
    let f = Fixture::new();
    let long = format!("{}the needle is here {}", "word ".repeat(100), "more ".repeat(100));
    f.write(DOCS, &[f.prompt(&long)]);
    let hits = f.search("needle").await;
    let snippet = &hits[0].snippets[0];
    let text = &snippet.text;
    assert!(text.starts_with('…') && text.ends_with('…'), "{text}");
    assert!(nodus_domain::js::utf16_len(text) < 180);
    let picked: Vec<String> = snippet
        .ranges
        .iter()
        .map(|[s, e]| slice_utf16(text, *s as usize, *e as usize))
        .collect();
    assert_eq!(picked, ["needle"]);
}

/// SRV-125: a server card runs in a worktree of its own, so its transcript
/// is under that folder's, not the project's, and paths show relative to it.
#[tokio::test]
async fn searches_a_cards_own_worktree() {
    let f = Fixture::new();
    let worktree = f._home.path().join("projects/shop/worktrees/fix-login");
    fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let card = "00000000-0000-4000-8000-0000000000aa";
    let file = worktree.clone() + &sep_path(&["", "src", "session.ts"]);
    f.write_file(
        &f.transcripts.transcript_dir(&worktree).join(format!("{card}.jsonl")),
        &[
            f.prompt("Rotate the session cookie on sign-in"),
            f.tool("Write", json!({ "file_path": file, "content": "rotateCookie()" })),
        ],
    );
    let target = |folder: Option<String>| SearchTarget {
        session_id: card.into(),
        title: "Rotate".into(),
        folder,
    };
    // Looked for in the main checkout, it isn't there.
    assert!(
        f.transcripts
            .search(&f.repo, &[target(None)], "cookie sign")
            .await
            .is_empty()
    );
    let hits = f
        .transcripts
        .search(
            &f.repo,
            &[target(Some(worktree)), Fixture::targets()[0].clone()],
            "session",
        )
        .await;
    let hit = hits.iter().find(|h| h.session_id == card).expect("the worktree's card");
    assert!(hit.snippets[0].text.contains("session cookie"), "{:?}", hit.snippets);
    let tool = hit.snippets.iter().find(|s| s.kind == SearchSnippetKind::Tool).unwrap();
    assert_eq!(tool.text, sep_path(&["src", "session.ts"]));
    // The project's own sessions are still found beside it.
    assert!(ids(&hits).contains(&AUTH), "{:?}", ids(&hits));
}

#[tokio::test]
async fn returns_nothing_for_an_empty_query() {
    let f = Fixture::new();
    assert!(f.search("   ").await.is_empty());
}

#[test]
fn splits_lowercases_and_dedupes_words_longest_first() {
    assert_eq!(query_terms("  Auth  middleware AUTH "), ["middleware", "auth"]);
}

#[test]
fn merges_overlapping_highlights_and_ignores_case() {
    assert_eq!(
        match_ranges("Auth middleware: authmiddleware", &["middleware", "auth"]),
        [[0, 4], [5, 15], [17, 31]]
    );
}

#[test]
fn treats_regex_characters_literally() {
    assert_eq!(match_ranges("call foo.bar() now", &["foo.bar()"]), [[5, 14]]);
}
