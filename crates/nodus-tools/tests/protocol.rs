//! boardTools.test.ts, ported, and the answers compared byte for byte with
//! what 0.9.4's boardTools.ts gives (fixtures/tools.json, from generate.mjs).

use std::sync::Mutex;

use nodus_board::notes::NotePatch;
use nodus_domain::json as js_json;
use nodus_tools::{BOARD_MCP_CONFIG, BOARD_TOOL_RULES, BoardToolHandlers, BoxFuture, McpServer, ToolError};
use serde_json::{Value, json};

const RUN: &str = "run-session";

/// Every call the server made, as `[name, args…]`, and what to answer.
#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<Value>>,
    fail_create_note: bool,
    fail_update_view: bool,
    list_projects: Option<Value>,
}

impl Mock {
    fn record(&self, call: Value) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<Value> {
        self.calls.lock().unwrap().clone()
    }

    fn called(&self, name: &str) -> Vec<Value> {
        self.calls().into_iter().filter(|c| c[0] == name).collect()
    }
}

fn patch_json(p: &NotePatch) -> Value {
    json!({ "text": p.text, "color": p.color })
}

impl BoardToolHandlers for Mock {
    fn project_for_session(&self, session_id: &str) -> Option<String> {
        // Every session is on project p1, except "gone", which has left the board.
        (session_id != "gone").then(|| "p1".into())
    }
    fn is_run(&self, session_id: &str) -> bool {
        session_id == RUN
    }
    fn list_projects(&self, own: &str) -> Result<Value, ToolError> {
        self.record(json!(["listProjects", own]));
        Ok(self
            .list_projects
            .clone()
            .unwrap_or(json!([{ "id": "p1" }, { "id": "p2" }])))
    }
    fn create_project(&self, folder: Option<&Value>, name: Option<&Value>) -> Result<Value, ToolError> {
        self.record(json!(["createProject", folder, name]));
        Ok(json!({ "projectId": "p3" }))
    }
    fn find_project(&self, reference: Option<&Value>) -> Result<String, ToolError> {
        match reference.and_then(Value::as_str) {
            Some("p2" | "Docs") => Ok("p2".into()),
            _ => Err(format!(
                "There’s no project called “{}”; list_projects lists them",
                reference.and_then(Value::as_str).unwrap_or_default()
            )
            .into()),
        }
    }
    fn list_sessions(&self, project_id: &str, self_id: &str, include_archived: bool) -> Result<Value, ToolError> {
        self.record(json!(["listSessions", project_id, self_id, include_archived]));
        Ok(json!([{ "id": "s2" }]))
    }
    fn archive_sessions(
        &self,
        project_id: &str,
        self_id: &str,
        ids: &[String],
        force: bool,
    ) -> Result<Value, ToolError> {
        self.record(json!(["archiveSessions", project_id, self_id, ids, force]));
        Ok(json!({ "archived": [] }))
    }
    fn archive_self(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        self.record(json!(["archiveSelf", project_id, self_id]));
        Ok(json!({ "archiving": "later" }))
    }
    fn list_notes(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        self.record(json!(["listNotes", project_id, self_id]));
        Ok(json!([]))
    }
    fn create_note(&self, project_id: &str, self_id: &str, note: &NotePatch) -> Result<Value, ToolError> {
        self.record(json!(["createNote", project_id, self_id, patch_json(note)]));
        if self.fail_create_note {
            return Err("A note needs some text".into());
        }
        Ok(json!({ "noteId": "n1" }))
    }
    fn update_note(&self, project_id: &str, note_id: &str, patch: &NotePatch) -> Result<Value, ToolError> {
        self.record(json!(["updateNote", project_id, note_id, patch_json(patch)]));
        Ok(json!({ "noteId": "n1" }))
    }
    fn list_views(&self, project_id: &str, self_id: &str) -> Result<Value, ToolError> {
        self.record(json!(["listViews", project_id, self_id]));
        Ok(json!([]))
    }
    fn get_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<Value, ToolError> {
        // An absent view_id is recorded as "undefined".
        self.record(json!([
            "getView",
            project_id,
            self_id,
            view_id.cloned().unwrap_or(json!("undefined"))
        ]));
        Ok(json!({ "id": "v1" }))
    }
    fn create_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        self.record(json!(["createView", project_id, self_id, args]));
        Box::pin(async {
            tokio::task::yield_now().await;
            Ok(json!({ "view_id": "v1" }))
        })
    }
    fn update_view<'a>(
        &'a self,
        project_id: &'a str,
        self_id: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        self.record(json!(["updateView", project_id, self_id, args]));
        let fail = self.fail_update_view;
        Box::pin(async move {
            tokio::task::yield_now().await;
            if fail {
                Err("There’s no view with that id on this board".into())
            } else {
                Ok(json!({ "view_id": "v1", "version": 2 }))
            }
        })
    }
}

