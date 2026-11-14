//! What the tools do, on the store and the board: ipc.ts's
//! `configureBoardTools({…})` handlers (TOOL-026 to TOOL-040).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nodus_board::archive::{archive_node, drop_node, is_empty};
use nodus_board::cards::session_list_items;
use nodus_board::notes::{self, AwayAuthor, NotePatch};
use nodus_board::{Board, BoardSessions, Clock, Effects, to_card};
use nodus_domain::{CreatedBy, Event, EventSink, Project, SessionCard, SessionKind, SessionStatus, js};
use nodus_store::{BoardState, Store};
use serde_json::{Value, json};

use crate::drawing::{self, Picture};
use crate::look::{self, Look};
use crate::server::{BoardToolHandlers, BoxFuture, ToolError};

/// The biggest picture a session can put up, and that look_at_board reads.
const MAX_PICTURE_BYTES: u64 = 25 << 20;

/// What `create_project` answers on a server (SRV-146).
pub const SERVER_PROJECTS: &str =
    "Projects on a Nodus server come from GitHub: ask a member to add the repository with Add from GitHub";

/// Adds a project for a folder (`addProject`, 01 and 08), which nodus-core
/// has: the folder's real path must be a folder (`Folder not found: <folder>`,
/// `Not a folder: <real>`); a folder that is already a project gives that
/// project back, un-archived; a new one gets an id, its name (or the folder's)
/// and the least used colour. `true` when the project is new.
pub trait ProjectAdder: Send + Sync {
    fn add_project(&self, folder: &Path, name: Option<&str>) -> Result<(Project, bool), String>;
}

/// The view tools, which nodus-core passes on to nodus-views' `Views`
/// (`listViewsForTool`, `getViewForTool`, `createViewFromTool`,
/// `updateViewFromTool`). Errors are the messages Claude reads.
pub trait ViewTools: Send + Sync {
    fn list_views(&self, project_id: &str, self_id: &str) -> Result<Value, String>;
    fn get_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<Value, String>;
    fn create_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>>;
    fn update_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>>;
}

/// The board tools' handlers on the app's store and boards.
pub struct StoreHandlers {
    store: Arc<Store>,
    board: Arc<Board>,
    sessions: Arc<dyn BoardSessions>,
    events: Arc<dyn EventSink>,
    projects: Arc<dyn ProjectAdder>,
    views: Arc<dyn ViewTools>,
    clock: Clock,
    home: Option<PathBuf>,
    on_server: bool,
    pictures: Vec<PathBuf>,
}

/// What the handlers work with. `board` must be on the same `store`.
pub struct StoreHandlersConfig {
    pub store: Arc<Store>,
    pub board: Arc<Board>,
    pub sessions: Arc<dyn BoardSessions>,
    pub events: Arc<dyn EventSink>,
    pub projects: Arc<dyn ProjectAdder>,
    pub views: Arc<dyn ViewTools>,
    /// `Date.now()`, for when a session is archived.
    pub clock: Clock,
    /// `os.homedir()`, for a project path that starts with `~`.
    pub home: Option<PathBuf>,
    /// On a Nodus server, where these handlers run as the server's own user
    /// and every request comes from a session that may be steered by what it
    /// read (SRV-146).
    pub on_server: bool,
    /// The folders a board's pictures are in, by name: the first is where
    /// new ones go (`board-images`), the rest are only read from (a
    /// server's uploads, SRV-163).
    pub pictures: Vec<PathBuf>,
}

impl StoreHandlers {
    pub fn new(config: StoreHandlersConfig) -> Self {
        Self {
            store: config.store,
            board: config.board,
            sessions: config.sessions,
            events: config.events,
            projects: config.projects,
            views: config.views,
            clock: config.clock,
            home: config.home,
            on_server: config.on_server,
            pictures: config.pictures,
        }
    }

    /// A picture's bytes, by the name a picture shape holds: a plain file
    /// in one of the pictures folders, never a link to one elsewhere.
    fn picture(&self, name: &str) -> Option<Vec<u8>> {
        if !nodus_board::shapes::image_name_ok(name) {
            return None;
        }
        self.pictures.iter().find_map(|dir| {
            let path = dir.join(name);
            let meta = std::fs::symlink_metadata(&path).ok()?;
            (meta.is_file() && meta.len() <= MAX_PICTURE_BYTES).then(|| std::fs::read(&path).ok())?
        })
    }

