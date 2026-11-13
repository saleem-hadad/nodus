//! `nodus.servers()`: the Nodus servers this app connects to, and their
//! projects linked into the grid (SRV-160, SRV-161). [`Remotes`] holds a
//! connection per server, passes each server's events on as the core's own,
//! and remembers what routing needs: which server each session is on, and
//! each server board's last snapshot.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use nodus_domain::focus::is_focusing;
use nodus_domain::{BoardSnapshot, Event, Project, ProjectStats, ServerInfo, ServerState};
use nodus_platform::secrets::{decode_base64, encode_base64};
use nodus_remote::{Remote, RemoteEvent, RemoteEvents, Request};
use nodus_store::ServerRecord;
use nodus_views::PageResponse;
use serde_json::Map;

use crate::nodus::{Inner, lock};
use crate::projects::now_ms;
use crate::{CommandError, CommandResult, Nodus};

/// How long the grid waits on a server for its projects' names, colours
/// and counts before it shows what it last had.
const LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// What adding a server says when the token can't be sealed (as PLAT-119
/// says it for the voice key).
const CANNOT_SAVE_TOKEN: &str = if cfg!(target_os = "macos") {
    "The Keychain isn’t available, so the token can’t be saved."
} else {
    "Encrypted storage isn’t available, so the token can’t be saved."
};

/// How long a view's frame waits for its page from the server.
const PAGE_WAIT: Duration = Duration::from_secs(3);

/// How long a file that couldn't be fetched waits before it's tried again.
const FETCH_RETRY: Duration = Duration::from_secs(10);

/// What a server view's frame shows when its page can't be had.
const PAGE_AWAY: &str = "The server can’t be reached.";

/// The headers a view's page keeps from its server (VIEW-020).
const PAGE_HEADERS: [&str; 4] = [
    "Content-Type",
    "Content-Security-Policy",
    "Cache-Control",
    "X-Content-Type-Options",
];

/// What a server whose saved token won't open says.
const TOKEN_UNREADABLE: &str = "The saved token can’t be read on this device. Add the server again.";

