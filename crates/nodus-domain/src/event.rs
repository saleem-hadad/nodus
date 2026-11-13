//! What the app tells whoever is watching, one variant per `api.on.*` event in
//! `src/shared/api.ts` (the `ev:*` IPC channels).
//!
//! Domain crates emit these into an [`EventSink`] they're handed, and
//! `nodus-core` passes them on: to the window, and from a server to the apps
//! connected to it.
//!
//! On a server's WebSocket an event is `{ "ev": "<channel>", "p": {…} }`,
//! named after its channel without `ev:` (SRV-023, SRV-024): the arguments
//! 0.9.4 passed one by one are `p`'s fields (`{ "sessionId", "data" }`), and
//! an event of one value has that value as `p`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ActionRunResult, CaptureContext, CaptureShortcut, ClipboardOffer, MenuCommand, OpenCardRequest, Presence,
    QueuedPrompt, ServerState, ServerUpdateStatus, SessionStatus, SessionUsage, SummarizeProgress, TabAttachment,
    UpdateStatus, ViewEvent,
};

/// One event, named after its channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "ev",
    content = "p",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// `ev:pty-data`: output from a session's terminal.
    PtyData { session_id: String, data: String },
    /// `ev:pty-exit`: a session's process ended.
    PtyExit { session_id: String, exit_code: i32 },
    /// `ev:status`: a session started working, needs the user, or finished.
    Status {
        session_id: String,
        status: SessionStatus,
        live: bool,
    },
    /// `ev:agent-status`: a subagent started working, needs the user, or finished.
    AgentStatus {
        session_id: String,
        agent_id: String,
        status: SessionStatus,
    },
    /// `ev:queue`: the prompts waiting for a session's turn to end changed
    /// (queued, edited, removed, or one handed to Claude).
    Queue {
        session_id: String,
        items: Vec<QueuedPrompt>,
    },
    /// `ev:usage`: a session's model, context, running time or cost changed.
    Usage { session_id: String, usage: SessionUsage },
    /// `ev:board-changed`: transcripts or board state changed on disk.
    BoardChanged { project_id: String },
    /// `ev:projects-changed`: a project was added from outside the window.
    ProjectsChanged,
    /// `ev:summarize-progress`.
    SummarizeProgress(SummarizeProgress),
    /// `ev:run-finished`: an action run finished and left the board.
    RunFinished(ActionRunResult),
    /// `ev:menu`: a menu command for the window to carry out.
    Menu(MenuCommand),
    /// `ev:update-status`.
    UpdateStatus(UpdateStatus),
    /// `ev:view`: new data, saved state or a command's result for a view's page.
    View {
        project_id: String,
        view_id: String,
        event: ViewEvent,
        payload: Value,
    },
    /// `ev:open-card`: the user clicked a notification about a card.
    OpenCard(OpenCardRequest),
    /// `ev:capture-shown`: the capture panel came up, over this.
    CaptureShown(CaptureContext),
    /// `ev:capture-context`: more of what the panel came up over, arriving after it.
    CaptureContext(CaptureContextPatch),
    /// `ev:server-status`: the connection to a server changed (SRV-028). The
    /// app's own: a server never sends it.
    ServerStatus { server_id: String, state: ServerState },
    /// `ev:server-notice`: something a server tells every member, such as
    /// `Saleem signed in on Studio` (SRV-042), shown as a toast. The app's
    /// own: a server sends its notices apart from its events.
    ServerNotice {
        server_id: String,
        kind: String,
        message: String,
    },
    /// `ev:accounts-changed`: a server's pool of Claude accounts changed
    /// (SRV-105): one was added, renamed, removed, taken, limited, freed or
    /// failed, or its usage moved. A server sends it without `serverId`;
    /// the app names the server it came from.
    AccountsChanged {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        server_id: String,
    },
    /// `ev:presence`: everyone else on a server board (SRV-180). A server
    /// sends it to each connection on the board, leaving that connection
    /// out; it never passes through the core's own events there.
    Presence(Presence),
    /// `ev:pty-size`: a server session's terminal is this size now (SRV-144).
    /// It follows whoever typed last, so a viewer's terminal may be given a
    /// size it didn't ask for.
    PtySize { session_id: String, cols: f64, rows: f64 },
    /// `ev:server-update`: how updating a server is going (SRV-200), told
    /// to every member's app whoever pressed Update server. A server sends
    /// it without `serverId`; the app names the server it came from.
    ServerUpdate {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        server_id: String,
        status: ServerUpdateStatus,
    },
}

/// `Partial<CaptureContext>`: the parts of the context that arrived late.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureContextPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<ClipboardOffer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<CaptureShortcut>,
}

/// Where a domain sends its events.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

impl<F: Fn(Event) + Send + Sync> EventSink for F {
    fn emit(&self, event: Event) {
        self(event);
    }
}

/// Keeps every event it's given, for tests.
#[derive(Debug, Default)]
pub struct RecordingSink(std::sync::Mutex<Vec<Event>>);

