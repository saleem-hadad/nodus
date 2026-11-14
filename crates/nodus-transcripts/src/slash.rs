//! The slash commands Claude Code offers in a session, for the chat's `/`
//! menu (CHAT-042…CHAT-046 in spec 16): Claude Code's own, the commands and
//! skills in the session's folder and in the user's `~/.claude`, the enabled
//! plugins', and the skills Claude Code listed in the transcript (its bundled
//! ones and those synced from claude.ai, which aren't anywhere we can read).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read as _;
use std::path::Path;

use serde_json::Value;

/// Only this much of a command's or skill's file is read: its frontmatter
/// and first line are at the top.
const HEAD_BYTES: u64 = 16 * 1024;

/// A description is cut to this many characters.
const DESCRIPTION_MAX_CHARS: usize = 240;

/// No folder gives more than this many commands.
const MAX_COMMANDS: usize = 1000;

/// How deep a `commands` folder is looked into.
const COMMANDS_DEPTH: usize = 4;

/// One command the `/` menu offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    /// Its name, without the slash: `compact`, `taco`, `plugin:skill`.
    pub name: String,
    pub description: String,
    /// What it takes after its name (`[filename]`), or nothing.
    pub hint: String,
    /// Other names Claude Code takes for it (built-ins only).
    pub aliases: Vec<String>,
    pub source: Source,
}

/// Where a command comes from (CHAT-043).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Claude Code's own.
    BuiltIn,
    /// The session's folder's `.claude`.
    Project,
    /// The user's `~/.claude`.
    User,
    /// An enabled plugin, by its name.
    Plugin(String),
    /// A skill Claude Code listed in the transcript and we found nowhere else.
    Listed,
}

impl Source {
    /// What the menu says about it; nothing for Claude Code's own.
    pub fn label(&self) -> &str {
        match self {
            Self::BuiltIn => "",
            Self::Project => "project",
            Self::User => "user",
            Self::Plugin(name) => name,
            Self::Listed => "skill",
        }
    }
}

/// Claude Code's own commands, as of 2.1.281: name and description.
const BUILT_IN: &[(&str, &str)] = &[
    ("add-dir", "Add a new working directory"),
    ("advisor", "Let Claude consult a stronger model at key moments"),
    ("artifacts", "Browse your published and shared artifacts"),
    ("autocompact", "Set how full the context gets before auto-summarizing"),
    (
        "background",
        "Send this session to the background and free the terminal",
    ),
    ("branch", "Create a branch of the current conversation at this point"),
    (
        "btw",
        "Ask a quick side question without interrupting the main conversation",
    ),
    ("cd", "Move this session to a new working directory"),
    (
        "clear",
        "Start a new session with empty context; previous session stays on disk",
    ),
    ("color", "Set the prompt bar color for this session"),
    ("compact", "Free up context by summarizing the conversation so far"),
    ("config", "Open settings"),
    ("context", "Show current context usage"),
    (
        "copy",
        "Copy Claude's last response to clipboard (or /copy N for the Nth-latest)",
    ),
    ("diff", "Show uncommitted changes and what each turn changed"),
    ("doctor", "Health-check your setup and fix issues"),
    ("effort", "Set effort level for model usage"),
    ("exit", "Exit Claude Code"),
    ("export", "Export the current conversation to a file or clipboard"),
    ("fast", "Toggle fast mode"),
    ("feedback", "Send feedback to Anthropic or report a bug"),
    ("focus", "Toggle focus view: just your prompt, summary, and response"),
    ("fork", "Spawn a background agent that inherits the full conversation"),
    ("goal", "Set a goal Claude checks before stopping"),
    ("help", "Show help and available commands"),
    ("hooks", "View hook configurations for tool events"),
    ("ide", "Manage IDE integrations and show status"),
    ("import", "Import config from another AI coding agent"),
    ("init", "Initialize a new CLAUDE.md file with codebase documentation"),
    ("insights", "Generate a report analyzing your Claude Code sessions"),
    ("install-github-app", "Set up Claude GitHub Actions for a repository"),
    ("install-slack-app", "Install the Claude Slack app"),
    ("keybindings", "Open your keyboard shortcuts file"),
    (
        "list-agents",
        "List subagents, teammates, and other Claude sessions you can message",
    ),
    ("login", "Sign in with your Anthropic account"),
    ("logout", "Sign out from your Anthropic account"),
    ("loops", "List, create, and delete loops"),
    ("mcp", "Manage MCP servers"),
    ("memory", "Edit CLAUDE.md files and memory settings"),
    ("model", "Set the AI model for Claude Code"),
    ("output-style", "List output styles or switch to one"),
    ("permissions", "Manage allow and deny tool permission rules"),
    ("plan", "Enable plan mode or view the current session plan"),
    ("plugin", "Manage Claude Code plugins"),
    (
        "powerup",
        "Discover Claude Code features through quick interactive lessons",
    ),
    ("privacy-settings", "View and update your privacy settings"),
    ("recap", "Generate a one-line session recap now"),
    ("release-notes", "View release notes"),
    (
        "reload-plugins",
        "Activate pending plugin changes in the current session",
    ),
    (
        "reload-skills",
        "Pick up skills added or changed on disk during this session",
    ),
    ("rename", "Rename the current conversation"),
    ("resume", "Resume a previous conversation"),
    ("rewind", "Restore the code and/or conversation to a previous point"),
    (
        "security-review",
        "Complete a security review of the pending changes on the current branch",
    ),
    ("skills", "List available skills"),
    (
        "status",
        "Show Claude Code status including version, model, account and API connectivity",
    ),
    ("statusline", "Set up Claude Code's status line UI"),
    (
        "subtask",
        "Send a subagent off with your full context; its result comes back here",
    ),
    ("tasks", "View and manage everything running in the background"),
    (
        "teleport",
        "Send this session to the cloud, or resume one from claude.ai",
    ),
    ("terminal-setup", "Install the Shift+Enter key binding for newlines"),
    ("theme", "Change the theme"),
    ("ultraplan", "A cloud session drafts a plan you can edit and approve"),
    ("update", "Switch to the latest version (conversation continues)"),
    ("upgrade", "Upgrade to Max for higher rate limits and more Opus"),
    ("usage", "Show session cost, plan usage, and activity stats"),
    ("version", "Show this session's version"),
    ("voice", "Toggle voice mode"),
    ("workflows", "Browse running and completed workflows"),
];