/// `nodus.servers()`.
pub struct ServersApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl ServersApi<'_> {
    /// `servers:list`: every server, in the order they were added, with how
    /// its connection is doing.
    pub async fn list(&self) -> Vec<ServerInfo> {
        let records = self.nodus.inner.store.read(|state| state.servers.clone());
        records
            .iter()
            .map(|record| self.nodus.inner.remotes.info(record))
            .collect()
    }

    /// `servers:add`: checks that `url` is a Nodus server with this app's
    /// protocol (SRV-027) that takes `token`, by connecting once, then saves
    /// it with the token sealed and keeps it connected. A server already
    /// added at that address gets the new token. Fails with the reason it
    /// can't be reached, `This server runs Nodus <v>. Update it in Settings
    /// → Servers.`, or the token turned down.
    pub async fn add(&self, url: &str, token: &str) -> CommandResult<ServerInfo> {
        const CHANNEL: &str = "servers:add";
        let url = server_url(url).ok_or_else(|| CommandError::new(CHANNEL, format!("Not a server address: {url}")))?;
        let token = token.trim();
        if token.is_empty() {
            return Err(CommandError::new(CHANNEL, "The server’s token is needed"));
        }
        let hello = self
            .nodus
            .on_core(nodus_remote::check(&url, token))
            .await
            .map_err(|err| CommandError::new(CHANNEL, err.to_string()))?;
        let sealed = self
            .nodus
            .seal(token)
            .ok_or_else(|| CommandError::new(CHANNEL, CANNOT_SAVE_TOKEN))?;
        let record = self.nodus.inner.store.update(|state| {
            if let Some(known) = state.servers.iter_mut().find(|s| s.url == url) {
                known.name.clone_from(&hello.name);
                known.token = sealed;
                return known.clone();
            }
            let record = ServerRecord {
                id: uuid::Uuid::new_v4().to_string(),
                url,
                name: hello.name.clone(),
                token: sealed,
                extra: Map::new(),
            };
            state.servers.push(record.clone());
            record
        });
        self.nodus.connect_server(&record, token);
        Ok(self.nodus.inner.remotes.info(&record))
    }

    /// `servers:remove`: disconnects and forgets a server on this device,
    /// with its projects' links (SRV-165, Disconnect). Nothing changes on
    /// the server.
    pub async fn remove(&self, id: &str) {
        self.nodus.inner.remotes.close(id);
        let unlinked = self.nodus.inner.store.update(|state| {
            state.servers.retain(|s| s.id != id);
            let unlinked: Vec<String> = state
                .projects
                .iter()
                .filter(|p| p.server.as_deref() == Some(id))
                .map(|p| p.id.clone())
                .collect();
            state.projects.retain(|p| p.server.as_deref() != Some(id));
            for project_id in &unlinked {
                state.server_boards.shift_remove(project_id);
            }
            unlinked
        });
        self.nodus.inner.remotes.forget_projects(&unlinked);
        self.nodus.emit(Event::ProjectsChanged);
    }

    /// `servers:projects`: the server's own projects, linked or not, as it
    /// lists them, for **Show server projects…** (SRV-161).
    pub async fn projects(&self, server_id: &str) -> CommandResult<Vec<Project>> {
        self.server_projects(server_id, "servers:projects").await
    }

    /// `servers:link`: puts one of the server's projects in this app's grid
    /// (SRV-161), with the server's name, colour and actions; its group,
    /// focus and archive are this app's own. Its events come from now on. A
    /// project already linked is brought back from the archive.
    pub async fn link(&self, server_id: &str, project_id: &str) -> CommandResult<Project> {
        const CHANNEL: &str = "servers:link";
        let theirs = self
            .server_projects(server_id, CHANNEL)
            .await?
            .into_iter()
            .find(|p| p.id == project_id)
            .ok_or_else(|| CommandError::new(CHANNEL, format!("Unknown project {project_id}")))?;
        let now = now_ms();
        let project = self.nodus.inner.store.update(|state| {
            let focusing = is_focusing(
                &state.projects,
                state.settings.focus_feature && state.settings.focus_mode,
            );
            if let Some(known) = state.projects.iter_mut().find(|p| p.id == project_id) {
                if known.server.as_deref() != Some(server_id) {
                    return Err(CommandError::new(CHANNEL, "That project is already in the grid"));
                }
                share(known, &theirs);
                known.archived = None;
                return Ok(known.clone());
            }
            let project = Project {
                id: theirs.id.clone(),
                name: theirs.name.clone(),
                repo_path: String::new(),
                color: theirs.color,
                created_at: now,
                opened_at: None,
                group_id: None,
                actions: theirs.actions.clone(),
                focused: focusing.then_some(true),
                archived: None,
                server: Some(server_id.to_owned()),
                extra: Map::new(),
            };
            state.projects.push(project.clone());
            Ok(project)
        })?;
        self.nodus.rewatch(server_id);
        self.nodus.emit(Event::ProjectsChanged);
        Ok(project)
    }

    /// `servers:unlink`: takes a server project out of this app's grid, with
    /// what was this person's own of its board. It stays on the server.
    /// `Not a server project: <id>`.
    pub async fn unlink(&self, project_id: &str) -> CommandResult<()> {
        const CHANNEL: &str = "servers:unlink";
        let server_id = self.nodus.inner.store.update(|state| {
            let server_id = state
                .project_server(project_id)
                .map(str::to_owned)
                .ok_or_else(|| CommandError::new(CHANNEL, format!("Not a server project: {project_id}")))?;
            state.projects.retain(|p| p.id != project_id);
            state.server_boards.shift_remove(project_id);
            Ok::<_, CommandError>(server_id)
        })?;
        self.nodus.inner.remotes.forget_projects(&[project_id.to_owned()]);
        self.nodus.rewatch(&server_id);
        self.nodus.emit(Event::ProjectsChanged);
        Ok(())
    }

    /// `servers:status`: one server, with how its connection is doing; none
    /// for an id that isn't one. Each change also arrives as
    /// `Event::ServerStatus`.
    pub fn status(&self, server_id: &str) -> Option<ServerInfo> {
        let record = self.nodus.inner.store.read(|state| state.server(server_id).cloned())?;
        Some(self.nodus.inner.remotes.info(&record))
    }

    async fn server_projects(&self, server_id: &str, channel: &'static str) -> CommandResult<Vec<Project>> {
        let remote = self
            .nodus
            .inner
            .remotes
            .get(server_id)
            .ok_or_else(|| CommandError::new(channel, format!("Unknown server {server_id}")))?;
        self.nodus
            .on_core(async move { remote.call_as(&Request::ProjectsList {}).await })
            .await
            .map_err(|err| CommandError::new(channel, err.message))
    }
}

