//! `window.api.projects`: the projects, adding folders as projects, and
//! changing them.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use nodus_domain::focus::is_focusing;
use nodus_domain::{FolderColor, Project, ProjectPatch, ProjectStats};
use serde_json::Map;

use crate::error::OnChannel;
use crate::realpath::realpath;
use crate::{CommandError, CommandResult, Nodus};

/// `window.api.projects`.
pub struct ProjectsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl ProjectsApi<'_> {
    /// `projects:list`: every project in the order they were added, archived
    /// ones too (PLAT-067).
    pub async fn list(&self) -> Vec<Project> {
        self.nodus.inner.store.read(|state| state.projects.clone())
    }

    /// `projects:add`: adds `repo_path` as a project, or, without one, the
    /// folder the user picks in the native picker; none when they cancel
    /// (PLAT-068). A folder that's already a project gives that project back
    /// (PLAT-071).
    pub async fn add(&self, repo_path: Option<String>) -> CommandResult<Option<Project>> {
        let target = match repo_path.filter(|p| !p.is_empty()) {
            Some(path) => path,
            None => match self.nodus.inner.dialogs.pick_folder().await {
                Some(path) => path.to_string_lossy().into_owned(),
                None => return Ok(None),
            },
        };
        let (project, _added) = self.add_project(&target, None)?;
        Ok(Some(project))
    }

    /// `addProject(folder, name?)` (PLAT-069…PLAT-073): the board tools'
    /// `create_project` uses it too. Gives the project and whether it's new.
    pub fn add_project(&self, folder: &str, name: Option<&str>) -> CommandResult<(Project, bool)> {
        const CHANNEL: &str = "projects:add";
        let real = realpath(Path::new(folder))
            .map_err(|_| CommandError::new(CHANNEL, format!("Folder not found: {folder}")))?;
        let is_dir = std::fs::metadata(&real).map(|m| m.is_dir()).on(CHANNEL)?;
        let real = real.to_string_lossy().into_owned();
        if !is_dir {
            return Err(CommandError::new(CHANNEL, format!("Not a folder: {real}")));
        }
        let mut basename = Path::new(&real)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if basename.is_empty() {
            // path.basename('/') is ''.
            basename = String::new();
        }
        let now = now_ms();
        let picked = random_index();
        Ok(self.nodus.inner.store.update(|state| {
            // UI-300: the same test as the renderer's focus.ts.
            let focusing = is_focusing(
                &state.projects,
                state.settings.focus_feature && state.settings.focus_mode,
            );
            if let Some(existing) = state.projects.iter_mut().find(|p| real_repo_path(&p.repo_path) == real) {
                if existing.archived == Some(true) || (focusing && existing.focused != Some(true)) {
                    existing.archived = None;
                    if focusing {
                        existing.focused = Some(true);
                    }
                }
                return (existing.clone(), false);
            }
            let project = Project {
                id: uuid::Uuid::new_v4().to_string(),
                name: name
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map_or(basename, str::to_owned),
                repo_path: real,
                color: pick_folder_color(&state.projects, picked),
                created_at: now,
                opened_at: None,
                group_id: None,
                actions: None,
                focused: focusing.then_some(true),
                archived: None,
                extra: Map::new(),
            };
            state.projects.push(project.clone());
            (project, true)
        }))
    }

    /// `projects:update` (PLAT-074): renames, recolours, archives, focuses or
    /// files a project under a group.
    pub async fn update(&self, id: &str, patch: ProjectPatch) -> CommandResult<Project> {
        const CHANNEL: &str = "projects:update";
        self.nodus.inner.store.update(|state| {
            let group = match &patch.group_id {
                Some(Some(group_id)) if !group_id.is_empty() => Some(state.group(group_id).map(|g| g.id.clone())),
                _ => None,
            };
            let project = state.project_mut(id).on(CHANNEL)?;
            if let Some(name) = patch.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                project.name = name.to_owned();
            }
            if let Some(color) = patch.color {
                project.color = color;
            }
            match patch.archived {
                Some(true) => project.archived = Some(true),
                Some(false) => project.archived = None,
                None => {}
            }
            match patch.focused {
                Some(true) => project.focused = Some(true),
                Some(false) => project.focused = None,
                None => {}
            }
            // The TypeScript applies the changes above before it finds the
            // group missing, and keeps them.
            match (&patch.group_id, group) {
                (Some(None), _) => project.group_id = None,
                (_, Some(found)) => project.group_id = Some(found.on(CHANNEL)?),
                _ => {}
            }
            Ok(project.clone())
        })
    }

    /// `projects:remove` (PLAT-075): its processes are stopped, its
    /// transcripts no longer watched and its views' folders deleted, then the
    /// project and its board leave state.json. Its groups, captures and
    /// transcripts on disk stay.
    pub async fn remove(&self, id: &str) {
        self.nodus.project_removing(id);
        self.nodus.inner.store.update(|state| {
            state.projects.retain(|p| p.id != id);
            state.boards.shift_remove(id);
        });
    }

    /// `projects:reveal` (PLAT-076): shows the project's folder, selected, in
    /// Finder or File Explorer.
    pub async fn reveal_in_finder(&self, id: &str) -> CommandResult<()> {
        const CHANNEL: &str = "projects:reveal";
        let path = self
            .nodus
            .inner
            .store
            .read(|state| state.project(id).map(|p| p.repo_path.clone()))
            .on(CHANNEL)?;
        nodus_platform::shell::show_item_in_folder(Path::new(&path));
        Ok(())
    }

    /// `projects:stats` (PLAT-077): each project's sessions and what's
    /// running in it, by project id (03 `projectStats`). Until the core
    /// connects the session host nothing reads as running.
    pub async fn stats(&self) -> IndexMap<String, ProjectStats> {
        self.nodus.inner.board.all_project_stats()
    }
}

