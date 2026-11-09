//! Plugs the cards, notes, archive and pad into the canvas's node interface
//! (`board::nodes`): [`CardRenderers`] turns each node's props and the
//! board's context into the elements of this module, and [`CanvasActions`]
//! maps [`CardActions`] onto the canvas's `BoardActions` and the app state.

use std::rc::Rc;

use gpui::{AnyElement, App, Bounds, IntoElement, Window, point, px, size};
use nodus_domain::{BoardNote, NodePatch, OpenCardRequest, Rect};

use super::present::StackLook;
use super::{BoardEnv, CardActions, Resized, StackView, Z, archive_zone, note, note_pad, session_card};
use crate::board::nodes::{
    self, ArchiveNode, BoardActions, BoardCx, CardNode, NodeRenderers, NoteNode, PadNode, StackPlace,
};
use crate::parity;
use crate::platform::Platform;
use crate::shell::window::Core;
use crate::state::AppState;

/// Installs the renderers (from `board::init`).
pub fn install(cx: &mut App) {
    nodes::install(CardRenderers, cx);
}

/// Draws the board's cards, notes, archive and pad.
pub struct CardRenderers;

/// The canvas's actions, and what 0.9.4's nodes asked of the store.
pub struct CanvasActions(pub BoardActions);

impl CanvasActions {
    fn state(&self, cx: &App) -> Option<gpui::Entity<AppState>> {
        Some(self.0.canvas.upgrade()?.read(cx).state.clone())
    }
}

impl CardActions for CanvasActions {
    fn rename(&self, session_id: &str, title: &str, cx: &mut App) {
        self.0.rename(session_id, title, cx);
    }
    fn set_renaming(&self, id: Option<&str>, cx: &mut App) {
        self.0.set_renaming(id, cx);
    }
    fn expand(&self, session_ids: &[String], expanded: bool, cx: &mut App) {
        self.0.expand(session_ids, expanded, cx);
        if expanded {
            // CARD-023: expanding also wakes the card's terminal.
            for id in session_ids {
                super::expanded_card::wake_card_terminal(id, cx);
            }
        }
    }
    fn archive(&self, session_ids: &[String], cx: &mut App) {
        self.0.archive(session_ids, cx);
    }
    fn stop(&self, session_ids: &[String], cx: &mut App) {
        self.0.stop(session_ids, cx);
    }
    fn edit_note(&self, project_id: &str, note: &BoardNote, text: &str, cx: &mut App) {
        // NOTEUI-015: once the board has gone (another project, or closed),
        // the text goes straight to the note's own project.
        let here = self
            .0
            .canvas
            .upgrade()
            .is_some_and(|c| c.read(cx).project_id == project_id);
        if here {
            self.0.edit_note(note, text, cx);
        } else if let Some(core) = cx.try_global::<Core>().map(|c| c.0.clone()) {
            let project = project_id.to_owned();
            let note = BoardNote {
                text: text.to_owned(),
                ..note.clone()
            };
            cx.spawn(async move |_| core.board().save_note(&project, &note).await)
                .detach();
        }
    }
    fn start_note(&self, note_id: &str, cx: &mut App) {
        self.0.start_note(note_id, cx);
    }
    fn open_session(&self, session_id: &str, cx: &mut App) {
        self.0.open_session(session_id, cx);
    }
    fn show_card(&self, project_id: &str, session_id: Option<&str>, cx: &mut App) {
        if let Some(state) = self.state(cx) {
            let request = OpenCardRequest {
                project_id: project_id.to_owned(),
                session_id: session_id.map(str::to_owned),
            };
            state.update(cx, |s, cx| s.show_card(request, cx));
        }
    }
    fn open_agents(&self, session_id: &str, agent_id: Option<&str>, cx: &mut App) {
        if let Some(state) = self.state(cx) {
            let view = (session_id.to_owned(), agent_id.map(str::to_owned));
            state.update(cx, |s, cx| {
                s.agent_view = Some(view);
                cx.notify();
            });
        }
    }
    fn open_terminal(&self, session_id: &str, cx: &mut App) {
        if let Some(state) = self.state(cx) {
            let id = session_id.to_owned();
            state.update(cx, |s, cx| {
                s.terminal_session_id = Some(id);
                cx.notify();
            });
        }
    }
    fn remove_notes(&self, note_ids: &[String], cx: &mut App) {
        self.0.remove_notes(note_ids.to_vec(), cx);
    }
    fn resize(&self, node: &Resized, rect: (f32, f32, f32, f32), done: bool, cx: &mut App) {
        let (x, y, w, h) = rect;
        let id = match node {
            Resized::Card(id) => id.clone(),
            Resized::Note(note) => note.id.clone(),
        };
        if !done {
            let rect = Rect {
                x: f64::from(x),
                y: f64::from(y),
                w: f64::from(w),
                h: f64::from(h),
            };
            self.0.hold_rect(&id, Some(rect), cx);
            return;
        }
        let (x, y, w, h) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h));
        match node {
            Resized::Card(id) => self.0.patch_cards(
                vec![NodePatch {
                    session_id: id.clone(),
                    x: Some(x),
                    y: Some(y),
                    w: Some(w),
                    h: Some(h),
                    ..NodePatch::default()
                }],
                cx,
            ),
            Resized::Note(note) => self.0.save_note(
                BoardNote {
                    x,
                    y,
                    w: Some(w),
                    h: Some(h),
                    ..(**note).clone()
                },
                cx,
            ),
        }
        self.0.hold_rect(&id, None, cx);
    }
    fn press_title(&self) {
        self.0.press_title();
    }
    fn raise(&self, session_id: &str, raised: bool, cx: &mut App) {
        self.0.raise(session_id, raised, cx);
    }
    fn set_stack_query(&self, query: &str, cx: &mut App) {
        self.0.set_stack_query(query, cx);
    }
    fn open_stack(&self, cx: &mut App) {
        self.0.open_stack(cx);
    }
    fn close_stack(&self, cx: &mut App) {
        self.0.close_stack(cx);
    }
    fn submit_stack(&self, window: &mut Window, cx: &mut App) {
        self.0.submit_stack(window, cx);
    }
}

