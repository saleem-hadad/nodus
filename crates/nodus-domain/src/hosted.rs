//! What a server board keeps that a local one never has: who made each
//! thing on it (SRV-183), and a server project's repository and its cards'
//! worktrees, setup and preview ports (SRV-120…127).
//!
//! These live with the fields this version doesn't know (`extra`), read and
//! written through the accessors here, so local state.json never gains them
//! and the types every other crate builds stay as they are.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::board::{BoardLink, BoardNode, BoardNote, BoardSection, BoardShape, InkStroke};
use crate::project::Project;
use crate::view::BoardView;

/// The key [`CreatedBy`] keeps a member's login under.
pub const CREATED_BY: &str = "createdBy";

/// Something on a board that records who made it (SRV-183): a member's
/// GitHub login, made on a server board by them, or by a session they
/// started. Left out everywhere else.
pub trait CreatedBy {
    fn extra(&self) -> &Map<String, Value>;
    fn extra_mut(&mut self) -> &mut Map<String, Value>;

    /// The login of the member who made it, when a member did.
    fn created_by(&self) -> Option<&str> {
        self.extra()
            .get(CREATED_BY)
            .and_then(Value::as_str)
            .filter(|login| !login.is_empty())
    }

    /// Records who made it; none takes it away.
    fn set_created_by(&mut self, login: Option<&str>) {
        set(self.extra_mut(), CREATED_BY, login.map(Value::from));
    }
}

macro_rules! created_by {
    ($($ty:ty),+) => {
        $(impl CreatedBy for $ty {
            fn extra(&self) -> &Map<String, Value> {
                &self.extra
            }
            fn extra_mut(&mut self) -> &mut Map<String, Value> {
                &mut self.extra
            }
        })+
    };
}

created_by!(
    BoardNode,
    BoardNote,
    BoardSection,
    BoardShape,
    BoardLink,
    InkStroke,
    BoardView
);

/// Sets or takes away one key, keeping the others' order (`shift_remove`).
fn set(extra: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(value) => {
            extra.insert(key.to_owned(), value);
        }
        None => {
            extra.shift_remove(key);
        }
    }
}

fn read<T: for<'de> Deserialize<'de>>(extra: &Map<String, Value>, key: &str) -> Option<T> {
    serde_json::from_value(extra.get(key)?.clone()).ok()
}

fn write<T: Serialize>(extra: &mut Map<String, Value>, key: &str, value: Option<&T>) {
    set(extra, key, value.and_then(|value| serde_json::to_value(value).ok()));
}

/// A server project's repository on GitHub, through the server's App
/// (SRV-120): `github` on the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGithub {
    /// `owner/name`.
    pub full_name: String,
    /// The branch new cards' branches are cut from.
    pub default_branch: String,
    /// The App's installation that reaches it.
    pub installation_id: u64,
}

impl Project {
    /// The GitHub repository a server project was added from (SRV-120).
    /// Its cards each get a worktree (SRV-122).
    pub fn github(&self) -> Option<ProjectGithub> {
        read(&self.extra, "github")
    }

    pub fn set_github(&mut self, github: Option<&ProjectGithub>) {
        write(&mut self.extra, "github", github);
    }
}

/// A card's own checkout of its project (SRV-122): `worktree` on the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    /// `<run home>/projects/<slug>/worktrees/<first 8 of the card's id>`.
    pub path: String,
    /// `nodus/<title slug>-<first 4 of the card's id>`.
    pub branch: String,
}

string_enum! {
    /// How a worktree's `.nodus/setup` is going (SRV-123).
    pub enum SetupState {
        Running = "running",
        Failed = "failed",
    }
}

/// A card's `.nodus/setup` while it runs, or once it failed (SRV-123):
/// `setup` on the card. Gone once it has succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSetup {
    pub state: SetupState,
    /// The script's last line of output so far.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub line: String,
}

impl BoardNode {
    /// The card's worktree, on a server project from GitHub (SRV-122).
    pub fn worktree(&self) -> Option<Worktree> {
        read(&self.extra, "worktree")
    }

