//! Goals on the canvas (spec 21): their menu, opening their session, the
//! New goal dialog's requests, and moving their tiles.

use gpui::{Context, Task, WeakEntity};
use nodus_board::geometry::Point;
use nodus_domain::{BoardGoal, GoalStatus};
use serde_json::json;

use super::canvas::BoardCanvas;
use crate::components::context_menu::MenuItem;
use crate::components::icon::Icon;

/// What the New goal dialog was opened for (GOAL-010…GOAL-013): where the
/// tile goes, and the session to make a goal's, with its title.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalAsk {
    pub at: Option<Point>,
    pub session: Option<(String, String)>,
}

/// What a menu row does to the canvas.
type Command = Box<dyn Fn(&mut BoardCanvas, &mut Context<'_, BoardCanvas>)>;

impl BoardCanvas {
    /// GOAL-010, GOAL-012, GOAL-013: the New goal dialog.
    pub(crate) fn open_goal_dialog(
        &mut self,
        at: Option<Point>,
        session: Option<(String, String)>,
        cx: &mut Context<'_, Self>,
    ) {
        self.goal_dialog = Some(GoalAsk { at, session });
        cx.notify();
    }

    pub(crate) fn close_goal_dialog(&mut self, cx: &mut Context<'_, Self>) {
        self.goal_dialog = None;
        cx.notify();
    }

    /// GOAL-011: starts a goal; the board shows its tile once it's there.
    pub(crate) fn create_goal(
        &mut self,
        mission: &str,
        every: f64,
        ask: &GoalAsk,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<(), String>> {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let project_id = self.project_id.clone();
        // The tile's middle at the spot.
        let at = ask
            .at
            .map(|p| json!({ "x": p.x - BoardGoal::W / 2.0, "y": p.y - BoardGoal::H / 2.0 }));
        let raw = json!({
            "mission": mission,
            "every": every,
            "at": at,
            "sessionId": ask.session.as_ref().map(|(id, _)| id.clone()),
        });
        let state = self.state.clone();
        cx.spawn(async move |_, cx| {
            let created =
                crate::board::on_core(&rt, async move { nodus.goals().create(&project_id, &raw).await }).await;
            match created {
                Ok(goal) => {
                    cx.update(|cx| state.update(cx, |s, cx| s.show_node(goal.id, cx)));
                    Ok(())
                }
                Err(err) => Err(crate::views::reason(&err.ipc_message())),
            }
        })
    }

    /// GOAL-034: a goal's session, opened as a click on its card opens it.
    pub(crate) fn open_goal(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(goal) = self.board.as_ref().and_then(|b| b.goals.iter().find(|g| g.id == id)) else {
            return;
        };
        let session = goal.session_id.clone();
        self.open_card(&session, cx);
    }

    /// GOAL-032: a goal's menu.
    pub(crate) fn goal_menu(&self, goal: &BoardGoal, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let item = |label: &'static str, icon: Icon, run: Command| {
            let me = me.clone();
            MenuItem::item(label, icon, move |_, cx| {
                if let Some(canvas) = me.upgrade() {
                    canvas.update(cx, |c, cx| run(c, cx));
                }
            })
        };
        let id = goal.id.clone();
        let mut items = Vec::new();
        {
            let id = id.clone();
            items.push(item(
                "Open its session",
                Icon::ExternalLink,
                Box::new(move |c, cx| c.open_goal(&id, cx)),
            ));
        }
        if goal.status == GoalStatus::Active {
            let id = id.clone();
            items.push(item(
                "Check in now",
                Icon::Play,
                Box::new(move |c, cx| c.check_in_goal(&id, cx)),
            ));
        }
        items.push(MenuItem::Separator);
        match goal.status {
            GoalStatus::Active => {
                let id1 = id.clone();
                items.push(item(
                    "Pause",
                    Icon::Square,
                    Box::new(move |c, cx| c.change_goal(&id1, json!({ "status": "paused" }), cx)),
                ));
                let id2 = id.clone();
                items.push(item(
                    "Mark as reached",
                    Icon::SquareCheck,
                    Box::new(move |c, cx| c.change_goal(&id2, json!({ "status": "done" }), cx)),
                ));
            }
            _ => {
                let id = id.clone();
                items.push(item(
                    "Resume",
                    Icon::Play,
                    Box::new(move |c, cx| c.change_goal(&id, json!({ "status": "active" }), cx)),
                ));
            }
        }
        for (label, every) in [("Check in at most hourly", 60.0), ("Check in at most daily", 1440.0)] {
            if (goal.every - every).abs() > f64::EPSILON {
                let id = id.clone();
                items.push(item(
                    label,
                    Icon::Clock,
                    Box::new(move |c, cx| c.change_goal(&id, json!({ "every": every }), cx)),
                ));
            }
        }
        items.push(MenuItem::Separator);
        items.push(
            item(
                "Remove goal",
                Icon::Trash2,
                Box::new(move |c, cx| c.remove_goal(&id, cx)),
            )
            .danger(),
        );
        items
    }

    fn check_in_goal(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let (project_id, id) = (self.project_id.clone(), id.to_owned());
        cx.spawn(async move |this, cx| {
            let done = crate::board::on_core(&rt, async move { nodus.goals().check_in(&project_id, &id).await }).await;
            if let Err(err) = done {
                let _ = this.update(cx, |c, cx| c.show_toast(err.ipc_message(), cx));
            }
        })
        .detach();
    }

    fn change_goal(&mut self, id: &str, patch: serde_json::Value, cx: &mut Context<'_, Self>) {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let (project_id, id) = (self.project_id.clone(), id.to_owned());
        cx.spawn(async move |this, cx| {
            let done =
                crate::board::on_core(&rt, async move { nodus.goals().change(&project_id, &id, &patch).await }).await;
            if let Err(err) = done {
                let _ = this.update(cx, |c, cx| c.show_toast(err.ipc_message(), cx));
            }
        })
        .detach();
    }

    fn remove_goal(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let (project_id, id) = (self.project_id.clone(), id.to_owned());
        cx.spawn(async move |_, _| {
            crate::board::on_core(&rt, async move { nodus.goals().remove(&project_id, &id).await }).await;
        })
        .detach();
    }

    /// GOAL-033: goals' tiles dragged to new spots, there at once, then saved.
    pub(crate) fn move_goals(&mut self, moved: &[(String, Point)], cx: &mut Context<'_, Self>) {
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
                    if let Some(goal) = board.goals.iter_mut().find(|g| &g.id == id) {
                        (goal.x, goal.y) = (*x, *y);
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
                    let _ = nodus.goals().change(&project_id, &id, &json!({ "x": x, "y": y })).await;
                }
            })
            .await;
        })
        .detach();
    }
}
