//! A view's brain: a Claude session of its own, hidden from the board like an
//! action run, that builds the page and keeps it up to date. It's started
//! when first needed, stops after five idle minutes, and is resumed when it's
//! next needed (VIEW-088 to VIEW-099).

use std::time::Duration;

use nodus_domain::{BoardNode, BoardView, SessionStatus, js};
use nodus_sessions::launch::{PermissionMode, SpawnOptions, bracketed_paste};
use nodus_store::BoardState;
use serde_json::{Map, Value};

use crate::ViewError;
use crate::check::{MAX_PROMPT_CHARS, clip};
use crate::page::VIEW_GUIDE;
use crate::views::{Views, no_view};

/// How long a brain sits at its prompt before it's stopped (VIEW-096).
pub const BRAIN_IDLE: Duration = Duration::from_millis(5 * 60 * 1000);
/// Claude's prompt takes a moment to come back after a turn ends (VIEW-096).
const QUEUED_DELAY: Duration = Duration::from_millis(500);
/// Return goes this long after a pasted prompt (VIEW-095).
const RETURN_DELAY: Duration = Duration::from_millis(200);
/// A brain leaving auto mode whose process hasn't gone yet (VIEW-156).
const BRAIN_STOPPING: &str = "The view’s brain is still stopping; try again in a moment";

/// Who is asking a brain: the user (the card, the ask bar), the page
/// (`nodus.ask`), or a refresh schedule, which skips a busy brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskOrigin {
    User,
    Page,
    Schedule,
}

/// The brain's system prompt, built from the view as it is now (Data & formats).
pub fn brain_prompt(view: &BoardView) -> String {
    format!(
        "You are the brain of a view on the user's Nodus board: a small live page right on the board, beside their Claude Code sessions. The view is \"{title}\" (id {id}). The user asked for:\n\n{description}\n\nYou build the page and keep it up to date with the mcp__nodus__* board tools. update_view sets its html, data, title, commands and refresh schedule. get_view shows it as it is now, with its commands' last output and any JavaScript errors it hit. Leave out view_id: both act on this view. If you're picking up a view that already has a page, read it with get_view before changing it.\n\n{VIEW_GUIDE}\n\nNobody watches you while you work, so don't stop to chat. Work out where the data comes from by looking around the repository, the CLIs on the PATH and your MCP tools. If you need the user, for a missing API token or a choice only they can make, ask with AskUserQuestion, which shows on the view that you need them. End each turn with one short line on what you did.",
        title = view.title,
        id = view.id,
        description = view.description,
    )
}

/// Extra `claude` options for a view's brain, every time it starts, resumes,
/// is opened or is sent a prompt; none for a node that isn't a brain of a
/// view on its board (VIEW-090).
pub fn brain_args(board: &BoardState, node: &BoardNode) -> Option<Vec<String>> {
    let view = board.views.get(node.view_of.as_deref().filter(|v| !v.is_empty())?)?;
    Some(vec!["--append-system-prompt".into(), brain_prompt(view)])
}

/// A brain's card title (VIEW-088, VIEW-091).
pub(crate) fn brain_title(view_title: &str) -> String {
    format!("View: {view_title}")
}

impl Views {
    /// Starts a fresh brain for a view, on `prompt` if there is one. Its card
    /// is on the board while it starts, and taken off again if it can't
    /// (VIEW-088, VIEW-089).
    pub(crate) async fn start_brain(
        &self,
        project_id: &str,
        view_id: &str,
        prompt: Option<String>,
    ) -> Result<String, ViewError> {
        let project = self.project(project_id)?;
        let now = self.now();
        let session_id = uuid::Uuid::new_v4().to_string();
        let (previous, extra_args, auto) = self.update_view(project_id, view_id, |board, id| {
            let view = &board.views[id];
            // VIEW-155
            let auto = view.runs_in_auto_mode();
            let node = BoardNode {
                session_id: session_id.clone(),
                kind: None,
                x: view.x,
                y: view.y,
                archived: false,
                title: Some(brain_title(&view.title)),
                parents: None,
                created_by_app: Some(true),
                run_of: None,
                expanded: None,
                view_of: Some(id.to_owned()),
                auto_mode: auto.then_some(true),
                from_view: None,
                created_at: now,
                w: None,
                h: None,
                archived_at: None,
                section_id: None,
                extra: Map::new(),
            };
            let extra_args = brain_args(board, &node);
            board.nodes.insert(session_id.clone(), node);
            let previous = board.views[id].session_id.replace(session_id.clone());
            Ok((previous, extra_args, auto))
        })?;
        let spawned = self
            .inner
            .brains
            .spawn(SpawnOptions {
                session_id: session_id.clone(),
                project_id: project_id.to_owned(),
                cwd: project.repo_path,
                resume: false,
                initial_prompt: prompt,
                permission_mode: auto.then_some(PermissionMode::Auto),
                extra_args,
                ..SpawnOptions::default()
            })
            .await;
        if let Err(err) = spawned {
            self.inner.store.update(|state| {
                let board = state.board_mut(project_id);
                board.nodes.shift_remove(&session_id);
                if let Some(view) = board.views.get_mut(view_id) {
                    view.session_id = previous;
                }
            });
            return Err(ViewError::new(err));
        }
        self.board_changed(project_id);
        Ok(session_id)
    }