    /// TOOL-112: the picture files draw_on_board's shapes name, each checked
    /// and copied into the board's pictures under a new name, by the
    /// shape's place in the list.
    fn copy_pictures(&self, args: &Value) -> Result<HashMap<usize, Picture>, ToolError> {
        let mut copied = HashMap::new();
        let Some(Value::Array(shapes)) = args.get("shapes") else {
            return Ok(copied);
        };
        let wanted: Vec<(usize, &Value)> = shapes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.get("kind").and_then(Value::as_str) == Some("image"))
            .collect();
        if wanted.is_empty() {
            return Ok(copied);
        }
        // A session on a server runs as another user, in a folder of its
        // own, so a path it names isn't one this side should read.
        if self.on_server {
            return Err("Pictures can’t be put up from a file on a Nodus server’s board yet".into());
        }
        let Some(dir) = self.pictures.first() else {
            return Err("This board has nowhere to keep pictures".into());
        };
        let mut written: Vec<PathBuf> = Vec::new();
        let result = (|| {
            for (i, shape) in wanted {
                let at = format!("shapes[{i}]");
                let Some(path) = trimmed(shape.get("path")) else {
                    return Err(format!("{at}: a picture needs path, its file’s absolute path"));
                };
                let file = expand_home(path, self.home.as_deref());
                if !is_absolute(&file) {
                    return Err(format!("{at}: path must be absolute"));
                }
                let meta = std::fs::metadata(&file).map_err(|_| format!("{at}: there’s no file at “{file}”"))?;
                if !meta.is_file() {
                    return Err(format!("{at}: “{file}” isn’t a file"));
                }
                if meta.len() > MAX_PICTURE_BYTES {
                    return Err(format!(
                        "{at}: “{file}” is {} MB; a picture can be at most {} MB",
                        meta.len().div_ceil(1 << 20),
                        MAX_PICTURE_BYTES >> 20
                    ));
                }
                let bytes = std::fs::read(&file).map_err(|err| format!("{at}: {}", fs_message(&err, "open", &file)))?;
                let Some(ext) = nodus_board::pictures::picture_kind(&bytes) else {
                    return Err(format!("{at}: “{file}” isn’t a PNG, JPEG, GIF or WebP picture"));
                };
                let size = nodus_board::pictures::picture_size(&bytes);
                let name = format!("{}.{ext}", nodus_board::shapes::new_id());
                std::fs::create_dir_all(dir)
                    .and_then(|()| std::fs::write(dir.join(&name), &bytes))
                    .map_err(|err| {
                        format!(
                            "{at}: {}",
                            fs_message(&err, "open", &dir.join(&name).display().to_string())
                        )
                    })?;
                written.push(dir.join(&name));
                copied.insert(i, Picture { name, size });
            }
            Ok(())
        })();
        if let Err(err) = result {
            self.drop_pictures(&written);
            return Err(err.into());
        }
        Ok(copied)
    }

    /// Pictures copied in for a drawing that didn't go up.
    fn drop_pictures(&self, files: &[PathBuf]) {
        for file in files {
            let _ = std::fs::remove_file(file);
        }
    }

    fn project(&self, project_id: &str) -> Result<Project, ToolError> {
        self.store
            .read(|state| state.project(project_id).cloned())
            .map_err(ToolError::of)
    }

    fn board_changed(&self, project_id: &str) {
        self.events.emit(Event::BoardChanged {
            project_id: project_id.to_owned(),
        });
    }
}

/// `typeof s === 'string' && s` after `trim()`.
fn trimmed(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).map(js::trim).filter(|s| !s.is_empty())
}

/// The id of the project `reference` names: its id, or else its name, any
/// case, archived projects included, when exactly one has it (TOOL-028).
pub fn find_project(projects: &[Project], reference: Option<&Value>) -> Result<String, ToolError> {
    let Some(asked) = trimmed(reference) else {
        return Err("project must be a project’s id or name, from list_projects".into());
    };
    if let Some(project) = projects.iter().find(|p| p.id == asked) {
        return Ok(project.id.clone());
    }
    let lower = asked.to_lowercase();
    let named: Vec<&Project> = projects.iter().filter(|p| p.name.to_lowercase() == lower).collect();
    match named.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!("There’s no project called “{asked}”; list_projects lists them").into()),
        _ => Err(format!("More than one project is called “{asked}”; use its id from list_projects").into()),
    }
}

