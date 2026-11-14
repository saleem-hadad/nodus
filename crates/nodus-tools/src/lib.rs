//! The board tools: an MCP server that lets Claude sessions work with the
//! board they run on. Sessions write sticky notes, put up views, and draw
//! shapes and links (and see the links touching their own card), action runs
//! list and archive the board's sessions, and a session can add a project
//! when the user asks for one.
//!
//! Claude Code reaches the server over the MCP streamable HTTP transport, at
//! the session host, which passes each request on to the app (nodus-host,
//! TOOL-004 to TOOL-013). The app answers it with [`McpServer::answer`]. The
//! host outlives the app, as the sessions do, so a session keeps its tools
//! when the app restarts. Every process gets the same config
//! ([`BOARD_MCP_CONFIG`]): Claude Code fills in the host's port and token, and
//! the session's id, from the environment `hookEnv` gives it, so the token
//! never shows up in a process listing.
//!
//! Ports `src/main/boardTools.ts` and its handlers in `src/main/ipc.ts`.
//!
//! # Wiring
//!
//! nodus-core builds a [`StoreHandlers`] on the store and the board, with a
//! [`ProjectAdder`] (its `projects.add`) and [`ViewTools`] (nodus-views'
//! `Views`), and an [`McpServer`] with the app's version and nodus-views'
//! `VIEW_GUIDE`. The session host client hands it each relayed request and
//! sends back what it answers (TOOL-014).

mod chat;
mod drawing;
mod handlers;
mod schema;
mod server;

pub use handlers::{
    ProjectAdder, SERVER_PROJECTS, StoreHandlers, StoreHandlersConfig, ViewTools, expand_home, find_project,
    is_absolute,
};
pub use schema::{
    BOARD_MCP_CONFIG, BOARD_SERVER, BOARD_TOOL_RULES, CHAT_TOOL_NAMES, COMMON_TOOL_NAMES, INSTRUCTIONS,
    PROTOCOL_VERSION, SESSION_TOOL_NAMES, VIEW_TOOL_NAMES,
};
pub use server::{BoardToolHandlers, BoxFuture, McpRelay, McpServer, ToolError};
