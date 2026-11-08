//! What the app tells whoever is watching, one variant per `api.on.*` event in
//! `src/shared/api.ts` (the `ev:*` IPC channels).
//!
//! Domain crates emit these into an [`EventSink`] they're handed, and
//! `nodus-core` passes them on: to the window today, and to a remote client
//! once Nodus can run headless.

use serde_json::Value;

use crate::{
    ActionRunResult, CaptureContext, CaptureShortcut, ClipboardOffer, MenuCommand, OpenCardRequest, SessionStatus,
    SessionUsage, SummarizeProgress, TabAttachment, UpdateStatus, ViewEvent,
};

/// One event, named after its channel.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `ev:pty-data`: output from a session's terminal.
    PtyData { session_id: String, data: String },
    /// `ev:pty-exit`: a session's process ended.
    PtyExit { session_id: String, exit_code: i32 },
    /// `ev:status`: a session started working, needs the user, or finished.
    Status {
        session_id: String,
        status: SessionStatus,
        live: bool,
    },
    /// `ev:agent-status`: a subagent started working, needs the user, or finished.
    AgentStatus {
        session_id: String,
        agent_id: String,
        status: SessionStatus,
    },
    /// `ev:usage`: a session's model, context, running time or cost changed.
    Usage { session_id: String, usage: SessionUsage },
    /// `ev:board-changed`: transcripts or board state changed on disk.
    BoardChanged { project_id: String },
    /// `ev:projects-changed`: a project was added from outside the window.
    ProjectsChanged,
    /// `ev:summarize-progress`.
    SummarizeProgress(SummarizeProgress),
    /// `ev:run-finished`: an action run finished and left the board.
    RunFinished(ActionRunResult),
    /// `ev:menu`: a menu command for the window to carry out.
    Menu(MenuCommand),
    /// `ev:update-status`.
    UpdateStatus(UpdateStatus),
    /// `ev:view`: new data, saved state or a command's result for a view's page.
    View {
        project_id: String,
        view_id: String,
        event: ViewEvent,
        payload: Value,
    },
    /// `ev:open-card`: the user clicked a notification about a card.
    OpenCard(OpenCardRequest),
    /// `ev:capture-shown`: the capture panel came up, over this.
    CaptureShown(CaptureContext),
    /// `ev:capture-context`: more of what the panel came up over, arriving after it.
    CaptureContext(CaptureContextPatch),
}

/// `Partial<CaptureContext>`: the parts of the context that arrived late.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaptureContextPatch {
    pub app: Option<String>,
    pub tab: Option<TabAttachment>,
    pub clipboard: Option<ClipboardOffer>,
    pub shortcut: Option<CaptureShortcut>,
}

/// Where a domain sends its events.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

impl<F: Fn(Event) + Send + Sync> EventSink for F {
    fn emit(&self, event: Event) {
        self(event);
    }
}

/// Keeps every event it's given, for tests.
#[derive(Debug, Default)]
pub struct RecordingSink(std::sync::Mutex<Vec<Event>>);

impl RecordingSink {
    /// The events so far, oldest first, leaving them recorded.
    pub fn events(&self) -> Vec<Event> {
        self.lock().clone()
    }

    /// The events so far, oldest first, and forgets them.
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.lock())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Event>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: Event) {
        self.lock().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closures_and_recorders_take_events() {
        let recorder = RecordingSink::default();
        recorder.emit(Event::ProjectsChanged);
        assert_eq!(recorder.events(), vec![Event::ProjectsChanged]);
        assert_eq!(recorder.take(), vec![Event::ProjectsChanged]);
        assert!(recorder.events().is_empty());

        let seen = std::sync::Mutex::new(0);
        let count = |_: Event| *seen.lock().unwrap() += 1;
        count.emit(Event::ProjectsChanged);
        assert_eq!(*seen.lock().unwrap(), 1);
    }
}
