//! Searches what was said and done in a board's sessions: the prompts,
//! Claude's replies, and its tool calls (paths, commands, and what it wrote),
//! its subagents' included. Tool output is left out: it's most of a
//! transcript's bytes, and a file a session only read would match everything
//! in it.
//!
//! Each transcript's searchable text is kept in memory, and like the metadata
//! it is read again only from where it left off when the file grows.
//!
//! Ports `src/main/search.ts` and the query helpers in `src/shared/search.ts`,
//! which the window also uses to highlight matches.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use nodus_domain::js::{self, utf16_len};
use nodus_domain::{SearchSnippet, SearchSnippetKind, SessionSearchHit};
use regex::Regex;
use serde_json::Value;

use crate::lines::{Json, read_lines};
use crate::parse::{assistant_parts, input_str, prompt_full, time_of, tool_name, use_input, user_text};
use crate::paths::{SEP, basename, list_agent_files, mtime_ms, real_repo_path, transcript_path};
use crate::value::{as_str, field, truthy};

/// More words than this add little but cost a pass over every message each.
const MAX_TERMS: usize = 8;
const MAX_RESULTS: usize = 50;
const SNIPPETS: usize = 3;
const SNIPPET_CHARS: usize = 160;
/// How much of the snippet comes before the match.
const SNIPPET_LEAD: usize = 48;

/// Tool input fields worth searching, paths first since they say what was touched.
const PATH_FIELDS: [&str; 3] = ["file_path", "notebook_path", "path"];
const TEXT_FIELDS: [&str; 10] = [
    "command",
    "pattern",
    "url",
    "query",
    "description",
    "skill",
    "prompt",
    // What it wrote.
    "content",
    "new_string",
    "old_string",
];

const AGENT_WEIGHT: f64 = 0.8;

fn kind_weight(kind: SearchSnippetKind) -> f64 {
    match kind {
        SearchSnippetKind::Prompt => 1.3,
        SearchSnippetKind::Tool => 1.1,
        SearchSnippetKind::Reply => 1.0,
    }
}

// ---------------------------------------------------------------------------
// Query helpers (src/shared/search.ts)

/// The words of a query, lowercased, each once, the first 8; longest first so
/// highlights prefer them (SESS-176).
pub fn query_terms(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    let mut seen = HashSet::new();
    let mut words: Vec<String> = lower
        .split(js::is_space)
        .filter(|w| !w.is_empty())
        .filter(|w| seen.insert(*w))
        .take(MAX_TERMS)
        .map(str::to_owned)
        .collect();
    // Stable, so words as long keep their order.
    words.sort_by_key(|w| std::cmp::Reverse(utf16_len(w)));
    words
}

/// Matches any of the terms, ignoring case; at one place the first (longest)
/// term wins (SESS-177).
pub fn terms_pattern<S: AsRef<str>>(terms: &[S]) -> Option<Regex> {
    if terms.is_empty() {
        return None;
    }
    let alternatives: Vec<String> = terms.iter().map(|t| regex::escape(t.as_ref())).collect();
    Regex::new(&format!("(?i){}", alternatives.join("|"))).ok()
}

/// Where the terms occur in `text`, as sorted [start, end) ranges in UTF-16
/// units with overlapping and touching ones merged (SESS-178).
pub fn match_ranges<S: AsRef<str>>(text: &str, terms: &[S]) -> Vec<[u32; 2]> {
    let Some(re) = terms_pattern(terms) else {
        return Vec::new();
    };
    let mut ranges: Vec<[u32; 2]> = Vec::new();
    // UTF-16 offsets, counted as the matches go by.
    let (mut byte, mut unit) = (0, 0);
    for m in re.find_iter(text) {
        unit += utf16_len(&text[byte..m.start()]);
        let start = unit;
        let end = start + utf16_len(m.as_str());
        unit = end;
        byte = m.end();
        let (start, end) = (start as u32, end as u32);
        match ranges.last_mut() {
            Some(last) if start <= last[1] => last[1] = last[1].max(end),
            _ => ranges.push([start, end]),
        }
    }
    ranges
}

// ---------------------------------------------------------------------------
// The index

/// A searchable message or tool call.
#[derive(Debug, Clone)]
struct Doc {
    kind: SearchSnippetKind,
    tool: Option<String>,
    /// For a tool call: the file it acted on, and where in `text` the path ends.
    file: Option<String>,
    path_end: Option<usize>,
    agent: bool,
    text: String,
    lower: String,
    at: f64,
}

