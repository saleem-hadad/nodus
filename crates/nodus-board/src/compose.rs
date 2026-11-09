//! What the prompt bar makes and what it hands a new session to read first
//! (`src/renderer/src/components/sidekick/compose.ts`, spec 11 › Slash
//! commands and Context template). The board builds the context from its
//! selection (BOARD-133, BAR-025); the bar matches the slash commands.

use nodus_domain::js;

/// What the bar makes besides a session, which is what it makes unless told
/// otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    View,
    Note,
}

/// A slash command the bar offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub target: Target,
    pub command: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: [SlashCommand; 2] = [
    SlashCommand {
        target: Target::View,
        command: "/view",
        description: "A live page Claude keeps up to date",
    },
    SlashCommand {
        target: Target::Note,
        command: "/note",
        description: "A sticky on the board, nothing runs",
    },
];

/// The commands what's typed could still become: while it's one word that
/// starts with a slash (`/^\/\S*$/`).
pub fn match_commands(text: &str) -> Vec<SlashCommand> {
    let Some(word) = text.strip_prefix('/') else {
        return Vec::new();
    };
    if word.chars().any(js::is_space) {
        return Vec::new();
    }
    let typed = text.to_lowercase();
    COMMANDS
        .iter()
        .filter(|c| c.command.starts_with(&typed))
        .copied()
        .collect()
}

/// A command typed out and followed by a space, or pasted with more after
/// it: what it makes, and the rest (`/^\/(view|note)\s+([\s\S]*)$/i`).
pub fn take_command(text: &str) -> Option<(Target, String)> {
    let word = text.strip_prefix('/')?;
    let (target, after) = [("view", Target::View), ("note", Target::Note)]
        .into_iter()
        .find_map(|(name, target)| {
            let head = word.get(..name.len())?;
            head.eq_ignore_ascii_case(name).then(|| (target, &word[name.len()..]))
        })?;
    let rest = after.trim_start_matches(js::is_space);
    // `\s+` needs at least one space.
    (rest.len() < after.len()).then(|| (target, rest.to_owned()))
}

/// Something selected on the board, which a new session gets to read first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextItem {
    Session {
        title: String,
        branch: Option<String>,
        last_reply: Option<String>,
        transcript: Option<String>,
    },
    Note {
        text: String,
    },
    View {
        title: String,
        description: String,
    },
}

fn quote(text: &str) -> String {
    js::trim(text)
        .split('\n')
        .map(|line| if line.is_empty() { ">".to_owned() } else { format!("> {line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

fn describe(item: &ContextItem) -> String {
    match item {
        ContextItem::Session {
            title,
            branch,
            last_reply,
            transcript,
        } => {
            let mut lines = vec![format!("## Session “{title}”")];
            if let Some(branch) = branch.as_deref().filter(|b| !b.is_empty()) {
                lines.push(format!("Branch: {branch}"));
            }
            match last_reply.as_deref().filter(|r| !js::trim(r).is_empty()) {
                Some(reply) => lines.push(format!("Its last reply:\n{}", quote(reply))),
                None => lines.push("It has no replies yet.".to_owned()),
            }
            if let Some(transcript) = transcript.as_deref().filter(|t| !t.is_empty()) {
                lines.push(format!("Its whole conversation, if you need more: {transcript}"));
            }
            lines.join("\n")
        }
        ContextItem::Note { text } => format!("## Note\n{}", js::trim(text)),
        ContextItem::View { title, description } => format!("## View “{title}”\n{}", js::trim(description)),
    }
}

/// The prompt, after what was selected on the board, written out for Claude
/// to read first.
pub fn with_context(items: &[ContextItem], prompt: &str) -> String {
    if items.is_empty() {
        return prompt.to_owned();
    }
    let head = "For context, I selected these on my board:";
    let body = items.iter().map(describe).collect::<Vec<_>>().join("\n\n");
    format!("{head}\n\n{body}\n\n---\n\n{prompt}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(text: &str) -> Vec<&'static str> {
        match_commands(text).into_iter().map(|c| c.command).collect()
    }

    #[test]
    fn match_commands_offers_every_command_for_a_lone_slash() {
        assert_eq!(commands("/"), ["/view", "/note"]);
    }

    #[test]
    fn match_commands_narrows_as_the_word_is_typed_in_any_case() {
        assert_eq!(commands("/V"), ["/view"]);
        assert_eq!(commands("/note"), ["/note"]);
    }

    #[test]
    fn match_commands_offers_nothing_for_other_words_or_once_there_is_a_space() {
        assert!(commands("/x").is_empty());
        assert!(commands("/view ").is_empty());
        assert!(commands("fix the /view").is_empty());
        assert!(commands("").is_empty());
    }

    #[test]
    fn take_command_takes_a_command_followed_by_a_space_with_what_comes_after() {
        assert_eq!(take_command("/view "), Some((Target::View, String::new())));
        assert_eq!(
            take_command("/NOTE  ask finance\nby Friday"),
            Some((Target::Note, "ask finance\nby Friday".to_owned()))
        );
    }

    #[test]
    fn take_command_leaves_a_command_still_being_typed_and_anything_else() {
        assert_eq!(take_command("/view"), None);
        assert_eq!(take_command("/views of x"), None);
        assert_eq!(take_command("add a /view"), None);
    }

    #[test]
    fn with_context_leaves_a_prompt_alone_when_nothing_is_selected() {
        assert_eq!(with_context(&[], "Fix it"), "Fix it");
    }

    #[test]
    fn with_context_writes_the_selection_out_before_the_prompt() {
        let prompt = with_context(
            &[
                ContextItem::Session {
                    title: "Fix cart rounding".into(),
                    branch: Some("fix/cart-rounding".into()),
                    last_reply: Some("All 6 tests pass.\n\nDone.".into()),
                    transcript: Some("/t/a.jsonl".into()),
                },
                ContextItem::Session {
                    title: "New session".into(),
                    branch: None,
                    last_reply: None,
                    transcript: None,
                },
                ContextItem::Note {
                    text: " EUR rounding: ask finance ".into(),
                },
                ContextItem::View {
                    title: "Tests".into(),
                    description: "Test results by file".into(),
                },
            ],
            "Why does cart.test.js fail?",
        );
        assert_eq!(
            prompt,
            [
                "For context, I selected these on my board:",
                "",
                "## Session “Fix cart rounding”",
                "Branch: fix/cart-rounding",
                "Its last reply:",
                "> All 6 tests pass.",
                ">",
                "> Done.",
                "Its whole conversation, if you need more: /t/a.jsonl",
                "",
                "## Session “New session”",
                "It has no replies yet.",
                "",
                "## Note",
                "EUR rounding: ask finance",
                "",
                "## View “Tests”",
                "Test results by file",
                "",
                "---",
                "",
                "Why does cart.test.js fail?",
            ]
            .join("\n")
        );
    }
}
