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
pub mod edges;
mod frames;
pub mod fuzzy;
mod hud;
mod images;
mod input;
pub mod layout;
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
pub mod units;
pub mod viewport;

// For the Mac's scroll monitor (`mac::watch_scroll_devices`).
#[cfg(target_os = "macos")]
pub(crate) use input::saw_scroll;

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Entity, IntoElement, ParentElement, Styled, Transformation, Window,
    div, percentage,
};
use nodus_domain::{ProjectPatch, SessionKind};

use crate::components::context_menu::{MenuItem, MenuLayer};
use crate::components::icon::{Icon, icon};
use crate::components::split_button::SplitButton;
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

/// BOARD-147: the title bar's split button: `New Session`, and its menu
/// `More ways to start` (spec 09 § Title-bar actions), with the project's
/// worktree switch on a board in the app (SESS-217).
pub fn title_bar_button(
    board: &Entity<canvas::BoardCanvas>,
    menus: &Entity<MenuLayer>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (main, terminal, view) = (board.downgrade(), board.downgrade(), board.downgrade());
    let mut items = vec![
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
    ];
    items.extend(worktree_switch(board, cx).into_iter().flatten());
    SplitButton {
        id: "new-session".into(),
        label: "New Session".into(),
        icon: Some((Icon::Plus, 15., 2.2)),
        title: Some(format!("New session ({})", keys("⌘N")).into()),
        on_click: Rc::new(move |_, cx| {
            if let Some(b) = main.upgrade() {
                b.update(cx, |b, cx| b.create_session(None, SessionKind::Claude, cx));
            }
        }),
        items,
        menu_label: "More ways to start".into(),
    }
    .render(menus, window, cx)
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