/// A server's address as it's kept: `http://host[:port]`, trimmed, without
/// a trailing slash. None for anything else (no TLS in v1, SRV-021).
fn server_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url.strip_prefix("http://").unwrap_or(url);
    (!rest.is_empty() && !rest.contains("://") && !rest.contains('/') && !rest.contains(char::is_whitespace))
        .then(|| format!("http://{rest}"))
}

/// The server's name, colour and actions on a linked project. True when
/// something changed.
pub(crate) fn share(local: &mut Project, theirs: &Project) -> bool {
    let changed = local.name != theirs.name || local.color != theirs.color || local.actions != theirs.actions;
    if changed {
        local.name.clone_from(&theirs.name);
        local.color = theirs.color;
        local.actions.clone_from(&theirs.actions);
    }
    changed
}

/// Where a server session is: its server and its project (SRV-160).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionHome {
    pub(crate) server_id: String,
    pub(crate) project_id: String,
}

/// The connections to servers, and what routing remembers of them.
pub(crate) struct Remotes {
    remotes: Mutex<HashMap<String, Arc<Remote>>>,
    /// Servers that couldn't be started, and why.
    failed: Mutex<HashMap<String, String>>,
    /// Each server session's home, from the server's boards and answers.
    sessions: Mutex<HashMap<String, SessionHome>>,
    /// Each server board's last snapshot, as the server gave it (SRV-028).
    boards: Mutex<HashMap<String, BoardSnapshot>>,
    /// Where those snapshots are kept between runs.
    cache: PathBuf,
    /// Where servers' files are kept once fetched: `server-files/<server>/<id>`.
    files: PathBuf,
    /// Files being fetched, or that failed a moment ago, as `<server>/<id>`.
    fetching: Mutex<HashSet<String>>,
    /// Each server view's last page, for while its server can't be reached.
    pages: Mutex<HashMap<String, PageResponse>>,
}

impl Remotes {
    pub(crate) fn new(user_data: &std::path::Path) -> Self {
        Self {
            remotes: Mutex::default(),
            failed: Mutex::default(),
            sessions: Mutex::default(),
            boards: Mutex::default(),
            cache: user_data.join("server-boards"),
            files: user_data.join("server-files"),
            fetching: Mutex::default(),
            pages: Mutex::default(),
        }
    }

    pub(crate) fn get(&self, server_id: &str) -> Option<Arc<Remote>> {
        lock(&self.remotes).get(server_id).cloned()
    }

    /// The connected servers.
    fn connected(&self) -> Vec<(String, Arc<Remote>)> {
        lock(&self.remotes)
            .iter()
            .filter(|(_, remote)| remote.is_connected())
            .map(|(id, remote)| (id.clone(), Arc::clone(remote)))
            .collect()
    }

    fn insert(&self, server_id: &str, remote: Arc<Remote>) {
        lock(&self.failed).remove(server_id);
        if let Some(old) = lock(&self.remotes).insert(server_id.to_owned(), remote) {
            old.close();
        }
    }

    fn fail(&self, server_id: &str, reason: &str) {
        lock(&self.failed).insert(server_id.to_owned(), reason.to_owned());
    }

    pub(crate) fn close(&self, server_id: &str) {
        lock(&self.failed).remove(server_id);
        if let Some(remote) = lock(&self.remotes).remove(server_id) {
            remote.close();
        }
    }

