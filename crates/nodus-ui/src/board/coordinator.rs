//! The board's coordinator on the canvas (spec 22, beta): starting one from
//! the New coordinator dialog, the members menu its dots open, and the card
//! menu's items for it and its members.

use gpui::{Context, Pixels, Point as GPoint, Task, WeakEntity};
use nodus_board::coordinator::is_member;
use nodus_domain::{CreateSessionOptions, SessionCard, SessionStatus};

use super::canvas::BoardCanvas;
use super::layout::{NEW_CARD_SIZE, spot_for_new_session};
use crate::components::context_menu::MenuItem;
use crate::components::icon::Icon;

/// An item that runs a canvas command.
fn item(
    canvas: &WeakEntity<BoardCanvas>,
    label: impl Into<gpui::SharedString>,
    icon: Icon,
    run: impl Fn(&mut BoardCanvas, &mut Context<'_, BoardCanvas>) + 'static,
) -> MenuItem {
    let canvas = canvas.clone();
    MenuItem::item(label, icon, move |_, cx| {
        if let Some(canvas) = canvas.upgrade() {
            canvas.update(cx, |c, cx| run(c, cx));
        }
    })
}

impl BoardCanvas {
    /// The board's coordinator's session, when it has one.
    pub(crate) fn coordinator_id(&self) -> Option<&str> {
        Some(self.source.as_ref()?.coordinator.as_ref()?.session_id.as_str())
    }

    /// Its title as its card shows it.
    fn coordinator_title(&self) -> String {
        self.coordinator_id()
            .and_then(|id| self.any_card(id))
            .map_or_else(|| "the coordinator".into(), crate::board::cards::present::title)
    }

    /// A card on the whole board, inside sessions included (the canvas's own
    /// list leaves them out).
    fn any_card(&self, id: &str) -> Option<&SessionCard> {
        self.source.as_ref()?.cards.iter().find(|c| c.node.session_id == id)
    }

    /// COORD-003: the coordinator's members, in board order.
    fn members(&self) -> Vec<&SessionCard> {
        let (Some(source), Some(c)) = (self.source.as_ref(), self.coordinator_id()) else {
            return Vec::new();
        };
        source.cards.iter().filter(|x| is_member(&x.node, c)).collect()
    }

    /// COORD-011: the New coordinator dialog.
    pub(crate) fn open_coordinator_dialog(&mut self, cx: &mut Context<'_, Self>) {
        self.coordinator_dialog = true;
        cx.notify();
    }

    pub(crate) fn close_coordinator_dialog(&mut self, cx: &mut Context<'_, Self>) {
        self.coordinator_dialog = false;
        cx.notify();
    }

    /// COORD-012: starts the coordinator where a new session would go, then
    /// opens it.
    pub(crate) fn create_coordinator(&mut self, prompt: &str, cx: &mut Context<'_, Self>) -> Task<Result<(), String>> {
        let dock = self.dock();
        let others = self.notes_in_the_way();
        let spot = spot_for_new_session(dock, self.cards(), NEW_CARD_SIZE, &others);
        let opts = CreateSessionOptions {
            x: spot.x.round(),
            y: spot.y.round(),
            initial_prompt: Some(prompt.to_owned()),
            auto_mode: Some(true),
            ..CreateSessionOptions::default()
        };
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let created =
                crate::board::on_core(&rt, async move { nodus.coordinator().create(&project_id, &opts).await }).await;
            match created {
                Ok(card) => {
                    let id = card.node.session_id.clone();
                    let _ = this.update(cx, |c, cx| {
                        c.state.update(cx, |s, cx| s.upsert_card(card, cx));
                        c.ensure_visible(spot, NEW_CARD_SIZE, cx);
                        c.state.update(cx, |s, cx| s.open_terminal(&id, cx));
                    });
                    Ok(())
                }
                Err(err) => Err(crate::views::reason(&err.ipc_message())),
            }
        })
    }

    /// COORD-010: `Open coordinator`, and a member's mark (COORD-053).
    pub(crate) fn show_coordinator(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.coordinator_id().map(str::to_owned) {
            self.jump_to_card(&id, None, cx);
        }
    }

    /// COORD-051: a member's row: the board moves to its card, or an inside
    /// session opens in the session window.
    fn show_member(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let inside = self.any_card(id).is_some_and(|c| c.node.inside());
        if inside {
            let id = id.to_owned();
            self.state.update(cx, |s, cx| s.open_terminal(&id, cx));
        } else {
            self.jump_to_card(id, None, cx);
        }
    }

    /// COORD-051: the members menu, opened from the coordinator's dots.
    pub(crate) fn open_members(&mut self, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        let me = cx.entity().downgrade();
        let members: Vec<(String, String, SessionStatus, bool)> = self
            .members()
            .into_iter()
            .map(|c| {
                (
                    c.node.session_id.clone(),
                    crate::board::cards::present::title(c),
                    c.status,
                    c.node.inside(),
                )
            })
            .collect();
        let mut items: Vec<MenuItem> = Vec::new();
        if members.is_empty() {
            items.push(MenuItem::Item {
                label: "No sessions yet. Start one, or ask the coordinator to.".into(),
                icon: None,
                hint: None,
                danger: false,
                disabled: true,
                on_select: std::rc::Rc::new(|_, _| {}),
            });
        }
        for (id, title, status, inside) in &members {
            let place = if *inside { "Inside" } else { "On the board" };
            let hint = if *status == SessionStatus::Waiting {
                format!("{place} · waiting for you")
            } else {
                place.to_owned()
            };
            let icon = if *inside { Icon::Eye } else { Icon::Maximize2 };
            let id = id.clone();
            items.push(item(&me, title.clone(), icon, move |c, cx| c.show_member(&id, cx)).hint(hint));
        }
        let inside: Vec<&(String, String, SessionStatus, bool)> = members.iter().filter(|m| m.3).collect();
        if !inside.is_empty() {
            items.push(MenuItem::Separator);
            for (id, title, _, _) in inside {
                let id = id.clone();
                items.push(item(
                    &me,
                    format!("Show “{title}” on board"),
                    Icon::Maximize,
                    move |c, cx| {
                        c.set_coordinator_inside(&id, false, cx);
                    },
                ));
            }
        }
        self.open_menu(at, items, cx);
    }

    /// COORD-051, COORD-054: a member taken inside, or shown on the board
    /// (where the board then moves to it).
    pub(crate) fn set_coordinator_inside(&mut self, id: &str, inside: bool, cx: &mut Context<'_, Self>) {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let (project_id, session) = (self.project_id.clone(), id.to_owned());
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let done = crate::board::on_core(&rt, {
                let session = session.clone();
                async move { nodus.coordinator().set_inside(&project_id, &session, inside) }
            })
            .await;
            let _ = this.update(cx, |c, cx| match done {
                Ok(()) if !inside => state.update(cx, |s, cx| s.show_node(session, cx)),
                Ok(()) => {}
                Err(err) => c.show_toast(crate::views::reason(&err.ipc_message()), cx),
            });
        })
        .detach();
    }

    /// COORD-054: a card taken out of the coordinator's members, or put back.
    fn set_coordinator_member(&mut self, id: &str, member: bool, cx: &mut Context<'_, Self>) {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let (project_id, session) = (self.project_id.clone(), id.to_owned());
        cx.spawn(async move |this, cx| {
            let done = crate::board::on_core(&rt, async move {
                nodus.coordinator().set_member(&project_id, &session, member)
            })
            .await;
            if let Err(err) = done {
                let _ = this.update(cx, |c, cx| c.show_toast(crate::views::reason(&err.ipc_message()), cx));
            }
        })
        .detach();
    }

    /// COORD-013: `Stop coordinating`.
    fn stop_coordinating(&mut self, cx: &mut Context<'_, Self>) {
        let (nodus, rt) = (self.nodus(cx), crate::board::core_rt(cx));
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let done = crate::board::on_core(&rt, async move { nodus.coordinator().stop(&project_id) }).await;
            if let Err(err) = done {
                let _ = this.update(cx, |c, cx| c.show_toast(crate::views::reason(&err.ipc_message()), cx));
            }
        })
        .detach();
    }

    /// COORD-013, COORD-054: a Claude card's menu items for the coordinator.
    pub(crate) fn coordinator_card_items(&self, card: &SessionCard, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let Some(c) = self.coordinator_id().map(str::to_owned) else {
            return Vec::new();
        };
        let id = card.node.session_id.clone();
        if card.node.archived || self.read_only() {
            return Vec::new();
        }
        if id == c {
            return vec![item(me, "Stop coordinating", Icon::Ban, BoardCanvas::stop_coordinating)];
        }
        let title = self.coordinator_title();
        if is_member(&card.node, &c) {
            let (a, b) = (id.clone(), id);
            return vec![
                item(me, format!("Tuck into {title}"), Icon::Network, move |c, cx| {
                    c.set_coordinator_inside(&a, true, cx);
                }),
                item(me, "Leave the coordinator", Icon::Minus, move |c, cx| {
                    c.set_coordinator_member(&b, false, cx);
                }),
            ];
        }
        if card.node.no_coordinator() {
            return vec![item(me, format!("Report to {title}"), Icon::Network, move |c, cx| {
                c.set_coordinator_member(&id, true, cx);
            })];
        }
        Vec::new()
    }
}
