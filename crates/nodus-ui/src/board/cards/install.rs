//! Plugs the cards, notes, archive and pad into the canvas's node interface
//! (`board::nodes`): [`CardRenderers`] turns each node's props and the
//! board's context into the elements of this module, and [`CanvasActions`]
//! maps [`CardActions`] onto the canvas's `BoardActions` and the app state.

use std::rc::Rc;

use gpui::{AnyElement, App, Bounds, IntoElement, Window, point, px, size};
use nodus_domain::{BoardNote, NodePatch, OpenCardRequest, Rect};

use super::present::{Phase, StackLook};
use super::{
    BoardEnv, CardActions, Resized, StackView, TerminalHost, Z, archive_zone, note, note_pad, session_card, shape,
};
use crate::board::nodes::{
    self, ArchiveNode, BoardActions, BoardCx, CardNode, NodeRenderers, NoteNode, PadNode, ShapeNode, StackPlace,
};
use crate::parity;
use crate::platform::Platform;
use crate::shell::window::Core;
use crate::state::AppState;
use crate::terminal::card::CardTerminals;
use nodus_terminal::host::Phase as TermPhase;

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
        app_state(cx)
    }
}

/// The app's state, from the main window's shell: never through the
/// canvas, which is being updated while it draws its nodes (and may be when
/// a node's action runs), so reading it there would panic.
fn app_state(cx: &App) -> Option<gpui::Entity<AppState>> {
    Some(crate::shell::window::main_shell(cx)?.read(cx).state.clone())
}

impl CardActions for CanvasActions {
    fn rename(&self, session_id: &str, title: &str, cx: &mut App) {
        self.0.rename(session_id, title, cx);
    }
    fn set_renaming(&self, id: Option<&str>, cx: &mut App) {
        self.0.set_renaming(id, cx);
    }
    fn expand(&self, session_ids: &[String], expanded: bool, cx: &mut App) {
        // CARD-023: the canvas wakes the cards it expands (`NodeRenderers::wake`).
        self.0.expand(session_ids, expanded, cx);
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
        let here = app_state(cx)
            .and_then(|s| s.read(cx).board.as_ref().map(|b| b.project.id == project_id))
            .unwrap_or(false);
        if here {
            self.0.edit_note(note, text, cx);
        } else if let Some(core) = cx.try_global::<Core>().map(|c| c.0.clone()) {
            let project = project_id.to_owned();
            let note = BoardNote {
                text: text.to_owned(),
                ..note.clone()
            };
            let rt = crate::board::core_rt(cx);
            cx.spawn(async move |_| {
                crate::board::on_core(&rt, async move { core.board().save_note(&project, &note).await }).await;
            })
            .detach();
        }
    }
    fn start_note(&self, note_id: &str, cx: &mut App) {
        self.0.start_note(note_id, cx);
    }
    fn edit_shape(&self, shape_id: &str, text: &str, cx: &mut App) {
        self.0.edit_shape(shape_id, text, cx);
    }
    fn focus_board(&self, window: &mut Window, cx: &mut App) {
        self.0.focus_board(window, cx);
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
            Resized::Card(id) | Resized::Shape(id) => id.clone(),
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
            Resized::Shape(id) => self.0.resize_shape(id, Rect { x, y, w, h }, cx),
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
pub fn env(board: &BoardCx, zoom: f32, window: &Window, cx: &mut App) -> Rc<BoardEnv> {
    let state = app_state(cx);
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
        renaming: board.renaming.as_ref().map(ToString::to_string),
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
        terminals: Some(Rc::new(CardHost {
            terminals: card_terminals(&board.project_id, zoom, cx),
            project_id: board.project_id.to_string(),
        })),
    })
}

/// The expanded cards' terminals (spec 12's `CardTerminals`), the card's
/// wheel arbiter deciding who a gesture belongs to (XCARD-027, XCARD-028).
fn card_terminals(project_id: &str, zoom: f32, cx: &mut App) -> CardTerminals {
    CardTerminals::new(project_id.to_owned(), zoom, cx).with_wheel_gate(Rc::new(|_, ctrl, cx| {
        cx.default_global::<super::expanded_card::Wheel>()
            .0
            .claims(std::time::Instant::now(), ctrl)
    }))
}

