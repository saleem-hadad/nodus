//! Lineage edges, the dashed view edge and the dotted line between sessions
//! that message each other (BOARD-025…BOARD-031, BOARD-184,
//! `LineageEdge.tsx`): which edges exist, where they attach and their
//! Bézier path (React Flow's `getBezierPath`, curvature 0.25).

use std::collections::HashSet;

use nodus_board::geometry::Point;
use nodus_domain::{BoardSnapshot, Rect, SessionStatus};

/// What an edge joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// From a session to one started from it (BOARD-025).
    Lineage,
    /// From a view to a session it started (`view-edge`, BOARD-027).
    View,
    /// Between two sessions that messaged each other (BOARD-184).
    Talk,
}

/// One edge, by node ids.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    /// Dashes moving: the child is working (lineage), always (view edge).
    pub animated: bool,
    pub kind: EdgeKind,
}

impl Edge {
    /// Joins these two nodes, either way round.
    pub fn joins(&self, a: &str, b: &str) -> bool {
        (self.source == a && self.target == b) || (self.source == b && self.target == a)
    }
}

/// The edges for a board (BOARD-025…BOARD-027, BOARD-184): lineage between
/// active cards, each active card a view started back to that view, then a
/// line between active cards that messaged each other.
pub fn edges(board: &BoardSnapshot) -> Vec<Edge> {
    let active: Vec<&str> = board
        .cards
        .iter()
        .filter(|c| !c.node.archived)
        .map(|c| c.node.session_id.as_str())
        .collect();
    let mut out = Vec::new();
    for child in &board.cards {
        if !active.contains(&child.node.session_id.as_str()) {
            continue;
        }
        for parent in child.node.parents.iter().flatten() {
            if active.contains(&parent.as_str()) {
                out.push(Edge {
                    id: format!("{parent}->{}", child.node.session_id),
                    source: parent.clone(),
                    target: child.node.session_id.clone(),
                    // BOARD-026: dashes march while the child works.
                    animated: child.status == SessionStatus::Working,
                    kind: EdgeKind::Lineage,
                });
            }
        }
    }
    for card in &board.cards {
        let Some(view) = card.node.from_view.as_deref() else {
            continue;
        };
        if board.views.iter().any(|v| v.id == view) && active.contains(&card.node.session_id.as_str()) {
            out.push(Edge {
                id: format!("{view}->{}", card.node.session_id),
                source: view.to_owned(),
                target: card.node.session_id.clone(),
                animated: true,
                kind: EdgeKind::View,
            });
        }
    }
    // BOARD-184: one line a pair, from the first to message the other. A
    // pair lineage already joins has its messages run along that.
    let mut pairs: HashSet<(&str, &str)> = HashSet::new();
    for talk in &board.talks {
        let (from, to) = (talk.from.as_str(), talk.to.as_str());
        if !active.contains(&from) || !active.contains(&to) {
            continue;
        }
        let pair = if from < to { (from, to) } else { (to, from) };
        if pairs.contains(&pair) || out.iter().any(|e| e.kind == EdgeKind::Lineage && e.joins(from, to)) {
            continue;
        }
        pairs.insert(pair);
        out.push(Edge {
            id: format!("{}<->{}", pair.0, pair.1),
            source: from.to_owned(),
            target: to.to_owned(),
            animated: false,
            kind: EdgeKind::Talk,
        });
    }
    out
}

/// Which side of a node an edge leaves from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// The midpoint of the side of `from` that faces `to` (BOARD-029).
pub fn facing_side(from: Rect, to: Rect) -> (Point, Side) {
    let (ax, ay) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
    let (bx, by) = (to.x + to.w / 2.0, to.y + to.h / 2.0);
    let (dx, dy) = (bx - ax, by - ay);
    if dx.abs() / from.w > dy.abs() / from.h {
        if dx > 0.0 {
            (
                Point {
                    x: ax + from.w / 2.0,
                    y: ay,
                },
                Side::Right,
            )
        } else {
            (
                Point {
                    x: ax - from.w / 2.0,
                    y: ay,
                },
                Side::Left,
            )
        }
    } else if dy > 0.0 {
        (
            Point {
                x: ax,
                y: ay + from.h / 2.0,
            },
            Side::Bottom,
        )
    } else {
        (
            Point {
                x: ax,
                y: ay - from.h / 2.0,
            },
            Side::Top,
        )
    }
}

