//! The project rail on a board (`ProjectRail.tsx`, `ProjectRail.css`;
//! UI-253…UI-284): a Dock-like strip that slides in from the window's left
//! edge, or a pinned sidebar; ⌘1–⌘9 switch boards, and holding ⌘ numbers them.

use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent,
    ModifiersChangedEvent, MouseButton, ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement,
    Styled, Task, Window, canvas, deferred, div, px,
};
use nodus_domain::focus::shows_in_focus;
use nodus_domain::{Project, ProjectPatch, ProjectStats};

use crate::components::context_menu::{Menu, MenuItem, MenuLayer};
use crate::components::folder_icon::{folder_icon, project_glyph};
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::status_dot::status_dot;
use crate::components::tween::{enter, transition};
use crate::platform::{Platform, keys};
use crate::projects::folder_item::ServerBadge;
use crate::state::{AppState, Route, group_projects};
use crate::theme::tokens::*;
use crate::theme::{self, fonts, motion};

/// Resting on the edge this long reveals the rail (UI-262).
pub const REVEAL_DELAY: Duration = Duration::from_millis(100);
/// The grace after the pointer leaves (UI-264).
pub const HIDE_DELAY: Duration = Duration::from_millis(300);
/// Holding ⌘ alone this long numbers the projects (UI-275).
pub const NUMBERS_DELAY: Duration = Duration::from_millis(300);

/// The hovered item: a project, or the `+` button, and its vertical centre
/// in window coordinates.
#[derive(Debug, Clone, PartialEq)]
struct Hint {
    project_id: Option<String>,
    center: Pixels,
}

pub struct ProjectRail {
    state: Entity<AppState>,
    menus: Entity<MenuLayer>,
    open: bool,
    numbered: bool,
    hint: Option<Hint>,
    menu_open: bool,
    over_rail: bool,
    /// The pending reveal or hide.
    timer: Option<Task<()>>,
    /// The pending ⌘ numbers.
    numbers_wait: Option<Task<()>>,
    numbers_serial: usize,
}

impl ProjectRail {
    pub fn new(state: Entity<AppState>, menus: Entity<MenuLayer>, cx: &mut Context<'_, Self>) -> Self {
        // UI-283: the badges need fresh stats.
        state.update(cx, |s, cx| s.refresh_projects(cx).detach());
        Self {
            state,
            menus,
            open: false,
            numbered: false,
            hint: None,
            menu_open: false,
            over_rail: false,
            timer: None,
            numbers_wait: None,
            numbers_serial: 0,
        }
    }

