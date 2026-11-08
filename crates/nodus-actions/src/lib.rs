//! Project actions, the buttons in the board's actions rail: Claude drafting
//! one from what the user describes, and running it as a session that shows
//! beside its button and goes when its turn ends.
//!
//! Ports `src/main/actions.ts`, the `actions:*` handlers in `src/main/ipc.ts`
//! and the logic of `ActionRail.tsx` and `ActionDialog.tsx`. The icons are
//! `nodus_domain::ActionIcon`.
//!
//! - [`normalize_action`] checks and trims what the window saves (ACT-001…012);
//! - [`Actions`] keeps the project's own and the global lists in state.json
//!   (ACT-002, ACT-013…015);
//! - [`draft_action`] asks `claude -p` for a draft (ACT-016, ACT-019…027);
//! - [`run_args`] is a press's extra `claude` arguments (ACT-028). Starting,
//!   finishing and taking over a run are nodus-board's (`create_session`,
//!   `finish_run`, `on_done`, `claim_run`) and nodus-sessions'
//!   (`launch::for_create`, `launch::is_takeover`);
//! - [`rail`] and [`dialog`] are the rail's and the dialog's logic, for nodus-ui.

pub mod dialog;
mod draft;
mod lists;
mod normalize;
pub mod rail;
mod run;

pub use draft::{
    DRAFT_SYSTEM_PROMPT, DRAFT_TIMEOUT, DraftError, draft_action, draft_args, draft_prompt, draft_schema_json,
    draft_with, read_draft,
};
pub use lists::{Actions, find_action, is_run};
pub use normalize::{MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_PROMPT_CHARS, clip, is_action_icon, normalize_action};
pub use run::{run_args, run_system_prompt};

/// Why saving or removing an action failed; the text is 0.9.4's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    /// ACT-005.
    #[error("An action needs a prompt")]
    NoPrompt,
    /// `Unknown project <id>` (ACT-014, ACT-015).
    #[error(transparent)]
    NotFound(#[from] nodus_store::NotFound),
}

/// ACT-083: how an error from a command reaches the window in 0.9.4, which
/// the action dialog, the run toast, voice notices and the Changes modal show
/// as it is: `Error invoking remote method '<channel>': Error: <message>`.
pub fn remote_error(channel: &str, message: &str) -> String {
    format!("Error invoking remote method '{channel}': Error: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_errors_as_electrons_ipc_does() {
        assert_eq!(
            remote_error("actions:draft", "Drafting the button took too long"),
            "Error invoking remote method 'actions:draft': Error: Drafting the button took too long"
        );
    }
}
