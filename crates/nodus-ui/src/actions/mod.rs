//! The actions rail on the board's right edge (`ActionRail.tsx`,
//! `actions.css`, `ProjectRail.css`'s base; ACT-017, ACT-018, ACT-031,
//! ACT-039, ACT-046…ACT-066, ACT-081, ACT-082) and its dialog ([`dialog`]).
//!
//! The rail is the project rail's twin: it slides in from the right edge
//! 100 ms after the pointer reaches it and out 300 ms after it leaves. It
//! lists the project's own actions, then the global ones, and `+`. A running
//! action's session fans out beside it; while the rail is hidden a sliver on
//! the edge says something runs, red when it needs the user.
//!
//! The board view mounts it with [`action_rail`] over the board's area (the
//! rail sits at its right edge), passing [`RailHooks`] for what is the
//! board's: where a run's card would go, the board toast, and the overlay
//! flag that holds the board's shortcuts.
//!
//! [`icons`] are spec 05's lucide icons, which the git controls and the mics
//! draw too.

pub mod dialog;
pub mod icons;

use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext as _, Bounds, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled, Task, Window,
    anchored, canvas, deferred, div, point, px,
};
use nodus_actions::rail::{self as logic, HintAt, Press, Rail};
use nodus_domain::{CreateSessionOptions, Event, ProjectAction, SessionCard, SessionStatus};

use self::dialog::ActionDialogView;
use self::icons::{Glyph, glyph};
use crate::components::context_menu::{Menu, MenuItem, MenuLayer};
use crate::components::frosted::frosted;
use crate::components::modal::Modal;
use crate::components::status_dot::status_dot;
use crate::components::tween::{enter, transition};
use crate::components::{Kind, button, child_id};
use crate::format::format_duration;
use crate::parity;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, motion};

/// What the board does for the rail (0.9.4's `onRun` and `onOverlayChange`
/// props, and the board's `showToast`). Each has a stand-in for a board that
/// doesn't give one.
#[derive(Clone, Default)]
pub struct RailHooks {
    /// ACT-031: where a new session card would go (`spotForNewSession`),
    /// kept for the run in case it's taken over. Without it, the board's
    /// origin.
    pub spot: Option<Rc<dyn Fn(&mut App) -> Point<f64>>>,
    /// The board toast (09: 4.2 s, replaced by the next).
    pub toast: Option<Rc<dyn Fn(String, &mut App)>>,
    /// ACT-053: a dialog, menu or confirmation of the rail is open.
    pub overlay: Option<Rc<dyn Fn(bool, &mut App)>>,
}

impl RailHooks {
    fn toast(&self, message: String, cx: &mut App, state: &Entity<AppState>) {
        match &self.toast {
            Some(toast) => toast(message, cx),
            None => state.update(cx, |s, cx| s.show_toast(message, cx)),
        }
    }
}

/// The rail of the board `state` shows, made once per board (a fresh one for
/// each project, as 0.9.4's BoardView is keyed by project). Draws nothing
/// off a board.
pub fn action_rail(
    state: &Entity<AppState>,
    menus: &Entity<MenuLayer>,
    hooks: RailHooks,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(project_id) = state.read(cx).route.project_id().map(str::to_owned) else {
        return div().into_any_element();
    };
    let (state_for, menus_for) = (state.clone(), menus.clone());
    let rail = window.use_keyed_state(
        SharedString::from(format!("actions-rail-{project_id}")),
        cx,
        move |_, cx| cx.new(|cx| ActionRail::new(state_for, menus_for, project_id, cx)),
    );
    let rail = rail.read(cx).clone();
    rail.update(cx, |r, _| r.hooks = hooks);
    div().absolute().inset_0().child(rail).into_any_element()
}

/// A delete confirmation on show (ACT-062).
#[derive(Clone)]
struct Confirm {
    action: ProjectAction,
    title: String,
    message: &'static str,
}

pub struct ActionRail {
    state: Entity<AppState>,
    menus: Entity<MenuLayer>,
    project_id: String,
    pub hooks: RailHooks,
    rail: Rail,
    timer: Option<Task<()>>,
    /// The action whose menu is up.
    menu: Option<ProjectAction>,
    dialog: Option<Entity<ActionDialogView>>,
    confirm: Option<Confirm>,
    /// ACT-053: what the board was last told.
    overlay_told: bool,
    /// Where each button is, in window coordinates, by action id (`+` is "").
    buttons: Rc<std::cell::RefCell<std::collections::HashMap<String, Bounds<Pixels>>>>,
    /// The fan's measured height, and what it was measured for (ACT-064).
    fan_height: Rc<std::cell::Cell<Option<Pixels>>>,
    fan_for: Option<(String, Vec<String>)>,
    /// The fan's clock (`useNow`): a frame each second while it shows.
    _tick: Option<Task<()>>,
    /// The board loaded for the rail while no board view loads one.
    stand_in: StandIn,
    _events: Task<()>,
}