    /// The rail's blocks: ungrouped, then each group, in the order they were
    /// added; while focusing only the projects in focus, waiting or open
    /// (UI-254, UI-255).
    fn blocks(&self, cx: &App) -> Vec<Vec<Project>> {
        let s = self.state.read(cx);
        let focusing = s.focusing();
        let open = s.route.project_id().map(str::to_owned);
        let mut active: Vec<Project> = s
            .projects
            .iter()
            .filter(|p| p.archived != Some(true))
            .filter(|p| !focusing || shows_in_focus(p, s.stats.get(&p.id), open.as_deref()))
            .cloned()
            .collect();
        active.sort_by(|a, b| {
            a.created_at
                .partial_cmp(&b.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let (ungrouped, by_group) = group_projects(&active, &s.groups);
        std::iter::once(ungrouped)
            .chain(by_group.into_iter().map(|(_, ps)| ps))
            .filter(|b| !b.is_empty())
            .collect()
    }

    fn order(&self, cx: &App) -> Vec<Project> {
        self.blocks(cx).into_iter().flatten().collect()
    }

    fn keep_open(&mut self) {
        self.timer = None;
    }

    /// UI-262, UI-263, UI-265: resting on the edge 100ms reveals it, unless
    /// a button is held.
    fn reveal(&mut self, buttons_held: bool, cx: &mut Context<'_, Self>) {
        if self.open {
            self.keep_open();
            return;
        }
        if buttons_held || self.timer.is_some() {
            return;
        }
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_DELAY).await;
            let _ = this.update(cx, |rail, cx| {
                rail.timer = None;
                rail.open = true;
                cx.notify();
            });
        }));
    }

    /// UI-264, UI-267: slides out 300ms after the pointer leaves, not while
    /// its menu is open.
    fn hide(&mut self, cx: &mut Context<'_, Self>) {
        self.keep_open();
        if self.menu_open {
            return;
        }
        let delay = if self.open { HIDE_DELAY } else { Duration::ZERO };
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |rail, cx| {
                rail.timer = None;
                rail.open = false;
                rail.hint = None;
                cx.notify();
            });
        }));
    }

    /// UI-279: the badges and a pending wait end.
    pub fn hide_numbers(&mut self, cx: &mut Context<'_, Self>) {
        self.numbers_wait = None;
        if self.numbered {
            self.numbered = false;
            // UI-281: let go over the rail, it stays out.
            if self.over_rail {
                self.open = true;
            }
            cx.notify();
        }
    }

    /// UI-280: a press or a scroll while ⌘ is held cancels the pending badges.
    pub fn cancel_numbers(&mut self) {
        self.numbers_wait = None;
    }

    fn blocked(&self, m: &gpui::Modifiers, cx: &App) -> bool {
        let platform = Platform::CURRENT;
        m.shift || m.alt || platform.other_mod_key(m) || self.state.read(cx).settings_open
    }

    /// UI-275, UI-276: holding ⌘ alone 300ms numbers the projects.
    pub fn on_modifiers_changed(&mut self, e: &ModifiersChangedEvent, cx: &mut Context<'_, Self>) {
        let m = &e.modifiers;
        let held = Platform::CURRENT.mod_key(m);
        if !held {
            self.hide_numbers(cx);
            return;
        }
        if self.blocked(m, cx) {
            self.hide_numbers(cx);
            return;
        }
        if self.numbers_wait.is_none() && !self.numbered {
            self.numbers_wait = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(NUMBERS_DELAY).await;
                let _ = this.update(cx, |rail, cx| {
                    rail.numbers_wait = None;
                    rail.numbered = true;
                    rail.numbers_serial += 1;
                    cx.notify();
                });
            }));
        }
    }

    /// UI-277…UI-279: ⌘ and a digit opens the Nth project; any other key
    /// ends the numbers. Returns whether the key was taken.
    pub fn on_key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let m = &e.keystroke.modifiers;
        let platform = Platform::CURRENT;
        let digit = e.keystroke.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d));
        if let Some(n) = digit
            && platform.mod_key(m)
            && !self.blocked(m, cx)
        {
            let Some(project) = self.order(cx).into_iter().nth(n - 1) else {
                // No project there: the key passes on, the numbers stay.
                return false;
            };
            self.state.update(cx, |s, cx| {
                if s.route.project_id() != Some(project.id.as_str()) {
                    s.navigate(Route::Board(project.id.clone()), cx);
                }
            });
            // UI-278: the numbers stay while ⌘ is held.
            return true;
        }
        self.hide_numbers(cx);
        false
    }

    pub fn on_key_up(&mut self, e: &KeyUpEvent, cx: &mut Context<'_, Self>) {
        if !Platform::CURRENT.mod_key(&e.keystroke.modifiers) {
            self.hide_numbers(cx);
        }
    }

    /// UI-274: Add to / Remove from Focus.
    fn open_menu(&mut self, project: &Project, at: gpui::Point<Pixels>, cx: &mut Context<'_, Self>) {
        let state = self.state.clone();
        let focused = project.focused == Some(true);
        let id = project.id.clone();
        let rail = cx.entity();
        let item = MenuItem::item(
            if focused { "Remove from Focus" } else { "Add to Focus" },
            Icon::Target,
            move |_, cx| {
                state.update(cx, |s, cx| {
                    s.update_project(
                        id.clone(),
                        ProjectPatch {
                            focused: Some(!focused),
                            ..ProjectPatch::default()
                        },
                        cx,
                    );
                });
            },
        );
        let mut menu = Menu::at(at, vec![item]);
        menu.on_close = Some(Rc::new(move |_, cx| {
            rail.update(cx, |r, cx| {
                r.menu_open = false;
                let pinned = r.state.read(cx).pin_rail();
                if !pinned && !r.over_rail {
                    r.keep_open();
                    r.timer = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(HIDE_DELAY).await;
                        let _ = this.update(cx, |rail, cx| {
                            rail.timer = None;
                            rail.open = false;
                            cx.notify();
                        });
                    }));
                }
            });
        }));
        self.menu_open = true;
        self.hint = None;
        self.menus.update(cx, |m, cx| m.open(menu, cx));
        cx.notify();
    }

    fn item(
        &self,
        project: &Project,
        at: ItemState,
        stats: Option<&ProjectStats>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let ItemState {
            number,
            active,
            focusing,
        } = at;
        let t = theme::tokens(cx);
        let id = SharedString::from(format!("rail-{}", project.id));
        let status = stats.and_then(|s| {
            if s.waiting > 0 {
                Some(nodus_domain::SessionStatus::Waiting)
            } else if s.working > 0 {
                Some(nodus_domain::SessionStatus::Working)
            } else {
                None
            }
        });
        let unfocused = focusing && project.focused != Some(true);
        let mut folder = folder_icon(34., Some(project.color)).glyph(project_glyph(&project.name));
        if unfocused {
            folder = folder.saturate(0.4);
        }
        let focus_feature = self.state.read(cx).focus_feature();
        let server = ServerBadge::of(project, self.state.read(cx));
        let state = self.state.clone();
        let open_id = project.id.clone();
        let rail = cx.entity();
        let hint_id = project.id.clone();
        let menu_project = project.clone();
        let rail_for_menu = cx.entity();
        let center = window.use_keyed_state(crate::components::child_id(id.clone(), "center"), cx, |_, _| px(0.));
        let center_read = center.clone();
        let bg = if active { Some(t.bg_selected) } else { None };
        let badge = number.map(|n| {
            let pop = enter(
                window,
                cx,
                (id.clone(), self.numbers_serial),
                DUR_FAST,
                motion::ease_out,
            );
            div()
                .absolute()
                .right(px(0.))
                .bottom(px(1.))
                .min_w(px(16.))
                .h(px(16.))
                .px(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .border(px(0.5))
                .border_color(t.border_strong)
                .rounded(px(5.))
                .bg(t.bg_elevated)
                .shadow(t.sm())
                .font_family(fonts::UI)
                .text_size(px(10.5))
                .line_height(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .font_features(fonts::tabular())
                .text_color(t.text_primary)
                .scale(0.7 + 0.3 * pop)
                .group_opacity(pop)
                .child(n.to_string())
        });
        div()
            .id(id)
            .relative()
            .flex_none()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .when_some(bg, Styled::bg)
            .when(!active, |d| {
                d.hover(|s| s.bg(t.bg_hover)).active(|s| s.bg(t.bg_pressed))
            })
            .on_hover(move |h, _, cx| {
                let center = *center_read.read(cx);
                rail.update(cx, |r, cx| {
                    r.hint = h.then(|| Hint {
                        project_id: Some(hint_id.clone()),
                        center,
                    });
                    cx.notify();
                });
            })
            // UI-260
            .on_click(move |_, _, cx| {
                if !active {
                    let id = open_id.clone();
                    state.update(cx, |s, cx| s.navigate(Route::Board(id), cx));
                }
            })
            .when(focus_feature, |d| {
                d.on_mouse_down(MouseButton::Right, move |e, _, cx| {
                    cx.stop_propagation();
                    let project = menu_project.clone();
                    rail_for_menu.update(cx, |r, cx| r.open_menu(&project, e.position, cx));
                })
            })
            .when(active, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(-6.))
                        .top(px(11.))
                        .w(px(3.))
                        .h(px(18.))
                        .rounded_tr(px(3.))
                        .rounded_br(px(3.))
                        .bg(t.accent),
                )
            })
            .child(div().when(unfocused, |d| d.opacity(0.45)).child(folder))
            // UI-340: a server project's badge, faded with its folder.
            .when_some(server, |d, server| {
                d.child(
                    server
                        .mark(14., 9., t)
                        .absolute()
                        .left(px(1.))
                        .bottom(px(2.))
                        .shadow(t.sm())
                        .when(unfocused, |d| d.opacity(0.45)),
                )
            })
            .when_some(status, |d, status| {
                d.child(
                    div().absolute().top(px(4.)).right(px(3.)).child(
                        status_dot((SharedString::from(format!("rail-dot-{}", project.id)), 0usize), status)
                            .size(px(9.))
                            .border(px(1.5), t.bg_elevated),
                    ),
                )
            })
            .children(badge)
            .child(
                canvas(
                    move |b, _, cx| center.update(cx, |c, _| *c = b.center().y),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    fn hint_card(&self, pinned: bool, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let hint = self.hint.clone()?;
        let t = theme::tokens(cx);
        let s = self.state.read(cx);
        let focusing = s.focusing();
        let order = self.order(cx);
        let (name, detail) = match &hint.project_id {
            None => ("New Project".to_owned(), Some(keys("⇧⌘N"))),
            Some(id) => {
                let project = s.projects.iter().find(|p| &p.id == id)?;
                let group = project
                    .group_id
                    .as_ref()
                    .and_then(|g| s.groups.iter().find(|x| &x.id == g))
                    .map(|g| g.name.as_str());
                let detail = HintDetail {
                    group,
                    server: ServerBadge::of(project, s),
                    stats: s.stats.get(id),
                    not_in_focus: focusing && project.focused != Some(true),
                    number: order.iter().position(|p| &p.id == id).map(|i| i + 1),
                };
                (project.name.clone(), detail.text())
            }
        };
        let slide = enter(
            window,
            cx,
            crate::components::child_id(hint_key(&hint), "rail-hint"),
            DUR_FAST,
            motion::ease_out,
        );
        // Centred on the item (translateY(-50%)).
        let height = 10.0 + 16.8 + if detail.is_some() { 15.4 } else { 0.0 };
        let left = if pinned { px(58.) } else { px(66.) };
        // The main area starts under the title bar.
        let top = hint.center - TITLEBAR_H;
        Some(
            deferred(
                div()
                    .absolute()
                    .left(left)
                    .translate(px(-3. * (1. - slide)), px(0.))
                    .top(top)
                    .child(
                        div()
                            .relative()
                            .top(px(-height / 2.))
                            .flex()
                            .flex_col()
                            .max_w(px(260.))
                            .px(px(9.))
                            .py(px(5.))
                            .rounded(RADIUS_SM)
                            .bg(t.bg_elevated)
                            .shadow(t.md())
                            .opacity(slide)
                            .child(
                                div()
                                    .text_size(TEXT_SM)
                                    .line_height(px(16.8))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text_primary)
                                    .truncate()
                                    .child(name),
                            )
                            .when_some(detail, |d, detail| {
                                d.child(
                                    div()
                                        .text_size(TEXT_XS)
                                        .line_height(px(15.4))
                                        .text_color(t.text_secondary)
                                        .child(detail),
                                )
                            }),
                    ),
            )
            .with_priority(60)
            .into_any_element(),
        )
    }
}