    pub fn set_worktree(&mut self, worktree: Option<&Worktree>) {
        write(&mut self.extra, "worktree", worktree);
    }

    /// Its worktree's `.nodus/setup`, while it runs or once it failed (SRV-123).
    pub fn setup(&self) -> Option<WorktreeSetup> {
        read(&self.extra, "setup")
    }

    pub fn set_setup(&mut self, setup: Option<&WorktreeSetup>) {
        write(&mut self.extra, "setup", setup);
    }

    /// The card's preview port, 8100–8999, given as `PORT` and
    /// `NODUS_PREVIEW_PORT` (SRV-127).
    pub fn preview_port(&self) -> Option<u16> {
        read(&self.extra, "previewPort")
    }

    pub fn set_preview_port(&mut self, port: Option<u16>) {
        write(&mut self.extra, "previewPort", port.as_ref());
    }

    /// Whether something listens on its preview port, as the server last
    /// looked (SRV-127): the card's header shows **Open preview** then.
    pub fn preview_live(&self) -> bool {
        self.extra.get("previewLive").and_then(Value::as_bool) == Some(true)
    }

    pub fn set_preview_live(&mut self, live: bool) {
        set(&mut self.extra, "previewLive", live.then_some(Value::Bool(true)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node() -> BoardNode {
        serde_json::from_value(json!({
            "sessionId": "s1", "x": 0, "y": 0, "archived": false, "createdAt": 1, "model": "opus"
        }))
        .unwrap()
    }

    #[test]
    fn keeps_who_made_it_after_what_it_already_had() {
        let mut node = node();
        assert_eq!(node.created_by(), None);
        node.set_created_by(Some("alice"));
        assert_eq!(node.created_by(), Some("alice"));
        assert_eq!(
            crate::json::to_string(&node).unwrap(),
            r#"{"sessionId":"s1","x":0,"y":0,"archived":false,"createdAt":1,"model":"opus","createdBy":"alice"}"#
        );
        node.set_created_by(None);
        assert_eq!(
            crate::json::to_string(&node).unwrap(),
            r#"{"sessionId":"s1","x":0,"y":0,"archived":false,"createdAt":1,"model":"opus"}"#
        );
    }

    #[test]
    fn reads_and_writes_a_cards_worktree_setup_and_preview() {
        let mut node = node();
        let worktree = Worktree {
            path: "/home/nodus-run/projects/shop/worktrees/0123abcd".into(),
            branch: "nodus/fix-login-0123".into(),
        };
        node.set_worktree(Some(&worktree));
        node.set_setup(Some(&WorktreeSetup {
            state: SetupState::Running,
            line: "npm install".into(),
        }));
        node.set_preview_port(Some(8100));
        node.set_preview_live(true);
        let text = crate::json::to_string(&node).unwrap();
        assert!(
            text.ends_with(
                r#""worktree":{"path":"/home/nodus-run/projects/shop/worktrees/0123abcd","branch":"nodus/fix-login-0123"},"setup":{"state":"running","line":"npm install"},"previewPort":8100,"previewLive":true}"#
            ),
            "{text}"
        );
        let read: BoardNode = serde_json::from_str(&text).unwrap();
        assert_eq!(read.worktree(), Some(worktree));
        assert_eq!(read.preview_port(), Some(8100));
        assert!(read.preview_live());
        node.set_preview_live(false);
        node.set_setup(None);
        assert!(!node.preview_live() && node.setup().is_none());
    }

    #[test]
    fn a_project_knows_its_repository() {
        let mut project: Project = serde_json::from_value(json!({
            "id": "p1", "name": "Shop", "repoPath": "/r", "color": "blue", "createdAt": 1
        }))
        .unwrap();
        assert_eq!(project.github(), None);
        let github = ProjectGithub {
            full_name: "acme/shop".into(),
            default_branch: "main".into(),
            installation_id: 77,
        };
        project.set_github(Some(&github));
        assert_eq!(project.github(), Some(github));
        assert!(
            crate::json::to_string(&project)
                .unwrap()
                .ends_with(r#""github":{"fullName":"acme/shop","defaultBranch":"main","installationId":77}}"#)
        );
    }
}
