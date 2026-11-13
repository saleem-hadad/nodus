//! The app's persistent state: the projects, their boards and the settings,
//! in state.json in the app's data folder.
//!
//! A port of `src/main/store.ts`, with the data folder from
//! `src/main/index.ts`. The file is read and written exactly as 0.9.4 does:
//! the same folder and `NODUS_USER_DATA` override, the same shape, the same
//! back-compat on load, and the same bytes on save (two-space JSON, keys in
//! the order they were read, numbers as JavaScript writes them), through a
//! temporary file and a rename, 300 ms after the last change.
//!
//! Two things differ, both so a file is never lost: fields this version
//! doesn't know are kept rather than dropped, and a file it can't read is
//! moved aside rather than written over.

mod paths;
mod records;
mod state;
mod store;

pub use paths::user_data_dir;
pub use records::Records;
pub use state::{
    BoardState, CaptureRecord, DEFAULT_ARCHIVE_W, NotFound, ServerBoardState, ServerRecord, State, normalize_settings,
};
pub use store::{SAVE_DELAY, Store};