impl RecordingSink {
    /// The events so far, oldest first, leaving them recorded.
    pub fn events(&self) -> Vec<Event> {
        self.lock().clone()
    }

    /// The events so far, oldest first, and forgets them.
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.lock())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Event>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: Event) {
        self.lock().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closures_and_recorders_take_events() {
        let recorder = RecordingSink::default();
        recorder.emit(Event::ProjectsChanged);
        assert_eq!(recorder.events(), vec![Event::ProjectsChanged]);
        assert_eq!(recorder.take(), vec![Event::ProjectsChanged]);
        assert!(recorder.events().is_empty());

        let seen = std::sync::Mutex::new(0);
        let count = |_: Event| *seen.lock().unwrap() += 1;
        count.emit(Event::ProjectsChanged);
        assert_eq!(*seen.lock().unwrap(), 1);
    }

    /// SRV-023: `{ ev, p }`, named after the channel, with JavaScript's numbers.
    #[test]
    fn goes_over_the_wire_named_after_its_channel() {
        let cases = [
            (
                Event::PtyData {
                    session_id: "s1".into(),
                    data: "hi\r\n".into(),
                },
                r#"{"ev":"pty-data","p":{"sessionId":"s1","data":"hi\r\n"}}"#,
            ),
            (
                Event::PtyExit {
                    session_id: "s1".into(),
                    exit_code: -1,
                },
                r#"{"ev":"pty-exit","p":{"sessionId":"s1","exitCode":-1}}"#,
            ),
            (
                Event::Status {
                    session_id: "s1".into(),
                    status: SessionStatus::Waiting,
                    live: true,
                },
                r#"{"ev":"status","p":{"sessionId":"s1","status":"waiting","live":true}}"#,
            ),
            (
                Event::AgentStatus {
                    session_id: "s1".into(),
                    agent_id: "a1".into(),
                    status: SessionStatus::Done,
                },
                r#"{"ev":"agent-status","p":{"sessionId":"s1","agentId":"a1","status":"done"}}"#,
            ),
            (
                Event::BoardChanged {
                    project_id: "p1".into(),
                },
                r#"{"ev":"board-changed","p":{"projectId":"p1"}}"#,
            ),
            (Event::ProjectsChanged, r#"{"ev":"projects-changed"}"#),
            (Event::Menu(MenuCommand::Settings), r#"{"ev":"menu","p":"settings"}"#),
            (
                Event::AccountsChanged {
                    server_id: String::new(),
                },
                r#"{"ev":"accounts-changed","p":{}}"#,
            ),
            (
                Event::ServerUpdate {
                    server_id: String::new(),
                    status: ServerUpdateStatus::Downloading {
                        version: "1.1.0".into(),
                        percent: 40,
                    },
                },
                r#"{"ev":"server-update","p":{"status":{"state":"downloading","version":"1.1.0","percent":40}}}"#,
            ),
            (
                Event::ServerStatus {
                    server_id: "srv1".into(),
                    state: ServerState::ProtocolMismatch,
                },
                r#"{"ev":"server-status","p":{"serverId":"srv1","state":"protocol-mismatch"}}"#,
            ),
            (
                Event::View {
                    project_id: "p1".into(),
                    view_id: "v1".into(),
                    event: ViewEvent::State,
                    payload: serde_json::json!({ "tab": 2 }),
                },
                r#"{"ev":"view","p":{"projectId":"p1","viewId":"v1","event":"state","payload":{"tab":2}}}"#,
            ),
            (
                Event::CaptureContext(CaptureContextPatch {
                    app: Some("Safari".into()),
                    ..CaptureContextPatch::default()
                }),
                r#"{"ev":"capture-context","p":{"app":"Safari"}}"#,
            ),
            // SRV-180
            (
                Event::Presence(Presence {
                    project: "p1".into(),
                    peers: Vec::new(),
                }),
                r#"{"ev":"presence","p":{"project":"p1","peers":[]}}"#,
            ),
            // SRV-144
            (
                Event::PtySize {
                    session_id: "s1".into(),
                    cols: 120.0,
                    rows: 40.0,
                },
                r#"{"ev":"pty-size","p":{"sessionId":"s1","cols":120,"rows":40}}"#,
            ),
        ];
        for (event, wire) in cases {
            assert_eq!(crate::json::to_string(&event).unwrap(), wire);
            assert_eq!(crate::json::from_str::<Event>(wire).unwrap(), event);
        }

        let usage = Event::Usage {
            session_id: "s1".into(),
            usage: SessionUsage {
                model: Some("Opus 5.5".into()),
                context_percent: None,
                context_tokens: None,
                context_size: None,
                cost_usd: Some(0.25),
                duration_ms: None,
                at: 1_700_000_000_000.0,
            },
        };
        let wire = crate::json::to_string(&usage).unwrap();
        assert_eq!(
            wire,
            r#"{"ev":"usage","p":{"sessionId":"s1","usage":{"model":"Opus 5.5","costUsd":0.25,"at":1700000000000}}}"#
        );
        assert_eq!(crate::json::from_str::<Event>(&wire).unwrap(), usage);
    }
}