fn control_offset(distance: f64, curvature: f64) -> f64 {
    if distance >= 0.0 {
        0.5 * distance
    } else {
        curvature * 25.0 * (-distance).sqrt()
    }
}

pub(crate) fn control(side: Side, p: Point, other: Point) -> Point {
    const C: f64 = 0.25;
    match side {
        Side::Left => Point {
            x: p.x - control_offset(p.x - other.x, C),
            y: p.y,
        },
        Side::Right => Point {
            x: p.x + control_offset(other.x - p.x, C),
            y: p.y,
        },
        Side::Top => Point {
            x: p.x,
            y: p.y - control_offset(p.y - other.y, C),
        },
        Side::Bottom => Point {
            x: p.x,
            y: p.y + control_offset(other.y - p.y, C),
        },
    }
}

/// The edge's cubic Bézier between two node rects (flow units): start,
/// first control, second control, end (BOARD-030).
pub fn path(source: Rect, target: Rect) -> [Point; 4] {
    let (s, s_side) = facing_side(source, target);
    let (t, t_side) = facing_side(target, source);
    [s, control(s_side, s, t), control(t_side, t, s), t]
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 288.0,
        h: 216.0,
    };

    fn board(talks: &serde_json::Value) -> BoardSnapshot {
        let card = |id: &str, archived: bool, parents: serde_json::Value| {
            serde_json::json!({ "sessionId": id, "x": 0.0, "y": 0.0, "archived": archived, "parents": parents, "createdAt": 0.0,
                "status": "done", "live": false })
        };
        serde_json::from_value(serde_json::json!({
            "project": { "id": "p", "name": "p", "repoPath": "/r", "color": "blue", "createdAt": 0.0 },
            "cards": [
                card("a", false, serde_json::Value::Null),
                card("b", false, serde_json::json!(["a"])),
                card("c", false, serde_json::Value::Null),
                card("gone", true, serde_json::Value::Null),
            ],
            "archive": { "x": 0.0, "y": 0.0, "w": 344.0, "h": 362.0 },
            "sections": [], "notes": [], "views": [], "ink": [],
            "talks": talks,
        }))
        .unwrap()
    }

    #[test]
    fn draws_one_line_between_sessions_that_talked() {
        let talk = |id: &str, from: &str, to: &str| serde_json::json!({ "id": id, "from": from, "to": to, "at": 0.0 });
        let out = edges(&board(&serde_json::json!([
            talk("1", "c", "a"),
            talk("2", "a", "c"),
            // Lineage joins a and b already.
            talk("3", "b", "a"),
            // Archived.
            talk("4", "gone", "c"),
        ])));
        let ids: Vec<(&str, EdgeKind)> = out.iter().map(|e| (e.id.as_str(), e.kind)).collect();
        assert_eq!(ids, vec![("a->b", EdgeKind::Lineage), ("a<->c", EdgeKind::Talk)]);
        let talk = &out[1];
        assert_eq!((talk.source.as_str(), talk.target.as_str()), ("c", "a"));
        assert!(talk.joins("a", "c") && talk.joins("c", "a") && !talk.joins("a", "b"));
    }

    #[test]
    fn attaches_to_the_facing_sides() {
        let right = Rect { x: 600.0, ..A };
        assert_eq!(facing_side(A, right), (Point { x: 288.0, y: 108.0 }, Side::Right));
        assert_eq!(facing_side(right, A), (Point { x: 600.0, y: 108.0 }, Side::Left));
        let below = Rect {
            x: 100.0,
            y: 400.0,
            ..A
        };
        assert_eq!(facing_side(A, below).1, Side::Bottom);
        assert_eq!(facing_side(below, A).1, Side::Top);
    }

    #[test]
    fn bends_like_react_flows_bezier() {
        let right = Rect { x: 600.0, ..A };
        let [s, c1, c2, t] = path(A, right);
        assert_eq!((s, t), (Point { x: 288.0, y: 108.0 }, Point { x: 600.0, y: 108.0 }));
        assert_eq!(c1, Point { x: 444.0, y: 108.0 });
        assert_eq!(c2, Point { x: 444.0, y: 108.0 });
        // An end behind its side curves out by 0.25 × 25 × √d.
        assert_eq!(
            control(Side::Right, Point { x: 100.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }).x,
            162.5
        );
    }
}
