//! The projects grid (`ProjectsView.tsx`, `ProjectsView.css`;
//! UI-183…UI-252): the ungrouped projects, one section per group, the
//! archived ones, focus filtering, Finder-style selection and keys, rename,
//! context menus, the remove dialog, the group picker, drags between
//! sections, folders dropped from the OS, and the toast.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, DragMoveEvent, Entity, ExternalPaths, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Point, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Transformation, Window, canvas,
    div, px, radians,
};
use nodus_domain::focus::shows_in_focus;
use nodus_domain::{FolderColor, GroupPatch, Project, ProjectPatch};

use crate::components::button::{Kind, button};
use crate::components::color_swatches::color_swatches;
use crate::components::context_menu::{Menu, MenuItem, MenuLayer};
use crate::components::folder_icon::{Glyph, folder_icon};
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::scroll::scroll_area;
use crate::components::spaced_text::spaced_text;
use crate::components::text_field::{focus_all, input, new_input};
use crate::components::tooltip::{TooltipPress, native_tooltip, pressable_tooltip};
use crate::components::tween::{enter, transition};
use crate::platform::{Platform, keys};
use crate::projects::folder_item::{FolderItem, ServerBadge, rename_frame};
use crate::state::{AppState, Route, group_projects, sort_projects};
use crate::theme::tokens::*;
use crate::theme::{self, fonts, motion};

/// The drop-target key of the ungrouped section; groups use their id.
const UNGROUPED: &str = "ungrouped";

/// A folder being dragged onto another section.
#[derive(Clone)]
pub struct DraggedProject {
    pub id: String,
    pub project: Project,
}

