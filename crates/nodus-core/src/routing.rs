//! SRV-160: a server project's calls go to its server. Each API method that
//! takes a project or a session asks [`Nodus::route_project`] or
//! [`Nodus::route_session`] first; for a server project it makes the
//! matching `nodus-wire` call through that server's connection and reads
//! the answer as the type the local path gives, and otherwise runs locally
//! as before.
//!
//! What's this person's own on a server board stays here (SRV-164): the
//! viewport and which cards are expanded live in `serverBoards` in
//! state.json, are never sent, and are put into every snapshot the server
//! gives. Each server board's last snapshot is kept, so it still opens,
//! read-only, while the server can't be reached (SRV-028).

use std::sync::Arc;

use nodus_domain::{BoardSnapshot, CreateSessionOptions, NodePatch, Project, ProjectPatch, SessionCard, Viewport};
use nodus_remote::{DISCONNECTED, OFFLINE, Remote, Request, TIMED_OUT};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::projects::now_ms;
use crate::servers::{SessionHome, share};
use crate::{CommandError, CommandResult, Nodus};

/// Where a server project's calls go.
pub(crate) struct Route {
    nodus: Nodus,
    home: SessionHome,
    remote: Option<Arc<Remote>>,
}

impl Nodus {
    /// The server a project's calls go to, for a server project; none for a
    /// local one.
    pub(crate) fn route_project(&self, project_id: &str) -> Option<Route> {
        let server_id = self
            .inner
            .store
            .read(|state| state.project_server(project_id).map(str::to_owned))?;
        Some(Route {
            nodus: self.clone(),
            remote: self.inner.remotes.get(&server_id),
            home: SessionHome {
                server_id,
                project_id: project_id.to_owned(),
            },
        })
    }

    /// The server a session's calls go to, for a session on a server board
    /// this app has loaded or started (SRV-160); none for a local one.
    pub(crate) fn route_session(&self, session_id: &str) -> Option<Route> {
        let home = self.inner.remotes.session_home(session_id)?;
        Some(Route {
            nodus: self.clone(),
            remote: self.inner.remotes.get(&home.server_id),
            home,
        })
    }
}

impl Route {
    fn project_id(&self) -> String {
        self.home.project_id.clone()
    }

    /// The server the calls go to.
    pub(crate) fn server_id(&self) -> &str {
        &self.home.server_id
    }

    /// Makes the call on the server and reads the answer as the local
    /// path's type. Fails with the server's error, verbatim, on the call's
    /// channel, or with `Not connected to the server`.
    pub(crate) async fn call<T: DeserializeOwned>(&self, request: Request) -> CommandResult<T> {
        let channel = request.name();
        let Some(remote) = self.remote.clone() else {
            return Err(CommandError::new(channel, OFFLINE));
        };
        self.nodus
            .on_core(async move { remote.call_as::<T>(&request).await })
            .await
            .map_err(|err| CommandError::new(channel, err.message))
    }

    /// A call whose local command gives nothing back: a failure is logged,
    /// as the local one would only have logged it.
    pub(crate) async fn run(&self, request: Request) {
        let channel = request.name();
        if let Err(err) = self.call::<Value>(request).await {
            tracing::warn!("[remote] {channel} failed: {}", err.message);
        }
    }

    /// A call sent without waiting, in order with the others (keys typed,
    /// resizes, a view's reports).
    pub(crate) fn send(&self, request: Request) {
        let channel = request.name();
        if !self.remote.as_ref().is_some_and(|remote| remote.send(&request)) {
            tracing::warn!("[remote] {channel} not sent: {OFFLINE}");
        }
    }

    /// Remembers that a session is on this route's server board.
    pub(crate) fn remember(&self, session_id: &str) {
        self.nodus.inner.remotes.remember(session_id, &self.home);
    }

    /// `board:load` for a server board: the server's snapshot with this
    /// person's own viewport and expanded cards (SRV-164). While the server
    /// can't be reached, the last snapshot, marked `offline` (SRV-028).
    pub(crate) async fn load_board(&self) -> CommandResult<BoardSnapshot> {
        let project_id = self.project_id();
        let remotes = &self.nodus.inner.remotes;
        let (snapshot, offline) = match self
            .call::<BoardSnapshot>(Request::BoardLoad {
                project_id: project_id.clone(),
            })
            .await
        {
            Ok(snapshot) => {
                remotes.keep_board(&project_id, &snapshot, &self.nodus.inner.runtime);
                (snapshot, false)
            }
            Err(err) if unreachable(&err) => match remotes.last_board(&project_id) {
                Some(snapshot) => (snapshot, true),
                None => return Err(err),
            },
            Err(err) => return Err(err),
        };
        for card in &snapshot.cards {
            self.remember(&card.node.session_id);
        }
        for brain in snapshot.views.iter().filter_map(|v| v.session_id.as_deref()) {
            self.remember(brain);
        }
        Ok(self.localize(snapshot, offline))
    }

