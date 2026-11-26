//! The board (`views/BoardView.tsx`, spec 09): one project's infinite,
//! pannable, zoomable canvas of session cards, sections, notes, views and the
//! archive, and everything the board does with them.
//!
//! - [`canvas`]: the canvas itself (`BoardCanvas`).
//! - [`nodes`]: the plug-in interface node contents are drawn through
//!   (cards, notes, the archive and the pad from `board/cards/`, views).
//! - [`units`]: flow units on screen ([`units::fu`], [`units::Zoomed`]).
//! - [`layout`]: where things go (ports of `layout.ts`, `sections.ts`,
//!   `notes.ts`).

pub mod bar;
pub mod canvas;
pub mod cards;
mod clipboard;
mod commands;
mod coordinator;
pub(crate) mod coordinator_dialog;
pub mod edges;
mod frames;
pub mod fuzzy;
pub(crate) mod goal_dialog;
mod goals;
mod hud;
mod images;
mod input;
pub mod layout;
mod loupe;
mod menus;
pub(crate) mod moments;
mod naming;
pub mod node_view;
pub mod nodes;
pub mod overlays;
mod pen;
pub mod presence;
mod render;
pub mod scene;
mod section;
pub(crate) mod shapes;
mod store;
mod tiles;
pub mod units;
pub mod viewport;

// For the Mac's scroll monitor (`mac::watch_scroll_devices`).
#[cfg(target_os = "macos")]
pub(crate) use input::saw_scroll;

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Entity, IntoElement, ParentElement, Styled, Transformation, Window,
    div, percentage,
};
use nodus_domain::{ProjectPatch, SessionKind};

use crate::components::context_menu::{MenuItem, MenuLayer};
use crate::components::icon::{Icon, icon};
use crate::components::menu_button::MenuButton;
use crate::platform::keys;
use crate::theme;

/// The tokio runtime the core runs on: core commands that spawn processes
/// or talk to the session host must run there, not on GPUI's executor.
#[derive(Clone)]
pub struct CoreRuntime(pub tokio::runtime::Handle);

impl gpui::Global for CoreRuntime {}

/// The core's runtime, once the app has set it.
pub(crate) fn core_rt(cx: &App) -> Option<tokio::runtime::Handle> {
    cx.try_global::<CoreRuntime>().map(|r| r.0.clone())
}

/// Runs a core command on the core's runtime and waits for it from the UI.
/// Never finishes when the runtime has gone (the app is quitting).
pub(crate) async fn on_core<T: Send + 'static>(
    rt: &Option<tokio::runtime::Handle>,
    fut: impl Future<Output = T> + Send + 'static,
) -> T {
    match rt {
        Some(handle) => match handle.spawn(fut).await {
            Ok(value) => value,
            Err(_) => std::future::pending().await,
        },
        None => fut.await,
    }
}

/// Sets the board up once at start-up: installs the node renderers.
pub fn init(cx: &mut App) {
    // board/cards/ installs its renderers here: `cards::install(cx)`.
    cards::install(cx);
    // Views draw their own nodes, whichever renderers are installed (spec 04).
    crate::views::install(cx);
    if cx.try_global::<nodes::Renderers>().is_none() {
        nodes::install(nodes::Placeholders, cx);
    }
}

/// BOARD-001: the surface shown until the board is in, with a spinner while
/// it loads (`Loader2`, 20px, a turn every 0.9s).
pub fn loading(spinning: bool, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    let frozen = crate::parity::frozen(cx);
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.bg_canvas)
        .children(spinning.then(|| {
            let spinner = icon(Icon::Loader2, 20., 2., t.text_tertiary);
            if frozen {
                spinner.into_any_element()
            } else {
                spinner
                    .with_animation(
                        "board-spin",
                        Animation::new(Duration::from_millis(900)).repeat(),
                        |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                    )
                    .into_any_element()
            }
        }))
        .into_any_element()
}

