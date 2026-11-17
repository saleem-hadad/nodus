//! Scheduled actions' tiles (spec 20): one on the board for each of the
//! project's own actions that has a schedule. Its runs come out beside it,
//! and the last one's result stays on it.

use nodus_domain::{BoardNode, BoardTile, Event, Project, ProjectAction, Rect, TileRun};
use nodus_store::BoardState;
use serde_json::Map;

use crate::board::Board;
use crate::geometry::{GAP, Point, Size, card_size, dock_rect, find_free_spot, find_spot_beside};
use crate::notes::{obstacles, rightmost_card};

const TILE_SIZE: Size = Size {
    w: BoardTile::W,
    h: BoardTile::H,
};

/// The project's own actions that have a schedule (SCHED-007: global ones
/// can't).
pub fn scheduled(project: &Project) -> impl Iterator<Item = &ProjectAction> {
    project.actions.iter().flatten().filter(|a| a.schedule.is_some())
}

/// SCHED-025: where a new tile goes: beside the rightmost card, else right of
/// the dock, in the nearest free spot.
pub fn place_tile(board: &BoardState) -> Point {
    let obstacles = obstacles(board, None);
    let dock = dock_rect(board.archive);
    match rightmost_card(board) {
        Some(card) => {
            let size = card_size(Some(card));
            let anchor = Rect {
                x: card.x,
                y: card.y,
                w: size.w,
                h: size.h,
            };
            find_spot_beside(anchor, TILE_SIZE, &obstacles, Some(dock), GAP)
        }
        None => find_free_spot(
            Point {
                x: dock.x + dock.w + 80.0,
                y: dock.y,
            },
            &obstacles,
            Some(dock),
            TILE_SIZE,
        ),
    }
}

/// SCHED-024: a tile for each scheduled action and none for others. A new
/// tile's schedule is handled up to now, so nothing due before it runs.
/// Whether anything changed.
pub fn sync(board: &mut BoardState, project: &Project, now: f64) -> bool {
    let wanted: Vec<&str> = scheduled(project).map(|a| a.id.as_str()).collect();
    let before = board.tiles.values().count();
    board.tiles.retain(|t| wanted.contains(&t.id.as_str()));
    let mut changed = board.tiles.values().count() != before;
    for id in wanted {
        if board.tiles.contains_key(id) {
            continue;
        }
        let at = place_tile(board);
        board.tiles.insert(
            id.to_owned(),
            BoardTile {
                id: id.to_owned(),
                x: at.x,
                y: at.y,
                handled: now,
                last: None,
                created_at: now,
                extra: Map::new(),
            },
        );
        changed = true;
    }
    changed
}

/// SCHED-026: where a run of a tile's action goes: beside its tile, clear of
/// the board's things.
pub fn run_spot(board: &BoardState, tile: &BoardTile, node: &BoardNode) -> Point {
    let obstacles = obstacles(board, None);
    find_spot_beside(
        tile.rect(),
        card_size(Some(node)),
        &obstacles,
        Some(dock_rect(board.archive)),
        GAP,
    )
}

/// A scheduled action as the core's ticker sees it: its project, the action,
/// its tile and whether a run of it is on the board now.
#[derive(Debug, Clone)]
pub struct Scheduled {
    pub project_id: String,
    pub action: ProjectAction,
    pub tile: BoardTile,
    pub running: bool,
}

impl Board {
    /// SCHED-024: brings the project's tiles in line with its actions, after
    /// one is saved or removed, and tells the window when that changed them.
    pub fn sync_tiles(&self, project_id: &str) {
        let Ok(project) = self.project(project_id) else {
            return;
        };
        let now = self.now();
        let changed = self
            .store
            .update(|state| sync(state.board_mut(project_id), &project, now));
        if changed {
            self.events.emit(Event::BoardChanged {
                project_id: project_id.to_owned(),
            });
        }
    }

    /// SCHED-020: a schedule that was changed, or resumed, is handled up to now.
    pub fn reset_tile(&self, project_id: &str, action_id: &str) {
        let now = self.now();
        self.store.update(|state| {
            if let Some(tile) = state.board_mut(project_id).tiles.get_mut(action_id) {
                tile.handled = now;
            }
        });
    }

