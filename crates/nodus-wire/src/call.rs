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
    BoardLink, BoardNote, BoardSection, BoardShape, CreateSessionOptions, CreateViewOptions, GitFileChange, GroupPatch,
    InkStroke, NodePatch, ProjectPatch, Rect, ViewPatch, ViewReport, Viewport,
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
    /// `string`: the name Claude gave the section.
    BoardNameSection = "board:name-section" { project_id: String, session_ids: Vec<String> },
    /// `null`.
    BoardSaveNote = "board:save-note" { project_id: String, note: BoardNote },
    /// `null`.
    BoardRemoveNotes = "board:remove-notes" { project_id: String, note_ids: Vec<String> },
    /// `null`.
    BoardSaveInk = "board:save-ink" { project_id: String, strokes: Vec<InkStroke> },
    /// `null`.
    BoardSaveShapes = "board:save-shapes" { project_id: String, shapes: Vec<BoardShape> },
    /// `null`.
    BoardRemoveShapes = "board:remove-shapes" { project_id: String, ids: Vec<String> },
    /// `null`.
    BoardSaveLinks = "board:save-links" { project_id: String, links: Vec<BoardLink> },
    /// `null`.
    BoardRemoveLinks = "board:remove-links" { project_id: String, ids: Vec<String> },

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

    /// `GitStatus | null`.
    GitStatus = "git:status" { project_id: String },
    /// `GitFileDiff`.
    GitDiff = "git:diff" { project_id: String, file: GitFileChange },
    /// `GitCommit[]`.
    GitLog = "git:log" {
        project_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<f64>,
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
