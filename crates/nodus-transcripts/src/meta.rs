//! A session's card metadata, parsed from its transcript and cached: the
//! title, first and last prompts, last reply, branch, counts and times.
//!
//! Transcripts are append-only, so the parsed state is kept per file, and when
//! the file has grown only the new bytes are read.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use indexmap::IndexMap;
use nodus_domain::SessionMeta;
use nodus_domain::js::{self, utf16_len};
use nodus_domain::json::number_to_string;
use serde_json::Value;

use crate::lines::{Json, read_lines};
use crate::parse::{assistant_parts, prompt_summary, time_of, truncate, user_text};
use crate::paths::{list_session_files, mtime_ms};
use crate::peers::Peers;
use crate::value::{field, strict_eq, truthy};

/// Over this, a transcript is read only at its head and tail (SESS-042).
pub const LARGE_FILE: u64 = 4 * 1024 * 1024;
pub const HEAD_BYTES: u64 = 256 * 1024;
pub const TAIL_BYTES: u64 = 1024 * 1024;

/// A session found on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredSession {
    pub meta: SessionMeta,
    /// Worth importing: a real conversation, not a sidechain or an empty/command-only file.
    pub eligible: bool,
    /// Has at least a prompt or a title; otherwise the meta is not worth showing.
    pub has_content: bool,
    /// When the user last sent a prompt; agents that finished before it belong to an earlier run.
    pub last_prompt_at: Option<f64>,
    /// Messages to and from other sessions (BOARD-184).
    pub peers: Peers,
}

/// What has been read of a transcript so far.
#[derive(Debug, Clone, Default)]
struct ParseState {
    size: u64,
    mtime_ms: f64,
    offset: u64,
    sidechain: Option<bool>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    first_prompt: Option<String>,
    first_command: Option<String>,
    last_prompt: Option<String>,
    last_prompt_at: Option<f64>,
    last_assistant_text: Option<String>,
    /// Any JSON value, as the transcript has it; only truthy ones are kept.
    last_assistant_id: Option<Value>,
    git_branch: Option<String>,
    prompts: u32,
    replies: u32,
    has_assistant: bool,
    created_at: Option<f64>,
    updated_at: Option<f64>,
    peers: Peers,
}

impl ParseState {
    fn fresh(size: u64, mtime_ms: f64) -> Self {
        Self {
            size,
            mtime_ms,
            ..Self::default()
        }
    }

