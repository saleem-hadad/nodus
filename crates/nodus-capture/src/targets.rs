//! What the panel can send to: the projects, their sessions, the recent
//! captures and where a capture goes before Claude has guessed; and
//! remembering what was made. The "Projects, sessions and recents" part of
//! `src/main/capture.ts`.

use std::collections::HashSet;

use nodus_domain::{
    CaptureKind, CaptureProject, CaptureReceipt, CaptureRecent, CaptureSession, CaptureTargets, SessionKind,
    SessionMeta, SessionStatus,
};
use nodus_store::{CaptureRecord, State};

/// How many captures are remembered for the recents (CAP-155).
pub const MAX_RECORDS: usize = 20;

/// What capture needs to know about a session that isn't in state.json: its
/// process, its status and its transcript. nodus-core implements it with the
/// session host, the status machine and nodus-transcripts.
///
/// Capture may call these while it reads the state, so they mustn't use the
/// `Store`.
pub trait CaptureSessions: Send + Sync {
    /// `pty.isLive`: whether a process is running for the session.
    fn is_live(&self, session_id: &str) -> bool;
    /// `hooks.getStatus`.
    fn status(&self, session_id: &str) -> SessionStatus;
    /// `readSession(sessionId, transcriptPath(repoPath, sessionId))?.meta`:
    /// what its transcript says, if it has one.
    fn meta(&self, repo_path: &str, session_id: &str) -> Option<SessionMeta>;
}

