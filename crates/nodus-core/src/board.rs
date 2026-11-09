//! `window.api.board`: a project's board, merged with the sessions on disk,
//! and what the window changes on it (`board:*` in `src/main/ipc.ts`).

use nodus_domain::{BoardNote, BoardSection, BoardSnapshot, InkStroke, NodePatch, Rect, Viewport};
use serde_json::Value;

use crate::error::OnChannel;
use crate::{CommandResult, Nodus};

/// `window.api.board`.
pub struct BoardApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl BoardApi<'_> {
    /// `board:load` (SESS-071…081): runs that ended while the app was closed
    /// finished, the board merged with the sessions on disk (new ones laid
    /// out in the archive), and its transcript folder watched: each change
    /// there arrives as `Event::BoardChanged`. `Unknown project <id>`.
    ///
    /// Capture is told the board opened first (SESS-071 step 1) once the
    /// capture track is wired.
    pub async fn load(&self, project_id: &str) -> CommandResult<BoardSnapshot> {
        self.nodus
            .on_core(async {
                self.nodus.inner.capture.board_opened(project_id);
                self.nodus.inner.board.load(project_id).on("board:load")
            })
            .await
    }

    /// `board:save-nodes` (SESS-090…099): moves, renames, archives (stopping
    /// the process), groups and resizes cards. A card with nothing said in it
    /// is removed rather than archived.
    pub async fn save_nodes(&self, project_id: &str, patches: &[NodePatch]) -> CommandResult<()> {
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
        self.nodus
            .on_core(async {
                self.nodus.inner.board.save_archive(project_id, rect);
            })
            .await;
    }

    /// `board:save-viewport` (SESS-106).
    pub async fn save_viewport(&self, project_id: &str, viewport: Viewport) {
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
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_node(project_id, session_id).await;
            })
            .await;
    }

    /// `board:save-section` (SESS-100): creates or updates a section.
    pub async fn save_section(&self, project_id: &str, section: &BoardSection) {
        self.nodus
            .on_core(async {
                self.nodus.inner.board.save_section(project_id, &to_json(section));
            })
            .await;
    }

    /// `board:remove-section` (SESS-101): its cards stay where they are.
    pub async fn remove_section(&self, project_id: &str, section_id: &str) {
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
        self.nodus
            .on_core(async {
                let env = self.nodus.env().await;
                self.nodus
                    .inner
                    .board
                    .name_section(project_id, session_ids, &env)
                    .await
                    .on("board:name-section")
            })
            .await
    }

    /// `board:save-note` (SESS-103): creates or updates a sticky note.
    pub async fn save_note(&self, project_id: &str, note: &BoardNote) {
        self.nodus
            .on_core(async {
                self.nodus.inner.board.save_note(project_id, &to_json(note));
            })
            .await;
    }

    /// `board:remove-notes` (SESS-104).
    pub async fn remove_notes(&self, project_id: &str, note_ids: &[String]) {
        self.nodus
            .on_core(async {
                self.nodus.inner.board.remove_notes(project_id, note_ids);
            })
            .await;
    }

    /// `board:save-ink` (SESS-105, INK-012): replaces the board's ink with
    /// these strokes, cleaned as views-tools' `cleanInk` does.
    pub async fn save_ink(&self, project_id: &str, strokes: &[InkStroke]) {
        self.nodus
            .on_core(async {
                let strokes = nodus_board::ink::clean_ink(&to_json(&strokes));
                self.nodus.inner.board.save_ink(project_id, strokes);
            })
            .await;
    }
}

/// What the window sent, as the JSON the handlers check.
fn to_json(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