/// How a rail item shows: its ⌘ number, whether it's the open project, and
/// whether focus mode is narrowing the rail.
#[derive(Clone, Copy)]
struct ItemState {
    number: Option<usize>,
    active: bool,
    focusing: bool,
}

/// What a project's hint says under its name (UI-272).
struct HintDetail<'a> {
    group: Option<&'a str>,
    /// UI-340: where a server project lives.
    server: Option<ServerBadge>,
    stats: Option<&'a ProjectStats>,
    not_in_focus: bool,
    /// Its place in the rail, from 1.
    number: Option<usize>,
}

impl HintDetail<'_> {
    /// The parts that apply, joined with ` · `: its group, `On <server>`
    /// (the badge's tooltip), what waits and works, `Not in focus` and its
    /// shortcut among the first nine; none with none.
    fn text(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.group.map(str::to_owned));
        parts.extend(self.server.as_ref().map(ServerBadge::tooltip));
        if let Some(st) = self.stats {
            if st.waiting > 0 {
                parts.push(format!("{} waiting", st.waiting));
            }
            if st.working > 0 {
                parts.push(format!("{} working", st.working));
            }
        }
        if self.not_in_focus {
            parts.push("Not in focus".to_owned());
        }
        parts.extend(self.number.filter(|n| *n <= 9).map(|n| keys(&format!("⌘{n}"))));
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

