//! What the server tells Claude Code: its instructions, and each tool's name,
//! description and input schema, word for word as 0.9.4 has them, with keys
//! in the same order (TOOL-019, Data & formats in spec 04), and the drawing
//! tools added since (TOOL-101 to TOOL-113).

use nodus_domain::{FolderColor, InkColor};
use serde_json::{Value, json};

use crate::drawing::DRAWN_KINDS;

/// The server's name in Claude Code: its tools are `mcp__nodus__<tool>` (TOOL-001).
pub const BOARD_SERVER: &str = "nodus";

/// The protocol version the server answers `initialize` with when the client
/// doesn't ask for one (TOOL-017).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The server's instructions, sent with `initialize` (TOOL-017): 0.9.4's,
/// and since then the shapes and links (TOOL-110), and looking at the board
/// and drawing on it unasked (TOOL-114).
pub const INSTRUCTIONS: &str = "Nodus is the board of Claude Code sessions the user runs this session from. Besides the session cards, it holds sticky notes and views. When the user asks you to put something on the board, or to make notes of it, write it with create_note. Read the notes they have put up with list_notes. Write notes when the user asks for them, or when a short one would help them come back to something later. When the user asks for a view, a dashboard, a table, a chart or other custom UI on the board, build it with create_view; list_views, get_view and update_view find and change the views there. To put a note on another project’s board, or read the notes there, pass that project to the note tools; list_projects lists the projects. When the user asks for a new project, add it with create_project. When the user asks you to archive yourself, use archive_self. The board also holds shapes (boxes, ellipses, diamonds, words and emoji stickers) and links, arrows between cards, notes, views and shapes. The user links cards to show how their work relates: list_board shows what is on the board and where, and what this session’s card is linked to. look_at_board shows the board as a picture, with the pictures and ink sketches words can’t describe: look at it when the user points at something on the board or asks about what is there. Draw with draw_on_board when the user asks for something drawn, laid out, linked or marked, and on your own when a small sketch, a picture you made or a mark on what you mean would say it better than words: keep it small and near your card. update_drawing and erase_from_board change and take away drawings; change the user’s only when they ask. The user reads this session as a chat, which can show more than text. To show a chart, stats, a table or a text layout, call show_visual (or put the same JSON in a ```visual block in your reply). To ask the user questions with choices, call ask (or use a ```questions block), then end your turn: the answers arrive as their next message. For a mockup, a side-by-side comparison or a small tool that needs HTML, call show_page, which puts a page on the board beside your card. Prefer show_visual and ask, and don't repeat in words what they show.";

/// The `--mcp-config` for every claude process (TOOL-002). Claude Code expands
/// the `${…}` from the environment `hookEnv` gives it, so the token never
/// shows in a process listing. The path is the session host's `MCP_PATH`
/// (`/mcp`) and the header its `MCP_SESSION_HEADER` (`X-Nodus-Session`).
pub const BOARD_MCP_CONFIG: &str = concat!(
    r#"{"mcpServers":{"nodus":{"type":"http","url":"http://127.0.0.1:${NODUS_PORT}/mcp","#,
    r#""headers":{"Authorization":"Bearer ${NODUS_TOKEN}","X-Nodus-Session":"${NODUS_SESSION}"}}}}"#
);

/// Every session's tools: 0.9.4's six, then the drawing tools (TOOL-101,
/// TOOL-111).
pub const COMMON_TOOL_NAMES: [&str; 11] = [
    "create_note",
    "update_note",
    "list_notes",
    "list_projects",
    "create_project",
    "archive_self",
    "list_board",
    "look_at_board",
    "draw_on_board",
    "update_drawing",
    "erase_from_board",
];

/// Every session's view tools.
pub const VIEW_TOOL_NAMES: [&str; 4] = ["create_view", "update_view", "get_view", "list_views"];

/// Every session's tools that show something in its chat (spec 16).
pub const CHAT_TOOL_NAMES: [&str; 3] = ["show_visual", "ask", "show_page"];

/// An action run's tools, on top of the others.
pub const SESSION_TOOL_NAMES: [&str; 2] = ["list_sessions", "archive_sessions"];