/// The expanded cards' terminals and mics on one board.
struct CardHost {
    terminals: CardTerminals,
    project_id: String,
}

impl TerminalHost for CardHost {
    fn needs_open(&self, session_id: &str) -> bool {
        self.terminals.needs_open(session_id)
    }
    fn set_enabled(&self, session_id: &str, kind_terminal: bool, enabled: bool, cx: &mut App) {
        self.terminals.set_enabled(session_id, kind_terminal, enabled, cx);
    }
    fn phase(&self, session_id: &str, cx: &App) -> Phase {
        match self.terminals.phase(session_id, cx) {
            None => Phase::Idle,
            Some(p) => match p.phase {
                TermPhase::Loading => Phase::Loading,
                TermPhase::Ready => Phase::Ready,
                TermPhase::Error => Phase::Error(p.error),
            },
        }
    }
    fn reattach(&self, session_id: &str, cx: &mut App) {
        self.terminals.reattach(session_id, cx);
    }
    fn focus(&self, session_id: &str, window: &mut Window, cx: &mut App) {
        self.terminals.focus(session_id, window, cx);
    }
    fn type_in(&self, session_id: &str, text: &str, starting: bool, cx: &mut App) {
        self.terminals.type_in(session_id, text, starting, cx);
    }
    fn element(&self, session_id: &str, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.terminals.element(session_id, window, cx)
    }
    fn mic(&self, session_id: &str, live: bool, scale: f32, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        // XCARD-010 (TERM-102): `typeIn`, then the wake, as ExpandedCard.tsx.
        let terminals = self.terminals.clone();
        let id = session_id.to_owned();
        let type_in: crate::voice::OnText = Rc::new(move |text: String, cx: &mut App| {
            terminals.type_in(&id, &text, !live, cx);
            super::expanded_card::wake_card_terminal(&id, cx);
        });
        Some(crate::voice::card_mic(
            session_id,
            &self.project_id,
            scale,
            type_in,
            window,
            cx,
        ))
    }
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
            // BOARD-036: a search jump's flash.
            flash: board
                .flash
                .as_ref()
                .filter(|(id, _)| id.as_ref() == node.card.node.session_id)
                .map(|(_, at)| *at),
        }
        .into_any_element()
    }

    fn note(&self, node: NoteNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        let signature = signature_for(node.note, board, cx);
        note::render(&env, &node, signature, window, cx)
    }

    fn archive(&self, node: ArchiveNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        archive_zone::render(&env, &node, board, window, cx)
    }

    fn pad(&self, node: PadNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        note_pad::render(&env, &node, window, cx)
    }

    fn shape(&self, node: ShapeNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement {
        let env = env(board, node.node.zoom, window, cx);
        shape::render(&env, &node, window, cx)
    }

    fn wake(&self, session_id: &str, cx: &mut App) {
        super::expanded_card::wake_card_terminal(session_id, cx);
    }

    /// XCARD-017: a click on the title bar focuses the terminal.
    fn focus_terminal(&self, session_id: &str, window: &mut Window, cx: &mut App) {
        // BOARD-042, XCARD-017: `focus` needs no board (only `element` does).
        CardTerminals::new("", 1.0, cx).focus(session_id, window, cx);
    }
}

/// NOTEUI-018, NOTEUI-019: who signs a note, from the board's cards and the
/// projects.
fn signature_for(note: &BoardNote, board: &BoardCx, cx: &App) -> Option<super::present::Signature> {
    note.from.as_ref()?;
    let _ = board;
    let state = app_state(cx)?;
    let state = state.read(cx);
    let cards = state.board.as_ref().map_or(&[][..], |b| &b.cards[..]);
    super::present::signature(note, cards, &state.projects)
}
