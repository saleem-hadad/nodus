//! `window.api.board`: a project's board, merged with the sessions on disk,
//! and what the window changes on it (`board:*` in `src/main/ipc.ts`).

use std::path::{Path, PathBuf};

use std::collections::HashMap;

use nodus_domain::{
    BoardLink, BoardNote, BoardSection, BoardShape, BoardSnapshot, CreatedBy, InkStroke, NodePatch, Rect, Viewport,
};
use nodus_store::BoardState;
use serde_json::Value;

use nodus_remote::Request;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.board`.
pub struct BoardApi<'a> {
    pub(crate) nodus: &'a Nodus,
    /// The member a server makes these calls for ([`Nodus::by`]): what
    /// they add records them as `createdBy` (SRV-183). None in the app.
    pub(crate) by: Option<&'a str>,
}

impl BoardApi<'_> {
    /// `board:load` (SESS-071…081): runs that ended while the app was closed
    /// finished, the board merged with the sessions on disk (new ones laid
    /// out in the archive), and its transcript folder watched: each change
    /// there arrives as `Event::BoardChanged`. `Unknown project <id>`.
    ///
    /// Capture is told the board opened first (SESS-071 step 1, CAP-154).
    ///
    /// A server project's board comes from its server, with this person's
    /// own viewport and expanded cards; while the server can't be reached,
    /// its last snapshot, marked `offline` (SRV-028, SRV-164).
    pub async fn load(&self, project_id: &str) -> CommandResult<BoardSnapshot> {
        let route = self.nodus.route_project(project_id);
        self.nodus
            .on_core(async {
                self.nodus.inner.capture.board_opened(project_id);
                match route {
                    Some(route) => route.load_board().await,
                    None => self.nodus.inner.board.load(project_id).on("board:load"),
                }
            })
            .await
    }

    /// `board:save-nodes` (SESS-090…099): moves, renames, archives (stopping
    /// the process), groups and resizes cards. A card with nothing said in it
    /// is removed rather than archived.
    pub async fn save_nodes(&self, project_id: &str, patches: &[NodePatch]) -> CommandResult<()> {
        if let Some(route) = self.nodus.route_project(project_id) {
            return route.save_nodes(patches).await;
        }
        self.nodus
            .on_core(async {
                self.nodus
                    .inner
                    .board
                    .save_nodes(project_id, patches)
                    .on("board:save-nodes")
            })
            .await
    }

    /// `board:save-archive` (SESS-106).
    pub async fn save_archive(&self, project_id: &str, rect: Rect) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let project_id = project_id.to_owned();
            return route.run(Request::BoardSaveArchive { project_id, rect }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.save_archive(project_id, rect);
            })
            .await;
    }

    /// `board:save-viewport` (SESS-106). A server board's is this person's
    /// own, kept here (SRV-164).
    pub async fn save_viewport(&self, project_id: &str, viewport: Viewport) {
        if let Some(route) = self.nodus.route_project(project_id) {
            return route.save_viewport(viewport);
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.save_viewport(project_id, viewport);
            })
            .await;
    }

    /// `board:remove-node` (SESS-107, SESS-108): the card goes, its session
    /// is hidden for good unless it's a terminal, and this waits for its
    /// process to stop. The transcript stays.
    pub async fn remove_node(&self, project_id: &str, session_id: &str) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, session_id) = (project_id.to_owned(), session_id.to_owned());
            return route.run(Request::BoardRemoveNode { project_id, session_id }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_node(project_id, session_id).await;
            })
            .await;
    }

    /// `board:save-section` (SESS-100): creates or updates a section.
    pub async fn save_section(&self, project_id: &str, section: &BoardSection) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, section) = (project_id.to_owned(), section.clone());
            return route.run(Request::BoardSaveSection { project_id, section }).await;
        }
        self.nodus
            .on_core(async {
                let ids = [section.id.as_str()];
                let before = self.creators(project_id, &ids, |b, id| b.sections.get(id).map(|s| s.created_by()));
                self.nodus.inner.board.save_section(project_id, &to_json(section));
                self.attribute(project_id, &ids, before, |b, id| b.sections.get_mut(id).map(as_created));
            })
            .await;
    }

    /// `board:remove-section` (SESS-101): its cards stay where they are.
    pub async fn remove_section(&self, project_id: &str, section_id: &str) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, section_id) = (project_id.to_owned(), section_id.to_owned());
            return route.run(Request::BoardRemoveSection { project_id, section_id }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_section(project_id, section_id);
            })
            .await;
    }

    /// `board:name-section` (SESS-102): a short name from Claude for a
    /// section holding these sessions, once the login shell's environment is
    /// in. `These sessions have nothing to name the section after yet`.
    pub async fn name_section(&self, project_id: &str, session_ids: &[String]) -> CommandResult<String> {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, session_ids) = (project_id.to_owned(), session_ids.to_vec());
            return route
                .call(Request::BoardNameSection {
                    project_id,
                    session_ids,
                })
                .await;
        }
        self.nodus
            .on_core(async {
                let env = self.nodus.env().await;
                self.nodus
                    .inner
                    .board
                    .name_section(project_id, session_ids, &env, &*self.nodus.inner.runner)
                    .await
                    .on("board:name-section")
            })
            .await
    }

    /// `board:save-note` (SESS-103): creates or updates a sticky note.
    pub async fn save_note(&self, project_id: &str, note: &BoardNote) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, note) = (project_id.to_owned(), note.clone());
            return route.run(Request::BoardSaveNote { project_id, note }).await;
        }
        self.nodus
            .on_core(async {
                let ids = [note.id.as_str()];
                let before = self.creators(project_id, &ids, |b, id| b.notes.get(id).map(|n| n.created_by()));
                self.nodus.inner.board.save_note(project_id, &to_json(note));
                self.attribute(project_id, &ids, before, |b, id| b.notes.get_mut(id).map(as_created));
            })
            .await;
    }

    /// `board:remove-notes` (SESS-104).
    pub async fn remove_notes(&self, project_id: &str, note_ids: &[String]) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, note_ids) = (project_id.to_owned(), note_ids.to_vec());
            return route.run(Request::BoardRemoveNotes { project_id, note_ids }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_notes(project_id, note_ids);
            })
            .await;
    }

    /// `board:save-ink` (SESS-105, INK-012): replaces the board's ink with
    /// these strokes, cleaned as views-tools' `cleanInk` does.
    pub async fn save_ink(&self, project_id: &str, strokes: &[InkStroke]) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, strokes) = (project_id.to_owned(), strokes.to_vec());
            return route.run(Request::BoardSaveInk { project_id, strokes }).await;
        }
        self.nodus
            .on_core(async {
                let ids: Vec<&str> = strokes.iter().map(|s| s.id.as_str()).collect();
                let before = self.creators(project_id, &ids, |b, id| {
                    let ink = b.ink.as_deref().unwrap_or_default();
                    ink.iter().find(|s| s.id == id).map(|s| s.created_by())
                });
                let strokes = nodus_board::ink::clean_ink(&to_json(&strokes));
                self.nodus.inner.board.save_ink(project_id, strokes);
                self.attribute(project_id, &ids, before, |b, id| {
                    b.ink.as_mut()?.iter_mut().find(|s| s.id == id).map(as_created)
                });
            })
            .await;
    }

    /// `board:save-shapes` (added after 0.9.4): creates or updates shapes.
    pub async fn save_shapes(&self, project_id: &str, shapes: &[BoardShape]) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, shapes) = (project_id.to_owned(), shapes.to_vec());
            return route.run(Request::BoardSaveShapes { project_id, shapes }).await;
        }
        self.nodus
            .on_core(async {
                let ids: Vec<&str> = shapes.iter().map(|s| s.id.as_str()).collect();
                let before = self.creators(project_id, &ids, |b, id| b.shapes.get(id).map(|s| s.created_by()));
                self.nodus.inner.board.save_shapes(project_id, shapes);
                self.attribute(project_id, &ids, before, |b, id| b.shapes.get_mut(id).map(as_created));
            })
            .await;
    }

    /// `board:remove-shapes`: the shapes go, with their links.
    pub async fn remove_shapes(&self, project_id: &str, ids: &[String]) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, ids) = (project_id.to_owned(), ids.to_vec());
            return route.run(Request::BoardRemoveShapes { project_id, ids }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_shapes(project_id, ids);
            })
            .await;
    }

    /// `board:save-links`: creates or updates links.
    pub async fn save_links(&self, project_id: &str, links: &[BoardLink]) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, links) = (project_id.to_owned(), links.to_vec());
            return route.run(Request::BoardSaveLinks { project_id, links }).await;
        }
        self.nodus
            .on_core(async {
                let ids: Vec<&str> = links.iter().map(|l| l.id.as_str()).collect();
                let before = self.creators(project_id, &ids, |b, id| b.links.get(id).map(|l| l.created_by()));
                self.nodus.inner.board.save_links(project_id, links);
                self.attribute(project_id, &ids, before, |b, id| b.links.get_mut(id).map(as_created));
            })
            .await;
    }

    /// `board:remove-links`.
    pub async fn remove_links(&self, project_id: &str, ids: &[String]) {
        if let Some(route) = self.nodus.route_project(project_id) {
            let (project_id, ids) = (project_id.to_owned(), ids.to_vec());
            return route.run(Request::BoardRemoveLinks { project_id, ids }).await;
        }
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_links(project_id, ids);
            })
            .await;
    }

    /// The project whose board has this session, as a card or a view's
    /// brain; none once it has left every board. A server sends a session's
    /// events only to the clients watching its project (SRV-026). A session
    /// on a server board this app has loaded is found there (SRV-160).
    pub fn session_project(&self, session_id: &str) -> Option<String> {
        let local = self.nodus.inner.store.read(|state| {
            state
                .boards
                .iter()
                .find(|(_, board)| board.nodes.contains_key(session_id))
                .map(|(project_id, _)| project_id.clone())
        });
        local.or_else(|| {
            self.nodus
                .inner
                .remotes
                .session_home(session_id)
                .map(|home| home.project_id)
        })
    }

    /// Who made each of these before a save: the ones already on the board,
    /// by id, with their `createdBy` (SRV-183).
    fn creators(
        &self,
        project_id: &str,
        ids: &[&str],
        read: impl for<'b> Fn(&'b BoardState, &str) -> Option<Option<&'b str>>,
    ) -> HashMap<String, Option<String>> {
        self.nodus.inner.store.read(|state| {
            let Some(board) = state.board(project_id) else {
                return HashMap::new();
            };
            ids.iter()
                .filter_map(|&id| read(board, id).map(|by| (id.to_owned(), by.map(str::to_owned))))
                .collect()
        })
    }

    /// SRV-183: once saved, each keeps who made it, and a new one is the
    /// member's who saved it. Saves rebuild what they're given, so this puts
    /// `createdBy` back. Nothing changes where nobody made anything: a local
    /// board, or one from before attribution.
    fn attribute(
        &self,
        project_id: &str,
        ids: &[&str],
        before: HashMap<String, Option<String>>,
        mut find: impl for<'b> FnMut(&'b mut BoardState, &str) -> Option<&'b mut dyn CreatedBy>,
    ) {
        if self.by.is_none() && before.values().all(Option::is_none) {
            return;
        }
        self.nodus.inner.store.update(|state| {
            let board = state.board_mut(project_id);
            for &id in ids {
                let by = match before.get(id) {
                    Some(made) => made.as_deref(),
                    None => self.by,
                };
                if let Some(item) = find(board, id) {
                    item.set_created_by(by);
                }
            }
        });
    }

    /// Where the board's pictures are kept: `board-images` in the data folder.
    pub fn images_dir(&self) -> PathBuf {
        images_dir(&self.nodus.inner.user_data)
    }

    /// A picture shape's file on `project_id`'s board, by its name; none for
    /// a name that isn't one. A server board's pictures are its server's:
    /// each is kept here once fetched, and until it is, this is none, its
    /// fetch starts, and the board changes when it's in (SRV-163).
    pub fn image_path(&self, project_id: &str, name: &str) -> Option<PathBuf> {
        if !nodus_board::shapes::image_name_ok(name) {
            return None;
        }
        match self.nodus.route_project(project_id) {
            Some(route) => self.nodus.server_file(route.server_id(), project_id, name),
            None => Some(self.images_dir().join(name)),
        }
    }

    /// `board:save-image`: keeps a picture's bytes under a new name, with
    /// the extension of its type (`png`, `jpg`, `gif`, `webp`…), and returns
    /// the name a picture shape holds. A server board's picture is uploaded
    /// to its server, which picks the name, and kept here too (SRV-163).
    pub async fn save_image(&self, project_id: &str, bytes: Vec<u8>, ext: &str) -> CommandResult<String> {
        let ext = clean_ext(ext);
        if let Some(route) = self.nodus.route_project(project_id) {
            let upload = route
                .upload("board:save-image", bytes.clone(), &format!("picture.{ext}"))
                .await?;
            self.nodus.keep_server_file(route.server_id(), &upload.id, bytes).await;
            return Ok(upload.id);
        }
        let dir = self.images_dir();
        self.nodus
            .on_core(async move {
                let name = format!("{}.{ext}", uuid::Uuid::new_v4());
                tokio::fs::create_dir_all(&dir).await.on("board:save-image")?;
                tokio::fs::write(dir.join(&name), bytes).await.on("board:save-image")?;
                Ok(name)
            })
            .await
    }

    /// `board:import-image`: copies a picture file in, as `save_image` keeps bytes.
    pub async fn import_image(&self, project_id: &str, file: &Path) -> CommandResult<String> {
        let bytes = tokio::fs::read(file).await.on("board:import-image")?;
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("png").to_owned();
        self.save_image(project_id, bytes, &ext).await
    }
}

/// `board-images` in the data folder: where a board's pictures are kept.
pub fn images_dir(user_data: &Path) -> PathBuf {
    user_data.join("board-images")
}

/// A picture's extension, lower case and plain; `png` for anything odd.
fn clean_ext(ext: &str) -> String {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext
    } else {
        "png".to_owned()
    }
}

/// Anything that records who made it, as one.
fn as_created<T: CreatedBy>(item: &mut T) -> &mut dyn CreatedBy {
    item
}

/// What the window sent, as the JSON the handlers check.
fn to_json(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