/// Allow rules that let a session use every board tool it has without asking
/// (TOOL-003): `--allowedTools`, one argument each, in the order the tools
/// are listed.
pub const BOARD_TOOL_RULES: [&str; 20] = [
    "mcp__nodus__create_note",
    "mcp__nodus__update_note",
    "mcp__nodus__list_notes",
    "mcp__nodus__list_projects",
    "mcp__nodus__create_project",
    "mcp__nodus__archive_self",
    "mcp__nodus__list_board",
    "mcp__nodus__look_at_board",
    "mcp__nodus__draw_on_board",
    "mcp__nodus__update_drawing",
    "mcp__nodus__erase_from_board",
    "mcp__nodus__create_view",
    "mcp__nodus__update_view",
    "mcp__nodus__get_view",
    "mcp__nodus__list_views",
    "mcp__nodus__show_visual",
    "mcp__nodus__ask",
    "mcp__nodus__show_page",
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
            "description": "Puts a sticky note on the user’s Nodus board, beside this session’s card. Use it when the user asks you to put something on the board: findings, a summary, a plan, a list to come back to; and on your own, sparingly, when a short note would help them come back to something later. A note shows its text as written, so Markdown is not rendered: skip **bold** and # headings, though \"- \" bullets and blank lines read well. Keep each note to one topic someone can take in at a glance, starting with a short line that says what it is about, and write several notes for several topics. Returns the note’s id.",
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

fn ink_color_property(description: &str) -> Value {
    let colors: Vec<&str> = InkColor::ALL.iter().map(|c| c.as_str()).collect();
    json!({ "type": "string", "enum": colors, "description": description })
}

fn number_property(description: &str) -> Value {
    json!({ "type": "number", "description": description })
}

/// Where something is: the id of a card, note, view, shape, ink or frame
/// from list_board, or "self", for draw_on_board's ink and look_at_board.
const TARGET: &str = "the id of a card (its session id), note, view, shape, ink stroke or frame from list_board, a key from shapes, or \"self\" for this session’s card";

/// Every session's drawing tools, added after 0.9.4 (TOOL-101 to TOOL-113).
fn drawing_tools() -> Vec<Value> {
    let kinds: Vec<&str> = DRAWN_KINDS.iter().map(|k| k.as_str()).collect();
    let point = json!({
        "type": "array",
        "items": { "type": "number" },
        "description": "[x, y] in board coordinates."
    });
    let end =
        "the id of a card (its session id), note, view or shape from list_board, or \"self\" for this session’s card";
    vec![
        json!({
            "name": "list_board",
            "description": "Shows what is on this session’s Nodus board and where: the session cards (one of them this session’s), sticky notes, views and shapes, each with its id and its box (x and y are its top left corner in board units, y growing downward), and the links between them. It lists the user’s frames too, named boxes whose contents move with them (what lies in a frame is what has its centre in its box), and groups, things the user glued together, each as the ids in it, and the ink drawn over the board: pen strokes, arrows, boxes and handwritten words, each with its box, what an arrow points from and to, and what a stroke goes around. linked_to_this_session says in words what this session’s card is linked to and from. The user links cards to show how their work relates, so look there to see which work bears on yours. Use it before drawing on the board, rearranging it or linking things on it; to see the pictures and sketches themselves, use look_at_board.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "look_at_board",
            "description": "Shows this session’s Nodus board as a picture: the cards, notes, views, shapes and links list_board lists, and what words can’t: the pictures on it, the user’s ink (sketches, circles, arrows, handwriting) and how things sit together. Each thing carries a small tag, such as c1 for a card, n2 a note, v3 a view, s4 a shape, p5 a picture, i6 ink, l7 a link and f8 a frame, and tags maps each one to its id for the other board tools. This session’s own card is a dashed outline, so what lies under it shows. Without area or around it shows the whole board; to read small things, look closer with area or around. Use it when the user points at something on the board, asks about a picture or a sketch there, or wants something drawn beside what is there. Returns the picture, the area it shows, its scale (pixels per board unit) and the tags.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "area": {
                        "type": "object",
                        "description": "The part of the board to show, in board coordinates as list_board gives them.",
                        "properties": { "x": { "type": "number" }, "y": { "type": "number" }, "w": { "type": "number" }, "h": { "type": "number" } },
                        "required": ["x", "y", "w", "h"]
                    },
                    "around": {
                        "description": "Shows the space around something instead: the id of a card, note, view, shape, ink stroke or frame from list_board, \"self\" for this session’s card, or a list of them.",
                        "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "string" } }]
                    },
                    "margin": number_property("How much room to show around what around names, in board units. Defaults to 240.")
                }
            }
        }),
        json!({
            "name": "draw_on_board",
            "description": "Draws on the user’s Nodus board, as on a whiteboard: boxes, ellipses, diamonds, words, emoji stickers and pictures; links, arrows between them and the cards, notes and views already there; and ink, marks over the board: a circle around something, a line under it, an arrow, a box, words, or a free pen stroke. Use it when the user asks for something drawn, laid out, linked or marked on the board, such as a diagram, a flow, or how pieces of work relate, and on your own when a small sketch, a picture you made (a screenshot, a rendered diagram or chart) or a mark on what you mean would help more than words; keep those small, near your card, and say what you drew. Give shapes’ x and y from the drawing’s own top left: the whole drawing goes in the nearest free spot beside this session’s card, unless at puts it somewhere. Shapes without x and y go in a row, left to right. Ink is placed in board coordinates, or on what it marks. Returns the new shapes’ ids by key (or by their place in shapes, from 0), the new links’ ids, the new ink’s ids, and the area the drawing took.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "shapes": {
                        "type": "array",
                        "description": "The shapes to draw.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": { "type": "string", "description": "A name for the shape in this call, for links to point at it by. Its id comes back under it." },
                                "kind": { "type": "string", "enum": kinds, "description": "rect, ellipse and diamond are outlines with words inside; text is words on their own; sticker is an emoji; image is a picture from a file." },
                                "path": { "type": "string", "description": "An image’s file: the absolute path of a PNG, JPEG, GIF or WebP picture, at most 25 MB. It is copied onto the board." },
                                "text": { "type": "string", "description": "The words in it, or a sticker’s emoji." },
                                "color": ink_color_property("Its colour. Defaults to graphite."),
                                "x": number_property("Its left edge, from the drawing’s left. Give x and y, or neither to put it in the row."),
                                "y": number_property("Its top edge, from the drawing’s top."),
                                "w": number_property("Its width. Defaults to 180 for a rect or an ellipse, 160 for a diamond, 220 for text and 64 for a sticker; an image keeps its proportions, at most 480 across by default."),
                                "h": number_property("Its height. Defaults to 90 for a rect or an ellipse, 110 for a diamond, 44 for text and 64 for a sticker.")
                            },
                            "required": ["kind"]
                        }
                    },
                    "links": {
                        "type": "array",
                        "description": "Arrows to draw.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string", "description": format!("What the arrow starts at: a key from shapes, {end}.") },
                                "to": { "type": "string", "description": "What the arrow points at, given the same way." },
                                "label": { "type": "string", "description": "A few words on the arrow." },
                                "color": ink_color_property("Its colour. Defaults to graphite.")
                            },
                            "required": ["from", "to"]
                        }
                    },
                    "ink": {
                        "type": "array",
                        "description": "Ink marks to draw, in board coordinates as list_board gives them.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": crate::drawing::MARKS, "description": "circle goes around what around names; underline goes under what under names; rect is a box around what around names; arrow runs from from to to; text is words at x and y, or beside something; pen is a free stroke through points." },
                                "around": { "description": format!("For circle and rect: {TARGET}, or a list of them."), "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "string" } }] },
                                "under": { "description": format!("For underline: {TARGET}, or a list of them."), "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "string" } }] },
                                "from": { "description": format!("For arrow: where it starts, {TARGET}, or a point."), "anyOf": [{ "type": "string" }, point.clone()] },
                                "to": { "description": "For arrow: what it points at, given the same way.", "anyOf": [{ "type": "string" }, point.clone()] },
                                "text": { "type": "string", "description": "For text: the words." },
                                "x": number_property("For text: its left edge."),
                                "y": number_property("For text: its top edge."),
                                "beside": { "type": "string", "description": format!("For text instead of x and y: {TARGET}, to put the words to its right.") },
                                "points": { "type": "array", "items": point, "description": "For pen: the points it runs through, 2 to 2000." },
                                "color": ink_color_property("Its colour. Defaults to graphite."),
                                "size": number_property("How thick, 0.5 to 4. Defaults to 1.")
                            },
                            "required": ["kind"]
                        }
                    },
                    "at": {
                        "type": "object",
                        "description": "Where the drawing’s top left goes, in board coordinates as list_board gives them. Leave it out to put the drawing beside this session’s card.",
                        "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
                        "required": ["x", "y"]
                    }
                }
            }
        }),
        json!({
            "name": "update_drawing",
            "description": "Changes what is on the board: a shape’s place, size, words or colour, a link’s label, colour or ends, and where cards, notes and views are, which it only moves. Positions are board coordinates, as list_board gives them. Use it when the user asks for the board to be changed, rearranged or regrouped, or for something drawn on it to be fixed, and to fix what you drew; change the user’s own only when they ask. Ink isn’t changed: erase it and draw it again. Returns the ids it changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "description": "Shapes to change, and cards, notes and views to move.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "The id of a shape, card (its session id), note or view, from list_board." },
                                "x": number_property("Its new left edge."),
                                "y": number_property("Its new top edge."),
                                "w": number_property("A shape’s new width."),
                                "h": number_property("A shape’s new height."),
                                "text": { "type": "string", "description": "A shape’s new words, or a sticker’s new emoji." },
                                "color": ink_color_property("A shape’s new colour.")
                            },
                            "required": ["id"]
                        }
                    },
                    "links": {
                        "type": "array",
                        "description": "Links to change.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "A link’s id, from list_board or draw_on_board." },
                                "label": { "type": "string", "description": "Its new words. An empty string takes them off." },
                                "color": ink_color_property("Its new colour."),
                                "from": { "type": "string", "description": format!("What it starts at now: {end}.") },
                                "to": { "type": "string", "description": "What it points at now, given the same way." }
                            },
                            "required": ["id"]
                        }
                    }
                }
            }
        }),
        json!({
            "name": "erase_from_board",
            "description": "Takes shapes, links and ink off the board. A shape’s links go with it. Cards, notes and views can’t be erased. Use it when the user asks for something drawn on the board to go, and to take away what you drew once it’s no longer needed; erase the user’s own only when they ask. Returns what was erased, and what was skipped and why.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "Ids of shapes, links and ink, from list_board, look_at_board or draw_on_board." }
                },
                "required": ["ids"]
            }
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

