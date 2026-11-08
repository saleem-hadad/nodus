//! Read-only git queries behind the title bar's branch chip and the Changes
//! modal: the branch and its uncommitted changes, a file's diff against HEAD,
//! and the branch's recent commits; and the Changes modal's diff parser and
//! labels.
//!
//! Ports `src/main/git.ts`, `src/renderer/src/components/git/diff.ts` and the
//! pure parts of `GitControls.tsx` and `DiffModal.tsx`. [`Git`] runs git the
//! way 0.9.4 does (GIT-001): `git --no-optional-locks -c core.quotepath=false
//! …` on the login shell's PATH with `GIT_TERMINAL_PROMPT=0`, no console
//! window on Windows, and a cap on each output stream.
//!
//! Commands (nodus-core wires them, after the login-shell env is ready and
//! the project's `repoPath` is resolved, GIT-003):
//!
//! - `git:status` → [`Git::status`] (a failure reads as `null` in the window);
//! - `git:diff` → [`Git::diff`];
//! - `git:log` → [`Git::log`] with [`clamp_limit`].

mod collate;
mod diff;
pub mod labels;
mod log;
pub mod nodepath;
pub mod parse;
mod runner;
mod status;

pub use collate::locale_compare;
pub use diff::{DiffError, diff_args, in_repository, is_binary_patch};
pub use log::{DEFAULT_LOG_LIMIT, clamp_limit, log_args, parse_log};
pub use parse::{DiffLine, DiffLineKind, ParsedDiff, parse_diff};
pub use runner::{DEFAULT_MAX_BUFFER, Git, GitResult, MAX_DIFF_BYTES, RunError};
pub use status::{STATUS_ARGS, parse_status};
