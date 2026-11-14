//! The calls a server answers (SRV-024, SRV-025): everything about its
//! projects, which is projects, groups, board, sessions, views, git and
//! actions, plus what a connection asks of the server itself (`watch`,
//! `session:close`).
//!
//! Each is named after the core's IPC channel, the one `CommandError`
//! carries, and takes its arguments as an object named as `api.ts` names
//! them (`{ "projectId", "sessionId" }`), in the serde forms the domain types
//! already have. What stays on the app (settings, zoom, capture, voice,
//! updates, menus, Report a Problem, revealing a folder) isn't here: the
//! server answers it with [`NOT_AVAILABLE`].

use nodus_domain::{
    BoardGroup, BoardLink, BoardNote, BoardSection, BoardShape, CreateSessionOptions, CreateViewOptions, FolderColor,
    GitFileChange, GroupPatch, InkStroke, NodePatch, ProjectPatch, Rect, ViewPatch, ViewReport, Viewport,
};
use serde_json::{Map, Value};

/// What a server answers a call it doesn't take (SRV-025).
pub const NOT_AVAILABLE: &str = "Not available on a server";

/// Declares the calls once: [`Request`], its [`Request::name`]s and [`CALLS`].
macro_rules! calls {
    (
        $(
            $(#[$meta:meta])*
            $variant:ident = $name:literal {
                $( $(#[$field_meta:meta])* $field:ident : $ty:ty ),* $(,)?
            }
        ),+ $(,)?
    ) => {
        /// One call, as `{ "call": <name>, "args": { … } }`. Each says what
        /// its answer's `ok` holds.
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(tag = "call", content = "args", rename_all_fields = "camelCase")]
        pub enum Request {
            $(
                $(#[$meta])*
                #[serde(rename = $name)]
                $variant { $( $(#[$field_meta])* $field: $ty ),* },
            )+
        }

        /// Every call a server answers, by name.
        pub const CALLS: &[&str] = &[$($name),+];

        impl Request {
            /// The call's name: its channel.
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $name,)+
                }
            }
        }
    };
}

calls! {
    // ---- projects (PLAT-067…077)

    /// `Project[]`, in the order they were added, archived ones too.
    ProjectsList = "projects:list" {},
    /// `Project`: a folder on the server added as a project, or the project
    /// it already is. There's no folder picker on a server, so the path is
    /// needed. Slice 4 adds projects from the GitHub App's repositories
    /// instead (SRV-120).
    ProjectsAdd = "projects:add" {
        repo_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// `Project`: a repository one of the App's installations reaches,
    /// cloned on the server and added (SRV-120). The name and colour
    /// default from the repository. Answered once it's cloned; a
    /// repository already added is given back as it is.
    ProjectsAddGithub = "projects:add-github" {
        full_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<FolderColor>,
    },
    /// `Project`.
    ProjectsUpdate = "projects:update" { id: String, patch: ProjectPatch },
    /// `null`.
    ProjectsRemove = "projects:remove" { id: String },
    /// `{ [projectId]: ProjectStats }`.
    ProjectsStats = "projects:stats" {},

    // ---- groups (PLAT-079…082)

    /// `ProjectGroup[]`.
    GroupsList = "groups:list" {},
    /// `ProjectGroup`.
    GroupsCreate = "groups:create" { name: String },
    /// `ProjectGroup`.
    GroupsUpdate = "groups:update" { id: String, patch: GroupPatch },
    /// `null`.
    GroupsRemove = "groups:remove" { id: String },

    // ---- board (SESS-071…108)

    /// `BoardSnapshot`. The board's transcript folder is watched from now on.
    BoardLoad = "board:load" { project_id: String },
    /// `null`.
    BoardSaveNodes = "board:save-nodes" { project_id: String, patches: Vec<NodePatch> },
    /// `null`.
    BoardSaveArchive = "board:save-archive" { project_id: String, rect: Rect },
    /// `null`.
    BoardSaveViewport = "board:save-viewport" { project_id: String, viewport: Viewport },
    /// `null`, once the card's process has stopped.
    BoardRemoveNode = "board:remove-node" { project_id: String, session_id: String },
    /// `null`.
    BoardSaveSection = "board:save-section" { project_id: String, section: BoardSection },
    /// `null`.
    BoardRemoveSection = "board:remove-section" { project_id: String, section_id: String },
    /// `string`: the name Claude gave the frame. `session_ids` holds the
    /// notes', views' and shapes' ids in it too (FRAME-031).
    BoardNameSection = "board:name-section" { project_id: String, session_ids: Vec<String> },
    /// `null`.
    BoardSaveNote = "board:save-note" { project_id: String, note: BoardNote },
    /// `null`.
    BoardRemoveNotes = "board:remove-notes" { project_id: String, note_ids: Vec<String> },
    /// `null`. The whole list: local boards save ink this way. A server
    /// board's app sends single strokes instead (SRV-184).
    BoardSaveInk = "board:save-ink" { project_id: String, strokes: Vec<InkStroke> },
    /// `null`: strokes added, or changed by id, on top of the board's ink as
    /// it is, so two people drawing at once keep both (SRV-184).
    BoardAddInk = "board:add-ink" { project_id: String, strokes: Vec<InkStroke> },
    /// `null`: these strokes, by id, gone from the board's ink (SRV-184).
    BoardRemoveInk = "board:remove-ink" { project_id: String, ids: Vec<String> },
    /// `null`.
    BoardSaveShapes = "board:save-shapes" { project_id: String, shapes: Vec<BoardShape> },
    /// `null`.
    BoardRemoveShapes = "board:remove-shapes" { project_id: String, ids: Vec<String> },
    /// `null`.
    BoardSaveLinks = "board:save-links" { project_id: String, links: Vec<BoardLink> },
    /// `null`.
    BoardRemoveLinks = "board:remove-links" { project_id: String, ids: Vec<String> },
    /// `null` (GROUP-030).
    BoardSaveGroups = "board:save-groups" { project_id: String, groups: Vec<BoardGroup> },
    /// `null` (GROUP-031).
    BoardRemoveGroups = "board:remove-groups" { project_id: String, ids: Vec<String> },

    // ---- sessions (SESS-114…175)

    /// `SessionCard`.
    SessionCreate = "session:create" { project_id: String, opts: CreateSessionOptions },
    /// `OpenSessionResult`. The session's `pty-data` comes to this
    /// connection from now on, after what `replay` holds, until
    /// `session:close` (SRV-026).
    SessionOpen = "session:open" { project_id: String, session_id: String, cols: f64, rows: f64 },
    /// `null`: no more of the session's `pty-data` to this connection (SRV-026).
    SessionClose = "session:close" { session_id: String },
    /// `null`: keys to the session's terminal, in the order they were sent.
    SessionWrite = "session:write" { session_id: String, data: String },
    /// `null`.
    SessionSend = "session:send" { project_id: String, session_id: String, text: String },
    /// `null`.
    SessionResize = "session:resize" { session_id: String, cols: f64, rows: f64 },
    /// `null`, once the process has exited or after 4 s.
    SessionKill = "session:kill" { session_id: String },
    /// `null`, once every one has exited or timed out (SESS-153).
    SessionStop = "session:stop" { session_ids: Vec<String> },
    /// `TranscriptMessage[]`.
    SessionTranscript = "session:transcript" { project_id: String, session_id: String },
    /// `SummaryResult[]`; progress comes as `summarize-progress` events.
    SessionSummarize = "session:summarize" {
        project_id: String,
        session_ids: Vec<String>,
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        focus: Option<String>,
    },
    /// `SessionSearchHit[]`.
    SessionSearch = "session:search" { project_id: String, query: String },
    /// `AgentInfo[]`.
    SessionAgents = "session:agents" { project_id: String, session_id: String },
    /// `TranscriptMessage[]`.
    SessionAgentTranscript = "session:agent-transcript" { project_id: String, session_id: String, agent_id: String },

    // ---- views (VIEW-060…120)

    /// `BoardView`.
    ViewsCreate = "views:create" { project_id: String, opts: CreateViewOptions },
    /// `null`.
    ViewsSave = "views:save" { project_id: String, patch: ViewPatch },
    /// `null`.
    ViewsRemove = "views:remove" { project_id: String, view_id: String },
    /// Whatever the view's method answers.
    ViewsCall = "views:call" {
        project_id: String,
        view_id: String,
        method: String,
        #[serde(default)]
        args: Value,
    },
    /// `null`.
    ViewsReport = "views:report" { project_id: String, view_id: String, report: ViewReport },
    /// `null`.
    ViewsApprove = "views:approve" { project_id: String, view_id: String, name: String, allow: bool },
    /// `CommandResult[]`.
    ViewsRefresh = "views:refresh" { project_id: String, view_id: String },
    /// `null`.
    ViewsAsk = "views:ask" { project_id: String, view_id: String, prompt: String },
    /// `string`: the brain's session id.
    ViewsBrain = "views:brain" { project_id: String, view_id: String },

    // ---- git (GIT-003…021)

    /// `GitStatus | null`. With `sessionId`, the card's worktree's, when
    /// it has one; else the main checkout's (SRV-128).
    GitStatus = "git:status" {
        project_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// `GitFileDiff`, in the card's worktree as `git:status`.
    GitDiff = "git:diff" {
        project_id: String,
        file: GitFileChange,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// `GitCommit[]`, in the card's worktree as `git:status`.
    GitLog = "git:log" {
        project_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },

    // ---- actions (ACT-001…027)

    /// `ProjectAction[]`.
    ActionsGlobal = "actions:global" {},
    /// `ActionLists`. `action` is checked and trimmed as 0.9.4 does.
    ActionsSave = "actions:save" {
        project_id: String,
        #[serde(default)]
        action: Value,
    },
    /// `ActionLists`.
    ActionsRemove = "actions:remove" { project_id: String, action_id: String },
    /// `ActionDraft`.
    ActionsDraft = "actions:draft" { project_id: String, description: String, global: bool },

    // ---- the connection

    /// `null`: this connection gets these projects' events from now on,
    /// and no others' (SRV-026). Each `watch` replaces the last.
    Watch = "watch" { projects: Vec<String> },

    // ---- the server itself (SRV-040…063, SRV-210). Every member may make
    // every one of these: there are no roles (SRV-040).

    /// `SetupStatus`. With a setup token too.
    SetupStatus = "setup:status" {},
    /// `SetupStatus`: names the server, which is step 1. With a setup token too.
    SetupName = "setup:name" { name: String },
    /// `SetupStatus`: a step after step 2 is finished or skipped:
    /// `members`, `accounts` or `project`.
    SetupDone = "setup:done" { step: crate::SetupStep },
    /// `Member[]`, in the order they were added.
    MembersList = "members:list" {},
    /// `Member`: the GitHub login resolved to its id and added (SRV-044).
    MembersInvite = "members:invite" { login: String },
    /// `null`: the member and every device of theirs, whose sockets close.
    /// The last member can't remove themselves. With `removeAccounts`, the
    /// Claude accounts they added go too (SRV-040).
    MembersRemove = "members:remove" {
        github_id: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        remove_accounts: bool,
    },
    /// `Device[]`: every member's, oldest first.
    DevicesList = "devices:list" {},
    /// `null`: its socket closes at once (SRV-042).
    DevicesRevoke = "devices:revoke" { id: String },
    /// `AuditLine[]`: the last 200 lines of the audit log, oldest first (SRV-210).
    AuditList = "audit:list" {},
    /// `ServerInfo`.
    ServerInfo = "server:info" {},
    /// `ServerUpdateStatus`, `latest` or `restarting`: the server updates
    /// itself to the newest release, never an older one, and exits once
    /// it has answered, for systemd to start the new version (SRV-200).
    /// Its progress goes to every member as `server-update` events. The one
    /// call an app makes whatever protocol the server speaks (SRV-027), on
    /// a connection of its own, so it has to stay as it is.
    ServerUpdate = "server:update" {},
    /// `Installation[]`: the App's installations, as GitHub says now. With
    /// a setup token too, since step 2 waits for one.
    GithubInstallations = "github:installations" {},
    /// `GithubApp`: a new private key for the App, checked against GitHub
    /// before it replaces the old one (SRV-063).
    GithubReplaceKey = "github:replace-key" { pem: String },
    /// `GithubRepo[]`: every repository the App's installations reach,
    /// which projects can be added from (SRV-120).
    GithubRepos = "github:repos" {},

    // ---- the pool of Claude accounts (SRV-100…105). No answer ever holds
    // a token.

    /// `ClaudeAccount[]`, in the order they were added.
    AccountsList = "accounts:list" {},
    /// `ClaudeAccount`: a token from `claude setup-token`, taken once one
    /// `claude -p` run with it has worked (SRV-100).
    AccountsAdd = "accounts:add" { label: String, token: String },
    /// `ClaudeAccount`.
    AccountsRename = "accounts:rename" { id: String, label: String },
    /// `null`: the sessions running on it move to another (SRV-105).
    AccountsRemove = "accounts:remove" { id: String },
}

/// The calls a setup token may make: wizard steps 1 and 2 (SRV-060). The
/// rest need a device token.
pub const SETUP_CALLS: &[&str] = &["setup:status", "setup:name", "github:installations"];

impl Request {
    /// Whether the server answers it itself, rather than its core.
    pub fn is_server_call(&self) -> bool {
        matches!(
            self,
            Self::SetupStatus {}
                | Self::SetupName { .. }
                | Self::SetupDone { .. }
                | Self::MembersList {}
                | Self::MembersInvite { .. }
                | Self::MembersRemove { .. }
                | Self::DevicesList {}
                | Self::DevicesRevoke { .. }
                | Self::AuditList {}
                | Self::ServerInfo {}
                | Self::ServerUpdate {}
                | Self::GithubInstallations {}
                | Self::GithubReplaceKey { .. }
                | Self::GithubRepos {}
                | Self::ProjectsAddGithub { .. }
                | Self::AccountsList {}
                | Self::AccountsAdd { .. }
                | Self::AccountsRename { .. }
                | Self::AccountsRemove { .. }
        )
    }
}

impl Request {
    /// The request a call frame makes, given a name in [`CALLS`]. Missing
    /// or `null` arguments are `{}`. Err with serde's reason when the
    /// arguments don't fit.
    pub fn from_call(call: &str, args: Value) -> Result<Self, String> {
        let args = if args.is_null() {
            Value::Object(Map::new())
        } else {
            args
        };
        let mut frame = Map::new();
        frame.insert("call".into(), Value::String(call.to_owned()));
        frame.insert("args".into(), args);
        serde_json::from_value(Value::Object(frame)).map_err(|err| err.to_string())
    }

    /// Its arguments, as a call frame carries them.
    pub fn args(&self) -> Value {
        match serde_json::to_value(self) {
            Ok(Value::Object(mut frame)) => frame.shift_remove("args").unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_each_call_once_by_its_channel() {
        let mut names = CALLS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CALLS.len());
        for name in ["board:load", "session:create", "views:call", "git:status", "watch"] {
            assert!(CALLS.contains(&name), "{name}");
        }
        // What the app keeps to itself (SRV-025).
        for name in [
            "settings:get",
            "capture:submit",
            "voice:transcribe",
            "projects:reveal",
            "updates:check",
        ] {
            assert!(!CALLS.contains(&name), "{name}");
        }
    }

    #[test]
    fn a_setup_token_makes_only_the_wizards_first_calls() {
        for name in SETUP_CALLS {
            assert!(CALLS.contains(name), "{name}");
        }
        assert!(!SETUP_CALLS.contains(&"members:invite"));
        let remove = Request::from_call("members:remove", json!({ "githubId": 583231 })).unwrap();
        assert_eq!(
            remove,
            Request::MembersRemove {
                github_id: 583231,
                remove_accounts: false
            }
        );
        assert_eq!(
            Request::from_call("members:remove", json!({ "githubId": 1, "removeAccounts": true })).unwrap(),
            Request::MembersRemove {
                github_id: 1,
                remove_accounts: true
            }
        );
        assert!(Request::AccountsList {}.is_server_call());
        assert!(remove.is_server_call());
        assert!(!Request::ProjectsList {}.is_server_call());
        assert_eq!(
            Request::from_call("setup:done", json!({ "step": "members" })).unwrap(),
            Request::SetupDone {
                step: crate::SetupStep::Members
            }
        );
    }

    #[test]
    fn takes_its_arguments_by_name() {
        let open = Request::from_call(
            "session:open",
            json!({ "projectId": "p1", "sessionId": "s1", "cols": 100, "rows": 30.5 }),
        )
        .unwrap();
        assert_eq!(
            open,
            Request::SessionOpen {
                project_id: "p1".into(),
                session_id: "s1".into(),
                cols: 100.0,
                rows: 30.5,
            }
        );
        assert_eq!(open.name(), "session:open");
        assert_eq!(
            nodus_domain::json::to_string(&open.args()).unwrap(),
            r#"{"projectId":"p1","sessionId":"s1","cols":100,"rows":30.5}"#
        );
    }

    #[test]
    fn reads_no_arguments_as_none() {
        for args in [Value::Null, json!({})] {
            assert_eq!(
                Request::from_call("projects:list", args).unwrap(),
                Request::ProjectsList {}
            );
        }
        assert_eq!(
            Request::from_call("git:log", json!({ "projectId": "p1" })).unwrap(),
            Request::GitLog {
                project_id: "p1".into(),
                limit: None,
                session_id: None,
            }
        );
        assert_eq!(
            Request::from_call(
                "views:call",
                json!({ "projectId": "p", "viewId": "v", "method": "sessions" })
            )
            .unwrap(),
            Request::ViewsCall {
                project_id: "p".into(),
                view_id: "v".into(),
                method: "sessions".into(),
                args: Value::Null,
            }
        );
    }

    #[test]
    fn says_why_arguments_dont_fit() {
        let err = Request::from_call("board:load", json!({ "project": "p1" })).unwrap_err();
        assert!(err.contains("projectId"), "{err}");
        let err = Request::from_call("session:write", json!({ "sessionId": 7, "data": "x" })).unwrap_err();
        assert!(err.contains("invalid type"), "{err}");
    }

    #[test]
    fn keeps_a_patchs_null_apart_from_its_absence() {
        let ungroup =
            Request::from_call("projects:update", json!({ "id": "p1", "patch": { "groupId": null } })).unwrap();
        let Request::ProjectsUpdate { patch, .. } = &ungroup else {
            unreachable!()
        };
        assert_eq!(patch.group_id, Some(None));
        assert_eq!(
            nodus_domain::json::to_string(&ungroup.args()).unwrap(),
            r#"{"id":"p1","patch":{"groupId":null}}"#
        );
    }
}