/// What the grid knows only while it's showing; the selection and toast
/// live in [`AppState`], as they survive navigation (UI-204).
pub struct ProjectsView {
    state: Entity<AppState>,
    menus: Entity<MenuLayer>,
    scroll: ScrollHandle,
    loaded: bool,
    renaming: Option<(String, Entity<InputState>)>,
    renaming_group: Option<(String, Entity<InputState>)>,
    remove_target: Option<Project>,
    remove_focus: FocusHandle,
    cancel_focus: FocusHandle,
    move_target: Option<(Project, Entity<InputState>)>,
    menu_open: Rc<Cell<bool>>,
    dragging_files: bool,
    drag_id: Option<String>,
    new_ids: HashSet<String>,
    seen: Option<HashSet<String>>,
    /// Each listed folder's bounds last frame, for ↑/↓ and keeping the
    /// selection in view.
    bounds: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    scrolled_to: Option<String>,
    modal_serial: usize,
    /// The toast, sinking away once gone (MOTION-011).
    toast: crate::components::presence::Presence<(String, u64)>,
    _pop_timer: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectsView {
    pub fn new(state: Entity<AppState>, menus: Entity<MenuLayer>, _: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        // UI-183: showing the grid refreshes the projects.
        let refresh = state.update(cx, AppState::refresh_projects);
        cx.spawn(async move |this, cx| {
            refresh.await;
            let _ = this.update(cx, |view, cx| {
                view.loaded = true;
                view.track_new(cx);
                cx.notify();
            });
        })
        .detach();
        let subscriptions = vec![cx.observe(&state, |view, _, cx| {
            view.track_new(cx);
            cx.notify();
        })];
        Self {
            state,
            menus,
            scroll: ScrollHandle::new(),
            loaded: false,
            renaming: None,
            renaming_group: None,
            remove_target: None,
            remove_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            move_target: None,
            menu_open: Rc::new(Cell::new(false)),
            dragging_files: false,
            drag_id: None,
            new_ids: HashSet::new(),
            seen: None,
            bounds: Rc::default(),
            scrolled_to: None,
            modal_serial: 0,
            _pop_timer: None,
            toast: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    /// UI-230: folders that appear after the first refresh pop in once.
    fn track_new(&mut self, cx: &mut Context<'_, Self>) {
        if !self.loaded {
            return;
        }
        let ids: Vec<String> = self.state.read(cx).projects.iter().map(|p| p.id.clone()).collect();
        let Some(seen) = &mut self.seen else {
            self.seen = Some(ids.into_iter().collect());
            return;
        };
        let fresh: Vec<String> = ids.into_iter().filter(|id| !seen.contains(id)).collect();
        if fresh.is_empty() {
            return;
        }
        seen.extend(fresh.iter().cloned());
        self.new_ids.extend(fresh.iter().cloned());
        self._pop_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(700)).await;
            let _ = this.update(cx, |view, cx| {
                for id in &fresh {
                    view.new_ids.remove(id);
                }
                cx.notify();
            });
        }));
    }

    /// The folders in reading order: ungrouped, each expanded group, archived.
    fn lists(&self, cx: &App) -> Lists {
        let s = self.state.read(cx);
        let sorted = sort_projects(&s.projects);
        let focusing = s.focusing();
        let active: Vec<Project> = sorted.iter().filter(|p| p.archived != Some(true)).cloned().collect();
        let listed: Vec<Project> = if focusing {
            active
                .iter()
                .filter(|p| shows_in_focus(p, s.stats.get(&p.id), None))
                .cloned()
                .collect()
        } else {
            active.clone()
        };
        let archived: Vec<Project> = if focusing {
            Vec::new()
        } else {
            sorted.iter().filter(|p| p.archived == Some(true)).cloned().collect()
        };
        let hidden = sorted.len() - listed.len() - archived.len();
        let (ungrouped, mut by_group) = group_projects(&listed, &s.groups);
        if focusing {
            // UI-197: groups with nothing in focus keep out of the way.
            by_group.retain(|(_, members)| !members.is_empty());
        }
        let ordered = ungrouped
            .iter()
            .chain(
                by_group
                    .iter()
                    .filter(|(g, _)| g.collapsed != Some(true))
                    .flat_map(|(_, ps)| ps.iter()),
            )
            .chain(archived.iter())
            .cloned()
            .collect();
        Lists {
            total: sorted.len(),
            active_count: active.len(),
            listed_count: listed.len(),
            ungrouped,
            by_group,
            archived,
            hidden,
            focusing,
            ordered,
        }
    }

    fn busy(&self) -> bool {
        self.menu_open.get()
            || self.remove_target.is_some()
            || self.move_target.is_some()
            || self.renaming.is_some()
            || self.renaming_group.is_some()
    }

    /// The window's keys while the grid shows (UI-208…UI-215, UI-250).
    pub fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let key = e.keystroke.key.as_str();
        let m = e.keystroke.modifiers;
        if let Some(target) = self.remove_target.clone() {
            match key {
                "escape" => self.remove_target = None,
                "enter" | "space" if self.cancel_focus.is_focused(window) => self.remove_target = None,
                "enter" | "space" => {
                    self.remove_target = None;
                    self.state.update(cx, |s, cx| s.remove_project(target.id, cx));
                }
                "tab" => {
                    let next = if self.remove_focus.is_focused(window) {
                        &self.cancel_focus
                    } else {
                        &self.remove_focus
                    };
                    window.focus(next, cx);
                }
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if self.move_target.is_some() {
            if key == "escape" {
                self.move_target = None;
                cx.notify();
                return true;
            }
            return false;
        }
        if self.busy() {
            return false;
        }
        let platform = Platform::CURRENT;
        let lists = self.lists(cx);
        let current = self.state.read(cx).selected_id.clone();
        let index = current
            .as_ref()
            .and_then(|id| lists.ordered.iter().position(|p| &p.id == id));
        let project = index.map(|i| lists.ordered[i].clone());

        if platform.mod_key(&m) && (key == "o" || key == "down") {
            if let Some(project) = project {
                self.state.update(cx, |s, cx| s.navigate(Route::Board(project.id), cx));
                return true;
            }
            return false;
        }
        if platform.mod_key(&m) && key == "backspace" {
            if let Some(project) = project {
                self.open_remove(project, window, cx);
                return true;
            }
            return false;
        }
        if m.platform || m.control || m.alt {
            return false;
        }
        match key {
            "enter" => {
                let Some(project) = project else { return false };
                self.start_rename(&project, window, cx);
                true
            }
            "escape" => {
                self.state.update(cx, |s, cx| s.select(None, cx));
                false
            }
            "left" | "right" | "up" | "down" if !lists.ordered.is_empty() => {
                let Some(index) = index else {
                    let first = lists.ordered[0].id.clone();
                    self.state.update(cx, |s, cx| s.select(Some(first), cx));
                    return true;
                };
                let next = match key {
                    "left" => index.checked_sub(1).map(|i| lists.ordered[i].id.clone()),
                    "right" => lists.ordered.get(index + 1).map(|p| p.id.clone()),
                    _ => folder_in_next_row(
                        &self.bounds.borrow(),
                        &lists.ordered,
                        &lists.ordered[index].id,
                        if key == "down" { 1.0 } else { -1.0 },
                    ),
                };
                if let Some(next) = next {
                    self.state.update(cx, |s, cx| s.select(Some(next), cx));
                }
                true
            }
            _ => false,
        }
    }

    fn open_remove(&mut self, project: Project, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.remove_target = Some(project);
        self.modal_serial += 1;
        // UI-248: Remove has the keyboard focus.
        window.focus(&self.remove_focus, cx);
        cx.notify();
    }

    fn open_picker(&mut self, project: Project, window: &mut Window, cx: &mut Context<'_, Self>) {
        let field = new_input("", "New group name", false, window, cx);
        let sub = cx.subscribe_in(&field, window, |view, field, e: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = e {
                view.create_group(&field.read(cx).value(), cx);
            }
            cx.notify();
        });
        self._subscriptions.push(sub);
        // UI-246: with no groups yet the name field takes focus.
        if self.state.read(cx).groups.is_empty() {
            field.update(cx, |f, cx| f.focus(window, cx));
        }
        self.move_target = Some((project, field));
        self.modal_serial += 1;
        cx.notify();
    }

    fn create_group(&mut self, name: &str, cx: &mut Context<'_, Self>) {
        let trimmed = name.trim().to_owned();
        let Some((project, _)) = self.move_target.clone() else {
            return;
        };
        if trimmed.is_empty() {
            return;
        }
        self.move_target = None;
        self.state
            .update(cx, |s, cx| s.create_group_with(project.id, trimmed, cx));
        cx.notify();
    }

    /// UI-237: the name becomes a field with its text selected.
    fn start_rename(&mut self, project: &Project, window: &mut Window, cx: &mut Context<'_, Self>) {
        let field = new_input(&project.name, "", false, window, cx);
        let id = project.id.clone();
        let initial = project.name.clone();
        let done = Rc::new(Cell::new(false));
        let sub = cx.subscribe_in(&field, window, move |view, field, e: &InputEvent, _, cx| {
            // UI-238: Enter or leaving the field commits, once.
            if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) && !done.replace(true) {
                let name = field.read(cx).value().to_string();
                view.renaming = None;
                if let Some(name) = changed_name(&name, &initial) {
                    let id = id.clone();
                    view.state.update(cx, |s, cx| {
                        s.update_project(
                            id,
                            ProjectPatch {
                                name: Some(name),
                                ..ProjectPatch::default()
                            },
                            cx,
                        );
                    });
                }
                cx.notify();
            }
        });
        self._subscriptions.push(sub);
        focus_all(&field, window, cx);
        self.renaming = Some((project.id.clone(), field));
        cx.notify();
    }

    /// UI-222: the group heading becomes a field.
    fn start_group_rename(&mut self, id: String, name: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let field = new_input(&name, "", false, window, cx);
        let done = Rc::new(Cell::new(false));
        let group_id = id.clone();
        let sub = cx.subscribe_in(&field, window, move |view, field, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) && !done.replace(true) {
                let value = field.read(cx).value().to_string();
                view.renaming_group = None;
                if let Some(name) = changed_name(&value, &name) {
                    let id = group_id.clone();
                    view.state.update(cx, |s, cx| {
                        s.update_group(
                            id,
                            GroupPatch {
                                name: Some(name),
                                ..GroupPatch::default()
                            },
                            cx,
                        );
                    });
                }
                cx.notify();
            }
        });
        self._subscriptions.push(sub);
        focus_all(&field, window, cx);
        self.renaming_group = Some((id, field));
        cx.notify();
    }

    /// Escape in a rename field cancels it (UI-238).
    fn cancel_renames(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.renaming = None;
        self.renaming_group = None;
        window.blur(cx);
        cx.notify();
    }

    fn open_menu(&mut self, at: Point<Pixels>, items: Vec<MenuItem>, cx: &mut Context<'_, Self>) {
        let flag = self.menu_open.clone();
        flag.set(true);
        let mut menu = Menu::at(at, items);
        menu.on_close = Some(Rc::new(move |_, _| flag.set(false)));
        self.menus.update(cx, |m, cx| m.open(menu, cx));
    }

    /// UI-216: a folder's menu.
    fn project_menu(&mut self, project: &Project, at: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let view = cx.entity();
        let state = self.state.clone();
        let platform = Platform::CURRENT;
        let focus_feature = state.read(cx).focus_feature();
        let archived = project.archived == Some(true);
        let id = project.id.clone();
        let mut items = Vec::new();
        {
            let state = state.clone();
            let id = id.clone();
            items.push(
                MenuItem::item("Open", Icon::FolderOpen, move |_, cx| {
                    state.update(cx, |s, cx| s.navigate(Route::Board(id.clone()), cx));
                })
                .hint(keys("⌘O")),
            );
        }
        {
            let view = view.clone();
            let project = project.clone();
            items.push(
                MenuItem::item("Rename", Icon::Pencil, move |window, cx| {
                    view.update(cx, |v, cx| v.start_rename(&project, window, cx));
                })
                .hint(keys("↩")),
            );
        }
        if !archived && focus_feature {
            let focused = project.focused == Some(true);
            let state = state.clone();
            let id = id.clone();
            items.push(MenuItem::item(
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
            ));
        }
        items.push(MenuItem::Separator);
        {
            let state = state.clone();
            let menus = self.menus.clone();
            let id = id.clone();
            let color = project.color;
            items.push(MenuItem::Custom(Rc::new(move |window, cx| {
                let state = state.clone();
                let menus = menus.clone();
                let id = id.clone();
                // UI-217: picking closes the menu and saves the colour.
                let pick: crate::components::ColorHandler = Rc::new(move |color: FolderColor, window, cx| {
                    menus.update(cx, |m, cx| m.close(window, cx));
                    state.update(cx, |s, cx| {
                        s.update_project(
                            id.clone(),
                            ProjectPatch {
                                color: Some(color),
                                ..ProjectPatch::default()
                            },
                            cx,
                        );
                    });
                });
                color_swatches(color, &pick, window, cx)
            })));
        }
        items.push(MenuItem::Separator);
        if !archived {
            let view = view.clone();
            let project = project.clone();
            items.push(MenuItem::item(
                "Move to Group…",
                Icon::FolderInput,
                move |window, cx| {
                    view.update(cx, |v, cx| v.open_picker(project.clone(), window, cx));
                },
            ));
        }
        // SRV-163: a server project has no folder here.
        if project.server.is_none() {
            let state = state.clone();
            let id = id.clone();
            items.push(MenuItem::item(
                if platform.is_mac() {
                    "Reveal in Finder"
                } else {
                    "Show in File Explorer"
                },
                Icon::Search,
                move |_, cx| state.update(cx, |s, cx| s.reveal(id.clone(), cx)),
            ));
        }
        items.push(MenuItem::Separator);
        {
            let state = state.clone();
            let id = id.clone();
            items.push(MenuItem::item(
                if archived { "Unarchive" } else { "Archive" },
                if archived { Icon::ArchiveRestore } else { Icon::Archive },
                move |_, cx| {
                    // UI-218: an archived project leaves the focus with it.
                    let patch = if archived {
                        ProjectPatch {
                            archived: Some(false),
                            ..ProjectPatch::default()
                        }
                    } else {
                        ProjectPatch {
                            archived: Some(true),
                            focused: Some(false),
                            ..ProjectPatch::default()
                        }
                    };
                    state.update(cx, |s, cx| s.update_project(id.clone(), patch.clone(), cx));
                },
            ));
        }
        {
            let view = view.clone();
            let project = project.clone();
            items.push(
                MenuItem::item("Remove from Nodus…", Icon::Trash2, move |window, cx| {
                    view.update(cx, |v, cx| v.open_remove(project.clone(), window, cx));
                })
                .danger(),
            );
        }
        self.open_menu(at, items, cx);
    }

    /// UI-221: a group heading's menu.
    fn group_menu(&mut self, id: String, name: String, at: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let view = cx.entity();
        let state = self.state.clone();
        let rename_id = id.clone();
        let items = vec![
            MenuItem::item("Rename Group", Icon::Pencil, move |window, cx| {
                view.update(cx, |v, cx| {
                    v.start_group_rename(rename_id.clone(), name.clone(), window, cx);
                });
            }),
            MenuItem::Separator,
            MenuItem::item("Delete Group", Icon::Trash2, move |_, cx| {
                state.update(cx, |s, cx| s.remove_group(id.clone(), cx));
            })
            .danger(),
        ];
        self.open_menu(at, items, cx);
    }

    /// One folder, with its pointer handling.
    fn folder(
        &mut self,
        project: &Project,
        lists: &Lists,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let s = self.state.read(cx);
        let selected = s.selected_id.as_deref() == Some(project.id.as_str());
        let stats = s.stats.get(&project.id).copied();
        let focus_feature = s.focus_feature();
        // SRV-162: a server project's badge names its server.
        let server = project.server.as_ref().map(|server_id| ServerBadge {
            name: s
                .server_of(project)
                .map_or_else(|| server_id.clone(), |server| server.name.clone()),
            away: s.server_away(project),
        });
        let id = SharedString::from(format!("folder-{}", project.id));
        let hovered = window.use_keyed_state((id.clone(), 0usize), cx, |_, _| false);
        let pressed = window.use_keyed_state((id.clone(), 2usize), cx, |_, _| TooltipPress::default());
        let pop = if self.new_ids.contains(&project.id) {
            enter(
                window,
                cx,
                (id.clone(), 1usize),
                Duration::from_millis(520),
                motion::ease_spring,
            )
        } else {
            1.0
        };
        let rename = self
            .renaming
            .as_ref()
            .filter(|(rid, _)| rid == &project.id)
            .map(|(_, field)| {
                let t = theme::tokens(cx);
                let view = cx.entity();
                // UI-239: keys, presses and double-clicks stay in the field.
                div()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_key_down(move |e: &KeyDownEvent, window, cx| {
                        if e.keystroke.key == "escape" {
                            cx.stop_propagation();
                            view.update(cx, |v, cx| v.cancel_renames(window, cx));
                        }
                    })
                    .child(rename_frame(input(field, px(16.9), true), t))
                    .into_any_element()
            });
        // CSS :active: from a primary press until its release, wherever.
        let down = window.use_keyed_state((id.clone(), 3usize), cx, |_, _| false);
        let item = FolderItem {
            pressed: *down.read(cx),
            project,
            stats: stats.as_ref(),
            selected,
            focused: focus_feature && project.focused == Some(true) && project.archived != Some(true),
            dimmed: lists.focusing && project.focused != Some(true),
            rename,
            pop,
            server: server.clone(),
        }
        .render(&id.clone().into(), &hovered, window, cx);
        // UI-232: the folder's path; a server project has none here.
        let tip = server
            .as_ref()
            .map_or_else(|| project.repo_path.clone(), ServerBadge::tooltip);

        let state = self.state.clone();
        let select_id = project.id.clone();
        let open_id = project.id.clone();
        let menu_project = project.clone();
        let bounds = self.bounds.clone();
        let bounds_id = project.id.clone();
        let draggable = project.archived != Some(true) && self.renaming.as_ref().is_none_or(|(r, _)| r != &project.id);
        let dragged = DraggedProject {
            id: project.id.clone(),
            project: project.clone(),
        };
        let view = cx.entity();

        div()
            .id(id)
            .relative()
            .child(item)
            .tooltip(pressable_tooltip(tip.into(), pressed.clone()))
            .capture_any_mouse_down({
                let pressed = pressed.clone();
                move |_, _, cx| pressed.update(cx, |p, _| p.press())
            })
            .on_mouse_move({
                let pressed = pressed.clone();
                move |_, _, cx| pressed.update(cx, |p, _| p.moved())
            })
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
                pressed.update(cx, |p, _| p.hover(*h));
            })
            // UI-202: a primary press selects.
            .on_mouse_down(MouseButton::Left, {
                let down = down.clone();
                move |_, _, cx| {
                    cx.stop_propagation();
                    down.update(cx, |d, cx| {
                        *d = true;
                        cx.notify();
                    });
                    let id = select_id.clone();
                    state.update(cx, |s, cx| s.select(Some(id), cx));
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let down = down.clone();
                move |_, _, cx| {
                    down.update(cx, |d, cx| {
                        *d = false;
                        cx.notify();
                    });
                }
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                down.update(cx, |d, cx| {
                    *d = false;
                    cx.notify();
                });
            })
            // UI-207: a double-click opens.
            .on_click({
                let state = self.state.clone();
                move |e, _, cx| {
                    if e.click_count() == 2 {
                        let id = open_id.clone();
                        state.update(cx, |s, cx| s.navigate(Route::Board(id), cx));
                    }
                }
            })
            // UI-216: a right-click selects and opens its menu.
            .on_mouse_down(MouseButton::Right, {
                let view = view.clone();
                let state = self.state.clone();
                move |e, _, cx| {
                    cx.stop_propagation();
                    let id = menu_project.id.clone();
                    state.update(cx, |s, cx| s.select(Some(id), cx));
                    let project = menu_project.clone();
                    view.update(cx, |v, cx| v.project_menu(&project, e.position, cx));
                }
            })
            .when(draggable, |d| {
                let view = view.clone();
                d.on_drag(dragged, move |dragged, _, _, cx| {
                    let id = dragged.id.clone();
                    view.update(cx, |v, cx| {
                        v.drag_id = Some(id);
                        cx.notify();
                    });
                    let project = dragged.project.clone();
                    cx.new(|_| DragPreview { project })
                })
            })
            .child(
                canvas(
                    move |b, _, _| {
                        bounds.borrow_mut().insert(bounds_id.clone(), b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    /// A section that takes folders dragged onto it (UI-224…UI-226).
    fn drop_section(
        &self,
        key: &str,
        own_section: impl Fn(&str) -> String + 'static,
        t: Tokens,
    ) -> gpui::Stateful<gpui::Div> {
        let key_over = key.to_owned();
        let key_drop = key.to_owned();
        let state = self.state.clone();
        let own = Rc::new(own_section);
        let own_drop = own.clone();
        div()
            .id(SharedString::from(format!("section-{key}")))
            .mx(px(-10.))
            .px(px(10.))
            .pb(px(6.))
            .rounded(RADIUS_LG)
            // UI-225: highlighted only when it isn't the folder's own section.
            .drag_over::<DraggedProject>(move |style, dragged, _, _| {
                if own(&dragged.id) == key_over {
                    style
                } else {
                    style
                        .bg(t.archive_drop)
                        .shadow(vec![inset_shadow(0., 0., 0., 1.5, t.drop_ring)])
                }
            })
            .on_drop::<DraggedProject>(move |dragged, _, cx| {
                if own_drop(&dragged.id) == key_drop {
                    return;
                }
                let group = (key_drop != UNGROUPED).then(|| key_drop.clone());
                let id = dragged.id.clone();
                state.update(cx, |s, cx| s.move_to_group(id, group, cx));
            })
    }

    fn render_remove(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let target = self.remove_target.clone()?;
        let t = theme::tokens(cx);
        let view = cx.entity();
        let cancel = view.clone();
        let confirm_view = view.clone();
        let target_id = target.id.clone();
        let state = self.state.clone();
        let server = target.server.is_some();
        let body = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .pt(px(22.))
                    .px(px(22.))
                    .pb(px(8.))
                    .child(folder_icon(56., Some(target.color)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .mt(px(4.))
                                    .mb(px(6.))
                                    .text_size(TEXT_LG)
                                    .line_height(px(21.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Remove “{}”?", target.name)),
                            )
                            .child(div().mb(px(8.)).text_color(t.text_secondary).child(if server {
                                // SRV-161: only this grid's link goes.
                                "It leaves your grid. The project, its board and its sessions stay on its server, and Show Server Projects… brings it back."
                            } else {
                                "The project and its board layout are removed from Nodus. The repository and its Claude session transcripts stay on disk, untouched."
                            }))
                            .when(!server, |d| d.child(
                                // UI-008: the path is selectable.
                                div()
                                    .mb(px(8.))
                                    .font_family(fonts::mono())
                                    .text_size(TEXT_XS)
                                    .line_height(px(15.4))
                                    .text_color(t.text_tertiary)
                                    .cursor_text()
                                    .truncate()
                                    .child(crate::components::selectable::selectable(
                                        "pv-confirm-path",
                                        target.repo_path.clone(),
                                        window,
                                        cx,
                                    )),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(12.))
                    .px(px(22.))
                    .pb(px(18.))
                    .child(
                        button("remove-cancel")
                            .label("Cancel")
                            .focus(&self.cancel_focus)
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |v, cx| {
                                    v.remove_target = None;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        button("remove-confirm")
                            .kind(Kind::Danger)
                            .label("Remove")
                            .focus(&self.remove_focus)
                            .on_click(move |_, _, cx| {
                                confirm_view.update(cx, |v, cx| {
                                    v.remove_target = None;
                                    cx.notify();
                                });
                                let id = target_id.clone();
                                state.update(cx, |s, cx| s.remove_project(id, cx));
                            }),
                    ),
            );
        Some(
            Modal::new(("remove-dialog", self.modal_serial), px(440.), move |_, cx| {
                view.update(cx, |v, cx| {
                    v.remove_target = None;
                    cx.notify();
                });
            })
            .render(body, window, cx),
        )
    }

    fn render_picker(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let (project, field) = self.move_target.clone()?;
        let t = theme::tokens(cx);
        let s = self.state.read(cx);
        let groups = crate::state::sort_groups(&s.groups);
        let current = project.group_id.clone().filter(|g| s.groups.iter().any(|x| &x.id == g));
        let view = cx.entity();
        let mut rows: Vec<(Option<String>, String)> =
            groups.iter().map(|g| (Some(g.id.clone()), g.name.clone())).collect();
        rows.push((None, "No Group".into()));
        let has_groups = !groups.is_empty();
        let value = field.read(cx).value().to_string();
        let blank = value.trim().is_empty();
        let field_focused = field.read(cx).focus_handle(cx).is_focused(window);

        let list = has_groups.then(|| {
            div()
                .id("gp-list")
                .flex()
                .flex_col()
                .gap(px(1.))
                .max_h(px(260.))
                .mb(px(12.))
                .overflow_y_scroll()
                .children(rows.into_iter().enumerate().map(|(i, (id, name))| {
                    let is_current = id == current;
                    let view = view.clone();
                    let state = self.state.clone();
                    let project_id = project.id.clone();
                    let current = current.clone();
                    div()
                        .id(("gp-row", i))
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(RADIUS_SM)
                        .text_color(t.text_secondary)
                        .when(is_current, |d| d.bg(t.bg_selected))
                        .when(!is_current, |d| {
                            d.hover(|s| s.bg(t.bg_hover)).active(|s| s.bg(t.bg_pressed))
                        })
                        .on_click(move |_, _, cx| {
                            // UI-244
                            view.update(cx, |v, cx| {
                                v.move_target = None;
                                cx.notify();
                            });
                            if id != current {
                                let id = id.clone();
                                let project_id = project_id.clone();
                                state.update(cx, |s, cx| s.move_to_group(project_id, id, cx));
                            }
                        })
                        .child(icon(
                            if i < groups.len() {
                                Icon::Folders
                            } else {
                                Icon::FolderMinus
                            },
                            15.,
                            2.,
                            t.text_secondary,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(t.text_primary)
                                .child(name),
                        )
                        .when(is_current, |d| d.child(icon(Icon::Check, 15., 2.4, t.accent)))
                }))
        });

        let create_view = view.clone();
        let create_field = field.clone();
        let body =
            div()
                .flex()
                .flex_col()
                .pt(px(18.))
                .px(px(16.))
                .pb(px(16.))
                .child(
                    div()
                        .mx(px(4.))
                        .mb(px(12.))
                        .text_size(TEXT_LG)
                        .line_height(px(21.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(format!("Move “{}” to", project.name)),
                )
                .children(list)
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(28.))
                                .px(px(9.))
                                .flex()
                                .items_center()
                                .border(px(0.5))
                                .border_color(if field_focused { t.accent } else { t.border_strong })
                                .rounded(RADIUS_SM)
                                .bg(t.bg_sunken)
                                .when(field_focused, |d| d.shadow(vec![ring(3., t.focus_ring)]))
                                .child(input(&field, px(18.), false)),
                        )
                        .child(button("gp-create").primary().label("Create").disabled(blank).on_click(
                            move |_, _, cx| {
                                let name = create_field.read(cx).value().to_string();
                                create_view.update(cx, |v, cx| v.create_group(&name, cx));
                            },
                        )),
                );
        Some(
            Modal::new(("group-picker", self.modal_serial), px(340.), move |_, cx| {
                view.update(cx, |v, cx| {
                    v.move_target = None;
                    cx.notify();
                });
            })
            .render(body, window, cx),
        )
    }
}

struct Lists {
    total: usize,
    active_count: usize,
    listed_count: usize,
    ungrouped: Vec<Project>,
    by_group: Vec<(nodus_domain::ProjectGroup, Vec<Project>)>,
    archived: Vec<Project>,
    hidden: usize,
    focusing: bool,
    ordered: Vec<Project>,
}

/// The trimmed name when it's non-empty and new (UI-238).
pub fn changed_name(value: &str, initial: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty() && trimmed != initial).then(|| trimmed.to_owned())
}

/// UI-214: the folder in the next row up (−1) or down (1) whose left edge is
/// closest to the current one's, first in reading order on a tie.
pub fn folder_in_next_row(
    bounds: &HashMap<String, Bounds<Pixels>>,
    ordered: &[Project],
    from: &str,
    dir: f32,
) -> Option<String> {
    let items: Vec<(&str, Bounds<Pixels>)> = ordered
        .iter()
        .filter_map(|p| bounds.get(&p.id).map(|b| (p.id.as_str(), *b)))
        .collect();
    let from = items.iter().find(|(id, _)| *id == from)?.1;
    let beyond: Vec<&(&str, Bounds<Pixels>)> = items
        .iter()
        .filter(|(_, b)| f32::from(b.top() - from.top()) * dir > 1.0)
        .collect();
    let tops = beyond.iter().map(|(_, b)| f32::from(b.top()));
    let row_top = if dir > 0.0 {
        tops.fold(f32::INFINITY, f32::min)
    } else {
        tops.fold(f32::NEG_INFINITY, f32::max)
    };
    let offset = |b: &Bounds<Pixels>| f32::from((b.left() - from.left()).abs());
    beyond
        .into_iter()
        .filter(|(_, b)| (f32::from(b.top()) - row_top).abs() < 1.0)
        .fold(None::<&(&str, Bounds<Pixels>)>, |best, item| match best {
            Some(b) if offset(&b.1) <= offset(&item.1) => Some(b),
            _ => Some(item),
        })
        .map(|(id, _)| (*id).to_owned())
}

/// What follows the pointer while a folder is dragged.
struct DragPreview {
    project: Project,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        div()
            .opacity(0.7)
            .flex()
            .flex_col()
            .items_center()
            .w(px(144.))
            .child(
                folder_icon(112., Some(self.project.color)).glyph(crate::components::project_glyph(&self.project.name)),
            )
            .child(
                div()
                    .text_color(t.text_primary)
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.project.name.clone()),
            )
    }
}

impl Render for ProjectsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let lists = self.lists(cx);
        if !cx.has_active_drag() {
            self.drag_id = None;
            self.dragging_files = false;
        }
        let (toast, selected, groups_known) = {
            let s = self.state.read(cx);
            (s.toast.clone(), s.selected_id.clone(), s.groups.clone())
        };
        let focus_mode = self.state.read(cx).focus_mode();

        // UI-206: scroll the least needed to show the selected folder.
        if selected != self.scrolled_to {
            self.scrolled_to = selected.clone();
            if let Some(id) = &selected
                && let Some(b) = self.bounds.borrow().get(id).copied()
            {
                let view_b = self.scroll.bounds();
                let mut offset = self.scroll.offset();
                if b.top() < view_b.top() {
                    offset.y += view_b.top() - b.top();
                } else if b.bottom() > view_b.bottom() {
                    offset.y -= b.bottom() - view_b.bottom();
                }
                self.scroll.set_offset(offset);
            }
        }
        self.bounds.borrow_mut().clear();

        // The section a folder sits in, for the drop rules.
        let section_of = {
            let groups: Vec<String> = lists.by_group.iter().map(|(g, _)| g.id.clone()).collect();
            let projects = self.state.read(cx).projects.clone();
            move |id: &str| -> String {
                projects
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| p.group_id.clone())
                    .filter(|g| groups.contains(g))
                    .unwrap_or_else(|| UNGROUPED.to_owned())
            }
        };
        let drag_from = self.drag_id.as_deref().map(&section_of);

        let heading_font = gpui::Font {
            family: fonts::DISPLAY.into(),
            weight: FontWeight::SEMIBOLD,
            ..gpui::Font::default()
        };

        let content = if self.loaded && lists.total == 0 {
            None
        } else if lists.total > 0 {
            // The top section: heading, then the ungrouped folders.
            let ungrouped: Vec<AnyElement> = lists
                .ungrouped
                .clone()
                .iter()
                .map(|p| self.folder(p, &lists, window, cx))
                .collect();
            let top = self
                .drop_section(UNGROUPED, section_of.clone(), t)
                .child(heading_row(
                    spaced_text(
                        if lists.focusing { "In Focus" } else { "Projects" },
                        heading_font.clone(),
                        TEXT_2XL,
                        px(-0.52),
                        px(36.4),
                        t.text_primary,
                    )
                    .into_any_element(),
                    lists.listed_count,
                    TEXT_LG,
                    t,
                    22.,
                    0.,
                ))
                .map(|d| {
                    if !ungrouped.is_empty() {
                        d.child(grid(ungrouped))
                    } else if drag_from.as_deref().is_some_and(|f| f != UNGROUPED) {
                        d.child(placeholder("Drop here to take it out of its group", t))
                    } else if lists.active_count == 0 {
                        d.child(
                            div()
                                .ml(px(6.))
                                .text_color(t.text_tertiary)
                                .child("All projects are archived."),
                        )
                    } else {
                        d
                    }
                });

            let mut sections = vec![top.into_any_element()];
            for (group, members) in lists.by_group.clone() {
                let collapsed = group.collapsed == Some(true);
                let folders: Vec<AnyElement> = if collapsed {
                    Vec::new()
                } else {
                    members.iter().map(|p| self.folder(p, &lists, window, cx)).collect()
                };
                let renaming = self
                    .renaming_group
                    .as_ref()
                    .filter(|(id, _)| id == &group.id)
                    .map(|(_, f)| f.clone());
                let chevron = transition(
                    window,
                    cx,
                    (SharedString::from(format!("chevron-{}", group.id)), 0usize),
                    if collapsed { -90.0 } else { 0.0 },
                    DUR_FAST,
                    motion::ease_out,
                );
                let toggle_state = self.state.clone();
                let toggle_id = group.id.clone();
                let menu_view = cx.entity();
                let menu_id = group.id.clone();
                let menu_name = group.name.clone();
                let title: AnyElement = match renaming {
                    Some(field) => {
                        let view = cx.entity();
                        div()
                            .w(px(260.))
                            .ml(px(-4.))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(RADIUS_SM)
                            .bg(t.bg_elevated)
                            .shadow(vec![ring(4., t.focus_ring), ring(1., t.accent)])
                            .font_family(fonts::DISPLAY)
                            .text_size(TEXT_XL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .capture_key_down(move |e: &KeyDownEvent, window, cx| {
                                if e.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    view.update(cx, |v, cx| v.cancel_renames(window, cx));
                                }
                            })
                            .child(input(&field, px(28.), false))
                            .into_any_element()
                    }
                    None => div()
                        .id(SharedString::from(format!("group-toggle-{}", group.id)))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .min_w_0()
                        .ml(px(-6.))
                        .pt(px(2.))
                        .pb(px(2.))
                        .pl(px(3.))
                        .pr(px(8.))
                        .rounded(RADIUS_SM)
                        .hover(|s| s.bg(t.bg_hover))
                        .active(|s| s.bg(t.bg_pressed))
                        .tooltip(native_tooltip(
                            if collapsed { "Show projects" } else { "Hide projects" }.into(),
                        ))
                        // UI-191: collapses or expands at once, saved.
                        .on_click(move |_, _, cx| {
                            let id = toggle_id.clone();
                            toggle_state.update(cx, |s, cx| {
                                s.update_group(
                                    id,
                                    GroupPatch {
                                        collapsed: Some(!collapsed),
                                        ..GroupPatch::default()
                                    },
                                    cx,
                                );
                            });
                        })
                        .child(
                            icon(Icon::ChevronDown, 16., 2.2, t.text_tertiary)
                                .with_transformation(Transformation::rotate(radians(chevron.to_radians()))),
                        )
                        .child(
                            spaced_text(
                                group.name.clone(),
                                heading_font.clone(),
                                TEXT_XL,
                                px(-0.2),
                                px(28.),
                                t.text_primary,
                            )
                            .truncate(),
                        )
                        .into_any_element(),
                };
                let header = div()
                    .id(SharedString::from(format!("group-header-{}", group.id)))
                    .flex()
                    .items_end()
                    .gap(px(10.))
                    .mb(px(14.))
                    .ml(px(6.))
                    .pt(px(8.))
                    .on_mouse_down(MouseButton::Right, move |e, _, cx| {
                        cx.stop_propagation();
                        let (id, name, at) = (menu_id.clone(), menu_name.clone(), e.position);
                        menu_view.update(cx, |v, cx| v.group_menu(id, name, at, cx));
                    })
                    .child(title)
                    .child(count(members.len(), TEXT_MD, 4.4, t));
                let section = self
                    .drop_section(&group.id, section_of.clone(), t)
                    .mt(px(20.))
                    .child(header)
                    .when(!collapsed, |d| {
                        if folders.is_empty() {
                            d.child(placeholder("Drag projects here", t))
                        } else {
                            d.child(grid(folders))
                        }
                    });
                sections.push(section.into_any_element());
            }
            if !lists.archived.is_empty() {
                let folders: Vec<AnyElement> = lists
                    .archived
                    .clone()
                    .iter()
                    .map(|p| self.folder(p, &lists, window, cx))
                    .collect();
                sections.push(
                    div()
                        .mt(px(36.))
                        .mb(px(16.))
                        .ml(px(6.))
                        .pt(px(18.))
                        .border_t(px(0.5))
                        .border_color(t.separator)
                        .flex()
                        .items_end()
                        .gap(px(10.))
                        .child(
                            div()
                                .font_family(fonts::DISPLAY)
                                .text_size(TEXT_LG)
                                .line_height(px(21.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text_secondary)
                                .child("Archived"),
                        )
                        .child(count(lists.archived.len(), TEXT_MD, 0.7, t))
                        .into_any_element(),
                );
                sections.push(grid(folders).into_any_element());
            }
            // UI-198, UI-199
            let note = if lists.focusing && lists.hidden > 0 {
                Some(format!(
                    "{} other {} hidden by focus mode.",
                    lists.hidden,
                    if lists.hidden == 1 {
                        "project is"
                    } else {
                        "projects are"
                    }
                ))
            } else if focus_mode && !lists.focusing && lists.active_count > 0 {
                Some(
                    "Focus mode is on, but no project is in focus yet. Right-click a project and choose Add to Focus."
                        .into(),
                )
            } else {
                None
            };
            if let Some(note) = note {
                sections.push(
                    div()
                        .mt(px(28.))
                        .ml(px(6.))
                        .pt(px(16.))
                        .border_t(px(0.5))
                        .border_color(t.separator)
                        .text_color(t.text_tertiary)
                        .child(note)
                        .into_any_element(),
                );
            }
            let state = self.state.clone();
            Some(scroll_area(
                "pv-scroll",
                &self.scroll.clone(),
                [28., 32., 48., 32.],
                div()
                    .id("pv-content")
                    .flex()
                    .flex_col()
                    // UI-203: a press anywhere off a folder clears the selection.
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        state.update(cx, |s, cx| s.select(None, cx));
                    })
                    .children(sections),
                window,
                cx,
            ))
        } else {
            None
        };

        let empty = (self.loaded && lists.total == 0).then(|| empty_state(&self.state, window, t));
        let drop_overlay = self.dragging_files.then(|| drop_overlay(window, cx, t));
        let toast = self
            .toast
            .frame("pv-toast-drawn", toast, window, cx)
            .map(|((message, seq), leaving)| {
                let rise =
                    leaving.unwrap_or_else(|| enter(window, cx, ("pv-toast", seq as usize), DUR_MED, motion::ease_out));
                div()
                    .absolute()
                    .bottom(px(24.))
                    .translate(px(0.), px(8. * (1. - rise)))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .max_w(px(520.).min(window.viewport_size().width - px(48.)))
                            .px(px(14.))
                            .py(px(9.))
                            .rounded(RADIUS_MD)
                            .bg(t.bg_elevated)
                            .shadow(t.lg())
                            .opacity(rise)
                            .child(message),
                    )
            });
        let remove = self.render_remove(window, cx);
        let picker = self.render_picker(window, cx);
        let _ = groups_known;
        let state = self.state.clone();
        let view = cx.entity();

        div()
            .id("pv")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(t.bg_window)
            // UI-228: files from the OS dragged over the grid show the overlay.
            .on_drag_move::<ExternalPaths>(cx.listener(|view, e: &DragMoveEvent<ExternalPaths>, _, cx| {
                let inside = e.bounds.contains(&e.event.position);
                if view.dragging_files != inside {
                    view.dragging_files = inside;
                    cx.notify();
                }
            }))
            // UI-229: each dropped path is added in turn.
            .on_drop::<ExternalPaths>(move |paths, _, cx| {
                view.update(cx, |v, cx| {
                    v.dragging_files = false;
                    cx.notify();
                });
                for path in paths.paths() {
                    let path = path.to_string_lossy().into_owned();
                    state.update(cx, |s, cx| s.add_project(Some(path), false, cx));
                }
            })
            .children(content)
            .children(empty)
            .children(drop_overlay)
            .children(toast)
            .children(remove)
            .children(picker)
    }
}

