//! Scheduled actions' tiles on the canvas (spec 20): their menu, opening the
//! last run, pausing and taking off a schedule, and moving them.

use gpui::{Context, WeakEntity};
use nodus_domain::{BoardTile, ProjectAction};
use serde_json::Value;

use super::canvas::BoardCanvas;
use crate::components::context_menu::MenuItem;
use crate::components::icon::Icon;
use nodus_board::geometry::Point;

/// What a menu row does to the canvas.
type Command = Box<dyn Fn(&mut BoardCanvas, &mut Context<'_, BoardCanvas>)>;

impl BoardCanvas {
    /// A tile's action, from the board's project.
    pub(crate) fn tile_action(&self, id: &str) -> Option<ProjectAction> {
        let board = self.board.as_ref()?;
        board.project.actions.iter().flatten().find(|a| a.id == id).cloned()
    }

    /// The live run of a tile's action, if one is on the board.
    pub(crate) fn tile_run(&self, id: &str) -> Option<String> {
        let source = self.source.as_ref()?;
        source
            .cards
            .iter()
            .find(|c| !c.node.archived && c.node.run_of.as_deref() == Some(id))
            .map(|c| c.node.session_id.clone())
    }

    /// SCHED-035: a tile's menu.
    pub(crate) fn tile_menu(&self, tile: &BoardTile, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let item = |label: &'static str, icon: Icon, run: Command| {
            let me = me.clone();
            MenuItem::item(label, icon, move |_, cx| {
                if let Some(canvas) = me.upgrade() {
                    canvas.update(cx, |c, cx| run(c, cx));
                }
            })
        };
        let Some(action) = self.tile_action(&tile.id) else {
            return Vec::new();
        };
        let paused = action.schedule.as_ref().is_some_and(|s| s.paused == Some(true));
        let id = tile.id.clone();
        let mut items = Vec::new();
        match self.tile_run(&id) {
            Some(run) => items.push(item("Show run", Icon::Eye, Box::new(move |c, cx| c.show_run(&run, cx)))),
            None => {
                let (id, name) = (id.clone(), action.name.clone());
                items.push(item(
                    "Run now",
                    Icon::Play,
                    Box::new(move |c, cx| c.run_action(id.clone(), name.clone(), cx)),
                ));
            }
        }
        if tile.last.as_ref().is_some_and(|l| !l.session_id.is_empty()) {
            let id = id.clone();
            items.push(item(
                "Open last run",
                Icon::ExternalLink,
                Box::new(move |c, cx| c.open_tile(&id, cx)),
            ));
        }
        items.push(MenuItem::Separator);
        {
            let id = id.clone();
            items.push(item(
                "Edit automation…",
                Icon::PencilLine,
                Box::new(move |c, cx| c.edit_tile_action(&id, cx)),
            ));
        }
        {
            let id = id.clone();
            let (label, icon) = if paused {
                ("Resume schedule", Icon::Play)
            } else {
                ("Pause schedule", Icon::Square)
            };
            items.push(item(
                label,
                icon,
                Box::new(move |c, cx| c.set_schedule(&id, Some(!paused), cx)),
            ));
        }
        items.push(MenuItem::Separator);
        items.push(
            item(
                "Remove schedule",
                Icon::Trash2,
                Box::new(move |c, cx| c.set_schedule(&id, None, cx)),
            )
            .danger(),
        );
        items
    }

    /// SCHED-036: a tile's last run, read in the transcript preview; the
    /// tile's dot goes.
    pub(crate) fn open_tile(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.as_ref() else { return };
        let Some(tile) = board.tiles.iter().find(|t| t.id == id) else {
            return;
        };
        let session = tile.last.as_ref().map(|l| l.session_id.clone()).unwrap_or_default();
        if tile
            .last
            .as_ref()
            .is_some_and(|l| l.done_at.is_some() && l.seen.is_none())
        {
            let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
            let (project_id, id) = (self.project_id.clone(), id.to_owned());
            self.state.update(cx, |s, cx| {
                if let Some(last) = s
                    .board
                    .as_mut()
                    .and_then(|b| b.tiles.iter_mut().find(|t| t.id == id))
                    .and_then(|t| t.last.as_mut())
                {
                    last.seen = Some(true);
                }
                cx.notify();
            });
            cx.spawn(async move |_, _| {
                crate::board::on_core(&rt, async move { nodus.board().see_tile(&project_id, &id).await }).await;
            })
            .detach();
        }
        if let Some(run) = self.tile_run(id) {
            self.show_run(&run, cx);
        } else if !session.is_empty() {
            crate::terminal::actions::open_preview(&self.state, &session, None, cx);
        }
    }

    /// A tile's run, live on the board: shown as a click on its card shows it.
    fn show_run(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        self.open_card(session_id, cx);
    }

    /// SCHED-035: `Edit action…` opens the action dialog, which the rail owns.
    fn edit_tile_action(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| {
            s.action_request = Some(crate::state::ActionRequest::EditAutomation(id.to_owned()));
            cx.notify();
        });
    }

    /// SCHED-035: pauses or resumes a tile's schedule (`Some(paused)`), or
    /// takes it off (`None`), which takes the tile off the board. The action
    /// stays in the rail.
    fn set_schedule(&mut self, id: &str, paused: Option<bool>, cx: &mut Context<'_, Self>) {
        let Some(action) = self.tile_action(id) else { return };
        let Ok(mut raw) = serde_json::to_value(&action) else {
            return;
        };
        match (paused, raw.get_mut("schedule").and_then(Value::as_object_mut)) {
            (Some(true), Some(s)) => {
                s.insert("paused".into(), Value::Bool(true));
            }
            (Some(false), Some(s)) => {
                s.shift_remove("paused");
            }
            _ => {
                if let Some(o) = raw.as_object_mut() {
                    o.shift_remove("schedule");
                }
            }
        }
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let project_id = self.project_id.clone();
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let saved = crate::board::on_core(&rt, async move { nodus.actions().save(&project_id, &raw).await }).await;
            let _ = this.update(cx, |c, cx| match saved {
                Ok(lists) => state.update(cx, |s, cx| crate::actions::set_actions(s, lists, cx)),
                Err(err) => c.show_toast(err.ipc_message(), cx),
            });
        })
        .detach();
    }

    /// SCHED-032: while the board has tiles it's drawn again every 20 s, so
    /// their rings fill and their next run counts down.
    pub(crate) fn keep_tiles_ticking(&mut self, cx: &mut Context<'_, Self>) {
        let has_tiles = self.board.as_ref().is_some_and(|b| !b.tiles.is_empty());
        if !has_tiles {
            self.tile_tick = None;
            return;
        }
        if self.tile_tick.is_some() {
            return;
        }
        self.tile_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(20)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    /// SCHED-045, SCHED-048: the action dialog as a new automation, its tile
    /// at `at` (its top left, a tile's middle at the point) when given.
    pub(crate) fn new_automation(&mut self, at: Option<Point>, cx: &mut Context<'_, Self>) {
        let at = at.map(|p| {
            (
                nodus_domain::js::round(p.x - BoardTile::W / 2.0),
                nodus_domain::js::round(p.y - BoardTile::H / 2.0),
            )
        });
        self.state.update(cx, |s, cx| {
            s.action_request = Some(crate::state::ActionRequest::NewAutomation(at));
            cx.notify();
        });
    }

    /// SCHED-033: tiles dragged to new spots, there at once, then saved.
    pub(crate) fn move_tiles(&mut self, moved: &[(String, Point)], cx: &mut Context<'_, Self>) {
        if moved.is_empty() {
            return;
        }
        let moved: Vec<(String, f64, f64)> = moved
            .iter()
            .map(|(id, p)| (id.clone(), nodus_domain::js::round(p.x), nodus_domain::js::round(p.y)))
            .collect();
        self.state.update(cx, |s, cx| {
            if let Some(board) = s.board.as_mut() {
                for (id, x, y) in &moved {
                    if let Some(tile) = board.tiles.iter_mut().find(|t| &t.id == id) {
                        (tile.x, tile.y) = (*x, *y);
                    }
                }
            }
            cx.notify();
        });
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let project_id = self.project_id.clone();
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, async move {
                for (id, x, y) in moved {
                    nodus.board().move_tile(&project_id, &id, x, y).await;
                }
            })
            .await;
        })
        .detach();
    }
}
