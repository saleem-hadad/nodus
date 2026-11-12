//! The board:* handlers that keep what the window changes, and the session:*
//! ones that read transcripts (ipc.ts).

use indexmap::IndexMap;
use nodus_domain::js::{self, slice_utf16};
use nodus_domain::{
    BoardLink, BoardNote, BoardSection, BoardShape, Event, FolderColor, InkStroke, NodePatch, Rect, SessionKind,
    SessionSearchHit, SummarizePhase, SummarizeProgress, SummaryResult, TranscriptMessage, Viewport,
};
use nodus_platform::Env;
use nodus_platform::runner::Runner;
use nodus_transcripts::{ClaudeCall, NamingSource, SearchTarget, SummarizeTarget, is_agent_id, read_transcript};
use serde_json::{Map, Value};

use crate::archive::{drop_node, is_empty, unstick_notes};
use crate::board::Board;
use crate::cards::set;
use crate::{BoardError, Effects};

/// The most a note holds, in UTF-16 units (notes.ts).
pub const NOTE_MAX_CHARS: usize = 20_000;

/// Bounds for an expanded card's size, in board units (SESS-097).
pub fn clamp_size(n: f64) -> f64 {
    js::round(n.clamp(200.0, 4000.0))
}

/// Bounds for a note's size, in board units (notes.ts).
pub fn clamp_note_size(n: f64) -> f64 {
    js::round(n.clamp(120.0, 1200.0))
}

/// `Number.isFinite(v)`: a number, and not an infinity.
fn finite(v: Option<&Value>) -> Option<f64> {
    v?.as_f64().filter(|n| n.is_finite())
}

/// `typeof v === 'string'`.
fn string(v: Option<&Value>) -> Option<&str> {
    v?.as_str()
}

/// One of the folder colours, by name.
fn folder_color(v: Option<&Value>) -> Option<FolderColor> {
    string(v).and_then(FolderColor::parse)
}

impl Board {
    /// `board:save-nodes`: moves, renames, archives, groups and resizes cards
    /// (SESS-090…099).
    pub fn save_nodes(&self, project_id: &str, patches: &[NodePatch]) -> Result<(), BoardError> {
        let project = self.project(project_id)?;
        let now = self.now();
        let mut effects = Effects::default();
        self.store.update(|state| {
            let board = state.board_mut(project_id);
            let mut dropped = false;
            for patch in patches {
                // SESS-090
                let Some(node) = board.nodes.get(&patch.session_id) else {
                    continue;
                };
                // SESS-091: a session nothing was said in is removed rather than
                // archived; its note falls off where the card was before it moved.
                if patch.archived == Some(true) && !node.archived {
                    let meta = self
                        .transcripts
                        .session(&project.repo_path, &node.session_id)
                        .map(|f| f.meta);
                    if is_empty(node, meta.as_ref()) {
                        drop_node(board, &patch.session_id, &mut effects);
                        dropped = true;
                        continue;
                    }
                }
                let sections = &board.sections;
                let node = board.nodes.get_mut(&patch.session_id).expect("looked up above");
                // SESS-092
                if let Some(x) = patch.x.filter(|x| x.is_finite()) {
                    node.x = x;
                }
                if let Some(y) = patch.y.filter(|y| y.is_finite()) {
                    node.y = y;
                }
                // SESS-093
                if let Some(title) = &patch.title {
                    let title = js::trim(title);
                    node.title = (!title.is_empty()).then(|| title.to_owned());
                }
                // SESS-094: terminals have no transcript to keep, so they are removed
                // instead of archived; a view's brain goes with its view.
                if let Some(archived) = patch.archived
                    && node.kind != Some(SessionKind::Terminal)
                    && !set(node.view_of.as_deref())
                {
                    if archived && !node.archived {
                        effects.stop.push(node.session_id.clone());
                        node.archived_at = Some(now);
                    }
                    node.archived = archived;
                }
                // SESS-095
                match &patch.section_id {
                    Some(None) => node.section_id = None,
                    Some(Some(section)) if sections.contains_key(section) => node.section_id = Some(section.clone()),
                    _ => {}
                }
                // SESS-096
                match patch.expanded {
                    Some(true) => node.expanded = Some(true),
                    Some(false) => node.expanded = None,
                    None => {}
                }
                // SESS-097
                if let Some(w) = patch.w.filter(|w| w.is_finite()) {
                    node.w = Some(clamp_size(w));
                }
                if let Some(h) = patch.h.filter(|h| h.is_finite()) {
                    node.h = Some(clamp_size(h));
                }
                // SESS-098: an archived card leaves its section, and folds back to a card.
                if node.archived {
                    node.section_id = None;
                    node.expanded = None;
                }
            }
            // SESS-099: the window drops empty cards itself, by what it last read
            // of their transcripts; one it archived instead goes when it reloads.
            if dropped {
                effects.events.push(Event::BoardChanged {
                    project_id: project_id.to_owned(),
                });
            }
        });
        self.apply(effects);
        Ok(())
    }