/// The tools that show something in the session's chat (TOOLCHAT-001…003).
fn chat_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "show_visual",
            "description": VISUAL_GUIDE,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "visual": { "type": "object", "description": "A chart (an object with \"mark\") or a layout (an object with \"rows\"), as described above." }
                },
                "required": ["visual"]
            }
        }),
        json!({
            "name": "ask",
            "description": ASK_GUIDE,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "A short, specific name for what this asks, such as \"Settings redesign: layout\"." },
                    "submit": { "type": "string", "description": "The send button's label. Defaults to Send answers." },
                    "questions": {
                        "type": "array",
                        "description": "1 to 20 questions, in the order the user should answer them.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "tag": { "type": "string", "description": "A short label to refer to it by, such as Q1." },
                                "title": { "type": "string", "description": "The question." },
                                "detail": { "type": "string", "description": "Why it matters, in a sentence or two." },
                                "name": { "type": "string", "description": "The answer's key. Defaults to the title." },
                                "multiple": { "type": "boolean", "description": "The user may pick several choices." },
                                "type": { "type": "string", "enum": ["text"], "description": "A text box instead of choices." },
                                "placeholder": { "type": "string", "description": "The text box's hint." },
                                "choices": {
                                    "type": "array",
                                    "description": "2 to 26 choices: each its label, or { label, detail, value, recommended }.",
                                    "items": {}
                                }
                            },
                            "required": ["title"]
                        }
                    }
                },
                "required": ["title", "questions"]
            }
        }),
        json!({
            "name": "show_page",
            "description": PAGE_GUIDE,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "A short, specific name for the page." },
                    "html": { "type": "string", "description": "A whole, self-contained HTML page, as for create_view." },
                    "description": { "type": "string", "description": "What the page is for." },
                    "data": { "description": "JSON the page reads as nodus.data." },
                    "width": { "type": "number", "description": "Width in px; defaults to 480." },
                    "height": { "type": "number", "description": "Height in px; defaults to 330." }
                },
                "required": ["title", "html"]
            }
        }),
    ]
}