    fn apply(&mut self, line: &Json) {
        // SESS-029: any line with a time.
        let t = time_of(line);
        if let Some(t) = t {
            if self.created_at.is_none_or(|c| t < c) {
                self.created_at = Some(t);
            }
            if self.updated_at.is_none_or(|u| t > u) {
                self.updated_at = Some(t);
            }
        }
        // SESS-030: any line with a branch; the last one wins.
        if let Some(branch) = line.get("gitBranch").and_then(Value::as_str).filter(|b| !b.is_empty()) {
            self.git_branch = Some(branch.to_owned());
        }
        self.peers.apply(line);
        let trimmed = |key: &str| {
            line.get(key)
                .and_then(Value::as_str)
                .map(js::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };

        // SESS-038: other types add only their time and branch.
        match line.get("type").and_then(Value::as_str) {
            // SESS-031
            Some("custom-title") => {
                if let Some(title) = trimmed("customTitle") {
                    self.custom_title = Some(title);
                }
            }
            // SESS-032
            Some("ai-title") => {
                if let Some(title) = trimmed("aiTitle") {
                    self.ai_title = Some(title);
                }
            }
            // SESS-033: older transcripts stored a generated title as a summary line.
            Some("summary") => {
                if self.ai_title.is_none()
                    && let Some(title) = trimmed("summary")
                {
                    self.ai_title = Some(title);
                }
            }
            Some("user") => {
                self.note_sidechain(line);
                // SESS-035
                let Some(u) = user_text(line, false) else {
                    return;
                };
                self.prompts += 1;
                if let Some(t) = t {
                    self.last_prompt_at = Some(self.last_prompt_at.unwrap_or(0.0).max(t));
                }
                if u.is_command {
                    self.first_command.get_or_insert(u.text);
                } else {
                    let summary = prompt_summary(&u.text);
                    self.first_prompt.get_or_insert_with(|| summary.clone());
                    self.last_prompt = Some(summary);
                }
            }
            Some("assistant") => {
                self.note_sidechain(line);
                // SESS-036
                let parts = assistant_parts(line);
                if !parts.text.is_empty() || !parts.tools.is_empty() {
                    self.has_assistant = true;
                }
                if parts.text.is_empty() {
                    return;
                }
                self.last_assistant_text = Some(parts.text);
                // One API message can be split across several lines; count it once.
                let id = field(line.get("message"), "id");
                if !truthy(id) || !strict_eq(id, self.last_assistant_id.as_ref()) {
                    self.replies += 1;
                }
                if truthy(id) {
                    self.last_assistant_id = id.cloned();
                }
            }
            _ => {}
        }
    }

    /// SESS-034: the first user or assistant line with a boolean flag settles it.
    /// SESS-037: sidechain lines still count like any other.
    fn note_sidechain(&mut self, line: &Json) {
        if self.sidechain.is_none()
            && let Some(flag) = line.get("isSidechain").and_then(Value::as_bool)
        {
            self.sidechain = Some(flag);
        }
    }

    /// `readSession`'s view of the state (SESS-044…049).
    fn discovered(&self, session_id: &str, file: &Path) -> DiscoveredSession {
        let mtime = self.mtime_ms;
        // `firstPrompt ?? firstCommand`: an empty first prompt still stands.
        let fallback_title = self.first_prompt.as_ref().or(self.first_command.as_ref());
        // Truthiness, as `||` and `?:` read them.
        fn set(s: &Option<String>) -> Option<&String> {
            s.as_ref().filter(|s| !s.is_empty())
        }
        let title = set(&self.custom_title)
            .or(set(&self.ai_title))
            .cloned()
            .unwrap_or_else(|| match fallback_title.filter(|t| !t.is_empty()) {
                Some(fallback) => truncate(fallback, 80),
                None => "Untitled session".into(),
            });
        let meta = SessionMeta {
            session_id: session_id.to_owned(),
            title,
            first_prompt: set(&self.first_prompt).map(|p| truncate(p, 500)),
            last_prompt: set(&self.last_prompt).map(|p| truncate(p, 500)),
            last_assistant_text: set(&self.last_assistant_text).map(|t| truncate(t, 400)),
            git_branch: self.git_branch.clone(),
            message_count: self.prompts + self.replies,
            created_at: self.created_at.unwrap_or(mtime),
            updated_at: self.updated_at.unwrap_or(mtime),
            transcript_path: file.to_string_lossy().into_owned(),
        };
        DiscoveredSession {
            meta,
            eligible: self.sidechain != Some(true) && self.prompts > 0 && self.has_assistant,
            has_content: self.prompts > 0 || set(&self.custom_title).is_some() || set(&self.ai_title).is_some(),
            last_prompt_at: self.last_prompt_at,
            peers: self.peers.clone(),
        }
    }
}

/// Parsed transcripts by file, for the life of the app.
#[derive(Debug, Default)]
pub struct MetaCache {
    files: Mutex<HashMap<PathBuf, ParseState>>,
}

impl MetaCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The session's metadata, read from what's new in its transcript since
    /// last time; none when the file can't be read (SESS-039…049).
    pub fn read_session(&self, session_id: &str, file: &Path) -> Option<DiscoveredSession> {
        let mut files = self.lock();
        // SESS-043: gone, so forgotten.
        let Ok(stat) = fs::metadata(file) else {
            files.remove(file);
            return None;
        };
        let size = stat.len();
        let mtime = mtime_ms(&stat);
        let cached = files.get_mut(file);
        // SESS-039
        if let Some(cached) = &cached
            && cached.size == size
            && cached.mtime_ms == mtime
        {
            return Some(cached.discovered(session_id, file));
        }
        // SESS-043: there but unreadable, so what was read before.
        let Ok(fd) = File::open(file) else {
            return cached.map(|c| c.discovered(session_id, file));
        };
        // SESS-040: append-only growth.
        if let Some(cached) = cached
            && size > cached.size
        {
            cached.offset = read_lines(&fd, cached.offset, size, false, |l| cached.apply(l), None);
            cached.size = size;
            cached.mtime_ms = mtime;
            return Some(cached.discovered(session_id, file));
        }
        // SESS-041
        let mut state = ParseState::fresh(size, mtime);
        if size > LARGE_FILE {
            // SESS-042: the head has the first prompt, the tail has titles and the latest reply.
            read_lines(&fd, 0, HEAD_BYTES, false, |l| state.apply(l), None);
            state.offset = read_lines(&fd, size - TAIL_BYTES, size, true, |l| state.apply(l), None);
        } else {
            state.offset = read_lines(&fd, 0, size, false, |l| state.apply(l), None);
        }
        let found = state.discovered(session_id, file);
        files.insert(file.to_owned(), state);
        Some(found)
    }

