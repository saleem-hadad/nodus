//! The board service: loading a board (the merge with what's on disk), the
//! cards, session lists and project stats (board.ts and `board:load`).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use nodus_domain::{BoardSnapshot, Event, EventSink, Project, ProjectStats, RunningCounts, SessionCard, SessionStatus};
use nodus_store::{BoardState, State, Store};
use nodus_transcripts::{DiscoveredSession, Transcripts, Watcher};

use crate::archive::{grow_archive, import_sessions};
use crate::cards::{SessionListItem, session_list_items, set, to_card};
use crate::{BoardError, BoardSessions, Effects};

/// `Date.now()`, in ms since the epoch.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// The system clock, in whole milliseconds as `Date.now()` gives them.
pub fn real_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_millis() as f64)
    })
}

/// Every project's board, as the window, the board tools and capture change it.
pub struct Board {
    pub(crate) store: Arc<Store>,
    pub(crate) transcripts: Arc<Transcripts>,
    pub(crate) sessions: Arc<dyn BoardSessions>,
    pub(crate) events: Arc<dyn EventSink>,
    pub(crate) watcher: Watcher,
    /// Sessions that asked to be archived (archive_self), archived when their
    /// turn ends. In memory only (SESS-157).
    pub(crate) archive_when_done: Mutex<HashSet<String>>,
    pub(crate) clock: Clock,
}

impl Board {
    /// The boards in `store`, with transcripts read through `transcripts`, the
    /// processes reached through `sessions`, and events sent to `events`.
    /// Transcript folders are watched on `runtime`.
    pub fn new(
        store: Arc<Store>,
        transcripts: Arc<Transcripts>,
        sessions: Arc<dyn BoardSessions>,
        events: Arc<dyn EventSink>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let watcher = transcripts.watcher(runtime);
        Self {
            store,
            transcripts,
            sessions,
            events,
            watcher,
            archive_when_done: Mutex::new(HashSet::new()),
            clock: real_clock(),
        }
    }

    /// The same, telling the time by `clock`.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The transcript watcher, for stopping a project's watch when it's
    /// removed and every watch when the app quits (SESS-070).
    pub fn watcher(&self) -> &Watcher {
        &self.watcher
    }

    pub fn transcripts(&self) -> &Arc<Transcripts> {
        &self.transcripts
    }

    pub(crate) fn now(&self) -> f64 {
        (self.clock)()
    }

    /// `getProject(id)`, as it is now.
    pub(crate) fn project(&self, project_id: &str) -> Result<Project, BoardError> {
        Ok(self.store.read(|state| state.project(project_id).cloned())?)
    }

    /// Reads a project's board, made empty first if it has none (`getBoard`).
    pub(crate) fn read_board<R>(&self, project_id: &str, read: impl FnOnce(&State, &BoardState) -> R) -> R {
        let mut read = Some(read);
        let found = self.store.read(|state| {
            let board = state.board(project_id)?;
            Some((read.take().expect("read once"))(state, board))
        });
        match found {
            Some(result) => result,
            None => self.store.update(|state| {
                state.board_mut(project_id);
                let board = state.board(project_id).expect("just made");
                (read.take().expect("read once"))(state, board)
            }),
        }
    }

    /// Carries out what a change asked of the processes and the window.
    pub(crate) fn apply(&self, effects: Effects) {
        for id in &effects.stop {
            self.sessions.stop(id);
        }
        for id in &effects.stop_and_forget {
            self.sessions.stop_and_forget(id);
        }
        for event in effects.events {
            self.events.emit(event);
        }
    }