    /// `board:save-section`: ignores input without a string id or a finite
    /// position (SESS-100, SESS-204).
    pub fn save_section(&self, project_id: &str, section: &Value) {
        let (Some(id), Some(x), Some(y)) = (
            string(section.get("id")),
            finite(section.get("x")),
            finite(section.get("y")),
        ) else {
            return;
        };
        let now = self.now();
        self.store.update(|state| {
            let board = state.board_mut(project_id);
            let created_at = board.sections.get(id).map_or(now, |s| s.created_at);
            board.sections.insert(
                id.to_owned(),
                BoardSection {
                    id: id.to_owned(),
                    name: string(section.get("name")).map(js::trim).unwrap_or_default().to_owned(),
                    color: folder_color(section.get("color")).unwrap_or(FolderColor::Blue),
                    x,
                    y,
                    created_at,
                    extra: Map::new(),
                },
            );
        });
    }

    /// `board:remove-section`: its cards stay where they are (SESS-101).
    pub fn remove_section(&self, project_id: &str, section_id: &str) {
        self.store.update(|state| {
            let board = state.board_mut(project_id);
            board.sections.shift_remove(section_id);
            for node in board.nodes.values_mut() {
                if node.section_id.as_deref() == Some(section_id) {
                    node.section_id = None;
                }
            }
        });
    }

    /// `board:name-section`, once the login-shell environment is ready: a name
    /// from Claude for the sessions grouped in a section (SESS-102), asked
    /// through `runner` (SRV-005).
    pub async fn name_section(
        &self,
        project_id: &str,
        session_ids: &[String],
        env: &Env,
        runner: &dyn Runner,
    ) -> Result<String, BoardError> {
        let project = self.project(project_id)?;
        let nodes = self.read_board(project_id, |_, board| {
            session_ids
                .iter()
                .filter_map(|id| board.nodes.get(id).cloned())
                .collect::<Vec<_>>()
        });
        let mut sources = Vec::new();
        for node in nodes {
            // A shell says nothing about what the group is for.
            if node.kind == Some(SessionKind::Terminal) {
                continue;
            }
            let meta = self
                .transcripts
                .session(&project.repo_path, &node.session_id)
                .filter(|f| f.has_content)
                .map(|f| f.meta);
            let title = [node.title.clone(), meta.as_ref().map(|m| m.title.clone())]
                .into_iter()
                .flatten()
                .find(|t| !t.is_empty());
            if let Some(title) = title {
                sources.push(NamingSource {
                    title,
                    first_prompt: meta.and_then(|m| m.first_prompt),
                });
            }
        }
        if sources.is_empty() {
            return Err(BoardError::NothingToName);
        }
        let claude = runner.claude(env)?;
        let call = ClaudeCall {
            runner,
            program: &claude,
            env: &env.spawn_env(),
        };
        Ok(nodus_transcripts::name_section(call, &sources).await?)
    }

    /// `board:save-note`: the note as the window has it, checked (SESS-103).
    pub fn save_note(&self, project_id: &str, note: &Value) {
        let (Some(id), Some(x), Some(y)) = (string(note.get("id")), finite(note.get("x")), finite(note.get("y")))
        else {
            return;
        };
        let now = self.now();
        self.store.update(|state| {
            let board = state.board_mut(project_id);
            let stored = board.notes.get(id);
            let session_id = string(note.get("sessionId")).filter(|s| board.nodes.contains_key(*s));
            let saved = BoardNote {
                id: id.to_owned(),
                text: string(note.get("text")).map_or_else(String::new, |t| slice_utf16(t, 0, NOTE_MAX_CHARS)),
                color: folder_color(note.get("color")).unwrap_or(FolderColor::Yellow),
                x: js::round(x),
                y: js::round(y),
                w: finite(note.get("w")).map(clamp_note_size),
                h: finite(note.get("h")).map(clamp_note_size),
                // Only a card on this board can wear it.
                session_id: session_id.map(str::to_owned),
                // Only the board tools say which session wrote a note.
                from: stored.and_then(|n| n.from.clone()),
                from_project: None,
                from_title: None,
                created_at: stored.map_or(now, |n| n.created_at),
                extra: Map::new(),
            };
            board.notes.insert(id.to_owned(), saved);
        });
    }