/// `folder.replace(/^~(?=$|[\\/])/, os.homedir())` (TOOL-034).
pub fn expand_home(folder: &str, home: Option<&Path>) -> String {
    let Some(rest) = folder.strip_prefix('~') else {
        return folder.to_owned();
    };
    match home {
        Some(home) if rest.is_empty() || rest.starts_with(['/', '\\']) => format!("{}{rest}", home.display()),
        _ => folder.to_owned(),
    }
}

/// Node's `path.isAbsolute` on this platform: a leading separator, or on
/// Windows a drive letter and one.
pub fn is_absolute(path: &str) -> bool {
    if cfg!(windows) {
        let bytes = path.as_bytes();
        let sep = |b: u8| b == b'/' || b == b'\\';
        bytes.first().is_some_and(|&b| sep(b))
            || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && sep(bytes[2]))
    } else {
        path.starts_with('/')
    }
}

/// `EACCES: permission denied, mkdir '<path>'`, as Node words it.
fn fs_message(err: &io::Error, syscall: &str, path: &str) -> String {
    let named = match err.kind() {
        io::ErrorKind::NotFound => Some(("ENOENT", "no such file or directory")),
        io::ErrorKind::PermissionDenied => Some(("EACCES", "permission denied")),
        io::ErrorKind::AlreadyExists => Some(("EEXIST", "file already exists")),
        io::ErrorKind::NotADirectory => Some(("ENOTDIR", "not a directory")),
        io::ErrorKind::ReadOnlyFilesystem => Some(("EROFS", "read-only file system")),
        _ => None,
    };
    match named {
        Some((code, description)) => format!("{code}: {description}, {syscall} '{path}'"),
        None => err.to_string(),
    }
}

impl BoardToolHandlers for StoreHandlers {
    fn project_for_session(&self, session_id: &str) -> Option<String> {
        self.store
            .read(|state| state.project_id_for_session(session_id).map(str::to_owned))
    }

    fn is_run(&self, session_id: &str) -> bool {
        self.store.read(|state| {
            state
                .project_id_for_session(session_id)
                .and_then(|p| state.board(p)?.nodes.get(session_id))
                .is_some_and(|n| n.run_of.as_deref().is_some_and(|r| !r.is_empty()))
        })
    }

    /// TOOL-033. Server projects aren't this app's to write on: their
    /// boards are on their servers (SRV-160).
    fn list_projects(&self, own: &str) -> Result<Value, ToolError> {
        Ok(self.store.read(|state| {
            Value::Array(
                state
                    .projects
                    .iter()
                    .filter(|p| p.server.is_none())
                    .map(|p| {
                        let mut item = json!({ "id": p.id, "name": p.name, "folder": p.repo_path });
                        if p.archived == Some(true) {
                            item["archived"] = Value::Bool(true);
                        }
                        if p.id == own {
                            item["thisSessionsBoard"] = Value::Bool(true);
                        }
                        item
                    })
                    .collect(),
            )
        }))
    }

    /// TOOL-034
    fn create_project(&self, folder: Option<&Value>, name: Option<&Value>) -> Result<Value, ToolError> {
        // SRV-146: on a server this runs as `nodus`, which mustn't make
        // folders where a session says. Its projects come from GitHub.
        if self.on_server {
            return Err(SERVER_PROJECTS.into());
        }
        let Some(folder) = trimmed(folder) else {
            return Err("path is required".into());
        };
        let target = expand_home(folder, self.home.as_deref());
        if !is_absolute(&target) {
            return Err("path must be absolute".into());
        }
        let made_folder = !Path::new(&target).exists();
        if made_folder {
            std::fs::create_dir_all(&target).map_err(|err| ToolError(fs_message(&err, "mkdir", &target)))?;
        }
        let (project, added) = self
            .projects
            .add_project(Path::new(&target), name.and_then(Value::as_str))
            .map_err(ToolError)?;
        self.events.emit(Event::ProjectsChanged);
        let mut out = json!({ "projectId": project.id, "name": project.name, "folder": project.repo_path });
        if made_folder {
            out["madeFolder"] = Value::Bool(true);
        }
        if !added {
            out["alreadyAProject"] = Value::Bool(true);
        }
        Ok(out)
    }

    fn find_project(&self, reference: Option<&Value>) -> Result<String, ToolError> {
        self.store.read(|state| {
            let local: Vec<Project> = state.projects.iter().filter(|p| p.server.is_none()).cloned().collect();
            find_project(&local, reference)
        })
    }