/// The board's context as the cards read it.
pub fn env(board: &BoardCx, zoom: f32, window: &Window, cx: &App) -> Rc<BoardEnv> {
    let state = board
        .actions
        .canvas
        .upgrade()
        .map(|c| c.read(cx).state.clone());
    let state = state.as_ref().map(|s| s.read(cx));
    let repo_path = state
        .and_then(|s| s.board.as_ref())
        .filter(|b| b.project.id == board.project_id.as_ref())
        .map(|b| b.project.repo_path.clone());
    let viewport = window.viewport_size();
    Rc::new(BoardEnv {
        scale: Z(zoom),
        zoom: board.zoom,
        project_id: board.project_id.to_string(),
        repo_path,
        renaming: board.renaming.as_ref().map(|r| r.to_string()),
        archive_hot: board.archive_hot,
        stack: StackView {
            open: board.stack.open,
            hover: board.stack.hover,
            query: board.stack.query.to_string(),
            result_count: board.stack.result_count,
            scroll: board.stack.scroll,
            scroll_max: board.stack.scroll_max,
        },
        // The canvas spans the window under the title bar.
        board_bounds: Bounds::new(point(px(0.), px(52.)), size(viewport.width, viewport.height - px(52.))),
        now_ms: parity::now_ms(cx),
        text_focus: board.renaming.is_some(),
        terminal_modal: state.and_then(|s| s.terminal_session_id.clone()),
        platform: Platform::CURRENT,
        actions: Rc::new(CanvasActions(board.actions.clone())),
        terminals: None,
    })
}

/// BOARD-015 → the look of an archived card in the stack.
pub fn stack_look(place: StackPlace, rise: Option<gpui::Point<f32>>, lift: bool) -> StackLook {
    match place {
        StackPlace::Pile { depth } => StackLook {
            depth: depth.min(3) as u8,
            result: false,
            lifted: lift,
            rise: rise.map(|p| (p.x, p.y, 0)),
        },
        StackPlace::Result { slot } => StackLook {
            depth: 0,
            result: true,
            lifted: false,
            rise: rise.map(|p| (p.x, p.y, slot)),
        },
    }
}

impl NodeRenderers for CardRenderers {
    fn card(&self, node: CardNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        session_card::SessionNode {
            env,
            card: node.card.clone(),
            matched: node.matched.map(<[usize]>::to_vec).unwrap_or_default(),
            stickers: node.stickers.to_vec(),
            selected: node.node.selected,
            dragging: node.node.dragging,
            stack: node
                .stack
                .map(|s| stack_look(s.place, s.rise, board.stack_lift && !board.stack.open)),
            size: (node.node.size.width, node.node.size.height),
        }
        .into_any_element()
    }

    fn note(&self, node: NoteNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        let signature = signature_for(node.note, board, cx);
        note::render(&env, node, signature, window, cx)
    }

    fn archive(&self, node: ArchiveNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        archive_zone::render(&env, &node, board, window, cx)
    }

    fn pad(&self, node: PadNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        note_pad::render(&env, &node, window, cx)
    }
}

/// NOTEUI-018, NOTEUI-019: who signs a note, from the board's cards and the
/// projects.
fn signature_for(note: &BoardNote, board: &BoardCx, cx: &App) -> Option<super::present::Signature> {
    note.from.as_ref()?;
    let canvas = board.actions.canvas.upgrade()?;
    let state = canvas.read(cx).state.read(cx);
    let cards = state.board.as_ref().map_or(&[][..], |b| &b.cards[..]);
    super::present::signature(note, cards, &state.projects)
}
