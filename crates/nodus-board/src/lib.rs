//! A project's board: its stored cards merged with the sessions found on
//! disk, the archive they're laid out in, sections, sticky notes and where
//! they go, the geometry they share, and the ink drawn over them.
//!
//! Ports `src/main/board.ts`, `src/main/notes.ts`, `src/main/ink.ts`,
//! `src/shared/geometry.ts` and `src/shared/sessions.ts`, the stop rules in
//! `stopSessions.ts`, and the board:* and session:* handlers' board side in
//! `src/main/ipc.ts`.
//!
//! [`Board`] is the service: it reads and changes the boards in the
//! [`nodus_store::Store`], reads transcripts through
//! [`nodus_transcripts::Transcripts`], and reaches the session processes and
//! their status through [`BoardSessions`], which `nodus-core` implements.

pub mod archive;
mod board;
pub mod cards;
mod create;
pub mod geometry;
mod handlers;
pub mod ink;
pub mod notes;
mod runs;
pub mod stop;

use std::future::Future;
use std::pin::Pin;

use nodus_domain::{Event, SessionCard, SessionStatus, SessionUsage};

pub use archive::{archive_node, drop_node, is_empty_session, unstick_notes};
pub use board::{Board, Clock, real_clock};
pub use cards::{SessionListItem, to_card};
pub use create::{Launcher, NewSession, Resume, StartNew, new_session};
pub use handlers::{NOTE_MAX_CHARS, clamp_note_size, clamp_size};
pub use runs::find_action;
pub use stop::{StopConfirmation, stop_confirmation, stoppable};

/// Locks a mutex, carrying on after a panic in another thread that held it.
pub(crate) trait PoisonFree<T> {
    fn lock_free(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> PoisonFree<T> for std::sync::Mutex<T> {
    fn lock_free(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A boxed future, for trait methods.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the board needs from the session processes and the status machine
/// (02). `nodus-core` implements it with the session host and the hooks.
///
/// The board may call the query methods while it holds the state, so they
/// mustn't use the `Store`.
pub trait BoardSessions: Send + Sync {
    /// `pty.isLive`: whether a process is running for the session.
    fn is_live(&self, session_id: &str) -> bool;
    /// `hooks.getStatus`: `Done` for a process that isn't live.
    fn status(&self, session_id: &str) -> SessionStatus;
    /// `hooks.getUsage`.
    fn usage(&self, session_id: &str) -> Option<SessionUsage>;
    /// `agents.attachAgents`: adds `agent_count` and `agents` to a card.
    fn attach_agents(&self, card: &mut SessionCard, repo_path: &str, last_prompt_at: Option<f64>);
    /// `pty.kill` without waiting, its status kept (archiving).
    fn stop(&self, session_id: &str);
    /// `pty.kill`, then `hooks.forgetStatus` once the process is gone, without
    /// waiting (a card dropped, a run finished).
    fn stop_and_forget(&self, session_id: &str);
    /// `pty.kill`, waited for: done once the process exits, or after 4 s.
    fn kill(&self, session_id: &str) -> BoxFuture<'_, ()>;
    /// `hooks.forgetStatus`.
    fn forget_status(&self, session_id: &str);
}

/// What a change to a board asks of the session processes and the window,
/// carried out once the state is let go of.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Effects {
    /// Processes to stop, keeping their status (archived cards).
    pub stop: Vec<String>,
    /// Processes to stop and forget the status of (dropped cards, finished runs).
    pub stop_and_forget: Vec<String>,
    /// Events for the window.
    pub events: Vec<Event>,
}

impl Effects {
    pub fn is_empty(&self) -> bool {
        self.stop.is_empty() && self.stop_and_forget.is_empty() && self.events.is_empty()
    }
}

/// Why a board command failed; the text is what 0.9.4 shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoardError {
    /// `Unknown project <id>`.
    #[error(transparent)]
    NotFound(#[from] nodus_store::NotFound),
    #[error("These sessions have nothing to name the section after yet")]
    NothingToName,
    #[error(transparent)]
    NoClaude(#[from] nodus_platform::ClaudeNotFound),
    #[error(transparent)]
    Naming(#[from] nodus_transcripts::NamingError),
    #[error("Not an agent of this session")]
    NotAnAgent,
    /// A transcript that's there but can't be opened.
    #[error(transparent)]
    Read(#[from] nodus_transcripts::ReadError),
    #[error("That action no longer exists")]
    NoAction,
    #[error("That session is no longer on the board")]
    NotOnBoard,
    #[error("It’s waiting for your answer. Open it to reply.")]
    Waiting,
    /// Starting the session's process failed (the host's error, 02).
    #[error("{0}")]
    Start(String),
}
