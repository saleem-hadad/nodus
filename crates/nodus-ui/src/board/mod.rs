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
pub mod cards;
pub mod layout;
pub mod nodes;
pub mod units;

use gpui::App;

/// Sets the board up once at start-up: installs the node renderers.
pub fn init(cx: &mut App) {
    // board/cards/ installs its renderers here: `cards::install(cx)`.
    cards::install(cx);
    if cx.try_global::<nodes::Renderers>().is_none() {
        nodes::install(nodes::Placeholders, cx);
    }
}
