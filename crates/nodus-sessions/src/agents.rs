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
//! Ports `src/main/agents.ts`. It needs a few of discovery.ts's line helpers
//! (the private ones at the bottom), which the transcripts crate owns; they
//! can move to it once both are in.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::UNIX_EPOCH;

use nodus_domain::{AgentInfo, SessionCard, SessionKind, SessionStatus, ToolCall, js};
use serde_json::Value;

use crate::hooks::{Clock, StatusMachine};
use crate::jsvalue::{collapse_spaces, get, is_space, path as json_path, str_of, string, trim, trim_end, truthy};

/// Read in chunks this big, so a long agent transcript doesn't need one huge buffer.
const CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// The tool a background agent calls to deliver its report; its message is the agent's last word.
const HANDBACK_TOOL: &str = "SubagentHandback";

/// A subagent's files (discovery.ts's `AgentFile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    pub agent_id: String,
    /// `agent-<id>.jsonl`, its transcript.
    pub file: PathBuf,
    /// `agent-<id>.meta.json`: the agent's type and description, rewritten when it ends.
    pub meta_file: PathBuf,
}

/// HOST-183: the agents in a session's `subagents` folder, as 03's
/// `listAgentFiles` lists them given the folder: every `agent-<id>.jsonl`
/// with `<id>` of `[\w-]+`, in the order Node's readdir gives (by bytes on
/// macOS, as the file system lists them on Windows). None when the folder
/// can't be read.
pub fn list_agent_files(subagents_dir: &Path) -> Vec<AgentFile> {
    let Ok(entries) = fs::read_dir(subagents_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    if cfg!(unix) {
        // libuv's scandir sorts with strcmp.
        names.sort_unstable();
    }
    names
        .into_iter()
        .filter_map(|name| {
            let id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
            is_agent_id(id).then(|| AgentFile {
                agent_id: id.to_owned(),
                file: subagents_dir.join(&name),
                meta_file: subagents_dir.join(format!("agent-{id}.meta.json")),
            })
        })
        .collect()
}

/// `/^[\w-]+$/`: agent and session ids end up in paths, so only ever accept this shape.
pub fn is_agent_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
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
        let Ok(mut handle) = File::open(file) else {
            return cached.cloned();
        };
        let mut s = match cached {
            Some(cached) if size > cached.size => cached.clone(),
            _ => AgentParse::default(),
        };
        let mut chunk = CHUNK_BYTES;
        while s.offset < size {
            let end = size.min(s.offset + chunk);
            let clock = &self.shared.clock;
            let Ok(next) = read_lines(&mut handle, s.offset, end, |line| apply_line(&mut s, &line, &**clock)) else {
                break;
            };
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

/// HOST-186 to HOST-191: one transcript line.
fn apply_line(s: &mut AgentParse, line: &Value, clock: &dyn Clock) {
    // JSON.parse gives null for a `null` line, and reading its timestamp throws.
    if line.is_null() {
        return;
    }
    let t = time_of(line);
    if let Some(t) = t {
        if s.started_at.is_none_or(|started| t < started) {
            s.started_at = Some(t);
        }
        if s.updated_at.is_none_or(|updated| t > updated) {
            s.updated_at = Some(t);
        }
    }
    match get(line, "type").and_then(Value::as_str) {
        Some("user") => {
            // A command it sent to the background, and later the notification that it ended.
            if let Some(task) =
                str_of(json_path(line, &["toolUseResult", "backgroundTaskId"])).filter(|t| !t.is_empty())
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
            let content = json_path(line, &["message", "content"]);
            let is_prompt = match content {
                Some(Value::String(_)) => true,
                Some(Value::Array(blocks)) => !blocks.iter().any(|b| block_type(b) == Some("tool_result")),
                _ => false,
            };
            if is_prompt && !truthy(get(line, "isMeta")) {
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
            let end_turn = json_path(line, &["message", "stop_reason"]).and_then(Value::as_str) == Some("end_turn");
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

// ---------------------------------------------------------------------------
// discovery.ts's line helpers, as agents.ts uses them.

/// discovery.ts `truncate`: whitespace collapsed, and at most `max` UTF-16
/// units, ending in `…` when cut.
fn truncate(text: &str, max: usize) -> String {
    let flat = collapse_spaces(text);
    let flat = trim(&flat);
    if js::utf16_len(flat) > max {
        format!("{}…", trim_end(&js::slice_utf16(flat, 0, max - 1)))
    } else {
        flat.to_owned()
    }
}

struct AssistantParts<'a> {
    text: String,
    /// The tool_use blocks, the handback left out.
    uses: Vec<&'a Value>,
    /// A background agent handed in its report: `text` ends with it.
    handback: bool,
}

/// discovery.ts `assistantParts`.
fn assistant_parts(line: &Value) -> AssistantParts<'_> {
    let blocks = match json_path(line, &["message", "content"]) {
        Some(Value::String(content)) => {
            return AssistantParts {
                text: trim(content).to_owned(),
                uses: Vec::new(),
                handback: false,
            };
        }
        Some(Value::Array(blocks)) => blocks,
        _ => {
            return AssistantParts {
                text: String::new(),
                uses: Vec::new(),
                handback: false,
            };
        }
    };
    let mut texts: Vec<&str> = blocks
        .iter()
        .filter(|b| block_type(b) == Some("text"))
        .filter_map(|b| str_of(get(b, "text")))
        .collect();
    let mut uses = Vec::new();
    let mut handback = false;
    for block in blocks {
        let Some(name) = str_of(get(block, "name")).filter(|_| block_type(block) == Some("tool_use")) else {
            continue;
        };
        // The handback is the agent's reply, not a step of its work.
        if name == HANDBACK_TOOL {
            handback = true;
            if let Some(message) = str_of(json_path(block, &["input", "message"])) {
                texts.push(message);
            }
        } else {
            uses.push(block);
        }
    }
    AssistantParts {
        text: trim(&texts.join("\n")).to_owned(),
        uses,
        handback,
    }
}

/// discovery.ts `toolName`: MCP tools are named `mcp__<server>__<tool>`; the tool's own name reads best.
fn tool_name(name: &str) -> &str {
    if !name.starts_with("mcp__") {
        return name;
    }
    let own = name.rfind("__").map_or("", |at| &name[at + 2..]);
    if own.is_empty() { name } else { own }
}

/// discovery.ts `describeToolUse`: a tool_use block as a name and a one-line
/// summary of what it acted on.
fn describe_tool_use(block: &Value) -> ToolCall {
    let input = get(block, "input").filter(|input| input.is_object() || input.is_array());
    let field = |key: &str| str_of(input.and_then(|input| get(input, key))).filter(|value| !trim(value).is_empty());
    let detail = field("command")
        .map(str::to_owned)
        .or_else(|| {
            field("file_path")
                .or_else(|| field("notebook_path"))
                .map(|file| basename(file).to_owned())
        })
        .or_else(|| {
            ["pattern", "url", "query", "description", "skill", "path", "prompt"]
                .into_iter()
                .find_map(field)
                .map(str::to_owned)
        });
    ToolCall {
        name: tool_name(&string(get(block, "name"))).to_owned(),
        detail: detail
            .filter(|detail| !detail.is_empty())
            .map(|detail| truncate(&detail, 160)),
    }
}

/// Node's `path.basename`, for the platform it runs on.
fn basename(path: &str) -> &str {
    let is_separator = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let mut path = path;
    if cfg!(windows) {
        let bytes = path.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            path = &path[2..];
        }
    }
    let trimmed = path.trim_end_matches(is_separator);
    trimmed.rfind(is_separator).map_or(trimmed, |at| &trimmed[at + 1..])
}

/// discovery.ts `messageText`: all the text in a line's message, whether its content is a string or blocks.
fn message_text(line: &Value) -> String {
    match json_path(line, &["message", "content"]) {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|b| str_of(get(b, "text")).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// discovery.ts `isTaskNotification`: a background command (or task) reporting that it ended.
fn is_task_notification(line: &Value) -> bool {
    json_path(line, &["origin", "kind"]).and_then(Value::as_str) == Some("task-notification")
        || message_text(line).contains("<task-notification>")
}

fn block_type(block: &Value) -> Option<&str> {
    get(block, "type").and_then(Value::as_str)
}

/// discovery.ts `timeOf`: the line's `timestamp`, as `Date.parse` reads it.
fn time_of(line: &Value) -> Option<f64> {
    parse_date(str_of(get(line, "timestamp"))?)
}

/// `Date.parse` for the timestamps Claude Code writes (`toISOString()`), and
/// the variants V8 reads the same way: dates alone, times to the minute or
/// with any number of fraction digits (milliseconds kept, the rest dropped),
/// `t`, `z` or a space for `T` and `Z`, offsets `±HH:mm` or `±HHmm`, and
/// six-digit years with a sign. A date and time without an offset is local
/// time to V8; that and V8's other lenient formats read as none here.
fn parse_date(text: &str) -> Option<f64> {
    let mut p = DateParser { rest: text.as_bytes() };
    let year = match p.peek() {
        Some(sign @ (b'+' | b'-')) => {
            p.rest = &p.rest[1..];
            let year = p.digits(6)?;
            if sign == b'-' && year == 0 {
                return None;
            }
            if sign == b'-' { -year } else { year }
        }
        _ => p.digits(4)?,
    };
    let (mut month, mut day) = (1, 1);
    if p.eat(b'-') {
        month = p.digits(2)?;
        if p.eat(b'-') {
            day = p.digits(2)?;
        }
    }
    let (mut hour, mut minute, mut second, mut ms) = (0, 0, 0, 0);
    let mut offset_minutes = 0;
    let has_time = matches!(p.peek(), Some(b'T' | b't' | b' '));
    if has_time {
        p.rest = &p.rest[1..];
        hour = p.digits(2)?;
        if !p.eat(b':') {
            return None;
        }
        minute = p.digits(2)?;
        if p.eat(b':') {
            second = p.digits(2)?;
            if p.eat(b'.') {
                let fraction = p.rest.iter().take_while(|b| b.is_ascii_digit()).count();
                if fraction == 0 {
                    return None;
                }
                let digits = &p.rest[..fraction];
                ms = digits
                    .iter()
                    .chain(std::iter::repeat(&b'0'))
                    .take(3)
                    .fold(0, |n, d| n * 10 + i64::from(d - b'0'));
                p.rest = &p.rest[fraction..];
            }
        }
    }
    match p.peek() {
        Some(b'Z' | b'z') => p.rest = &p.rest[1..],
        Some(sign @ (b'+' | b'-')) if has_time => {
            p.rest = &p.rest[1..];
            let hours = p.digits(2)?;
            p.eat(b':');
            let minutes = p.digits(2)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            offset_minutes = (hours * 60 + minutes) * if sign == b'+' { 1 } else { -1 };
        }
        // Local time.
        None if has_time => return None,
        None => {}
        Some(_) => return None,
    }
    if !p.rest.is_empty()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 24
        || minute > 59
        || second > 59
        || (hour == 24 && (minute > 0 || second > 0 || ms > 0))
    {
        return None;
    }
    let days = days_from_civil(year, month, 1) + day - 1;
    let time = (((days * 24 + hour) * 60 + minute - offset_minutes) * 60 + second) * 1000 + ms;
    // TimeClip
    (time.abs() <= 8_640_000_000_000_000).then_some(time as f64)
}

struct DateParser<'a> {
    rest: &'a [u8],
}

impl DateParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.rest.first().copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.rest = &self.rest[1..];
        }
        found
    }

    /// Exactly `count` digits.
    fn digits(&mut self, count: usize) -> Option<i64> {
        let digits = self.rest.get(..count)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.rest = &self.rest[count..];
        Some(digits.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    }
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// discovery.ts `readLines`: parses the complete JSON lines in bytes
/// `start..end` and returns the offset after the last newline. Lines that
/// don't parse are skipped.
fn read_lines(file: &mut File, start: u64, end: u64, mut on_line: impl FnMut(Value)) -> std::io::Result<u64> {
    let length = usize::try_from(end.saturating_sub(start)).unwrap_or(usize::MAX);
    if length == 0 {
        return Ok(start);
    }
    let mut buffer = Vec::with_capacity(length);
    file.seek(SeekFrom::Start(start))?;
    file.take(length as u64).read_to_end(&mut buffer)?;
    let Some(last_newline) = buffer.iter().rposition(|&b| b == b'\n') else {
        return Ok(start);
    };
    let text = String::from_utf8_lossy(&buffer[..last_newline]);
    for raw in text.split('\n') {
        if raw.is_empty() {
            continue;
        }
        if let Ok(line) = nodus_domain::json::from_str::<Value>(raw) {
            on_line(line);
        }
    }
    Ok(start + last_newline as u64 + 1)
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

/// `stat.mtimeMs` as Node works it out: seconds × 1000 + nanoseconds / 10⁶.
fn mtime_ms(stat: &Metadata) -> f64 {
    let Ok(modified) = stat.modified() else {
        return 0.0;
    };
    match modified.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as f64 * 1000.0 + f64::from(since.subsec_nanos()) / 1e6,
        Err(before) => {
            let before = before.duration();
            -(before.as_secs() as f64 * 1000.0 + f64::from(before.subsec_nanos()) / 1e6)
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_timestamps_as_date_parse_does() {
        // Node 22, TZ=UTC; local-time forms left out.
        let cases: &[(&str, Option<f64>)] = &[
            ("2025-06-01T12:34:56.789Z", Some(1748781296789.0)),
            ("2025-06-01T12:34:56Z", Some(1748781296000.0)),
            ("2025-06-01T12:34Z", Some(1748781240000.0)),
            ("2025-06-01", Some(1748736000000.0)),
            ("2025-06", Some(1748736000000.0)),
            ("2025", Some(1735689600000.0)),
            ("2025-06-01T12:34:56.7Z", Some(1748781296700.0)),
            ("2025-06-01T12:34:56.78Z", Some(1748781296780.0)),
            ("2025-06-01T12:34:56.789123Z", Some(1748781296789.0)),
            ("2025-06-01T12:34:56.7899Z", Some(1748781296789.0)),
            ("2025-06-01T12:34:56.1234567890123Z", Some(1748781296123.0)),
            ("2025-06-01T12:34:56.Z", None),
            ("2025-06-01T12:34:56+04:00", Some(1748766896000.0)),
            ("2025-06-01T12:34:56-0130", Some(1748786696000.0)),
            ("2025-06-01T12:34:56.789+0130", Some(1748775896789.0)),
            ("2025-06-01T12:34:56.789-01:30", Some(1748786696789.0)),
            ("2025-06-01T12:34:56+04", None),
            ("2025-06-01T12:34:56+24:00", None),
            ("2025-06-01T12:34:56+23:59", Some(1748694956000.0)),
            ("2025-06-01T12:34:56+12:60", None),
            ("2025-06-01T24:00:00Z", Some(1748822400000.0)),
            ("2025-06-01T24:00:01Z", None),
            ("2025-02-30T00:00:00Z", Some(1740873600000.0)),
            ("2025-02-31T00:00:00Z", Some(1740960000000.0)),
            ("2025-13-01T00:00:00Z", None),
            ("2025-06-32T00:00:00Z", None),
            ("2025-00-01", None),
            ("2025-06-00", None),
            ("2025-06-01T25:00:00Z", None),
            ("2025-06-01T12:60:00Z", None),
            ("2025-06-01T12:34:60Z", None),
            ("+002025-06-01T00:00:00Z", Some(1748736000000.0)),
            ("-000001-01-01T00:00:00Z", Some(-62198755200000.0)),
            ("-000000-01-01T00:00:00Z", None),
            ("+275760-09-13T00:00:00.000Z", Some(8640000000000000.0)),
            ("+275760-09-13T00:00:00.001Z", None),
            ("275760-09-13T00:00:00Z", None),
            ("0000-01-01T00:00:00Z", Some(-62167219200000.0)),
            ("2025-06-01t12:34:56z", Some(1748781296000.0)),
            ("2025-06-01T12:34:56z", Some(1748781296000.0)),
            ("2025-06-01 12:34:56Z", Some(1748781296000.0)),
            ("2025-06-01 12:34:56.789Z", Some(1748781296789.0)),
            ("2025-06-01 12:34Z", Some(1748781240000.0)),
            ("2025-06-01Z", Some(1748736000000.0)),
            ("2025-06-01T12:34:56.789 Z", None),
            (" 2025-06-01T00:00:00Z", None),
            ("2025-06-01T00:00:00Z ", None),
            ("", None),
            ("abc", None),
            ("1970-01-01T00:00:00.000Z", Some(0.0)),
            ("1969-12-31T23:59:59.999Z", Some(-1.0)),
            ("2025-06-01T12Z", None),
            ("2025-06-01T1:02:03Z", None),
            ("20250601T000000Z", None),
        ];
        for &(text, expected) in cases {
            assert_eq!(parse_date(text), expected, "{text:?}");
        }
    }

    #[test]
    fn truncates_like_discovery() {
        assert_eq!(truncate("  a \n b  ", 10), "a b");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab   cdef", 4), "ab…");
        // Lengths are UTF-16 units: the emoji is two.
        assert_eq!(truncate("a🎉bc", 4), "a🎉…");
        // JavaScript cuts the emoji in half; the half left reads as U+FFFD.
        assert_eq!(truncate("a🎉bc", 3), "a\u{FFFD}…");
        assert_eq!(truncate("ab🎉c", 3), "ab…");
    }

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
    fn names_tools_and_what_they_acted_on() {
        let call = |block: Value| describe_tool_use(&block);
        assert_eq!(
            call(json!({"name": "Bash", "input": {"command": "ls", "description": "List"}})),
            ToolCall {
                name: "Bash".into(),
                detail: Some("ls".into())
            }
        );
        assert_eq!(
            call(json!({"name": "Read", "input": {"file_path": "/a/b/c.ts/"}}))
                .detail
                .as_deref(),
            Some("c.ts")
        );
        // A file whose name is empty stops the search.
        assert_eq!(
            call(json!({"name": "Read", "input": {"file_path": "/", "pattern": "x"}})).detail,
            None
        );
        assert_eq!(
            call(json!({"name": "Grep", "input": {"command": " ", "pattern": "TODO"}}))
                .detail
                .as_deref(),
            Some("TODO")
        );
        assert_eq!(
            call(json!({"name": "mcp__nodus__create_note", "input": "x"})).name,
            "create_note"
        );
        assert_eq!(call(json!({"name": "mcp__nodus__", "input": {}})).name, "mcp__nodus__");
        assert_eq!(call(json!({"name": "mcp__a___b"})).name, "b");
        assert_eq!(call(json!({"name": "Task", "input": ["prompt"]})).detail, None);
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