    /// Every session on disk for a repo whose metadata could be read, keyed by
    /// session id, in the folder's order (SESS-050).
    pub fn scan_repo(&self, home: &Path, repo_path: &str) -> IndexMap<String, DiscoveredSession> {
        let mut out = IndexMap::new();
        for f in list_session_files(home, repo_path) {
            if let Some(found) = self.read_session(&f.session_id, &f.file) {
                out.insert(f.session_id, found);
            }
        }
        out
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, ParseState>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A compact fingerprint used to detect meaningful changes for the watcher
/// (SESS-051).
pub fn meta_signature(meta: &SessionMeta) -> String {
    [
        meta.title.clone(),
        meta.message_count.to_string(),
        number_to_string(meta.updated_at),
        meta.git_branch.clone().unwrap_or_default(),
        meta.last_assistant_text
            .as_deref()
            .map(|t| utf16_len(t).to_string())
            .unwrap_or_default(),
    ]
    .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    const ID: &str = "00000000-0000-4000-8000-000000000001";

    fn ts(s: u32) -> String {
        format!("2026-11-01T10:00:{s:02}.000Z")
    }

    fn write_lines(file: &Path, lines: &[Value]) {
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(file, text).unwrap();
    }

    fn append(file: &Path, text: &str) {
        let mut f = fs::OpenOptions::new().append(true).open(file).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn prompt(text: &str, s: u32) -> Value {
        json!({ "type": "user", "timestamp": ts(s), "gitBranch": "main", "message": { "role": "user", "content": text } })
    }

    fn reply(text: &str, id: &str, s: u32) -> Value {
        json!({ "type": "assistant", "timestamp": ts(s), "message": { "id": id, "content": [{ "type": "text", "text": text }] } })
    }

    fn read(file: &Path) -> DiscoveredSession {
        MetaCache::new().read_session(ID, file).unwrap()
    }

    #[test]
    fn reads_a_conversation() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join(format!("{ID}.jsonl"));
        write_lines(
            &file,
            &[
                json!({ "type": "system", "timestamp": ts(1), "gitBranch": "dev" }),
                prompt("Fix the login", 2),
                reply("Looking", "m1", 3),
                reply("Still the same message", "m1", 4),
                json!({ "type": "assistant", "timestamp": ts(5), "message": { "id": "m2", "content": [{ "type": "tool_use", "name": "Bash", "input": {} }] } }),
                reply("Done", "m3", 6),
                prompt("<command-name>/clear</command-name>", 7),
                prompt("Thanks", 8),
                json!({ "type": "user", "message": { "content": [{ "type": "tool_result" }] } }),
            ],
        );
        let found = read(&file);
        let meta = &found.meta;
        assert_eq!(meta.title, "Fix the login");
        assert_eq!(meta.first_prompt.as_deref(), Some("Fix the login"));
        assert_eq!(meta.last_prompt.as_deref(), Some("Thanks"));
        assert_eq!(meta.last_assistant_text.as_deref(), Some("Done"));
        assert_eq!(meta.git_branch.as_deref(), Some("main"));
        // 3 prompts (the command counts) and 2 replies (m1 once, the tool-only line not at all).
        assert_eq!(meta.message_count, 5);
        assert_eq!(meta.created_at, date_ms(1));
        assert_eq!(meta.updated_at, date_ms(8));
        assert_eq!(meta.transcript_path, file.to_string_lossy());
        assert!(found.eligible && found.has_content);
        assert_eq!(found.last_prompt_at, Some(date_ms(8)));
    }

    fn date_ms(s: u32) -> f64 {
        crate::date::parse(&ts(s)).unwrap()
    }

    #[test]
    fn picks_the_title_custom_then_ai_then_summary_then_prompt() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        let title = |lines: &[Value]| {
            write_lines(&file, lines);
            read(&file).meta.title
        };
        assert_eq!(title(&[]), "Untitled session");
        assert_eq!(title(&[prompt("<command-name>/init</command-name>", 1)]), "/init");
        assert_eq!(title(&[prompt(&"word ".repeat(30), 1)]).chars().count(), 80);
        assert_eq!(
            title(&[json!({ "type": "summary", "summary": " Old " }), prompt("p", 1)]),
            "Old"
        );
        assert_eq!(
            title(&[
                json!({ "type": "summary", "summary": "Old" }),
                json!({ "type": "ai-title", "aiTitle": "AI" }),
                json!({ "type": "summary", "summary": "Older" }),
            ]),
            "AI"
        );
        assert_eq!(
            title(&[
                json!({ "type": "custom-title", "customTitle": " Mine " }),
                json!({ "type": "ai-title", "aiTitle": "AI" }),
                json!({ "type": "custom-title", "customTitle": "   " }),
            ]),
            "Mine"
        );
        // Not truncated.
        let long = "t".repeat(120);
        assert_eq!(title(&[json!({ "type": "ai-title", "aiTitle": long })]), long);
    }

    #[test]
    fn counts_replies_across_split_message_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        write_lines(
            &file,
            &[
                prompt("p", 1),
                reply("a", "m1", 2),
                // A text line without an id counts, and leaves the remembered id.
                json!({ "type": "assistant", "message": { "content": "b" } }),
                reply("c", "m1", 3),
                json!({ "type": "assistant", "message": { "id": 7, "content": "d" } }),
                json!({ "type": "assistant", "message": { "id": 7.0, "content": "e" } }),
                json!({ "type": "assistant", "message": { "id": {}, "content": "f" } }),
                json!({ "type": "assistant", "message": { "id": {}, "content": "g" } }),
            ],
        );
        // a, b, d, f, g; c and e repeat the id before them.
        assert_eq!(read(&file).meta.message_count, 1 + 5);
    }

