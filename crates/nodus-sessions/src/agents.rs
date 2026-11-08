//! Subagents a session started with Claude's Agent tool.
//!
//! Claude Code keeps each agent's transcript beside the session's, in
//! `<session id>/subagents/agent-<id>.jsonl`, next to an
//! `agent-<id>.meta.json` with its type and the description Claude gave it.
//! The meta file is rewritten with `finished` (or `stoppedByUser`) when the
//! agent ends. Transcripts are parsed incrementally, like the sessions' own.
//!
//! An agent can end its turn with a command still running in the background;
//! a task notification wakes it when that finishes. Until then it isn't done,
//! though its SubagentStop has fired and its meta file says `finished`.
//!
//! Nothing here watches the files (HOST-201): the project watcher notices
//! them changing and the board reloads, which reads agents again.
//!
//! Ports `src/main/agents.ts`. The line helpers it shares with discovery.ts
//! (`readLines`, `assistantParts`, `describeToolUse`, `messageText`,
//! `isTaskNotification`, `timeOf`, `truncate`) are nodus-transcripts'.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nodus_domain::{AgentInfo, SessionCard, SessionKind, SessionStatus};
use nodus_transcripts::paths::mtime_ms;
use nodus_transcripts::{
    Json, assistant_parts, describe_tool_use, is_task_notification, message_text, read_lines, time_of, truncate,
};
use serde_json::Value;

use crate::hooks::{Clock, StatusMachine};
use crate::jsvalue::{get, is_space, path as json_path, str_of, string, trim, truthy};

pub use nodus_transcripts::{AgentFile, is_agent_id};

/// Read in chunks this big, so a long agent transcript doesn't need one huge buffer.
const CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// HOST-183: the agents in a session's `subagents` folder, as 03's
/// `listAgentFiles` lists them given the folder: every `agent-<id>.jsonl`
/// with `<id>` of `[\w-]+`, in the order Node's readdir gives (by bytes on
/// macOS, as the file system lists them on Windows). None when the folder
/// can't be read.
pub fn list_agent_files(subagents_dir: &Path) -> Vec<AgentFile> {
    nodus_transcripts::list_agent_files_in(subagents_dir)
}

/// What `session:agent-transcript` says about ids that aren't an agent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Not an agent of this session")]
pub struct NotAnAgent;

/// HOST-200: the transcript `session:agent-transcript` reads (as 03's
/// `readTranscript` with `agent: true`), once both ids are known to be safe
/// in a path.
pub fn agent_transcript_file(subagents_dir: &Path, session_id: &str, agent_id: &str) -> Result<PathBuf, NotAnAgent> {
    if !is_agent_id(agent_id) || !is_agent_id(session_id) {
        return Err(NotAnAgent);
    }
    Ok(subagents_dir.join(format!("agent-{agent_id}.jsonl")))
}

/// Reads subagents' transcripts and meta files, keeping what it parsed.
///
/// Cheap to clone; clones share their caches.
#[derive(Clone)]
pub struct Agents {
    shared: Arc<Shared>,
}

struct Shared {
    /// `~/.claude/projects/`, with the separator: only transcripts under it can have pending work.
    projects_root: String,
    clock: Arc<dyn Clock>,
    metas: Mutex<HashMap<PathBuf, MetaState>>,
    parses: Mutex<HashMap<PathBuf, AgentParse>>,
}

#[derive(Debug, Clone)]
struct MetaState {
    mtime_ms: f64,
    /// The parsed file: an object or an array, as `typeof meta === 'object'` lets through.
    meta: Value,
}