    pub(crate) fn close_all(&self) {
        for (_, remote) in lock(&self.remotes).drain() {
            remote.close();
        }
    }

    /// A server as the app shows it.
    pub(crate) fn info(&self, record: &ServerRecord) -> ServerInfo {
        let (state, version, error) = match self.get(&record.id) {
            Some(remote) => {
                let status = remote.status();
                (status.state, status.hello.map(|h| h.version), status.error)
            }
            None => match lock(&self.failed).get(&record.id) {
                Some(reason) => (ServerState::Offline, None, Some(reason.clone())),
                None => (ServerState::Connecting, None, None),
            },
        };
        ServerInfo {
            id: record.id.clone(),
            url: record.url.clone(),
            name: record.name.clone(),
            state,
            version,
            error,
        }
    }

    pub(crate) fn remember(&self, session_id: &str, home: &SessionHome) {
        lock(&self.sessions).insert(session_id.to_owned(), home.clone());
    }

    pub(crate) fn session_home(&self, session_id: &str) -> Option<SessionHome> {
        lock(&self.sessions).get(session_id).cloned()
    }

    /// Keeps a server board's snapshot, here and on disk, for while the
    /// server can't be reached (SRV-028).
    pub(crate) fn keep_board(&self, project_id: &str, snapshot: &BoardSnapshot, runtime: &tokio::runtime::Handle) {
        lock(&self.boards).insert(project_id.to_owned(), snapshot.clone());
        let (Some(file), Ok(text)) = (self.cache_file(project_id), nodus_domain::json::to_string(snapshot)) else {
            return;
        };
        let dir = self.cache.clone();
        runtime.spawn(async move {
            let written = async {
                tokio::fs::create_dir_all(&dir).await?;
                let partial = file.with_extension("json.tmp");
                tokio::fs::write(&partial, text).await?;
                tokio::fs::rename(&partial, &file).await
            };
            if let Err(err) = written.await {
                tracing::warn!("[remote] couldn't keep {}: {err}", file.display());
            }
        });
    }

    /// A server board's last snapshot, from this run or an earlier one.
    pub(crate) fn last_board(&self, project_id: &str) -> Option<BoardSnapshot> {
        if let Some(snapshot) = lock(&self.boards).get(project_id) {
            return Some(snapshot.clone());
        }
        let text = std::fs::read_to_string(self.cache_file(project_id)?).ok()?;
        let snapshot: BoardSnapshot = nodus_domain::json::from_str(&text).ok()?;
        lock(&self.boards).insert(project_id.to_owned(), snapshot.clone());
        Some(snapshot)
    }

    /// The project of the server board a view is on, from the snapshots
    /// loaded this run.
    fn view_project(&self, view_id: &str) -> Option<String> {
        lock(&self.boards)
            .iter()
            .find(|(_, board)| board.views.iter().any(|v| v.id == view_id))
            .map(|(project_id, _)| project_id.clone())
    }

    /// Where a server's file is kept here; none for an id that isn't safe
    /// as a file name.
    fn file(&self, server_id: &str, id: &str) -> Option<PathBuf> {
        let safe = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && !s.starts_with('.')
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        (safe(server_id) && safe(id)).then(|| self.files.join(server_id).join(id))
    }

    /// Forgets unlinked projects' snapshots and sessions.
    pub(crate) fn forget_projects(&self, project_ids: &[String]) {
        lock(&self.boards).retain(|id, _| !project_ids.contains(id));
        lock(&self.sessions).retain(|_, home| !project_ids.contains(&home.project_id));
        for project_id in project_ids {
            if let Some(file) = self.cache_file(project_id) {
                let _ = std::fs::remove_file(file);
            }
        }
    }

    /// The file a server board's snapshot is kept in; none for an id that
    /// isn't safe as a file name.
    fn cache_file(&self, project_id: &str) -> Option<PathBuf> {
        let safe = !project_id.is_empty()
            && project_id.len() <= 64
            && project_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        safe.then(|| self.cache.join(format!("{project_id}.json")))
    }
}