    /// `views:brain`: the view's brain, started idle if it has none, for the
    /// board to open (VIEW-092, VIEW-120).
    pub async fn brain(&self, project_id: &str, view_id: &str) -> Result<String, ViewError> {
        self.env().await;
        self.ensure_brain(project_id, view_id).await
    }

    /// The view's brain, started idle if it has none (VIEW-092).
    pub async fn ensure_brain(&self, project_id: &str, view_id: &str) -> Result<String, ViewError> {
        let view = self.view(project_id, view_id).ok_or_else(no_view)?;
        if let Some(node) = self.brain_node(project_id, &view) {
            return Ok(self.settle_brain_mode(project_id, &view, node).await?.session_id);
        }
        self.start_brain(project_id, view_id, None).await
    }

    /// VIEW-156: a brain whose view isn't in auto mode any more leaves it. A
    /// process still running in auto mode is stopped, to be resumed in the
    /// view's mode when it's next needed, and only then does its node stop
    /// saying `autoMode`, so one that outlives the kill is stopped next time.
    /// Its transcript stays.
    pub(crate) async fn settle_brain_mode(
        &self,
        project_id: &str,
        view: &BoardView,
        mut node: BoardNode,
    ) -> Result<BoardNode, ViewError> {
        if node.auto_mode != Some(true) || view.runs_in_auto_mode() {
            return Ok(node);
        }
        let sessions = &self.inner.sessions;
        if sessions.is_live(&node.session_id) {
            sessions.kill(&node.session_id).await;
            if sessions.is_live(&node.session_id) {
                return Err(ViewError::new(BRAIN_STOPPING));
            }
        }
        node.auto_mode = None;
        self.inner.store.update(|state| {
            if let Some(kept) = state.board_mut(project_id).nodes.get_mut(&node.session_id) {
                kept.auto_mode = None;
            }
        });
        Ok(node)
    }

    pub(crate) fn brain_node(&self, project_id: &str, view: &BoardView) -> Option<BoardNode> {
        let id = view.session_id.as_deref()?;
        self.inner
            .store
            .read(|state| state.board(project_id)?.nodes.get(id).cloned())
    }

    /// `views:ask`: the user asking a view's brain from its card (VIEW-120).
    pub async fn ask(&self, project_id: &str, view_id: &str, prompt: &str) -> Result<(), ViewError> {
        self.env().await;
        self.ask_brain(project_id, view_id, prompt, AskOrigin::User).await
    }

    /// Passes `prompt` to a view's brain: typed in if it's waiting at its
    /// prompt, queued for when its turn ends if it's busy, or with the brain
    /// resumed (or a first one started) if it isn't running. A scheduled
    /// refresh skips a busy brain rather than queue behind it (VIEW-093, VIEW-094).
    pub async fn ask_brain(
        &self,
        project_id: &str,
        view_id: &str,
        prompt: &str,
        origin: AskOrigin,
    ) -> Result<(), ViewError> {
        let view = self.view(project_id, view_id).ok_or_else(no_view)?;
        let text = clip(Some(&Value::String(prompt.to_owned())), MAX_PROMPT_CHARS);
        if text.is_empty() {
            return Err(ViewError::new("Say what the view’s brain should do"));
        }
        let Some(node) = self.brain_node(project_id, &view) else {
            self.start_brain(project_id, view_id, Some(text)).await?;
            return Ok(());
        };
        let node = self.settle_brain_mode(project_id, &view, node).await?;
        let id = node.session_id.clone();
        let sessions = &self.inner.sessions;
        if sessions.is_live(&id) {
            if sessions.status(&id) == SessionStatus::Done {
                self.type_into(&id, &text);
            } else if origin != AskOrigin::Schedule {
                let mut state = self.lock();
                let queued = match state.pending_asks.remove(&id) {
                    Some(queued) => {
                        let joined = format!("{queued}\n\n{text}");
                        let len = js::utf16_len(&joined);
                        js::slice_utf16(&joined, len.saturating_sub(MAX_PROMPT_CHARS), len)
                    }
                    None => text,
                };
                state.pending_asks.insert(id, queued);
            }
            return Ok(());
        }
        let project = self.project(project_id)?;
        let resume = self.inner.board.has_transcript(&project, &id);
        let extra_args = self
            .inner
            .store
            .read(|state| state.board(project_id).and_then(|b| brain_args(b, &node)));
        self.inner
            .brains
            .spawn(SpawnOptions {
                session_id: id,
                project_id: project_id.to_owned(),
                cwd: project.repo_path,
                resume,
                initial_prompt: Some(text),
                // VIEW-155
                permission_mode: view.runs_in_auto_mode().then_some(PermissionMode::Auto),
                extra_args,
                ..SpawnOptions::default()
            })
            .await
            .map_err(ViewError::new)
    }

