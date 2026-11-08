//! Where actions are kept: a project's own on the project, the global ones at
//! the top of state.json (ACT-002), and the `actions:global`, `actions:save`
//! and `actions:remove` commands (ACT-013…ACT-015).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nodus_domain::{ActionLists, Project, ProjectAction};
use nodus_store::{State, Store};
use serde_json::Value;

use crate::ActionError;
use crate::normalize::normalize_action;

/// The actions in state.json.
pub struct Actions {
    store: Arc<Store>,
}

impl std::fmt::Debug for Actions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Actions")
    }
}

/// Now, in ms, as `Date.now()` gives it.
pub(crate) fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// One of the project's own actions, or a global one, by id.
pub fn find_action<'a>(project: &'a Project, global: &'a [ProjectAction], id: &str) -> Option<&'a ProjectAction> {
    project.actions.iter().flatten().chain(global).find(|a| a.id == id)
}

/// ACT-033's `isRun`: whether the session's card is an action run, which
/// gives it the board's session tools (04 asks per call).
pub fn is_run(state: &State, session_id: &str) -> bool {
    state
        .project_id_for_session(session_id)
        .and_then(|p| state.board(p))
        .and_then(|b| b.nodes.get(session_id))
        .is_some_and(|n| n.run_of.as_deref().is_some_and(|r| !r.is_empty()))
}

/// Into the list its scope names, in place if it's there already; out of the other.
fn put(list: Vec<ProjectAction>, action: &ProjectAction, belongs: bool) -> Vec<ProjectAction> {
    if !belongs {
        return list.into_iter().filter(|a| a.id != action.id).collect();
    }
    if list.iter().any(|a| a.id == action.id) {
        list.into_iter()
            .map(|a| if a.id == action.id { action.clone() } else { a })
            .collect()
    } else {
        let mut list = list;
        list.push(action.clone());
        list
    }
}

/// `setActions`: stores both lists, leaving out a list that's empty.
fn set_actions(
    state: &mut State,
    project_id: &str,
    own: Vec<ProjectAction>,
    global: Vec<ProjectAction>,
) -> ActionLists {
    let project = state.project_mut(project_id).expect("the project was found");
    project.actions = (!own.is_empty()).then_some(own);
    let project = project.clone();
    state.actions = (!global.is_empty()).then(|| global.clone());
    ActionLists { project, global }
}

impl Actions {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// ACT-013: `actions:global`.
    pub fn global(&self) -> Vec<ProjectAction> {
        self.store.read(|state| state.actions.clone().unwrap_or_default())
    }

    /// ACT-014: `actions:save(projectId, raw)`: the action, normalized, put in
    /// the list its scope names and taken out of the other, so turning
    /// `global` on or off moves it.
    pub fn save(&self, project_id: &str, raw: &Value) -> Result<ActionLists, ActionError> {
        self.save_at(project_id, raw, now_ms())
    }

    /// [`Actions::save`] at a given time, for tests.
    pub fn save_at(&self, project_id: &str, raw: &Value, now: f64) -> Result<ActionLists, ActionError> {
        self.store.update(|state| {
            let project = state.project(project_id)?;
            let own = project.actions.clone().unwrap_or_default();
            let global = state.actions.clone().unwrap_or_default();
            let raw_id = raw.get("id").and_then(Value::as_str);
            let existing = raw_id.and_then(|id| own.iter().chain(&global).find(|a| a.id == id));
            // `raw ?? {}`: null is nothing.
            let action = normalize_action(raw, existing, now)?;
            let is_global = action.global == Some(true);
            let own = put(own, &action, !is_global);
            let global = put(global, &action, is_global);
            Ok(set_actions(state, project_id, own, global))
        })
    }

    /// ACT-015: `actions:remove(projectId, actionId)`: from the project's own
    /// list and the global one, so a global action goes from every project. A
    /// run already going keeps going.
    pub fn remove(&self, project_id: &str, action_id: &str) -> Result<ActionLists, ActionError> {
        self.store.update(|state| {
            let project = state.project(project_id)?;
            let drop = |list: Option<&Vec<ProjectAction>>| {
                list.into_iter()
                    .flatten()
                    .filter(|a| a.id != action_id)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let own = drop(project.actions.as_ref());
            let global = drop(state.actions.as_ref());
            Ok(set_actions(state, project_id, own, global))
        })
    }
}