impl Doc {
    fn new(kind: SearchSnippetKind, text: String, at: f64) -> Self {
        Self {
            kind,
            tool: None,
            file: None,
            path_end: None,
            agent: false,
            lower: text.to_lowercase(),
            text,
            at,
        }
    }
}

#[derive(Debug, Default)]
struct FileIndex {
    size: u64,
    mtime_ms: f64,
    offset: u64,
    docs: Vec<Doc>,
}

/// Tool results are most of a transcript and never searched, so their lines
/// aren't even parsed (SESS-186).
fn not_tool_result(raw: &str) -> bool {
    !raw.contains("\"type\":\"tool_result\"")
}

/// A tool call's paths and what it ran or wrote, as one document (SESS-185).
fn tool_doc(use_block: &Value, at: f64, agent: bool, roots: &[String]) -> Option<Doc> {
    let input = use_input(use_block);
    let relative = |p: &str| -> String {
        roots
            .iter()
            .find_map(|root| p.strip_prefix(root.as_str())?.strip_prefix(SEP))
            .unwrap_or(p)
            .to_owned()
    };
    let paths: Vec<String> = PATH_FIELDS
        .iter()
        .filter_map(|key| input_str(input, key))
        .map(relative)
        .collect();
    let rest: Vec<&str> = TEXT_FIELDS.iter().filter_map(|key| input_str(input, key)).collect();
    if paths.is_empty() && rest.is_empty() {
        return None;
    }
    let head = paths.join("\n");
    let text = std::iter::once(head.as_str())
        .filter(|h| !h.is_empty())
        .chain(rest)
        .collect::<Vec<_>>()
        .join("\n");
    let name = as_str(field(Some(use_block), "name")).unwrap_or("undefined");
    Some(Doc {
        tool: Some(tool_name(name).to_owned()),
        file: paths.first().filter(|p| !p.is_empty()).map(|p| basename(p).to_owned()),
        path_end: Some(utf16_len(&head)),
        agent,
        ..Doc::new(SearchSnippetKind::Tool, text, at)
    })
}

/// The searchable messages and tool calls in a line. A subagent's prompt is
/// its session's tool call, so only its work counts (SESS-184).
fn line_docs(line: &Json, agent: bool, roots: &[String], out: &mut Vec<Doc>) {
    if truthy(line.get("isSidechain")) && !agent {
        return;
    }
    let at = time_of(line).unwrap_or(0.0);
    match line.get("type").and_then(Value::as_str) {
        Some("user") if !agent => {
            if let Some(u) = user_text(line, false) {
                out.push(Doc::new(SearchSnippetKind::Prompt, prompt_full(&u.text), at));
            }
        }
        Some("assistant") => {
            let parts = assistant_parts(line);
            if !parts.text.is_empty() {
                out.push(Doc {
                    agent,
                    ..Doc::new(SearchSnippetKind::Reply, parts.text, at)
                });
            }
            out.extend(parts.uses.iter().filter_map(|u| tool_doc(u, at, agent, roots)));
        }
        _ => {}
    }
}

/// Each transcript's searchable text, by file, kept for the life of the app (SESS-187).
#[derive(Debug, Default)]
pub struct SearchIndex {
    files: Mutex<HashMap<PathBuf, FileIndex>>,
}