/// A heading row: the title, then its count, on the text baseline.
fn heading_row(title: AnyElement, n: usize, count_size: Pixels, t: Tokens, below: f32, above: f32) -> gpui::Div {
    div()
        .flex()
        .items_end()
        .gap(px(10.))
        .mb(px(below))
        .ml(px(6.))
        .pt(px(above))
        .child(title)
        .child(count(n, count_size, 3.8, t))
}

/// A count beside a heading, in `--text-tertiary` tabular figures, raised
/// `lift` px so its baseline meets the heading's (`align-items: baseline`).
fn count(n: usize, size: Pixels, lift: f32, t: Tokens) -> gpui::Div {
    div()
        .text_size(size)
        .line_height(size * 1.4)
        .text_color(t.text_tertiary)
        .font_features(fonts::tabular())
        .pb(px(lift))
        .child(n.to_string())
}

/// The folders' grid: 144px columns as many as fit, gaps 14 × 10 (UI-187).
fn grid(items: Vec<AnyElement>) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .items_start()
        .gap_x(px(10.))
        .gap_y(px(14.))
        .children(items)
}

/// A dashed placeholder (UI-192, UI-227).
fn placeholder(text: &'static str, t: Tokens) -> gpui::Div {
    div()
        .h(px(72.))
        .mx(px(6.))
        .mb(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .border(px(1.5))
        .border_dashed()
        .border_color(t.border_strong)
        .rounded(RADIUS_MD)
        .text_size(TEXT_SM)
        .text_color(t.text_tertiary)
        .child(text)
}

/// UI-185: no projects yet.
fn empty_state(state: &Entity<AppState>, window: &mut Window, t: Tokens) -> AnyElement {
    let state = state.clone();
    let font = gpui::Font {
        family: fonts::DISPLAY.into(),
        weight: FontWeight::SEMIBOLD,
        ..gpui::Font::default()
    };
    let height = window.viewport_size().height;
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .p(px(32.))
        .pb(height * 0.12)
        .text_center()
        .child(
            div()
                .mb(px(10.))
                .opacity(0.55)
                .child(folder_icon(148., None).glyph(Some(Glyph::Icon(Icon::FolderPlus)))),
        )
        .child(spaced_text(
            "Add your first project",
            font,
            TEXT_XL,
            px(-0.2),
            px(28.),
            t.text_primary,
        ))
        .child(
            div()
                .max_w(px(360.))
                .mb(px(14.))
                .text_color(t.text_secondary)
                .child("Link a local repository to organize its Claude sessions on a board."),
        )
        .child(
            button("choose-folder")
                .primary()
                .label("Choose Folder…")
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.add_project(None, false, cx))),
        )
        .child(
            div()
                .mt(px(6.))
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .text_color(t.text_tertiary)
                .child("or drop a folder anywhere in this window"),
        )
        .into_any_element()
}