    /// `board:remove-notes` (SESS-104).
    pub fn remove_notes(&self, project_id: &str, note_ids: &[String]) {
        self.store.update(|state| {
            let board = state.board_mut(project_id);
            for id in note_ids {
                board.notes.shift_remove(id);
            }
            crate::shapes::prune_links(board);
        });
    }

    /// `board:save-shapes` (added after 0.9.4): new or changed shapes, each
    /// checked (`shapes::clean_shape`).
    pub fn save_shapes(&self, project_id: &str, shapes: &[BoardShape]) {
        let now = self.now();
        self.store
            .update(|state| crate::shapes::save_shapes(state.board_mut(project_id), shapes, now));
    }

    /// `board:remove-shapes`: the shapes go, with the links they held.
    pub fn remove_shapes(&self, project_id: &str, ids: &[String]) {
        self.store
            .update(|state| crate::shapes::remove_shapes(state.board_mut(project_id), ids));
    }

    /// `board:save-links`: new or changed links, each checked; one whose
    /// ends aren't on the board is left out.
    pub fn save_links(&self, project_id: &str, links: &[BoardLink]) {
        let now = self.now();
        self.store
            .update(|state| crate::shapes::save_links(state.board_mut(project_id), links, now));
    }

    /// `board:remove-links`.
    pub fn remove_links(&self, project_id: &str, ids: &[String]) {
        self.store
            .update(|state| crate::shapes::remove_links(state.board_mut(project_id), ids));
    }

    /// `board:save-ink`, with strokes already cleaned (`cleanInk`, 04) (SESS-105).
    pub fn save_ink(&self, project_id: &str, strokes: Vec<InkStroke>) {
        self.store
            .update(|state| state.board_mut(project_id).ink = Some(strokes));
    }

    /// `board:save-archive`: kept as given (SESS-106).
    pub fn save_archive(&self, project_id: &str, rect: Rect) {
        self.store.update(|state| state.board_mut(project_id).archive = rect);
    }

    /// `board:save-viewport`: kept as given (SESS-106).
    pub fn save_viewport(&self, project_id: &str, viewport: Viewport) {
        self.store
            .update(|state| state.board_mut(project_id).viewport = Some(viewport));
    }

    /// `board:remove-node`: the card goes, its session is hidden for good
    /// unless it's a terminal, and this waits for its process to stop
    /// (SESS-107, SESS-108). The transcript is never touched.
    pub async fn remove_node(&self, project_id: &str, session_id: &str) {
        self.store.update(|state| {
            let board = state.board_mut(project_id);
            let terminal = board
                .nodes
                .get(session_id)
                .is_some_and(|n| n.kind == Some(SessionKind::Terminal));
            if let Some(node) = board.nodes.get(session_id).cloned() {
                unstick_notes(board, &node);
            }
            board.nodes.shift_remove(session_id);
            crate::shapes::prune_links(board);
            // Terminals are never discovered on disk, so there is nothing to hide.
            if !terminal && !board.hidden.iter().any(|h| h == session_id) {
                board.hidden.push(session_id.to_owned());
            }
        });
        self.sessions.kill(session_id).await;
        self.sessions.forget_status(session_id);
    }

    /// A session's title, as its card would show it: a view's brain goes by
    /// its view's (SESS-158).
    pub fn session_title(&self, project_id: &str, session_id: &str) -> Result<String, BoardError> {
        let (node, view_title) = self.read_board(project_id, |_, board| {
            let node = board.nodes.get(session_id).cloned();
            let view_title = node
                .as_ref()
                .and_then(|n| n.view_of.as_deref())
                .filter(|v| !v.is_empty())
                .map(|v| board.views.get(v).map(|view| view.title.clone()).unwrap_or_default());
            (node, view_title)
        });
        if let Some(title) = view_title {
            return Ok(if title.is_empty() { "A view".into() } else { title });
        }
        let project = self.project(project_id)?;
        let meta = self.transcripts.session(&project.repo_path, session_id).map(|f| f.meta);
        let title = [node.as_ref().and_then(|n| n.title.clone()), meta.map(|m| m.title)]
            .into_iter()
            .flatten()
            .find(|t| !t.is_empty());
        Ok(title.unwrap_or_else(|| {
            if node.is_some_and(|n| n.kind == Some(SessionKind::Terminal)) {
                "Terminal"
            } else {
                "New session"
            }
            .into()
        }))
    }