    #[test]
    fn needs_a_prompt_and_a_reply_and_no_sidechain_to_import() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        let check = |lines: &[Value]| {
            write_lines(&file, lines);
            let found = read(&file);
            (found.eligible, found.has_content)
        };
        assert_eq!(check(&[prompt("p", 1)]), (false, true));
        assert_eq!(check(&[prompt("p", 1), reply("r", "m", 2)]), (true, true));
        let mut side = prompt("p", 1);
        side["isSidechain"] = json!(true);
        let mut later = reply("r", "m", 2);
        later["isSidechain"] = json!(false);
        assert_eq!(check(&[side.clone(), later]), (false, true));
        // Only the first flag counts, and a non-boolean one doesn't settle it.
        let mut odd = prompt("p", 1);
        odd["isSidechain"] = json!("yes");
        assert_eq!(check(&[odd, side, reply("r", "m", 2)]), (false, true));
        assert_eq!(check(&[json!({ "type": "ai-title", "aiTitle": "T" })]), (false, true));
        assert_eq!(check(&[reply("r", "m", 2)]), (false, false));
    }

    #[test]
    fn falls_back_to_the_files_mtime() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        write_lines(&file, &[json!({ "type": "user", "message": { "content": "p" } })]);
        let found = read(&file);
        let mtime = mtime_ms(&fs::metadata(&file).unwrap());
        assert_eq!((found.meta.created_at, found.meta.updated_at), (mtime, mtime));
        assert_eq!(found.last_prompt_at, None);
    }

    #[test]
    fn reads_appended_lines_as_a_fresh_parse_would() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        write_lines(&file, &[prompt("first", 1), reply("one", "m1", 2)]);
        let cache = MetaCache::new();
        let before = cache.read_session(ID, &file).unwrap();
        assert_eq!(before.meta.message_count, 2);
        // A line written in two parts is read once it's whole.
        let next = format!("{}\n", reply("two", "m2", 3));
        let (head, tail) = next.split_at(10);
        append(&file, head);
        assert_eq!(cache.read_session(ID, &file).unwrap().meta.message_count, 2);
        append(&file, tail);
        append(&file, &format!("{}\n", prompt("second", 4)));
        let grown = cache.read_session(ID, &file).unwrap();
        assert_eq!(grown, MetaCache::new().read_session(ID, &file).unwrap());
        assert_eq!(grown.meta.message_count, 4);
        assert_eq!(grown.meta.last_prompt.as_deref(), Some("second"));
    }

    #[test]
    fn forgets_a_file_that_goes() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        write_lines(&file, &[prompt("p", 1)]);
        let cache = MetaCache::new();
        assert!(cache.read_session(ID, &file).is_some());
        fs::remove_file(&file).unwrap();
        assert!(cache.read_session(ID, &file).is_none());
        assert!(cache.lock().is_empty());
    }

    #[test]
    fn reads_only_the_head_and_tail_of_a_large_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("t.jsonl");
        let mut text = format!("{}\n", prompt("The very first prompt", 1));
        let filler = format!("{}\n", prompt(&"x".repeat(1000), 2));
        // A prompt in the middle that's never read.
        while (text.len() as u64) < HEAD_BYTES + 10_000 {
            text.push_str(&filler);
        }
        text.push_str(&format!("{}\n", prompt("Hidden in the middle", 3)));
        while (text.len() as u64) < LARGE_FILE + TAIL_BYTES {
            text.push_str(&filler);
        }
        text.push_str(&format!("{}\n", reply("The latest reply", "m9", 9)));
        fs::write(&file, &text).unwrap();
        let found = read(&file);
        assert_eq!(found.meta.first_prompt.as_deref(), Some("The very first prompt"));
        assert_eq!(found.meta.last_assistant_text.as_deref(), Some("The latest reply"));
        let head = HEAD_BYTES as usize / filler.len() + 1;
        let tail = TAIL_BYTES as usize / filler.len();
        let prompts = found.meta.message_count - 1;
        assert!(prompts as usize <= head + tail + 1, "{prompts}");
        assert_ne!(found.meta.last_prompt.as_deref(), Some("Hidden in the middle"));
    }

    #[test]
    fn signs_metadata_for_the_watcher() {
        let meta = SessionMeta {
            session_id: ID.into(),
            title: "T".into(),
            first_prompt: None,
            last_prompt: None,
            last_assistant_text: Some("a😀".into()),
            git_branch: None,
            message_count: 3,
            created_at: 1.0,
            updated_at: 1_700_000_000_000.5,
            transcript_path: String::new(),
        };
        assert_eq!(meta_signature(&meta), "T|3|1700000000000.5||3");
        let bare = SessionMeta {
            last_assistant_text: None,
            git_branch: Some("main".into()),
            ..meta
        };
        assert_eq!(meta_signature(&bare), "T|3|1700000000000.5|main|");
    }
}