/// Until the board view (09) loads boards into the window's state, the rail
/// loads its board itself, once, and again when the core says it changed.
#[derive(Default, PartialEq, Eq)]
enum StandIn {
    #[default]
    Untried,
    Loading,
    Loaded,
    /// Someone else loads boards.
    Off,
}

impl ActionRail {
    fn new(state: Entity<AppState>, menus: Entity<MenuLayer>, project_id: String, cx: &mut Context<'_, Self>) -> Self {
        let mut rx = state.read(cx).nodus.subscribe();
        let events = cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if this.update(cx, |r, cx| r.on_event(event, cx)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Self {
            state,
            menus,
            project_id,
            hooks: RailHooks::default(),
            rail: Rail::default(),
            timer: None,
            menu: None,
            dialog: None,
            confirm: None,
            overlay_told: false,
            buttons: Rc::default(),
            fan_height: Rc::default(),
            fan_for: None,
            _tick: None,
            stand_in: StandIn::Untried,
            _events: events,
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<'_, Self>) {
        match event {
            // ACT-039: a finished run has left the board; say what it did.
            Event::RunFinished(run) if run.project_id == self.project_id => {
                self.state.update(cx, |s, cx| {
                    if s.terminal_session_id.as_deref() == Some(run.session_id.as_str()) {
                        s.terminal_session_id = None;
                        cx.notify();
                    }
                });
                let message = logic::finished_toast(&run.action_name, run.message.as_deref());
                self.hooks.clone().toast(message, cx, &self.state);
            }
            Event::BoardChanged { project_id } if project_id == self.project_id && self.stand_in == StandIn::Loaded => {
                self.load_board(cx);
            }
            _ => {}
        }
    }

    /// The stand-in for the store's `loadBoard` (see [`StandIn`]).
    fn ensure_board(&mut self, cx: &mut Context<'_, Self>) {
        if self.stand_in != StandIn::Untried {
            return;
        }
        let s = self.state.read(cx);
        if s.board.is_some() || s.board_loading {
            self.stand_in = StandIn::Off;
            return;
        }
        self.load_board(cx);
    }

    fn load_board(&mut self, cx: &mut Context<'_, Self>) {
        self.stand_in = StandIn::Loading;
        let nodus = self.state.read(cx).nodus.clone();
        let project = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let board = nodus.board().load(&project).await;
            let _ = this.update(cx, |r, cx| {
                r.stand_in = StandIn::Loaded;
                if let Ok(board) = board {
                    r.state.update(cx, |s, cx| {
                        if s.route.project_id() == Some(project.as_str()) {
                            s.board = Some(board);
                            cx.notify();
                        }
                    });
                }
            });
        })
        .detach();
    }

    /// ACT-047: the project's own actions, then the global ones.
    fn blocks(&self, cx: &App) -> Vec<Vec<ProjectAction>> {
        let s = self.state.read(cx);
        let Some(board) = s.board.as_ref().filter(|b| b.project.id == self.project_id) else {
            return Vec::new();
        };
        logic::blocks(board.project.actions.as_deref(), &s.global_actions)
            .into_iter()
            .map(<[ProjectAction]>::to_vec)
            .collect()
    }

    fn cards(&self, cx: &App) -> Vec<SessionCard> {
        self.state
            .read(cx)
            .board
            .as_ref()
            .map(|b| b.cards.clone())
            .unwrap_or_default()
    }

    /// Runs a rail timer (ACT-049, ACT-050).
    fn arm(&mut self, timer: logic::Timer, cx: &mut Context<'_, Self>) {
        self.timer = Some(cx.spawn(async move |this, cx| {
            if !timer.delay.is_zero() {
                cx.background_executor().timer(timer.delay).await;
            }
            let _ = this.update(cx, |r, cx| {
                r.rail.fire(timer.token);
                r.timer = None;
                cx.notify();
            });
        }));
    }

    fn reveal(&mut self, buttons_held: bool, cx: &mut Context<'_, Self>) {
        if let Some(timer) = self.rail.reveal(buttons_held) {
            self.arm(timer, cx);
        } else if self.rail.is_open() {
            self.timer = None;
        }
    }

    fn hide(&mut self, cx: &mut Context<'_, Self>) {
        let timer = self.rail.hide();
        self.arm(timer, cx);
    }

    fn keep_open(&mut self) {
        self.rail.keep_open();
        self.timer = None;
    }

