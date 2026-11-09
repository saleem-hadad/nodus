//! The board's overlays (spec 11): the ink layer and its dock, the prompt
//! bar with the selection bar and toast stacked above it, ⌘K search and the
//! merge dialog. The canvas holds their state in [`Overlays`] and draws them
//! from its render; the board's side of what they do is in `board/bar.rs`.

pub mod icons;
pub mod ink;

/// Nodes at this z or above (the open archive panel and its results,
/// `RAISED_Z`) draw over the ink (INKUI-025).
pub const INK_Z_BELOW: i32 = super::scene::RAISED_Z;

/// The overlays' state on the board canvas: everything is per board mount,
/// so switching project starts each one over (BAR-041, INKUI-021).
#[derive(Default)]
pub struct Overlays {
    pub(crate) ink: ink::Ink,
}