    /// Types a prompt at Claude's prompt as a paste, so lines don't send it
    /// early, then sends it (VIEW-095). The idle timer is cancelled but its
    /// entry kept, so the user's input still counts until the next status.
    fn type_into(&self, session_id: &str, text: &str) {
        if let Some(Some(timer)) = self.lock().idle_timers.get_mut(session_id).map(Option::take) {
            timer.abort();
        }
        self.inner.brains.write(session_id, &bracketed_paste(text));
        let brains = std::sync::Arc::clone(&self.inner.brains);
        let id = session_id.to_owned();
        self.spawn(async move {
            tokio::time::sleep(RETURN_DELAY).await;
            brains.write(&id, "\r");
        });
    }

    /// A session's status changed. When a brain's turn ends it takes what was
    /// asked meanwhile, or stops once it has been idle for a while: it's
    /// resumed when it's next needed, and nobody is looking at it in between
    /// (VIEW-096, VIEW-097). nodus-core calls it on every status change.
    pub fn brain_status(&self, session_id: &str, status: SessionStatus, live: bool) {
        let is_brain = self.inner.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            let node = state.board(project_id)?.nodes.get(session_id)?;
            Some(node.view_of.as_deref().is_some_and(|v| !v.is_empty()))
        });
        if is_brain != Some(true) {
            return;
        }
        let queued = {
            let mut state = self.lock();
            if let Some(Some(timer)) = state.idle_timers.remove(session_id) {
                timer.abort();
            }
            if status != SessionStatus::Done || !live {
                return;
            }
            state.pending_asks.remove(session_id)
        };
        let id = session_id.to_owned();
        let views = self.clone();
        if let Some(queued) = queued {
            self.spawn(async move {
                tokio::time::sleep(QUEUED_DELAY).await;
                views.type_into(&id, &queued);
            });
            return;
        }
        let weak = std::sync::Arc::downgrade(&self.inner);
        drop(views);
        let timer = self.spawn(async move {
            tokio::time::sleep(BRAIN_IDLE).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let views = Views { inner };
            let last_input = {
                let mut state = views.lock();
                state.idle_timers.remove(&id);
                state.last_input.get(&id).copied().unwrap_or(0.0)
            };
            let sessions = std::sync::Arc::clone(&views.inner.sessions);
            let quiet = views.now() - last_input >= BRAIN_IDLE.as_millis() as f64;
            if sessions.status(&id) == SessionStatus::Done && quiet {
                sessions.kill(&id).await;
            } else if sessions.status(&id) == SessionStatus::Done {
                views.brain_status(&id, SessionStatus::Done, sessions.is_live(&id));
            }
        });
        self.lock().idle_timers.insert(session_id.to_owned(), Some(timer));
    }

    /// The user typed into a session; an idle brain they're talking to isn't
    /// idle (VIEW-098). nodus-core calls it on every `session:write`.
    pub fn brain_input(&self, session_id: &str) {
        let now = self.now();
        let mut state = self.lock();
        if state.idle_timers.contains_key(session_id) {
            state.last_input.insert(session_id.to_owned(), now);
        }
    }

    pub(crate) fn forget_brain(&self, session_id: &str) {
        let mut state = self.lock();
        if let Some(Some(timer)) = state.idle_timers.remove(session_id) {
            timer.abort();
        }
        state.pending_asks.remove(session_id);
        state.last_input.remove(session_id);
    }
}