    /// `board:load` after capture is told the board opened (SESS-071): the
    /// project (`Unknown project <id>` if it's gone), runs that ended while the
    /// app was closed finished (SESS-156), the board merged with what's on
    /// disk, and its transcript folder watched.
    pub fn load(&self, project_id: &str) -> Result<BoardSnapshot, BoardError> {
        let project = self.project(project_id)?;
        let stale: Vec<String> = self.read_board(project_id, |_, board| {
            board
                .nodes
                .values()
                .filter(|n| set(n.run_of.as_deref()) && !self.sessions.is_live(&n.session_id))
                .map(|n| n.session_id.clone())
                .collect()
        });
        for id in stale {
            self.finish_run(&id);
        }
        let snapshot = self.load_board(project_id)?;
        let events = Arc::clone(&self.events);
        let id = project_id.to_owned();
        self.watcher.watch_project(
            project_id,
            &project.repo_path,
            Arc::new(move || {
                events.emit(Event::BoardChanged { project_id: id.clone() });
            }),
        );
        Ok(snapshot)
    }

    /// `loadBoard`: the board merged with the sessions on disk, saved, as cards
    /// (SESS-072…081).
    pub fn load_board(&self, project_id: &str) -> Result<BoardSnapshot, BoardError> {
        let repo_path = self.project(project_id)?.repo_path;
        let disk = self.transcripts.scan_repo(&repo_path);
        let folder_exists = self.transcripts.transcript_dir(&repo_path).exists();
        let now = self.now();
        let snapshot = self.store.update(|state| {
            let State { projects, boards, .. } = state;
            let project = projects
                .iter_mut()
                .find(|p| p.id == project_id)
                .ok_or_else(|| nodus_store::NotFound::Project(project_id.into()))?;
            let board = boards.entry(project_id.to_owned()).or_default();
            merge(board, &disk, folder_exists, self.sessions.as_ref());
            // Links whose ends went while the board wasn't looking (a view
            // deleted, a card dropped) go too.
            crate::shapes::prune_links(board);
            // SESS-078
            project.opened_at = Some(now);
            let snapshot = BoardSnapshot {
                project: project.clone(),
                cards: Vec::new(),
                archive: board.archive,
                sections: sorted(board.sections.values(), |s| s.created_at),
                notes: sorted(board.notes.values(), |n| n.created_at),
                views: sorted(board.views.values(), |v| v.created_at),
                ink: board.ink.clone().unwrap_or_default(),
                shapes: board.shapes.values().cloned().collect(),
                links: board.links.values().cloned().collect(),
                viewport: board.viewport,
            };
            Ok::<_, BoardError>((snapshot, board.nodes.values().cloned().collect::<Vec<_>>()))
        });
        let (snapshot, nodes) = snapshot?;
        // SESS-079: every node, in the board's order.
        let cards = nodes
            .iter()
            .map(|node| {
                let found = disk.get(&node.session_id);
                let mut card = to_card(node, found, self.sessions.as_ref());
                self.sessions
                    .attach_agents(&mut card, &repo_path, found.and_then(|f| f.last_prompt_at));
                card
            })
            .collect();
        Ok(BoardSnapshot { cards, ..snapshot })
    }