#[derive(Debug, Clone, Default)]
struct AgentParse {
    size: u64,
    mtime_ms: f64,
    offset: u64,
    started_at: Option<f64>,
    updated_at: Option<f64>,
    tool_count: u32,
    /// The latest step: a tool call, or a reply.
    last: Option<Step>,
    /// When the agent finished its work, by its transcript: a final reply or a handback.
    ended_at: Option<f64>,
    /// Background tasks it started that haven't reported back yet, by task id.
    pending: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
struct Step {
    tool: Option<String>,
    text: Option<String>,
}

impl Agents {
    /// Agents of sessions whose transcripts are under `home`'s
    /// `.claude/projects`, timing replies without a timestamp by `clock`.
    pub fn new(home: &Path, clock: Arc<dyn Clock>) -> Self {
        let root = home.join(".claude").join("projects");
        Self {
            shared: Arc::new(Shared {
                projects_root: format!("{}{}", root.to_string_lossy(), std::path::MAIN_SEPARATOR),
                clock,
                metas: Mutex::default(),
                parses: Mutex::default(),
            }),
        }
    }

    /// HOST-189: whether the agent with this transcript has background work
    /// that will wake it again. Asked when its SubagentStop arrives, so it
    /// doesn't read as done meanwhile.
    pub fn has_pending_work(&self, transcript_path: &str) -> bool {
        let file = resolve(Path::new(transcript_path));
        let name = file.to_string_lossy();
        if !name.starts_with(&self.shared.projects_root) || !name.ends_with(".jsonl") {
            return false;
        }
        self.parse(&file).is_some_and(|parse| !parse.pending.is_empty())
    }

    /// HOST-198: every subagent the session has started, oldest first.
    /// `files` is what [`list_agent_files`] lists for the session.
    pub fn read_agents(&self, status: &StatusMachine, session_id: &str, files: &[AgentFile]) -> Vec<AgentInfo> {
        let mut agents: Vec<AgentInfo> = files
            .iter()
            .filter_map(|file| self.read_agent(status, session_id, file))
            .collect();
        // Stable, as Array.prototype.sort is.
        agents.sort_by(|a, b| a.started_at.total_cmp(&b.started_at));
        agents
    }

    /// HOST-199: adds a session's subagents to its card: how many it has
    /// started, and on an active card the current run's, which are those
    /// still running and those that finished since the user's last prompt
    /// (`last_prompt_at`, none when unknown).
    pub fn attach_agents(
        &self,
        status: &StatusMachine,
        card: &mut SessionCard,
        files: &[AgentFile],
        last_prompt_at: Option<f64>,
    ) {
        if card.node.kind == Some(SessionKind::Terminal) || files.is_empty() {
            return;
        }
        card.agent_count = Some(u32::try_from(files.len()).unwrap_or(u32::MAX));
        if card.node.archived {
            return;
        }
        let since = last_prompt_at.unwrap_or(0.0);
        let current: Vec<AgentInfo> = self
            .read_agents(status, &card.node.session_id, files)
            .into_iter()
            .filter(|agent| agent.status != SessionStatus::Done || agent.updated_at >= since)
            .collect();
        if !current.is_empty() {
            card.agents = Some(current);
        }
    }

    /// HOST-197
    fn read_agent(&self, status: &StatusMachine, session_id: &str, file: &AgentFile) -> Option<AgentInfo> {
        let parse = self.parse(&file.file)?;
        let meta_state = self.read_meta(&file.meta_file);
        let empty = Value::Object(Default::default());
        let meta = meta_state.as_ref().map_or(&empty, |state| &state.meta);
        let stopped = get(meta, "stoppedByUser") == Some(&Value::Bool(true));
        // HOST-192
        let meta_ended = (get(meta, "finished") == Some(&Value::Bool(true)) || stopped)
            .then(|| meta_state.as_ref().map(|state| state.mtime_ms))
            .flatten();
        let ended_at = match (parse.ended_at, meta_ended) {
            (None, None) => None,
            (parsed, meta) => Some(parsed.unwrap_or(0.0).max(meta.unwrap_or(0.0))),
        };
        let started_at = parse.started_at.unwrap_or(parse.mtime_ms);
        let status = status_of(status, session_id, &file.agent_id, &parse, ended_at);
        let waiting_on = if status == SessionStatus::Working && parse.ended_at.is_some() {
            parse.pending.len()
        } else {
            0
        };
        // `meta.x || fallback`; a value that isn't a string is written as String() writes it.
        let text = |key: &str| truthy(get(meta, key)).then(|| string(get(meta, key)));
        let last = parse.last.unwrap_or_default();
        Some(AgentInfo {
            agent_id: file.agent_id.clone(),
            agent_type: text("agentType").unwrap_or_else(|| "agent".into()),
            description: text("description")
                .or_else(|| text("name"))
                .unwrap_or_else(|| "Agent".into()),
            status,
            parent_agent_id: text("parentAgentId"),
            stopped: stopped.then_some(true),
            tool: last.tool,
            activity: last.text,
            background: (waiting_on > 0).then(|| u32::try_from(waiting_on).unwrap_or(u32::MAX)),
            tool_count: parse.tool_count,
            started_at,
            updated_at: parse.updated_at.unwrap_or(started_at),
        })
    }