/// A string JavaScript reads as true.
fn set(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// The session check (`sessionInfo`, CAP-148): a Claude session on the
/// project's board, not archived, not a terminal, an action run or a view's
/// brain; with its title, status and when it was last active.
pub fn session_info(
    state: &State,
    project_id: &str,
    session_id: &str,
    sessions: &dyn CaptureSessions,
) -> Option<CaptureSession> {
    let project = state.project(project_id).ok()?;
    let node = state.board(project_id)?.nodes.get(session_id)?;
    if node.archived
        || node.kind == Some(SessionKind::Terminal)
        || set(node.run_of.as_deref())
        || set(node.view_of.as_deref())
    {
        return None;
    }
    let meta = sessions.meta(&project.repo_path, session_id);
    // Until Claude names a session started here, it goes by what was captured.
    let captured = state
        .captures
        .iter()
        .flatten()
        .find(|c| c.kind == CaptureKind::Session && c.session_id.as_deref() == Some(session_id))
        .map(|c| c.title.as_str());
    let title = [node.title.as_deref(), meta.as_ref().map(|m| m.title.as_str()), captured]
        .into_iter()
        .flatten()
        .find(|t| !t.is_empty())
        .unwrap_or("Untitled session");
    Some(CaptureSession {
        project_id: project_id.to_owned(),
        session_id: session_id.to_owned(),
        title: title.to_owned(),
        status: sessions.status(session_id),
        live: sessions.is_live(session_id),
        updated_at: meta.map_or(node.created_at, |m| m.updated_at),
    })
}

/// A capture record as a recent, while what it made is still there
/// (CAP-150): a session that passes the session check, with its title and
/// status now; a note or a view still on the board.
pub fn recent_of(state: &State, record: &CaptureRecord, sessions: &dyn CaptureSessions) -> Option<CaptureRecent> {
    let board = state.board(&record.project_id)?;
    // Archived projects too.
    state.project(&record.project_id).ok()?;
    let base = CaptureRecent {
        id: record.id.clone(),
        kind: record.kind,
        project_id: record.project_id.clone(),
        title: record.title.clone(),
        created_at: record.created_at,
        session_id: None,
        note_id: None,
        view_id: None,
        status: None,
    };
    match record.kind {
        CaptureKind::Session | CaptureKind::Into => {
            let session_id = record.session_id.as_deref().filter(|id| !id.is_empty())?;
            let session = session_info(state, &record.project_id, session_id, sessions)?;
            // A session goes by Claude's title for it once it has one.
            Some(CaptureRecent {
                title: session.title,
                session_id: Some(session.session_id),
                status: Some(session.status),
                ..base
            })
        }
        CaptureKind::Note => {
            let note = board
                .notes
                .get(record.note_id.as_deref().filter(|id| !id.is_empty())?)?;
            Some(CaptureRecent {
                note_id: Some(note.id.clone()),
                ..base
            })
        }
        CaptureKind::View => {
            let view = board
                .views
                .get(record.view_id.as_deref().filter(|id| !id.is_empty())?)?;
            let title = if view.title == "New view" {
                record.title.clone()
            } else {
                view.title.clone()
            };
            Some(CaptureRecent {
                title,
                view_id: Some(view.id.clone()),
                ..base
            })
        }
    }
}

/// Where a capture goes before Claude has guessed (CAP-151): the project
/// captured into last, unless a board was opened since; else the board
/// opened last in the window this run; else the project opened most recently.
pub fn default_project(state: &State, ids: &HashSet<&str>, last_board: Option<&str>) -> Option<String> {
    let last_capture = state
        .captures
        .iter()
        .flatten()
        .find(|c| ids.contains(c.project_id.as_str()));
    // A stable sort, newest first, with no openedAt as 0.
    let mut projects: Vec<_> = state.projects.iter().filter(|p| ids.contains(p.id.as_str())).collect();
    projects.sort_by(|a, b| {
        let at = |p: &&nodus_domain::Project| p.opened_at.unwrap_or(0.0);
        at(b).partial_cmp(&at(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let opened = projects.first();
    let opened_at = opened.and_then(|p| p.opened_at).filter(|at| *at != 0.0 && !at.is_nan());
    if let Some(capture) = last_capture
        && opened_at.is_none_or(|at| capture.created_at > at)
    {
        return Some(capture.project_id.clone());
    }
    if let Some(board) = last_board.filter(|b| ids.contains(b)) {
        return Some(board.to_owned());
    }
    opened.map(|p| p.id.clone())
}

/// Projects without a board yet, which `targets` reads as empty and the
/// TypeScript's `getBoard` makes in memory, to be saved with the next save.
pub fn missing_boards(state: &State) -> Vec<String> {
    state
        .projects
        .iter()
        .filter(|p| p.archived != Some(true) && state.board(&p.id).is_none())
        .map(|p| p.id.clone())
        .collect()
}

/// `capture:targets` (CAP-149): the projects that aren't archived, in
/// state.json's order; their sessions, most recently active first; the
/// recents that still resolve; and the default project.
pub fn targets(state: &State, last_board: Option<&str>, sessions: &dyn CaptureSessions) -> CaptureTargets {
    let projects: Vec<_> = state.projects.iter().filter(|p| p.archived != Some(true)).collect();
    let mut found = Vec::new();
    for project in &projects {
        let Some(board) = state.board(&project.id) else {
            continue;
        };
        for node in board.nodes.values() {
            if let Some(info) = session_info(state, &project.id, &node.session_id, sessions) {
                found.push(info);
            }
        }
    }
    // Stable, newest first.
    found.sort_by(|a, b| {
        b.updated_at
            .partial_cmp(&a.updated_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let recents = state
        .captures
        .iter()
        .flatten()
        .filter_map(|record| recent_of(state, record, sessions))
        .collect();
    let ids: HashSet<&str> = projects.iter().map(|p| p.id.as_str()).collect();
    CaptureTargets {
        projects: projects
            .iter()
            .map(|p| CaptureProject {
                id: p.id.clone(),
                name: p.name.clone(),
                color: p.color,
            })
            .collect(),
        sessions: found,
        recents,
        default_project_id: default_project(state, &ids, last_board),
    }
}

/// Remembers a capture for the recents, newest first, keeping 20 (CAP-155).
/// The caller saves the state.
pub fn remember(state: &mut State, receipt: &CaptureReceipt, id: String, now: f64) {
    let record = CaptureRecord {
        id,
        kind: receipt.kind,
        project_id: receipt.project_id.clone(),
        title: receipt.title.clone(),
        created_at: now,
        session_id: receipt.session_id.clone(),
        note_id: receipt.note_id.clone(),
        view_id: receipt.view_id.clone(),
        extra: Default::default(),
    };
    let records = state.captures.get_or_insert_with(Vec::new);
    records.insert(0, record);
    records.truncate(MAX_RECORDS);
}

#[cfg(test)]
pub(crate) mod tests;
