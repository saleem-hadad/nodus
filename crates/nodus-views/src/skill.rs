//! Nodus's own Claude Code plugin, `nodus`, with one skill: `nodus:views`,
//! on making views and pages. The app writes it into its data folder at
//! start, and every Claude process with the board tools loads it with
//! `--plugin-dir`, for that process alone, so nothing goes into the user's
//! `~/.claude` or the repository (VIEW-159, VIEW-160).
//!
//! The skill is one SKILL.md: when to use which tool, handing a view to its
//! brain, writing a page, the guide (`VIEW_GUIDE`) and worked examples. The
//! Skill tool loads it whole, so Claude never reads a file outside the
//! project for it, which would stop to ask the user.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::page::VIEW_GUIDE;

/// The plugin's folder in the app's data folder.
pub const PLUGIN_DIR: &str = "claude-plugin";

/// What comes before the guide: when to use it, the brief, the workflow.
const HEAD: &str = include_str!("skill/head.md");

/// Pages that follow the guide, each with what it shows how to do.
const EXAMPLES: [(&str, &str, &str); 4] = [
    (
        "A live table from a command",
        "Declares one command and draws every result, scheduled runs included. Handles the command waiting for the user to allow it, failing, and having nothing to show.",
        include_str!("skill/live-table.html"),
    ),
    (
        "Numbers and a chart that Claude keeps fresh",
        "Draws nodus.data, which the view's brain sends with update_view, as stat tiles and an inline SVG chart. Its Update button asks the brain, and shows Claude at work until the new data arrives.",
        include_str!("skill/stats-chart.html"),
    ),
    (
        "The board at a glance",
        "No data or commands: it reads the board's sessions with nodus.sessions(), shows each with the board's status dot, and opens one on a click.",
        include_str!("skill/board-sessions.html"),
    ),
    (
        "A choice sent back to the session (show_page)",
        "Two layouts as mockups side by side, a note and a Send button that calls nodus.submit.",
        include_str!("skill/choice.html"),
    ),
];

/// The skill's SKILL.md (VIEW-160).
pub fn skill_markdown() -> String {
    let mut out = String::from(HEAD);
    out.push_str("\n## How a page works\n\n");
    out.push_str(VIEW_GUIDE);
    out.push_str("\n\n## Examples\n");
    for (i, (title, about, html)) in EXAMPLES.iter().enumerate() {
        out.push_str(&format!(
            "\n### {n}. {title}\n\n{about}\n\n```html\n{html}\n```\n",
            n = i + 1,
            html = html.trim_end()
        ));
    }
    out
}

/// The plugin's `.claude-plugin/plugin.json`.
fn manifest(version: &str) -> String {
    let manifest = json!({
        "name": "nodus",
        "version": version,
        "description": "Nodus's own skills, for the Claude sessions it runs."
    });
    format!("{manifest:#}\n")
}

/// Writes the plugin into `user_data`, leaving files that already say the
/// same alone, and returns its folder (VIEW-159).
pub fn write_plugin(user_data: &Path, version: &str) -> io::Result<PathBuf> {
    let dir = user_data.join(PLUGIN_DIR);
    write_if_changed(&dir.join(".claude-plugin").join("plugin.json"), &manifest(version))?;
    write_if_changed(&dir.join("skills").join("views").join("SKILL.md"), &skill_markdown())?;
    Ok(dir)
}

/// Through `<name>.<pid>.tmp` and a rename, so a session loading the skill
/// never sees it half written.
fn write_if_changed(path: &Path, text: &str) -> io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_has_its_name_the_guide_and_every_example() {
        let skill = skill_markdown();
        assert!(skill.starts_with("---\nname: views\ndescription: "));
        assert!(skill.contains(VIEW_GUIDE));
        assert_eq!(skill.matches("```html\n<!doctype html>").count(), EXAMPLES.len());
        // Every example closes its fence.
        assert_eq!(skill.matches("\n```\n").count(), EXAMPLES.len());
    }

    #[test]
    fn writing_it_twice_leaves_it_alone_and_a_new_version_rewrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = write_plugin(dir.path(), "1.0.0").unwrap();
        assert_eq!(plugin, dir.path().join(PLUGIN_DIR));
        let manifest_path = plugin.join(".claude-plugin/plugin.json");
        let skill_path = plugin.join("skills/views/SKILL.md");
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["name"], "nodus");
        assert_eq!(manifest["version"], "1.0.0");
        assert_eq!(std::fs::read_to_string(&skill_path).unwrap(), skill_markdown());

        let written = std::fs::metadata(&skill_path).unwrap().modified().unwrap();
        write_plugin(dir.path(), "1.0.1").unwrap();
        assert_eq!(std::fs::metadata(&skill_path).unwrap().modified().unwrap(), written);
        assert!(std::fs::read_to_string(&manifest_path).unwrap().contains("\"1.0.1\""));
        // No temporary files left behind.
        let leftovers = std::fs::read_dir(plugin.join("skills/views")).unwrap().count();
        assert_eq!(leftovers, 1);
    }
}
