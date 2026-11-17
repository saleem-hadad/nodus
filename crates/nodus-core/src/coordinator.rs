//! `window.api.coordinator` (spec 22, beta) and what the core does for a
//! board's coordinator: the playbook and ask rules it starts with, the note
//! its members start with, what they do turned into events (their turns
//! ending, waiting for the user, their pull requests changing), and the
//! reports sent every few seconds once events settle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use std::time::Duration;

use nodus_board::coordinator::{coordinator_of, is_member, member_note, playbook};
use nodus_domain::{
    CardPr, CoordinatorEventKind, CreateSessionOptions, PrChecks, PrState, SessionCard, SessionStatus,
    coordinator_message,
};
use serde_json::{Value, json};

use crate::error::OnChannel;
use crate::nodus::{Inner, Nodus, lock};
use crate::wiring::CoreLauncher;
use crate::{CommandError, CommandResult, Delivery};

/// How often settled events are looked for (COORD-021).
const TICK: Duration = Duration::from_secs(5);
/// COORD-020: a member waiting this long is reported.
const WAITING_MS: f64 = 60_000.0;

/// What a session starts with for its board's coordinator.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Coordination {
    /// The playbook or a member's note, after `--append-system-prompt`.
    pub note: Option<String>,
    /// COORD-031: the coordinator's ask rules go in its `--settings`.
    pub ask_rules: bool,
}

/// Members waiting for the user: since when, and whether it was reported.
#[derive(Default)]
pub(crate) struct Waits(Mutex<HashMap<String, (f64, bool)>>);

/// COORD-021: sends the reports that are due every few seconds.
pub(crate) async fn follow_coordinators(core: Weak<Inner>) {
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let Some(inner) = core.upgrade() else {
            return;
        };
        Nodus { inner }.send_due_reports().await;
    }
}

impl Inner {
    /// COORD-030, COORD-031, COORD-040: the coordinator's playbook and ask
    /// rules, or a member's note; nothing for anyone else.
    pub(crate) fn coordination(&self, project_id: &str, session_id: &str) -> Coordination {
        let found = self.store.read(|state| {
            let board = state.board(project_id)?;
            let c = coordinator_of(board)?.session_id.clone();
            if c == session_id {
                return Some((c, true, false));
            }
            let node = board.nodes.get(session_id)?;
            let started = node.started_by().as_deref() == Some(c.as_str());
            is_member(node, &c).then_some((c, false, started))
        });
        match found {
            Some((_, true, _)) => Coordination {
                note: Some(playbook()),
                ask_rules: true,
            },
            Some((c, false, started)) => {
                let title = self
                    .board
                    .session_title(project_id, &c)
                    .unwrap_or_else(|_| "Coordinator".into());
                Coordination {
                    note: Some(member_note(&title, started)),
                    ask_rules: false,
                }
            }
            None => Coordination::default(),
        }
    }

    /// COORD-020: a member's status changed: its turn ended, or it started or
    /// stopped waiting for the user.
    pub(crate) fn coordinator_status(&self, session_id: &str, status: SessionStatus) {
        let mut waits = lock(&self.coordinator_waits.0);
        match status {
            SessionStatus::Waiting => {
                waits.entry(session_id.to_owned()).or_insert((self.board.now(), false));
            }
            SessionStatus::Working => {
                waits.remove(session_id);
            }
            SessionStatus::Done => {
                waits.remove(session_id);
                drop(waits);
                if self.board.reports_to_coordinator(session_id) {
                    let words = self.board.turn_words(session_id);
                    self.board
                        .coordinator_heard(session_id, CoordinatorEventKind::Turn, &words);
                }
            }
        }
    }

    /// COORD-020: a card's pull requests as they were and as they are now.
    pub(crate) fn coordinator_prs(&self, session_id: &str, was: &[CardPr], now: &[CardPr]) {
        for pr in now {
            let before = was.iter().find(|p| p.number == pr.number);
            let Some(said) = pr_change(before, pr) else {
                continue;
            };
            let title = if pr.title.is_empty() {
                "Pull request"
            } else {
                pr.title.as_str()
            };
            let text = format!("#{} {title} · {said}", pr.number);
            self.board
                .coordinator_heard(session_id, CoordinatorEventKind::Pr, &text);
        }
    }
}