    /// TOOL-035
    fn list_sessions(&self, project_id: &str, self_id: &str, include_archived: bool) -> Result<Value, ToolError> {
        let items = self
            .board
            .session_list(project_id, Some(self_id), include_archived)
            .map_err(ToolError::of)?;
        serde_json::to_value(items).map_err(ToolError::of)
    }

    /// TOOL-036
    fn archive_sessions(
        &self,
        project_id: &str,
        self_id: &str,
        session_ids: &[String],
        force: bool,
    ) -> Result<Value, ToolError> {
        let project = self.project(project_id)?;
        let transcripts = self.board.transcripts();
        let now = (self.clock)();
        let mut effects = Effects::default();
        let (archived, removed, skipped) = self.store.update(|state| {
            let board = state.board_mut(project_id);
            let mut archived = Vec::new();
            let mut removed = Vec::new();
            let mut skipped = Vec::new();
            for id in session_ids {
                // Ids come from Claude: only the board's own nodes count.
                let node = board.nodes.get(id).cloned();
                let status = self.sessions.status(id);
                let reason = match &node {
                    None => Some("not on the board"),
                    Some(_) if id == self_id => Some("it is the session running this action"),
                    Some(n) if n.view_of.as_deref().is_some_and(|v| !v.is_empty()) => {
                        Some("it is a view’s brain, which goes with its view")
                    }
                    Some(n) if n.kind == Some(SessionKind::Terminal) => Some("terminals can’t be archived"),
                    Some(n) if n.archived => Some("already archived"),
                    Some(_) if status != SessionStatus::Done && !force => Some(if status == SessionStatus::Waiting {
                        "it is waiting for the user"
                    } else {
                        "it is working"
                    }),
                    Some(_) => None,
                };
                if let Some(reason) = reason {
                    skipped.push(json!({ "id": id, "reason": reason }));
                    continue;
                }
                let node = node.expect("a node that wasn't skipped");
                // Nothing was said in it, so there is nothing to keep.
                let meta = transcripts.session(&project.repo_path, id).map(|f| f.meta);
                if is_empty(&node, meta.as_ref()) {
                    drop_node(board, id, &mut effects);
                    removed.push(id.clone());
                    continue;
                }
                archive_node(board, id, now, &mut effects);
                archived.push(id.clone());
            }
            (archived, removed, skipped)
        });
        for id in &effects.stop {
            self.sessions.stop(id);
        }
        for id in &effects.stop_and_forget {
            self.sessions.stop_and_forget(id);
        }
        if !archived.is_empty() || !removed.is_empty() {
            self.board_changed(project_id);
        }
        Ok(json!({ "archived": archived, "removed": removed, "skipped": skipped }))
    }

    /// TOOL-037; the session is archived at its next `done` by the board
    /// (`Board::archive_if_asked`, TOOL-038).
    fn archive_self(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        let node = self
            .store
            .read(|state| state.board(project_id).and_then(|b| b.nodes.get(self_id).cloned()));
        let Some(node) = node else {
            return Err("This session is no longer on the board".into());
        };
        if node.kind == Some(SessionKind::Terminal) {
            return Err("Terminals can’t be archived".into());
        }
        if node.archived {
            return Err("This session is already archived".into());
        }
        if node.view_of.as_deref().is_some_and(|v| !v.is_empty()) {
            return Err("A view’s brain stays with its view; deleting the view stops it".into());
        }
        // A run leaves the board when it finishes, with nothing to archive.
        if node.run_of.as_deref().is_some_and(|r| !r.is_empty()) {
            return Err("An action run leaves the board by itself when its turn ends".into());
        }
        self.board.archive_when_done(self_id);
        Ok(json!({ "archiving": "This session moves to the archive when this turn ends." }))
    }

    /// TOOL-032
    fn list_notes(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        Ok(self.store.read(|state| match state.board(project_id) {
            Some(board) => notes::list_notes(board, self_id),
            None => Value::Array(Vec::new()),
        }))
    }

    /// TOOL-029
    fn create_note(&self, project_id: &str, self_id: &str, note: &NotePatch) -> Result<Value, ToolError> {
        // Written from another project's board, where its card isn't, it says where it came from.
        let own = self.project_for_session(self_id);
        let away = match own {
            Some(own) if own != project_id => Some(AwayAuthor {
                title: self.board.session_title(&own, self_id).map_err(ToolError::of)?,
                project: own,
            }),
            _ => None,
        };
        let now = (self.clock)();
        let created = self
            .store
            .update(|state| {
                // SRV-183: on a server, the note is the session's starter's.
                let starter = state.session_starter(self_id).map(str::to_owned);
                let board = state.board_mut(project_id);
                let created = notes::create_note(board, self_id, note, away.as_ref(), now)?;
                if let Some(note) = board.notes.get_mut(&created.id).filter(|_| starter.is_some()) {
                    note.set_created_by(starter.as_deref());
                }
                Ok::<_, notes::NoteError>(created)
            })
            .map_err(ToolError::of)?;
        self.board_changed(project_id);
        Ok(json!({ "noteId": created.id }))
    }