impl Nodus {
    /// `nodus.servers()`: the servers this app connects to (SRV-160…165).
    pub fn servers(&self) -> ServersApi<'_> {
        ServersApi { nodus: self }
    }

    /// Connects to every saved server not connected yet, at launch.
    pub(crate) async fn connect_servers(&self) {
        let records = self.inner.store.read(|state| state.servers.clone());
        for record in records {
            if self.inner.remotes.get(&record.id).is_some() {
                continue;
            }
            match self.open_token(&record.token).await {
                Some(token) => self.connect_server(&record, &token),
                None => self.inner.remotes.fail(&record.id, TOKEN_UNREADABLE),
            }
        }
    }

    /// Starts (or starts again) the connection to a server, watching its
    /// linked projects, and passes its events on as the core's own.
    fn connect_server(&self, record: &ServerRecord, token: &str) {
        let (remote, events) = Remote::start(&self.inner.runtime, &record.url, token);
        remote.watch(self.linked_projects(&record.id));
        self.inner.remotes.insert(&record.id, Arc::new(remote));
        self.inner
            .runtime
            .spawn(pass_on(Arc::downgrade(&self.inner), record.id.clone(), events));
    }

    /// The server's linked projects, by id.
    fn linked_projects(&self, server_id: &str) -> Vec<String> {
        self.inner.store.read(|state| {
            state
                .projects
                .iter()
                .filter(|p| p.server.as_deref() == Some(server_id))
                .map(|p| p.id.clone())
                .collect()
        })
    }

    /// The server watches what's linked now (SRV-026).
    fn rewatch(&self, server_id: &str) {
        if let Some(remote) = self.inner.remotes.get(server_id) {
            remote.watch(self.linked_projects(server_id));
        }
    }

    /// A token sealed as the voice key is, in base64; none when there's no
    /// encryption to seal it with.
    fn seal(&self, token: &str) -> Option<String> {
        let secrets = &self.inner.secrets;
        if !secrets.is_encryption_available() {
            return None;
        }
        secrets.encrypt_string(token).ok().map(|sealed| encode_base64(&sealed))
    }

    /// A sealed token, opened off the core's threads (the Keychain may be
    /// slow the first time).
    async fn open_token(&self, sealed: &str) -> Option<String> {
        let secrets = Arc::clone(&self.inner.secrets);
        let sealed = decode_base64(sealed);
        let opened = self
            .inner
            .runtime
            .spawn_blocking(move || secrets.decrypt_string(&sealed))
            .await
            .ok()?;
        match opened {
            Ok(token) if !token.is_empty() => Some(token),
            Ok(_) => None,
            Err(err) => {
                tracing::error!("[remote] could not decrypt a server's token {err}");
                None
            }
        }
    }

    /// A server board's picture kept here (SRV-163): its file once fetched;
    /// until then none, and one fetch starts, after which the board changes.
    pub(crate) fn server_file(&self, server_id: &str, project_id: &str, id: &str) -> Option<PathBuf> {
        let remotes = &self.inner.remotes;
        let file = remotes.file(server_id, id)?;
        if file.exists() {
            return Some(file);
        }
        let key = format!("{server_id}/{id}");
        let remote = remotes.get(server_id).filter(|r| r.is_connected())?;
        if !lock(&remotes.fetching).insert(key.clone()) {
            return None;
        }
        let core = Arc::downgrade(&self.inner);
        let (id, project_id) = (id.to_owned(), project_id.to_owned());
        self.inner.runtime.spawn(async move {
            let fetched = remote.download(&id).await;
            let Some(core) = core.upgrade() else { return };
            let kept = match fetched {
                Ok(body) if body.status == 200 => write_file(&file, &body.body).await.is_ok(),
                _ => false,
            };
            if kept {
                lock(&core.remotes.fetching).remove(&key);
                let _ = core.events.send(Event::BoardChanged { project_id });
            } else {
                // Not again at once: the next draw would ask again.
                tokio::time::sleep(FETCH_RETRY).await;
                lock(&core.remotes.fetching).remove(&key);
            }
        });
        None
    }

    /// Keeps a file just uploaded to a server, so it isn't fetched back.
    pub(crate) async fn keep_server_file(&self, server_id: &str, id: &str, bytes: Vec<u8>) {
        if let Some(file) = self.inner.remotes.file(server_id, id)
            && let Err(err) = write_file(&file, &bytes).await
        {
            tracing::warn!("[remote] couldn't keep {}: {err}", file.display());
        }
    }

    /// The server a view is on, for a view on a server board this app has
    /// loaded; none for a local view.
    pub(crate) fn view_server(&self, view_id: &str) -> Option<String> {
        let project_id = self.inner.remotes.view_project(view_id)?;
        self.inner
            .store
            .read(|state| state.project_server(&project_id).map(str::to_owned))
    }

    /// A server view's page, from its server and nowhere else (SRV-163):
    /// asked for with `GET /api/files/<viewId>`, waiting at most
    /// [`PAGE_WAIT`] (the frame's request waits on it). While the server
    /// can't be reached, the last page it gave, else a 503 that says so.
    pub(crate) fn server_page(&self, server_id: &str, view_id: &str) -> PageResponse {
        let (sent, answer) = std::sync::mpsc::channel();
        let remote = self.inner.remotes.get(server_id);
        let id = view_id.to_owned();
        self.inner.runtime.spawn(async move {
            let page = match remote {
                Some(remote) => remote.download(&id).await.ok(),
                None => None,
            };
            let _ = sent.send(page);
        });
        let fetched = answer.recv_timeout(PAGE_WAIT).ok().flatten();
        let pages = &self.inner.remotes.pages;
        match fetched.filter(|body| body.status == 200) {
            Some(body) => {
                let page = PageResponse {
                    status: body.status,
                    headers: PAGE_HEADERS
                        .iter()
                        .filter_map(|&name| body.header(name).map(|value| (name, value.to_owned())))
                        .collect(),
                    body: String::from_utf8_lossy(&body.body).into_owned(),
                };
                lock(pages).insert(view_id.to_owned(), page.clone());
                page
            }
            None => lock(pages).get(view_id).cloned().unwrap_or_else(|| PageResponse {
                status: 503,
                headers: vec![("Content-Type", "text/plain;charset=UTF-8".to_owned())],
                body: PAGE_AWAY.to_owned(),
            }),
        }
    }

    /// Linked projects get their server's name, colour and actions, from
    /// every connected server that has any, for `projects:list`.
    pub(crate) async fn refresh_linked_projects(&self) {
        for (server_id, remote) in self.inner.remotes.connected() {
            let linked = self.linked_projects(&server_id);
            if linked.is_empty() {
                continue;
            }
            let answer = remote.call_within(&Request::ProjectsList {}, LIST_TIMEOUT).await;
            let theirs: Vec<Project> = match answer.map(serde_json::from_value) {
                Ok(Ok(theirs)) => theirs,
                _ => continue,
            };
            let stale = self.inner.store.read(|state| {
                state.projects.iter().any(|local| {
                    local.server.as_deref() == Some(server_id.as_str())
                        && theirs.iter().any(|t| t.id == local.id && share(&mut local.clone(), t))
                })
            });
            if stale {
                self.inner.store.update(|state| {
                    for local in &mut state.projects {
                        if local.server.as_deref() != Some(server_id.as_str()) {
                            continue;
                        }
                        if let Some(t) = theirs.iter().find(|t| t.id == local.id) {
                            share(local, t);
                        }
                    }
                });
            }
        }
    }

    /// Every connected server's counts for its linked projects, for
    /// `projects:stats` (the title bar's running list, SRV-160).
    pub(crate) async fn linked_project_stats(&self) -> HashMap<String, ProjectStats> {
        let mut stats = HashMap::new();
        for (server_id, remote) in self.inner.remotes.connected() {
            let linked = self.linked_projects(&server_id);
            if linked.is_empty() {
                continue;
            }
            let answer = remote.call_within(&Request::ProjectsStats {}, LIST_TIMEOUT).await;
            if let Ok(Ok(theirs)) = answer.map(serde_json::from_value::<HashMap<String, ProjectStats>>) {
                stats.extend(theirs.into_iter().filter(|(id, _)| linked.contains(id)));
            }
        }
        stats
    }
}