/// BOARD-147: the title bar's `New` menu (spec 09 § Title-bar actions): a
/// session, terminal or view, then an automation (SCHED-045) or a goal
/// (GOAL-014), with the project's worktree switch on a board in the app
/// (SESS-217), or Add from GitHub on a server's (SRV-120).
pub fn new_button(
    board: &Entity<canvas::BoardCanvas>,
    menus: &Entity<MenuLayer>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (session, terminal, view) = (board.downgrade(), board.downgrade(), board.downgrade());
    let (automation, goal, coordinator) = (board.downgrade(), board.downgrade(), board.downgrade());
    // COORD-010: one coordinator a board; once it has one, this opens it.
    let has_coordinator = board.read(cx).coordinator_id().is_some();
    let on_server = board.read(cx).read_only() || board.read(cx).on_server(cx);
    let mut items = vec![
        MenuItem::item("New session", Icon::Plus, move |_, cx| {
            if let Some(b) = session.upgrade() {
                b.update(cx, |b, cx| b.create_session(None, SessionKind::Claude, cx));
            }
        })
        .hint(keys("⌘N")),
        MenuItem::item("New terminal", Icon::SquareTerminal, move |_, cx| {
            if let Some(b) = terminal.upgrade() {
                b.update(cx, |b, cx| b.create_session(None, SessionKind::Terminal, cx));
            }
        })
        .hint(keys("⌘T")),
        // BOARD-144: the New view dialog (spec 04), centred in view.
        MenuItem::item("New view…", Icon::LayoutDashboard, move |_, cx| {
            if let Some(b) = view.upgrade() {
                b.update(cx, |b, cx| b.open_view_dialog(None, cx));
            }
        }),
        MenuItem::Separator,
        MenuItem::item("New automation…", Icon::Clock, move |_, cx| {
            if let Some(b) = automation.upgrade() {
                b.update(cx, |b, cx| b.new_automation(None, cx));
            }
        }),
        MenuItem::item("New goal…", Icon::Target, move |_, cx| {
            if let Some(b) = goal.upgrade() {
                b.update(cx, |b, cx| b.open_goal_dialog(None, None, cx));
            }
        }),
    ];
    if has_coordinator {
        items.push(MenuItem::item("Open coordinator", Icon::Network, move |_, cx| {
            if let Some(b) = coordinator.upgrade() {
                b.update(cx, canvas::BoardCanvas::show_coordinator);
            }
        }));
    } else if !on_server {
        items.push(
            MenuItem::item("New coordinator…", Icon::Network, move |_, cx| {
                if let Some(b) = coordinator.upgrade() {
                    b.update(cx, canvas::BoardCanvas::open_coordinator_dialog);
                }
            })
            .hint("Beta"),
        );
    }
    items.extend(worktree_switch(board, cx).into_iter().flatten());
    items.extend(add_from_github(board, cx).into_iter().flatten());
    MenuButton {
        id: "new".into(),
        label: "New".into(),
        icon: Some((Icon::Plus, 15., 2.2)),
        items,
    }
    .render(menus, window, cx)
}

/// SRV-120: `Add from GitHub on <server>…` on a server's board, for more
/// projects from its server, while that's connected (UI-329).
fn add_from_github(board: &Entity<canvas::BoardCanvas>, cx: &App) -> Option<[MenuItem; 2]> {
    let canvas = board.read(cx);
    let state = canvas.state.clone();
    let s = state.read(cx);
    let project = s.projects.iter().find(|p| p.id == canvas.project_id)?;
    let item = crate::servers::add_from_github(&state, s.server_of(project)?)?;
    Some([MenuItem::Separator, item])
}

/// SESS-217: `Own worktree per session`, a checkbox for the board's project,
/// off until it's ticked. None on a server's board, whose cards have theirs
/// anyway (SRV-122).
fn worktree_switch(board: &Entity<canvas::BoardCanvas>, cx: &App) -> Option<[MenuItem; 2]> {
    let canvas = board.read(cx);
    if canvas.on_server(cx) {
        return None;
    }
    let state = canvas.state.clone();
    let project = state.read(cx).projects.iter().find(|p| p.id == canvas.project_id)?;
    let (id, on) = (project.id.clone(), project.worktrees == Some(true));
    let check = if on { Icon::SquareCheck } else { Icon::Square };
    Some([
        MenuItem::Separator,
        MenuItem::item("Own worktree per session", check, move |_, cx| {
            let patch = ProjectPatch {
                worktrees: Some(!on),
                ..ProjectPatch::default()
            };
            state.update(cx, |s, cx| s.update_project(id.clone(), patch, cx));
        }),
    ])
}