    /// `board:move-tile`: a tile dragged to a new spot (SCHED-033).
    pub fn move_tile(&self, project_id: &str, id: &str, x: f64, y: f64) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        self.store.update(|state| {
            if let Some(tile) = state.board_mut(project_id).tiles.get_mut(id) {
                tile.x = nodus_domain::js::round(x);
                tile.y = nodus_domain::js::round(y);
            }
        });
    }

    /// SCHED-023: a run of a tile's action started. A scheduled one hands the
    /// schedule on to the time it ran for.
    pub fn tile_started(&self, project_id: &str, action_id: &str, run: TileRun) {
        let changed = self.store.update(|state| {
            let Some(tile) = state.board_mut(project_id).tiles.get_mut(action_id) else {
                return false;
            };
            if let Some(due) = run.due {
                tile.handled = tile.handled.max(due);
            }
            tile.last = Some(run);
            true
        });
        if changed {
            self.events.emit(Event::BoardChanged {
                project_id: project_id.to_owned(),
            });
        }
    }

    /// SCHED-020: a time came while a run was still going; it's skipped.
    pub fn tile_skipped(&self, project_id: &str, action_id: &str, due: f64) {
        self.store.update(|state| {
            if let Some(tile) = state.board_mut(project_id).tiles.get_mut(action_id) {
                tile.handled = tile.handled.max(due);
            }
        });
    }

    /// SCHED-036: the user looked at a tile's result.
    pub fn see_tile(&self, project_id: &str, id: &str) {
        let changed = self.store.update(|state| {
            let last = state
                .board_mut(project_id)
                .tiles
                .get_mut(id)
                .and_then(|t| t.last.as_mut())
                .filter(|l| l.seen.is_none() && l.done_at.is_some());
            last.map(|l| l.seen = Some(true)).is_some()
        });
        if changed {
            self.events.emit(Event::BoardChanged {
                project_id: project_id.to_owned(),
            });
        }
    }

    /// Every local project's scheduled actions, for the core's ticker
    /// (SCHED-020). Archived projects' don't run.
    pub fn scheduled(&self) -> Vec<Scheduled> {
        self.store.read(|state| {
            let mut out = Vec::new();
            for project in state.projects.iter().filter(|p| p.archived != Some(true)) {
                let Some(board) = state.board(&project.id) else {
                    continue;
                };
                for action in scheduled(project) {
                    let Some(tile) = board.tiles.get(&action.id) else {
                        continue;
                    };
                    // A run left from before a restart that isn't live doesn't count.
                    let running = board.nodes.values().any(|n| {
                        !n.archived
                            && n.run_of.as_deref() == Some(action.id.as_str())
                            && self.sessions.is_live(&n.session_id)
                    });
                    out.push(Scheduled {
                        project_id: project.id.clone(),
                        action: action.clone(),
                        tile: tile.clone(),
                        running,
                    });
                }
            }
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::ActionSchedule;
    use serde_json::json;

    fn project(actions: &[serde_json::Value]) -> Project {
        serde_json::from_value(json!({
            "id": "p", "name": "p", "repoPath": "/r", "color": "blue", "createdAt": 0, "actions": actions
        }))
        .unwrap()
    }

    fn action(id: &str, scheduled: bool) -> serde_json::Value {
        let mut a = json!({ "id": id, "name": id, "icon": "zap", "description": "", "prompt": "p", "createdAt": 1 });
        if scheduled {
            a["schedule"] = json!({ "days": [1, 2, 3, 4, 5], "at": "09:00" });
        }
        a
    }

    #[test]
    fn keeps_a_tile_for_each_scheduled_action() {
        let mut board = BoardState::default();
        let p = project(&[action("a", true), action("b", false), action("c", true)]);
        assert!(sync(&mut board, &p, 50.0));
        let ids: Vec<&str> = board.tiles.values().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
        assert!(board.tiles.values().all(|t| t.handled == 50.0 && t.created_at == 50.0));
        // Two tiles don't sit on each other.
        let (a, c) = (
            board.tiles.get("a").unwrap().rect(),
            board.tiles.get("c").unwrap().rect(),
        );
        assert!(!crate::geometry::intersects(a, c, 0.0));
        // Unchanged: nothing to do.
        assert!(!sync(&mut board, &p, 60.0));
        // A schedule taken off takes its tile.
        let p = project(&[action("a", false), action("c", true)]);
        assert!(sync(&mut board, &p, 70.0));
        let ids: Vec<&str> = board.tiles.values().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c"]);
        let schedule: ActionSchedule = serde_json::from_value(json!({ "at": "09:00" })).unwrap();
        assert_eq!(schedule.days, Vec::<u8>::new());
    }

    #[test]
    fn puts_a_run_beside_its_tile() {
        let mut board = BoardState::default();
        sync(&mut board, &project(&[action("a", true)]), 1.0);
        let tile = board.tiles.get("a").unwrap().clone();
        let node: BoardNode = serde_json::from_value(json!({
            "sessionId": "s", "x": 0, "y": 0, "createdAt": 0, "archived": false
        }))
        .unwrap();
        let at = run_spot(&board, &tile, &node);
        let size = card_size(Some(&node));
        let run = Rect {
            x: at.x,
            y: at.y,
            w: size.w,
            h: size.h,
        };
        assert!(!crate::geometry::intersects(run, tile.rect(), 0.0));
        let gap = (run.x - (tile.x + BoardTile::W))
            .abs()
            .min((tile.x - (run.x + run.w)).abs());
        assert!(gap <= 2.0 * GAP + 1.0, "beside it: {run:?} {tile:?}");
    }
}
