//! What the server tells Claude Code: its instructions, and each tool's name,
//! description and input schema, word for word as 0.9.4 has them, with keys
//! in the same order (TOOL-019, Data & formats in spec 04).

use nodus_domain::FolderColor;
use serde_json::{Value, json};

/// The server's name in Claude Code: its tools are `mcp__nodus__<tool>` (TOOL-001).
pub const BOARD_SERVER: &str = "nodus";

/// The protocol version the server answers `initialize` with when the client
/// doesn't ask for one (TOOL-017).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The server's instructions, sent with `initialize` (TOOL-017).
pub const INSTRUCTIONS: &str = "Nodus is the board of Claude Code sessions the user runs this session from. Besides the session cards, it holds sticky notes and views. When the user asks you to put something on the board, or to make notes of it, write it with create_note. Read the notes they have put up with list_notes. Only write notes when the user asks for them. When the user asks for a view, a dashboard, a table, a chart or other custom UI on the board, build it with create_view; list_views, get_view and update_view find and change the views there. To put a note on another project’s board, or read the notes there, pass that project to the note tools; list_projects lists the projects. When the user asks for a new project, add it with create_project. When the user asks you to archive yourself, use archive_self.";

/// The `--mcp-config` for every claude process (TOOL-002). Claude Code expands
/// the `${…}` from the environment `hookEnv` gives it, so the token never
/// shows in a process listing. The path is the session host's `MCP_PATH`
/// (`/mcp`) and the header its `MCP_SESSION_HEADER` (`X-Nodus-Session`).
pub const BOARD_MCP_CONFIG: &str = concat!(
    r#"{"mcpServers":{"nodus":{"type":"http","url":"http://127.0.0.1:${NODUS_PORT}/mcp","#,
    r#""headers":{"Authorization":"Bearer ${NODUS_TOKEN}","X-Nodus-Session":"${NODUS_SESSION}"}}}}"#
);

/// Every session's tools.
pub const COMMON_TOOL_NAMES: [&str; 6] = [
    "create_note",
    "update_note",
    "list_notes",
    "list_projects",
    "create_project",
    "archive_self",
];

/// Every session's view tools.
pub const VIEW_TOOL_NAMES: [&str; 4] = ["create_view", "update_view", "get_view", "list_views"];

/// An action run's tools, on top of the others.
pub const SESSION_TOOL_NAMES: [&str; 2] = ["list_sessions", "archive_sessions"];

/// Allow rules that let a session use every board tool it has without asking
/// (TOOL-003): `--allowedTools`, one argument each.
pub const BOARD_TOOL_RULES: [&str; 12] = [
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
];

fn color_property(description: &str) -> Value {
    let colors: Vec<&str> = FolderColor::ALL.iter().map(|c| c.as_str()).collect();
    json!({ "type": "string", "enum": colors, "description": description })
}

/// Points a note tool at another project's board.
fn project_property() -> Value {
    json!({
        "type": "string",
        "description": "Another project’s board to use, by its id or name from list_projects. Leave out for this session’s own board."
    })
}

/// Every session's tools (`COMMON_TOOLS`).
fn common_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "create_note",
            "description": "Puts a sticky note on the user’s Nodus board, beside this session’s card. Use it when the user asks you to put something on the board: findings, a summary, a plan, a list to come back to. A note shows its text as written, so Markdown is not rendered: skip **bold** and # headings, though \"- \" bullets and blank lines read well. Keep each note to one topic someone can take in at a glance, starting with a short line that says what it is about, and write several notes for several topics. Returns the note’s id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "What the note says." },
                    "color": color_property("The note’s colour. Defaults to yellow. Colours can tell kinds of notes apart."),
                    "project": project_property()
                },
                "required": ["text"]
            }
        }),
        json!({
            "name": "update_note",
            "description": "Changes the text or colour of a note on the board. Use it to revise a note rather than writing another beside it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note_id": { "type": "string", "description": "An id from create_note or list_notes." },
                    "text": { "type": "string", "description": "The note’s new text, in full." },
                    "color": color_property("The note’s new colour."),
                    "project": project_property()
                },
                "required": ["note_id"]
            }
        }),
        json!({
            "name": "list_notes",
            "description": "Lists the sticky notes on the board, the user’s and any Claude wrote: each one’s id, text and colour, and whether this session wrote it.",
            "inputSchema": { "type": "object", "properties": { "project": project_property() } }
        }),
        json!({
            "name": "list_projects",
            "description": "Lists the user’s Nodus projects, each with a board of its own: its id, name and folder, whether it is archived, and which one this session is on. Use it to find the project for a note on another board.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "create_project",
            "description": "Adds a project to Nodus: a folder on this machine, which gets a board of its own. Use it only when the user asks for a new project. The folder is made if it isn’t there yet, and a folder that is already a project gives back that project. Returns the project’s id and name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The project’s folder, as an absolute path. A leading ~ is the home folder." },
                    "name": { "type": "string", "description": "What to call the project. Defaults to the folder’s name." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "archive_self",
            "description": "Moves this session into the board’s archive when your current reply ends, and stops it. Its transcript is kept, so the user can pick it up again later. Use it only when the user asks you to archive yourself, and call it as the last thing you do: the session stops as soon as this turn ends.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

/// What a view's page, data, commands and refresh look like, for create_view
/// and update_view (`VIEW_PROPERTIES`), in order.
fn view_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "title",
            json!({ "type": "string", "description": "A short name for the view, shown on its card." }),
        ),
        (
            "html",
            json!({
                "type": "string",
                "description": "The page: one self-contained HTML document, with its CSS and JavaScript inline. It draws from nodus.data."
            }),
        ),
        (
            "data",
            json!({
                "description": "JSON the page draws from, as nodus.data. Sending new data updates the page in place without reloading it."
            }),
        ),
        (
            "commands",
            json!({
                "type": "array",
                "description": "Shell commands the page may run, replacing the ones it had. The user allows each command line once on the card; an unchanged one stays allowed.",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "What the page calls it by: nodus.run(name)." },
                        "run": { "type": "string", "description": "The command line, run in the project folder with the user’s shell environment." },
                        "every": { "type": "number", "description": "Minutes between runs while the app is open (at least 1). Leave out to run only when the page asks." }
                    },
                    "required": ["name", "run"]
                }
            }),
        ),
        (
            "refresh",
            json!({
                "type": ["object", "null"],
                "description": "Asks the view’s brain to bring it up to date every so often, for data only Claude can fetch (an MCP server, say). Each refresh costs tokens. null stops it.",
                "properties": {
                    "every": { "type": "number", "description": "Minutes between refreshes, at least 10." },
                    "prompt": {
                        "type": "string",
                        "description": "What the brain is asked each time. It starts fresh, so say where the data comes from and to send it with update_view."
                    }
                },
                "required": ["every", "prompt"]
            }),
        ),
        (
            "start_sessions",
            json!({
                "type": "boolean",
                "description": "Lets the page start Claude sessions on the board with nodus.startSession, from buttons the user clicks. Only turn it on when the user asks for the view to start sessions."
            }),
        ),
        (
            "width",
            json!({ "type": "number", "description": "Card width in board units. Defaults to 480." }),
        ),
        (
            "height",
            json!({ "type": "number", "description": "Card height in board units. Defaults to 360." }),
        ),
    ]
}

