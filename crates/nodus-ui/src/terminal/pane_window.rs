//! The session window as a whole (MODAL-032, SPLIT-010…SPLIT-013): a modal
//! over the board with a pane per session on show, in up to two columns,
//! and the lines between them to drag.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, CursorStyle, DispatchPhase, Entity, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render,
    Styled, Subscription, Window, canvas, div, px, relative,
};

use super::actions;
use super::panes::{Shares, clamp_share, columns, window_size};
use super::session_window::SessionWindow;
use crate::components::modal::Modal;
use crate::state::AppState;
use crate::theme;

/// The width of the lines between panes.
const LINE: Pixels = px(1.);
/// How far either side of a line a press takes hold of it.
const GRAB: Pixels = px(4.);

/// A line being dragged (SPLIT-012): the one between the columns, or the
/// one between a column's rows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Columns,
    Rows(usize),
}

pub struct PaneWindow {
    state: Entity<AppState>,
    project_id: String,
    panes: Vec<Entity<SessionWindow>>,
    shares: Shares,
    drag: Option<Drag>,
    /// Where the panes were drawn last, for turning the pointer into shares.
    grid: Bounds<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl PaneWindow {
    pub fn new(state: Entity<AppState>, project_id: String, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        let mut this = Self {
            state,
            project_id,
            panes: Vec::new(),
            shares: Shares::default(),
            drag: None,
            grid: Bounds::default(),
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// A pane per session on show: kept while its session stays, built for
    /// one that's new, the focused one taking the keyboard (MODAL-007).
    fn sync(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (ids, focused) = {
            let panes = &self.state.read(cx).session_window;
            (panes.ids().to_vec(), panes.focused().map(str::to_owned))
        };
        let current: Vec<String> = self.panes.iter().map(|p| p.read(cx).session_id.clone()).collect();
        if ids == current {
            return;
        }
        let mut old = std::mem::take(&mut self.panes);
        for id in &ids {
            if let Some(at) = old.iter().position(|p| p.read(cx).session_id == *id) {
                self.panes.push(old.remove(at));
            } else {
                let (state, project, id) = (self.state.clone(), self.project_id.clone(), id.clone());
                let focus = focused.as_deref() == Some(id.as_str());
                self.panes
                    .push(cx.new(|cx| SessionWindow::new(state, id, project, focus, window, cx)));
            }
        }
        // SPLIT-012: the lines start even again when a pane comes or goes.
        if ids.len() != current.len() {
            self.shares = Shares::default();
        }
    }

    pub fn panes(&self) -> &[Entity<SessionWindow>] {
        &self.panes
    }

    /// The pane the keyboard is in.
    pub fn focused(&self, cx: &App) -> Option<Entity<SessionWindow>> {
        let focused = self.state.read(cx).session_window.focused()?.to_owned();
        self.panes.iter().find(|p| p.read(cx).session_id == focused).cloned()
    }

    fn drag_to(&mut self, at: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let grid = self.grid;
        match self.drag {
            Some(Drag::Columns) if grid.size.width > px(0.) => {
                self.shares.columns = clamp_share((at.x - grid.origin.x) / grid.size.width);
            }
            Some(Drag::Rows(column)) if grid.size.height > px(0.) => {
                self.shares.rows[column] = clamp_share((at.y - grid.origin.y) / grid.size.height);
            }
            _ => return,
        }
        cx.notify();
    }

    /// SPLIT-012: the grip over a line; a double press evens it out.
    fn grip(&self, drag: Drag, bounds: Bounds<Pixels>, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let held = self.drag == Some(drag);
        let across = drag == Drag::Columns;
        let group = match drag {
            Drag::Columns => "pane-grip-columns".to_owned(),
            Drag::Rows(c) => format!("pane-grip-rows-{c}"),
        };
        let line = div()
            .absolute()
            .bg(if held { t.text_tertiary } else { t.separator })
            .group_hover(group.clone(), |s| s.bg(t.text_tertiary))
            .map(|d| {
                if across {
                    d.top_0().bottom_0().left(GRAB).w(LINE)
                } else {
                    d.left_0().right_0().top(GRAB).h(LINE)
                }
            });
        div()
            .id(gpui::SharedString::from(group.clone()))
            .group(group)
            .absolute()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .cursor(if across {
                CursorStyle::ResizeLeftRight
            } else {
                CursorStyle::ResizeUpDown
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    if e.click_count >= 2 {
                        match drag {
                            Drag::Columns => this.shares.columns = 0.5,
                            Drag::Rows(c) => this.shares.rows[c] = 0.5,
                        }
                        this.drag = None;
                    } else {
                        this.drag = Some(drag);
                    }
                    cx.notify();
                }),
            )
            .child(line)
            .into_any_element()
    }
}

/// Two lengths that fill `total` less a line between them, split at `share`.
fn split(total: Pixels, share: f32) -> [Pixels; 2] {
    let first = ((total - LINE) * share).round();
    [first, total - LINE - first]
}

impl Render for PaneWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync(window, cx);
        let viewport = window.viewport_size();
        let count = self.panes.len();
        let size = window_size(viewport, count);
        // The modal keeps 24px clear of each edge (UI-155).
        let w = size.width.min(viewport.width - px(48.));
        let h = size.height.min(viewport.height - px(48.));
        let cols = columns(count);
        let widths: Vec<Pixels> = if cols.len() == 2 {
            split(w, self.shares.columns).to_vec()
        } else {
            vec![w]
        };

        let mut grid = div().relative().flex().w(w).h(h).line_height(relative(1.4));
        let mut grips = Vec::new();
        let mut x = px(0.);
        for (c, col) in cols.iter().enumerate() {
            if c > 0 {
                grid = grid.child(div().flex_none().w(LINE).h_full());
                grips.push((
                    Drag::Columns,
                    Bounds::new(gpui::point(x - GRAB, px(0.)), gpui::size(LINE + GRAB * 2., h)),
                ));
                x += LINE;
            }
            let heights: Vec<Pixels> = if col.len() == 2 {
                split(h, self.shares.rows[c]).to_vec()
            } else {
                vec![h]
            };
            let mut column = div().flex().flex_col().flex_none().w(widths[c]).h(h);
            for (r, &pane) in col.iter().enumerate() {
                if r > 0 {
                    column = column.child(div().flex_none().h(LINE).w_full());
                    grips.push((
                        Drag::Rows(c),
                        Bounds::new(
                            gpui::point(x, heights[0] - GRAB),
                            gpui::size(widths[c], LINE + GRAB * 2.),
                        ),
                    ));
                }
                let entity = self.panes[pane].clone();
                let (state, id) = (self.state.clone(), entity.read(cx).session_id.clone());
                column = column.child(
                    div()
                        .flex_none()
                        .w_full()
                        .h(heights[r])
                        // SPLIT-008: a press anywhere in a pane puts the keyboard there.
                        .capture_any_mouse_down(move |_, _, cx| {
                            state.update(cx, |s, cx| s.focus_pane(&id, cx));
                        })
                        .child(entity),
                );
            }
            grid = grid.child(column);
            x += widths[c];
        }
        // Over the panes either side, so drawn after them.
        for (drag, bounds) in grips {
            let grip = self.grip(drag, bounds, cx);
            grid = grid.child(grip);
        }
        let me = cx.entity();
        let dragging = self.drag.is_some();
        grid = grid.child(
            canvas(
                {
                    let me = me.clone();
                    move |bounds, _, cx| me.update(cx, |this, _| this.grid = bounds)
                },
                move |_, _, window, _| {
                    if !dragging {
                        return;
                    }
                    let m = me.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            m.update(cx, |this, cx| this.drag_to(e.position, cx));
                        }
                    });
                    let m = me.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            m.update(cx, |this, cx| {
                                this.drag = None;
                                cx.notify();
                            });
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        );

        let state = self.state.clone();
        // MODAL-003: Escape is the terminal's; MODAL-004: the backdrop closes it.
        let modal = Modal::new("term-modal", w, move |_, cx| actions::close_terminal(&state, cx));
        modal.render(grid, window, cx)
    }
}
