//! Answering the JSON-RPC requests the session host relays from a session's
//! `claude` process (boardTools.ts's `answerMcp`).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nodus_board::notes::NotePatch;
use nodus_domain::{FolderColor, json};
use serde_json::{Map, Value};

use crate::schema::{BOARD_SERVER, INSTRUCTIONS, PROTOCOL_VERSION, SESSION_TOOL_NAMES, ToolLists};

/// A boxed future, for trait methods.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Why a tool call failed: the message Claude reads (TOOL-024).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError(pub String);

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

impl ToolError {
    /// The error's message, as `err.message` gives it.
    pub fn of(err: impl fmt::Display) -> Self {
        Self(err.to_string())
    }
}

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

/// What the tools do on the board (boardTools.ts's `BoardToolHandlers`).
/// [`crate::StoreHandlers`] implements it on the store and the board; tests
/// give their own. A handler's values come back to Claude as pretty JSON.
pub trait BoardToolHandlers: Send + Sync {
    /// `projectIdForSession`: the project whose board the session is a node on.
    fn project_for_session(&self, session_id: &str) -> Option<String>;
    /// Whether the session is an action run, which gets the session tools too.
    fn is_run(&self, session_id: &str) -> bool;
    /// Every project, saying which one `own` is.
    fn list_projects(&self, own: &str) -> Result<Value, ToolError>;
    /// Adds a folder as a project, making the folder if it isn't there.
    fn create_project(&self, folder: Option<&Value>, name: Option<&Value>) -> Result<Value, ToolError>;
    /// The id of the project `reference` names, by its id or its name.
    fn find_project(&self, reference: Option<&Value>) -> Result<String, ToolError>;
    /// The project's sessions, leaving out the calling run.
    fn list_sessions(&self, project_id: &str, self_id: &str, include_archived: bool) -> Result<Value, ToolError>;
    fn archive_sessions(
        &self,
        project_id: &str,
        self_id: &str,
        session_ids: &[String],
        force: bool,
    ) -> Result<Value, ToolError>;
    /// Archives the calling session once its turn ends.
    fn archive_self(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError>;
    fn list_notes(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError>;
    fn create_note(&self, project_id: &str, self_id: &str, note: &NotePatch) -> Result<Value, ToolError>;
    fn update_note(&self, project_id: &str, note_id: &str, patch: &NotePatch) -> Result<Value, ToolError>;
    fn list_views(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError>;
    /// `view_id` as Claude gave it, None when it left it out.
    fn get_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<Value, ToolError>;
    /// Answers once the board has had a chance to load the page (VIEW-108).
    fn create_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>>;
    fn update_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>>;
    /// What's on the board and where, and the links touching the caller's card.
    fn list_board(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError>;
    /// Draws shapes and links, `args` as Claude gave them.
    fn draw_on_board(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError>;
    /// Changes shapes and links, and moves cards, notes and views.
    fn update_drawing(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError>;
    /// Takes shapes and links off the board.
    fn erase_from_board(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError>;
}

impl<H: BoardToolHandlers + ?Sized> BoardToolHandlers for Arc<H> {
    fn project_for_session(&self, session_id: &str) -> Option<String> {
        (**self).project_for_session(session_id)
    }
    fn is_run(&self, session_id: &str) -> bool {
        (**self).is_run(session_id)
    }
    fn list_projects(&self, own: &str) -> Result<Value, ToolError> {
        (**self).list_projects(own)
    }
    fn create_project(&self, folder: Option<&Value>, name: Option<&Value>) -> Result<Value, ToolError> {
        (**self).create_project(folder, name)
    }
    fn find_project(&self, reference: Option<&Value>) -> Result<String, ToolError> {
        (**self).find_project(reference)
    }
    fn list_sessions(&self, project_id: &str, self_id: &str, include_archived: bool) -> Result<Value, ToolError> {
        (**self).list_sessions(project_id, self_id, include_archived)
    }
    fn archive_sessions(
        &self,
        project_id: &str,
        self_id: &str,
        session_ids: &[String],
        force: bool,
    ) -> Result<Value, ToolError> {
        (**self).archive_sessions(project_id, self_id, session_ids, force)
    }
    fn archive_self(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        (**self).archive_self(project_id, self_id)
    }
    fn list_notes(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        (**self).list_notes(project_id, self_id)
    }
    fn create_note(&self, project_id: &str, self_id: &str, note: &NotePatch) -> Result<Value, ToolError> {
        (**self).create_note(project_id, self_id, note)
    }
    fn update_note(&self, project_id: &str, note_id: &str, patch: &NotePatch) -> Result<Value, ToolError> {
        (**self).update_note(project_id, note_id, patch)
    }
    fn list_views(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        (**self).list_views(project_id, self_id)
    }
    fn get_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<Value, ToolError> {
        (**self).get_view(project_id, self_id, view_id)
    }
    fn create_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        (**self).create_view(project_id, self_id, args)
    }
    fn update_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        (**self).update_view(project_id, self_id, args)
    }
    fn list_board(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        (**self).list_board(project_id, self_id)
    }
    fn draw_on_board(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError> {
        (**self).draw_on_board(project_id, self_id, args)
    }
    fn update_drawing(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError> {
        (**self).update_drawing(project_id, self_id, args)
    }
    fn erase_from_board(&self, project_id: &str, self_id: &str, args: &Value) -> Result<Value, ToolError> {
        (**self).erase_from_board(project_id, self_id, args)
    }
}

/// The board tools' MCP server, as the app runs it behind the session host.
///
/// [`McpServer::answer`] answers one relayed request. It never fails: a tool
/// that fails answers with a tool error, and the host answers for an app
/// that isn't there (TOOL-011, nodus-host).
pub struct McpServer<H> {
    version: String,
    handlers: Option<H>,
    tools: ToolLists,
}

impl<H: BoardToolHandlers> McpServer<H> {
    /// A server that says it's `version` (the app's, TOOL-017), lists
    /// `create_view` with `view_guide` (nodus-views' `VIEW_GUIDE`, VIEW-121),
    /// and calls `handlers`; with none, every tool call fails with `The board
    /// is not available` (TOOL-025).
    pub fn new(version: impl Into<String>, view_guide: &str, handlers: Option<H>) -> Self {
        Self {
            version: version.into(),
            handlers,
            tools: ToolLists::new(view_guide),
        }
    }

    pub fn handlers(&self) -> Option<&H> {
        self.handlers.as_ref()
    }

    /// Answers a JSON-RPC request from a session's process, relayed by the
    /// session host. A tool call answers once it's done, which for a view's
    /// page means once the board has loaded it (TOOL-016 to TOOL-025).
    pub async fn answer(&self, session_id: &str, message: &Value) -> Value {
        let id = message.get("id");
        let method = message.get("method");
        let params = message.get("params");
        match method.and_then(Value::as_str) {
            Some("initialize") => {
                // TOOL-017
                let asked = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
                reply(
                    id,
                    "result",
                    serde_json::json!({
                        "protocolVersion": asked.unwrap_or(PROTOCOL_VERSION),
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": BOARD_SERVER, "version": self.version },
                        "instructions": INSTRUCTIONS
                    }),
                )
            }
            // TOOL-018
            Some("ping") => reply(id, "result", Value::Object(Map::new())),
            Some("tools/list") => {
                // TOOL-019: no pagination cursor.
                let run = self.handlers.as_ref().is_some_and(|h| h.is_run(session_id));
                let tools = if run { &self.tools.run } else { &self.tools.session };
                reply(id, "result", serde_json::json!({ "tools": tools }))
            }
            Some("tools/call") => {
                // `msg.params ?? {}`: a params that isn't an object has no fields.
                let result = self.call_tool(params.unwrap_or(&Value::Null), session_id).await;
                reply(id, "result", result)
            }
            // TOOL-020
            _ => reply(
                id,
                "error",
                serde_json::json!({ "code": -32601, "message": format!("Unknown method {}", js_string(method)) }),
            ),
        }
    }

    /// `callTool`: runs a tool and wraps what it gives, or why it failed, as
    /// a tool result (TOOL-021 to TOOL-025).
    async fn call_tool(&self, params: &Value, session_id: &str) -> Value {
        match self.dispatch(params, session_id).await {
            // TOOL-023: JSON.stringify(value, null, 2), without `isError`.
            Ok(value) => serde_json::json!({
                "content": [{ "type": "text", "text": json::to_string_pretty(&value).unwrap_or_default() }]
            }),
            // TOOL-024: a result, never a JSON-RPC error.
            Err(err) => serde_json::json!({
                "content": [{ "type": "text", "text": err.0 }],
                "isError": true
            }),
        }
    }

    async fn dispatch(&self, params: &Value, session_id: &str) -> Result<Value, ToolError> {
        // TOOL-022: `arguments` when it's an object (an array is one too), else {}.
        let empty = Value::Object(Map::new());
        let args = match params.get("arguments") {
            Some(a @ (Value::Object(_) | Value::Array(_))) => a,
            _ => &empty,
        };
        let name_value = params.get("name");
        let name = js_string(name_value);
        // TOOL-025, in order.
        let handlers = self
            .handlers
            .as_ref()
            .ok_or_else(|| ToolError::from("The board is not available"))?;
        let project_id = handlers
            .project_for_session(session_id)
            .ok_or_else(|| ToolError::from("This session is no longer on a Nodus board"))?;
        // A run the user took over is an ordinary session now.
        if SESSION_TOOL_NAMES.contains(&name.as_str()) && !handlers.is_run(session_id) {
            return Err(format!("{name} is only for action runs").into());
        }
        // TOOL-027: the note tools work on this session's board, or another project's.
        let notes_on = || match args.get("project") {
            None => Ok(project_id.clone()),
            Some(project) => handlers.find_project(Some(project)),
        };
        // TOOL-026
        let patch = || NotePatch {
            text: args.get("text").and_then(Value::as_str).map(str::to_owned),
            color: args.get("color").and_then(Value::as_str).and_then(FolderColor::parse),
        };
        // `switch (params.name)` matches only a string.
        let tool = name_value.and_then(Value::as_str).unwrap_or_default();
        match tool {
            "create_note" => handlers.create_note(&notes_on()?, session_id, &patch()),
            "update_note" => {
                // TOOL-031: before the project is looked up.
                let Some(note_id) = args.get("note_id").and_then(Value::as_str) else {
                    return Err("note_id is required".into());
                };
                handlers.update_note(&notes_on()?, note_id, &patch())
            }
            "list_notes" => handlers.list_notes(&notes_on()?, session_id),
            "list_projects" => handlers.list_projects(&project_id),
            "create_project" => handlers.create_project(args.get("path"), args.get("name")),
            // TOOL-035: anything but `true` is false.
            "list_sessions" => handlers.list_sessions(
                &project_id,
                session_id,
                args.get("include_archived") == Some(&Value::Bool(true)),
            ),
            "archive_sessions" => {
                // TOOL-036: only the strings, and force only when it's `true`.
                let ids: Vec<String> = match args.get("session_ids") {
                    Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                    _ => Vec::new(),
                };
                let force = args.get("force") == Some(&Value::Bool(true));
                handlers.archive_sessions(&project_id, session_id, &ids, force)
            }
            "archive_self" => handlers.archive_self(&project_id, session_id),
            // TOOL-039: the views tools work on this session's own board.
            "list_views" => handlers.list_views(&project_id, session_id),
            "get_view" => handlers.get_view(&project_id, session_id, args.get("view_id")),
            "create_view" => handlers.create_view(&project_id, session_id, args).await,
            "update_view" => handlers.update_view(&project_id, session_id, args).await,
            // TOOL-101: the drawing tools work on this session's own board too.
            "list_board" => handlers.list_board(&project_id, session_id),
            "draw_on_board" => handlers.draw_on_board(&project_id, session_id, args),
            "update_drawing" => handlers.update_drawing(&project_id, session_id, args),
            "erase_from_board" => handlers.erase_from_board(&project_id, session_id, args),
            _ => Err(format!("Unknown tool {name}").into()),
        }
    }
}

/// `{jsonrpc: '2.0', id, <key>: value}`, the id left out when the request had
/// none (TOOL-016).
fn reply(id: Option<&Value>, key: &str, value: Value) -> Value {
    let mut out = Map::new();
    out.insert("jsonrpc".into(), Value::String("2.0".into()));
    if let Some(id) = id {
        out.insert("id".into(), id.clone());
    }
    out.insert(key.into(), value);
    Value::Object(out)
}

/// `String(value)` for a JSON value, with `undefined` for one that isn't there.
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.as_f64().map_or_else(|| n.to_string(), json::number_to_string),
        Some(Value::String(s)) => s.clone(),
        // An array joins its items, with nothing for null.
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// The server as the session host client calls it for each relayed `mcp`
/// message (TOOL-014): it always has a reply, so the host never answers for it.
pub struct McpRelay<H>(pub Arc<McpServer<H>>);

impl<H: BoardToolHandlers + 'static> nodus_sessions::client::BoardTools for McpRelay<H> {
    fn answer(
        &self,
        session_id: &str,
        message: Value,
    ) -> nodus_sessions::client::BoxFuture<Result<Option<Value>, String>> {
        let server = Arc::clone(&self.0);
        let session_id = session_id.to_owned();
        Box::pin(async move { Ok(Some(server.answer(&session_id, &message).await)) })
    }
}