    /// HOST-184: the meta file, cached by mtime. A file mid-rewrite keeps
    /// what was read before; a missing one is forgotten.
    fn read_meta(&self, file: &Path) -> Option<MetaState> {
        let mut metas = lock(&self.shared.metas);
        let Ok(stat) = fs::metadata(file) else {
            metas.remove(file);
            return None;
        };
        let mtime_ms = mtime_ms(&stat);
        let cached = metas.get(file);
        if let Some(cached) = cached
            && cached.mtime_ms == mtime_ms
        {
            return Some(cached.clone());
        }
        let parsed = fs::read(file)
            .ok()
            .and_then(|bytes| nodus_domain::json::from_str::<Value>(&String::from_utf8_lossy(&bytes)).ok());
        let Some(meta) = parsed else {
            // Mid-rewrite: keep what we had.
            return cached.cloned();
        };
        let meta = if meta.is_object() || meta.is_array() {
            meta
        } else {
            Value::Object(Default::default())
        };
        let state = MetaState { mtime_ms, meta };
        metas.insert(file.to_owned(), state.clone());
        Some(state)
    }

    /// HOST-185: the transcript parsed so far. Append-only growth continues
    /// from where it stopped; anything else starts over.
    fn parse(&self, file: &Path) -> Option<AgentParse> {
        let mut parses = lock(&self.shared.parses);
        let Ok(stat) = fs::metadata(file) else {
            parses.remove(file);
            return None;
        };
        let (size, mtime_ms) = (stat.len(), mtime_ms(&stat));
        let cached = parses.get(file);
        if let Some(cached) = cached
            && cached.size == size
            && cached.mtime_ms == mtime_ms
        {
            return Some(cached.clone());
        }
        let Ok(handle) = File::open(file) else {
            return cached.cloned();
        };
        let mut s = match cached {
            Some(cached) if size > cached.size => cached.clone(),
            _ => AgentParse::default(),
        };
        let mut chunk = CHUNK_BYTES;
        while s.offset < size {
            let (start, end) = (s.offset, size.min(s.offset + chunk));
            let clock = &self.shared.clock;
            let next = read_lines(
                &handle,
                start,
                end,
                false,
                |line| apply_line(&mut s, line, &**clock),
                None,
            );
            if next > s.offset {
                s.offset = next;
                chunk = CHUNK_BYTES;
            } else if end < size {
                // One line longer than the chunk: widen it.
                chunk *= 2;
            } else {
                // The last line is still being written.
                break;
            }
        }
        s.size = size;
        s.mtime_ms = mtime_ms;
        parses.insert(file.to_owned(), s.clone());
        Some(s)
    }
}

/// HOST-193 to HOST-196: hooks decide while the session's process runs; an
/// agent no hook reported belongs to an earlier process, so it's done. One
/// that ended its turn with background work in flight will be woken by it,
/// so it's still working. When the transcript or the meta file says the agent
/// ended after its last hook, it ended without a SubagentStop reaching the app.
fn status_of(
    status: &StatusMachine,
    session_id: &str,
    agent_id: &str,
    parse: &AgentParse,
    ended_at: Option<f64>,
) -> SessionStatus {
    if !status.is_live(session_id) {
        return SessionStatus::Done;
    }
    let Some(hook) = status.agent_hook(session_id, agent_id) else {
        return SessionStatus::Done;
    };
    if parse.ended_at.is_some() && !parse.pending.is_empty() {
        status.hold_agent(session_id, agent_id);
        return SessionStatus::Working;
    }
    if hook.status == SessionStatus::Done {
        return SessionStatus::Done;
    }
    if ended_at.is_some_and(|ended| ended >= hook.at) {
        status.settle_agent(session_id, agent_id);
        return SessionStatus::Done;
    }
    hook.status
}

/// HOST-186 to HOST-191: one transcript line. (A line that isn't an object
/// adds nothing: JSON.parse gives null for a `null` line, and reading its
/// timestamp throws.)
fn apply_line(s: &mut AgentParse, line: &Json, clock: &dyn Clock) {
    let t = time_of(line);
    if let Some(t) = t {
        if s.started_at.is_none_or(|started| t < started) {
            s.started_at = Some(t);
        }
        if s.updated_at.is_none_or(|updated| t > updated) {
            s.updated_at = Some(t);
        }
    }
    let field = |key: &str| line.get(key);
    match field("type").and_then(Value::as_str) {
        Some("user") => {
            // A command it sent to the background, and later the notification that it ended.
            if let Some(task) =
                str_of(field("toolUseResult").and_then(|r| get(r, "backgroundTaskId"))).filter(|t| !t.is_empty())
            {
                s.pending.insert(task.to_owned());
            }
            // The notification wakes the agent to deal with what finished.
            if is_task_notification(line) {
                for task in task_ids(&message_text(line)) {
                    s.pending.remove(trim(task));
                }
                s.ended_at = None;
            }
            // A new message (the session writing to a resumed agent) starts more work.
            let content = field("message").and_then(|m| get(m, "content"));
            let is_prompt = match content {
                Some(Value::String(_)) => true,
                Some(Value::Array(blocks)) => !blocks.iter().any(|b| block_type(b) == Some("tool_result")),
                _ => false,
            };
            if is_prompt && !truthy(field("isMeta")) {
                s.ended_at = None;
            }
        }
        Some("assistant") => {
            let parts = assistant_parts(line);
            s.tool_count += u32::try_from(parts.uses.len()).unwrap_or(u32::MAX);
            if let Some(last) = parts.uses.last() {
                let call = describe_tool_use(last);
                s.last = Some(Step {
                    tool: Some(call.name),
                    text: call.detail,
                });
                s.ended_at = None;
            } else if !parts.text.is_empty() {
                s.last = Some(Step {
                    tool: None,
                    text: Some(reply_line(&parts.text)),
                });
            }
            let end_turn = field("message")
                .and_then(|m| json_path(m, &["stop_reason"]))
                .and_then(Value::as_str)
                == Some("end_turn");
            if parts.handback || (end_turn && parts.uses.is_empty()) {
                s.ended_at = Some(t.unwrap_or_else(|| clock.now()));
            }
        }
        _ => {}
    }
}

/// HOST-191: first line of a reply, without the Markdown headings agents
/// like to open their reports with.
fn reply_line(text: &str) -> String {
    let lines: Vec<&str> = text
        .split('\n')
        .filter(|line| !trim(line).is_empty() && !is_heading(line))
        .collect();
    let joined = lines.join(" ");
    let kept = if joined.is_empty() { text } else { &joined };
    truncate(&strip_emphasis(kept), 200)
}

/// `/^\s{0,3}#{1,6}\s/`
fn is_heading(line: &str) -> bool {
    let indent = line.chars().take_while(|&c| is_space(c)).count();
    if indent > 3 {
        return false;
    }
    let rest: String = line.chars().skip(indent).collect();
    let hashes = rest.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && rest.chars().nth(hashes).is_some_and(is_space)
}

/// `.replace(/(\*\*|__|`)/g, '')`
fn strip_emphasis(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("**") || rest.starts_with("__") {
            rest = &rest[2..];
        } else if c == '`' {
            rest = &rest[1..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The ids in `/<task-id>([^<]+)<\/task-id>/g`, untrimmed.
fn task_ids(text: &str) -> Vec<&str> {
    const OPEN: &str = "<task-id>";
    const CLOSE: &str = "</task-id>";
    let mut ids = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(OPEN) {
        let start = from + found + OPEN.len();
        let rest = &text[start..];
        match rest.find('<') {
            Some(len) if len > 0 && rest[len..].starts_with(CLOSE) => {
                ids.push(&rest[..len]);
                from = start + len + CLOSE.len();
            }
            // No match here; the search goes on from the next character.
            _ => from += found + 1,
        }
    }
    ids
}

fn block_type(block: &Value) -> Option<&str> {
    get(block, "type").and_then(Value::as_str)
}

/// Node's `path.resolve`: absolute against the working folder, with `.` and
/// `..` worked out by name, links left alone.
fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_lines_drop_headings_and_emphasis() {
        assert_eq!(reply_line("# Report\n\n**Done** with `tests`"), "Done with tests");
        assert_eq!(reply_line("# Only a heading"), "# Only a heading");
        assert_eq!(
            reply_line("####### Seven\n    # Four spaces\n   # Three"),
            "####### Seven # Four spaces"
        );
        assert_eq!(reply_line("***bold*** and ____"), "*bold* and");
        assert_eq!(reply_line("#\tTabbed\nkept"), "kept");
    }

    #[test]
    fn finds_task_ids_as_the_regex_does() {
        assert_eq!(
            task_ids("<task-id> a </task-id> <task-id>b</task-id>"),
            vec![" a ", "b"]
        );
        assert_eq!(task_ids("<task-id></task-id><task-id>c</task-id>"), vec!["c"]);
        assert_eq!(task_ids("<task-id>x<b></task-id>"), Vec::<&str>::new());
        assert_eq!(task_ids("<task-id>x"), Vec::<&str>::new());
        assert_eq!(task_ids("<task-id>a\nb</task-id>"), vec!["a\nb"]);
    }

    #[test]
    fn resolves_paths_by_name() {
        if cfg!(unix) {
            assert_eq!(resolve(Path::new("/a/b/../c/./d.jsonl")), PathBuf::from("/a/c/d.jsonl"));
            assert_eq!(resolve(Path::new("/../a")), PathBuf::from("/a"));
            assert_eq!(resolve(Path::new("//a//b")), PathBuf::from("/a/b"));
        }
    }

    #[test]
    fn lists_agent_files_by_their_names() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "agent-b.jsonl",
            "agent-a-1_x.jsonl",
            "agent-a.meta.json",
            "agent-.jsonl",
            "agent-a b.jsonl",
            "agent-é.jsonl",
            "notes.jsonl",
        ] {
            fs::write(dir.path().join(name), "").unwrap();
        }
        let files = list_agent_files(dir.path());
        let ids: Vec<&str> = files.iter().map(|f| f.agent_id.as_str()).collect();
        if cfg!(unix) {
            assert_eq!(ids, vec!["a-1_x", "b"]);
        } else {
            assert_eq!(ids.len(), 2);
        }
        let b = files.iter().find(|f| f.agent_id == "b").unwrap();
        assert_eq!(b.file, dir.path().join("agent-b.jsonl"));
        assert_eq!(b.meta_file, dir.path().join("agent-b.meta.json"));
        assert!(list_agent_files(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn agent_transcripts_need_safe_ids() {
        // HOST-200
        let dir = Path::new("subagents");
        assert_eq!(
            agent_transcript_file(dir, "s-1", "a_1"),
            Ok(dir.join("agent-a_1.jsonl"))
        );
        for (session, agent) in [("s1", "../x"), ("s/1", "a1"), ("", "a1"), ("s1", "")] {
            let error = agent_transcript_file(dir, session, agent).unwrap_err();
            assert_eq!(error.to_string(), "Not an agent of this session");
        }
    }
}