/// Writes a file whole, through a temporary name, making its folder first.
async fn write_file(file: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let partial = file.with_extension("part");
    tokio::fs::write(&partial, bytes).await?;
    tokio::fs::rename(&partial, file).await
}

/// Passes a server's events on to the core's subscribers, as if the core had
/// sent them, and tells them how the connection is doing. When it comes up
/// or goes, the boards of its projects reload (SRV-028): from the server,
/// or from their last snapshot, read-only.
async fn pass_on(core: Weak<Inner>, server_id: String, mut events: RemoteEvents) {
    let mut told = None;
    while let Some(event) = events.next().await {
        let Some(core) = core.upgrade() else {
            return;
        };
        match event {
            RemoteEvent::Event(event) => {
                let _ = core.events.send(event);
            }
            RemoteEvent::Status(status) => {
                if told == Some(status.state) {
                    continue;
                }
                let was_up = told == Some(ServerState::Connected);
                told = Some(status.state);
                let _ = core.events.send(Event::ServerStatus {
                    server_id: server_id.clone(),
                    state: status.state,
                });
                if status.state == ServerState::Connected || was_up {
                    let linked: Vec<String> = core.store.read(|state| {
                        state
                            .projects
                            .iter()
                            .filter(|p| p.server.as_deref() == Some(server_id.as_str()))
                            .map(|p| p.id.clone())
                            .collect()
                    });
                    for project_id in linked {
                        let _ = core.events.send(Event::BoardChanged { project_id });
                    }
                    let _ = core.events.send(Event::ProjectsChanged);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{FolderColor, NodePatch, Viewport};
    use nodus_remote::{NOT_AVAILABLE, OFFLINE};
    use serde_json::json;

    /// A server project linked into the grid, on a server that was never
    /// connected, with a snapshot of its board kept from an earlier run.
    fn linked(dir: &tempfile::TempDir, nodus: &Nodus) {
        nodus.inner.store.update(|state| {
            state.servers.push(ServerRecord {
                id: "srv".into(),
                url: "http://127.0.0.1:9".into(),
                name: "box".into(),
                token: String::new(),
                extra: Map::new(),
            });
            state.projects.push(Project {
                id: "p1".into(),
                name: "Shop".into(),
                repo_path: String::new(),
                color: FolderColor::Blue,
                created_at: 1.0,
                opened_at: None,
                group_id: None,
                actions: None,
                focused: None,
                archived: None,
                server: Some("srv".into()),
                extra: Map::new(),
            });
        });
        let kept = json!({
            "project": { "id": "p1", "name": "Shop", "repoPath": "/srv/shop", "color": "green", "createdAt": 1 },
            "cards": [{
                "sessionId": "c1", "x": 0, "y": 0, "archived": false, "createdAt": 1, "expanded": true,
                "status": "done", "live": false
            }],
            "archive": { "x": 0, "y": 0, "w": 100, "h": 100 },
            "sections": [], "notes": [], "views": [], "ink": [],
            "viewport": { "x": 9, "y": 9, "zoom": 2 }
        });
        let cache = dir.path().join("userData/server-boards");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("p1.json"), kept.to_string()).unwrap();
    }

    /// SRV-028, SRV-164
    #[tokio::test]
    async fn opens_a_server_board_read_only_while_away_with_this_persons_own_state() {
        let (dir, nodus) = crate::tests::nodus();
        linked(&dir, &nodus);

        let board = nodus.board().load("p1").await.unwrap();
        assert!(board.offline);
        // The project as this app keeps it, with the server's name and colour.
        assert_eq!(board.project.server.as_deref(), Some("srv"));
        assert_eq!(
            (board.project.repo_path.as_str(), board.project.color),
            ("", FolderColor::Green)
        );
        assert!(board.project.opened_at.is_some());
        // Neither the server's viewport nor its expanded cards are this person's.
        assert_eq!(board.viewport, None);
        assert_eq!(board.cards[0].node.expanded, None);

        // What's this person's own is kept here, server or no server.
        let viewport = Viewport {
            x: 1.0,
            y: 2.5,
            zoom: 0.5,
        };
        nodus.board().save_viewport("p1", viewport).await;
        let expand = NodePatch {
            session_id: "c1".into(),
            expanded: Some(true),
            ..NodePatch::default()
        };
        nodus.board().save_nodes("p1", &[expand]).await.unwrap();
        let board = nodus.board().load("p1").await.unwrap();
        assert_eq!(board.viewport, Some(viewport));
        assert_eq!(board.cards[0].node.expanded, Some(true));
        let kept = nodus.inner.store.read(|state| state.server_boards["p1"].clone());
        assert_eq!((kept.viewport, kept.expanded), (Some(viewport), vec!["c1".to_owned()]));

        // The rest is the server's, so it waits for it.
        let moved = NodePatch {
            session_id: "c1".into(),
            x: Some(5.0),
            ..NodePatch::default()
        };
        let err = nodus.board().save_nodes("p1", &[moved]).await.unwrap_err();
        assert_eq!((err.channel, err.message.as_str()), ("board:save-nodes", OFFLINE));
        let err = nodus.git().status("p1").await.unwrap_err();
        assert_eq!(err.message, OFFLINE);
        // Its sessions are the server's: typing goes nowhere locally.
        assert_eq!(nodus.board().session_project("c1").as_deref(), Some("p1"));
        nodus.sessions().write("c1", "x");
        // Archiving a card folds it back here too (SESS-098).
        let archive = NodePatch {
            session_id: "c1".into(),
            archived: Some(true),
            ..NodePatch::default()
        };
        assert!(nodus.board().save_nodes("p1", &[archive]).await.is_err());
        assert!(
            nodus
                .inner
                .store
                .read(|state| state.server_boards["p1"].expanded.is_empty())
        );
    }

    #[tokio::test]
    async fn a_server_project_leaves_only_this_grid() {
        let (dir, nodus) = crate::tests::nodus();
        linked(&dir, &nodus);
        let err = nodus.projects().reveal_in_finder("p1").await.unwrap_err();
        assert_eq!(err.message, NOT_AVAILABLE);
        assert_eq!(nodus.projects().stats().await["p1"], ProjectStats::default());
        let servers = nodus.servers().list().await;
        assert_eq!((servers.len(), servers[0].state), (1, ServerState::Connecting));
        nodus
            .board()
            .save_viewport(
                "p1",
                Viewport {
                    x: 0.0,
                    y: 0.0,
                    zoom: 1.0,
                },
            )
            .await;

        nodus.projects().remove("p1").await;
        assert!(nodus.projects().list().await.is_empty());
        assert!(nodus.inner.store.read(|state| state.server_boards.is_empty()));
        assert!(!dir.path().join("userData/server-boards/p1.json").exists());
        let err = nodus.servers().unlink("p1").await.unwrap_err();
        assert_eq!(err.message, "Not a server project: p1");

        nodus.servers().remove("srv").await;
        assert!(nodus.servers().list().await.is_empty());
        assert_eq!(nodus.servers().status("srv"), None);
    }

    #[test]
    fn keeps_a_server_address_as_plain_http() {
        assert_eq!(server_url(" nodus:7420/ ").as_deref(), Some("http://nodus:7420"));
        assert_eq!(server_url("http://100.64.0.1").as_deref(), Some("http://100.64.0.1"));
        for wrong in ["", "https://nodus", "http://nodus/api", "no dus"] {
            assert_eq!(server_url(wrong), None, "{wrong:?}");
        }
    }

    #[test]
    fn keeps_snapshots_only_under_safe_names() {
        let remotes = Remotes::new(std::path::Path::new("/data"));
        assert!(remotes.cache_file("0b9c-41aa").is_some());
        for wrong in ["", "../state", "a/b", "x.json"] {
            assert!(remotes.cache_file(wrong).is_none(), "{wrong:?}");
        }
    }
}