/// `show_visual`'s description: what it's for and the JSON it takes.
const VISUAL_GUIDE: &str = "Shows a chart, stat tiles, a table or a text layout in the user's chat, drawn natively under your reply. Prefer it to describing numbers in prose, and to ASCII tables. It returns an error naming the problem when the JSON doesn't fit; fix it and call again.\n\nThe visual is one of:\n- A chart: a Vega-Lite subset. \"mark\": bar, line, area, point (circle, square) or arc (pie; {\"type\": \"arc\", \"innerRadius\": 50} for a donut); the object form {\"type\": \"line\", \"point\": true} adds points. \"data\": {\"values\": [rows]} inline only. \"encoding\" channels: x, y, color, theta, tooltip, xOffset (grouped bars). Field definitions: field, type (quantitative, nominal, ordinal, temporal), aggregate (sum, mean, count, min, max, median), title, sort (\"ascending\", \"descending\", null, or an array of values), axis (null, or {\"title\"}), scale ({\"domain\": [min, max], \"zero\": false}), stack (null, true, \"normalize\"). Optional \"title\" and \"height\" (100 to 600). Bars that share an x stack by color unless xOffset groups them. Not supported: layer, facet, repeat, concat, transform, params, bin, data.url and the rule, text, tick and rect marks; compute bins, totals or averages into the rows yourself.\n- A layout: {\"title\"?, \"rows\": [part, …]}, top to bottom, where a part is {\"chart\": chart}, {\"stats\": [{\"label\", \"value\", \"note\"?, \"tone\"?: \"good\" | \"bad\" | \"neutral\"}]}, {\"table\": {\"columns\"?, \"rows\": [[…]] or [{…}]}}, {\"text\": markdown} or {\"columns\": [part, …]} side by side. Any part may have a \"title\".\n\nThe width always fits the chat. Aggregate large data before sending it.";

