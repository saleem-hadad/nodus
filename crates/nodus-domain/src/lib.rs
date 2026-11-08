//! The types every Nodus domain shares: projects, boards, sessions, views,
//! capture, git, updates and settings. Pure data, with no I/O.
//!
//! A port of `src/shared/types.ts`, `src/shared/actions.ts`, the menu commands
//! and patch shapes in `src/shared/api.ts`, and the constants in
//! `src/shared/capture.ts`. Serde reads and writes exactly the JSON the
//! TypeScript does, so state.json and everything else the app shares stays
//! compatible:
//!
//! - fields are camelCase, and an optional field that isn't set is left out;
//! - `null` is written only where the TypeScript writes it (`GitStatus::branch`,
//!   `CommandResult::code`), and `T | null` patch fields keep absent and `null`
//!   apart (`NodePatch::section_id`, `ProjectPatch::group_id`);
//! - unions of strings are enums spelled the same, and tagged unions keep their
//!   tag field (`kind`, `state`);
//! - every JavaScript number that is a time, a position or a size is an `f64`,
//!   since times can have a fraction (a file's mtime) and are saved that way;
//! - types kept in state.json hold the fields this version doesn't know in
//!   `extra`, so a file written by a newer version is never cut down.
//!
//! Write JSON with [`json`] rather than serde_json directly: it writes numbers
//! the way JSON.stringify does.

#[macro_use]
mod macros;

pub mod action;
pub mod app;
pub mod board;
pub mod capture;
pub mod git;
pub mod json;
mod nullable;
pub mod project;
pub mod search;
pub mod session;
pub mod settings;
pub mod update;
pub mod view;
pub mod voice;

pub use action::*;
pub use app::*;
pub use board::*;
pub use capture::*;
pub use git::*;
pub use project::*;
pub use search::*;
pub use session::*;
pub use settings::*;
pub use update::*;
pub use view::*;
pub use voice::*;