fn server(mock: Mock) -> McpServer<Mock> {
    McpServer::new("9.9.9", "<VIEW_GUIDE>", Some(mock))
}

fn mock(server: &McpServer<Mock>) -> &Mock {
    server.handlers().unwrap()
}

async fn call(server: &McpServer<Mock>, session_id: &str, name: &str, args: Option<Value>) -> Value {
    let mut params = json!({ "name": name });
    if let Some(args) = args {
        params["arguments"] = args;
    }
    server
        .answer(
            session_id,
            &json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params }),
        )
        .await
}

/// The reply's text, parsed as the JSON it holds.
fn parsed(reply: &Value) -> Value {
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn text(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"].as_str().unwrap()
}

async fn tool_names(server: &McpServer<Mock>, session_id: &str) -> Vec<String> {
    let reply = server
        .answer(session_id, &json!({ "id": 1, "method": "tools/list" }))
        .await;
    reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn protocol_answers_initialize_in_the_version_asked_for() {
    let server = server(Mock::default());
    let reply = server
        .answer(
            "s1",
            &json!({ "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }),
        )
        .await;
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 1);
    let result = &reply["result"];
    assert_eq!(result["protocolVersion"], "2025-03-26");
    assert_eq!(result["capabilities"], json!({ "tools": {} }));
    assert_eq!(result["serverInfo"], json!({ "name": "nodus", "version": "9.9.9" }));
    assert!(result["instructions"].as_str().unwrap().contains("create_note"));
}

#[tokio::test]
async fn protocol_answers_ping_and_unknown_methods_with_an_error() {
    let server = server(Mock::default());
    assert_eq!(
        server.answer("s1", &json!({ "id": 2, "method": "ping" })).await,
        json!({ "jsonrpc": "2.0", "id": 2, "result": {} })
    );
    assert_eq!(
        server
            .answer("s1", &json!({ "id": 3, "method": "resources/list" }))
            .await,
        json!({ "jsonrpc": "2.0", "id": 3, "error": { "code": -32601, "message": "Unknown method resources/list" } })
    );
}

#[tokio::test]
async fn protocol_gives_every_session_the_note_and_view_tools_and_action_runs_the_session_tools_too() {
    let server = server(Mock::default());
    let common = [
        "create_note",
        "update_note",
        "list_notes",
        "list_projects",
        "create_project",
        "archive_self",
        "create_view",
        "update_view",
        "get_view",
        "list_views",
    ];
    assert_eq!(tool_names(&server, "s1").await, common);
    let mut run = common.to_vec();
    run.extend(["list_sessions", "archive_sessions"]);
    assert_eq!(tool_names(&server, RUN).await, run);
}

#[test]
fn protocol_allows_every_tool_by_name_and_keeps_the_token_out_of_the_config() {
    assert_eq!(
        BOARD_TOOL_RULES,
        [
            "mcp__nodus__create_note",
            "mcp__nodus__update_note",
            "mcp__nodus__list_notes",
            "mcp__nodus__list_projects",
            "mcp__nodus__create_project",
            "mcp__nodus__archive_self",
            "mcp__nodus__create_view",
            "mcp__nodus__update_view",
            "mcp__nodus__get_view",
            "mcp__nodus__list_views",
            "mcp__nodus__list_sessions",
            "mcp__nodus__archive_sessions",
        ]
    );
    let config: Value = serde_json::from_str(BOARD_MCP_CONFIG).unwrap();
    let nodus = &config["mcpServers"]["nodus"];
    assert_eq!(nodus["url"], "http://127.0.0.1:${NODUS_PORT}/mcp");
    assert_eq!(
        nodus["headers"],
        json!({ "Authorization": "Bearer ${NODUS_TOKEN}", "X-Nodus-Session": "${NODUS_SESSION}" })
    );
}

#[tokio::test]
async fn tools_create_a_note_dropping_a_colour_that_is_not_one_of_the_boards() {
    let server = server(Mock::default());
    let reply = call(
        &server,
        "s1",
        "create_note",
        Some(json!({ "text": "Findings", "color": "chartreuse" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("createNote"),
        [json!(["createNote", "p1", "s1", { "text": "Findings", "color": null }])]
    );
    assert_eq!(reply["id"], 7);
    assert_eq!(parsed(&reply), json!({ "noteId": "n1" }));
}

#[tokio::test]
async fn tools_update_a_note_by_id() {
    let server = server(Mock::default());
    call(
        &server,
        "s1",
        "update_note",
        Some(json!({ "note_id": "n1", "color": "green" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("updateNote"),
        [json!(["updateNote", "p1", "n1", { "text": null, "color": "green" }])]
    );
    let reply = call(&server, "s1", "update_note", Some(json!({ "text": "no id" }))).await;
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(text(&reply), "note_id is required");
}

#[tokio::test]
async fn tools_list_notes_and_archive_the_calling_session() {
    let server = server(Mock::default());
    call(&server, "s1", "list_notes", None).await;
    assert_eq!(mock(&server).called("listNotes"), [json!(["listNotes", "p1", "s1"])]);
    call(&server, "s1", "archive_self", None).await;
    assert_eq!(
        mock(&server).called("archiveSelf"),
        [json!(["archiveSelf", "p1", "s1"])]
    );
}

#[tokio::test]
async fn tools_write_change_and_read_notes_on_another_projects_board_by_its_id_or_name() {
    let server = server(Mock::default());
    call(
        &server,
        "s1",
        "create_note",
        Some(json!({ "text": "For the docs", "project": "Docs" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("createNote"),
        [json!(["createNote", "p2", "s1", { "text": "For the docs", "color": null }])]
    );
    call(
        &server,
        "s1",
        "update_note",
        Some(json!({ "note_id": "n1", "text": "Revised", "project": "p2" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("updateNote"),
        [json!(["updateNote", "p2", "n1", { "text": "Revised", "color": null }])]
    );
    call(&server, "s1", "list_notes", Some(json!({ "project": "p2" }))).await;
    assert_eq!(mock(&server).called("listNotes"), [json!(["listNotes", "p2", "s1"])]);
}

#[tokio::test]
async fn tools_say_so_when_the_project_isnt_one_and_write_nothing() {
    let server = server(Mock::default());
    let reply = call(
        &server,
        "s1",
        "create_note",
        Some(json!({ "text": "Lost", "project": "Nowhere" })),
    )
    .await;
    assert_eq!(
        reply["result"],
        json!({
            "content": [{ "type": "text", "text": "There’s no project called “Nowhere”; list_projects lists them" }],
            "isError": true
        })
    );
    assert!(mock(&server).called("createNote").is_empty());
}

#[tokio::test]
async fn tools_list_projects_and_add_one() {
    let server = server(Mock::default());
    call(&server, "s1", "list_projects", None).await;
    assert_eq!(mock(&server).called("listProjects"), [json!(["listProjects", "p1"])]);
    let reply = call(
        &server,
        "s1",
        "create_project",
        Some(json!({ "path": "~/Code/new-app", "name": "New app" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("createProject"),
        [json!(["createProject", "~/Code/new-app", "New app"])]
    );
    assert_eq!(parsed(&reply), json!({ "projectId": "p3" }));
}

#[tokio::test]
async fn tools_keep_the_session_tools_to_action_runs() {
    let server = server(Mock::default());
    let reply = call(&server, "s1", "list_sessions", None).await;
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(text(&reply), "list_sessions is only for action runs");
    let reply = call(
        &server,
        "s1",
        "archive_sessions",
        Some(json!({ "session_ids": ["s2"] })),
    )
    .await;
    assert_eq!(reply["result"]["isError"], true);
    assert!(mock(&server).called("listSessions").is_empty());
    assert!(mock(&server).called("archiveSessions").is_empty());
}

#[tokio::test]
async fn tools_let_an_action_run_list_and_archive_sessions() {
    let server = server(Mock::default());
    call(
        &server,
        RUN,
        "list_sessions",
        Some(json!({ "include_archived": "yes" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("listSessions"),
        [json!(["listSessions", "p1", RUN, false])]
    );
    call(
        &server,
        RUN,
        "archive_sessions",
        Some(json!({ "session_ids": ["s2", 3, null, "s3"], "force": true })),
    )
    .await;
    assert_eq!(
        mock(&server).called("archiveSessions"),
        [json!(["archiveSessions", "p1", RUN, ["s2", "s3"], true])]
    );
}

#[tokio::test]
async fn tools_turn_failures_into_tool_errors_claude_can_read() {
    let server = server(Mock {
        fail_create_note: true,
        ..Mock::default()
    });
    let reply = call(&server, "s1", "create_note", Some(json!({ "text": "" }))).await;
    assert_eq!(
        reply["result"],
        json!({ "content": [{ "type": "text", "text": "A note needs some text" }], "isError": true })
    );
    let reply = call(&server, "gone", "list_notes", None).await;
    assert_eq!(text(&reply), "This session is no longer on a Nodus board");
    let reply = call(&server, "s1", "delete_everything", None).await;
    assert_eq!(text(&reply), "Unknown tool delete_everything");
}

#[tokio::test]
async fn tools_create_and_update_views_answering_once_the_handler_is_done() {
    let server = server(Mock::default());
    let created = call(
        &server,
        "s1",
        "create_view",
        Some(json!({ "title": "Deploys", "html": "<p>hi</p>" })),
    )
    .await;
    assert_eq!(
        mock(&server).called("createView"),
        [json!(["createView", "p1", "s1", { "title": "Deploys", "html": "<p>hi</p>" }])]
    );
    assert_eq!(parsed(&created), json!({ "view_id": "v1" }));
    let updated = call(&server, "s1", "update_view", Some(json!({ "data": { "count": 3 } }))).await;
    assert_eq!(
        mock(&server).called("updateView"),
        [json!(["updateView", "p1", "s1", { "data": { "count": 3 } }])]
    );
    assert_eq!(parsed(&updated), json!({ "view_id": "v1", "version": 2 }));
}

#[tokio::test]
async fn tools_read_a_view_by_id_or_the_callers_own_when_it_leaves_the_id_out() {
    let server = server(Mock::default());
    call(&server, "s1", "get_view", Some(json!({ "view_id": "v1" }))).await;
    call(&server, "s1", "get_view", None).await;
    assert_eq!(
        mock(&server).called("getView"),
        [
            json!(["getView", "p1", "s1", "v1"]),
            json!(["getView", "p1", "s1", "undefined"])
        ]
    );
    call(&server, "s1", "list_views", None).await;
    assert_eq!(mock(&server).called("listViews"), [json!(["listViews", "p1", "s1"])]);
}

#[tokio::test]
async fn tools_report_a_view_tool_that_fails_as_they_do_any_other() {
    let server = server(Mock {
        fail_update_view: true,
        ..Mock::default()
    });
    let reply = call(&server, "s1", "update_view", Some(json!({ "view_id": "nope" }))).await;
    assert_eq!(
        reply["result"],
        json!({ "content": [{ "type": "text", "text": "There’s no view with that id on this board" }], "isError": true })
    );
}

#[tokio::test]
async fn tools_say_the_board_isnt_there_before_handlers_are_given() {
    // TOOL-025, and tools/list without handlers lists an ordinary session's tools.
    let server: McpServer<Mock> = McpServer::new("9.9.9", "", None);
    let reply = call_none(&server).await;
    assert_eq!(reply["result"]["content"][0]["text"], "The board is not available");
    let tools = server.answer(RUN, &json!({ "id": 1, "method": "tools/list" })).await;
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 10);
}

async fn call_none(server: &McpServer<Mock>) -> Value {
    server
        .answer(
            "s1",
            &json!({ "id": 1, "method": "tools/call", "params": { "name": "list_notes" } }),
        )
        .await
}

#[tokio::test]
async fn tools_take_arguments_only_when_they_are_an_object() {
    // TOOL-022: an array is an object too; anything else is {}.
    let server = server(Mock::default());
    for args in [json!("text"), json!(5), Value::Null] {
        call(&server, "s1", "create_view", Some(args)).await;
    }
    call(&server, "s1", "create_view", Some(json!([1]))).await;
    let args: Vec<Value> = mock(&server)
        .called("createView")
        .iter()
        .map(|c| c[3].clone())
        .collect();
    assert_eq!(args, [json!({}), json!({}), json!({}), json!([1])]);
}

#[tokio::test]
async fn tools_check_the_note_id_before_the_project() {
    // TOOL-031
    let server = server(Mock::default());
    let reply = call(
        &server,
        "s1",
        "update_note",
        Some(json!({ "note_id": 5, "project": "Nowhere" })),
    )
    .await;
    assert_eq!(text(&reply), "note_id is required");
}

// ---------------------------------------------------------------------------
// Byte for byte against 0.9.4

fn fixtures() -> Value {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tools.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_config_and_rules_are_0_9_4s() {
    let fixtures = fixtures();
    assert_eq!(fixtures["mcpConfig"], BOARD_MCP_CONFIG);
    assert_eq!(fixtures["toolRules"], json!(BOARD_TOOL_RULES));
}

#[tokio::test]
async fn answers_as_0_9_4_does_on_macos_and_windows() {
    let fixtures = fixtures();
    for platform in ["darwin", "win32"] {
        let expected = &fixtures[platform];
        let guide = expected["viewGuide"].as_str().unwrap();
        let awkward = json!({
            "at": 1_700_000_000_000_i64,
            "half": 0.5,
            "tiny": 1e-7,
            "huge": 1e21,
            "negativeZero": -0.0,
            "text": "quote \" backslash \\ tab \t newline \n control \u{1} del \u{7f} é \u{2028} 😀",
            "empty": {},
            "none": [],
            "nested": [{ "a": [1, [2, {}]] }],
            "nil": null
        });
        let server = McpServer::new(
            "9.9.9",
            guide,
            Some(Mock {
                list_projects: Some(awkward),
                ..Mock::default()
            }),
        );
        let cases = [
            (
                "initialize",
                "s1",
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            ),
            (
                "initializeAsked",
                "s1",
                json!({ "id": "one", "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }),
            ),
            ("toolsListSession", "s1", json!({ "id": 2, "method": "tools/list" })),
            ("toolsListRun", RUN, json!({ "id": 3, "method": "tools/list" })),
            ("noId", "s1", json!({ "method": "ping" })),
            ("nullId", "s1", json!({ "id": null, "method": "ping" })),
            ("unknown", "s1", json!({ "id": 4, "method": "resources/list" })),
            (
                "pretty",
                "s1",
                json!({ "id": 5, "method": "tools/call", "params": { "name": "list_projects" } }),
            ),
            (
                "unknownTool",
                "s1",
                json!({ "id": 6, "method": "tools/call", "params": {} }),
            ),
            (
                "gone",
                "gone",
                json!({ "id": 7, "method": "tools/call", "params": { "name": "list_notes", "arguments": [] } }),
            ),
        ];
        for (name, session, message) in cases {
            let answer = js_json::to_string(&server.answer(session, &message).await).unwrap();
            assert_eq!(answer, expected[name].as_str().unwrap(), "{platform} {name}");
        }
    }
}
