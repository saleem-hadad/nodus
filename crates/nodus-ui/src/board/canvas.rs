//! The board canvas (`BoardCanvas` in `BoardView.tsx`): one project's
//! pannable, zoomable board. Work in progress: this first cut holds the
//! entry points the node plug-in interface (`nodes.rs`) calls; the canvas
//! mechanics land behaviour group by behaviour group (see
//! `migration/tracks/board-canvas.md`).

use gpui::{Context, Entity, IntoElement, ParentElement, Pixels, Point, Render, Styled, Window, div};
use nodus_domain::{BoardNote, NodePatch, Rect, ViewPatch};

use crate::state::AppState;
use crate::theme;

/// One project's board.
pub struct BoardCanvas {
    pub state: Entity<AppState>,
    pub project_id: String,
    /// The card, section, note or view being renamed (BOARD-148).
    pub renaming: Option<String>,
}

impl BoardCanvas {
    pub fn new(state: Entity<AppState>, project_id: String) -> Self {
        Self {
            state,
            project_id,
            renaming: None,
        }
    }

    pub fn rename_card(&mut self, session_id: &str, title: &str, cx: &mut Context<'_, Self>) {
        let _ = (session_id, title, cx);
    }

    pub fn set_renaming(&mut self, id: Option<String>, cx: &mut Context<'_, Self>) {
        self.renaming = id;
        cx.notify();
    }

    pub fn rename_section(&mut self, section_id: &str, name: &str, cx: &mut Context<'_, Self>) {
        let _ = (section_id, name, cx);
    }

    pub fn expand_cards(&mut self, ids: &[String], expanded: bool, cx: &mut Context<'_, Self>) {
        let _ = (ids, expanded, cx);
    }

    pub fn archive_cards(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let _ = (ids, cx);
    }

    pub fn stop_cards(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let _ = (ids, cx);
    }

    pub fn edit_note(&mut self, note: BoardNote, text: String, cx: &mut Context<'_, Self>) {
        let _ = (note, text, cx);
    }

    pub fn start_note(&mut self, note_id: String, cx: &mut Context<'_, Self>) {
        let _ = (note_id, cx);
    }

    pub fn open_card(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        let _ = (session_id, cx);
    }

    pub fn open_view_session(&mut self, view_id: String, cx: &mut Context<'_, Self>) {
        let _ = (view_id, cx);
    }

    pub fn rename_view(&mut self, view_id: &str, title: &str, cx: &mut Context<'_, Self>) {
        let _ = (view_id, title, cx);
    }

    pub fn view_menu(&mut self, view_id: &str, at: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let _ = (view_id, at, cx);
    }

    pub fn set_stack_query(&mut self, query: String, cx: &mut Context<'_, Self>) {
        let _ = (query, cx);
    }

    pub fn open_stack(&mut self, cx: &mut Context<'_, Self>) {
        let _ = cx;
    }

    pub fn close_stack(&mut self, cx: &mut Context<'_, Self>) {
        let _ = cx;
    }

    pub fn submit_stack(&mut self, cx: &mut Context<'_, Self>) {
        let _ = cx;
    }

    pub fn patch_cards(&mut self, patches: Vec<NodePatch>, cx: &mut Context<'_, Self>) {
        let _ = (patches, cx);
    }

    pub fn save_note(&mut self, note: BoardNote, cx: &mut Context<'_, Self>) {
        let _ = (note, cx);
    }

    pub fn remove_notes(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let _ = (ids, cx);
    }

    pub fn patch_view(&mut self, patch: ViewPatch, cx: &mut Context<'_, Self>) {
        let _ = (patch, cx);
    }

    pub fn hold_rect(&mut self, id: String, rect: Option<Rect>, cx: &mut Context<'_, Self>) {
        let _ = (id, rect, cx);
    }

    pub fn raise(&mut self, id: String, raised: bool, cx: &mut Context<'_, Self>) {
        let _ = (id, raised, cx);
    }

    pub fn show_toast(&mut self, message: String, cx: &mut Context<'_, Self>) {
        let _ = (message, cx);
    }
}

impl Render for BoardCanvas {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        div().size_full().bg(t.bg_canvas).child(div())
    }
}