    /// ACT-056, ACT-061, ACT-066: closes at once.
    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.rail.close();
        self.timer = None;
        cx.notify();
    }

    /// `openTerminal`.
    fn open_terminal(&mut self, session_id: String, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| {
            s.terminal_session_id = Some(session_id);
            s.preview_session_id = None;
            cx.notify();
        });
    }

    /// ACT-056: a run going opens it; otherwise a press starts one.
    fn press(&mut self, action: &ProjectAction, cx: &mut Context<'_, Self>) {
        match logic::press(&self.cards(cx), action) {
            Press::ShowRun { session_id } => {
                self.close(cx);
                self.open_terminal(session_id, cx);
            }
            Press::Run => self.run(action.clone(), cx),
        }
    }

    /// ACT-031 (`runAction`): a session on the action's prompt, at the spot
    /// a new card would take; its card goes into the store, where the rail
    /// finds it and the canvas leaves it out (ACT-032).
    fn run(&mut self, action: ProjectAction, cx: &mut Context<'_, Self>) {
        let spot = match &self.hooks.spot {
            Some(spot) => spot.clone()(cx),
            None => point(0., 0.),
        };
        let opts = CreateSessionOptions {
            x: nodus_domain::js::round(spot.x),
            y: nodus_domain::js::round(spot.y),
            action_id: Some(action.id.clone()),
            ..CreateSessionOptions::default()
        };
        let nodus = self.state.read(cx).nodus.clone();
        let project = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let created = nodus.sessions().create(&project, &opts).await;
            let _ = this.update(cx, |r, cx| match created {
                Ok(card) => r.state.update(cx, |s, cx| upsert_card(s, card, cx)),
                Err(err) => {
                    let message = logic::run_failed_toast(&action.name, &err.ipc_message());
                    r.hooks.clone().toast(message, cx, &r.state);
                }
            });
        })
        .detach();
    }

    /// ACT-061: `Edit…` and `+` close the rail and open the dialog.
    fn edit(&mut self, action: Option<ProjectAction>, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.close(cx);
        let state = self.state.clone();
        let project = self.project_id.clone();
        let rail = cx.entity();
        self.dialog = Some(cx.new(|cx| {
            ActionDialogView::new(
                state,
                project,
                action,
                Rc::new(move |_, cx| rail.update(cx, |r, cx| r.close_dialog(cx))),
                window,
                cx,
            )
        }));
        cx.notify();
    }

    /// ACT-082: the rail doesn't come back by itself when the dialog closes.
    fn close_dialog(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(dialog) = self.dialog.take() {
            dialog.update(cx, ActionDialogView::closed);
        }
        cx.notify();
    }

    /// ACT-060: Run / Show Run, Edit…, Delete….
    fn open_menu(&mut self, action: ProjectAction, at: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let running = logic::run_for(&self.cards(cx), &action.id).is_some();
        let rail = cx.entity();
        let menus = self.menus.clone();
        let run = {
            let rail = rail.clone();
            let action = action.clone();
            menu_row(
                &menus,
                "act-menu-run",
                logic::menu_run_label(running),
                Some(Glyph::Play),
                false,
                move |_, cx| rail.update(cx, |r, cx| r.press(&action, cx)),
            )
        };
        let edit = {
            let rail = rail.clone();
            let action = action.clone();
            MenuItem::item("Edit…", crate::components::Icon::Pencil, move |window, cx| {
                rail.update(cx, |r, cx| r.edit(Some(action.clone()), window, cx));
            })
        };
        let delete = {
            let rail = rail.clone();
            let action = action.clone();
            menu_row(&menus, "act-menu-delete", "Delete…", None, true, move |_, cx| {
                rail.update(cx, |r, cx| r.ask_delete(&action, cx));
            })
        };
        let mut menu = Menu::at(at, vec![run, edit, MenuItem::Separator, delete]);
        let closing = rail.clone();
        menu.on_close = Some(Rc::new(move |_, cx| {
            closing.update(cx, |r, cx| {
                r.menu = None;
                r.rail.set_menu(false);
                cx.notify();
            });
        }));
        self.menu = Some(action);
        self.rail.set_menu(true);
        self.rail.hint = None;
        self.menus.update(cx, |m, cx| m.open(menu, cx));
        cx.notify();
    }

    /// ACT-062.
    fn ask_delete(&mut self, action: &ProjectAction, cx: &mut Context<'_, Self>) {
        let (title, message) = logic::delete_confirm(action);
        self.confirm = Some(Confirm {
            action: action.clone(),
            title,
            message,
        });
        cx.notify();
    }

    /// ACT-062: confirmed; a run going keeps going.
    fn delete(&mut self, ok: bool, cx: &mut Context<'_, Self>) {
        let Some(confirm) = self.confirm.take() else { return };
        cx.notify();
        if !ok {
            return;
        }
        let state = self.state.clone();
        let project = self.project_id.clone();
        cx.spawn(async move |_, cx| {
            let nodus = cx.update(|cx| state.read(cx).nodus.clone());
            if let Ok(lists) = nodus.actions().remove(&project, &confirm.action.id).await {
                cx.update(|cx| state.update(cx, |s, cx| set_actions(s, lists, cx)));
            }
        })
        .detach();
    }

    /// ACT-053: tells the board while a dialog, menu or confirmation is up.
    fn tell_overlay(&mut self, cx: &mut Context<'_, Self>) {
        let open = self.dialog.is_some() || self.menu.is_some() || self.confirm.is_some();
        if open != self.overlay_told {
            self.overlay_told = open;
            if let Some(overlay) = self.hooks.overlay.clone() {
                overlay(open, cx);
            }
        }
    }
}

