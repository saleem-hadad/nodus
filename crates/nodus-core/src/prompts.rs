//! Who sent each prompt to a server session (SRV-142). Anyone can type
//! into any session, so a server remembers which member's client last
//! wrote to each session's terminal, and when Claude's `UserPromptSubmit`
//! hook comes, records that member as the prompt's sender on the card
//! (`prompts`). The transcript then names each user turn's sender. A local
//! core records nothing.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError, Weak};

use nodus_domain::{CreatedBy, PromptBy, SessionStatus, TranscriptMessage, attribute_turns};
use nodus_sessions::hooks::{SessionInput, StatusMachine};
use serde_json::Value;

use crate::nodus::Inner;

/// The member whose client last wrote to each session, by session id.
#[derive(Default)]
pub(crate) struct Writers(Mutex<HashMap<String, String>>);

impl Writers {
    pub(crate) fn wrote(&self, session_id: &str, login: &str) {
        let mut writers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if writers.get(session_id).is_none_or(|last| last != login) {
            writers.insert(session_id.to_owned(), login.to_owned());
        }
    }

    fn last(&self, session_id: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }
}

/// What the session host client tells the status machine, with each
/// prompt's sender recorded on the way, on a server.
pub(crate) struct PromptInput {
    pub(crate) status: StatusMachine,
    pub(crate) core: Weak<Inner>,
}

impl SessionInput for PromptInput {
    fn process_started(&self, session_id: &str, initial: SessionStatus) {
        self.status.process_started(session_id, initial);
    }

    fn process_exited(&self, session_id: &str) {
        self.status.process_exited(session_id);
    }

    fn hook(&self, event: &str, session_id: &str, payload: &Value, at: Option<f64>) {
        self.status.receive_hook(event, session_id, payload, at);
        // A live prompt to the session itself: not one replayed from a
        // welcome (`at`), nor an agent's.
        let agent = payload
            .get("agent_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty());
        if event == "UserPromptSubmit"
            && at.is_none()
            && !agent
            && let Some(core) = self.core.upgrade()
        {
            core.prompt_submitted(session_id);
        }
    }

    fn user_input(&self, session_id: &str, data: &str, at: Option<f64>) {
        self.status.note_user_input(session_id, data, at);
    }

    fn is_waiting(&self, session_id: &str) -> bool {
        SessionInput::is_waiting(&self.status, session_id)
    }
}

impl Inner {
    /// `UserPromptSubmit` came for a session: its sender is whoever wrote
    /// to it last, else whoever started it.
    fn prompt_submitted(&self, session_id: &str) {
        let Some(hosted) = self.hosted() else { return };
        let last = hosted.writers.last(session_id);
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        self.store.update(|state| {
            let node = state
                .boards
                .values_mut()
                .find_map(|board| board.nodes.get_mut(session_id));
            let Some(node) = node else { return };
            let by = last.or_else(|| node.created_by().map(str::to_owned));
            if let Some(by) = by {
                node.push_prompt(PromptBy { at, by });
            }
        });
    }

    /// Each user turn's sender, on a server session (SRV-142).
    pub(crate) fn attribute_transcript(&self, project_id: &str, session_id: &str, messages: &mut [TranscriptMessage]) {
        if self.hosted().is_none() {
            return;
        }
        let prompts = self.store.read(|state| {
            state
                .board(project_id)
                .and_then(|board| board.nodes.get(session_id))
                .map(nodus_domain::BoardNode::prompts)
                .unwrap_or_default()
        });
        attribute_turns(messages, &prompts);
    }
}
