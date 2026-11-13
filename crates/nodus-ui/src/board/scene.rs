//! The board's scene: which nodes React Flow drew, where, how big, in which
//! order and with which flags. A port of `buildNodes` (BoardView.tsx) with
//! `buildSectionNodes`, `buildPadNodes`, `buildNoteNodes` and
//! `buildViewNodes` (BOARD-007…BOARD-024): rebuilt from each board snapshot,
//! keeping each node's local state (selection) and, for nodes being dragged,
//! resized or held busy, their live position.

use std::collections::{HashMap, HashSet};

use nodus_board::geometry::{CARD_H, NOTE_H, NOTE_W, Point, Size, note_pad_rect, note_size, view_size};
use nodus_domain::{BoardSnapshot, Rect, SessionStatus};

use super::layout::{
    ARCHIVE_ID, NOTEPAD_ID, STACK_DEPTH, card_size, is_expanded, pad_sheet_spot, section_status, section_views,
    split_notes, stack_result_spot, stack_results_band, stack_top,
};
use super::nodes::{StackPlace, StackStyle};

/// How long a card keeps flying after the stack deals it out or takes it back.
pub const FLIGHT_MS: f64 = 700.0;
/// Z-order of the open archive panel, above every card on the board.
pub const RAISED_Z: i32 = 1500;
/// How far a result cut off at the panel's edge still shows its shadow.
pub const CLIP_BLEED: f64 = 24.0;
/// What a selected node's z-index gains (`elevateNodesOnSelect`).
pub const SELECTED_Z: i32 = 1000;

/// What a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Archive {
        count: usize,
    },
    Pad,
    /// Index into the board's sections, its member count and status.
    Section {
        index: usize,
        count: usize,
        status: Option<SessionStatus>,
    },
    /// Index into the board's cards.
    Card {
        index: usize,
        stack: Option<StackStyle>,
        matched: Option<Vec<usize>>,
    },
    /// Index into the board's views.
    View {
        index: usize,
    },
    /// Index into the board's notes; the pad's top sheet has none.
    Note {
        index: Option<usize>,
    },
    /// Index into the board's shapes (SHAPE-001).
    Shape {
        index: usize,
    },
}

/// One node as React Flow held it.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    /// Top-left, flow units.
    pub position: Point,
    pub size: Size,
    pub hidden: bool,
    /// The node's own z-index (selection adds [`SELECTED_Z`]).
    pub z: i32,
    pub draggable: bool,
    pub selectable: bool,
    pub selected: bool,
    /// Being dragged right now.
    pub dragging: bool,
    /// A result cut off at the panel's edges: (top, bottom) insets in flow
    /// units, negative to let the shadow bleed (`clip-path: inset(…)`).
    pub clip: Option<(f64, f64)>,
}

impl FlowNode {
    /// z-index as drawn.
    pub fn z_drawn(&self) -> i32 {
        self.z + if self.selected { SELECTED_Z } else { 0 }
    }

    pub fn rect(&self) -> Rect {
        Rect {
            x: self.position.x,
            y: self.position.y,
            w: self.size.w,
            h: self.size.h,
        }
    }

    pub fn is_card(&self) -> bool {
        matches!(self.kind, NodeKind::Card { .. })
    }

    pub fn card_index(&self) -> Option<usize> {
        match self.kind {
            NodeKind::Card { index, .. } => Some(index),
            _ => None,
        }
    }

    pub fn is_draft(&self) -> bool {
        matches!(self.kind, NodeKind::Note { index: None })
    }

    pub fn shape_index(&self) -> Option<usize> {
        match self.kind {
            NodeKind::Shape { index } => Some(index),
            _ => None,
        }
    }
}

/// The archive stack as laid out for one build.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StackView {
    /// Cards fanned out of the stack, in slot order, with the title
    /// characters the search matched.
    pub results: Vec<(String, Option<Vec<usize>>)>,
    /// Archived cards left in the stack, top first.
    pub pile: Vec<String>,
    pub open: bool,
    /// The panel floats above the board (while open, and while closing).
    pub raised: bool,
    /// The stack (or the open panel) as drawn.
    pub zone: Rect,
    pub scroll: f64,
    pub scrollable: bool,
}

/// A card mid-animation (`Flight` in BoardView.tsx).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flight {
    /// Keep drawing a card that has gone under the stack until it lands.
    Sink { until: f64 },
    /// A card that was hidden in the stack plays its fly-out from the stack top.
    Rise { until: f64, from: Point },
}