/// `realRepoPath`: the stored path's real path, or the path as stored when
/// it can't be resolved (PLAT-071).
fn real_repo_path(stored: &str) -> String {
    realpath(Path::new(stored)).map_or_else(|_| stored.to_owned(), |p| p.to_string_lossy().into_owned())
}

/// `pickFolderColor`: a random colour among those the fewest projects use,
/// archived ones counted, in FOLDER_COLORS order (PLAT-073). `random` picks
/// among the tied colours.
fn pick_folder_color(projects: &[Project], random: u32) -> FolderColor {
    let uses = |color: FolderColor| projects.iter().filter(|p| p.color == color).count();
    let fewest = FolderColor::ALL.iter().map(|&c| uses(c)).min().unwrap_or(0);
    let candidates: Vec<FolderColor> = FolderColor::ALL
        .iter()
        .copied()
        .filter(|&c| uses(c) == fewest)
        .collect();
    candidates[random as usize % candidates.len()]
}

/// A random number, from the same source as the ids.
fn random_index() -> u32 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// `Date.now()`.
pub(crate) fn now_ms() -> f64 {
    // Whole milliseconds fit an f64 exactly for the next quarter of a million years.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::nodus;
    use nodus_domain::{GroupPatch, SettingsPatch};

    #[tokio::test]
    async fn adds_a_folder_once_by_its_real_path() {
        let (dir, nodus) = nodus();
        let repo = dir.path().join("repos/shop");
        std::fs::create_dir_all(&repo).unwrap();
        let added = nodus
            .projects()
            .add(Some(repo.to_string_lossy().into_owned()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(added.name, "shop");
        assert_eq!(added.repo_path, realpath(&repo).unwrap().to_string_lossy());
        assert_eq!(added.focused, None);
        // Again, through a different spelling of the same folder.
        let again = nodus
            .projects()
            .add(Some(format!("{}/../shop/.", repo.display())))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.id, added.id);
        assert_eq!(nodus.projects().list().await.len(), 1);
    }

    #[tokio::test]
    async fn turns_down_missing_folders_and_files_in_0_9_4s_words() {
        let (dir, nodus) = nodus();
        let missing = dir.path().join("nope").to_string_lossy().into_owned();
        let err = nodus.projects().add(Some(missing.clone())).await.unwrap_err();
        assert_eq!(err.message, format!("Folder not found: {missing}"));
        assert_eq!(
            err.ipc_message(),
            format!("Error invoking remote method 'projects:add': Error: Folder not found: {missing}")
        );
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "").unwrap();
        let err = nodus
            .projects()
            .add(Some(file.to_string_lossy().into_owned()))
            .await
            .unwrap_err();
        assert_eq!(
            err.message,
            format!("Not a folder: {}", realpath(&file).unwrap().display())
        );
    }

    #[tokio::test]
    async fn cancelling_the_picker_adds_nothing() {
        let (_dir, nodus) = nodus();
        assert_eq!(nodus.projects().add(None).await.unwrap(), None);
        assert_eq!(nodus.projects().add(Some(String::new())).await.unwrap(), None);
    }

    #[tokio::test]
    async fn brings_an_archived_folder_back_and_into_focus_while_focusing() {
        let (dir, nodus) = nodus();
        for name in ["a", "b"] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        let path = |name: &str| Some(dir.path().join(name).to_string_lossy().into_owned());
        let a = nodus.projects().add(path("a")).await.unwrap().unwrap();
        let b = nodus.projects().add(path("b")).await.unwrap().unwrap();
        nodus
            .projects()
            .update(
                &a.id,
                ProjectPatch {
                    focused: Some(true),
                    ..ProjectPatch::default()
                },
            )
            .await
            .unwrap();
        nodus
            .projects()
            .update(
                &b.id,
                ProjectPatch {
                    archived: Some(true),
                    ..ProjectPatch::default()
                },
            )
            .await
            .unwrap();
        nodus
            .settings()
            .update(SettingsPatch {
                focus_feature: Some(true),
                focus_mode: Some(true),
                ..SettingsPatch::default()
            })
            .await;
        let back = nodus.projects().add(path("b")).await.unwrap().unwrap();
        assert_eq!(back.archived, None);
        assert_eq!(back.focused, Some(true));
        std::fs::create_dir_all(dir.path().join("c")).unwrap();
        let c = nodus.projects().add(path("c")).await.unwrap().unwrap();
        assert_eq!(c.focused, Some(true));
    }

    #[tokio::test]
    async fn updates_in_order_and_keeps_changes_made_before_an_unknown_group() {
        let (dir, nodus) = nodus();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        let a = nodus
            .projects()
            .add(Some(dir.path().join("a").to_string_lossy().into_owned()))
            .await
            .unwrap()
            .unwrap();
        let err = nodus
            .projects()
            .update(
                &a.id,
                ProjectPatch {
                    name: Some("  Shop  ".into()),
                    group_id: Some(Some("g?".into())),
                    ..ProjectPatch::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.message, "Unknown group g?");
        assert_eq!(nodus.projects().list().await[0].name, "Shop");

        let group = nodus.groups().create("Infra").await;
        let filed = nodus
            .projects()
            .update(
                &a.id,
                ProjectPatch {
                    name: Some("   ".into()),
                    group_id: Some(Some(group.id.clone())),
                    archived: Some(false),
                    ..ProjectPatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(filed.name, "Shop");
        assert_eq!(filed.group_id.as_deref(), Some(group.id.as_str()));
        let out = nodus
            .projects()
            .update(
                &a.id,
                ProjectPatch {
                    group_id: Some(None),
                    ..ProjectPatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(out.group_id, None);
        nodus
            .groups()
            .update(
                &group.id,
                GroupPatch {
                    collapsed: Some(true),
                    ..GroupPatch::default()
                },
            )
            .await
            .unwrap();

        let err = nodus
            .projects()
            .update("nope", ProjectPatch::default())
            .await
            .unwrap_err();
        assert_eq!(err.message, "Unknown project nope");
        assert_eq!(err.channel, "projects:update");
    }

    #[tokio::test]
    async fn removes_a_project_and_its_board() {
        let (dir, nodus) = nodus();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        let a = nodus
            .projects()
            .add(Some(dir.path().join("a").to_string_lossy().into_owned()))
            .await
            .unwrap()
            .unwrap();
        nodus.inner.store.update(|s| {
            s.board_mut(&a.id);
        });
        nodus.projects().remove(&a.id).await;
        assert!(nodus.projects().list().await.is_empty());
        assert!(nodus.inner.store.read(|s| s.boards.is_empty()));
        assert!(nodus.projects().stats().await.is_empty());
    }

    #[test]
    fn picks_among_the_least_used_colours() {
        let project = |color| Project {
            id: String::new(),
            name: String::new(),
            repo_path: String::new(),
            color,
            created_at: 0.0,
            opened_at: None,
            group_id: None,
            actions: None,
            focused: None,
            archived: None,
            extra: Map::new(),
        };
        assert_eq!(pick_folder_color(&[], 0), FolderColor::Blue);
        assert_eq!(pick_folder_color(&[], 7), FolderColor::Graphite);
        let used: Vec<Project> = FolderColor::ALL
            .iter()
            .filter(|&&c| c != FolderColor::Green)
            .map(|&c| project(c))
            .collect();
        for random in 0..20 {
            assert_eq!(pick_folder_color(&used, random), FolderColor::Green);
        }
    }
}