fn view_id_property() -> (&'static str, Value) {
    (
        "view_id",
        json!({
            "type": "string",
            "description": "An id from create_view or list_views. A view’s brain can leave it out for its own view."
        }),
    )
}

fn properties(entries: Vec<(&'static str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// Every session's view tools (`VIEW_TOOLS`). `view_guide` is the "how a view
/// works" text (`VIEW_GUIDE`, nodus-views), which ends `create_view`'s description.
fn view_tools(view_guide: &str) -> Vec<Value> {
    let mut create = view_properties();
    create.push((
        "description",
        json!({
            "type": "string",
            "description": "What the view is for and where its data comes from, for its brain to go on later."
        }),
    ));
    let mut update = vec![view_id_property()];
    update.extend(view_properties());
    vec![
        json!({
            "name": "create_view",
            "description": format!("Puts a view on the user’s Nodus board, beside this session’s card: a small live web page, such as a table, a dashboard or a chart, that stays up to date. Use it when the user asks for a view or for custom UI on the board. It returns the view’s id, the commands waiting for the user to allow them, and any JavaScript errors the page hit as it loaded. The view gets a brain of its own, a Claude session that keeps it up to date, when it first needs one.\n\n{view_guide}"),
            "inputSchema": { "type": "object", "properties": properties(create), "required": ["title", "html"] }
        }),
        json!({
            "name": "update_view",
            "description": "Changes a view on the board: its page, its data, its title, its commands or its refresh schedule. Leave out what stays the same. To show fresh data, send only data. Returns the commands waiting for the user to allow them and, when the page changed, any JavaScript errors it hit as it loaded.",
            "inputSchema": { "type": "object", "properties": properties(update) }
        }),
        json!({
            "name": "get_view",
            "description": "Reads a view as it is now: its page, data and saved state, its commands with whether the user allowed them and their last output, its refresh schedule, and the JavaScript errors it hit lately.",
            "inputSchema": { "type": "object", "properties": properties(vec![view_id_property()]) }
        }),
        json!({
            "name": "list_views",
            "description": "Lists the views on the board: each one’s id, title and description, and whether it’s yours.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

/// An action run's tools, on top of the common ones (`SESSION_TOOLS`).
fn session_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_sessions",
            "description": "Lists the sessions on this project’s Nodus board. Each has its id, title, kind (claude or terminal), status (working, waiting for the user, or done), whether its process is running, whether it is archived, the section it is in, Claude’s last reply, and when it was last active. The session running this action is not listed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_archived": { "type": "boolean", "description": "Also list archived sessions. Defaults to false." }
                }
            }
        }),
        json!({
            "name": "archive_sessions",
            "description": "Moves sessions into the board’s archive and stops their processes. Their transcripts are kept, so they can be picked up again later. A session nothing was said in is removed from the board instead, as there is nothing to keep. Sessions that are working or waiting for the user are skipped unless force is true. Terminals cannot be archived.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_ids": { "type": "array", "items": { "type": "string" }, "description": "Ids from list_sessions." },
                    "force": { "type": "boolean", "description": "Archive sessions even if they are working or waiting." }
                },
                "required": ["session_ids"]
            }
        }),
    ]
}

/// The `tools/list` answers: an ordinary session's tools, and an action run's.
#[derive(Debug, Clone)]
pub(crate) struct ToolLists {
    pub(crate) session: Value,
    pub(crate) run: Value,
}

impl ToolLists {
    pub(crate) fn new(view_guide: &str) -> Self {
        let mut session = common_tools();
        session.extend(view_tools(view_guide));
        let mut run = session.clone();
        run.extend(session_tools());
        Self {
            session: Value::Array(session),
            run: Value::Array(run),
        }
    }
}