/// What some of [`BUILT_IN`] take after their name.
const BUILT_IN_HINTS: &[(&str, &str)] = &[
    ("add-dir", "<path>"),
    ("autocompact", "[auto|<tokens>]"),
    ("background", "[prompt]"),
    ("branch", "[name]"),
    ("btw", "[question]"),
    ("cd", "<path>"),
    ("compact", "[instructions]"),
    ("copy", "[N]"),
    ("effort", "[level]"),
    ("export", "[filename]"),
    ("fast", "[on|off]"),
    ("feedback", "[report]"),
    ("fork", "<directive>"),
    ("goal", "[<condition> | clear]"),
    ("ide", "[open]"),
    ("import", "[codex|gemini|cursor]"),
    ("mcp", "[reconnect|enable|disable]"),
    ("model", "[model]"),
    ("output-style", "[style]"),
    ("plan", "[open|<description>]"),
    ("rename", "[name]"),
    ("resume", "[conversation id or search term]"),
    ("subtask", "<task>"),
    ("ultraplan", "<prompt>"),
    ("voice", "[hold|tap|off]"),
];

/// The other names Claude Code takes for some of [`BUILT_IN`].
const BUILT_IN_ALIASES: &[(&str, &[&str])] = &[
    ("background", &["bg"]),
    ("clear", &["reset", "new"]),
    ("config", &["settings"]),
    ("doctor", &["checkup"]),
    ("exit", &["quit"]),
    ("feedback", &["bug"]),
    ("list-agents", &["peers"]),
    ("permissions", &["allowed-tools"]),
    ("plugin", &["plugins", "marketplace"]),
    ("resume", &["continue"]),
    ("rewind", &["checkpoint", "undo"]),
    ("tasks", &["bashes"]),
    ("teleport", &["tp"]),
    ("update", &["restart"]),
    ("usage", &["cost", "stats"]),
];