impl Drop for ActionRail {
    fn drop(&mut self) {
        // ACT-053: on unmount it reports false (the board's hook is told on
        // its next render; nothing to call without an App here).
        self.overlay_told = false;
    }
}

/// `upsertCard`: replaces the card with the same session, or adds it.
pub fn upsert_card(s: &mut AppState, card: SessionCard, cx: &mut Context<'_, AppState>) {
    let Some(board) = s.board.as_mut() else { return };
    match board
        .cards
        .iter_mut()
        .find(|c| c.node.session_id == card.node.session_id)
    {
        Some(existing) => *existing = card,
        None => board.cards.push(card),
    }
    cx.notify();
}

/// ACT-017 (`setActions`): the project the core changed replaces the list's
/// and the open board's copy; the global list is replaced.
pub fn set_actions(s: &mut AppState, lists: nodus_domain::ActionLists, cx: &mut Context<'_, AppState>) {
    let project = lists.project;
    if let Some(p) = s.projects.iter_mut().find(|p| p.id == project.id) {
        *p = project.clone();
    }
    if let Some(board) = s.board.as_mut()
        && board.project.id == project.id
    {
        board.project = project;
    }
    s.global_actions = lists.global;
    cx.notify();
}

/// A context-menu row drawn as `MenuItem::Item` draws one, with one of
/// spec 05's icons (the shared menu takes only the shell's).
fn menu_row(
    menus: &Entity<MenuLayer>,
    key: &'static str,
    label: &'static str,
    icon: Option<Glyph>,
    danger: bool,
    on_select: impl Fn(&mut Window, &mut App) + 'static,
) -> MenuItem {
    let menus = menus.clone();
    let on_select = Rc::new(on_select);
    MenuItem::Custom(Rc::new(move |window, cx| {
        let t = theme::tokens(cx);
        let hovered = window.use_keyed_state(key, cx, |_, _| false);
        let on = *hovered.read(cx);
        let color = if on {
            t.text_on_accent
        } else if danger {
            t.status_waiting
        } else {
            t.text_primary
        };
        let menus = menus.clone();
        let on_select = on_select.clone();
        div()
            .id(key)
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(26.))
            .pl(px(6.))
            .pr(px(10.))
            .rounded(px(5.))
            .text_color(color)
            .when(on, |d| d.bg(t.accent))
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            // UI-162: closes, then runs the item.
            .on_click(move |_, window, cx| {
                menus.update(cx, |m, cx| m.close(window, cx));
                on_select(window, cx);
            })
            .child(
                div()
                    .flex()
                    .justify_center()
                    .w(px(16.))
                    .flex_none()
                    .opacity(0.85)
                    .when_some(icon, |d, i| d.child(glyph(i, 14., 2., color))),
            )
            .child(div().flex_1().child(label))
            .into_any_element()
    }))
}