/// `ask`'s description.
const ASK_GUIDE: &str = "Asks the user questions as a form in their chat: every question at once, choices picked with a click (lettered for them, with an \"Other\" choice and a note box added), and one Send button. Use it instead of asking in prose or with AskUserQuestion when you need the user's decisions. Give the form a short, specific title, and each choice question 2 or more choices; mark the one you recommend with \"recommended\": true, and link what a question is about (a PR, a file) in its title or detail. After calling it, end your turn: the answers arrive as the user's next message, in a <nodus-answers> block keyed by question (its name, else its title). A question left blank arrives as \"skipped\", a note as \"<key> (note)\".";

/// `show_page`'s description.
const PAGE_GUIDE: &str = "Puts a page on the user's Nodus board beside this session's card, and a card for it in their chat: for what show_visual and ask can't express, such as a UI mockup, layouts compared side by side, or a small interactive tool. Write one self-contained HTML page as for create_view, styled with the board's CSS variables. A page that asks the user something calls nodus.submit(answers) (any JSON) from its own Send button; the answers arrive as the user's next message, in a <nodus-answers> block. Returns the view's id; change the page with update_view.";

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
        session.extend(drawing_tools());
        session.extend(view_tools(view_guide));
        session.extend(chat_tools());
        let mut run = session.clone();
        run.extend(session_tools());
        Self {
            session: Value::Array(session),
            run: Value::Array(run),
        }
    }
}