/// CHAT-043: Claude Code's own commands.
pub fn built_in() -> Vec<SlashCommand> {
    let find = |table: &[(&str, &'static str)], name: &str| table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
    BUILT_IN
        .iter()
        .map(|(name, description)| SlashCommand {
            name: (*name).to_owned(),
            description: (*description).to_owned(),
            hint: find(BUILT_IN_HINTS, name).unwrap_or_default().to_owned(),
            aliases: BUILT_IN_ALIASES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, aliases)| aliases.iter().map(|a| (*a).to_owned()).collect())
                .unwrap_or_default(),
            source: Source::BuiltIn,
        })
        .collect()
}

/// CHAT-043: the commands and skills on disk for a session running in
/// `folder`: the folder's `.claude/commands` and `.claude/skills`, the same
/// under `home`'s `.claude`, then each enabled plugin's. The first of a name
/// wins.
pub fn on_disk(home: &Path, folder: &Path) -> Vec<SlashCommand> {
    let mut found = Vec::new();
    let claude = home.join(".claude");
    let project = folder.join(".claude");
    if project != claude {
        read_place(&project, &Source::Project, "", &mut found);
    }
    read_place(&claude, &Source::User, "", &mut found);
    for (name, path) in enabled_plugins(&claude, &project, folder) {
        let prefix = format!("{name}:");
        read_place(Path::new(&path), &Source::Plugin(name), &prefix, &mut found);
    }
    found
}

/// CHAT-043: everything the menu offers, by name: what's on disk first, then
/// Claude Code's own, then the skills only the transcript listed.
pub fn merge(on_disk: &[SlashCommand], listed: &[SlashCommand]) -> Vec<SlashCommand> {
    let mut seen = HashSet::new();
    let mut all: Vec<SlashCommand> = on_disk
        .iter()
        .cloned()
        .chain(built_in())
        .chain(listed.iter().cloned())
        .filter(|c| seen.insert(c.name.to_lowercase()))
        .collect();
    all.sort_by_key(|c| c.name.to_lowercase());
    all
}

/// CHAT-044: the commands that what's typed after the slash could mean,
/// best first: the name or an alias itself, then names that start with it,
/// then aliases that do, then names with a part (after `:` or `-`) that
/// does, then names that hold it anywhere, then descriptions that do. Ties
/// keep `commands`' order. Everything, as it is, for nothing typed.
pub fn matching<'a>(commands: &'a [SlashCommand], typed: &str) -> Vec<&'a SlashCommand> {
    let typed = typed.to_lowercase();
    if typed.is_empty() {
        return commands.iter().collect();
    }
    let rank = |c: &SlashCommand| {
        let name = c.name.to_lowercase();
        let aliases = || c.aliases.iter().map(|a| a.to_lowercase());
        if name == typed || aliases().any(|a| a == typed) {
            Some(0)
        } else if name.starts_with(&typed) {
            Some(1)
        } else if aliases().any(|a| a.starts_with(&typed)) {
            Some(2)
        } else if name.split([':', '-', '_']).skip(1).any(|part| part.starts_with(&typed)) {
            Some(3)
        } else if name.contains(&typed) {
            Some(4)
        } else if typed.chars().count() >= 3 && c.description.to_lowercase().contains(&typed) {
            Some(5)
        } else {
            None
        }
    };
    let mut ranked: Vec<(u8, &SlashCommand)> = commands.iter().filter_map(|c| Some((rank(c)?, c))).collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, c)| c).collect()
}

/// CHAT-043: the skills a `skill_listing` attachment names, with the first
/// line of what it says about each (`- name: description`).
pub fn listed_skills(names: &[&str], content: &str) -> Vec<SlashCommand> {
    let mut said: HashMap<&str, &str> = HashMap::new();
    for line in content.lines() {
        if let Some((name, description)) = line.strip_prefix("- ").and_then(|l| l.split_once(": ")) {
            said.entry(name).or_insert(description);
        }
    }
    names
        .iter()
        .filter(|name| is_command_name(name))
        .map(|name| SlashCommand {
            name: (*name).to_owned(),
            description: clip(said.get(name).copied().unwrap_or_default()),
            hint: String::new(),
            aliases: Vec::new(),
            source: Source::Listed,
        })
        .collect()
}

/// What can follow a slash: no spaces, nothing empty.
fn is_command_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(char::is_whitespace)
}

