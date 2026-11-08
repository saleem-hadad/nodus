//! What a press runs (ACT-028): an ordinary Claude session on the action's
//! prompt, in auto mode, with a note on how a run should behave. The board
//! (nodus-board's `create_session`, `finish_run`, `claim_run`) and the
//! command line (nodus-sessions' `launch::for_create`) do the rest.

use nodus_domain::ProjectAction;

/// The run's note (`runSystemPrompt`): the action's name goes in between
/// straight double quotes, as it is.
pub fn run_system_prompt(name: &str) -> String {
    format!(
        "You are running \"{name}\", an action button the user pressed on the Nodus board for this project. Nobody is watching while you work, so do the job in the prompt without stopping to chat. If you need the user to decide something, ask with AskUserQuestion: that turns the action's button red to call them. When you are done, end with one line saying what you did. The user is shown that line, and this session then goes away. The mcp__nodus__* tools let you list and archive the board's sessions, and write sticky notes on the board."
    )
}

/// ACT-028: `runArgs(action)`, the extra `claude` arguments for a run.
pub fn run_args(action: &ProjectAction) -> Vec<String> {
    vec!["--append-system-prompt".to_owned(), run_system_prompt(&action.name)]
}

#[cfg(test)]
mod tests {
    //! actions.test.ts › runArgs.
    use super::*;
    use serde_json::json;

    #[test]
    fn tells_the_run_which_action_it_is() {
        let action = crate::normalize_action(&json!({ "name": "Tidy Up", "prompt": "Tidy." }), None, 0.0).unwrap();
        let args = run_args(&action);
        assert_eq!(args[0], "--append-system-prompt");
        assert!(args[1].contains("\"Tidy Up\""));
        assert_eq!(args.len(), 2);
        // Quotes in a name go in unescaped.
        assert!(run_system_prompt("Say \"hi\"").starts_with("You are running \"Say \"hi\"\", an action button"));
        assert!(!args[1].contains('\n'));
    }
}