/// COORD-020: what changed about a pull request worth a report: it showed
/// up, its state changed, or its checks settled.
fn pr_change(before: Option<&CardPr>, now: &CardPr) -> Option<&'static str> {
    let state = |p: &CardPr| match p.state {
        Some(PrState::Open) => Some("open"),
        Some(PrState::Draft) => Some("draft"),
        Some(PrState::Merged) => Some("merged"),
        Some(PrState::Closed) => Some("closed"),
        None => None,
    };
    let Some(before) = before else {
        return state(now).map(|_| "opened");
    };
    if now.state.is_some() && before.state != now.state {
        return state(now);
    }
    if before.checks != now.checks {
        return match now.checks {
            Some(PrChecks::Passed) => Some("checks passed"),
            Some(PrChecks::Failed) => Some("checks failed"),
            _ => None,
        };
    }
    None
}

impl Nodus {
    /// `window.api.coordinator` (spec 22).
    pub fn coordinator(&self) -> CoordinatorApi<'_> {
        CoordinatorApi { nodus: self }
    }

    /// COORD-020, COORD-021: members waiting long enough become events, and
    /// the reports that are due go to their coordinators.
    pub async fn send_due_reports(&self) {
        let inner = &self.inner;
        let now = inner.board.now();
        let waited: Vec<String> = {
            let mut waits = lock(&inner.coordinator_waits.0);
            waits
                .iter_mut()
                .filter(|(_, (since, told))| !*told && now - *since >= WAITING_MS)
                .map(|(id, (_, told))| {
                    *told = true;
                    id.clone()
                })
                .collect()
        };
        for id in waited {
            inner
                .board
                .coordinator_heard(&id, CoordinatorEventKind::Waiting, "is waiting for the user");
        }
        for (project_id, coordinator, text) in inner.board.due_reports(now) {
            if let Err(err) = self.sessions().deliver(&project_id, &coordinator, &text).await {
                tracing::warn!(%err, "[coordinator] couldn't send a report");
            }
        }
    }
}

/// The coordinator's tools' work (COORD-033, COORD-034), for the board
/// tools' handlers.
pub(crate) struct CoreCoordinatorTools(pub(crate) Weak<Inner>);

impl CoreCoordinatorTools {
    fn nodus(&self) -> Result<Nodus, String> {
        self.0
            .upgrade()
            .map(|inner| Nodus { inner })
            .ok_or_else(|| "Nodus is closing".to_owned())
    }
}

impl nodus_tools::CoordinatorTools for CoreCoordinatorTools {
    fn start_session<'a>(
        &'a self,
        coordinator: &'a str,
        title: &'a str,
        prompt: &'a str,
    ) -> nodus_tools::BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let nodus = self.nodus()?;
            let card = nodus.coordinator().start_session(coordinator, title, prompt).await?;
            Ok(json!({ "session_id": card.node.session_id, "title": card.node.title, "inside": true }))
        })
    }

    fn message_session<'a>(
        &'a self,
        coordinator: &'a str,
        session_id: &'a str,
        text: &'a str,
    ) -> nodus_tools::BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let nodus = self.nodus()?;
            let queued = nodus.coordinator().message(coordinator, session_id, text).await?;
            Ok(json!({ "ok": true, "queued": queued }))
        })
    }
}

