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

pub mod canvas;
mod commands;
pub mod edges;
pub mod fuzzy;
mod hud;
mod input;
pub mod layout;
mod menus;
pub mod nodes;
mod render;
pub mod scene;
mod section;
mod store;
pub mod units;
pub mod viewport;

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Entity, IntoElement, ParentElement, Styled, Transformation, Window,
    div, percentage,
};
use nodus_domain::SessionKind;

use crate::components::context_menu::{MenuItem, MenuLayer};
use crate::components::icon::{Icon, icon};
use crate::components::split_button::SplitButton;
use crate::platform::keys;
use crate::theme;

/// Sets the board up once at start-up: installs the node renderers.
pub fn init(cx: &mut App) {
    // board/cards/ installs its renderers here: `cards::install(cx)`.
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
/// `More ways to start` (spec 09 § Title-bar actions).
pub fn title_bar_button(
    board: &Entity<canvas::BoardCanvas>,
    menus: &Entity<MenuLayer>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (main, terminal) = (board.downgrade(), board.downgrade());
    SplitButton {
        id: "new-session".into(),
        label: "New Session".into(),
        icon: Some((Icon::Plus, 15., 2.2)),
        title: Some(format!("New session ({})", keys("⌘N")).into()),
        on_click: Rc::new(move |_, cx| {
            if let Some(b) = main.upgrade() {
                b.update(cx, |b, cx| b.create_session(None, SessionKind::Claude, None, cx));
            }
        }),
        items: vec![
            MenuItem::item("New terminal", Icon::SquareTerminal, move |_, cx| {
                if let Some(b) = terminal.upgrade() {
                    b.update(cx, |b, cx| b.create_session(None, SessionKind::Terminal, None, cx));
                }
            })
            .hint(keys("⌘T")),
            // The New view dialog is spec 04's.
            MenuItem::item("New view…", Icon::LayoutDashboard, |_, _| {}),
        ],
        menu_label: "More ways to start".into(),
    }
    .render(menus, window, cx)
}