    /// TOOL-031
    fn update_note(&self, project_id: &str, note_id: &str, patch: &NotePatch) -> Result<Value, ToolError> {
        self.store
            .update(|state| notes::update_note(state.board_mut(project_id), note_id, patch))
            .map_err(ToolError::of)?;
        self.board_changed(project_id);
        Ok(json!({ "noteId": note_id }))
    }

    fn list_views(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        self.views.list_views(project_id, self_id).map_err(ToolError)
    }

    fn get_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<Value, ToolError> {
        self.views.get_view(project_id, self_id, view_id).map_err(ToolError)
    }

    fn create_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move {
            self.views
                .create_view(project_id, self_id, args)
                .await
                .map_err(ToolError)
        })
    }

    fn update_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move {
            self.views
                .update_view(project_id, self_id, args)
                .await
                .map_err(ToolError)
        })
    }

    /// TOOL-102
    fn list_board(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        let project = self.project(project_id)?;
        // Read before the state is held: titles come from the transcripts.
        let disk = self.board.scan_board(&project);
        Ok(self.store.read(|state| {
            let empty = BoardState::default();
            let board = state.board(project_id).unwrap_or(&empty);
            let cards: Vec<SessionCard> = notes::active_cards(board)
                .map(|n| to_card(n, disk.get(&n.session_id), self.sessions.as_ref()))
                .collect();
            let items = session_list_items(&cards, &[], None, false);
            drawing::list_board(board, self_id, &items)
        }))
    }

    /// TOOL-111
    fn look_at_board(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Look, ToolError> {
        let project = self.project(project_id)?;
        let disk = self.board.scan_board(&project);
        // A copy of the board, so it's drawn without holding the store.
        let (board, items) = self.store.read(|state| {
            let board = state.board(project_id).cloned().unwrap_or_default();
            let cards: Vec<SessionCard> = notes::active_cards(&board)
                .map(|n| to_card(n, disk.get(&n.session_id), self.sessions.as_ref()))
                .collect();
            let items = session_list_items(&cards, &[], None, false);
            (board, items)
        });
        look::look(&board, self_id, &items, args, &|name| self.picture(name)).map_err(ToolError)
    }

    /// TOOL-103 to TOOL-106, TOOL-112, TOOL-113
    fn draw_on_board(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError> {
        self.project(project_id)?;
        let pictures = self.copy_pictures(args)?;
        let now = (self.clock)();
        let drawn = self.store.update(|state| {
            // SRV-183: on a server, what it draws is the session's starter's.
            let starter = state.session_starter(self_id).map(str::to_owned);
            drawing::draw(
                state.board_mut(project_id),
                self_id,
                starter.as_deref(),
                args,
                now,
                &pictures,
            )
        });
        let drawn = match drawn {
            Ok(drawn) => drawn,
            Err(err) => {
                let files: Vec<PathBuf> = match self.pictures.first() {
                    Some(dir) => pictures.values().map(|p| dir.join(&p.name)).collect(),
                    None => Vec::new(),
                };
                self.drop_pictures(&files);
                return Err(err.into());
            }
        };
        self.board_changed(project_id);
        Ok(drawn)
    }

    /// TOOL-107, TOOL-108
    fn update_drawing(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError> {
        self.project(project_id)?;
        let now = (self.clock)();
        let updated = self
            .store
            .update(|state| drawing::update(state.board_mut(project_id), self_id, args, now))?;
        self.board_changed(project_id);
        Ok(updated)
    }

    /// TOOL-109
    fn erase_from_board(&self, project_id: &str, _self_id: &str, args: &Value) -> Result<Value, ToolError> {
        self.project(project_id)?;
        let erased = self
            .store
            .update(|state| drawing::erase(state.board_mut(project_id), args))?;
        if erased["erased"].as_array().is_some_and(|e| !e.is_empty()) {
            self.board_changed(project_id);
        }
        Ok(erased)
    }
}