/// `window.api.coordinator`.
pub struct CoordinatorApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl CoordinatorApi<'_> {
    fn local(&self, channel: &'static str, project_id: &str) -> CommandResult<()> {
        match self.nodus.route_project(project_id) {
            Some(_) => Err(CommandError::new(channel, "Coordinators aren’t on a Nodus server yet")),
            None => Ok(()),
        }
    }

    /// `coordinator:create` (COORD-012): a coordinator on its own new session,
    /// at `opts.x`, `opts.y`, on `opts.initialPrompt`.
    pub async fn create(&self, project_id: &str, opts: &CreateSessionOptions) -> CommandResult<SessionCard> {
        self.local("coordinator:create", project_id)?;
        self.nodus
            .on_core(async {
                self.nodus.env().await;
                let launcher = CoreLauncher(Arc::downgrade(&self.nodus.inner));
                self.nodus
                    .inner
                    .board
                    .create_coordinator(project_id, opts, &launcher)
                    .await
                    .on("coordinator:create")
            })
            .await
    }

    /// COORD-051, COORD-054: a member taken inside, or shown on the board.
    pub fn set_inside(&self, project_id: &str, session_id: &str, inside: bool) -> CommandResult<()> {
        self.local("coordinator:inside", project_id)?;
        self.nodus
            .inner
            .board
            .set_inside(project_id, session_id, inside)
            .on("coordinator:inside")
    }

    /// COORD-054: a card taken out of the members, or put back.
    pub fn set_member(&self, project_id: &str, session_id: &str, member: bool) -> CommandResult<()> {
        self.local("coordinator:member", project_id)?;
        self.nodus
            .inner
            .board
            .set_member(project_id, session_id, member)
            .on("coordinator:member")
    }

    /// COORD-013: `Stop coordinating`.
    pub fn stop(&self, project_id: &str) -> CommandResult<()> {
        self.local("coordinator:stop", project_id)?;
        self.nodus.inner.board.stop_coordinating(project_id);
        Ok(())
    }

    /// COORD-033: `start_session` from the coordinator's session.
    pub async fn start_session(&self, coordinator: &str, title: &str, prompt: &str) -> Result<SessionCard, String> {
        self.nodus
            .on_core(async {
                self.nodus.env().await;
                let launcher = CoreLauncher(Arc::downgrade(&self.nodus.inner));
                self.nodus
                    .inner
                    .board
                    .start_inside(coordinator, title, prompt, &launcher)
                    .await
                    .map_err(|err| match err {
                        nodus_board::BoardError::Invalid(why) => why,
                        err => format!("Couldn’t start it: {err}"),
                    })
            })
            .await
    }

    /// COORD-034: `message_session` from the coordinator's session. Whether
    /// it waits for the member's turn to end.
    pub async fn message(&self, coordinator: &str, session_id: &str, text: &str) -> Result<bool, String> {
        let board = &self.nodus.inner.board;
        let Some((project_id, _)) = board.coordinator_of_session(coordinator) else {
            return Err("message_session is only for the board’s coordinator".into());
        };
        let member = board
            .read_node(&project_id, session_id)
            .is_some_and(|n| is_member(&n, coordinator));
        if !member {
            return Err("That session isn’t one of your members".into());
        }
        if text.trim().is_empty() {
            return Err("There’s nothing to send".into());
        }
        let title = board
            .session_title(&project_id, coordinator)
            .unwrap_or_else(|_| "Coordinator".into());
        let message = coordinator_message(&title, text);
        match self.nodus.sessions().deliver(&project_id, session_id, &message).await {
            Ok(Delivery::Queued) => Ok(true),
            Ok(Delivery::Sent) => Ok(false),
            Err(err) => Err(err.message().to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(state: Option<PrState>, checks: Option<PrChecks>) -> CardPr {
        CardPr {
            url: "https://github.com/o/n/pull/64".into(),
            number: 64,
            title: "Fork".into(),
            state,
            checks,
        }
    }

    #[test]
    fn says_what_changed_about_a_pull_request() {
        let open = pr(Some(PrState::Open), Some(PrChecks::Pending));
        assert_eq!(pr_change(None, &open), Some("opened"));
        assert_eq!(pr_change(None, &pr(None, None)), None);
        assert_eq!(pr_change(Some(&open), &open), None);
        let passed = pr(Some(PrState::Open), Some(PrChecks::Passed));
        assert_eq!(pr_change(Some(&open), &passed), Some("checks passed"));
        let failed = pr(Some(PrState::Open), Some(PrChecks::Failed));
        assert_eq!(pr_change(Some(&open), &failed), Some("checks failed"));
        let merged = pr(Some(PrState::Merged), Some(PrChecks::Passed));
        assert_eq!(pr_change(Some(&passed), &merged), Some("merged"));
        // Checks starting again say nothing.
        assert_eq!(pr_change(Some(&passed), &open), None);
    }
}
