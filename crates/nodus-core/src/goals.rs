//! `window.api.goals` (spec 21): starting a goal, changing it, a check-in now,
//! taking it off the board. The 30 s ticker wakes the ones that are due
//! (`schedules.rs`).

use nodus_board::goals::NewGoal;
use nodus_domain::BoardGoal;
use nodus_remote::Request;
use serde_json::Value;

use crate::error::OnChannel;
use crate::wiring::CoreLauncher;
use crate::{CommandResult, Nodus};

/// `window.api.goals`.
pub struct GoalsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl GoalsApi<'_> {
    /// `goals:create` (GOAL-010…GOAL-013): `{ mission, every?, at?: {x, y},
    /// sessionId? }`. Starts its session (or resumes the one given) on the
    /// first check-in.
    pub async fn create(&self, project_id: &str, raw: &Value) -> CommandResult<BoardGoal> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, goal) = (project_id.to_owned(), raw.clone());
            return route.call(Request::GoalsCreate { project_id, goal }).await;
        }
        let new = NewGoal {
            mission: raw
                .get("mission")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            every: raw.get("every").and_then(Value::as_f64),
            at: raw
                .get("at")
                .and_then(|at| Some((at.get("x")?.as_f64()?, at.get("y")?.as_f64()?))),
            session_id: raw.get("sessionId").and_then(Value::as_str).map(str::to_owned),
        };
        self.nodus
            .on_core(async {
                self.nodus.env().await;
                let launcher = CoreLauncher(std::sync::Arc::downgrade(&self.nodus.inner));
                self.nodus
                    .inner
                    .board
                    .create_goal(project_id, &new, &launcher)
                    .await
                    .on("goals:create")
            })
            .await
    }

    /// `goals:change` (GOAL-032): `{ status?, mission?, every?, x?, y? }`.
    pub async fn change(&self, project_id: &str, id: &str, patch: &Value) -> CommandResult<BoardGoal> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, id, patch) = (project_id.to_owned(), id.to_owned(), patch.clone());
            return route.call(Request::GoalsChange { project_id, id, patch }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .board
                    .change_goal(project_id, id, patch)
                    .on("goals:change")
            })
            .await
    }

    /// `goals:check-in` (GOAL-032): a check-in now, whenever the next was due.
    pub async fn check_in(&self, project_id: &str, id: &str) -> CommandResult<()> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, id) = (project_id.to_owned(), id.to_owned());
            return route.call(Request::GoalsCheckIn { project_id, id }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.env().await;
                let launcher = CoreLauncher(std::sync::Arc::downgrade(&self.nodus.inner));
                self.nodus
                    .inner
                    .board
                    .check_in(project_id, id, &launcher)
                    .await
                    .on("goals:check-in")
            })
            .await
    }

    /// `goals:remove` (GOAL-033): its session stays.
    pub async fn remove(&self, project_id: &str, id: &str) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, id) = (project_id.to_owned(), id.to_owned());
            return route.run(Request::GoalsRemove { project_id, id }).await;
        }
        self.nodus
            .on_core(async { self.nodus.inner.board.remove_goal(project_id, id) })
            .await;
    }
}