impl SearchIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lets go of what's been read.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, FileIndex>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether reading the file means parsing it, rather than answering from
    /// memory (SESS-183).
    fn stale(&self, file: &Path) -> bool {
        let files = self.lock();
        let Some(cached) = files.get(file) else {
            return true;
        };
        match fs::metadata(file) {
            Ok(stat) => stat.len() != cached.size || mtime_ms(&stat) != cached.mtime_ms,
            Err(_) => false,
        }
    }

    /// The sessions whose title and transcripts hold every word of the query,
    /// best first. A session ranks by how well its best matches fit: the whole
    /// phrase over all its words over some, and what the user asked or a tool
    /// touched over what Claude said (SESS-176…195).
    pub async fn search(
        &self,
        home: &Path,
        repo_path: &str,
        targets: &[SearchTarget],
        query: &str,
    ) -> Vec<SessionSearchHit> {
        // SESS-179
        let terms = query_terms(query);
        if terms.is_empty() {
            return Vec::new();
        }
        // SESS-180
        let normalized = js::collapse_spaces(js::trim(&query.to_lowercase()));
        let phrase = (terms.len() > 1).then_some(normalized.as_str());
        // SESS-181: paths show relative to the folder the session ran in.
        let roots_of = |folder: &str| {
            let mut roots = vec![folder.to_owned()];
            let real = real_repo_path(folder);
            if real != folder {
                roots.push(real);
            }
            roots
        };
        let project_roots = roots_of(repo_path);

        let mut scored: Vec<(SessionSearchHit, f64, f64)> = Vec::new();
        for target in targets {
            // SRV-125: a card in a worktree of its own keeps its transcripts there.
            let folder = target.folder.as_deref().unwrap_or(repo_path);
            let worktree_roots;
            let roots = if folder == repo_path {
                &project_roots
            } else {
                worktree_roots = roots_of(folder);
                &worktree_roots
            };
            // SESS-182
            let mut files = vec![(transcript_path(home, folder, &target.session_id), false)];
            files.extend(
                list_agent_files(home, folder, &target.session_id)
                    .into_iter()
                    .map(|a| (a.file, true)),
            );
            // SESS-183: reading a transcript the first time takes a moment, so let other work in between.
            if files.iter().any(|(file, _)| self.stale(file)) {
                tokio::task::yield_now().await;
            }
            if let Some(hit) = self.score(target, &files, &terms, phrase, roots) {
                scored.push(hit);
            }
        }

        // SESS-194
        scored.sort_by(|a, b| desc(a.1, b.1).then(desc(a.2, b.2)));
        scored.into_iter().take(MAX_RESULTS).map(|(hit, ..)| hit).collect()
    }

    /// A session's hit, its score and its latest match; none unless every term
    /// is in its title or somewhere in its transcripts.
    fn score(
        &self,
        target: &SearchTarget,
        files: &[(PathBuf, bool)],
        terms: &[String],
        phrase: Option<&str>,
        roots: &[String],
    ) -> Option<(SessionSearchHit, f64, f64)> {
        let n = terms.len() as f64;
        // SESS-188
        let title = target.title.to_lowercase();
        let mut covered: Vec<bool> = terms.iter().map(|term| title.contains(term.as_str())).collect();
        let title_words = covered.iter().filter(|c| **c).count();
        let title_score = if phrase.is_some_and(|p| title.contains(p)) || title_words == terms.len() {
            4.0
        } else {
            title_words as f64 / n
        };

        struct Scored<'a> {
            doc: &'a Doc,
            score: f64,
            all: bool,
        }
        let mut index = self.lock();
        for (file, agent) in files {
            index_file(&mut index, file, *agent, roots);
        }
        let mut hits: Vec<Scored<'_>> = Vec::new();
        for (file, _) in files {
            let Some(found) = index.get(file) else {
                continue;
            };
            for doc in &found.docs {
                // SESS-189
                let mut found_terms = 0;
                for (i, term) in terms.iter().enumerate() {
                    if doc.lower.contains(term.as_str()) {
                        found_terms += 1;
                        covered[i] = true;
                    }
                }
                if found_terms == 0 {
                    continue;
                }
                let all = found_terms == terms.len();
                let fit = if phrase.is_some_and(|p| doc.lower.contains(p)) {
                    3.0
                } else if all {
                    2.0
                } else {
                    found_terms as f64 / n
                };
                let agent_weight = if doc.agent { AGENT_WEIGHT } else { 1.0 };
                hits.push(Scored {
                    doc,
                    score: fit * kind_weight(doc.kind) * agent_weight,
                    all,
                });
            }
        }
        // SESS-190
        if !covered.iter().all(|c| *c) {
            return None;
        }

        // SESS-191
        hits.sort_by(|a, b| desc(a.score, b.score).then(desc(a.doc.at, b.doc.at)));
        let complete = hits.iter().filter(|h| h.all).count();
        let matches = if complete > 0 { complete } else { hits.len() };
        let best: f64 = hits.iter().take(5).map(|h| h.score).sum();
        // SESS-192: the same command run twice reads the same; show it once.
        let mut snippets: Vec<SearchSnippet> = Vec::new();
        for h in &hits {
            if snippets.len() == SNIPPETS {
                break;
            }
            let s = snippet(h.doc, terms, phrase);
            if !snippets.iter().any(|o| o.text == s.text && o.tool == s.tool) {
                snippets.push(s);
            }
        }
        let latest = hits.iter().fold(0.0_f64, |max, h| max.max(h.doc.at));
        let score = title_score + best + (1.0 + matches as f64).log2() / 2.0;
        let hit = SessionSearchHit {
            session_id: target.session_id.clone(),
            matches: matches as u32,
            snippets,
        };
        Some((hit, score, latest))
    }
}

