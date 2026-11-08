//! The title bar's count of the Claude sessions the app is running, in every
//! project (`runningSessions.ts`; UI-102…UI-105).

use indexmap::IndexMap;
use nodus_domain::{Project, ProjectStats, RunningCounts};

/// Claude processes, hidden ones included. Shells aren't counted.
fn claude(r: &RunningCounts) -> u32 {
    r.sessions + r.background
}

/// `runningTotal`: Claude sessions running across projects.
pub fn running_total(stats: &IndexMap<String, ProjectStats>) -> u32 {
    stats.values().map(|s| claude(&s.running)).sum()
}

/// `runningByProject`: each project with Claude sessions running, and how
/// many, in project order: (id, name, count).
pub fn running_by_project(projects: &[Project], stats: &IndexMap<String, ProjectStats>) -> Vec<(String, String, u32)> {
    projects
        .iter()
        .filter_map(|p| {
            let count = stats.get(&p.id).map_or(0, |s| claude(&s.running));
            (count > 0).then(|| (p.id.clone(), p.name.clone(), count))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! runningSessions.test.ts
    use super::*;
    use nodus_domain::FolderColor;
    use serde_json::Map;

    fn stats(sessions: u32, background: u32, terminals: u32) -> ProjectStats {
        ProjectStats {
            running: RunningCounts {
                sessions,
                background,
                terminals,
            },
            ..ProjectStats::default()
        }
    }

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            repo_path: format!("/repos/{id}"),
            color: FolderColor::Blue,
            created_at: 0.0,
            opened_at: None,
            group_id: None,
            actions: None,
            focused: None,
            archived: None,
            extra: Map::new(),
        }
    }

    fn map(entries: &[(&str, ProjectStats)]) -> IndexMap<String, ProjectStats> {
        entries.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    fn rows(projects: &[Project], stats: &IndexMap<String, ProjectStats>) -> Vec<String> {
        running_by_project(projects, stats)
            .into_iter()
            .map(|(_, name, count)| format!("{name} ({count})"))
            .collect()
    }

    #[test]
    fn running_total_counts_claude_sessions_in_every_project_hidden_ones_too_but_not_shells() {
        assert_eq!(running_total(&map(&[("a", stats(2, 0, 3)), ("b", stats(0, 1, 0))])), 3);
    }

    #[test]
    fn running_total_is_zero_with_nothing_running() {
        assert_eq!(running_total(&map(&[])), 0);
        assert_eq!(running_total(&map(&[("a", stats(0, 0, 1))])), 0);
    }

    #[test]
    fn running_by_project_gives_each_project_with_claude_sessions_running_and_its_count_in_project_order() {
        let projects = [project("a", "p1"), project("b", "p2"), project("c", "idle")];
        let s = map(&[("b", stats(1, 0, 0)), ("a", stats(2, 0, 1)), ("c", stats(0, 0, 0))]);
        assert_eq!(rows(&projects, &s), ["p1 (2)", "p2 (1)"]);
    }

    #[test]
    fn running_by_project_counts_a_projects_hidden_sessions_with_its_cards() {
        let projects = [project("a", "p1")];
        assert_eq!(rows(&projects, &map(&[("a", stats(1, 2, 0))])), ["p1 (3)"]);
    }

    #[test]
    fn running_by_project_leaves_out_a_project_thats_only_running_shells() {
        let projects = [project("a", "p1"), project("b", "p2")];
        let s = map(&[("a", stats(0, 0, 2)), ("b", stats(1, 0, 0))]);
        assert_eq!(rows(&projects, &s), ["p2 (1)"]);
    }
}