    /// `session:search`: the board's sessions, live and archived, that hold
    /// every word of the query (SESS-175).
    pub async fn search(&self, project_id: &str, query: &str) -> Result<Vec<SessionSearchHit>, BoardError> {
        let project = self.project(project_id)?;
        let nodes = self.read_board(project_id, |_, board| board.nodes.values().cloned().collect::<Vec<_>>());
        let targets: Vec<SearchTarget> = nodes
            .iter()
            .filter(|n| {
                n.kind != Some(SessionKind::Terminal) && !set(n.run_of.as_deref()) && !set(n.view_of.as_deref())
            })
            .map(|n| SearchTarget {
                session_id: n.session_id.clone(),
                title: n
                    .title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .or_else(|| {
                        self.transcripts
                            .session(&project.repo_path, &n.session_id)
                            .map(|f| f.meta.title)
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Ok(self.transcripts.search(&project.repo_path, &targets, query).await)
    }

    /// `session:summarize`, once the login-shell environment is ready:
    /// handoff summaries of sessions, one per id asked for, in that order
    /// (SESS-165…174), each asked through `runner` (SRV-005).
    pub async fn summarize(
        &self,
        project_id: &str,
        session_ids: &[String],
        request_id: &str,
        focus: Option<&str>,
        env: &Env,
        runner: &dyn Runner,
    ) -> Result<Vec<SummaryResult>, BoardError> {
        let project = self.project(project_id)?;
        let titles: Vec<Option<String>> = self.read_board(project_id, |_, board| {
            session_ids
                .iter()
                .map(|id| board.nodes.get(id).and_then(|n| n.title.clone()))
                .collect()
        });
        let progress = |p: SummarizeProgress| self.events.emit(Event::SummarizeProgress(p));
        let mut results: IndexMap<String, SummaryResult> = IndexMap::new();
        let mut with_transcript = Vec::new();
        for (session_id, node_title) in session_ids.iter().zip(titles) {
            let file = self.transcripts.transcript_path(&project.repo_path, session_id);
            let meta = self.transcripts.read_session(session_id, &file).map(|f| f.meta);
            // SESS-166
            let title = [node_title, meta.as_ref().map(|m| m.title.clone())]
                .into_iter()
                .flatten()
                .find(|t| !t.is_empty())
                .unwrap_or_else(|| "New session".into());
            // SESS-165
            if meta.is_none() || !file.exists() {
                let error = "This session has no conversation yet".to_owned();
                results.insert(
                    session_id.clone(),
                    SummaryResult {
                        session_id: session_id.clone(),
                        title,
                        summary: String::new(),
                        error: Some(error.clone()),
                    },
                );
                progress(SummarizeProgress {
                    request_id: request_id.to_owned(),
                    session_id: session_id.clone(),
                    phase: SummarizePhase::Error,
                    error: Some(error),
                });
            } else {
                with_transcript.push(SummarizeTarget {
                    session_id: session_id.clone(),
                    title,
                });
            }
        }
        // SESS-174: after the sessions without a conversation were reported.
        let claude = runner.claude(env)?;
        let call = ClaudeCall {
            runner,
            program: &claude,
            env: &env.spawn_env(),
        };
        let summaries = nodus_transcripts::summarize_sessions(
            call,
            &project.repo_path,
            &with_transcript,
            request_id,
            &progress,
            focus,
        )
        .await;
        for r in summaries {
            results.insert(r.session_id.clone(), r);
        }
        // SESS-173: a session asked for twice gets the later result both times.
        Ok(session_ids.iter().map(|id| results[id].clone()).collect())
    }

    /// `session:transcript`: the conversation, for the preview (SESS-162).
    pub fn transcript(&self, project_id: &str, session_id: &str) -> Result<Vec<TranscriptMessage>, BoardError> {
        let project = self.project(project_id)?;
        Ok(self.transcripts.transcript(&project.repo_path, session_id)?)
    }

    /// `session:agent-transcript`: a subagent's steps, for the agent viewer.
    /// Both ids end up in a path, so only safe ones are read (SESS-163).
    pub fn agent_transcript(
        &self,
        project_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<TranscriptMessage>, BoardError> {
        if !is_agent_id(agent_id) || !is_agent_id(session_id) {
            return Err(BoardError::NotAnAgent);
        }
        let project = self.project(project_id)?;
        let dir = self.transcripts.subagents_dir(&project.repo_path, session_id);
        Ok(read_transcript(&dir.join(format!("agent-{agent_id}.jsonl")), true)?)
    }
}