fn hint_key(hint: &Hint) -> SharedString {
    SharedString::from(hint.project_id.clone().unwrap_or_default())
}

impl Render for ProjectRail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let (pinned, focusing, active_id, stats) = {
            let s = self.state.read(cx);
            (
                s.pin_rail(),
                s.focusing(),
                s.route.project_id().map(str::to_owned),
                s.stats.clone(),
            )
        };
        let shown = pinned || self.open || self.numbered;
        let blocks = self.blocks(cx);
        let order: Vec<String> = blocks.iter().flatten().map(|p| p.id.clone()).collect();

        let mut children: Vec<AnyElement> = Vec::new();
        for block in &blocks {
            for project in block {
                let number = self
                    .numbered
                    .then(|| order.iter().position(|id| id == &project.id).map(|i| i + 1))
                    .flatten()
                    .filter(|n| *n <= 9);
                let active = active_id.as_deref() == Some(project.id.as_str());
                children.push(self.item(
                    project,
                    ItemState {
                        number,
                        active,
                        focusing,
                    },
                    stats.get(&project.id),
                    window,
                    cx,
                ));
            }
            // UI-256: after every block.
            children.push(
                div()
                    .flex_none()
                    .w(px(24.))
                    .h(px(0.5))
                    .my(px(2.))
                    .bg(t.border_strong)
                    .into_any_element(),
            );
        }
        let add_state = self.state.clone();
        let add_rail = cx.entity();
        let add_center = window.use_keyed_state("rail-add-center", cx, |_, _| px(0.));
        let add_center_read = add_center.clone();
        children.push(
            div()
                .id("rail-add")
                .relative()
                .flex_none()
                .size(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_MD)
                .hover(|s| s.bg(t.bg_hover))
                .active(|s| s.bg(t.bg_pressed))
                .on_hover(move |h, _, cx| {
                    let center = *add_center_read.read(cx);
                    add_rail.update(cx, |r, cx| {
                        r.hint = h.then_some(Hint {
                            project_id: None,
                            center,
                        });
                        cx.notify();
                    });
                })
                // UI-261: adds a project and opens it.
                .on_click(move |_, _, cx| add_state.update(cx, |s, cx| s.add_project(None, true, cx)))
                .child(icon(
                    Icon::Plus,
                    18.,
                    2.,
                    if self.hint.as_ref().is_some_and(|h| h.project_id.is_none()) {
                        t.text_primary
                    } else {
                        t.text_secondary
                    },
                ))
                .child(
                    canvas(
                        move |b, _, cx| add_center.update(cx, |c, _| *c = b.center().y),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .into_any_element(),
        );

        // UI-268: slides in from 60px left and fades, 220ms.
        let slide = transition(
            window,
            cx,
            "rail-slide",
            if shown { 1.0 } else { 0.0 },
            DUR_MED,
            motion::ease_out,
        );
        let fade = transition(
            window,
            cx,
            "rail-fade",
            if shown { 1.0 } else { 0.0 },
            DUR_MED,
            motion::ease,
        );
        let main_h = window.viewport_size().height - TITLEBAR_H;
        let rail_id = cx.entity();
        let leave_rail = cx.entity();

        let rail = div()
            .id("rail")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .w(RAIL_W)
            .overflow_y_scroll()
            .map(|d| {
                if pinned {
                    d.h(main_h)
                        .py(px(8.))
                        .border_r(px(0.5))
                        .border_color(t.separator)
                        .bg(t.bg_window)
                } else {
                    d.translate(px(-60. * (1. - slide)), px(0.))
                        .max_h(main_h - RAIL_INSET * 2.)
                        .py(px(6.))
                        .rounded(RADIUS_LG)
                        .map(|d| frosted(d, t.rail_surface, 24., 1.8))
                        .shadow(t.lg())
                        .group_opacity(fade)
                }
            })
            .when(shown, |d| {
                d.occlude()
                    .on_hover(move |h, _, cx| {
                        rail_id.update(cx, |r, cx| {
                            r.over_rail = *h;
                            if pinned {
                                return;
                            }
                            if *h {
                                // UI-266: entering keeps it open.
                                r.keep_open();
                                r.open = true;
                            } else {
                                r.hide(cx);
                            }
                            cx.notify();
                        });
                    })
                    .on_scroll(move |_, _, cx| {
                        // UI-273: scrolling hides the hint.
                        leave_rail.update(cx, |r, cx| {
                            r.hint = None;
                            cx.notify();
                        });
                    })
                    .children(children)
            });

        let priority = if self.numbered { 600 } else { 40 };
        let edge = (!pinned).then(|| {
            let enter_rail = cx.entity();
            let leave = cx.entity();
            deferred(
                div()
                    .id("rail-edge")
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(6.))
                    .h(main_h)
                    .on_mouse_move(move |e, _, cx| {
                        let held = e.pressed_button.is_some();
                        enter_rail.update(cx, |r, cx| r.reveal(held, cx));
                    })
                    .on_hover(move |h, _, cx| {
                        if !*h {
                            leave.update(cx, ProjectRail::hide);
                        }
                    }),
            )
            .with_priority(41)
        });
        let hint = if shown {
            self.hint_card(pinned, window, cx)
        } else {
            None
        };

        // Floating, it's as tall as its items and centred in the main area
        // (`height: fit-content; margin: auto 0`); pinned, it's the left column.
        let column = div()
            .absolute()
            .top_0()
            .left(if pinned { px(0.) } else { RAIL_INSET })
            .h(main_h)
            .flex()
            .flex_col()
            .justify_center()
            .child(rail);
        div()
            .absolute()
            .inset_0()
            .child(deferred(column).with_priority(priority))
            .children(edge)
            .children(hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail() -> HintDetail<'static> {
        HintDetail {
            group: None,
            server: None,
            stats: None,
            not_in_focus: false,
            number: None,
        }
    }

    /// UI-272
    #[test]
    fn joins_what_applies() {
        let stats = ProjectStats {
            waiting: 1,
            working: 2,
            ..ProjectStats::default()
        };
        assert_eq!(detail().text(), None);
        let hint = HintDetail {
            group: Some("Clients"),
            stats: Some(&stats),
            not_in_focus: true,
            number: Some(3),
            ..detail()
        };
        assert_eq!(
            hint.text().unwrap(),
            format!("Clients · 1 waiting · 2 working · Not in focus · {}", keys("⌘3"))
        );
        let tenth = HintDetail {
            number: Some(10),
            ..detail()
        };
        assert_eq!(tenth.text(), None);
    }

    /// UI-340
    #[test]
    fn says_which_server_a_project_is_on() {
        let on = |away| HintDetail {
            group: Some("Clients"),
            server: Some(ServerBadge {
                name: "Studio".into(),
                away,
            }),
            number: Some(1),
            ..detail()
        };
        let one = keys("⌘1");
        assert_eq!(on(false).text().unwrap(), format!("Clients · On Studio · {one}"));
        assert_eq!(
            on(true).text().unwrap(),
            format!("Clients · On Studio · Reconnecting… · {one}")
        );
    }
}
