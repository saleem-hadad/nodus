//! Focus mode: which projects the grid and the rail show while it's on.
//!
//! A port of `src/renderer/src/components/projects/focus.ts`. The core's
//! add-project asks the same question (UI-300), so a project added while
//! focusing joins the focus.

use crate::{AppSettings, Project, ProjectStats};

/// Whether focus mode is on: offered in Settings, and switched on in the
/// title bar (UI-296). Settings not loaded yet count as off.
pub fn focus_mode_on(settings: Option<&AppSettings>) -> bool {
    settings.is_some_and(|s| s.focus_feature && s.focus_mode)
}

/// Whether focus mode is narrowing the project lists: it's on, and some
/// project (not archived) is in focus. With nothing in focus, every project
/// shows (UI-297).
pub fn is_focusing(projects: &[Project], focus_mode: bool) -> bool {
    focus_mode
        && projects
            .iter()
            .any(|p| p.focused == Some(true) && p.archived != Some(true))
}

/// Whether a project shows while focusing: it's in focus, one of its sessions
/// is waiting on the user (so nothing that needs you goes out of sight), or
/// it's `open_id`, the project whose board is open (UI-298).
pub fn shows_in_focus(project: &Project, stats: Option<&ProjectStats>, open_id: Option<&str>) -> bool {
    project.focused == Some(true) || stats.is_some_and(|s| s.waiting > 0) || open_id == Some(project.id.as_str())
}

#[cfg(test)]
mod tests {
    //! focus.test.ts
    use super::*;
    use crate::{FolderColor, RunningCounts};
    use serde_json::Map;

    fn project(id: &str, focused: bool, archived: bool) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            repo_path: format!("/repos/{id}"),
            color: FolderColor::Blue,
            created_at: 0.0,
            opened_at: None,
            group_id: None,
            actions: None,
            focused: focused.then_some(true),
            archived: archived.then_some(true),
            extra: Map::new(),
        }
    }

    fn stats(waiting: u32) -> ProjectStats {
        ProjectStats {
            total: 1,
            active: 1,
            working: 0,
            waiting,
            running: RunningCounts::default(),
        }
    }

    fn settings(focus_feature: bool, focus_mode: bool) -> AppSettings {
        AppSettings {
            focus_feature,
            focus_mode,
            ..AppSettings::default()
        }
    }

    #[test]
    fn focus_mode_on_is_on_only_while_focus_is_offered_in_settings_and_switched_on() {
        assert!(focus_mode_on(Some(&settings(true, true))));
        assert!(!focus_mode_on(Some(&settings(true, false))));
        assert!(!focus_mode_on(None));
    }

    #[test]
    fn focus_mode_on_stays_off_while_focus_is_turned_off_in_settings_whatever_the_switch_says() {
        assert!(!focus_mode_on(Some(&settings(false, true))));
    }

    #[test]
    fn is_focusing_is_off_while_focus_mode_is_off_even_with_projects_in_focus() {
        assert!(!is_focusing(&[project("a", true, false)], false));
    }

    #[test]
    fn is_focusing_narrows_the_lists_once_focus_mode_is_on_and_a_project_is_in_focus() {
        assert!(is_focusing(
            &[project("a", true, false), project("b", false, false)],
            true
        ));
    }

    #[test]
    fn is_focusing_shows_everything_when_nothing_is_in_focus() {
        assert!(!is_focusing(
            &[project("a", false, false), project("b", false, false)],
            true
        ));
    }

    #[test]
    fn is_focusing_ignores_archived_projects_in_focus() {
        assert!(!is_focusing(
            &[project("a", true, true), project("b", false, false)],
            true
        ));
    }

    #[test]
    fn shows_in_focus_shows_the_projects_in_focus() {
        assert!(shows_in_focus(&project("a", true, false), None, None));
    }

    #[test]
    fn shows_in_focus_hides_the_others() {
        assert!(!shows_in_focus(&project("a", false, false), None, None));
        assert!(!shows_in_focus(&project("a", false, false), Some(&stats(0)), None));
    }

    #[test]
    fn shows_in_focus_shows_a_project_out_of_focus_while_a_session_in_it_waits_on_you() {
        assert!(shows_in_focus(&project("a", false, false), Some(&stats(1)), None));
    }

    #[test]
    fn shows_in_focus_shows_the_project_whose_board_is_open() {
        assert!(shows_in_focus(&project("a", false, false), None, Some("a")));
        assert!(!shows_in_focus(&project("a", false, false), None, Some("b")));
    }
}
