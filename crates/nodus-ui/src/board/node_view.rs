//! Cards, notes, the archive frame and the note pad drawn as their own
//! cached views. The canvas is one view, so anything on it that moved (a
//! working session's dot breathing, a terminal's output) drew every node
//! again, each frame. Each of those is now a view whose drawing the next
//! frame reuses unless:
//!
//! - it asked to be drawn again itself (its dot's breath, an animation, its
//!   terminal's output: a view inside it, so GPUI draws it again), or
//! - what the canvas hands it changed (the card, the note, its box, the
//!   board's context), or
//! - anything else in the window changed: then every node draws again, as
//!   before, since a node can read more than it's handed (the app's state,
//!   the theme). [`partial_frame`] tells the frames where nothing did.

use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, Entity, EntityId, IntoElement, Render, StyleRefinement, Styled as _, Window,
};
use nodus_domain::{BoardNote, SessionCard};

use super::nodes::{ArchiveNode, BoardCx, CardNode, NodeRenderers, NodeState, NoteNode, PadNode, StackStyle};
use crate::components::ambient;

/// What a node draws from.
#[derive(Clone, PartialEq)]
enum Props {
    Card {
        node: NodeState,
        card: Box<SessionCard>,
        matched: Option<Vec<usize>>,
        stickers: Vec<BoardNote>,
        stack: Option<StackStyle>,
    },
    Note {
        node: NodeState,
        note: BoardNote,
        draft: bool,
    },
    Archive {
        node: NodeState,
        count: usize,
    },
    Pad {
        node: NodeState,
    },
}

/// One card, note, archive frame or note pad on the board.
#[derive(Default)]
pub struct NodeView {
    props: Option<Props>,
    board: Option<BoardCx>,
    renderers: Option<Rc<dyn NodeRenderers>>,
    /// On the canvas: off it, its ambient motion keeps still.
    visible: bool,
}

impl NodeView {
    /// Hands a card its props; whether they changed.
    pub fn set_card(&mut self, node: CardNode<'_>, board: &BoardCx, visible: bool) -> bool {
        let same = matches!(&self.props, Some(Props::Card { node: n, card, matched, stickers, stack })
            if *n == node.node
                && **card == *node.card
                && matched.as_deref() == node.matched
                && stickers.as_slice() == node.stickers
                && *stack == node.stack);
        if !same {
            self.props = Some(Props::Card {
                node: node.node,
                card: Box::new(node.card.clone()),
                matched: node.matched.map(<[usize]>::to_vec),
                stickers: node.stickers.to_vec(),
                stack: node.stack,
            });
        }
        self.set_board(board, visible) || !same
    }

    /// Hands a note its props; whether they changed.
    pub fn set_note(&mut self, node: NoteNode<'_>, board: &BoardCx, visible: bool) -> bool {
        let same = matches!(&self.props, Some(Props::Note { node: n, note, draft })
            if *n == node.node && note == node.note && *draft == node.draft);
        if !same {
            self.props = Some(Props::Note {
                node: node.node,
                note: node.note.clone(),
                draft: node.draft,
            });
        }
        self.set_board(board, visible) || !same
    }

    /// Hands the archive frame its props; whether they changed.
    pub fn set_archive(&mut self, node: ArchiveNode, board: &BoardCx, visible: bool) -> bool {
        self.set_props(
            Props::Archive {
                node: node.node,
                count: node.count,
            },
            board,
            visible,
        )
    }

    /// Hands the note pad its props; whether they changed.
    pub fn set_pad(&mut self, node: PadNode, board: &BoardCx, visible: bool) -> bool {
        self.set_props(Props::Pad { node: node.node }, board, visible)
    }

    fn set_props(&mut self, props: Props, board: &BoardCx, visible: bool) -> bool {
        let same = self.props.as_ref() == Some(&props);
        if !same {
            self.props = Some(props);
        }
        self.set_board(board, visible) || !same
    }

    fn set_board(&mut self, board: &BoardCx, visible: bool) -> bool {
        let moved = self.board.as_ref() != Some(board);
        if moved {
            self.board = Some(board.clone());
        }
        let changed = moved || self.visible != visible;
        self.visible = visible;
        changed
    }

    pub fn set_renderers(&mut self, renderers: &Rc<dyn NodeRenderers>) {
        if !self.renderers.as_ref().is_some_and(|r| Rc::ptr_eq(r, renderers)) {
            self.renderers = Some(renderers.clone());
        }
    }
}

impl Render for NodeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let (Some(props), Some(board), Some(renderers)) = (&self.props, &self.board, self.renderers.clone()) else {
            return gpui::Empty.into_any_element();
        };
        ambient::hidden(!self.visible, cx, |cx| match props {
            Props::Card {
                node,
                card,
                matched,
                stickers,
                stack,
            } => renderers.card(
                CardNode {
                    node: node.clone(),
                    card,
                    matched: matched.as_deref(),
                    stickers,
                    stack: *stack,
                },
                board,
                window,
                cx,
            ),
            Props::Note { node, note, draft } => renderers.note(
                NoteNode {
                    node: node.clone(),
                    note,
                    draft: *draft,
                },
                board,
                window,
                cx,
            ),
            Props::Archive { node, count } => renderers.archive(
                ArchiveNode {
                    node: node.clone(),
                    count: *count,
                },
                board,
                window,
                cx,
            ),
            Props::Pad { node } => renderers.pad(PadNode { node: node.clone() }, board, window, cx),
        })
    }
}

/// The node as an element: its last drawing reused when nothing it draws
/// from changed (`fresh` false), drawn now otherwise.
pub fn element(view: &Entity<NodeView>, fresh: bool) -> AnyElement {
    let view = AnyView::from(view.clone());
    if fresh {
        // Uncached, so the stale drawing isn't reused this frame; the next
        // frame caches it again.
        view.into_any_element()
    } else {
        view.cached(StyleRefinement::default().size_full()).into_any_element()
    }
}

/// Whether only the nodes moved since the last frame: everything notified
/// is a node's own view or drawn inside one (its terminal, its fields), or
/// was moved on by the ambient clock. Then a node whose props didn't change
/// can keep its drawing. Nothing notified (a forced draw) counts as more.
pub fn partial_frame(nodes: &HashSet<EntityId>, window: &Window, cx: &App) -> bool {
    let notified = window.notified_entities();
    if notified.is_empty() {
        return false;
    }
    let ticked = ambient::ticked(cx);
    notified
        .iter()
        .all(|e| ticked.is_some_and(|t| t.contains(e)) || window.view_ancestors(*e).any(|v| nodes.contains(&v)))
}
