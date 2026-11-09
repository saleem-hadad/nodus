//! The window's root (`App.tsx`, `app.css`; UI-023…UI-047): the title bar
//! over the main area, which holds the projects grid or a board with the
//! project rail, and the layers above both (Settings, menus).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    KeyUpEvent, ModifiersChangedEvent, ParentElement, Render, Styled, Subscription, Window, div, px,
};
use nodus_core::Nodus;

use crate::board::canvas::{BoardCanvas, new_canvas};
use crate::components::context_menu::MenuLayer;
use crate::platform::Platform;
use crate::projects::grid::ProjectsView;
use crate::projects::rail::ProjectRail;
use crate::settings::SettingsDialog;
use crate::shell::title_bar::{new_project_button, title_bar};
use crate::state::{AppState, Route};
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// The main window's root view.
pub struct Shell {
    pub state: Entity<AppState>,
    pub menus: Entity<MenuLayer>,
    focus: FocusHandle,
    grid: Option<Entity<ProjectsView>>,
    rail: Option<Entity<ProjectRail>>,
    settings: Option<Entity<SettingsDialog>>,
    /// The session window, transcript preview and agents viewer (spec 12).
    pub terminal_layers: Entity<crate::terminal::layers::TerminalLayers>,
    /// The board the main area shows; a fresh one per project (UI-028,
    /// BOARD-002).
    board_key: Option<String>,
    board: Option<Entity<BoardCanvas>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(nodus: Nodus, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let state = cx.new(|cx| AppState::new(nodus, cx));
        let menus = cx.new(|_| MenuLayer::default());
        let terminal_layers = cx.new(|cx| crate::terminal::layers::TerminalLayers::new(state.clone(), cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.observe(&menus, |_, _, cx| cx.notify()),
            // UI-002: the OS switching appearance re-themes the window.
            cx.observe_window_appearance(window, |_, window, cx| theme::sync_window(window, cx)),
            // UI-160: losing focus closes a menu; UI-279: and ends the ⌘ numbers.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.menus.update(cx, |m, cx| m.close(window, cx));
                    if let Some(rail) = this.rail.clone() {
                        rail.update(cx, ProjectRail::hide_numbers);
                    }
                    // The board lets go of Space (React Flow's key state).
                    if let Some(board) = this.board.clone() {
                        board.update(cx, |b, _| b.release_keys());
                    }
                }
            }),
            // UI-160: so does resizing the window.
            cx.observe_window_bounds(window, |this, window, cx| {
                this.menus.update(cx, |m, cx| m.close(window, cx));
            }),
        ];
        Self {
            state,
            menus,
            focus,
            grid: None,
            rail: None,
            settings: None,
            terminal_layers,
            board_key: None,
            board: None,
            _subscriptions: subscriptions,
        }
    }

    /// Keeps the views that belong to the route alive, and only those.
    fn sync_views(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (route, settings_open) = {
            let s = self.state.read(cx);
            (s.route.clone(), s.settings_open)
        };
        match &route {
            Route::Projects => {
                if self.grid.is_none() {
                    let state = self.state.clone();
                    let menus = self.menus.clone();
                    self.grid = Some(cx.new(|cx| ProjectsView::new(state, menus, window, cx)));
                }
                // UI-029: the rail starts over the next time a board opens.
                self.rail = None;
                self.board_key = None;
                self.board = None;
            }
            Route::Board(id) => {
                self.grid = None;
                if self.rail.is_none() {
                    let state = self.state.clone();
                    let menus = self.menus.clone();
                    self.rail = Some(cx.new(|cx| ProjectRail::new(state, menus, cx)));
                }
                if self.board_key.as_ref() != Some(id) || self.board.is_none() {
                    let state = self.state.clone();
                    let menus = self.menus.clone();
                    self.board = Some(new_canvas(state, menus, id.clone(), cx));
                }
                self.board_key = Some(id.clone());
            }
        }
        // UI-118, UI-119: Settings reloads and takes focus when it opens, and
        // hands focus back when it closes.
        match (settings_open, &self.settings) {
            (true, None) => {
                let state = self.state.clone();
                let previous = window.focused(cx);
                self.settings = Some(cx.new(|cx| SettingsDialog::new(state, previous, window, cx)));
            }
            (false, Some(dialog)) => {
                let previous = dialog.read(cx).previous_focus.clone();
                self.settings = None;
                match previous {
                    Some(previous) => window.focus(&previous, cx),
                    None => window.focus(&self.focus, cx),
                }
            }
            _ => {}
        }
    }

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let platform = Platform::CURRENT;
        let m = &e.keystroke.modifiers;
        // UI-042: off a Mac, the window takes Ctrl+N, Ctrl+T and Ctrl+W
        // outside terminals.
        if !platform.is_mac() && m.control && !m.shift && !m.alt && !m.platform && !e.is_held {
            let command = match e.keystroke.key.as_str() {
                "n" => Some("new-session"),
                "t" => Some("new-terminal"),
                "w" => Some("close"),
                _ => None,
            };
            if let Some(command) = command {
                cx.stop_propagation();
                let nodus = self.state.read(cx).nodus.clone();
                cx.spawn(async move |_, _| nodus.app().menu_command(command).await)
                    .detach();
                return;
            }
        }
        if let Some(rail) = self.rail.clone()
            && rail.update(cx, |r, cx| r.on_key_down(e, window, cx))
        {
            cx.stop_propagation();
            return;
        }
        // The board's keys (BOARD-156…BOARD-170), unless a field has the keyboard.
        if let Some(board) = self.board.clone() {
            let board_focus = board.read(cx).focus.clone();
            let typing = window.focused(cx).is_some_and(|f| f != self.focus && f != board_focus);
            if board.update(cx, |b, cx| b.on_key_down(e, typing, cx)) {
                cx.stop_propagation();
                return;
            }
        }
        if let Some(grid) = self.grid.clone()
            && grid.update(cx, |g, cx| g.on_key_down(e, window, cx))
        {
            cx.stop_propagation();
        }
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync_views(window, cx);
        let t = theme::tokens(cx);
        let route = self.state.read(cx).route.clone();
        let pinned = self.state.read(cx).pin_rail();
        let actions = match route {
            Route::Projects => vec![new_project_button(&self.state)],
            // BOARD-147: the board's split button.
            Route::Board(_) => self
                .board
                .clone()
                .map(|board| vec![crate::board::title_bar_button(&board, &self.menus, window, cx)])
                .unwrap_or_default(),
        };
        let bar = title_bar(&self.state, actions, window, cx);
        // The actions rail (05) on the board's right edge, until the board
        // view (09) mounts it itself with its hooks.
        let action_rail = matches!(route, Route::Board(_)).then(|| {
            crate::actions::action_rail(
                &self.state,
                &self.menus,
                crate::actions::RailHooks::default(),
                window,
                cx,
            )
        });
        let menus_open = self.menus.read(cx).is_open();

        let main = div()
            .id("app-main")
            .relative()
            .flex_1()
            .min_h_0()
            .map(|d| match &route {
                Route::Projects => d.children(self.grid.clone()),
                Route::Board(_) => d
                    .child(
                        // UI-026, UI-027: the board, beside a pinned rail;
                        // BOARD-001: the loading surface until it's in.
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .left(if pinned { RAIL_W } else { px(0.) })
                            .bg(t.bg_canvas)
                            .map(|d| match &self.board {
                                Some(board) if board.read(cx).board().is_some() => d.child(board.clone()),
                                _ => d.child(crate::board::loading(self.state.read(cx).board_loading, cx)),
                            })
                            .children(action_rail),
                    )
                    .children(self.rail.clone()),
            });

        div()
            .id("app")
            .key_context("Nodus")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(t.bg_window)
            .font_family(fonts::UI)
            .text_size(TEXT_MD)
            .line_height(px(18.2))
            .text_color(t.text_primary)
            // UI-160: Escape closes a menu and goes no further.
            .capture_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if menus_open && e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.menus.update(cx, |m, cx| m.close(window, cx));
                }
            }))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| {
                if let Some(rail) = this.rail.clone() {
                    rail.update(cx, |r, cx| r.on_key_up(e, cx));
                }
                if let Some(board) = this.board.clone() {
                    board.update(cx, |b, cx| b.on_key_up(e, cx));
                }
            }))
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, cx| {
                if let Some(rail) = this.rail.clone() {
                    rail.update(cx, |r, cx| r.on_modifiers_changed(e, cx));
                }
            }))
            // UI-280: ⌘-click and ⌘-scroll aren't for the numbers.
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                if let Some(rail) = this.rail.clone() {
                    rail.update(cx, |r, _| r.cancel_numbers());
                }
            }))
            .on_scroll(cx.listener(|this, _, _, cx| {
                if let Some(rail) = this.rail.clone() {
                    rail.update(cx, |r, _| r.cancel_numbers());
                }
            }))
            .child(bar)
            .child(main)
            .child(self.terminal_layers.clone())
            .children(self.settings.clone())
            .child(self.menus.clone())
    }
}