/// A transcript's searchable text, read from where it was left off when the
/// file has grown (SESS-187).
fn index_file(index: &mut HashMap<PathBuf, FileIndex>, file: &Path, agent: bool, roots: &[String]) {
    let Ok(stat) = fs::metadata(file) else {
        index.remove(file);
        return;
    };
    let size = stat.len();
    let mtime = mtime_ms(&stat);
    if let Some(cached) = index.get(file)
        && cached.size == size
        && cached.mtime_ms == mtime
    {
        return;
    }
    // One that can't be opened keeps what was read before.
    let Ok(fd) = File::open(file) else {
        return;
    };
    let grown = index.get(file).is_some_and(|cached| size > cached.size);
    if !grown {
        index.insert(file.to_owned(), FileIndex::default());
    }
    let idx = index.get_mut(file).expect("just checked or inserted");
    let mut docs = std::mem::take(&mut idx.docs);
    idx.offset = read_lines(
        &fd,
        idx.offset,
        size,
        false,
        |line| line_docs(line, agent, roots, &mut docs),
        Some(&not_tool_result),
    );
    idx.docs = docs;
    idx.size = size;
    idx.mtime_ms = mtime;
}

/// A session to search: its id, the title it goes by on the board, and
/// the folder it runs in when that isn't the project's: a server card's
/// own worktree (SRV-125), where its transcripts are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchTarget {
    pub session_id: String,
    pub title: String,
    pub folder: Option<String>,
}

/// The part of a match's text worth showing: a line around the first place
/// it matched (SESS-193).
fn snippet(d: &Doc, terms: &[String], phrase: Option<&str>) -> SearchSnippet {
    let find = |re: Option<Regex>| re.and_then(|re| re.find(&d.text).map(|m| (m.start(), m.end())));
    let found = phrase
        .and_then(|p| find(terms_pattern(&[p])))
        .or_else(|| find(terms_pattern(terms)));
    let (at, match_end) = match found {
        Some((start, end)) => {
            let at = utf16_len(&d.text[..start]);
            (at, at + utf16_len(&d.text[start..end]))
        }
        None => (0, 0),
    };
    let units: Vec<u16> = d.text.encode_utf16().collect();
    // A tool call matched in its path shows the path; matched in what it wrote,
    // it shows that and names the file beside it.
    let in_path = d.path_end.is_some_and(|end| at < end);
    let path_end = d.path_end.unwrap_or(0);
    let base = if d.kind == SearchSnippetKind::Tool && !in_path && path_end > 0 {
        path_end + 1
    } else {
        0
    };
    let limit = if in_path { path_end } else { units.len() };
    let mut start = base.max(at.saturating_sub(SNIPPET_LEAD));
    // Start on a word.
    if start > base
        && start < at
        && let Some(space) = units[start..at].iter().position(|&u| is_space_unit(u))
    {
        start += space + 1;
    }
    let mut end = limit.min(start + SNIPPET_CHARS);
    if end < limit
        && match_end < end
        && let Some(space) = units[match_end..end].iter().rposition(|&u| u == u16::from(b' '))
        && space > 0
    {
        end = match_end + space;
    }
    let cut = if start < end {
        String::from_utf16_lossy(&units[start..end])
    } else {
        String::new()
    };
    let mut text = js::trim(&js::collapse_spaces(&cut)).to_owned();
    if start > base {
        text = format!("…{text}");
    }
    if end < limit {
        text.push('…');
    }
    SearchSnippet {
        kind: d.kind,
        tool: d.tool.clone(),
        file: if d.kind == SearchSnippetKind::Tool && !in_path {
            d.file.clone()
        } else {
            None
        },
        agent: d.agent.then_some(true),
        ranges: match_ranges(&text, terms),
        text,
        timestamp: d.at,
    }
}

/// `(a, b) => b - a`, as a comparator of a stable sort.
fn desc(a: f64, b: f64) -> std::cmp::Ordering {
    b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
}

/// A UTF-16 unit `\s` matches; all of them are in the BMP.
fn is_space_unit(u: u16) -> bool {
    char::from_u32(u32::from(u)).is_some_and(js::is_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lowercases_and_dedupes_words_longest_first() {
        assert_eq!(query_terms("  Auth  middleware AUTH "), ["middleware", "auth"]);
        assert_eq!(query_terms("a\u{FEFF}b"), ["a", "b"]);
        assert_eq!(
            query_terms("1 2 3 4 5 6 7 8 9"),
            ["1", "2", "3", "4", "5", "6", "7", "8"]
        );
        assert!(query_terms("   ").is_empty());
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

    #[test]
    fn counts_ranges_in_utf16_units() {
        assert_eq!(match_ranges("😀 Auth é auth", &["auth"]), [[3, 7], [10, 14]]);
        assert!(match_ranges::<&str>("text", &[]).is_empty());
    }
}