impl Flight {
    pub fn until(&self) -> f64 {
        match *self {
            Self::Sink { until } | Self::Rise { until, .. } => until,
        }
    }
}

/// What the canvas holds besides the snapshot.
pub struct BuildInput<'a> {
    pub board: &'a BoardSnapshot,
    /// The scene as last built, with its live positions and selection.
    pub prev: &'a [FlowNode],
    /// Nodes that keep their live position while the store changes under them.
    pub busy: &'a HashSet<String>,
    pub archive_busy: bool,
    /// Live boxes of nodes being resized.
    pub held: &'a HashMap<String, Rect>,
    pub stack: &'a StackView,
    pub flights: &'a mut HashMap<String, Flight>,
    /// Milliseconds, on the canvas's monotonic clock.
    pub now: f64,
    pub draft_id: &'a str,
}

/// `buildNodes` (BOARD-007): archive, pad, sections, cards, views, shapes
/// (SHAPE-001), free notes, the pad's top sheet.
pub fn build_nodes(input: BuildInput<'_>) -> Vec<FlowNode> {
    let BuildInput {
        board,
        prev,
        busy,
        archive_busy,
        held,
        stack,
        flights,
        now,
        draft_id,
    } = input;
    let prev_by_id: HashMap<&str, &FlowNode> = prev.iter().map(|n| (n.id.as_str(), n)).collect();
    let archived_count = board.cards.iter().filter(|c| c.node.archived).count();
    let selected_of = |id: &str| prev_by_id.get(id).is_some_and(|p| p.selected);
    let dragging_of = |id: &str| prev_by_id.get(id).is_some_and(|p| p.dragging);

    let mut out = Vec::new();

    // BOARD-008: the archive frame.
    let pa = prev_by_id.get(ARCHIVE_ID);
    let keep_archive = pa.is_some_and(|p| archive_busy || p.dragging);
    out.push(FlowNode {
        id: ARCHIVE_ID.into(),
        kind: NodeKind::Archive { count: archived_count },
        position: match pa {
            Some(p) if keep_archive => p.position,
            _ => Point {
                x: stack.zone.x,
                y: stack.zone.y,
            },
        },
        size: Size {
            w: stack.zone.w,
            h: stack.zone.h,
        },
        hidden: false,
        z: if stack.raised { RAISED_Z } else { -1 },
        draggable: true,
        selectable: false,
        selected: false,
        dragging: dragging_of(ARCHIVE_ID),
        clip: None,
    });

    // BOARD-021: the pad.
    let pad = note_pad_rect(board.archive);
    let pp = prev_by_id.get(NOTEPAD_ID);
    out.push(FlowNode {
        id: NOTEPAD_ID.into(),
        kind: NodeKind::Pad,
        position: match pp {
            Some(p) if busy.contains(NOTEPAD_ID) => p.position,
            _ => Point { x: pad.x, y: pad.y },
        },
        size: Size { w: pad.w, h: pad.h },
        hidden: false,
        z: -1,
        draggable: false,
        selectable: false,
        selected: false,
        dragging: false,
        clip: None,
    });

    // BOARD-020: sections, oldest first.
    for (index, view) in section_views(&board.sections, &board.cards).into_iter().enumerate() {
        let id = view.section.id.clone();
        let p = prev_by_id.get(id.as_str());
        let held_here = p.is_some_and(|p| p.dragging || busy.contains(&id));
        let status = section_status(view.members.iter().map(|&i| &board.cards[i]));
        out.push(FlowNode {
            kind: NodeKind::Section {
                index,
                count: view.members.len(),
                status,
            },
            position: match p {
                Some(p) if held_here => p.position,
                _ => Point {
                    x: view.frame.x,
                    y: view.frame.y,
                },
            },
            size: Size {
                w: view.frame.w,
                h: view.frame.h,
            },
            hidden: false,
            z: -2,
            draggable: true,
            selectable: false,
            selected: false,
            dragging: dragging_of(&id),
            clip: None,
            id,
        });
    }

    // BOARD-009…BOARD-019: cards.
    let top = stack_top(board.archive);
    let (band_top, band_bottom) = stack_results_band(board.archive);
    let base = if stack.raised { RAISED_Z } else { 0 };
    for (index, card) in board.cards.iter().enumerate() {
        let id = card.node.session_id.as_str();
        let p = prev_by_id.get(id).copied();
        let folded = card.node.archived;
        let result = if folded {
            stack.results.iter().position(|(rid, _)| rid == id)
        } else {
            None
        };
        let matched = result.and_then(|slot| stack.results[slot].1.clone());
        // BOARD-011: depth is the place in the pile (results left out).
        let depth = stack.pile.iter().position(|pid| pid == id).unwrap_or(0);
        let live = held.get(id);
        let held_here = p.is_some_and(|p| p.dragging || busy.contains(id)) || live.is_some();
        let size = live.map_or_else(|| card_size(card), |r| Size { w: r.w, h: r.h });

        let mut position = Point {
            x: card.node.x,
            y: card.node.y,
        };
        // BOARD-010: the pile sits at the stack top whatever was stored;
        // results at their slots; a dragged, resized or busy card stays put.
        if let Some(slot) = result {
            position = stack_result_spot(board.archive, slot, stack.scroll);
        } else if folded {
            position = top;
        }
        if held_here {
            position = live.map_or_else(|| p.map_or(position, |p| p.position), |r| Point { x: r.x, y: r.y });
        }

        // BOARD-014: results past the band aren't drawn, ones across its edges are cut.
        let mut offscreen = false;
        let mut clip = None;
        if result.is_some() && !held_here {
            let cut_top = band_top - position.y;
            let cut_bottom = position.y + CARD_H - band_bottom;
            if cut_top >= CARD_H || cut_bottom >= CARD_H {
                offscreen = true;
            } else if cut_top > 0.0 || cut_bottom > 0.0 {
                clip = Some((cut_top.max(-CLIP_BLEED), cut_bottom.max(-CLIP_BLEED)));
            }
        }

        // BOARD-012, BOARD-013: deep pile cards hide, sinking first; cards
        // leaving the depths rise out of the stack.
        let flying = |flights: &mut HashMap<String, Flight>, sink: bool| {
            let f = flights.get(id).copied();
            let kind_ok = |f: &Flight| matches!((f, sink), (Flight::Sink { .. }, true) | (Flight::Rise { .. }, false));
            if let Some(f) = f
                && kind_ok(&f)
                && f.until() <= now
            {
                flights.remove(id);
            }
            f.is_some_and(|f| kind_ok(&f) && f.until() > now)
        };
        let hidden;
        if folded && result.is_none() && depth >= STACK_DEPTH {
            if matches!(flights.get(id), Some(Flight::Sink { .. })) {
                hidden = !flying(flights, true);
            } else if p.is_some_and(|p| !p.hidden) {
                flights.insert(id.to_owned(), Flight::Sink { until: now + FLIGHT_MS });
                hidden = false;
            } else {
                hidden = true;
            }
        } else {
            if matches!(flights.get(id), Some(Flight::Sink { .. })) {
                flights.remove(id);
            }
            hidden = offscreen;
            let was_result = p.is_some_and(|p| {
                matches!(
                    p.kind,
                    NodeKind::Card {
                        stack: Some(StackStyle {
                            place: StackPlace::Result { .. },
                            ..
                        }),
                        ..
                    }
                )
            });
            if p.is_some_and(|p| p.hidden) && !offscreen && !was_result {
                flights.insert(
                    id.to_owned(),
                    Flight::Rise {
                        until: now + FLIGHT_MS,
                        from: Point {
                            x: top.x - position.x,
                            y: top.y - position.y,
                        },
                    },
                );
            }
        }
        let rise = if flying(flights, false) {
            match flights.get(id) {
                Some(Flight::Rise { from, .. }) => Some(*from),
                _ => None,
            }
        } else {
            None
        };

        let stack_style = folded.then(|| StackStyle {
            place: match result {
                Some(slot) => StackPlace::Result { slot },
                None => StackPlace::Pile { depth },
            },
            rise: rise.map(|r| gpui::point(r.x as f32, r.y as f32)),
            nowheel: result.is_some() && stack.scrollable,
        });

        // BOARD-016.
        let z = if !folded {
            i32::from(is_expanded(card))
        } else if result.is_some() {
            base + 10
        } else {
            base + (STACK_DEPTH as i32 - depth as i32).max(0)
        };
        // BOARD-017: only the top card and the results can be grabbed.
        let draggable = !folded || result.is_some() || depth == 0;
        let selectable = !folded || result.is_some();
        out.push(FlowNode {
            id: id.to_owned(),
            kind: NodeKind::Card {
                index,
                stack: stack_style,
                matched,
            },
            position,
            size,
            hidden,
            z,
            draggable,
            selectable,
            selected: selectable && selected_of(id),
            dragging: dragging_of(id),
            clip,
        });
    }

    // BOARD-023: views, then free notes.
    for (index, view) in board.views.iter().enumerate() {
        let p = prev_by_id.get(view.id.as_str());
        let live = held.get(&view.id);
        let held_here = p.is_some_and(|p| p.dragging || busy.contains(&view.id));
        let size = live.map_or_else(|| view_size(view), |r| Size { w: r.w, h: r.h });
        out.push(FlowNode {
            id: view.id.clone(),
            kind: NodeKind::View { index },
            position: match (live, p) {
                (Some(r), _) => Point { x: r.x, y: r.y },
                (None, Some(p)) if held_here => p.position,
                _ => Point { x: view.x, y: view.y },
            },
            size,
            hidden: false,
            z: 0,
            draggable: true,
            selectable: true,
            selected: selected_of(&view.id),
            dragging: dragging_of(&view.id),
            clip: None,
        });
    }
    // SHAPE-001: shapes, oldest first, over the views and under the notes;
    // stickers with the notes, so one stuck on a card shows over it.
    for (index, shape) in board.shapes.iter().enumerate() {
        let p = prev_by_id.get(shape.id.as_str());
        let live = held.get(&shape.id);
        let held_here = p.is_some_and(|p| p.dragging || busy.contains(&shape.id));
        out.push(FlowNode {
            id: shape.id.clone(),
            kind: NodeKind::Shape { index },
            position: match (live, p) {
                (Some(r), _) => Point { x: r.x, y: r.y },
                (None, Some(p)) if held_here => p.position,
                _ => Point { x: shape.x, y: shape.y },
            },
            size: live.map_or(Size { w: shape.w, h: shape.h }, |r| Size { w: r.w, h: r.h }),
            hidden: false,
            z: if shape.kind == nodus_domain::ShapeKind::Sticker {
                2
            } else {
                0
            },
            draggable: true,
            selectable: true,
            selected: selected_of(&shape.id),
            dragging: dragging_of(&shape.id),
            clip: None,
        });
    }
    let (free, _) = split_notes(&board.notes, &board.cards);
    for note in free {
        let index = board.notes.iter().position(|n| std::ptr::eq(n, note));
        let p = prev_by_id.get(note.id.as_str());
        let live = held.get(&note.id);
        let held_here = p.is_some_and(|p| p.dragging || busy.contains(&note.id));
        let size = live.map_or_else(|| note_size(note), |r| Size { w: r.w, h: r.h });
        out.push(FlowNode {
            id: note.id.clone(),
            kind: NodeKind::Note { index },
            position: match (live, p) {
                (Some(r), _) => Point { x: r.x, y: r.y },
                (None, Some(p)) if held_here => p.position,
                _ => Point { x: note.x, y: note.y },
            },
            size,
            hidden: false,
            z: 2,
            draggable: true,
            selectable: true,
            selected: selected_of(&note.id),
            dragging: dragging_of(&note.id),
            clip: None,
        });
    }

    // BOARD-022: the pad's top sheet.
    let dp = prev_by_id.get(draft_id);
    let spot = pad_sheet_spot(pad);
    out.push(FlowNode {
        id: draft_id.to_owned(),
        kind: NodeKind::Note { index: None },
        position: match dp {
            Some(p) if p.dragging || busy.contains(draft_id) => p.position,
            _ => spot,
        },
        size: Size { w: NOTE_W, h: NOTE_H },
        hidden: false,
        z: 2,
        draggable: true,
        selectable: false,
        selected: false,
        dragging: dragging_of(draft_id),
        clip: None,
    });
    out
}