    /// A server snapshot as this person sees it: the project as this app
    /// keeps it, with the server's name, colour and actions, opened now
    /// (SESS-078); their viewport and expanded cards (SRV-164).
    fn localize(&self, mut snapshot: BoardSnapshot, offline: bool) -> BoardSnapshot {
        let now = now_ms();
        let (project, person) = self.nodus.inner.store.update(|state| {
            let person = state
                .server_boards
                .get(&self.home.project_id)
                .cloned()
                .unwrap_or_default();
            let project = state.project_mut(&self.home.project_id).ok().map(|local| {
                share(local, &snapshot.project);
                local.opened_at = Some(now);
                local.clone()
            });
            (project, person)
        });
        snapshot.project = project.unwrap_or_else(|| Project {
            repo_path: String::new(),
            server: Some(self.home.server_id.clone()),
            ..snapshot.project
        });
        snapshot.viewport = person.viewport;
        for card in &mut snapshot.cards {
            let expanded = !card.node.archived && person.expanded.contains(&card.node.session_id);
            card.node.expanded = expanded.then_some(true);
        }
        snapshot.offline = offline;
        snapshot
    }

    /// `board:save-nodes` for a server board: whether a card is expanded is
    /// kept here (SRV-164); the rest goes to the server.
    pub(crate) async fn save_nodes(&self, patches: &[NodePatch]) -> CommandResult<()> {
        let personal = patches.iter().any(|p| p.expanded.is_some() || p.archived == Some(true));
        if personal {
            self.nodus.inner.store.update(|state| {
                let person = state.server_boards.entry(self.project_id()).or_default();
                for patch in patches {
                    let id = &patch.session_id;
                    match patch.expanded {
                        Some(true) if patch.archived != Some(true) => {
                            if !person.expanded.contains(id) {
                                person.expanded.push(id.clone());
                            }
                        }
                        Some(_) => person.expanded.retain(|e| e != id),
                        // SESS-098: an archived card folds back to a card.
                        None if patch.archived == Some(true) => person.expanded.retain(|e| e != id),
                        None => {}
                    }
                }
            });
        }
        let shared: Vec<NodePatch> = patches
            .iter()
            .map(|patch| NodePatch {
                expanded: None,
                ..patch.clone()
            })
            .filter(|patch| {
                *patch
                    != NodePatch {
                        session_id: patch.session_id.clone(),
                        ..NodePatch::default()
                    }
            })
            .collect();
        if shared.is_empty() {
            return Ok(());
        }
        self.call(Request::BoardSaveNodes {
            project_id: self.project_id(),
            patches: shared,
        })
        .await
    }

    /// `board:save-viewport` for a server board: kept here, never sent (SRV-164).
    pub(crate) fn save_viewport(&self, viewport: Viewport) {
        self.nodus.inner.store.update(|state| {
            state.server_boards.entry(self.project_id()).or_default().viewport = Some(viewport);
        });
    }

    /// `session:create` on a server board. A card that starts expanded is
    /// expanded only here (SRV-164).
    pub(crate) async fn create_session(&self, opts: &CreateSessionOptions) -> CommandResult<SessionCard> {
        let expanded = opts.expanded == Some(true);
        let mut card: SessionCard = self
            .call(Request::SessionCreate {
                project_id: self.project_id(),
                opts: CreateSessionOptions {
                    expanded: None,
                    ..opts.clone()
                },
            })
            .await?;
        let id = card.node.session_id.clone();
        self.remember(&id);
        if expanded {
            self.nodus.inner.store.update(|state| {
                let person = state.server_boards.entry(self.project_id()).or_default();
                if !person.expanded.contains(&id) {
                    person.expanded.push(id.clone());
                }
            });
            card.node.expanded = Some(true);
        }
        Ok(card)
    }

    /// `projects:update` for a server project: its name and colour change
    /// on the server, for everyone; its group, focus and archive here.
    /// Gives the server's answer, for the caller to apply here too.
    pub(crate) async fn update_project(&self, patch: &ProjectPatch) -> CommandResult<Option<Project>> {
        if patch.name.is_none() && patch.color.is_none() {
            return Ok(None);
        }
        self.call(Request::ProjectsUpdate {
            id: self.project_id(),
            patch: ProjectPatch {
                name: patch.name.clone(),
                color: patch.color,
                ..ProjectPatch::default()
            },
        })
        .await
        .map(Some)
    }
}

/// Whether a call failed for want of the server, rather than by its answer.
fn unreachable(err: &CommandError) -> bool {
    [OFFLINE, DISCONNECTED, TIMED_OUT].contains(&err.message.as_str())
}
