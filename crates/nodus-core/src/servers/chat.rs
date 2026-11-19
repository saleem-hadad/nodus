//! What the app keeps of a server's sessions for their chats (SRV-166,
//! SRV-167): whether the server reads transcripts out at all, each
//! session's status as its card last had it, and its queued prompts.

use std::sync::Arc;

use nodus_domain::{Event, QueuedPrompt, SessionCard, SessionStatus};
use nodus_remote::{Remote, Request};

use super::Remotes;
use crate::nodus::{Inner, lock};

impl Remotes {
    /// The version a server runs, once it has said hello.
    fn version(&self, server_id: &str) -> Option<String> {
        self.get(server_id)?.status().hello.map(|hello| hello.version)
    }

    /// SRV-166: a server answered `session:chat` with `Not available on a
    /// server`, being older than the call.
    pub(crate) fn chat_unavailable(&self, server_id: &str) {
        let version = self.version(server_id);
        lock(&self.no_chat).insert(server_id.to_owned(), version);
    }

    /// Whether a server is known not to read transcripts out: it said so
    /// while running the version it runs now (or can't say what it runs).
    pub(crate) fn lacks_chat(&self, server_id: &str) -> bool {
        let Some(said) = lock(&self.no_chat).get(server_id).cloned() else {
            return false;
        };
        let now = self.version(server_id);
        now.is_none() || now == said
    }

    /// The cards' statuses in a server board's snapshot (QUEUE-013).
    pub(crate) fn keep_statuses(&self, cards: &[SessionCard]) {
        let mut statuses = lock(&self.statuses);
        for card in cards {
            statuses.insert(card.node.session_id.clone(), (card.status, card.live));
        }
    }

    /// QUEUE-013: a server session works or waits on the user while it
    /// runs, as its card last had it, so what's sent to it is queued.
    pub(crate) fn busy(&self, session_id: &str) -> bool {
        lock(&self.statuses)
            .get(session_id)
            .is_some_and(|&(status, live)| live && matches!(status, SessionStatus::Working | SessionStatus::Waiting))
    }

    /// Keeps what a server's event says of a session's status or queue.
    pub(crate) fn heard(&self, event: &Event) {
        match event {
            Event::Status {
                session_id,
                status,
                live,
            } => {
                lock(&self.statuses).insert(session_id.clone(), (*status, *live));
            }
            Event::Queue { session_id, items } => {
                lock(&self.queues).insert(session_id.clone(), Some(items.clone()));
            }
            _ => {}
        }
    }

    /// A server session's queued prompts as they're kept: `Some` once
    /// they're known or being asked for (empty until the answer comes),
    /// none before.
    pub(crate) fn kept_queue(&self, session_id: &str) -> Option<Vec<QueuedPrompt>> {
        lock(&self.queues)
            .get(session_id)
            .map(|items| items.clone().unwrap_or_default())
    }
}

/// SRV-167: asks a server for a session's queued prompts, which are kept
/// from the answer on and said with `Event::Queue`, unless a queue event
/// came first. When the asking fails, the next look asks again.
pub(crate) fn fetch_queue(core: &Arc<Inner>, remote: Arc<Remote>, session_id: &str) {
    lock(&core.remotes.queues).insert(session_id.to_owned(), None);
    let (weak, session_id) = (Arc::downgrade(core), session_id.to_owned());
    core.runtime.spawn(async move {
        let asked = remote
            .call_as::<Vec<QueuedPrompt>>(&Request::SessionQueue {
                session_id: session_id.clone(),
            })
            .await;
        let Some(core) = weak.upgrade() else {
            return;
        };
        let items = {
            let mut queues = lock(&core.remotes.queues);
            if !matches!(queues.get(&session_id), Some(None)) {
                return;
            }
            match asked {
                Ok(items) => {
                    queues.insert(session_id.clone(), Some(items.clone()));
                    items
                }
                Err(_) => {
                    queues.remove(&session_id);
                    return;
                }
            }
        };
        let _ = core.events.send(Event::Queue { session_id, items });
    });
}

/// Back from being unreachable, a server's queues are asked for again: they
/// may have changed while no events came.
pub(super) fn requeue(core: &Arc<Inner>, server_id: &str) {
    let Some(remote) = core.remotes.get(server_id) else {
        return;
    };
    let kept: Vec<String> = {
        let sessions = lock(&core.remotes.sessions);
        lock(&core.remotes.queues)
            .keys()
            .filter(|id| sessions.get(*id).is_some_and(|home| home.server_id == server_id))
            .cloned()
            .collect()
    };
    for session_id in kept {
        fetch_queue(core, Arc::clone(&remote), &session_id);
    }
}