/// When the next flight lands, if any (BOARD-024): rebuild then (+20ms).
pub fn next_landing(flights: &HashMap<String, Flight>) -> Option<f64> {
    flights.values().map(Flight::until).reduce(f64::min)
}

/// The order nodes are drawn in: by z-index (selection included), then
/// document order. Returns indices into `nodes`, hidden ones left out.
pub fn paint_order(nodes: &[FlowNode]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..nodes.len()).filter(|&i| !nodes[i].hidden).collect();
    order.sort_by_key(|&i| nodes[i].z_drawn());
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::layout::{stack_rank, stack_zone};
    use nodus_domain::{BoardNode, BoardSection, FolderColor, Project, SessionCard};
    use serde_json::Map;

    fn card(id: &str, x: f64, y: f64, archived: bool, created: f64) -> SessionCard {
        SessionCard {
            node: BoardNode {
                session_id: id.into(),
                kind: None,
                x,
                y,
                archived,
                title: None,
                parents: None,
                created_by_app: None,
                run_of: None,
                expanded: None,
                view_of: None,
                auto_mode: None,
                from_view: None,
                created_at: created,
                w: None,
                h: None,
                archived_at: None,
                section_id: None,
                extra: Map::new(),
            },
            meta: None,
            status: SessionStatus::Done,
            live: false,
            agents: None,
            agent_count: None,
            usage: None,
        }
    }

    fn board(cards: Vec<SessionCard>) -> BoardSnapshot {
        BoardSnapshot {
            project: Project {
                id: "p".into(),
                name: "p".into(),
                repo_path: "/r".into(),
                color: FolderColor::Blue,
                created_at: 0.0,
                opened_at: None,
                group_id: None,
                actions: None,
                focused: None,
                archived: None,
                server: None,
                extra: Map::new(),
            },
            cards,
            archive: Rect {
                x: 0.0,
                y: 0.0,
                w: 344.0,
                h: 362.0,
            },
            sections: vec![],
            notes: vec![],
            views: vec![],
            ink: vec![],
            shapes: vec![],
            links: vec![],
            viewport: None,
            offline: false,
        }
    }

    fn closed_stack(b: &BoardSnapshot) -> StackView {
        let mut archived: Vec<&SessionCard> = b.cards.iter().filter(|c| c.node.archived).collect();
        archived.sort_by(|a, b| stack_rank(b).partial_cmp(&stack_rank(a)).unwrap());
        StackView {
            results: vec![],
            pile: archived.iter().map(|c| c.node.session_id.clone()).collect(),
            open: false,
            raised: false,
            zone: stack_zone(b.archive, None),
            scroll: 0.0,
            scrollable: false,
        }
    }

    fn build(
        b: &BoardSnapshot,
        prev: &[FlowNode],
        stack: &StackView,
        flights: &mut HashMap<String, Flight>,
        now: f64,
    ) -> Vec<FlowNode> {
        build_nodes(BuildInput {
            board: b,
            prev,
            busy: &HashSet::new(),
            archive_busy: false,
            held: &HashMap::new(),
            stack,
            flights,
            now,
            draft_id: "draft",
        })
    }

    fn get<'a>(nodes: &'a [FlowNode], id: &str) -> &'a FlowNode {
        nodes.iter().find(|n| n.id == id).unwrap()
    }

    #[test]
    fn orders_the_scene_as_0_9_4_does() {
        let mut b = board(vec![card("a", 500.0, 0.0, false, 0.0)]);
        b.sections.push(BoardSection {
            id: "s".into(),
            name: String::new(),
            color: FolderColor::Blue,
            x: 900.0,
            y: 0.0,
            created_at: 0.0,
            extra: Map::new(),
        });
        let stack = closed_stack(&b);
        let nodes = build(&b, &[], &stack, &mut HashMap::new(), 0.0);
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, [ARCHIVE_ID, NOTEPAD_ID, "s", "a", "draft"]);
        assert_eq!(get(&nodes, NOTEPAD_ID).position, Point { x: 0.0, y: 386.0 });
        assert_eq!(get(&nodes, "draft").position, Point { x: 64.0, y: 438.0 });
        assert_eq!(get(&nodes, "s").z, -2);
    }

    #[test]
    fn piles_archived_cards_at_the_stack_top_and_hides_the_deep_ones() {
        let cards: Vec<SessionCard> = (0..5)
            .map(|i| card(&format!("c{i}"), 900.0, 900.0, true, f64::from(i)))
            .collect();
        let b = board(cards);
        let stack = closed_stack(&b);
        let nodes = build(&b, &[], &stack, &mut HashMap::new(), 0.0);
        // Newest first: c4 is the top card.
        let top = get(&nodes, "c4");
        assert_eq!(top.position, Point { x: 28.0, y: 52.0 });
        assert_eq!(
            (top.z, top.draggable, top.selectable, top.hidden),
            (3, true, false, false)
        );
        let second = get(&nodes, "c3");
        assert_eq!((second.z, second.draggable, second.hidden), (2, false, false));
        assert!(get(&nodes, "c1").hidden && get(&nodes, "c0").hidden);
    }

    #[test]
    fn sinks_a_card_pushed_deep_before_hiding_it() {
        let cards: Vec<SessionCard> = (0..3)
            .map(|i| card(&format!("c{i}"), 0.0, 0.0, true, f64::from(i)))
            .collect();
        let mut b = board(cards);
        let stack = closed_stack(&b);
        let mut flights = HashMap::new();
        let first = build(&b, &[], &stack, &mut flights, 0.0);
        assert!(!get(&first, "c0").hidden);
        // A newer archived card pushes c0 to depth 3.
        b.cards.push(card("c3", 0.0, 0.0, true, 10.0));
        let stack = closed_stack(&b);
        let second = build(&b, &first, &stack, &mut flights, 100.0);
        assert!(!get(&second, "c0").hidden);
        assert_eq!(next_landing(&flights), Some(800.0));
        let third = build(&b, &second, &stack, &mut flights, 820.0);
        assert!(get(&third, "c0").hidden);
        assert!(flights.is_empty());
    }

    #[test]
    fn deals_results_out_and_clips_them_at_the_band() {
        let cards: Vec<SessionCard> = (0..6)
            .map(|i| card(&format!("c{i}"), 0.0, 0.0, true, f64::from(i)))
            .collect();
        let b = board(cards);
        let mut stack = closed_stack(&b);
        stack.results = stack.pile.drain(..).map(|id| (id, None)).collect();
        stack.open = true;
        stack.raised = true;
        stack.zone = stack_zone(b.archive, Some(6));
        stack.scroll = 100.0;
        stack.scrollable = true;
        let nodes = build(&b, &[], &stack, &mut HashMap::new(), 0.0);
        let first = get(&nodes, "c5");
        assert_eq!(first.position, Point { x: 356.0, y: -48.0 });
        assert_eq!(first.clip, Some((100.0, -24.0)));
        assert_eq!((first.z, first.selectable), (RAISED_Z + 10, true));
        assert!(matches!(
            first.kind,
            NodeKind::Card {
                stack: Some(StackStyle {
                    place: StackPlace::Result { slot: 0 },
                    nowheel: true,
                    ..
                }),
                ..
            }
        ));
        assert_eq!(get(&nodes, ARCHIVE_ID).z, RAISED_Z);
    }

    #[test]
    fn keeps_selection_and_live_positions() {
        let b = board(vec![card("a", 500.0, 0.0, false, 0.0)]);
        let stack = closed_stack(&b);
        let mut prev = build(&b, &[], &stack, &mut HashMap::new(), 0.0);
        let a = prev.iter_mut().find(|n| n.id == "a").unwrap();
        a.selected = true;
        a.dragging = true;
        a.position = Point { x: 10.0, y: 20.0 };
        let next = build(&b, &prev, &stack, &mut HashMap::new(), 0.0);
        let a = get(&next, "a");
        assert!(a.selected && a.dragging);
        assert_eq!(a.position, Point { x: 10.0, y: 20.0 });
        assert_eq!(a.z_drawn(), SELECTED_Z);
    }

    #[test]
    fn raises_expanded_cards() {
        let mut c = card("a", 500.0, 0.0, false, 0.0);
        c.node.expanded = Some(true);
        let b = board(vec![c]);
        let nodes = build(&b, &[], &closed_stack(&b), &mut HashMap::new(), 0.0);
        let a = get(&nodes, "a");
        assert_eq!((a.z, a.size), (1, Size { w: 720.0, h: 480.0 }));
    }

    #[test]
    fn paints_by_z_then_document_order() {
        let b = board(vec![
            card("a", 500.0, 0.0, false, 0.0),
            card("b", 900.0, 0.0, false, 0.0),
        ]);
        let mut nodes = build(&b, &[], &closed_stack(&b), &mut HashMap::new(), 0.0);
        nodes.iter_mut().find(|n| n.id == "a").unwrap().selected = true;
        let order: Vec<&str> = paint_order(&nodes).into_iter().map(|i| nodes[i].id.as_str()).collect();
        assert_eq!(order, [ARCHIVE_ID, NOTEPAD_ID, "b", "draft", "a"]);
    }
}