    /// The project's sessions as the board tools and views see them, leaving
    /// out `self_id` and the views' brains, which aren't on the board. Loads
    /// the board in full first (SESS-159).
    pub fn session_list(
        &self,
        project_id: &str,
        self_id: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SessionListItem>, BoardError> {
        let snapshot = self.load_board(project_id)?;
        Ok(session_list_items(
            &snapshot.cards,
            &snapshot.sections,
            self_id,
            include_archived,
        ))
    }

    /// What the projects grid shows for a project (SESS-160).
    pub fn project_stats(&self, project: &Project) -> ProjectStats {
        let disk = self.transcripts.scan_repo(&project.repo_path);
        self.read_board(&project.id, |_, board| stats(board, &disk, self.sessions.as_ref()))
    }

    /// Every project's stats, for `projects:stats`.
    pub fn all_project_stats(&self) -> IndexMap<String, ProjectStats> {
        let projects = self.store.read(|state| state.projects.clone());
        projects
            .iter()
            .map(|project| (project.id.clone(), self.project_stats(project)))
            .collect()
    }

    /// Whether the session has a transcript yet (SESS-161).
    pub fn has_transcript(&self, project: &Project, session_id: &str) -> bool {
        self.transcripts.has_transcript(&project.repo_path, session_id)
    }

    /// A card for a node, as `toCard` without what's on disk (a new card).
    pub fn card(&self, node: &nodus_domain::BoardNode) -> SessionCard {
        to_card(node, None, self.sessions.as_ref())
    }
}

/// The board merged with what's on disk (SESS-072…077).
fn merge(
    board: &mut BoardState,
    disk: &IndexMap<String, DiscoveredSession>,
    folder_exists: bool,
    sessions: &dyn BoardSessions,
) {
    let hidden: HashSet<&str> = board.hidden.iter().map(String::as_str).collect();
    // SESS-072: imported cards whose transcript has been deleted are forgotten,
    // only when the folder is there, so a missing folder never wipes the layout.
    if folder_exists {
        board
            .nodes
            .retain(|id, node| node.created_by_app == Some(true) || disk.contains_key(id) || sessions.is_live(id));
    }
    // SESS-073
    let mut fresh: Vec<&DiscoveredSession> = disk
        .values()
        .filter(|s| {
            s.eligible && !board.nodes.contains_key(&s.meta.session_id) && !hidden.contains(s.meta.session_id.as_str())
        })
        .collect();
    fresh.sort_by(|a, b| {
        b.meta
            .updated_at
            .partial_cmp(&a.meta.updated_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // SESS-074, SESS-075
    import_sessions(board, &fresh);
    grow_archive(board);
    // SESS-076: only active cards belong to a section, and only to one that still exists.
    let sections = &board.sections;
    for node in board.nodes.values_mut() {
        if let Some(section) = node.section_id.as_deref()
            && !section.is_empty()
            && (node.archived || !sections.contains_key(section))
        {
            node.section_id = None;
        }
    }
    // SESS-077: a note whose card left the board some other way stays where it was.
    let nodes = &board.nodes;
    for note in board.notes.values_mut() {
        if let Some(id) = note.session_id.as_deref()
            && !id.is_empty()
            && !nodes.contains_key(id)
        {
            note.session_id = None;
        }
    }
}

/// SESS-160
fn stats(board: &BoardState, disk: &IndexMap<String, DiscoveredSession>, sessions: &dyn BoardSessions) -> ProjectStats {
    let hidden: HashSet<&str> = board.hidden.iter().map(String::as_str).collect();
    // A view's brain isn't on the board.
    let nodes: Vec<_> = board.nodes.values().filter(|n| !set(n.view_of.as_deref())).collect();
    let undiscovered = disk
        .iter()
        .filter(|(id, found)| found.eligible && !board.nodes.contains_key(*id) && !hidden.contains(id.as_str()))
        .count();
    let (mut working, mut waiting) = (0, 0);
    for node in &nodes {
        if node.archived || !sessions.is_live(&node.session_id) {
            continue;
        }
        match sessions.status(&node.session_id) {
            SessionStatus::Working => working += 1,
            SessionStatus::Waiting => waiting += 1,
            SessionStatus::Done => {}
        }
    }
    // Brains included: every process the app runs for the project has a node.
    let mut running = RunningCounts::default();
    for node in board.nodes.values() {
        if !sessions.is_live(&node.session_id) {
            continue;
        }
        if set(node.view_of.as_deref()) || set(node.run_of.as_deref()) {
            running.background += 1;
        } else if node.kind == Some(nodus_domain::SessionKind::Terminal) {
            running.terminals += 1;
        } else {
            running.sessions += 1;
        }
    }
    ProjectStats {
        total: (nodes.len() + undiscovered) as u32,
        active: nodes.iter().filter(|n| !n.archived).count() as u32,
        working,
        waiting,
        running,
    }
}

/// Sorted by when each was made, oldest first, keeping their order otherwise.
fn sorted<'a, T: Clone + 'a>(items: impl Iterator<Item = &'a T>, created_at: impl Fn(&T) -> f64) -> Vec<T> {
    let mut items: Vec<T> = items.cloned().collect();
    items.sort_by(|a, b| {
        created_at(a)
            .partial_cmp(&created_at(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items
}
