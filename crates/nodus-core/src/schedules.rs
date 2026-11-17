//! Scheduled actions' ticker (spec 20): every 30 s while the app is open, each
//! scheduled action that is due starts a run, as a press of its button does,
//! beside its tile on the board.

use std::sync::Weak;
use std::time::Duration;

use nodus_board::Launcher;
use nodus_domain::{CreateSessionOptions, TileRun};

use crate::nodus::{Inner, Nodus};
use crate::wiring::CoreLauncher;

/// How often the schedules are looked at (SCHED-020).
pub(crate) const TICK: Duration = Duration::from_secs(30);

/// SCHED-020: looks at the schedules every 30 s, the first time 30 s after
/// launch, once the session host's runs are known.
pub(crate) async fn follow_schedules(core: Weak<Inner>) {
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let Some(inner) = core.upgrade() else {
            return;
        };
        Nodus { inner }.run_due_schedules().await;
    }
}

impl Nodus {
    /// SCHED-020…SCHED-023: starts a run for each scheduled action that is
    /// due. Only the latest time it missed runs, once, marked late when it
    /// starts well after it. A time that comes while a run of it is still
    /// going is skipped.
    pub async fn run_due_schedules(&self) {
        let board = &self.inner.board;
        let now = board.now();
        for s in board.scheduled() {
            let Some(schedule) = &s.action.schedule else {
                continue;
            };
            let Some((due, late)) = nodus_actions::schedule::due_now(schedule, s.tile.handled, now) else {
                continue;
            };
            // Handled before it starts, so a slow start isn't started twice.
            board.tile_skipped(&s.project_id, &s.action.id, due);
            if s.running {
                continue;
            }
            self.env().await;
            let opts = CreateSessionOptions {
                action_id: Some(s.action.id.clone()),
                ..CreateSessionOptions::default()
            };
            let launcher = CoreLauncher(std::sync::Arc::downgrade(&self.inner));
            let run = match board
                .create_session(&s.project_id, &opts, None, &launcher as &dyn Launcher)
                .await
            {
                Ok(card) => TileRun {
                    session_id: card.node.session_id,
                    at: now,
                    due: Some(due),
                    late: late.then_some(true),
                    done_at: None,
                    message: None,
                    seen: None,
                },
                // SCHED-022: one that can't start says why on its tile.
                Err(err) => TileRun {
                    session_id: String::new(),
                    at: now,
                    due: Some(due),
                    late: late.then_some(true),
                    done_at: Some(now),
                    message: Some(format!("Couldn’t start: {err}")),
                    seen: None,
                },
            };
            board.tile_started(&s.project_id, &s.action.id, run);
        }
    }
}