/// A `.claude` folder's (or a plugin's) `commands/**/*.md` and
/// `skills/*/SKILL.md`, each name after `prefix`.
fn read_place(dir: &Path, source: &Source, prefix: &str, found: &mut Vec<SlashCommand>) {
    read_commands(&dir.join("commands"), source, prefix, COMMANDS_DEPTH, found);
    let Ok(skills) = fs::read_dir(dir.join("skills")) else {
        return;
    };
    let mut skills: Vec<_> = skills.flatten().map(|e| e.path()).collect();
    skills.sort();
    for skill in skills {
        if found.len() >= MAX_COMMANDS {
            return;
        }
        let Some(text) = head(&skill.join("SKILL.md")) else {
            continue;
        };
        let (meta, _) = frontmatter(&text);
        if meta.get("user-invocable").is_some_and(|v| v == "false") {
            continue;
        }
        let dir_name = skill.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let name = meta.get("name").map_or(dir_name, String::as_str);
        push(found, prefix, name, &meta, None, source);
    }
}

/// The Markdown files under a `commands` folder, each a command named after
/// its file (CHAT-043): subfolders only sort them.
fn read_commands(dir: &Path, source: &Source, prefix: &str, depth: usize, found: &mut Vec<SlashCommand>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if found.len() >= MAX_COMMANDS {
            return;
        }
        if path.is_dir() {
            if depth > 0 {
                read_commands(&path, source, prefix, depth - 1, found);
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let (Some(stem), Some(text)) = (path.file_stem().and_then(|s| s.to_str()), head(&path)) else {
            continue;
        };
        let (meta, body) = frontmatter(&text);
        push(found, prefix, stem, &meta, Some(body), source);
    }
}

fn push(
    found: &mut Vec<SlashCommand>,
    prefix: &str,
    name: &str,
    meta: &HashMap<String, String>,
    body: Option<&str>,
    source: &Source,
) {
    let name = format!("{prefix}{name}");
    if !is_command_name(&name) || found.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
        return;
    }
    // A command without a description is described by its first line.
    let description = meta.get("description").cloned().unwrap_or_else(|| {
        body.and_then(|b| b.lines().map(str::trim).find(|l| !l.is_empty()))
            .map(|l| l.trim_start_matches('#').trim().to_owned())
            .unwrap_or_default()
    });
    found.push(SlashCommand {
        name,
        description: clip(&description),
        hint: meta.get("argument-hint").cloned().unwrap_or_default(),
        aliases: Vec::new(),
        source: source.clone(),
    });
}

