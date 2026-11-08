//! `window.api.groups`: named sets of projects, each a section of the grid.

use nodus_domain::{GroupPatch, ProjectGroup};
use serde_json::Map;

use crate::error::OnChannel;
use crate::projects::now_ms;
use crate::{CommandResult, Nodus};

/// `window.api.groups`.
pub struct GroupsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl GroupsApi<'_> {
    /// `groups:list`: in the order they were created (PLAT-079).
    pub async fn list(&self) -> Vec<ProjectGroup> {
        self.nodus.inner.store.read(|state| state.groups.clone())
    }

    /// `groups:create`: a new group named `name`, trimmed, or `New Group`
    /// (PLAT-080).
    pub async fn create(&self, name: &str) -> ProjectGroup {
        let name = name.trim();
        let group = ProjectGroup {
            id: uuid::Uuid::new_v4().to_string(),
            name: if name.is_empty() {
                "New Group".into()
            } else {
                name.into()
            },
            created_at: now_ms(),
            collapsed: None,
            extra: Map::new(),
        };
        self.nodus.inner.store.update(|state| state.groups.push(group.clone()));
        group
    }

    /// `groups:update`: renames a group, or folds it away on the grid (PLAT-081).
    pub async fn update(&self, id: &str, patch: GroupPatch) -> CommandResult<ProjectGroup> {
        self.nodus.inner.store.update(|state| {
            let group = state.group_mut(id).on("groups:update")?;
            if let Some(name) = patch.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                group.name = name.to_owned();
            }
            match patch.collapsed {
                Some(true) => group.collapsed = Some(true),
                Some(false) => group.collapsed = None,
                None => {}
            }
            Ok(group.clone())
        })
    }

    /// `groups:remove`: deletes a group; its projects become ungrouped (PLAT-082).
    pub async fn remove(&self, id: &str) {
        self.nodus.inner.store.update(|state| {
            state.groups.retain(|g| g.id != id);
            for project in &mut state.projects {
                if project.group_id.as_deref() == Some(id) {
                    project.group_id = None;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::nodus;
    use nodus_domain::{GroupPatch, ProjectPatch};

    #[tokio::test]
    async fn creates_renames_collapses_and_removes() {
        let (dir, nodus) = nodus();
        let blank = nodus.groups().create("   ").await;
        assert_eq!(blank.name, "New Group");
        let infra = nodus.groups().create(" Infra ").await;
        assert_eq!(infra.name, "Infra");
        let renamed = nodus
            .groups()
            .update(
                &infra.id,
                GroupPatch {
                    name: Some(" Ops ".into()),
                    collapsed: Some(true),
                },
            )
            .await
            .unwrap();
        assert_eq!((renamed.name.as_str(), renamed.collapsed), ("Ops", Some(true)));
        let open = nodus
            .groups()
            .update(
                &infra.id,
                GroupPatch {
                    collapsed: Some(false),
                    ..GroupPatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(open.collapsed, None);
        assert_eq!(
            nodus
                .groups()
                .update("nope", GroupPatch::default())
                .await
                .unwrap_err()
                .message,
            "Unknown group nope"
        );

        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        let a = nodus
            .projects()
            .add(Some(dir.path().join("a").to_string_lossy().into_owned()))
            .await
            .unwrap()
            .unwrap();
        nodus
            .projects()
            .update(
                &a.id,
                ProjectPatch {
                    group_id: Some(Some(infra.id.clone())),
                    ..ProjectPatch::default()
                },
            )
            .await
            .unwrap();
        nodus.groups().remove(&infra.id).await;
        assert_eq!(nodus.groups().list().await.len(), 1);
        assert_eq!(nodus.projects().list().await[0].group_id, None);
    }
}
