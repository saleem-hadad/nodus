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
use crate::session::{MessageRole, TranscriptMessage};
use crate::view::{BoardView, ViewCommand};

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

    /// The id of the Claude account its process last started on, from the
    /// server's pool (SRV-101).
    pub fn account(&self) -> Option<String> {
        read::<String>(&self.extra, "account").filter(|id| !id.is_empty())
    }

    /// That account's label, as the card's menu and hover show it.
    pub fn account_label(&self) -> Option<String> {
        read::<String>(&self.extra, "accountLabel").filter(|label| !label.is_empty())
    }

    /// Records the account (id and label) its process started on; none
    /// takes both away.
    pub fn set_account(&mut self, account: Option<(&str, &str)>) {
        set(&mut self.extra, "account", account.map(|(id, _)| Value::from(id)));
        set(
            &mut self.extra,
            "accountLabel",
            account.map(|(_, label)| Value::from(label)),
        );
    }

    /// What its account did last, for the card to say (SRV-104): `Moved to
    /// <label>`, or that it waits for an account to free up.
    pub fn account_note(&self) -> Option<String> {
        read::<String>(&self.extra, "accountNote").filter(|note| !note.is_empty())
    }

    pub fn set_account_note(&mut self, note: Option<&str>) {
        set(&mut self.extra, "accountNote", note.map(Value::from));
    }
}

/// How many prompts a card keeps the senders of: the latest.
pub const PROMPTS_KEPT: usize = 200;

/// How far apart a prompt's hook and its turn in the transcript may be to
/// be the same prompt.
const SAME_PROMPT_MS: f64 = 10_000.0;

/// A prompt a server session was given, and the member who sent it
/// (SRV-142): whose client last wrote to its terminal before Claude's
/// `UserPromptSubmit` hook, else whoever started the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBy {
    /// When the hook came, in milliseconds since the epoch.
    pub at: u64,
    /// Their GitHub login.
    pub by: String,
}

impl BoardNode {
    /// Who sent the card's prompts, oldest first: `prompts` (SRV-142).
    pub fn prompts(&self) -> Vec<PromptBy> {
        read(&self.extra, "prompts").unwrap_or_default()
    }

    /// Records one more, keeping the last [`PROMPTS_KEPT`].
    pub fn push_prompt(&mut self, prompt: PromptBy) {
        let mut prompts = self.prompts();
        prompts.push(prompt);
        let over = prompts.len().saturating_sub(PROMPTS_KEPT);
        prompts.drain(..over);
        write(&mut self.extra, "prompts", Some(&prompts));
    }
}

/// Puts each user turn's sender on it (SRV-142): the prompt recorded
/// nearest the turn's time, within 10 seconds, each prompt for one turn
/// only. Claude's own notes in the transcript aren't anyone's.
pub fn attribute_turns(messages: &mut [TranscriptMessage], prompts: &[PromptBy]) {
    let mut used = vec![false; prompts.len()];
    for message in messages {
        if message.role != MessageRole::User || message.notice == Some(true) {
            continue;
        }
        let nearest = prompts
            .iter()
            .enumerate()
            .filter(|(i, p)| !used[*i] && (p.at as f64 - message.timestamp).abs() <= SAME_PROMPT_MS)
            .min_by(|(_, a), (_, b)| {
                let off = |p: &PromptBy| (p.at as f64 - message.timestamp).abs();
                off(a).total_cmp(&off(b))
            });
        if let Some((i, prompt)) = nearest {
            used[i] = true;
            message.by = Some(prompt.by.clone());
        }
    }
}

impl ViewCommand {
    /// The member who allowed the command line (SRV-145): `approvedBy`.
    /// Any member may; the server's audit log records it too (SRV-210).
    pub fn approved_by(&self) -> Option<&str> {
        self.extra.get("approvedBy").and_then(Value::as_str)
    }

    pub fn set_approved_by(&mut self, login: Option<&str>) {
        set(&mut self.extra, "approvedBy", login.map(Value::from));
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
        let mut moved = read.clone();
        moved.set_account(Some(("a2", "Team Max")));
        moved.set_account_note(Some("Moved to Team Max"));
        assert!(
            crate::json::to_string(&moved)
                .unwrap()
                .ends_with(r#""account":"a2","accountLabel":"Team Max","accountNote":"Moved to Team Max"}"#)
        );
        assert_eq!(moved.account().as_deref(), Some("a2"));
        assert_eq!(moved.account_label().as_deref(), Some("Team Max"));
        assert_eq!(moved.account_note().as_deref(), Some("Moved to Team Max"));
        moved.set_account(None);
        moved.set_account_note(None);
        assert_eq!(moved, read);
        assert_eq!(read.preview_port(), Some(8100));
        assert!(read.preview_live());
        node.set_preview_live(false);
        node.set_setup(None);
        assert!(!node.preview_live() && node.setup().is_none());
    }

    /// SRV-142
    #[test]
    fn keeps_the_last_prompts_senders() {
        let mut node = node();
        assert!(node.prompts().is_empty());
        for at in 0..(PROMPTS_KEPT as u64 + 5) {
            node.push_prompt(PromptBy { at, by: "alice".into() });
        }
        let prompts = node.prompts();
        assert_eq!(prompts.len(), PROMPTS_KEPT);
        assert_eq!(prompts[0].at, 5);
        let mut short = self::node();
        short.push_prompt(PromptBy {
            at: 1_700_000_000_000,
            by: "octo".into(),
        });
        assert!(
            crate::json::to_string(&short)
                .unwrap()
                .ends_with(r#""prompts":[{"at":1700000000000,"by":"octo"}]}"#)
        );
    }

    /// SRV-142
    #[test]
    fn a_turn_is_sent_by_the_prompt_nearest_it() {
        let turn = |role: MessageRole, timestamp: f64| TranscriptMessage {
            role,
            text: "x".into(),
            timestamp,
            tools: None,
            calls: None,
            notice: None,
            by: None,
        };
        let mut messages = vec![
            turn(MessageRole::User, 100_000.0),
            turn(MessageRole::Assistant, 101_000.0),
            turn(MessageRole::User, 200_000.0),
            turn(MessageRole::User, 300_000.0),
            TranscriptMessage {
                notice: Some(true),
                ..turn(MessageRole::User, 300_500.0)
            },
        ];
        let prompts = [
            PromptBy {
                at: 100_400,
                by: "alice".into(),
            },
            // Nothing near the second turn: a dev token's, say.
            PromptBy {
                at: 299_000,
                by: "octo".into(),
            },
            PromptBy {
                at: 300_200,
                by: "alice".into(),
            },
        ];
        attribute_turns(&mut messages, &prompts);
        let by: Vec<_> = messages.iter().map(|m| m.by.as_deref()).collect();
        assert_eq!(by, [Some("alice"), None, None, Some("alice"), None]);
    }

    /// SRV-145
    #[test]
    fn a_command_keeps_who_allowed_it() {
        let mut cmd: ViewCommand = serde_json::from_value(json!({ "name": "test", "run": "npm test" })).unwrap();
        cmd.set_approved_by(Some("alice"));
        assert_eq!(cmd.approved_by(), Some("alice"));
        assert_eq!(
            crate::json::to_string(&cmd).unwrap(),
            r#"{"name":"test","run":"npm test","approvedBy":"alice"}"#
        );
        cmd.set_approved_by(None);
        assert_eq!(cmd.approved_by(), None);
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