/// The plugins enabled for a session, as (name, install folder): those
/// `installed_plugins.json` lists that the user's settings, then the
/// project's, then the project's local settings turn on. A plugin installed
/// for one project counts only in that project.
fn enabled_plugins(claude: &Path, project: &Path, folder: &Path) -> Vec<(String, String)> {
    let mut enabled: HashMap<String, bool> = HashMap::new();
    for settings in [
        claude.join("settings.json"),
        project.join("settings.json"),
        project.join("settings.local.json"),
    ] {
        let Some(Value::Object(plugins)) = read_json(&settings).and_then(|s| s.get("enabledPlugins").cloned()) else {
            continue;
        };
        for (key, on) in plugins {
            if let Some(on) = on.as_bool() {
                enabled.insert(key, on);
            }
        }
    }
    let Some(Value::Object(installed)) =
        read_json(&claude.join("plugins/installed_plugins.json")).and_then(|i| i.get("plugins").cloned())
    else {
        return Vec::new();
    };
    let mut keys: Vec<&String> = installed.keys().collect();
    keys.sort();
    let mut plugins = Vec::new();
    for key in keys {
        if enabled.get(key) != Some(&true) {
            continue;
        }
        let name = key.split('@').next().unwrap_or(key).to_owned();
        let installs = installed[key].as_array().map(Vec::as_slice).unwrap_or_default();
        let install = installs
            .iter()
            .find(|i| match i.get("projectPath").and_then(Value::as_str) {
                Some(only) => Path::new(only) == folder,
                None => true,
            });
        if let Some(path) = install.and_then(|i| i.get("installPath")).and_then(Value::as_str) {
            plugins.push((name, path.to_owned()));
        }
    }
    plugins
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// The start of a file, as text.
fn head(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A description cut to [`DESCRIPTION_MAX_CHARS`], on one line.
fn clip(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= DESCRIPTION_MAX_CHARS {
        return one_line;
    }
    let cut: String = one_line.chars().take(DESCRIPTION_MAX_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// A Markdown file's YAML frontmatter as far as commands and skills use it
/// (`key: value` at the top level, plain, quoted or folded with `>` or `|`),
/// and the text after it.
fn frontmatter(text: &str) -> (HashMap<String, String>, &str) {
    let mut meta = HashMap::new();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---")
        .and_then(|r| r.strip_prefix('\n').or(r.strip_prefix("\r\n")))
    else {
        return (meta, text);
    };
    let mut lines = Vec::new();
    let mut body = "";
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare == "---" || bare == "..." {
            body = &rest[offset..];
            break;
        }
        lines.push(bare);
    }
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.starts_with([' ', '\t', '#']) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        // The lines indented under the key carry on its value.
        let mut more = Vec::new();
        while i < lines.len() && (lines[i].starts_with([' ', '\t']) || lines[i].trim().is_empty()) {
            more.push(lines[i].trim());
            i += 1;
        }
        while more.last().is_some_and(|l| l.is_empty()) {
            more.pop();
        }
        let value = match value.chars().next() {
            Some('|') => more.join("\n"),
            Some('>') => fold(&more),
            Some('"') => unquote(value, '"'),
            Some('\'') => unquote(value, '\''),
            _ => fold(&std::iter::once(value).chain(more.iter().copied()).collect::<Vec<_>>()),
        };
        meta.insert(key.to_owned(), js_trim(&value));
    }
    (meta, body)
}

/// Folded lines: one space between lines, a blank line a line break.
fn fold(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        if line.is_empty() {
            out.push('\n');
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            out.push_str(line);
        }
    }
    out
}

/// A one-line quoted value: `"…"` with its escapes, `'…'` with `''`.
fn unquote(value: &str, quote: char) -> String {
    let inner = value.strip_prefix(quote).unwrap_or(value);
    let inner = inner.rfind(quote).map_or(inner, |end| &inner[..end]);
    if quote == '\'' {
        return inner.replace("''", "'");
    }
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn js_trim(text: &str) -> String {
    nodus_domain::js::trim(text).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn names(commands: &[&SlashCommand]) -> Vec<String> {
        commands.iter().map(|c| c.name.clone()).collect()
    }

    #[test]
    fn frontmatter_reads_plain_quoted_and_folded_values() {
        let (meta, body) = frontmatter(
            "---\nname: taco\ndescription: >-\n  Work on tasks\n  from Taco.\nargument-hint: \"[task id]\"\nnote: 'it''s'\nlong: one\n  two\n---\n# Body\n",
        );
        assert_eq!(meta["name"], "taco");
        assert_eq!(meta["description"], "Work on tasks from Taco.");
        assert_eq!(meta["argument-hint"], "[task id]");
        assert_eq!(meta["note"], "it's");
        assert_eq!(meta["long"], "one two");
        assert_eq!(body, "# Body\n");
        let (meta, body) = frontmatter("Just a prompt\n");
        assert!(meta.is_empty());
        assert_eq!(body, "Just a prompt\n");
    }

    #[test]
    fn chat_043_reads_the_projects_the_users_and_enabled_plugins_commands_and_skills() {
        let home = tempfile::TempDir::new().unwrap();
        let repo = tempfile::TempDir::new().unwrap();
        let claude = home.path().join(".claude");
        write(
            &repo.path().join(".claude/commands/ship.md"),
            "---\ndescription: Ship it\nargument-hint: [branch]\n---\nShip $ARGUMENTS",
        );
        write(
            &repo.path().join(".claude/commands/ops/deploy.md"),
            "# Deploy the app\n\nSteps…",
        );
        write(&repo.path().join(".claude/commands/notes.txt"), "not a command");
        write(
            &claude.join("skills/taco/SKILL.md"),
            "---\nname: taco\ndescription: >-\n  Work on Taco\n  tasks\n---\n",
        );
        write(
            &claude.join("skills/hidden/SKILL.md"),
            "---\nname: hidden\ndescription: x\nuser-invocable: false\n---\n",
        );
        // The project's own wins over the user's of the same name.
        write(
            &claude.join("commands/ship.md"),
            "---\ndescription: The user's ship\n---\n",
        );
        let plugin = home.path().join("plugin-files");
        write(&plugin.join("commands/hello.md"), "---\ndescription: Say hello\n---\n");
        write(
            &plugin.join("skills/slides/SKILL.md"),
            "---\nname: slides\ndescription: Make slides\n---\n",
        );
        let off = home.path().join("off-files");
        write(&off.join("commands/nope.md"), "Off\n");
        write(
            &claude.join("plugins/installed_plugins.json"),
            &serde_json::json!({ "version": 2, "plugins": {
                "deck@market": [{ "scope": "user", "installPath": plugin }],
                "off@market": [{ "scope": "user", "installPath": off }],
            }})
            .to_string(),
        );
        write(
            &claude.join("settings.json"),
            r#"{ "enabledPlugins": { "deck@market": true, "off@market": false } }"#,
        );
        let found = on_disk(home.path(), repo.path());
        let summary: Vec<(&str, &str, &str, &str)> = found
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.description.as_str(),
                    c.hint.as_str(),
                    c.source.label(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("deploy", "Deploy the app", "", "project"),
                ("ship", "Ship it", "[branch]", "project"),
                ("taco", "Work on Taco tasks", "", "user"),
                ("deck:hello", "Say hello", "", "deck"),
                ("deck:slides", "Make slides", "", "deck"),
            ]
        );
    }

    #[test]
    fn chat_043_a_session_in_the_home_folder_reads_its_claude_once() {
        let home = tempfile::TempDir::new().unwrap();
        write(&home.path().join(".claude/commands/a.md"), "A\n");
        let found = on_disk(home.path(), home.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, Source::User);
    }

    #[test]
    fn chat_043_the_transcripts_skill_listing_names_its_skills() {
        let listed = listed_skills(
            &["simplify", "anthropic-skills:pdf", "bad name"],
            "- simplify: Review the changed code\n  and more\n- anthropic-skills:pdf: Use this skill for PDFs\n",
        );
        let summary: Vec<(&str, &str)> = listed
            .iter()
            .map(|c| (c.name.as_str(), c.description.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                ("simplify", "Review the changed code"),
                ("anthropic-skills:pdf", "Use this skill for PDFs")
            ]
        );
    }

    #[test]
    fn chat_043_merge_keeps_the_first_of_a_name_and_sorts_by_name() {
        let disk = [SlashCommand {
            name: "init".into(),
            description: "Our own init".into(),
            hint: String::new(),
            aliases: Vec::new(),
            source: Source::Project,
        }];
        let listed = listed_skills(&["init", "simplify"], "- init: Listed init\n- simplify: Review\n");
        let all = merge(&disk, &listed);
        let init = all.iter().find(|c| c.name == "init").unwrap();
        assert_eq!(init.description, "Our own init");
        assert!(all.iter().any(|c| c.name == "simplify" && c.source == Source::Listed));
        assert!(all.iter().any(|c| c.name == "compact" && c.source == Source::BuiltIn));
        let sorted: Vec<String> = all.iter().map(|c| c.name.to_lowercase()).collect();
        assert!(sorted.is_sorted());
    }

    #[test]
    fn chat_044_matching_ranks_names_then_parts_then_anywhere_then_descriptions() {
        let all = merge(
            &[],
            &listed_skills(
                &["anthropic-skills:pdf", "compactor"],
                "- anthropic-skills:pdf: Read PDFs\n- compactor: Packs things\n",
            ),
        );
        assert_eq!(names(&matching(&all, "compact"))[..2], ["compact", "compactor"]);
        assert_eq!(names(&matching(&all, "COST")), ["usage"]);
        // An alias comes after the names it shares a start with.
        let s = names(&matching(&all, "se"));
        assert_eq!(s[0], "security-review");
        assert!(s.iter().position(|n| n == "config") > s.iter().position(|n| n == "security-review"));
        assert_eq!(names(&matching(&all, "pdf")), ["anthropic-skills:pdf"]);
        assert_eq!(names(&matching(&all, "review"))[0], "security-review");
        assert!(names(&matching(&all, "packs")).contains(&"compactor".to_owned()));
        assert!(matching(&all, "zzzz").is_empty());
        assert_eq!(matching(&all, "").len(), all.len());
    }
}