/// Where a button is, measured as it's drawn.
fn measure(
    buttons: &Rc<std::cell::RefCell<std::collections::HashMap<String, Bounds<Pixels>>>>,
    key: String,
) -> impl IntoElement {
    let buttons = buttons.clone();
    canvas(
        move |bounds, _, _| {
            buttons.borrow_mut().insert(key, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

impl Render for ActionRail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.ensure_board(cx);
        self.tell_overlay(cx);
        let t = theme::tokens(cx);
        let blocks = self.blocks(cx);
        // ACT-046: only on a board whose project is loaded.
        let loaded = self
            .state
            .read(cx)
            .board
            .as_ref()
            .is_some_and(|b| b.project.id == self.project_id);
        if !loaded {
            return div().into_any_element();
        }
        let cards = self.cards(cx);
        let actions: Vec<ProjectAction> = blocks.iter().flatten().cloned().collect();
        let running: Vec<(ProjectAction, SessionCard)> = logic::running(&actions, &cards)
            .into_iter()
            .map(|(a, c)| (a.clone(), c.clone()))
            .collect();
        let shown = self.rail.shown();
        let viewport = window.viewport_size();
        let this = cx.entity();

        // The rail's items (ACT-047, ACT-057, ACT-058).
        let mut items: Vec<AnyElement> = Vec::new();
        for block in &blocks {
            for action in block {
                items.push(self.item(action, &cards, shown, window, cx));
            }
            items.push(
                div()
                    .flex_none()
                    .w(px(24.))
                    .h(px(0.5))
                    .my(px(2.))
                    .bg(t.border_strong)
                    .into_any_element(),
            );
        }
        items.push(self.add_button(shown, window, cx));

        // `.rail.is-right`: slides in from 8px past the edge and fades, 220ms.
        let slide = transition(
            window,
            cx,
            "act-rail-slide",
            if shown { 1.0 } else { 0.0 },
            DUR_MED,
            motion::ease_out,
        );
        let fade = transition(
            window,
            cx,
            "act-rail-fade",
            if shown { 1.0 } else { 0.0 },
            DUR_MED,
            motion::ease,
        );
        let main_h = viewport.height - TITLEBAR_H;
        let hover_rail = this.clone();
        let scroll_rail = this.clone();
        let rail = div()
            .id("act-rail")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .w(RAIL_W)
            .max_h(main_h - RAIL_INSET * 2.)
            .py(px(6.))
            .overflow_y_scroll()
            .rounded(RADIUS_LG)
            .map(|d| frosted(d, t.rail_surface, 24., 1.8))
            .shadow(t.lg())
            .translate(px((52. + 8.) * (1. - slide)), px(0.))
            .group_opacity(fade)
            .when(shown, |d| {
                d.occlude()
                    // ACT-050, ACT-051.
                    .on_hover(move |h, _, cx| {
                        let over = *h;
                        hover_rail.update(cx, |r, cx| {
                            if over {
                                r.keep_open();
                            } else {
                                r.hide(cx);
                            }
                        });
                    })
                    // ACT-054: any rail scroll hides the hint.
                    .on_scroll(move |_, _, cx| {
                        scroll_rail.update(cx, |r, cx| {
                            r.rail.hint = None;
                            cx.notify();
                        });
                    })
            })
            .children(items);
        let column = div()
            .absolute()
            .top_0()
            .right(RAIL_INSET)
            .h(main_h)
            .flex()
            .flex_col()
            .justify_center()
            .child(rail);

        // ACT-049, ACT-050, ACT-059: the 6px strip, and the sliver on it.
        let edge_status = (!shown)
            .then(|| logic::edge_mark(running.iter().map(|(_, c)| c)))
            .flatten();
        let reveal = this.clone();
        let leave = this.clone();
        let edge = div()
            .id("act-rail-edge")
            .absolute()
            .top_0()
            .right_0()
            .w(px(6.))
            .h(main_h)
            .on_mouse_move(move |e, _, cx| {
                let held = e.pressed_button.is_some();
                reveal.update(cx, |r, cx| r.reveal(held, cx));
            })
            .on_hover(move |h, _, cx| {
                if !*h {
                    leave.update(cx, ActionRail::hide);
                }
            })
            .children(edge_status.map(|status| edge_mark(status, cx)));

        let hint = if shown {
            self.hint(&actions, viewport.width, window, cx)
        } else {
            None
        };
        let fan = (shown && !running.is_empty()).then(|| self.fan(&running, viewport, window, cx));
        // The fan's clock ticks each second while it shows (`useNow`).
        if fan.is_some() && self._tick.is_none() {
            self._tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(1000)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        } else if fan.is_none() {
            self._tick = None;
        }

        let confirm = self.confirm.clone().map(|c| self.confirm_dialog(&c, window, cx));

        div()
            .absolute()
            .inset_0()
            .child(deferred(column).with_priority(40))
            .child(deferred(edge).with_priority(41))
            .children(fan)
            .children(hint)
            .children(self.dialog.clone())
            .children(confirm)
            .into_any_element()
    }
}

impl ActionRail {
    /// One action's button (ACT-054, ACT-056…ACT-058, ACT-060, ACT-081).
    fn item(
        &self,
        action: &ProjectAction,
        cards: &[SessionCard],
        shown: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = theme::tokens(cx);
        let run = logic::run_for(cards, &action.id).cloned();
        let id = SharedString::from(format!("act-{}", action.id));
        let (tile_bg, icon_color) = if run.is_some() {
            (alpha(t.status_working, 0.16), t.status_working)
        } else {
            (alpha(t.accent, 0.14), t.accent)
        };
        // 120ms `ease` on the tile's background and colour.
        let icon_color = transition(
            window,
            cx,
            child_id(id.clone(), "fg"),
            icon_color,
            DUR_FAST,
            motion::ease,
        );
        let rail = cx.entity();
        let press_action = action.clone();
        let menu_action = action.clone();
        let hint_id = action.id.clone();
        let buttons = self.buttons.clone();
        let running = run.is_some();
        let tile = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(px(9.))
            .bg(tile_bg)
            .child(glyph(Glyph::action(&action.icon), 17., 2., icon_color))
            .when(action.global == Some(true), |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(-3.))
                        .bottom(px(-3.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(14.))
                        .rounded_full()
                        .bg(t.bg_elevated)
                        .shadow(vec![ring(0.5, t.border_strong)])
                        .child(glyph(Glyph::Globe, 9., 2.4, t.text_secondary)),
                )
            });
        div()
            .id(id.clone())
            .relative()
            .flex_none()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .when(shown, |d| {
                let leave = rail.clone();
                let click = rail.clone();
                d.hover(|s| s.bg(t.bg_hover))
                    .active(|s| s.bg(t.bg_pressed))
                    .on_hover(move |h, _, cx| {
                        let over = *h;
                        let top = buttons.borrow().get(&hint_id).map(|b| f64::from(b.center().y));
                        leave.update(cx, |r, cx| {
                            // ACT-054: running actions show no hint; the fan says it all.
                            r.rail.hint = (over && !running).then(|| HintAt {
                                action_id: Some(hint_id.clone()),
                                top: top.unwrap_or_default(),
                            });
                            cx.notify();
                        });
                    })
                    .on_click(move |_, _, cx| click.update(cx, |r, cx| r.press(&press_action, cx)))
                    .on_mouse_down(MouseButton::Right, move |e, _, cx| {
                        cx.stop_propagation();
                        let action = menu_action.clone();
                        rail.update(cx, |r, cx| r.open_menu(action, e.position, cx));
                    })
            })
            .child(tile)
            .when_some(run, |d, run| {
                d.child(
                    div().absolute().top(px(4.)).right(px(3.)).child(
                        status_dot(child_id(id.clone(), "dot"), logic::dot_status(&run))
                            .size(px(9.))
                            .border(px(1.5), t.bg_elevated),
                    ),
                )
            })
            .child(measure(&self.buttons, action.id.clone()))
            .into_any_element()
    }

    /// `+` (ACT-047, ACT-061).
    fn add_button(&self, shown: bool, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let hovered = self.rail.hint.as_ref().is_some_and(|h| h.action_id.is_none());
        let rail = cx.entity();
        let buttons = self.buttons.clone();
        let _ = window;
        div()
            .id("act-add")
            .relative()
            .flex_none()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .when(shown, |d| {
                let click = rail.clone();
                d.hover(|s| s.bg(t.bg_hover))
                    .active(|s| s.bg(t.bg_pressed))
                    .on_hover(move |h, _, cx| {
                        let over = *h;
                        let top = buttons.borrow().get("").map(|b| f64::from(b.center().y));
                        rail.update(cx, |r, cx| {
                            r.rail.hint = over.then(|| HintAt {
                                action_id: None,
                                top: top.unwrap_or_default(),
                            });
                            cx.notify();
                        });
                    })
                    .on_click(move |_, window, cx| click.update(cx, |r, cx| r.edit(None, window, cx)))
            })
            .child(glyph(
                Glyph::Plus,
                18.,
                2.,
                if hovered { t.text_primary } else { t.text_secondary },
            ))
            .child(measure(&self.buttons, String::new()))
            .into_any_element()
    }

    /// ACT-054, ACT-055: the hint beside a pointed-at button.
    fn hint(
        &self,
        actions: &[ProjectAction],
        viewport_w: Pixels,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let hint = self.rail.hint.clone()?;
        let action = match &hint.action_id {
            Some(id) => Some(actions.iter().find(|a| &a.id == id)?),
            None => None,
        };
        let (name, detail) = logic::hint(action);
        let t = theme::tokens(cx);
        // `action-hint-in`: 120ms from 3px right, transparent.
        let p = enter(
            window,
            cx,
            SharedString::from(format!("act-hint-{}", hint.action_id.clone().unwrap_or_default())),
            DUR_FAST,
            motion::ease_out,
        );
        let card = div()
            .flex()
            .flex_col()
            .max_w(px(260.))
            .px(px(9.))
            .py(px(5.))
            .rounded(RADIUS_SM)
            .bg(t.bg_elevated)
            .shadow(t.md())
            .opacity(p)
            .translate(px(3. * (1. - p)), px(0.))
            .child(
                div()
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_primary)
                    .truncate()
                    .child(name),
            )
            .child(
                div()
                    .text_size(TEXT_XS)
                    .line_height(px(15.4))
                    .text_color(t.text_secondary)
                    .child(detail),
            );
        // Level with the button's middle, 6px left of the rail.
        let right = viewport_w - (RAIL_INSET + RAIL_W + px(6.));
        Some(
            deferred(
                anchored()
                    .position(point(right, px(hint.top as f32)))
                    .anchor(Anchor::RightCenter)
                    .child(card),
            )
            .with_priority(60)
            .into_any_element(),
        )
    }

    /// ACT-063…ACT-066: the running actions' sessions, dealt out beside the
    /// rail from the first running button down.
    fn fan(
        &mut self,
        running: &[(ProjectAction, SessionCard)],
        viewport: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let now = parity::now_ms(cx);
        let anchor_id = running[0].0.id.clone();
        let ids: Vec<String> = running.iter().map(|(_, c)| c.node.session_id.clone()).collect();
        // ACT-064: measured again when the anchor or the runs change.
        let key = (anchor_id.clone(), ids.clone());
        if self.fan_for.as_ref() != Some(&key) {
            self.fan_for = Some(key);
            self.fan_height.set(None);
        }
        let height = self.fan_height.get();
        let anchor_top = self.buttons.borrow().get(&anchor_id).map(|b| b.top());
        let top = match (height, anchor_top) {
            (Some(h), Some(anchor)) => {
                Some(px(
                    logic::fan_top(f64::from(anchor), f64::from(h), f64::from(viewport.height)) as f32,
                ))
            }
            _ => None,
        };
        if top.is_none() {
            window.request_animation_frame();
        }
        let cards: Vec<AnyElement> = running
            .iter()
            .enumerate()
            .map(|(i, (action, card))| self.fan_card(i, action, card, now, top.is_some(), window, cx))
            .collect();
        let fan_height = self.fan_height.clone();
        let rail = cx.entity();
        let leave = cx.entity();
        let right = viewport.width - (RAIL_INSET + RAIL_W + px(10.));
        deferred(
            anchored()
                .position(point(right, top.unwrap_or(px(0.))))
                .anchor(Anchor::TopRight)
                .child(
                    div()
                        .id("act-fan")
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .w(px(260.))
                        .when(top.is_none(), |d| d.opacity(0.))
                        .occlude()
                        .on_hover(move |h, _, cx| {
                            let over = *h;
                            leave.update(cx, |r, cx| {
                                if over {
                                    r.keep_open();
                                } else {
                                    r.hide(cx);
                                }
                            });
                        })
                        .children(cards)
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    if fan_height.get() != Some(bounds.size.height) {
                                        fan_height.set(Some(bounds.size.height));
                                        rail.update(cx, |_, cx| cx.notify());
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        ),
                ),
        )
        .with_priority(45)
        .into_any_element()
    }

    /// ACT-065, ACT-066: a fan card (`.fan-card`).
    #[allow(clippy::too_many_arguments)]
    fn fan_card(
        &self,
        i: usize,
        action: &ProjectAction,
        card: &SessionCard,
        now: f64,
        placed: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = theme::tokens(cx);
        let id = SharedString::from(format!("act-fan-{}", card.node.session_id));
        let edge = t.status(card.status);
        let hovered = window.use_keyed_state(child_id(id.clone(), "hover"), cx, |_, _| false);
        let on = *hovered.read(cx);
        // `fan-deal`: 520ms on the stack's spring, each a beat after the last.
        let deal = if placed {
            dealt(window, cx, child_id(id.clone(), "deal"), i)
        } else {
            0.
        };
        let mut shadows = vec![inset_shadow(3., 0., 0., 0., edge)];
        if on {
            shadows.push(ring(1., t.border_strong));
            shadows.extend(t.lg().into_iter().rev());
        } else {
            shadows.extend(t.md().into_iter().rev());
        }
        shadows.reverse();
        let status_color = match card.status {
            SessionStatus::Working => t.status_working,
            SessionStatus::Waiting => t.status_waiting,
            SessionStatus::Done => t.text_tertiary,
        };
        let rail = cx.entity();
        let session = card.node.session_id.clone();
        div()
            .id(id.clone())
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .pt(px(10.))
            .px(px(12.))
            .pb(px(11.))
            .rounded(RADIUS_MD)
            .bg(t.bg_elevated)
            .shadow(shadows)
            .text_color(t.text_secondary)
            .translate(px(46. * (1. - deal)), px(10. * (1. - deal)))
            .scale(0.92 + 0.08 * deal)
            .rotate(gpui::radians((-2.5f32).to_radians() * (1. - deal)))
            .group_opacity(deal.clamp(0., 1.))
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .tooltip(crate::components::native_tooltip(logic::fan_title(action).into()))
            .on_click(move |_, _, cx| {
                let session = session.clone();
                rail.update(cx, |r, cx| {
                    r.close(cx);
                    r.open_terminal(session, cx);
                });
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .min_w_0()
                    .child(status_dot(child_id(id.clone(), "dot"), card.status).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(TEXT_MD)
                            .line_height(px(18.2))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_primary)
                            .child(action.name.clone()),
                    ),
            )
            .child(
                div()
                    .pl(px(14.))
                    .text_size(TEXT_XS)
                    .line_height(px(15.4))
                    .line_clamp(2)
                    .child(logic::fan_step(card)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(8.))
                    .pl(px(14.))
                    .text_size(TEXT_XS)
                    .line_height(px(15.4))
                    .text_color(t.text_tertiary)
                    .font_features(theme::fonts::tabular())
                    .child(div().text_color(status_color).child(logic::fan_status(card.status)))
                    .child(format_duration(now - card.node.created_at)),
            )
            .into_any_element()
    }

    /// ACT-062: the confirmation (`ConfirmDialog`): Enter confirms, Escape
    /// cancels.
    fn confirm_dialog(&self, confirm: &Confirm, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let rail = cx.entity();
        let cancel = rail.clone();
        let ok = rail.clone();
        let keys = rail.clone();
        let focus = window.use_keyed_state("act-confirm-focus", cx, |_, cx| cx.focus_handle());
        let focus = focus.read(cx).clone();
        if !focus.is_focused(window) {
            window.focus(&focus, cx);
        }
        Modal::new("act-confirm", px(380.), move |_, cx| {
            cancel.update(cx, |r, cx| r.delete(false, cx));
        })
        .render(
            div()
                .track_focus(&focus)
                .on_key_down(move |e: &gpui::KeyDownEvent, _, cx| match e.keystroke.key.as_str() {
                    "enter" => {
                        cx.stop_propagation();
                        keys.update(cx, |r, cx| r.delete(true, cx));
                    }
                    "escape" => {
                        cx.stop_propagation();
                        keys.update(cx, |r, cx| r.delete(false, cx));
                    }
                    _ => {}
                })
                .pt(px(22.))
                .px(px(22.))
                .pb(px(18.))
                .text_color(t.text_primary)
                .child(
                    div()
                        .mb(px(6.))
                        .text_size(TEXT_LG)
                        .line_height(px(21.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(confirm.title.clone()),
                )
                .child(
                    div()
                        .mb(px(20.))
                        .text_color(t.text_secondary)
                        .line_height(px(18.85))
                        .child(confirm.message),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(button("act-confirm-cancel").label("Cancel").on_click({
                            let rail = rail.clone();
                            move |_, _, cx| rail.update(cx, |r, cx| r.delete(false, cx))
                        }))
                        .child(
                            button("act-confirm-ok")
                                .kind(Kind::Danger)
                                .label("Delete")
                                .on_click(move |_, _, cx| ok.update(cx, |r, cx| r.delete(true, cx))),
                        ),
                ),
            window,
            cx,
        )
    }
}

/// `fan-deal`'s progress for the `i`th card: 520ms on `--ease-stack`, after
/// `i × 45 + 30` ms.
fn dealt(window: &mut Window, cx: &mut App, key: gpui::ElementId, i: usize) -> f32 {
    let delay = 30. + 45. * i as f32;
    let total = Duration::from_millis((delay + 520.) as u64);
    let p = enter(window, cx, key, total, |t| t);
    let ms = p * (delay + 520.);
    if ms <= delay {
        0.
    } else {
        motion::ease_stack((ms - delay) / 520.)
    }
}

/// ACT-059: the 3×36 sliver on the edge, breathing (1.6 s, or 1.1 s red when
/// a run waits on the user).
fn edge_mark(status: SessionStatus, cx: &App) -> AnyElement {
    use gpui::{Animation, AnimationExt as _};
    let t = theme::tokens(cx);
    let (color, period) = if status == SessionStatus::Waiting {
        (t.status_waiting, 1100)
    } else {
        (t.status_working, 1600)
    };
    let mark = div()
        .absolute()
        .right(px(1.))
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .child(div().w(px(3.)).h(px(36.)).rounded(px(2.)).bg(color));
    if parity::frozen(cx) {
        return mark.into_any_element();
    }
    mark.with_animation(
        ("act-rail-mark", period),
        Animation::new(Duration::from_millis(period)).repeat(),
        |d, p| {
            // `action-mark-breathe`: 0.45 at 50%, ease-in-out each way.
            let half = if p < 0.5 { p * 2.0 } else { (1.0 - p) * 2.0 };
            d.opacity(1.0 - 0.55 * motion::ease_in_out(half))
        },
    )
    .into_any_element()
}
