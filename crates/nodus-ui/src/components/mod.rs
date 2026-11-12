//! The shared primitives of `global.css` and the shared widgets every screen
//! uses (spec 08: `.btn`, `.switch`, `.status-dot`, Modal, ContextMenu,
//! ColorSwatches, SplitButton, FolderIcon, UsageStats, RunningCount).

pub mod ambient;
pub mod button;
pub mod color_swatches;
pub mod context_menu;
pub mod focus_ring;
pub mod folder_icon;
pub mod frosted;
pub mod icon;
pub mod modal;
pub mod scroll;
pub mod selectable;
pub mod spaced_text;
pub mod spellcheck;
pub mod split_button;
pub mod status_dot;
pub mod switch;
pub mod text_field;
pub mod tooltip;
pub mod tween;
pub mod usage_stats;

pub use button::{Kind, button};

/// Something run on a click or a choice.
pub type Handler = std::rc::Rc<dyn Fn(&mut gpui::Window, &mut gpui::App)>;
/// Something told a new on/off state.
pub type BoolHandler = std::rc::Rc<dyn Fn(bool, &mut gpui::Window, &mut gpui::App)>;
/// Something told the folder colour picked.
pub type ColorHandler = std::rc::Rc<dyn Fn(nodus_domain::FolderColor, &mut gpui::Window, &mut gpui::App)>;

/// An id for a named part of an element: `parent`'s child `name`.
pub fn child_id(parent: impl Into<gpui::ElementId>, name: &'static str) -> gpui::ElementId {
    gpui::ElementId::NamedChild(std::sync::Arc::new(parent.into()), name.into())
}
pub use folder_icon::{FolderIcon, Glyph, folder_icon, project_glyph};
pub use icon::{Icon, icon};
pub use spaced_text::spaced_text;
pub use status_dot::status_dot;
pub use switch::switch;
pub use tooltip::native_tooltip;
