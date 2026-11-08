//! What's running: each project's running counts for `projects:stats`, and
//! the title bar's count of Claude sessions across projects.
//!
//! Ports the running half of `projectStats` in `src/main/board.ts` (its
//! `total` and `active` are the board's) and
//! `src/renderer/src/components/runningSessions.ts`.

use indexmap::IndexMap;
use nodus_domain::{BoardNode, Project, ProjectStats, RunningCounts, SessionKind, SessionStatus};

/// HOST-216: each node on a project's board with a live process, counted
/// once: a view's brain or an action run as `background`, a terminal as
/// `terminals`, any other as `sessions`. Brains count: every process the
/// app runs for the project has a node.
pub fn running_counts<'a>(
    nodes: impl IntoIterator<Item = &'a BoardNode>,
    is_live: impl Fn(&str) -> bool,
) -> RunningCounts {
    let mut running = RunningCounts {
        sessions: 0,
        background: 0,
        terminals: 0,
    };
    for node in nodes {
        if !is_live(&node.session_id) {
            continue;
        }
        if is_set(node.view_of.as_deref()) || is_set(node.run_of.as_deref()) {
            running.background += 1;
        } else if node.kind == Some(SessionKind::Terminal) {
            running.terminals += 1;
        } else {
            running.sessions += 1;
        }
    }
    running
}

/// HOST-217: how many of a project's live, active cards are working and how
/// many waiting, by their combined status. A view's brain isn't on the board.
pub fn working_and_waiting<'a>(
    nodes: impl IntoIterator<Item = &'a BoardNode>,
    is_live: impl Fn(&str) -> bool,
    status: impl Fn(&str) -> SessionStatus,
) -> (u32, u32) {
    let (mut working, mut waiting) = (0, 0);
    for node in nodes {
        if is_set(node.view_of.as_deref()) || node.archived || !is_live(&node.session_id) {
            continue;
        }
        match status(&node.session_id) {
            SessionStatus::Working => working += 1,
            SessionStatus::Waiting => waiting += 1,
            SessionStatus::Done => {}
        }
    }
    (working, waiting)
}

/// Claude processes, hidden ones included. Shells aren't counted.
fn claude(running: &RunningCounts) -> u32 {
    running.sessions + running.background
}

/// HOST-218: Claude sessions running across projects.
pub fn running_total<'a>(stats: impl IntoIterator<Item = &'a ProjectStats>) -> u32 {
    stats.into_iter().map(|s| claude(&s.running)).sum()
}

/// A project with Claude sessions running, for the title bar's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRunning {
    pub id: String,
    pub name: String,
    pub count: u32,
}

/// HOST-219: each project with Claude sessions running, and how many, in project order.
pub fn running_by_project(projects: &[Project], stats: &IndexMap<String, ProjectStats>) -> Vec<ProjectRunning> {
    projects
        .iter()
        .filter_map(|project| {
            let count = stats.get(&project.id).map_or(0, |s| claude(&s.running));
            (count > 0).then(|| ProjectRunning {
                id: project.id.clone(),
                name: project.name.clone(),
                count,
            })
        })
        .collect()
}

/// A field JavaScript would count as true: present and not empty.
fn is_set(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::FolderColor;
    use serde_json::json;

    fn project(id: &str, name: &str) -> Project {
        serde_json::from_value(json!({
            "id": id, "name": name, "repoPath": format!("/repos/{name}"), "color": FolderColor::Blue, "createdAt": 0
        }))
        .unwrap()
    }

    fn stats(sessions: u32, background: u32, terminals: u32) -> ProjectStats {
        ProjectStats {
            total: 0,
            active: 0,
            working: 0,
            waiting: 0,
            running: RunningCounts {
                sessions,
                background,
                terminals,
            },
        }
    }

    fn map(entries: &[(&str, ProjectStats)]) -> IndexMap<String, ProjectStats> {
        entries.iter().map(|(id, s)| ((*id).to_owned(), *s)).collect()
    }

    // runningSessions.test.ts
    #[test]
    fn running_total_counts_claude_sessions_in_every_project_hidden_ones_too_but_not_shells() {
        let all = map(&[("a", stats(2, 0, 3)), ("b", stats(0, 1, 0))]);
        assert_eq!(running_total(all.values()), 3);
    }

    #[test]
    fn running_total_is_zero_with_nothing_running() {
        assert_eq!(running_total(map(&[]).values()), 0);
        assert_eq!(running_total(map(&[("a", stats(0, 0, 1))]).values()), 0);
    }

    fn names(rows: &[ProjectRunning]) -> Vec<String> {
        rows.iter().map(|r| format!("{} ({})", r.name, r.count)).collect()
    }

    fn projects() -> Vec<Project> {
        vec![project("a", "alpha"), project("b", "beta"), project("c", "idle")]
    }

    #[test]
    fn running_by_project_gives_each_project_with_claude_sessions_running_and_its_count_in_project_order() {
        let rows = running_by_project(
            &projects(),
            &map(&[("b", stats(1, 0, 0)), ("a", stats(2, 0, 1)), ("c", stats(0, 0, 0))]),
        );
        assert_eq!(names(&rows), vec!["alpha (2)", "beta (1)"]);
        assert_eq!(rows[0].id, "a");
    }

    #[test]
    fn running_by_project_counts_a_projects_hidden_sessions_with_its_cards() {
        let rows = running_by_project(&projects(), &map(&[("a", stats(1, 2, 0))]));
        assert_eq!(names(&rows), vec!["alpha (3)"]);
    }

    #[test]
    fn running_by_project_leaves_out_a_project_thats_only_running_shells() {
        let rows = running_by_project(&projects(), &map(&[("a", stats(0, 0, 2)), ("b", stats(1, 0, 0))]));
        assert_eq!(names(&rows), vec!["beta (1)"]);
    }

    fn node(id: &str, extra: &serde_json::Value) -> BoardNode {
        let mut value = json!({"sessionId": id, "x": 0, "y": 0, "archived": false, "createdAt": 0});
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn counts_each_live_process_once_by_what_it_runs() {
        // HOST-216
        let nodes = [
            node("s1", &json!({})),
            node("s2", &json!({"archived": true})),
            node("t1", &json!({"kind": "terminal"})),
            node("v1", &json!({"viewOf": "view"})),
            node("r1", &json!({"runOf": "action"})),
            node("r2", &json!({"runOf": "action", "kind": "terminal"})),
            node("e1", &json!({"runOf": ""})),
            node("gone", &json!({})),
        ];
        let running = running_counts(&nodes, |id| id != "gone");
        assert_eq!(
            running,
            RunningCounts {
                sessions: 3,
                background: 3,
                terminals: 1
            }
        );
    }

    #[test]
    fn counts_live_active_cards_by_status() {
        // HOST-217
        let nodes = [
            node("w1", &json!({})),
            node("w2", &json!({"runOf": "action"})),
            node("q1", &json!({})),
            node("d1", &json!({})),
            node("v1", &json!({"viewOf": "view"})),
            node("x1", &json!({"archived": true})),
            node("gone", &json!({})),
        ];
        let status = |id: &str| match id {
            "q1" => SessionStatus::Waiting,
            "d1" => SessionStatus::Done,
            _ => SessionStatus::Working,
        };
        assert_eq!(working_and_waiting(&nodes, |id| id != "gone", status), (2, 1));
    }
}