/// UI-228: the dashed overlay while folders are dragged in from the OS.
fn drop_overlay(window: &mut Window, cx: &mut App, t: Tokens) -> AnyElement {
    let fade = enter(window, cx, "pv-drop", DUR_FAST, motion::ease);
    div()
        .absolute()
        .top(px(12.))
        .left(px(12.))
        .right(px(12.))
        .bottom(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .border(px(2.))
        .border_dashed()
        .border_color(t.accent)
        .rounded(RADIUS_XL)
        .map(|d| frosted(d, t.archive_drop, 2., 1.))
        .opacity(fade)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .text_size(TEXT_LG)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(folder_icon(88., None).glyph(Some(Glyph::Icon(Icon::Plus))))
                .child("Drop a folder to add it as a project"),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};
    use serde_json::Map;

    fn project(id: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            repo_path: String::new(),
            color: FolderColor::Blue,
            created_at: 0.0,
            opened_at: None,
            group_id: None,
            actions: None,
            focused: None,
            archived: None,
            server: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn finds_the_folder_below_by_its_left_edge() {
        let at = |x: f32, y: f32| Bounds::new(point(px(x), px(y)), size(px(144.), px(150.)));
        let bounds: HashMap<String, Bounds<Pixels>> = [
            ("a", at(0., 0.)),
            ("b", at(154., 0.)),
            ("c", at(308., 0.)),
            ("d", at(0., 200.)),
            ("e", at(154., 200.)),
            ("f", at(0., 400.)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let ordered: Vec<Project> = ["a", "b", "c", "d", "e", "f"].iter().map(|id| project(id)).collect();
        assert_eq!(folder_in_next_row(&bounds, &ordered, "c", 1.0).as_deref(), Some("e"));
        assert_eq!(folder_in_next_row(&bounds, &ordered, "b", 1.0).as_deref(), Some("e"));
        assert_eq!(folder_in_next_row(&bounds, &ordered, "e", -1.0).as_deref(), Some("b"));
        assert_eq!(folder_in_next_row(&bounds, &ordered, "a", -1.0), None);
        assert_eq!(folder_in_next_row(&bounds, &ordered, "f", 1.0), None);
    }

    #[test]
    fn renames_only_to_a_new_non_blank_name() {
        assert_eq!(changed_name("  shop ", "store").as_deref(), Some("shop"));
        assert_eq!(changed_name("store", "store"), None);
        assert_eq!(changed_name("   ", "store"), None);
    }
}
