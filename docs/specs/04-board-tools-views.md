# 04 · Board tools, notes placement & views

Sources:
- `src/main/boardTools.ts`, `src/main/boardTools.test.ts`
- `src/main/notes.ts`, `src/main/notes.test.ts`
- `src/main/views.ts`, `src/main/views.test.ts`
- `src/main/viewPage.ts`, `src/main/viewPage.test.ts`
- `src/main/ink.ts`, `src/main/ink.test.ts`
- `src/main/ipc.ts`: `configureBoardTools({...})` handlers (lines 667–788), `views:*` handlers (790–833), `views.configureViews` (256–260), `archiveWhenDone` (241–254), `sessionTitle` (158–164), `views.brainStatus`/`brainInput`/`brainArgs`/`removeProjectViews`/`flushViews` call sites, `board:save-ink` (557–560)
- `src/main/index.ts`: `nodus-view` scheme registration, `protocol.handle`, `will-frame-navigate` guard (lines 39–47, 87–94, 120–122, 136)
- `src/main/host.ts` lines 291–297, 328–413 (the `/mcp` relay; transport itself is 02's), `src/main/pty.ts` lines 179–185, 209, 383
- `src/main/hostProtocol.ts` (`MCP_PATH`, `MCP_SESSION_HEADER`, `mcp`/`mcp-reply` messages)
- `src/main/board.ts` `sessionList` (171–191), `archiveNode`, `dropNode`, `isEmpty` (used by tools)
- `src/main/store.ts` `getBoard`, `projectIdForView`, `projectIdForSession`, `getProject`
- `src/shared/types.ts` (`BoardView`, `ViewCommand`, `ViewRefresh`, `CommandResult`, `ViewEvent`, `ViewPatch`, `CreateViewOptions`, `BoardNote`, `InkStroke`, `INK_COLORS`, `INK_KINDS`, `FOLDER_COLORS`), `src/shared/geometry.ts` (view sizes, `clampViewSize`; `findFreeSpot`/`findSpotBeside`/`dockRect` are specified by 09)
- `src/shared/api.ts` / `src/preload/index.ts` (`views.*`, `on.view`)
- `src/renderer/src/components/board/viewRelay.ts`, `ViewNode.tsx`, `ViewDialog.tsx`, `views.ts`, `views.css`
- `src/renderer/src/views/BoardView.tsx` view parts (lines 212–215, 1257–1415, 1820–1905, 2033–2049, 2234–2237, 2391–2416, 2525, 2651–2653)
- `src/renderer/src/store.ts` `upsertView`, `patchView`, `removeView` (317–356)
- `src/renderer/src/styles/tokens.css`, `global.css` (`.btn`, `.status-dot`), `components/board/board.css` (`.card-tool`, `.spin`, resize handles), `components/actions/actions.css` (classes the New view dialog uses), `components/Modal.css`
- `docs/how-it-works.md` § Notes, § Views, § Board tools

Cross-domain owners referenced (not re-specified here): 01 platform shell (env: `shellCommand`, `spawnEnv`, `userShell`; `state.json`; `addProject`), 02 session host (hook server, socket protocol, `hookEnv`, status machine `getStatus`), 03 sessions (claude argv incl. `--mcp-config`/`--allowedTools`, `spawnSession`, `sessionList` source, transcripts, `findFreeSpot`/`findSpotBeside`/`dockRect` geometry), 05 git (`gitStatus` payload), 07 capture (creates views/notes), 08 shell UI (Modal, Menu, ConfirmDialog, tokens, `.btn`), 09 board canvas (pan/zoom/wheel, selection, edges incl. the view→session dashed edge, minimap, toasts, `ensureVisible`, `openCard`), 10 cards & notes UI (note rendering incl. the signature, `board:save-note`), 11 overlays (prompt bar `/view`, ink layer UI).

## Summary

Three things that let Claude sessions work on the board they run from, plus ink storage:

1. **Board tools.** An MCP server named `nodus`, reached by every `claude` process the app starts over the MCP streamable-HTTP transport at `http://127.0.0.1:<hook port>/mcp` on the session host. The host relays each JSON-RPC request over its socket to the running app, which answers it (`answerMcp`). Ordinary sessions get 14 tools (notes, projects, `archive_self`, the drawing tools added after 0.9.4, views); action runs also get `list_sessions` and `archive_sessions`. With the app closed every tool call fails with a fixed message.
2. **Notes placement.** Notes Claude writes go beside the writer's card, stacked under the last one it wrote, sized to fit their text (216–360 × 216–480), and signed (`from`, plus `fromProject`/`fromTitle` when written on another project's board, where they go beside the rightmost card).
3. **Views.** Small live HTML pages on the board. Each has files in `<userData>/views/<id>/` (`index.html`, `runtime.json`), is served on the custom scheme `nodus-view://<id>/` with a no-network CSP, an injected theme and a `window.nodus` bridge, and runs in a sandboxed frame (`sandbox="allow-scripts"`). It gets data from Claude (`update_view` data), from shell commands the user allows on the card (run in the project folder, 2-minute timeout, optional schedule ≥ 1 min), and from its *brain*: a hidden Claude session in auto mode (started on demand, stops after 5 idle minutes, resumes when needed; scheduled refresh ≥ 10 min; page asks at most every 15 s). A page may start sessions only when its brain enabled it and only within the user's transient activation, at most one every 3 s. The card UI has a title bar (status, ⟳ ✎ ⌨ ⤢ ⋯), approvals strip, building/empty states, an ask bar, an error line, and a large modal.
4. **Ink.** The renderer sends the full stroke list; main cleans it (`cleanInk`) before saving.

In Rust the frame becomes a `wry` webview embedded in the GPUI board (see Rust mapping); everything the page can observe (CSP, sandbox, bridge, messages, timings) must stay identical.


> **Since 1.0.14 (spec 16):** every session also has `show_visual`, `ask` and `show_page` (TOOLCHAT-001…TOOLCHAT-004), after the view tools in `tools/list` and `--allowedTools`; the instructions end with a paragraph on them; and a view's page has `nodus.submit(answers)` (CHAT-031).

> **Since 1.0.19:** `create_view` without `html` hands the view to its brain, which starts building it at once (VIEW-161), and every Claude process with the board tools loads Nodus's own plugin, whose `nodus:views` skill says how to make views and pages (VIEW-159, VIEW-160). The guide left `create_view`'s description for the skill; brains keep it in their system prompt.

## Behaviors

### Board tools: endpoint and relay

- **TOOL-001** The MCP server's name is `nodus`; Claude Code exposes its tools as `mcp__nodus__<tool>`.
- **TOOL-002** `BOARD_MCP_CONFIG` (passed as `--mcp-config` to every claude process when the host's welcome said `mcp: true`; argv owned by 03) is exactly the string `{"mcpServers":{"nodus":{"type":"http","url":"http://127.0.0.1:${NODUS_PORT}/mcp","headers":{"Authorization":"Bearer ${NODUS_TOKEN}","X-Nodus-Session":"${NODUS_SESSION}"}}}}` with the `${…}` left literal for Claude Code to expand from the process env (`hookEnv`, 02). The token never appears in argv.
- **TOOL-003** `BOARD_TOOL_RULES` (passed after `--allowedTools`) is exactly, in order: `mcp__nodus__create_note`, `mcp__nodus__update_note`, `mcp__nodus__list_notes`, `mcp__nodus__list_projects`, `mcp__nodus__create_project`, `mcp__nodus__archive_self`, `mcp__nodus__list_board`, `mcp__nodus__draw_on_board`, `mcp__nodus__update_drawing`, `mcp__nodus__erase_from_board`, `mcp__nodus__create_view`, `mcp__nodus__update_view`, `mcp__nodus__get_view`, `mcp__nodus__list_views`, `mcp__nodus__list_sessions`, `mcp__nodus__archive_sessions`. 0.9.4 has the same 12 without the four drawing rules (added after 0.9.4, TOOL-101).
- **TOOL-004** The host serves the MCP endpoint on its hook HTTP server (127.0.0.1, hook port) at path `/mcp` (`MCP_PATH`); any request whose URL pathname is `/mcp` goes to the MCP handler before hook handling.
- **TOOL-005** A request is authorized only if its `Authorization` header matches `^Bearer (\S+)$` and the token equals the host's hook token (constant-time compare, length must match), and the `X-Nodus-Session` header (read case-insensitively) names a session the host is running. Unauthorized or unknown session → `401` with an empty body (for any method); the request body is drained.
- **TOOL-006** Authorized, known session, method `DELETE` → `204` empty; any other non-`POST` method → `405` with header `Allow: POST`.
- **TOOL-007** The POST body is accumulated as text; once its length exceeds `1048576` (`MCP_MAX_BYTES`) the host answers `413` empty and stops.
- **TOOL-008** A body that isn't valid JSON → `200`, `Content-Type: application/json`, body `{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}`.
- **TOOL-009** The parsed body may be one message or an array (batch). Only items that are truthy objects with an `id` key (any value, including `null`) and a string `method` are requests; if none remain (notifications only) → `202` empty.
- **TOOL-010** Each request is relayed to the app: the newest connected app client (last added to the host's client set; only clients that said hello with the host's `PROTOCOL` are in it) gets `{type:'mcp', id, sessionId, message}` where `id` is a host-wide counter starting at `1`. The matching `{type:'mcp-reply', id, reply}` settles it. Timeout `20000` ms (`RELAY_TIMEOUT_MS`).
- **TOOL-011** No client, timeout, or a reply of `undefined` → the host's own answer, with `APP_CLOSED` = `Nodus isn’t open, so the board can’t be reached right now.`: for `tools/call` → `{"jsonrpc":"2.0","id":<id>,"result":{"content":[{"type":"text","text":APP_CLOSED}],"isError":true}}`; for any other method → `{"jsonrpc":"2.0","id":<id>,"error":{"code":-32603,"message":APP_CLOSED}}`.
- **TOOL-012** All requests of a batch are relayed concurrently; the response is `200`, `Content-Type: application/json`, body = the answers array in request order for an array body, or the single answer object for a single body.
- **TOOL-013** An exception while serving `/mcp` → logged `mcp request failed`, `500` if headers weren't sent yet, then the response ends.
- **TOOL-014** App side: on a host `mcp` message the app calls `answerMcp(sessionId, message)` asynchronously (after a microtask), awaits it, logs a rejection as `[board-tools]` (reply becomes `undefined`), and always sends `{type:'mcp-reply', id, reply}`.
- **TOOL-015** Because the server lives in the host, a session keeps its tools across app restarts; sessions started outside the app don't get them; a host started by a version without board tools (welcome lacks `mcp: true`) means new sessions get no `--mcp-config`.

### Board tools: JSON-RPC methods

- **TOOL-016** Every response has `jsonrpc: "2.0"` and `id` echoed from the request (`undefined` id → key omitted).
- **TOOL-017** `initialize` → result `{protocolVersion, capabilities: {tools: {}}, serverInfo: {name: "nodus", version: <app version, e.g. "0.9.4">}, instructions: INSTRUCTIONS}`; `protocolVersion` echoes `params.protocolVersion` when it is a string, else `"2025-06-18"`.
- **TOOL-018** `ping` → result `{}`.
- **TOOL-019** `tools/list` → result `{tools: [...]}`: the 6 common tools, the 4 drawing tools (TOOL-101, added after 0.9.4) then the 4 view tools (order in Data & formats) for every session; action runs (the session's node on its board has a truthy `runOf`) additionally get `list_sessions`, `archive_sessions` at the end. No pagination cursor.
- **TOOL-020** Any other method → `error: {code: -32601, message: "Unknown method <method>"}` (e.g. `Unknown method resources/list`).
- **TOOL-021** `tools/call` answers only once the tool's handler finished (view tools wait for the page check, VIEW-108).
- **TOOL-022** `tools/call` arguments = `params.arguments` when it is a non-null object (arrays included), else `{}`.
- **TOOL-023** A successful tool call's result is `{content: [{type: "text", text: JSON.stringify(value, null, 2)}]}` (2-space pretty JSON, no `isError` key).
- **TOOL-024** A failing tool call's result is `{content: [{type: "text", text: <error message>}], isError: true}` (a JSON-RPC result, never a JSON-RPC error). Non-`Error` throws use `String(err)`.
- **TOOL-025** Checks run in this order before dispatch: handlers not configured → `The board is not available`; the session isn't a node on any board → `This session is no longer on a Nodus board`; tool is `list_sessions`/`archive_sessions` and the session isn't an action run → `<name> is only for action runs` (a run the user took over is no longer a run). Unknown tool name → `Unknown tool <name>` (`String(params.name)`, so a missing name gives `Unknown tool undefined`).
- **TOOL-026** Note patches take `text` only when it is a string and `color` only when it is one of `FOLDER_COLORS` (`blue`, `purple`, `pink`, `red`, `orange`, `yellow`, `green`, `graphite`); any other colour is dropped silently (no error).
- **TOOL-027** The note tools work on the caller's own board when `project` is `undefined` (absent); any other value (including `null` and `""`) goes through project lookup (TOOL-028).
- **TOOL-028** Project lookup (`findProject`): `ref` must be a string non-empty after trim, else `project must be a project’s id or name, from list_projects`; an exact id match wins; otherwise projects (archived included) whose `name.toLowerCase()` equals the trimmed ref lowercased: exactly one → its id; several → `More than one project is called “<ref>”; use its id from list_projects`; none → `There’s no project called “<ref>”; list_projects lists them`. Nothing is written on failure.
- **TOOL-029** `create_note` → `createNote(target, caller, patch, away)` (NOTE-*), then `ev:board-changed(target)`; returns `{"noteId": "<id>"}`. `away` is set when the caller's own project (by `projectIdForSession`) exists and differs from the target: `{project: <own project id>, title: sessionTitle(own, caller)}`.
- **TOOL-030** `sessionTitle(projectId, sessionId)`: a view's brain → its view's title, or `A view` if empty/missing; otherwise the node's `title` || the transcript meta title || (`Terminal` for a terminal node) || `New session`.
- **TOOL-031** `update_note`: `note_id` must be a string, else `note_id is required` (checked before the project is resolved); then `updateNote(target, note_id, patch)`, `ev:board-changed(target)`; returns `{"noteId": "<note_id>"}`.
- **TOOL-032** `list_notes` → `listNotes(target, caller)` (NOTE-016).
- **TOOL-033** `list_projects` → every project in `state.projects` order as `{id, name, folder: repoPath, archived?: true, thisSessionsBoard?: true}` (`archived` only when archived; `thisSessionsBoard` only for the caller's project).
  - After 1.0.12, server projects linked into the grid (01 `Project.server`) are left out here and can't be named as a tool's `project`: their boards are on their servers (SRV-160).
- **TOOL-034** `create_project`: `path` must be a string non-empty after trim, else `path is required`; a leading `~` followed by end, `/` or `\` is replaced by the home directory (regex `^~(?=$|[\\/])`); the result must be absolute, else `path must be absolute`; if it doesn't exist it is created recursively (`madeFolder`); then `addProject(target, name if string)` (01/08: realpath, `Folder not found: <folder>`, `Not a folder: <real>`, existing project by realpath is returned and un-archived/joins focus, new project gets uuid, `name.trim()` || basename, least-used folder colour, `focused` when focusing); then the app emits `ev:projects-changed`. Returns `{projectId, name, folder: repoPath, madeFolder?: true, alreadyAProject?: true}`.
- **TOOL-035** `list_sessions` (runs only) → `sessionList(project, caller, include_archived === true)`: the board's cards except the caller and views' brains, archived ones only when asked, each `{id, title, kind, status, running, section?, actionRun?: true, branch?, lastReply?, lastActive}` where `title` = node title || meta title || `Terminal`/`New session`, `kind` = `claude` default, `status` = `archived` | `waiting for the user` | `working` | `done`, `section` = section name or `Untitled section`, `lastActive` = ISO string of meta `updatedAt` or node `createdAt`. A non-boolean `include_archived` (e.g. `"yes"`) means `false`.
- **TOOL-036** `archive_sessions` (runs only): `session_ids` keeps only string items (non-array → `[]`); `force` is `true` only for boolean `true`. For each id in order, the first matching skip reason: not an own property of the board's nodes → `not on the board`; the caller → `it is the session running this action`; a view's brain → `it is a view’s brain, which goes with its view`; terminal → `terminals can’t be archived`; archived → `already archived`; status not `done` and not forced → `it is waiting for the user` (status `waiting`) or `it is working`. Otherwise a session nothing was said in (`isEmpty`) is removed (`dropNode`: stickers fall off, process killed, status forgotten) and listed in `removed`; else archived (`archiveNode`: kill, slot 0 of the archive, `archived`, `archivedAt`, drops section and expanded) and listed in `archived`. If anything changed: save + `ev:board-changed`. Returns `{archived: [...], removed: [...], skipped: [{id, reason}]}`.
- **TOOL-037** `archive_self` errors, checked in order: node missing → `This session is no longer on the board`; terminal → `Terminals can’t be archived`; archived → `This session is already archived`; brain → `A view’s brain stays with its view; deleting the view stops it`; action run → `An action run leaves the board by itself when its turn ends`. Otherwise the session is remembered (in memory only) and the result is `{"archiving": "This session moves to the archive when this turn ends."}`.
- **TOOL-038** On the next status `done` for a remembered session (status hook path in ipc.ts, after `finishRun`), it is forgotten and, if its node still exists and isn't archived, archived with `archiveNode`, saved, and `ev:board-changed` sent. Remembered sessions are lost on app restart.
- **TOOL-039** `list_views`, `get_view`, `create_view`, `update_view` call the views handlers (VIEW-102–VIEW-114) with the caller's own project; there is no `project` argument for views.
- **TOOL-040** Claude cannot delete notes or views; there is no tool for it.

### Board tools: drawing, shapes and links (added after 0.9.4)

0.9.4 has none of these. They work on the shapes and links the user draws on the board (rect, ellipse, diamond, text, sticker and image shapes; arrows between cards, notes, views and shapes), kept in `boards[<projectId>].shapes` and `.links`. Code: `nodus-tools` `src/drawing.rs`; placement helpers in `nodus-board` `shapes.rs` (`drawn_size`, `drawing_obstacles`, `place_drawing`).

- **TOOL-101** Every session (runs included) gets four more tools, listed after `archive_self` and before `create_view`, with their allow rules in the same place in `BOARD_TOOL_RULES`: `list_board`, `draw_on_board`, `update_drawing`, `erase_from_board`, for the shapes and links of spec 14. They work on the caller's own board (no `project` argument; one given is ignored) and get `arguments` as given (TOOL-022). The descriptions of the three that change the board say to use them only when the user asks for something drawn, laid out, rearranged, linked or taken away.
- **TOOL-102** `list_board` → `{linked_to_this_session, cards, notes, views, shapes, links}`, every box `{x, y, w, h}` rounded (`Math.round`), in board units with `y` growing down. `cards`: the active cards (not archived, not a run, not a view's brain) in board order, `{id: <session id>, title, status, x, y, w, h, this_session?: true}`, with `title` and `status` as `list_sessions` gives them (TOOL-035) and the card's size as drawn (SESS-197). `notes`: free notes (not stuck to a card), oldest first, `{id, text, color, x, y, w, h}`, `text` cut to 200 UTF-16 units with `…`. `views`: oldest first, `{id, title, x, y, w, h}`. `shapes`: in board order, `{id, kind, text?, color, x, y, w, h, yours?: true}` (`yours` when the caller drew it). `links`: only those drawn, each end loose or on something in the lists above (a link to an archived card waits, hidden, as the board does), `{id, from, to, label?, color, yours?: true}`, an end being `{id, kind: card|note|view|shape, name}` or a loose `{x, y}`; `name` is a card's title, a note's first non-blank line (60 units, `…`), a view's title (`A view` when blank), or a shape's words (60 units) or else its kind. `linked_to_this_session`: one line per drawn link with an end on the caller's card, in board order: `→ <kind> “<name>”` for one that starts there, `← <kind> “<name>”` for one that ends there, `a loose end at (x, y)` for a loose other end, and ` (label: <label>)` when it has one; for example `["→ card “Fix cart rounding” (label: blocks)", "← note “Release checklist”"]`. Nothing is written and nothing is sent anywhere.
- **TOOL-103** `draw_on_board {shapes?, links?, at?}`. Everything is checked before anything is drawn; the first problem is the error and the board is left as it was. `shapes` and `links` must be lists when given (`shapes must be a list`); both empty → `Nothing to draw: give shapes, links or both`. Each shape (`shapes[i]: …`): an object; `kind` one of `rect`, `ellipse`, `diamond`, `text`, `sticker` (`image` → `pictures can’t be drawn with the board tools; kind must be rect, ellipse, diamond, text or sticker`); `key` optional, non-blank, not `self`, and no two shapes may go by the same name (a shape without a key goes by its index, `"0"`, `"1"`…); `text` a string, non-blank for `text` (`a text shape needs some text`) and `sticker` (`a sticker needs its emoji in text`); `color` one of the ink colours (`color must be graphite, red, orange, green, blue or purple`), default `graphite`; `x`, `y`, `w`, `h` finite numbers when given (`x must be a number`), `x` and `y` both or neither. `at`, when given, `{x, y}` finite. The board keeps at most 2000 shapes and 2000 links (`The board keeps at most 2000 shapes, and has <n>`).
- **TOOL-104** Shape sizes default by kind: rect and ellipse 180 × 90, diamond 160 × 110, text 220 × 44, sticker 64 × 64; any size is rounded into 16–4000. Positions are relative to the drawing's top left. Shapes without `x`/`y` go in a row, left to right, 64 apart, lined up on their middles (`y` offset rounded), starting at `(0, 0)`, or, when other shapes were placed, at the placed shapes' left edge, 64 under their lowest bottom. With `at`, each shape goes at `at + (x, y)`. Without it the drawing's bounding box goes where `findSpotBeside(anchor, box size, obstacles, dockRect(board.archive), 32)` puts it (SESS-202): the anchor is the caller's active card, else its view for a view's brain, else the rightmost active card (NOTE-006 (4)); with none, `findFreeSpot` right of the dock (`dock.x + dock.w + 80, dock.y`). Obstacles are a note's (NOTE-005) plus every shape. Positions are rounded.
- **TOOL-105** A link's `from` and `to` (`links[i]: …`) are, in this order: a shape name from this call; `self`, the caller's active card (`this session has no card on the board to link` when it has none: a run, an archived card, a brain); or the id of a card (session id), free note, view or shape that's drawn on the board. Something on the board but not drawn → `“<id>” isn’t drawn on the board right now (an archived card, an action run or a note stuck to a card can’t be linked)`; anything else → `“<id>” isn’t a key from shapes or the id of anything on this board; list_board lists what’s there`; missing → `from must be a key from shapes, the id of something on the board, or “self”`; the same thing at both ends → `a link needs two different ends`. `label` a string (trimmed, cut to 200 units), `color` as for shapes.
- **TOOL-106** The shapes and links are saved through `shapes::save_shapes` and `save_links` with new uuid ids, `by: <caller>` and `createdAt: now`; an attached end's `x`/`y` is its thing's middle. Then `ev:board-changed(project)`. Returns `{shapes: {<name>: <id>}, links: [<id>…], area?: {x, y, w, h}}`, `area` being where the shapes went.
- **TOOL-107** `update_drawing {items?, links?}`, checked whole before anything changes (both empty → `Nothing to change: give items, links or both`). Each item (`items[i]: …`) needs a string `id` (`id is required`). A shape takes `x`, `y`, `w`, `h`, `text`, `color` (checked as in TOOL-103; at least one, else `nothing to change; give x, y, w, h, text or color`), keeping its kind, who drew it and when. A card (session id), free note or view only moves: `x` and/or `y` (`nothing to change; give x, y or both`); `w`, `h`, `text` or `color` → `“<id>” is a card, which update_drawing only moves, so it takes x and y but not <key>` (a note adds `; update_note changes its text and colour`, a view `; update_view changes its size and title`). A card moves as `board:save-nodes` moves it (SESS-092: finite `x`/`y` set, nothing else touched, nothing archived); a note's and a view's position is rounded (SESS-103, VIEW-116). One that isn't drawn → `“<id>” isn’t drawn on the board right now, so it can’t be moved`; a link's id → `“<id>” is a link; change it under links`; else `there’s nothing with the id “<id>” on this board; list_board lists what’s there`. An id given twice gets both changes, in order.
- **TOOL-108** Each of `update_drawing`'s `links` needs the `id` of a link on the board (`there’s no link with the id “<id>” on this board; list_board lists them`) and one of `label` (trimmed, 200 units; `""` takes it off), `color`, `from`, `to`, the ends given as in TOOL-105 without shape names. Both ends on the same thing → `a link needs two different ends`. Then save, `ev:board-changed(project)`, and `{updated: [<id>…]}`, each id once, in order.
- **TOOL-109** `erase_from_board {ids}`: `ids` must be a list (`ids must be a list of the ids of shapes and links, from list_board`); its strings are taken, and none → `Nothing to erase: give the ids of shapes and links, from list_board`. Shapes and links go, and with each shape the links it held; anything else is skipped with a reason: `it is a card; only shapes and links can be erased` (likewise `a note`, `a view`), or `not on the board`. Returns `{erased: [shape ids, link ids given, then links that went with the shapes], skipped: [{id, reason}]}`; `ev:board-changed(project)` only when something went. Cards, notes and views are never removed (TOOL-040).
- **TOOL-110** `INSTRUCTIONS` ends, after 0.9.4's text, with a few sentences on shapes and links: that the user links cards to show how their work relates, that `list_board` shows the board and what this session's card is linked to, and to use the drawing tools when the user asks for something drawn, laid out, rearranged or linked (verbatim under Data & formats).

### Board tools: seeing the board, pictures and ink (added after 1.0.17)

Feedback #9 asked for Claude to understand the board visually and add to it. `look_at_board` shows the board as a picture, drawn from what's stored rather than taken from the window; `draw_on_board` puts up pictures from files and draws ink; and a session may draw on its own when it helps. Code: `nodus-tools` `src/look.rs` (the picture), `src/drawing.rs` (pictures and marks), `src/handlers.rs` (files); `nodus-board` `pictures.rs` (a picture's kind and size from its header).

- **TOOL-111** `look_at_board {area?, around?, margin?}`, listed after `list_board`, its allow rule after `list_board`'s. It answers with two content blocks: `{type: "image", data: <base64 PNG>, mimeType: "image/png"}`, then a text block, the key, pretty JSON `{area: {x, y, w, h}, scale, tags: {<tag>: {id, kind, name?, this_session?: true}}, untagged_ink?}`.
  - The area: `area` as given (finite `x`, `y`, `w`, `h`, `w` and `h` above 0; `area must be {"x": …, "y": …, "w": …, "h": …} in board coordinates as list_board gives them, w and h above 0`); or the box around what `around` names (an id, `"self"`, or a list of them; a card, note, view or shape drawn on the board, an ink stroke or a frame), grown by `margin` (0–4000, default 240) each way; or, with neither, everything on the board grown by 80, or `0, 0, 1200 × 800` for an empty board. Both → `give area or around, not both`; an `around` that isn't drawn → `“<id>” isn’t drawn on this board; list_board lists what’s there`; `"self"` with no card → `this session has no card on the board to look around`; a bad margin → `margin must be a number of board units, 0 or more`.
  - The picture: the area at a scale of `min(1568 / its longer side, 2)` pixels per board unit (`scale`, to 3 places), so its longer side is at most 1568 px. The light board's colours, whatever the app's theme. In order: frames (dashed, their names), shapes but stickers, views (a header with the title, then `A view: a live page.` and its description), the other active cards (title, status dot and word, the last reply), free notes (their colour's light paper, their text), stickers, links (straight arrows between the boxes' edges, the label in a pill), ink as drawn, then the caller's own card as a dashed accent outline with `this session (you)`, so what lies under it shows. Text never comes out under 9 px; it wraps and ends in … where it doesn't fit. Pictures are their files, shown whole; one bigger than 8000 px either way, or missing, is a grey box saying `picture`.
  - Tags: each thing in the area gets one, numbered by kind in that order: `c` cards (the caller's last), `n` notes, `v` views, `s` shapes and stickers, `p` pictures, `l` links, `i` ink (the newest 60 in the area; `untagged_ink` counts the rest), `f` frames. A tag is a 14 px pill, white on dark (accent for the caller's card): above a box's top left, below a link's label or middle, left of where a stroke starts; moved right off another tag.
  - Text uses the system's fonts, read once. Nothing else is read but the board's pictures, by name, from the pictures folders (TOOL-112), each a plain file of at most 25 MB.
- **TOOL-112** `draw_on_board`'s shapes may be `image`, with `path`: a PNG, JPEG, GIF or WebP file, by its absolute path (a leading `~` is the home folder). Before anything is drawn, each is checked and copied into the board's pictures (`board-images`) as `<uuid>.<png|jpg|gif|webp>`, its kind read from its first bytes. Errors: `shapes[i]: a picture needs path, its file’s absolute path`, `shapes[i]: path must be absolute`, `shapes[i]: there’s no file at “<path>”`, `shapes[i]: “<path>” isn’t a file`, `shapes[i]: “<path>” is <n> MB; a picture can be at most 25 MB`, `shapes[i]: “<path>” isn’t a PNG, JPEG, GIF or WebP picture`. It goes up at its own proportions, as SHAPE-006's do at zoom 1 (its longer side at most 480, never bigger than it is); a `w` or an `h` alone sets that side and the other follows; both set both. When the drawing fails after the copies, they're deleted. On a server board → `Pictures can’t be put up from a file on a Nodus server’s board yet`: the session's files aren't the server's to read. Shapes' `kind` error is now `kind must be rect, ellipse, diamond, text, sticker or image`.
- **TOOL-113** Ink. `draw_on_board` takes `ink`, a list of marks, checked with the rest (`ink[i]: …`); with no shapes, links or ink → `Nothing to draw: give shapes, links or ink`. A board keeps at most 2000 strokes (`The board keeps at most 2000 ink strokes, and has <n>`). Each mark: `kind` one of `pen`, `arrow`, `rect`, `text`, `circle`, `underline` (`kind must be pen, arrow, rect, text, circle, underline`); `color` an ink colour (default graphite); `size` 0.5–4 (default 1; `size must be between 0.5 and 4`). What a mark marks is a key from this call's shapes, `"self"`, or the id of a card, note, view, shape, ink stroke or frame drawn on the board (`“<name>” isn’t a key from shapes or the id of anything drawn on this board; list_board lists what’s there`).
  - `circle` `around` one or a list: a pen stroke, an ellipse 12% wider than their box plus `14 + 2·size`, a little uneven, its end running past its start (49 points).
  - `underline` `under` one or a list: a pen stroke of three points under their box.
  - `rect` `around`: an ink rectangle around their box.
  - `arrow` `from` and `to`, each something marked or a point (`{"x", "y"}` or `[x, y]`): from the edge of one toward the other, stopping short of it (`an arrow needs from and to`; ends under 8 apart → `the arrow’s ends are too close together to draw`).
  - `text` with `text` (trimmed, at most 500 units) at `x` and `y`, or `beside` something, to its right (`a text mark needs some text`, `a text mark needs x and y, or beside`).
  - `pen` through `points`, 2 to 2000, each `[x, y]` or `{"x", "y"}`.
  - Positions are board coordinates; `at` doesn't move ink. Strokes are cleaned as `cleanInk` cleans them and added on top (`board:add-ink`'s way), with new ids and the session in the stroke's `by` (kept among the fields older versions don't know, and through cleaning, up to 64 units; on a server, `createdBy` is the session's starter's). The result adds `ink: [ids]`.
  - `list_board` adds `ink` when there is some: the newest 300 strokes, oldest first, each `{id, kind, color, x, y, w, h, text?, from?, to?, around?, yours?: true}`: a label's words; for an arrow, what's under each end (within 24, the smallest there, `{id, kind, name}`), else the point; for a stroke or a rectangle, the smaller things whose middles are inside its box (at most 8). `ink_not_listed` counts the strokes left out.
  - `update_drawing` given an ink id → `“<id>” is ink, which update_drawing doesn’t change; erase it and draw it again`. `erase_from_board` erases ink by id too, with shapes and links; its messages say `shapes, links and ink`.
- **TOOL-114** Drawing unasked. The tools' descriptions and `INSTRUCTIONS` say to draw when the user asks, and on one's own when a small sketch, a picture one made or a mark on what one means would say it better than words: small, near one's card, saying what was drawn. `update_drawing` and `erase_from_board` change and take away one's own drawings, and the user's only when they ask. Notes may be written unasked too, sparingly. `INSTRUCTIONS` (verbatim under Data & formats) name `look_at_board` for when the user points at something on the board.

### Notes placement (notes Claude writes)

- **NOTE-001** Note text is `text.trim()` then its first `20000` UTF-16 code units (`NOTE_MAX_CHARS`); a missing or blank text throws `A note needs some text`.
- **NOTE-002** `fitNote(text)`: `lines = text.split('\n')`; `longest` = max line length (UTF-16 units); `w = clamp(longest × 7.4 + 36, 216, 360)`; `perLine = max(1, floor(((w − 36) / 7.4) × 0.9))`; `rows = Σ max(1, ceil(len(line) / perLine))`; `h = clamp(rows × 21 + 32 + 22, 216, 480)`. `clamp` rounds with JS `Math.round` after clamping. (Constants: `CHAR_W = 7.4`, `PAD_X = 36`, `LINE_H = 21`, `PAD_Y = 32`, `FROM_H = 22`, `FIT_MAX_W = 360`, `FIT_MAX_H = 480`, min = `NOTE_W = NOTE_H = 216`.)
- **NOTE-003** A short note stays square at 216 × 216: `w` is stored only when `≠ 216`, `h` only when `≠ 216` (otherwise the keys are absent). `'word '.repeat(400)` gives `{w: 360, h: 480}`.
- **NOTE-004** Colour defaults to `yellow`.
- **NOTE-005** Obstacles for placement: active cards (not archived, no `runOf`, no `viewOf`) at `cardSize`, free notes (no `sessionId`, i.e. not stickers) at `noteSize`, and every view at `viewSize`. The archive dock (`dockRect(board.archive)`) is avoided.
- **NOTE-006** Preferred spot, first that applies: (1) under the newest (`createdAt`) free note whose `from` is the writer: `{x: last.x, y: last.y + noteSize(last).h + 32}`; (2) if the writer is a view's brain whose view exists: right of the view `{x: view.x + viewSize(view).w + 32, y: view.y}`; (3) right of the writer's active card `{x: card.x + cardSize(card).w + 32, y: card.y}`; (4) right of the rightmost active card (largest `x`; on ties the earlier in board insertion order wins); (5) `{x: dock.x + dock.w + 80, y: dock.y}`.
- **NOTE-007** The final position is `findFreeSpot(preferred, obstacles, dock, size)` (geometry, 03/09) with the fitted size.
- **NOTE-008** A run, an archived card and a session on another project have no active card on the target board, so their first note goes beside the rightmost card (NOTE-006 (4)); later ones stack under it (NOTE-006 (1)).
- **NOTE-009** The note record: `{id: <uuid v4>, text, color, x, y, w?, h?, from: <writer session id>, fromProject?: <writer's project id>, fromTitle?: <writer's title at write time>, createdAt: Date.now()}`; `fromProject`/`fromTitle` only when written from another project's board.
- **NOTE-010** Creating a note writes it into `board.notes[id]` and saves `state.json`.
- **NOTE-011** `updateNote` finds the note only as an own property of the board's notes (ids like `__proto__`, `constructor`, `toString`, `hasOwnProperty` are not found), else `There’s no note with that id on this board`.
- **NOTE-012** `updateNote` with `text !== undefined`: the new text goes through NOTE-001; blank throws `A note needs some text` before anything changes (the colour is not applied either).
- **NOTE-013** `updateNote` sets `color` only when given (and valid, TOOL-026); `x`, `y`, `w`, `h` (possibly the user's size) are kept. Then saves.
- **NOTE-014** An empty patch changes nothing but still saves and succeeds.
- **NOTE-015** `clampNoteSize(n) = Math.round(min(1200, max(120, n)))`, used by `board:save-note` (10) for user resizes.
- **NOTE-016** `listNotes(project, self)`: every note on the board (stickers included), oldest `createdAt` first, as `{id, text, color, writtenByYou?: true}` (`true` when `from === self`, else key absent).

### Ink storage

- **INK-001** `cleanInk(raw)`: a non-array gives `[]`.
- **INK-002** Only the last `2000` items (`INK_MAX_STROKES`) of the array are considered (`raw.slice(-2000)`), before any filtering.
- **INK-003** An item that is falsy or not an object is dropped.
- **INK-004** `id` must be a non-empty string of at most 64 UTF-16 units and not seen earlier in this pass (first occurrence wins); otherwise dropped.
- **INK-005** `kind` must be one of `pen`, `arrow`, `text`, `rect` (the rectangle added after 0.9.4, spec 11 INKUI-027) and `color` one of `graphite`, `red`, `orange`, `green`, `blue`, `purple`; otherwise dropped.
- **INK-006** `points` must be an array whose every element is a finite number (no `NaN`, no strings); otherwise dropped.
- **INK-007** `points` needs length ≥ 2 and even; a `pen` keeps its first `4000` numbers (`INK_MAX_POINTS × 2`); an `arrow` and a `rect` need exactly 4; a `text` exactly 2; otherwise dropped.
- **INK-008** A `text` stroke's `text` is `trim()`ed and cut to `500` UTF-16 units (`INK_MAX_TEXT`); missing/blank → dropped. Other kinds never keep a `text` key.
- **INK-009** Every kept point is rounded to one decimal: `Math.round(n × 10) / 10` (JS rounding: halves go toward +∞).
- **INK-010** `size`: a finite number → `Math.round(min(10, max(0.25, n)) × 100) / 100`; anything else (missing, non-number) → `1`.
- **INK-011** Output objects are `{id, kind, color, points, size}` plus `text` only for `text` strokes, in that key order, in input order.
- **INK-012** `board:save-ink(projectId, strokes)` replaces `board.ink` with `cleanInk(strokes)` and saves (the ink UI is 11's).
- **INK-013** `board:add-ink(projectId, strokes)` and `board:remove-ink(projectId, ids)` (hosted Nodus, added after 1.0.12; SRV-184): a server board's ink a stroke at a time. Adding cleans the strokes as `cleanInk` does, puts each one with an id already there in its place and the rest on top, then keeps the last 2000 (INK_MAX_STROKES); removing drops those ids. Both leave every other stroke as it is, so two people drawing at once keep both. A new stroke records who drew it (`createdBy`, SRV-183). The app sends these for server boards (INKUI-028); local boards keep `board:save-ink`.

### Views: storage

- **VIEW-001** View ids are lowercase UUID v4. Any id that ends up in a path must match `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`, else `Not a view id` is thrown.
- **VIEW-002** A view's files live in `<userData>/views/<viewId>/`: `index.html` (the page exactly as Claude sent it, UTF-8) and `runtime.json`; never in the repository. `<userData>` is the app's data folder (honours `NODUS_USER_DATA`, 01).
- **VIEW-003** Reading `index.html` returns `''` on any error (missing file, bad id).
- **VIEW-004** Writing `index.html` creates the folder recursively first.
- **VIEW-005** Runtime (data, state, command results, errors, `refreshedAt`) is cached in memory per view; the first access reads `runtime.json`: `results` kept only if a truthy object (else `{}`), `errors` only if an array (else `[]`), `refreshedAt` only if a number; an unreadable/invalid file gives `{results: {}, errors: []}`.
- **VIEW-006** Runtime saves are debounced per view: each save request restarts a `500` ms timer; on fire the runtime is written as compact JSON to `runtime.json.<pid>.tmp` in the folder (created recursively) and renamed over `runtime.json`. A failure logs `[views] save failed`.
- **VIEW-007** On quit (`shutdown`) every pending runtime is written immediately (failure logs `[views] flush failed`), timers cleared, and the 30 s ticker stopped.
- **VIEW-008** Deleting a view's files cancels its pending runtime write, drops its cache and removes the folder recursively (`force`); a failure logs `[views] remove failed`.

### Views: validation of what Claude and the renderer send

- **VIEW-009** `clip(s, max)`: a non-string gives `''`; a string is `trim()`ed then cut to its first `max` UTF-16 units.
- **VIEW-010** Limits: page ≤ `2 × 1024 × 1024` UTF-8 bytes; data JSON ≤ `1024 × 1024` chars; state JSON ≤ `256 × 1024` chars; title `80`; description `4000`; refresh/ask prompt `4000`; commands `12`; command name `40`; command line `2000`; stdout `1024 × 1024` chars; stderr `64 × 1024` chars; kept errors `20`; remembered command lines `50`.
- **VIEW-011** `clampEvery(value, min)`: a finite number `> 0` → `min(10080, max(min, Math.round(value)))` (10080 = a week in minutes); anything else (missing, `0`, negative, non-number) → `undefined`. Commands use `min = 1`, refreshes `min = 10`.
- **VIEW-012** `normalizeCommands(raw, remembered = {})`: non-array → `commands must be a list of { name, run, every }`; more than 12 → `A view can have at most 12 commands`; each item: `name = clip(item.name, 40)`, `run = clip(item.run, 2000)`, either empty → `Each command needs a name and a run (its command line)`; a repeated name → `There are two commands called “<name>”`; output `{name, run, every: clampEvery(item.every, 1), approved: run if remembered.allowedRuns includes run exactly, denied: run if remembered.deniedRuns includes run exactly}` (undefined keys absent in JSON).
- **VIEW-013** `normalizeRefresh(raw)`: `null`/`undefined` → `undefined`; else `prompt = clip(raw.prompt, 4000)`, `every = clampEvery(raw.every, 10)`; either missing → `refresh needs every (minutes) and a prompt, or null to stop refreshing`; result `{every, prompt}`.
- **VIEW-014** `checkJson(value, max, what)`: `JSON.stringify` throws (cycle, BigInt) → `<what> must be JSON`; the JSON text's length > max → `<what> is too big (over <Math.round(max/1024)> KB)` (data: `The data is too big (over 1024 KB)`, state: `The state is too big (over 256 KB)`); `undefined` passes.
- **VIEW-015** `checkHtml(html)`: not a string or blank after trim → `html must be the page, as one HTML document`; UTF-8 byte length > 2 MiB → `The page is too big (over 2 MB)`. The page is stored untrimmed.
- **VIEW-016** `findView(projectId, id)`: the id must be a string that is an own property of that board's views, else `There’s no view with that id on this board` (so `__proto__` is not found).
- **VIEW-017** A command is *waiting* when `approved !== run && denied !== run`; *allowed* when `approved === run`; *turned down* when `denied === run`.

### Views: serving the page

- **VIEW-018** The scheme is `nodus-view` (`VIEW_SCHEME`), registered before app ready as privileged `{standard: true, secure: true}`, and every request to it is answered by `servePage(url)`.
- **VIEW-019** `servePage`: the URL's hostname is the view id and the pathname must be exactly `/` (query ignored, so `?v=<version>` is fine); the view must exist on some board (`projectIdForView`, own-property check over all boards); otherwise `404` with body `Not found`. `nodus-view://<id>/../state.json` normalizes to a non-`/` path → 404.
- **VIEW-020** A served page is `200` with headers exactly: `Content-Type: text/html; charset=utf-8`, `Content-Security-Policy: <VIEW_CSP>`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`.
  - After 1.0.12, a view on a server board (hosted Nodus) is served from its server, never the local core: the frame's request asks `GET /api/files/<viewId>` and keeps the page's status, body and these four headers, waiting at most 3 s; while the server can't be reached it serves the last page it got, else `503` `The server can’t be reached.` The page's bridge calls (`views:call` and the rest) go to that server too (SRV-163).
- **VIEW-021** The body is `buildViewPage(index.html || EMPTY_PAGE, boot)` where `boot = {viewId, title, startsSessions, data, state, results}` from the view's current title/`startsSessions` and its runtime (undefined keys absent). `EMPTY_PAGE` = `<!doctype html><html><head></head><body></body></html>`.
- **VIEW-022** `buildViewPage` inserts the injected block (VIEW-023) right after the first `<head …>` tag (`/<head(\s[^>]*)?>/i`, so `<header>` doesn't match); else, wraps it in `<head>…</head>` right after the first `<html …>` tag (`/<html(\s[^>]*)?>/i`); else, wraps it in `<head>…</head>` right after the doctype (`/<!doctype[^>]*>/i`); else returns `<!doctype html><html><head>INJECTED</head><body>HTML</body></html>`. In the first two cases, a page with no doctype gets `<!doctype html>` prepended. Nothing is ever put before an existing doctype (case kept, e.g. `<!DOCTYPE html><head>…`).
- **VIEW-023** The injected block is the concatenation, with no separators: `<meta charset="utf-8">` + `<meta http-equiv="Content-Security-Policy" content="<VIEW_CSP>">` + `<meta name="viewport" content="width=device-width, initial-scale=1">` + `<style><THEME></style>` + `<script><BRIDGE with __BOOT__ replaced by scriptJson(boot)></script>`. The replacement is literal (no `$` pattern expansion) and only the first `__BOOT__`.
- **VIEW-024** `scriptJson(value)` = `JSON.stringify(value) ?? 'null'` with every `<` replaced by `<`, U+2028 by ` `, U+2029 by ` ` (the six-character escapes). `scriptJson(undefined)` = `null`.
- **VIEW-025** The CSP (`VIEW_CSP`, verbatim in Data & formats) allows inline scripts and styles only, images/media from `data:`/`blob:`, fonts from `data:`, and nothing else: no network, no frames, no workers, no form submission, no `<base>`. It contains no `http:`/`https:`.
- **VIEW-026** The injected theme (Data & formats) defines `--nodus-*` colours for light and, under `prefers-color-scheme: dark`, dark; `html, body` get margin 0, background `var(--nodus-bg)`, colour `var(--nodus-text)`, `13px/1.45 var(--nodus-font)`, antialiased; links `var(--nodus-accent)`. All with `:where()` (zero specificity) so the page's CSS wins. The colour scheme follows the app's theme setting (System/Light/Dark), not only the OS. Since 1.0.15 (`crates/nodus-views/src/page/theme.css`) it is Nodus's design system for pages, so a page Claude writes looks and moves like the app: the app's tokens as variables (`--nodus-pressed`, `--selected`, `--border-strong`, `--accent-hover`, `--on-accent`, `--focus`, the statuses `--working`, `--waiting` and `--done`, `--radius`, and the motion of MOTION-036); buttons, inputs, selects, textareas, checkboxes, progress bars, tables, headings, paragraphs, code, `hr`, the focus ring, the selection and scrollbars drawn as the app draws them (controls 26px tall with 6px corners on `--nodus-surface`, `--nodus-hover` under the pointer, `--nodus-pressed` held; fields with a hairline that turns accent with a 3px focus ring); `nodus-primary` (accent-filled) and `nodus-ghost` buttons; `nodus-dot` with `is-working`, `is-waiting` and `is-done`, the board's status dot breathing and pulsing as CARD-012…CARD-014; and Claude's look (MOTION-037). Still all with `:where()`, so the page's CSS wins. 0.9.4's theme had the colours and the body only.

### Views: the frame

- **VIEW-027** A view's page shows in a frame with `sandbox="allow-scripts"` only (opaque origin: no same-origin, no storage/cookies, no popups, no modals such as `alert`, no forms, no top navigation, no downloads), `src = nodus-view://<id>/?v=<version>`, accessible `title` = the view's title, no border, background `--bg-elevated`. A version change reloads the page.
- **VIEW-028** A frame may only navigate to a `nodus-view://` URL, and once it shows a view page, only to that same view id; any other navigation (web, another view) is cancelled. The page cannot reach the board's window.
- **VIEW-029** Each mounted frame registers with the relay under its window: `{projectId, viewId, version}`; it re-registers when the project, view id or version changes and unregisters on unmount. A view can have two frames at once (card and large modal).
- **VIEW-030** The card renders its frame only when `version > 0` and a board project is open; otherwise the empty state (VIEW-137).

### Views: `window.nodus` (the injected bridge, page side)

- **VIEW-031** `window.nodus` has `viewId`, `title`, `canStartSessions` (`Boolean(boot.startsSessions)`), `data`, `state`, `results` (an object, `boot.results || {}`), and methods `on`, `setState`, `run`, `ask`, `sessions`, `notes`, `views`, `view`, `git`, `openSession`, `startSession`, `openUrl`.
- **VIEW-032** A call posts `{id: ++seq, method, args: args || {}, nodus: 1}` to `parent` with target origin `'*'` (`seq` starts at 0 so the first id is 1) and returns a Promise; the reply with the same `id` resolves with `result`, or rejects with `new Error(error)` when `error` is truthy; replies for unknown ids are ignored.
- **VIEW-033** Incoming messages are accepted only when `e.source === parent` and `e.data.nodus === 1`; a message with a truthy `id` is a reply (VIEW-032) and nothing else.
- **VIEW-034** On an event message: `data` → sets current data and `nodus.data`; `state` → sets current state and `nodus.state`; `result` (with a payload) → `results[payload.name] = payload`; then every listener of that event name is called with the payload, iterating over a copy of the listener list; a listener that throws is reported (VIEW-038) and the rest still run. Unknown event names (e.g. `board`, `git`) just call listeners.
- **VIEW-035** `nodus.on(event, fn)` adds `fn` and returns an unsubscribe function. For `data` and `state` it also calls `fn(current value)` once the DOM is ready (on `DOMContentLoaded` if still loading, else in a microtask), only if the current value isn't `undefined` and `fn` is still subscribed; a throw is reported.
- **VIEW-036** `nodus.setState(s)` sets current state and `nodus.state` immediately, then calls `setState` with `{state: s}`.
- **VIEW-037** Method argument shapes: `run(name)` → `{name}`; `ask(prompt)` → `{prompt}`; `sessions()`, `notes()`, `views()`, `git()` → `{}`; `view(id)` → `{id}`; `openSession(id)` → `{id}`; `startSession(prompt, options)` → `{prompt, title: options && options.title}`; `openUrl(url)` → `{url}`.
- **VIEW-038** Errors are reported to the board as `{error: <text>, nodus: 1}` where text = `err.stack || err.message` when either exists, else `String(err)`, cut to 2000 chars; reported on `window` `error` (`e.error || e.message`), `unhandledrejection` (`e.reason`) and listener throws.
- **VIEW-039** On `window` `load` the page posts `{loaded: true, nodus: 1}`.
- **VIEW-040** A click (capture phase, on `document`) whose target is inside an `a[href]`: an `href` attribute starting with `#` is left alone; otherwise the default is prevented and, when the resolved `a.href` matches `/^https?:/i`, `openUrl` is called with it (result ignored); other schemes do nothing.
- **VIEW-041** Wheel routing (listener on `window`, non-passive): ignored if `defaultPrevented` or both deltas are 0. A `ctrlKey` wheel (pinch) always goes to the board. Otherwise, when more than 250 ms passed since the previous wheel event, the gesture is decided afresh: the dominant axis (`|dx| > |dy|` → horizontal) is checked with `scrolls(target, dx|0, 0|dy)`; the board gets it when nothing can scroll that way. Within 250 ms the previous decision sticks. `lastWheel` updates on every non-ignored event.
- **VIEW-042** `scrolls(el, dx, dy)`: walks from the target up to (not including) `<html>`, element nodes only: an element scrolls when its computed `overflowY` (for dy) / `overflowX` (for dx) matches `/auto|scroll/` and it has room: for a negative delta `pos > 0`, else `pos < max − 1` with `max = scrollHeight − clientHeight` (or width). Then the page itself: the root's overflow on that axis, or the body's when the root's is `visible`; `hidden`/`clip` → can't scroll; else room on `documentElement` as above.
- **VIEW-043** A wheel that goes to the board is `preventDefault()`ed in the page and posted as `{wheel: {dx, dy, mode, x, y, ctrl, shift, alt, meta}, nodus: 1}` (`deltaX`, `deltaY`, `deltaMode`, `clientX`, `clientY`, modifier booleans).

### Views: the relay (board side)

- **VIEW-044** The board knows which view a message is from only by the frame window that sent it; messages from unregistered windows, non-objects or without `nodus === 1` are ignored.
- **VIEW-045** `{error: string}` → `views.report(projectId, viewId, {error})` (fire-and-forget; main ignores reports for views that have gone).
- **VIEW-046** `{loaded: truthy}` → `views.report(projectId, viewId, {loaded: <the version the frame was registered with>})`; the page can't choose the version.
- **VIEW-047** `{wheel: object}` → replayed on the frame element as a synthetic, bubbling, cancelable wheel event: deltas/mode from the message (non-finite → 0), `clientX = frameRect.left + x × scale`, `clientY = frameRect.top + y × scale`, `scale = rect.width / offsetWidth` (the board zoom; 1 if the width is 0), modifiers only when exactly `true`. The board then pans/zooms as for any wheel over a card (09).
- **VIEW-048** A request needs a numeric `id` and a string `method`, else ignored. It is answered by posting `{nodus: 1, id, result}` or `{nodus: 1, id, error: reason}` to that frame, `'*'` target.
- **VIEW-049** `openSession`: `args.id` must be a string, else `nodus.openSession needs a session id`; the board opens the card only if it is one of the board's cards (runs/brains excluded; archived cards included, which open as a preview, 09); unknown ids are silently ignored; result `null`.
- **VIEW-050** `startSession`, in order: `prompt` (string, trimmed) empty → `nodus.startSession needs a prompt`; not activated (VIEW-051) → `A view can only start a session within a few seconds of the user clicking something in it`; no board → `The board isn’t open`; then the board's start (VIEW-056–VIEW-058) with `title` trimmed when a string; result `null`.
- **VIEW-051** *Activated* = at the moment the request arrives, the board document's focused element is this frame AND the board window has transient user activation (Chromium's `navigator.userActivation.isActive`; a click or key press inside the frame activates the board window for a few seconds and focuses the frame; a click elsewhere on the board moves focus away). A page can fake neither.
- **VIEW-052** Every other method goes to main as `views.call(projectId, viewId, method, args)` (VIEW-060–VIEW-071).
- **VIEW-053** Error text returned to pages, dialogs and the card is the message with any leading `Error invoking remote method '<channel>': ` and an optional following `Error: ` removed (regex `^Error invoking remote method '[^']+': (Error: )?`).
- **VIEW-054** Main's `ev:view(projectId, viewId, event, payload)` is posted as `{nodus: 1, event, payload}` to every frame showing that view; for `data` the board also posts `{nodus: 1, event: 'board'}` (no payload) to every other view's frames.
- **VIEW-055** The board posts `{event: 'board'}` to all frames 300 ms after its cards, notes or views last changed (trailing debounce; also once 300 ms after the board mounts), and `{event: 'git', payload: <git status>}` to all frames whenever the board's git status changes (and on mount).

### Views: starting sessions from a page (board side)

- **VIEW-056** The view must have `startsSessions`; otherwise, if the view exists, a toast `“<view title>” tried to start a session. Ask its brain if you want it to be able to.` shows, and the call fails with `This view isn’t allowed to start sessions. The user can ask its brain to let it.`
- **VIEW-057** At most one start per view every `3000` ms (`VIEW_START_GAP_MS`, in-memory per board): sooner → `This view just started a session; wait a moment before starting another`. The timestamp is recorded before the session is created (so a failed create still counts).
- **VIEW-058** The new session is an expanded card (720 × 750) placed with `findSpotBeside({x: view.x, y: view.y, ...viewSize(view)}, 720×750, cards + notes + views obstacles, dock, gap 96)` (`VIEW_SESSION_GAP = 3 × GAP`), position rounded; created with `name = title.slice(0, 80) || undefined`, `initialPrompt = prompt`, `expanded: true`, `autoMode` = whether the view is in auto mode (VIEW-155), `fromView: view.id`; it joins the section at that position if any; a yellow sticker note with the prompt text (`id` uuid, at the card's position, `sessionId` = the card) is saved; the board pans to show it (`ensureVisible`).
- **VIEW-059** Main stores `fromView` only for a Claude session whose `fromView` is an own property of that board's views; the board draws an animated dashed edge from the view to it (09).

### Views: what main answers for a page (`views:call` → `callView`)

- **VIEW-060** `callView` first resolves the view on that board (VIEW-016); `args` that isn't a non-null object is treated as `{}`.
- **VIEW-061** The page's rate-limit and other per-view state are keyed by view id.
- **VIEW-062** `sessions` → `sessionList(project, null, false)` (TOOL-035 shape: active cards, no brains).
- **VIEW-063** `notes` → every note, oldest first, as `{id, text, color}`.
- **VIEW-064** `views` → every other view on the board, in board insertion order (not sorted), as `{id, title, description, data}` with `data` from that view's runtime.
- **VIEW-065** `view` → `{id, title, description, data}` of `findView(project, args.id)` (may be the page's own view).
- **VIEW-066** `git` → waits for the login-shell env, then `gitStatus(repoPath)` (05).
- **VIEW-067** `setState` → `checkJson(args.state, 262144, 'The state')`; stores it in the runtime, schedules a save, sends `ev:view(…, 'state', state)`; returns `null`.
- **VIEW-068** `run` → `runCommand(project, view, String(args.name ?? ''))` (VIEW-073).
- **VIEW-069** `ask`: if the previous page ask for this view was less than `15000` ms ago → `Asked too soon: a view can ask its brain once every 15 seconds`; otherwise the time is recorded *before* asking (so a failing ask still starts the 15 s), then `askBrain(…, String(args.prompt ?? ''), 'page')`; returns `null`.
- **VIEW-070** `openUrl`: `args.url` must be a string matching `/^https?:\/\//i`, else `Only http and https links open`; opened in the default browser; returns `null`.
- **VIEW-071** Any other method → `nodus.<method> isn’t something a view can do`.
- **VIEW-072** `openSession`/`startSession` never reach main (the relay answers them); if they did, VIEW-071 applies.

### Views: commands

- **VIEW-073** `runCommand(project, view, name)`: unknown view → throws synchronously (VIEW-016); no command with that name → rejects `This view has no command called “<name>”`; turned down → `The user didn’t allow “<name>” to run`; otherwise not allowed → `“<name>” is waiting for the user to allow it on the view`.
- **VIEW-074** While a run of the same view + command name is going, another request gets the same promise (no second process).
- **VIEW-075** When the run finishes and the view still exists, its result is stored as `runtime.results[name]`, a save is scheduled and `ev:view(…, 'result', result)` is sent; the result is returned either way.
- **VIEW-076** A run waits for the login-shell env, then spawns the user's shell on the command line (`shellCommand`, 01: `<shell> -c <run>` on macOS; PowerShell on Windows, Platform differences) with `cwd` = the project folder, env = `spawnEnv({NO_COLOR: '1'})`, stdin ignored, stdout/stderr piped, hidden console window.
- **VIEW-077** stdout/stderr chunks are decoded as UTF-8 per chunk and appended while the collected text is shorter than its cap (stdout 1048576, stderr 65536 chars); the stored text is then cut to the cap.
- **VIEW-078** After `120000` ms (`COMMAND_TIMEOUT_MS`) the whole process tree is killed (macOS: the process runs in its own process group and the group gets `SIGKILL`, falling back to killing the child; Windows: `taskkill /pid <pid> /t /f`); the result then has `code: null` and `error: "It took longer than 2 minutes, so it was stopped"`.
- **VIEW-079** A spawn that throws, or a child `error` event, resolves with `code: null` and `error` = the error message; the first completion wins.
- **VIEW-080** A result is `{name, code, stdout, stderr, at, durationMs, error?}`: `code` is the exit code (or `null` when killed by a signal / timed out / failed to start), `at` = finish time (ms), `durationMs` = finish − start. A non-zero exit is still a result, not a rejection.

### Views: approvals and refresh

- **VIEW-081** `approveCommand(project, view, name, allow)`: unknown command name → `That command is gone`. Allow: `approved = run`, `denied` removed, `allowedRuns = [...allowedRuns without run, run].slice(-50)`, `deniedRuns = deniedRuns without run`. Deny: mirror image (`denied = run`, `approved` removed, `deniedRuns` gains it (last 50), `allowedRuns` loses it). Both lists are written as arrays (possibly empty). Then save and `ev:board-changed`.
- **VIEW-082** Allowing a command runs it straight away (errors ignored).
- **VIEW-083** An answer is tied to the exact command line: any change asks again; a line Claude puts back exactly as an earlier allowed/turned-down one keeps that answer (VIEW-012 with the view's lists), whatever the command is called now.
- **VIEW-084** `refreshView(project, view)`: if the view has a refresh, `refreshedAt = now`, schedule a save, and `await askBrain(…, refresh.prompt, 'user')` (a failure rejects the refresh before any command runs); then every allowed command runs concurrently and the results are returned (array, may be empty).

### Views: schedules

- **VIEW-085** A ticker runs every `30000` ms, started once by `configureViews` at app start, while the app is open (whether or not any board is showing).
- **VIEW-086** Each tick visits every non-archived project's views; for each command with `every` that is allowed: due when `now − (last result's at, or 0) ≥ every × 60000`; a due command runs (failure logged `[views] command failed`). A never-run allowed scheduled command runs on the next tick.
- **VIEW-087** A view with `refresh` is due when `now − (runtime.refreshedAt ?? view.createdAt) ≥ refresh.every × 60000`; `refreshedAt = now` and a save are done before asking; the brain is asked with origin `schedule` (failure logged `[views] refresh failed`).

### Views: brains

- **VIEW-088** A brain is a board node `{sessionId: <uuid>, x: view.x, y: view.y, archived: false, title: "View: <view title>", createdByApp: true, viewOf: <view id>, autoMode: true (only for a view in auto mode, VIEW-155), createdAt: now}`; the view's `sessionId` points to it.
- **VIEW-089** `startBrain(project, view, prompt?)`: adds the node and sets `view.sessionId`, then spawns claude: fresh session (`resume: false`), `initialPrompt: prompt` (none for an idle brain), the view's permission mode (VIEW-155), extra args `brainArgs`, cwd = project folder. If spawning fails, the node is removed, `view.sessionId` restored, and the error rethrown. On success: save, `ev:board-changed`.
- **VIEW-090** `brainArgs(project, node)` = `['--append-system-prompt', brainPrompt(view)]` for a node whose `viewOf` view exists, else none; it is added every time a brain starts, resumes, is opened (`session:open`) or sent a prompt (`session:send`), built from the view's title, id and description at that moment.
- **VIEW-091** Renaming a view (tool or user) also sets its brain node's title to `View: <new title>`.
- **VIEW-092** `ensureBrain(project, view)` returns the brain's session id, starting an idle brain (no prompt) when the view has none (its node is missing); the board then opens that session's terminal.
- **VIEW-093** `askBrain(project, view, prompt, origin)`: `prompt` clipped to 4000; empty → `Say what the view’s brain should do`. No brain node → start one on the prompt. Brain process live: status `done` → type it in now (VIEW-095); otherwise (working/waiting) and origin isn't `schedule` → queue it; a scheduled ask to a busy brain is dropped. Brain not live → spawn it with `resume` = whether its transcript exists, `initialPrompt` = prompt, the view's mode (VIEW-155), `brainArgs`.
- **VIEW-094** Queued asks for a brain are joined as `<queued>\n\n<new>` and only the last 4000 characters are kept.
- **VIEW-095** Typing into a brain: cancel its idle timer, write `ESC[200~<text>ESC[201~` (bracketed paste, `\x1b[200~`…`\x1b[201~`), then 200 ms later write `\r`.
- **VIEW-096** On every status change of a brain (`brainStatus`, called from the status machine): cancel its idle timer; if the new status is `done` and the process is live: a queued ask is taken and typed in after 500 ms; otherwise an idle timer of `300000` ms (`BRAIN_IDLE_MS`) starts.
- **VIEW-097** When the idle timer fires: if the brain is still `done` and nothing was typed into it for ≥ 5 minutes (time since the last user input recorded while idle, or ever), its process is killed (the node and transcript stay; it resumes when next needed); if still `done` but typed into more recently, the idle timer re-arms for another full 5 minutes; otherwise nothing.
- **VIEW-098** Any terminal input the user sends to a session (`session:write`) records "last input" for it only while it has an idle timer.
- **VIEW-099** A brain is not drawn on the board and isn't a card anywhere: it is filtered out of the board's cards, `list_sessions`, `nodus.sessions()`, ⌘K search, notifications, and can't be archived (`board:save-nodes` ignores `archived` for it; `archive_sessions`/`archive_self` refuse). The title bar's running count includes it as a background session (08/03).

### Views: whose permission mode a brain runs in

- **VIEW-155** A view records whether its brain runs in auto mode (`autoMode`). One the user makes (VIEW-100) does. One a board tool puts up (VIEW-105) does only when the session calling the tool does: an `autoMode` card or an action run. Any other session, or one that isn't on the board, gives `autoMode: false`. A view saved before this counts as in auto mode when no session put it up (no `from`), and otherwise not. The brain's node and permission mode follow the view (VIEW-088, VIEW-089, VIEW-093): `auto` for a view in auto mode, else the ordinary cards' `acceptEdits`. So do the sessions its page starts (VIEW-058).
- **VIEW-156** `update_view` from a session not in auto mode, on a view in auto mode, takes the view out of it (`autoMode: false`) whatever it changes, and its brain's node stops saying `autoMode`. A brain process still running in auto mode is then stopped, to resume in the view's mode when it's next needed; so is one found running in auto mode for a view that isn't, when it's asked or opened. Nothing puts a view back into auto mode. Otherwise a session that has to ask before running a command could have it run by the view's brain, through the view's description (the brain's system prompt), its refresh schedule or its page's `nodus.ask`.
- **VIEW-157** `start_sessions: true` is taken only from a session in auto mode, or from the view's own brain, which runs in the view's mode (VIEW-155). From anyone else `create_view` and `update_view` fail before changing anything: `Only a session in auto mode, or the view’s own brain, can let a view start sessions`. `false` is taken from anyone.
- **VIEW-158** A brain stopped at a permission prompt is `waiting`, as one asking with AskUserQuestion is (02), so the view shows that Claude needs the user (its pulsing status dot, or `Claude needs you to carry on` before it has a page), and `Open its session` opens the brain's terminal (VIEW-092).

### Views: created from the board (renderer → `views:create`)

- **VIEW-100** `views:create(projectId, {description, x, y})` waits for the login-shell env, checks the project exists (`Unknown project <id>`), then `createView`: `description = clip(description, 4000)`, empty → `Describe what the view should show`; non-finite `x`/`y` → `A view needs a place on the board`; the view is `{id: uuid, title: "New view", description, x: round(x), y: round(y), version: 0, createdAt: now, updatedAt: now}`; its brain starts on `Build this view:\n\n<description>`; if that fails the view is removed and the error rethrown; on success save, `ev:board-changed`, return the view.
- **VIEW-101** The board's create (dialog, context menu, title-bar menu): preferred top-left = `{at.x − 240, at.y − 180}` for "here" or the viewport centre for a 480 × 360 view; `findFreeSpot(preferred, card + note + view obstacles, dock, 480×360)`; `x`/`y` rounded; on success the view is upserted into the board state and the board pans to show it (`ensureVisible`, 09).

### Views: created and changed by board tools

- **VIEW-102** `create_view` checks, in order: `title = clip(title, 80)` empty → `A view needs a title`; `checkHtml(html)` (since 1.0.19 only when `html` is given and not `null`; without it, VIEW-161); `data` (when not `undefined`) → `checkJson(data, 1048576, 'The data')`; `commands` (when not `undefined`) → `normalizeCommands(commands)` with no remembered answers (all new commands wait); `refresh` → `normalizeRefresh`.
- **VIEW-103** `create_view` placement (`placeView`): preferred = right of the caller's active card (`x + cardSize.w + 32`, same `y`), else right of the rightmost active card, else `{dock.x + dock.w + 80, dock.y}`; final = `findFreeSpot(preferred, obstacles(board), dock, viewSize(size))` where obstacles = active cards, free notes, all views. A brain calling it has no active card, so it goes beside the rightmost card.
- **VIEW-104** Size from tools: `clampViewSize({w: Number(width), h: Number(height)})`: non-finite → undefined (default 480/360); finite → `Math.round(min(2400, max(240, w)))`, `Math.round(min(1800, max(160, h)))`. `Number()` coercion applies: `"500"` → 500, `null`/`false` → 0 → minimum, `true` → 1 → minimum.
- **VIEW-105** The new view: `{id: uuid, title, description: clip(description, 4000) || title, x, y, w?, h?, from: <caller>, commands?: (only if non-empty), refresh?, startsSessions?: true (only when start_sessions === true exactly, VIEW-157), autoMode (VIEW-155), version: 1, createdAt: now, updatedAt: now}`.
- **VIEW-106** Then: write `index.html`; runtime `data = args.data`; if refresh, `refreshedAt = now`; schedule runtime save; add to the board; save; `ev:board-changed`; wait for the page check (VIEW-108); return `{view_id, waiting_for_approval: [names of waiting commands], ...check}`.
- **VIEW-107** A view put up by a tool with its `html` has no brain until it first needs one (an ask, refresh, ⌨, Build). Without `html` its brain starts at once (VIEW-161).
- **VIEW-108** Page check (`checkPage(view, since)`): wait up to `3000` ms (`LOAD_WAIT_MS`) for a `loaded` report with version ≥ the view's version; on timeout → `{checked: "The board isn’t showing this view right now, so the page couldn’t be checked for errors."}`; on load → wait `500` ms more (`SETTLE_MS`) → `{errors: [messages of runtime errors with at ≥ since]}` (`[]` when none).
- **VIEW-109** `update_view` resolves the view: `view_id` `undefined` → the caller's own view when the caller is a brain, else `view_id is required: this session isn’t a view’s brain`; otherwise `findView`.
- **VIEW-110** `update_view` validates everything before changing anything: `html` (when given) via `checkHtml`; `data` via `checkJson`; `commands` via `normalizeCommands(commands, view)` (remembered answers); `refresh` via `normalizeRefresh` only when given (`undefined` keeps the current one); `title = clip(title, 80)`.
- **VIEW-111** `update_view` then applies: non-empty title → rename (VIEW-091); `description` given → `clip(…, 4000) || old`; `commands` given → replaced (empty list → key removed); `refresh` given → set (null → removed) and `refreshedAt = now`; finite `width`/`height` → clamped size; `start_sessions` boolean → `true` sets `startsSessions: true`, `false` removes it (non-booleans ignored); `html` given → write `index.html` and `version += 1`; `data` given (any value incl. `null`) → runtime data = it, and if `html` wasn't given, `ev:view(…, 'data', data)` so the page updates in place without reloading; `updatedAt = now`; schedule runtime save; save; `ev:board-changed`.
- **VIEW-112** `update_view` returns `{view_id, version, waiting_for_approval: [...], ...check}` where the page check (VIEW-108) runs only when `html` was given (else nothing extra).
- **VIEW-113** `get_view` (own view when `view_id` is `undefined`, VIEW-109 error otherwise as `findView`) returns `{id, title, description, html: <index.html or ''>, data, state, commands: [{name, run, every, allowed, last?}], refresh, start_sessions: <boolean>, errors: [{message, at: <ISO 8601 string>}] (last 10)}` where `allowed` is `yes` | `no, the user turned it down` | `waiting for the user`, and `last` is the stored result with `stdout`/`stderr` over 4000 chars cut to 4000 + `… (<n> more characters)`.
- **VIEW-114** `list_views` returns every view oldest first as `{id, title, description, yours?: true}`, `yours` when the caller is the view's brain or put it up (`from`).
- **VIEW-115** A view starts sessions only after a tool sets `start_sessions: true` (VIEW-157); the user can only turn it off (VIEW-116, menu), never on; it survives other updates.

### Views: handed to their brain, and the `nodus:views` skill (since 1.0.19)

- **VIEW-159** Nodus has a Claude Code plugin of its own, `nodus`, with one skill, `nodus:views` (`crates/nodus-views/src/skill.rs`). At start the app writes it to `<userData>/claude-plugin/`: `.claude-plugin/plugin.json` (`{"name": "nodus", "version": <app version>, "description": "Nodus's own skills, for the Claude sessions it runs."}`, pretty-printed) and `skills/views/SKILL.md`, each only when it says something new, through `<name>.<pid>.tmp` and a rename. Every Claude process that gets the board tools (cards, brains, action runs; SESS-123) also gets `--plugin-dir <that folder>` right after the tool rules, so the skill is that process's alone: nothing goes into the user's `~/.claude` or the repository. If it can't be written the app logs `[views] couldn’t write the nodus plugin: <error>` and launches without it.
- **VIEW-160** `SKILL.md` is one file, so the Skill tool loads it whole and Claude never reads outside the project for it (a read there stops to ask the user): its frontmatter (`name: views`, a description naming create_view, show_page and update_view), which tool fits what (show_visual, ask, show_page, create_view), how to hand a view to its brain and what its brief must say, how to write a page (data first, start from an example, read the errors, then send data), `## How a page works` with `VIEW_GUIDE` (platform variant included), and `## Examples`: four pages that follow the guide, each with what it shows, in an `html` fence (a table fed by a command, numbers and an SVG chart from `nodus.data` with Claude's look while asked, the board's sessions with `nodus-dot`, and a `show_page` choice sent with `nodus.submit`).
- **VIEW-161** `create_view` without `html` (missing or `null`) hands the view to its brain. `description` is then required: `clip(description, 4000)` empty → `Describe the view for its brain, which builds it without this conversation: what it shows, where the data comes from and how fresh it should be. Or send the html to build it yourself`. Everything else is checked and the view made as VIEW-102…VIEW-105 (placement, size, commands, refresh, `start_sessions`, `autoMode` from the caller, VIEW-155), but with `version: 0` and no `index.html`. Data and refresh go into its runtime as VIEW-106. Its brain then starts on `Build this view:\n\n<description>` (VIEW-089, in the view's mode, so a session not in auto mode gets a brain that asks before it runs things); if that fails the view and its files go and the error is returned. There is no page check. The answer is `{view_id, waiting_for_approval, building: "Its brain, a Claude session of its own, is building it now beside your card, and keeps it up to date. Tell the user it’s on its way, and carry on: don’t wait for it or build it yourself."}`. `show_page` always passes `html` (`""` when missing or `null`), so a page is never handed off (TOOLCHAT-003). `create_view`'s description and `html`/`description` parameters say this, and point a session writing html itself to the skill; `update_view`'s and `show_page`'s descriptions and the server's instructions point to it too.

### Views: board edits, removal, reports

- **VIEW-116** `views:save(projectId, patch)` (`saveView`): `findView(patch.id)`; finite `x`/`y` → rounded and set; `clampViewSize({w, h})` values set when defined; `title = clip(title, 80)` non-empty → rename (VIEW-091); `startsSessions === false` → removed (any other value ignored); save. No `ev:board-changed`.
- **VIEW-117** `views:remove(projectId, viewId)` (`removeView`): `findView`; the view leaves the board; its brain node (if any) leaves the board's nodes, its id is added to `hidden` (once), its in-memory brain state (idle timer, queue, last input) is forgotten, its process is killed and then its status forgotten; its files are deleted (VIEW-008); save. The brain's transcript stays on disk.
- **VIEW-118** Removing a project deletes the files of all its views (brains stop with the project's sessions).
- **VIEW-119** `views:report` (fire-and-forget; any error, e.g. a view that has gone, is swallowed): a non-empty string `error` → appended as `{message: error.slice(0, 2000), at: now}`, keeping the last 20, save scheduled; a numeric `loaded ≥ view.version` → all page-check waiters of that view resolve.
- **VIEW-120** `views:refresh`, `views:ask` (origin `user`, prompt `String(prompt ?? '')`), `views:brain` wait for the login-shell env first; `views:approve` passes `allow === true`.
- **VIEW-121** The "how a view works" guide (`VIEW_GUIDE`) is given to brains (in their system prompt) and to every session (in `create_view`'s description; since 1.0.19 in the `nodus:views` skill instead, VIEW-160); on Windows it contains one extra sentence (Platform differences).

### Views: the card (renderer)

- **VIEW-122** Board nodes: one per view, `type: 'view'`, position `(x, y)`, size `viewSize(view)` (480 × 360 default, the caption included), `zIndex: 0`, `dragHandle: '.view-head'` (only the caption drags), selectable, not deletable by React Flow; node order is archive, pad, sections, sessions, views, notes, draft note (notes draw over views); a selected node is elevated (`elevateNodesOnSelect`, 09). The board's view list is ordered by `createdAt`.
- **VIEW-123** While a view is being dragged/resized (or the board marks it busy) its node keeps the live position (and the live size while resizing) instead of the saved one.
- **VIEW-124** The card's brain status = the brain card's status when its process is live, else `done`; *working* = `working`; *waiting* = live and `waiting`.
- **VIEW-125** Since 1.0.12 a view has no card around its page: the page is a rounded panel straight on the board, under a caption on the board (`.view-head`, 28 tall). The caption's left: when working or waiting, a small status dot (`data-status` = status) with tooltip `Claude is working on this view` / `Claude needs you to carry on with this view`; while refreshing, a spinning `RefreshCw` (12 px); otherwise the `LayoutDashboard` icon (12 px, stroke 2).
- **VIEW-126** Title: the view's title (single line, ellipsis, `--text-secondary`, `--text-primary` while the view is hovered or selected) with the description as tooltip; double-clicking it starts renaming (and doesn't reach the board). Double-clicks anywhere on the caption never reach the board.
- **VIEW-127** Renaming: an input (placeholder `View name`, spellcheck off) with the current title, focused and all selected; Enter or blur commits once: a trimmed non-empty value different from the title → `patchView({id, title})`; Escape cancels; keys don't reach the board; pointer/click/double-click don't reach the board. The resize handles hide while renaming.
- **VIEW-128** No tools on the view (1.0.12): its actions are in the view menu (VIEW-143), opened by a right-click (or ⌃-click on macOS) anywhere on the view, its page included. 0.9.4 had ⟳ ✎ ⌨ ⤢ ⋯ buttons in a title bar; VIEW-129…134 now describe the menu items that replaced them.
- **VIEW-129** `Refresh` (menu): while refreshing the caption's icon spins; afterwards the first result with an `error` or `code !== 0` shows the card error `“<name>” failed: <error>` or `“<name>” exited with code <code>` (`null` prints as `null`); a rejection shows its reason.
- **VIEW-130** `Ask Claude to change it…` (menu, `Sparkles`): opens the ask bar.
- **VIEW-131** Ask bar: an input (placeholder `Tell Claude what to change…`, spellcheck off) focused on open without scrolling, and a primary `Send` button (disabled when the trimmed text is empty or while sending; shows a spinning `LoaderCircle` 14 px instead of the label while sending). Enter or Send → `views.ask(projectId, viewId, trimmed)`; success closes the bar; failure shows the reason as the card error and re-enables the input. Escape closes it. Keys don't reach the board; pointer-down and double-click don't either; the input's wheel doesn't pan.
- **VIEW-132** `Open its session` (menu): `views.brain` then opens that session's terminal; failure toast `Couldn't open its session: <message>`.
- **VIEW-133** `Open large` (menu, `Maximize2`): disabled while `version === 0`; opens the large modal (VIEW-147).
- **VIEW-134** (0.9.4's ⋯ button is gone; the menu opens at the pointer.)
- **VIEW-135** Approvals: for each waiting command, in command order: `This view wants to run[ <every label>]:` followed by the full command line in a code block, then `Don’t allow` (ghost) and `Allow` (primary) buttons → `views.approve(…, false|true)`; failure shows its reason as the card error. The strip sits at the top of the view's panel, above the page, drawn by the board (the page can't draw over it).
- **VIEW-136** Every label (`everyLabel(minutes)`): not a whole number of hours → `every minute` (1) / `every <n> minutes`; whole days → `every day` (24 h) / `every <d> days`; else `every hour` (1 h) / `every <h> hours`.
- **VIEW-137** With `version === 0`, the body shows an empty state: working → the page being built and `Claude is building this view…` with the sparkle (MOTION-032; 0.9.4 showed a spinning `LoaderCircle`, 18 px), ghost button `Watch it work` (opens the brain); waiting → `Claude needs you to carry on`, button `Open its session` (opens the brain); otherwise → `No page yet`, button with `Wand2` icon (14 px) `Build it`. All three are followed by the description (max 4 lines).
- **VIEW-138** `Build it` asks the brain `Build this view.` (origin `user`); failure shows its reason as the card error.
- **VIEW-139** The card error line (`role="status"`) clears itself 6000 ms after it was last set.
- **VIEW-140** A selected view (not renaming) shows resize handles; minimum 240 × 160 in the board (no maximum in the board; main clamps to 2400 × 1800); on resize end `patchView({id, x, y, w, h})` rounded.
- **VIEW-141** While any node is being dragged, a selection rectangle exists, or this view is being resized, frames don't take the pointer, so drags and resizes pass over pages.
- **VIEW-142** A page takes clicks, typing and scrolling on the first interaction: there is no "click to use" shield and clicking in the page doesn't select the view (docs § Views says otherwise; the code wins, since commit 64d9dc4). A view is selected by clicking any part of it outside the page (caption, approvals, empty state, error line) or by marquee. A right-click or ⌃-click on the page is the board's (the view menu); on Windows only the parts outside the page take it. Scrolling over a page scrolls it where it can, otherwise pans the board; a pinch over it zooms the board (VIEW-041).
- **VIEW-143** View menu (right-click anywhere on the view): `Refresh` (`RefreshCw`, disabled unless refresh or an allowed command; VIEW-129 on the view; a view not drawn runs `views.refresh` with failure toast `Couldn't refresh it: <message>`); `Ask Claude to change it…` (`Sparkles`, VIEW-130); `Open large` (`Maximize2`, VIEW-133); `Open its session` (`SquareTerminal`); separator; `Rename` (`PencilLine`); `Stop it starting sessions` (`Ban`, only while `startsSessions`; `patchView({id, startsSessions: false})`); separator; `Delete view` (`Trash2`, danger, hint `⌫`).
- **VIEW-144** Deleting asks first (danger confirm): one view → title `Delete view?`, message `“<title>” and its page go, and the Claude session that keeps it up to date stops. Its transcript stays on disk.`; several → title `Delete <n> views?`, message `Their pages go, and the Claude sessions that keep them up to date stop. Their transcripts stay on disk.`; buttons `Cancel` / `Delete`. Confirmed → each view removed in turn (VIEW-145).
- **VIEW-145** Removing in the board: the view and its brain card leave the board state immediately; if the brain's terminal is open it closes; the brain's terminal is disposed; then `views.remove` is awaited.
- **VIEW-146** `patchView` applies the patch's defined fields to the board state immediately and sends `views.save` (not awaited).
- **VIEW-147** Large modal: a modal over the board (`view-modal`) with a header (`LayoutDashboard` 15 px, the title as heading, a close button `X` 15 px with tooltip/aria-label `Close`) and a second frame of the same page filling the body; Escape (when the board, not the page, has focus) or a backdrop click closes it.
- **VIEW-148** Dragging views: a view dropped onto the archive or the note pad bounces back to where its drag started; otherwise its rounded position is saved with `patchView`.
- **VIEW-149** Keyboard on the board: Backspace/Delete (no ⌘/Ctrl, no ⌥) with views selected → delete them, asking first (together with deleting selected notes and archiving selected cards); Escape clears the selection; Enter does nothing for views; ⌘A (select all active) doesn't select views. Keys typed while a page has focus go to the page, not the board.
- **VIEW-150** A source handle on the card's right edge (1 × 1 px, invisible, not connectable) anchors the dashed lines to sessions the view started.

### Views: the New view dialog

- **VIEW-151** Opened from the board's context menu `New view here…` (`LayoutDashboard`, disabled over the archive; the view is placed around the clicked point) and from the New Session split button's menu `New view…` (placed in view). Other entry points: `/view` in the prompt bar (11), floating capture (07).
- **VIEW-152** Content: icon tile (`LayoutDashboard` 16 px, stroke 2.2), title `New view`, subtitle `Say what it should show and where the data comes from. Claude builds it on the board and keeps it up to date.`; a 5-row textarea, autofocused, placeholder `For example: a table of the last 3 Vercel deployments, with the commit, who deployed it and the status.`; the label `Start from an example` and five chips (VIEW-153); the hint `Its page runs in a sandbox with no network. To fetch data it runs commands you allow on the card, or asks its own Claude session, which costs tokens each time.`; footer `Cancel` (ghost) and `Create view` (primary).
- **VIEW-153** Example chips (label → text), clicking fills the box with the text; a chip is active when the box equals its text exactly; tooltip = the text:
  1. `Vercel deployments` → `A table of the last 3 Vercel deployments of this project: the commit id, who deployed it, its status and when. Refresh every 10 minutes.`
  2. `Users in the database` → `The total number of users in the database, as one big number with when it was counted. Refresh every hour.`
  3. `My Linear tickets` → `All my open Linear tickets as a table: id, title, status and priority, each linking to the ticket. Refresh every hour.`
  4. `Open pull requests` → `The open pull requests on GitHub for this repository, with their author, CI status and age. Refresh every 15 minutes.`
  5. `Board at a glance` → `How this board is doing: how many sessions are working, waiting for me or done, and a list of the ones waiting for me that opens them when clicked.`
- **VIEW-154** `Create view` (or ⌘Enter / Ctrl+Enter anywhere in the dialog) with a non-blank description: everything disables, `Create view` shows a spinning `LoaderCircle` (14 px) before its label; success closes the dialog; failure shows `Couldn’t start building it: <reason>` in an error box and re-enables. `Create view` is disabled while the box is blank. Cancel is disabled while creating; Escape and a backdrop click still close the dialog (creation continues).

## Data & formats

All JSON below is shown as the Electron app emits it. Key order follows JS object insertion order; keys whose value is `undefined` are absent. Numbers print as JS does (`10`, not `10.0`). Rust must reproduce both (see Rust mapping, risks R6–R8).

### MCP `initialize` result

```json
{
  "protocolVersion": "2025-06-18",
  "capabilities": { "tools": {} },
  "serverInfo": { "name": "nodus", "version": "<app version, e.g. 0.9.4>" },
  "instructions": "<INSTRUCTIONS>"
}
```

`INSTRUCTIONS`, verbatim (one line; note the curly `’` in `project’s`):

```text
Nodus is the board of Claude Code sessions the user runs this session from. Besides the session cards, it holds sticky notes and views. When the user asks you to put something on the board, or to make notes of it, write it with create_note. Read the notes they have put up with list_notes. Only write notes when the user asks for them. When the user asks for a view, a dashboard, a table, a chart or other custom UI on the board, build it with create_view; list_views, get_view and update_view find and change the views there. To put a note on another project’s board, or read the notes there, pass that project to the note tools; list_projects lists the projects. When the user asks for a new project, add it with create_project. When the user asks you to archive yourself, use archive_self.
```

Since 0.9.4 it goes on, in the same line (TOOL-110):

```text
 The board also holds shapes (boxes, ellipses, diamonds, words and emoji stickers) and links, arrows between cards, notes, views and shapes. The user links cards to show how their work relates: list_board shows what is on the board and where, and what this session’s card is linked to. When the user asks for something drawn, laid out, rearranged or linked on the board, use draw_on_board, update_drawing and erase_from_board.
```

After 1.0.17 (TOOL-114), `Only write notes when the user asks for them.` reads `Write notes when the user asks for them, or when a short one would help them come back to something later.`, and the last sentence of TOOL-110's text gives way to:

```text
 look_at_board shows the board as a picture, with the pictures and ink sketches words can’t describe: look at it when the user points at something on the board or asks about what is there. Draw with draw_on_board when the user asks for something drawn, laid out, linked or marked, and on your own when a small sketch, a picture you made or a mark on what you mean would say it better than words: keep it small and near your card. update_drawing and erase_from_board change and take away drawings; change the user’s only when they ask.
```

The chat's sentences (TOOLCHAT) follow.

### MCP `tools/list` result, verbatim

`result.tools` for an action run, in this order. Ordinary sessions get the same array without the last two entries. Generated from the 0.9.4 source on macOS; `create_view`'s description is the text shown followed by `\n\n` and `VIEW_GUIDE` (next section), written here as `<VIEW_GUIDE>`. Recommended: commit the Electron app's actual `tools/list` output (macOS and Windows) as golden fixtures and compare byte for byte. Since 1.0.19 `create_view`'s description is `CREATE_VIEW` (`crates/nodus-tools/src/schema.rs`) without the guide, only `title` is required, and `tests/fixtures/tools.json` holds the current output, the same on both platforms (VIEW-161).

```json
[
  {
    "name": "create_note",
    "description": "Puts a sticky note on the user’s Nodus board, beside this session’s card. Use it when the user asks you to put something on the board: findings, a summary, a plan, a list to come back to. A note shows its text as written, so Markdown is not rendered: skip **bold** and # headings, though \"- \" bullets and blank lines read well. Keep each note to one topic someone can take in at a glance, starting with a short line that says what it is about, and write several notes for several topics. Returns the note’s id.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "text": {
          "type": "string",
          "description": "What the note says."
        },
        "color": {
          "type": "string",
          "enum": [
            "blue",
            "purple",
            "pink",
            "red",
            "orange",
            "yellow",
            "green",
            "graphite"
          ],
          "description": "The note’s colour. Defaults to yellow. Colours can tell kinds of notes apart."
        },
        "project": {
          "type": "string",
          "description": "Another project’s board to use, by its id or name from list_projects. Leave out for this session’s own board."
        }
      },
      "required": [
        "text"
      ]
    }
  },
  {
    "name": "update_note",
    "description": "Changes the text or colour of a note on the board. Use it to revise a note rather than writing another beside it.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "note_id": {
          "type": "string",
          "description": "An id from create_note or list_notes."
        },
        "text": {
          "type": "string",
          "description": "The note’s new text, in full."
        },
        "color": {
          "type": "string",
          "enum": [
            "blue",
            "purple",
            "pink",
            "red",
            "orange",
            "yellow",
            "green",
            "graphite"
          ],
          "description": "The note’s new colour."
        },
        "project": {
          "type": "string",
          "description": "Another project’s board to use, by its id or name from list_projects. Leave out for this session’s own board."
        }
      },
      "required": [
        "note_id"
      ]
    }
  },
  {
    "name": "list_notes",
    "description": "Lists the sticky notes on the board, the user’s and any Claude wrote: each one’s id, text and colour, and whether this session wrote it.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "project": {
          "type": "string",
          "description": "Another project’s board to use, by its id or name from list_projects. Leave out for this session’s own board."
        }
      }
    }
  },
  {
    "name": "list_projects",
    "description": "Lists the user’s Nodus projects, each with a board of its own: its id, name and folder, whether it is archived, and which one this session is on. Use it to find the project for a note on another board.",
    "inputSchema": {
      "type": "object",
      "properties": {}
    }
  },
  {
    "name": "create_project",
    "description": "Adds a project to Nodus: a folder on this machine, which gets a board of its own. Use it only when the user asks for a new project. The folder is made if it isn’t there yet, and a folder that is already a project gives back that project. Returns the project’s id and name.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string",
          "description": "The project’s folder, as an absolute path. A leading ~ is the home folder."
        },
        "name": {
          "type": "string",
          "description": "What to call the project. Defaults to the folder’s name."
        }
      },
      "required": [
        "path"
      ]
    }
  },
  {
    "name": "archive_self",
    "description": "Moves this session into the board’s archive when your current reply ends, and stops it. Its transcript is kept, so the user can pick it up again later. Use it only when the user asks you to archive yourself, and call it as the last thing you do: the session stops as soon as this turn ends.",
    "inputSchema": {
      "type": "object",
      "properties": {}
    }
  },
  {
    "name": "create_view",
    "description": "Puts a view on the user’s Nodus board, beside this session’s card: a small live web page, such as a table, a dashboard or a chart, that stays up to date. Use it when the user asks for a view or for custom UI on the board. It returns the view’s id, the commands waiting for the user to allow them, and any JavaScript errors the page hit as it loaded. The view gets a brain of its own, a Claude session that keeps it up to date, when it first needs one.\n\n<VIEW_GUIDE>",
    "inputSchema": {
      "type": "object",
      "properties": {
        "title": {
          "type": "string",
          "description": "A short name for the view, shown on its card."
        },
        "html": {
          "type": "string",
          "description": "The page: one self-contained HTML document, with its CSS and JavaScript inline. It draws from nodus.data."
        },
        "data": {
          "description": "JSON the page draws from, as nodus.data. Sending new data updates the page in place without reloading it."
        },
        "commands": {
          "type": "array",
          "description": "Shell commands the page may run, replacing the ones it had. The user allows each command line once on the card; an unchanged one stays allowed.",
          "items": {
            "type": "object",
            "properties": {
              "name": {
                "type": "string",
                "description": "What the page calls it by: nodus.run(name)."
              },
              "run": {
                "type": "string",
                "description": "The command line, run in the project folder with the user’s shell environment."
              },
              "every": {
                "type": "number",
                "description": "Minutes between runs while the app is open (at least 1). Leave out to run only when the page asks."
              }
            },
            "required": [
              "name",
              "run"
            ]
          }
        },
        "refresh": {
          "type": [
            "object",
            "null"
          ],
          "description": "Asks the view’s brain to bring it up to date every so often, for data only Claude can fetch (an MCP server, say). Each refresh costs tokens. null stops it.",
          "properties": {
            "every": {
              "type": "number",
              "description": "Minutes between refreshes, at least 10."
            },
            "prompt": {
              "type": "string",
              "description": "What the brain is asked each time. It starts fresh, so say where the data comes from and to send it with update_view."
            }
          },
          "required": [
            "every",
            "prompt"
          ]
        },
        "start_sessions": {
          "type": "boolean",
          "description": "Lets the page start Claude sessions on the board with nodus.startSession, from buttons the user clicks. Only turn it on when the user asks for the view to start sessions."
        },
        "width": {
          "type": "number",
          "description": "Card width in board units. Defaults to 480."
        },
        "height": {
          "type": "number",
          "description": "Card height in board units. Defaults to 360."
        },
        "description": {
          "type": "string",
          "description": "What the view is for and where its data comes from, for its brain to go on later."
        }
      },
      "required": [
        "title",
        "html"
      ]
    }
  },
  {
    "name": "update_view",
    "description": "Changes a view on the board: its page, its data, its title, its commands or its refresh schedule. Leave out what stays the same. To show fresh data, send only data. Returns the commands waiting for the user to allow them and, when the page changed, any JavaScript errors it hit as it loaded.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "view_id": {
          "type": "string",
          "description": "An id from create_view or list_views. A view’s brain can leave it out for its own view."
        },
        "title": {
          "type": "string",
          "description": "A short name for the view, shown on its card."
        },
        "html": {
          "type": "string",
          "description": "The page: one self-contained HTML document, with its CSS and JavaScript inline. It draws from nodus.data."
        },
        "data": {
          "description": "JSON the page draws from, as nodus.data. Sending new data updates the page in place without reloading it."
        },
        "commands": {
          "type": "array",
          "description": "Shell commands the page may run, replacing the ones it had. The user allows each command line once on the card; an unchanged one stays allowed.",
          "items": {
            "type": "object",
            "properties": {
              "name": {
                "type": "string",
                "description": "What the page calls it by: nodus.run(name)."
              },
              "run": {
                "type": "string",
                "description": "The command line, run in the project folder with the user’s shell environment."
              },
              "every": {
                "type": "number",
                "description": "Minutes between runs while the app is open (at least 1). Leave out to run only when the page asks."
              }
            },
            "required": [
              "name",
              "run"
            ]
          }
        },
        "refresh": {
          "type": [
            "object",
            "null"
          ],
          "description": "Asks the view’s brain to bring it up to date every so often, for data only Claude can fetch (an MCP server, say). Each refresh costs tokens. null stops it.",
          "properties": {
            "every": {
              "type": "number",
              "description": "Minutes between refreshes, at least 10."
            },
            "prompt": {
              "type": "string",
              "description": "What the brain is asked each time. It starts fresh, so say where the data comes from and to send it with update_view."
            }
          },
          "required": [
            "every",
            "prompt"
          ]
        },
        "start_sessions": {
          "type": "boolean",
          "description": "Lets the page start Claude sessions on the board with nodus.startSession, from buttons the user clicks. Only turn it on when the user asks for the view to start sessions."
        },
        "width": {
          "type": "number",
          "description": "Card width in board units. Defaults to 480."
        },
        "height": {
          "type": "number",
          "description": "Card height in board units. Defaults to 360."
        }
      }
    }
  },
  {
    "name": "get_view",
    "description": "Reads a view as it is now: its page, data and saved state, its commands with whether the user allowed them and their last output, its refresh schedule, and the JavaScript errors it hit lately.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "view_id": {
          "type": "string",
          "description": "An id from create_view or list_views. A view’s brain can leave it out for its own view."
        }
      }
    }
  },
  {
    "name": "list_views",
    "description": "Lists the views on the board: each one’s id, title and description, and whether it’s yours.",
    "inputSchema": {
      "type": "object",
      "properties": {}
    }
  },
  {
    "name": "list_sessions",
    "description": "Lists the sessions on this project’s Nodus board. Each has its id, title, kind (claude or terminal), status (working, waiting for the user, or done), whether its process is running, whether it is archived, the section it is in, Claude’s last reply, and when it was last active. The session running this action is not listed.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "include_archived": {
          "type": "boolean",
          "description": "Also list archived sessions. Defaults to false."
        }
      }
    }
  },
  {
    "name": "archive_sessions",
    "description": "Moves sessions into the board’s archive and stops their processes. Their transcripts are kept, so they can be picked up again later. A session nothing was said in is removed from the board instead, as there is nothing to keep. Sessions that are working or waiting for the user are skipped unless force is true. Terminals cannot be archived.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "session_ids": {
          "type": "array",
          "items": {
            "type": "string"
          },
          "description": "Ids from list_sessions."
        },
        "force": {
          "type": "boolean",
          "description": "Archive sessions even if they are working or waiting."
        }
      },
      "required": [
        "session_ids"
      ]
    }
  }
]
```

Since 0.9.4, `list_board`, `draw_on_board`, `update_drawing` and `erase_from_board` sit between `archive_self` and `create_view` (TOOL-101), and after 1.0.17 `look_at_board` after `list_board` (TOOL-111). The fixture's answers are regenerated from the Rust server when the tools change. Their schemas are in `crates/nodus-tools/src/schema.rs`, and verbatim in `ADDED_TOOLS` in `crates/nodus-tools/tests/fixtures/generate.mjs`, which splices them into 0.9.4's answers for `fixtures/tools.json`.

### `VIEW_GUIDE`, verbatim (macOS text)

On Windows, the text `' This is Windows, and each command line runs in PowerShell, so write it for PowerShell.'` (with its leading space) is inserted right after `and the page gets its output to parse.` in item 1, before ` The user allows each command line once…`. The `×` in `480×360` is U+00D7.

```text
How a view works:
- It's one self-contained HTML document, shown in a sandboxed frame on a card on the board. It has no network: it can't fetch URLs or load scripts, stylesheets, fonts or images from anywhere, so put everything inline (CSS in <style>, JS in <script>, charts as inline SVG or <canvas>, icons as inline SVG or Unicode). Links open in the user's browser.
- Draw from data rather than writing values into the page: the page renders nodus.data, and you change what it shows with update_view's data, which updates the page in place. Rewrite the html only to change how it looks or works.
- Fresh data comes two ways:
  1. Commands, for anything a CLI or curl can fetch. Declare them with update_view's commands. Each runs in the project folder with the user's shell environment (their PATH, their CLIs, and env vars such as API tokens), and the page gets its output to parse. The user allows each command line once on the card before it can run. The page runs one with nodus.run(name), or give it every (minutes) to run on a schedule while the app is open. Commands cost no tokens, so prefer them, and make their output easy to parse (JSON flags, --format options, jq). Try a command yourself before declaring it.
  2. You, for what needs your tools (MCP servers such as Linear or Sentry) or judgment. Set refresh to be asked to update the view every so often (each refresh costs tokens; at least every 10 minutes), and the page can ask you with nodus.ask(prompt). Fetch what's needed, then send it with update_view's data.
- The card starts 480×360 and the user resizes it, so keep the layout fluid and readable at that size. Use the board's colours, which follow light and dark mode: var(--nodus-bg), --nodus-surface, --nodus-hover, --nodus-text, --nodus-text-2, --nodus-text-3, --nodus-border, --nodus-accent, --nodus-green, --nodus-red, --nodus-orange, --nodus-yellow, --nodus-purple, and the fonts --nodus-font and --nodus-mono. The body is 13px on var(--nodus-bg) with no margin, so add your own padding. Show when the data is from, and a clear state while there's no data yet or when a command failed.
- The page's JavaScript API is window.nodus:
  nodus.on('data', fn): calls fn with nodus.data once the page has loaded, and again whenever new data is sent.
  nodus.run(name): runs one of the view's commands, and resolves to { name, code, stdout, stderr, at, error }. nodus.results[name] holds each command's last result, and nodus.on('result', fn) gets every new one, scheduled runs included.
  nodus.state and nodus.setState(obj): the page's own saved state, such as a filter or what the user ticked, kept across reloads.
  nodus.sessions(), nodus.notes(), nodus.views(), nodus.view(id), nodus.git(): resolve to the board's sessions (id, title, status, branch, last reply), its sticky notes, the other views with their data, and the repository's git status. nodus.on('board', fn) fires when sessions, notes or other views change, and nodus.on('git', fn) when the git status does.
  nodus.ask(prompt): asks the view's brain, a Claude session of its own, to do something, such as fetching fresh data. At most once every 15 seconds.
  nodus.openSession(id), nodus.openUrl(url).
  nodus.startSession(prompt, { title }): starts a Claude session on the board, beside the view, on prompt. It only works in a view that may start sessions (update_view's start_sessions, which nodus.canStartSessions reflects), and only within a few seconds of the user clicking or pressing something in the page, so call it from a button's handler. The session starts fresh, so put everything it needs in the prompt: what to do, and the details from the view, such as the failing test and its error.
- Only let a view start sessions when the user asks for that, such as a button beside each failing test that starts a session to fix it. Otherwise leave start_sessions off and don't add such buttons.
- After changing the html, check the errors update_view returns, and fix them.
```

### Tool results (the value that is pretty-printed into `content[0].text`)

| Tool | Result value |
| --- | --- |
| `create_note` | `{"noteId": "<uuid>"}` |
| `update_note` | `{"noteId": "<note_id as given>"}` |
| `list_notes` | `[{"id", "text", "color", "writtenByYou"?: true}]`, oldest first |
| `list_projects` | `[{"id", "name", "folder", "archived"?: true, "thisSessionsBoard"?: true}]` |
| `create_project` | `{"projectId", "name", "folder", "madeFolder"?: true, "alreadyAProject"?: true}` |
| `archive_self` | `{"archiving": "This session moves to the archive when this turn ends."}` |
| `list_sessions` | `[{"id", "title", "kind", "status", "running", "section"?, "actionRun"?: true, "branch"?, "lastReply"?, "lastActive"}]` |
| `archive_sessions` | `{"archived": [ids], "removed": [ids], "skipped": [{"id", "reason"}]}` |
| `create_view` | `{"view_id", "waiting_for_approval": [names], "errors": [messages]}` or `{"view_id", "waiting_for_approval": [...], "checked": "The board isn’t showing this view right now, so the page couldn’t be checked for errors."}` |
| `update_view` | `{"view_id", "version", "waiting_for_approval": [...]}` plus `"errors"` or `"checked"` when `html` was given |
| `get_view` | `{"id", "title", "description", "html", "data"?, "state"?, "commands": [{"name", "run", "every"?, "allowed", "last"?: CommandResult}], "refresh"?: {"every", "prompt"}, "start_sessions": bool, "errors": [{"message", "at": "<ISO 8601>"}]}` |
| `list_views` | `[{"id", "title", "description", "yours"?: true}]`, oldest first |
| `list_board` | `{"linked_to_this_session": [lines], "cards": [{"id", "title", "status", "x", "y", "w", "h", "this_session"?: true}], "notes": [{"id", "text", "color", "x", "y", "w", "h"}], "views": [{"id", "title", "x", "y", "w", "h"}], "shapes": [{"id", "kind", "text"?, "color", "x", "y", "w", "h", "yours"?: true}], "links": [{"id", "from", "to", "label"?, "color", "yours"?: true}]}` (TOOL-102; added after 0.9.4) |
| `list_board`, after 1.0.17 | adds `"ink"?: [{"id", "kind", "color", "x", "y", "w", "h", "text"?, "from"?, "to"?, "around"?, "yours"?: true}]` and `"ink_not_listed"?` (TOOL-113) |
| `look_at_board` | an image block, then `{"area": {"x", "y", "w", "h"}, "scale", "tags": {"<tag>": {"id", "kind", "name"?, "this_session"?: true}}, "untagged_ink"?}` (TOOL-111) |
| `draw_on_board` | `{"shapes": {"<key or index>": "<id>"}, "links": ["<id>"], "ink"?: ["<id>"], "area"?: {"x", "y", "w", "h"}}` |
| `update_drawing` | `{"updated": ["<id>"]}` |
| `erase_from_board` | `{"erased": ["<id>"], "skipped": [{"id", "reason"}]}` |

Example `content[0].text` for `create_note` (2-space indentation, `\n` newlines, no trailing newline):

```text
{
  "noteId": "3f0c9b8e-1d2a-4c55-9a77-0b1e2f3a4b5c"
}
```

### Host ⇄ app relay messages (wire format is 02's; listed for completeness)

- welcome carries `mcp: true` when the host relays board tools.
- host → app: `{ "type": "mcp", "id": <number>, "sessionId": "<id>", "message": <JSON-RPC request> }`
- app → host: `{ "type": "mcp-reply", "id": <same number>, "reply": <JSON-RPC response object, or absent> }`
- HTTP: `POST /mcp`, headers `Authorization: Bearer <hook token>`, `X-Nodus-Session: <session id>`; answer `200 application/json`, `202` (notifications only), `204` (DELETE), `401`, `405` (`Allow: POST`), `413`, `500`.

### Brain system prompt (`--append-system-prompt`), verbatim template

`${view.title}`, `${view.id}`, `${view.description}` are substituted as-is; `${VIEW_GUIDE}` is the guide above (platform variant included). Straight apostrophes throughout.

```text
You are the brain of a view on the user's Nodus board: a small live web page on a card, beside their Claude Code sessions. The view is "${view.title}" (id ${view.id}). The user asked for:

${view.description}

You build the page and keep it up to date with the mcp__nodus__* board tools. update_view sets its html, data, title, commands and refresh schedule. get_view shows it as it is now, with its commands' last output and any JavaScript errors it hit. Leave out view_id: both act on this view. If you're picking up a view that already has a page, read it with get_view before changing it.

${VIEW_GUIDE}

The nodus:views skill has example pages that follow this guide: start a new page from the nearest one.

Nobody watches you while you work, so don't stop to chat. Work out where the data comes from by looking around the repository, the CLIs on the PATH and your MCP tools. If you need the user, for a missing API token or a choice only they can make, ask with AskUserQuestion, which shows on the view that you need them. End each turn with one short line on what you did.
```

Prompts a brain receives as user input: `Build this view:\n\n<description>` (first prompt of a view created from the board), `Build this view.` (the **Build it** button), the view's `refresh.prompt` (⟳ and schedule), whatever the user types in the ask bar, whatever a page passes to `nodus.ask`.

### `state.json` additions owned here (per board in `boards[<projectId>]`)

```ts
views?: Record<string /* view id */, BoardView>   // getBoard() ensures {}
notes?: Record<string /* note id */, BoardNote>   // getBoard() ensures {}
ink?: InkStroke[]                                 // absent until first save
hidden?: string[]                                 // removed views' brains are added here
nodes: Record<string, BoardNode>                  // brains live here with viewOf

interface BoardView {
  id: string; title: string; description: string
  x: number; y: number; w?: number; h?: number
  sessionId?: string        // its brain
  from?: string             // session that put it up with create_view
  commands?: ViewCommand[]
  allowedRuns?: string[]    // ≤ 50, oldest first; may be []
  deniedRuns?: string[]     // ≤ 50, oldest first; may be []
  refresh?: { every: number /* minutes, 10..10080 */; prompt: string }
  startsSessions?: true
  version: number           // 0 until there's a page; +1 per html change
  createdAt: number; updatedAt: number   // ms since epoch
}
interface ViewCommand { name: string; run: string; every?: number /* 1..10080 */; approved?: string; denied?: string }
interface BoardNote {
  id: string; text: string; color: FolderColor; x: number; y: number; w?: number; h?: number
  sessionId?: string        // sticker on that card (10)
  from?: string             // session that wrote it with create_note
  fromProject?: string; fromTitle?: string   // written from another project's board
  createdAt: number
}
interface InkStroke { id: string; kind: 'pen'|'arrow'|'text'|'rect'; color: 'graphite'|'red'|'orange'|'green'|'blue'|'purple'; points: number[]; size: number; text?: string }
// A brain node: { sessionId, x, y, archived: false, title: "View: <title>", createdByApp: true, viewOf: <view id>, autoMode: true, createdAt }
// A session a page started: { …, expanded: true, autoMode: true, fromView: <view id> }
```

### `<userData>/views/<view id>/`

- `index.html`: the page as Claude sent it (UTF-8, unmodified). Absent until the first page.
- `runtime.json`: compact JSON (`JSON.stringify(runtime)`), written atomically via `runtime.json.<pid>.tmp` + rename:

```ts
{
  data?: any            // what update_view/create_view sent (nodus.data)
  state?: any           // what the page saved with setState (nodus.state)
  results: Record<string /* command name */, CommandResult>
  errors: { message: string /* ≤ 2000 */; at: number }[]   // ≤ 20, oldest first
  refreshedAt?: number  // when its brain was last asked to refresh
}
interface CommandResult {
  name: string
  code: number | null   // null: timed out, killed by a signal, or couldn't start
  stdout: string        // ≤ 1048576 chars
  stderr: string        // ≤ 65536 chars
  at: number            // finished, ms since epoch
  durationMs: number
  error?: string        // "It took longer than 2 minutes, so it was stopped" or the spawn error
}
```

Rust must read 0.9.4's files and write files 0.9.4 can read; key order inside `runtime.json` is not significant to readers.

### Content security policy (`VIEW_CSP`), verbatim

Sent both as the `Content-Security-Policy` response header and in the injected `<meta http-equiv>`:

```text
default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'
```

### The served page

```text
<doctype handling per VIEW-022> … <head> + INJECTED + rest of the page
INJECTED = <meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="VIEW_CSP"><meta name="viewport" content="width=device-width, initial-scale=1"><style>THEME</style><script>BRIDGE(__BOOT__ → scriptJson(boot))</script>
boot = {"viewId":"<id>","title":"<title>","startsSessions":true?,"data":…?,"state":…?,"results":{…}}
```

`THEME` as 0.9.4 had it, verbatim (the text between the backticks of `const THEME`, including its leading and trailing newline). Since 1.0.15 the theme is `crates/nodus-views/src/page/theme.css` (VIEW-026):

```css

:root {
  color-scheme: light dark;
  --nodus-bg: #ffffff;
  --nodus-surface: rgba(0, 0, 0, 0.035);
  --nodus-hover: rgba(0, 0, 0, 0.05);
  --nodus-text: rgba(0, 0, 0, 0.88);
  --nodus-text-2: rgba(0, 0, 0, 0.56);
  --nodus-text-3: rgba(0, 0, 0, 0.36);
  --nodus-border: rgba(0, 0, 0, 0.1);
  --nodus-accent: #007aff;
  --nodus-green: #2da44e;
  --nodus-red: #cf222e;
  --nodus-orange: #ff9500;
  --nodus-yellow: #d4a72c;
  --nodus-purple: #8250df;
  --nodus-font: -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Helvetica Neue', sans-serif;
  --nodus-mono: 'SF Mono', ui-monospace, Menlo, Monaco, monospace;
}
@media (prefers-color-scheme: dark) {
  :root {
    --nodus-bg: #2a2a2d;
    --nodus-surface: rgba(255, 255, 255, 0.04);
    --nodus-hover: rgba(255, 255, 255, 0.07);
    --nodus-text: rgba(255, 255, 255, 0.9);
    --nodus-text-2: rgba(255, 255, 255, 0.58);
    --nodus-text-3: rgba(255, 255, 255, 0.36);
    --nodus-border: rgba(255, 255, 255, 0.1);
    --nodus-accent: #0a84ff;
    --nodus-green: #3fb950;
    --nodus-red: #f85149;
    --nodus-orange: #ff9f0a;
    --nodus-yellow: #e3b341;
    --nodus-purple: #a371f7;
  }
}
:where(html, body) {
  margin: 0;
  background: var(--nodus-bg);
  color: var(--nodus-text);
  font: 13px/1.45 var(--nodus-font);
  -webkit-font-smoothing: antialiased;
}
:where(a) { color: var(--nodus-accent); }

```

`BRIDGE`, verbatim (the text between the backticks of `const BRIDGE`, including its leading and trailing newline; `__BOOT__` is replaced as in VIEW-023):

```js

(function () {
  var boot = __BOOT__;
  var seq = 0, pending = {}, listeners = {};
  var current = { data: boot.data, state: boot.state, results: boot.results || {} };
  function post(msg) { msg.nodus = 1; parent.postMessage(msg, '*'); }
  function call(method, args) {
    return new Promise(function (resolve, reject) {
      var id = ++seq;
      pending[id] = { resolve: resolve, reject: reject };
      post({ id: id, method: method, args: args || {} });
    });
  }
  function report(err) {
    var text = err && (err.stack || err.message) ? (err.stack || err.message) : String(err);
    post({ error: String(text).slice(0, 2000) });
  }
  function emit(event, payload) {
    (listeners[event] || []).slice().forEach(function (fn) {
      try { fn(payload); } catch (err) { report(err); }
    });
  }
  function whenReady(fn) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fn, { once: true });
    else Promise.resolve().then(fn);
  }
  window.addEventListener('message', function (e) {
    if (e.source !== parent) return;
    var m = e.data;
    if (!m || m.nodus !== 1) return;
    if (m.id) {
      var p = pending[m.id];
      if (!p) return;
      delete pending[m.id];
      if (m.error) p.reject(new Error(m.error)); else p.resolve(m.result);
      return;
    }
    if (m.event === 'data') { current.data = m.payload; nodus.data = m.payload; }
    else if (m.event === 'state') { current.state = m.payload; nodus.state = m.payload; }
    else if (m.event === 'result' && m.payload) { current.results[m.payload.name] = m.payload; }
    if (m.event) emit(m.event, m.payload);
  });
  var nodus = {
    viewId: boot.viewId,
    title: boot.title,
    canStartSessions: Boolean(boot.startsSessions),
    data: current.data,
    state: current.state,
    results: current.results,
    on: function (event, fn) {
      (listeners[event] = listeners[event] || []).push(fn);
      if (event === 'data' || event === 'state') {
        whenReady(function () {
          if (current[event] !== undefined && (listeners[event] || []).indexOf(fn) >= 0) {
            try { fn(current[event]); } catch (err) { report(err); }
          }
        });
      }
      return function () {
        listeners[event] = (listeners[event] || []).filter(function (f) { return f !== fn; });
      };
    },
    setState: function (state) {
      current.state = state;
      nodus.state = state;
      return call('setState', { state: state });
    },
    run: function (name) { return call('run', { name: name }); },
    ask: function (prompt) { return call('ask', { prompt: prompt }); },
    sessions: function () { return call('sessions'); },
    notes: function () { return call('notes'); },
    views: function () { return call('views'); },
    view: function (id) { return call('view', { id: id }); },
    git: function () { return call('git'); },
    openSession: function (id) { return call('openSession', { id: id }); },
    startSession: function (prompt, options) {
      return call('startSession', { prompt: prompt, title: options && options.title });
    },
    openUrl: function (url) { return call('openUrl', { url: url }); }
  };
  window.nodus = nodus;
  window.addEventListener('error', function (e) { report(e.error || e.message); });
  window.addEventListener('unhandledrejection', function (e) { report(e.reason); });
  window.addEventListener('load', function () { post({ loaded: true }); });
  // Links open in the browser. The frame itself never leaves the view.
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    var href = a.getAttribute('href') || '';
    if (href.charAt(0) === '#') return;
    e.preventDefault();
    if (/^https?:/i.test(a.href)) call('openUrl', { url: a.href });
  }, true);
  // Scrolling over the page scrolls it where something in it can scroll that
  // way. Otherwise the board gets the scroll, to pan as it would over any
  // card, and a pinch always zooms the board. A gesture stays with whichever
  // of the two it started on.
  function room(pos, max, d) { return d < 0 ? pos > 0 : pos < max - 1; }
  function scrolls(el, dx, dy) {
    var root = document.documentElement;
    for (; el && el !== root && el.nodeType === 1; el = el.parentElement) {
      var s = getComputedStyle(el);
      if (dy && /auto|scroll/.test(s.overflowY) && room(el.scrollTop, el.scrollHeight - el.clientHeight, dy)) return true;
      if (dx && /auto|scroll/.test(s.overflowX) && room(el.scrollLeft, el.scrollWidth - el.clientWidth, dx)) return true;
    }
    // The page itself. While the root's overflow is visible, the body's is the page's.
    var axis = dy ? 'overflowY' : 'overflowX';
    var overflow = getComputedStyle(root)[axis];
    if (overflow === 'visible' && document.body) overflow = getComputedStyle(document.body)[axis];
    if (/hidden|clip/.test(overflow)) return false;
    return dy
      ? room(root.scrollTop, root.scrollHeight - root.clientHeight, dy)
      : room(root.scrollLeft, root.scrollWidth - root.clientWidth, dx);
  }
  var toBoard = false, lastWheel = 0;
  window.addEventListener('wheel', function (e) {
    if (e.defaultPrevented || (!e.deltaX && !e.deltaY)) return;
    var now = Date.now();
    if (e.ctrlKey) toBoard = true;
    else if (now - lastWheel > 250) {
      var across = Math.abs(e.deltaX) > Math.abs(e.deltaY);
      toBoard = !scrolls(e.target, across ? e.deltaX : 0, across ? 0 : e.deltaY);
    }
    lastWheel = now;
    if (!toBoard) return;
    e.preventDefault();
    post({ wheel: {
      dx: e.deltaX, dy: e.deltaY, mode: e.deltaMode, x: e.clientX, y: e.clientY,
      ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, meta: e.metaKey
    } });
  }, { passive: false });
})();

```

### Frame ⇄ board message protocol (`postMessage`, target origin `'*'`)

Every message carries `nodus: 1`.

| Direction | Message | Meaning |
| --- | --- | --- |
| page → board | `{nodus:1, id:<n>, method:"<m>", args:{…}}` | request (VIEW-032); methods `setState`, `run`, `ask`, `sessions`, `notes`, `views`, `view`, `git`, `openSession`, `startSession`, `openUrl` |
| board → page | `{nodus:1, id:<n>, result:<any>}` / `{nodus:1, id:<n>, error:"<reason>"}` | reply |
| page → board | `{nodus:1, error:"<≤2000 chars>"}` | a JavaScript error (stored, VIEW-119) |
| page → board | `{nodus:1, loaded:true}` | `window` load (board reports the frame's version) |
| page → board | `{nodus:1, wheel:{dx,dy,mode,x,y,ctrl,shift,alt,meta}}` | a scroll the page didn't use |
| board → page | `{nodus:1, event:"data", payload:<data>}` | new data in place (from `update_view` data-only) |
| board → page | `{nodus:1, event:"state", payload:<state>}` | the view's state was saved (both frames of the view) |
| board → page | `{nodus:1, event:"result", payload:CommandResult}` | a command finished (incl. scheduled) |
| board → page | `{nodus:1, event:"board"}` | cards/notes/views changed (300 ms debounce), or another view's data changed |
| board → page | `{nodus:1, event:"git", payload:<git status or null>}` | the board's git status changed |

### IPC (`window.api.views`, channels in `src/shared/api.ts`) and event

| Call | Channel | Kind | Main |
| --- | --- | --- | --- |
| `create(projectId, {description, x, y})` → `BoardView` | `views:create` | invoke | VIEW-100 |
| `save(projectId, ViewPatch)` | `views:save` | invoke | VIEW-116 |
| `remove(projectId, viewId)` | `views:remove` | invoke | VIEW-117 |
| `call(projectId, viewId, method, args)` → any | `views:call` | invoke | VIEW-060 |
| `report(projectId, viewId, {error?, loaded?})` | `views:report` | send (no reply) | VIEW-119 |
| `approve(projectId, viewId, name, allow)` | `views:approve` | invoke | VIEW-081 |
| `refresh(projectId, viewId)` → `CommandResult[]` | `views:refresh` | invoke | VIEW-084 |
| `ask(projectId, viewId, prompt)` | `views:ask` | invoke | VIEW-093 |
| `brain(projectId, viewId)` → session id | `views:brain` | invoke | VIEW-092 |
| event `on.view(cb(projectId, viewId, event, payload))` | `ev:view` | main → renderer | VIEW-054 |

`ViewPatch = {id, x?, y?, w?, h?, title?, startsSessions?: false}`. Ink: `board:save-ink(projectId, strokes)` (INK-012). Tools also emit `ev:board-changed(projectId)` and `ev:projects-changed`.

## API surface

`nodus-core` exposes, one to one with the IPC above (names indicative):

- `views_create(project_id, CreateViewOptions) -> Result<BoardView>`
- `views_save(project_id, ViewPatch) -> Result<()>`
- `views_remove(project_id, view_id) -> Result<()>`
- `views_call(project_id, view_id, method: String, args: serde_json::Value) -> Result<Value>`
- `views_report(project_id, view_id, Report { error: Option<String>, loaded: Option<f64> })` (infallible to the caller)
- `views_approve(project_id, view_id, name, allow: bool) -> Result<()>`
- `views_refresh(project_id, view_id) -> Result<Vec<CommandResult>>`
- `views_ask(project_id, view_id, prompt) -> Result<()>`
- `views_brain(project_id, view_id) -> Result<SessionId>`
- `board_save_ink(project_id, Value)` → `cleanInk`
- events: `ViewEvent { project_id, view_id, kind: Data|State|Result, payload }`, `BoardChanged(project_id)`, `ProjectsChanged`

`nodus-tools`:

- `const BOARD_SERVER: &str = "nodus"`, `BOARD_MCP_CONFIG: &str`, `BOARD_TOOL_RULES: [&str; 16]` (12 in 0.9.4)
- `async fn answer_mcp(session_id, message: Value) -> Option<Value>` (None only when the app can't answer)
- `trait BoardToolHandlers { is_run, list_projects, create_project, find_project, list_sessions, archive_sessions, archive_self, list_notes, create_note, update_note, list_views, get_view, create_view, update_view, list_board, look_at_board, draw_on_board, update_drawing, erase_from_board }` (mirrors `BoardToolHandlers`, implemented by `StoreHandlers` with the logic in TOOL-028–TOOL-038 and TOOL-101–TOOL-114; `look_at_board` answers a `Look { png, key }`, and its default refuses)

`nodus-board` (notes and ink): `NOTE_MAX_CHARS`, `clamp_note_size`, `fit_note`, `create_note(board, from, patch, away)`, `update_note`, `list_notes`, `clean_ink(Value) -> Vec<InkStroke>`.

`nodus-views`: storage (VIEW-001–008), validation (VIEW-009–017), `serve_page(url) -> Response`, `build_view_page`, `script_json`, `VIEW_CSP`, `VIEW_GUIDE` (cfg per OS), `THEME`, `BRIDGE`, `create_view`, `save_view`, `remove_view`, `remove_project_views`, `approve_command`, `refresh_view`, `report_view`, `call_view`, `run_command`, `ensure_brain`, `ask_brain`, `brain_status`, `brain_input`, `brain_args`, ticker, `flush_views`, `create_view_from_tool`, `update_view_from_tool`, `get_view_for_tool`, `list_views_for_tool`, `obstacles`. Hooks it needs from the outside: `notify(view event)`, `board_changed(project)`, `env_ready` future, plus the sessions API (spawn/is_live/write/kill/get_status/forget_status/has_transcript) and `session_list`.

`nodus-ui`: the view card, view frame host (wry), relay, ask bar, title editor, large modal, New view dialog, view menu items and delete confirm.

## UI details

### Tokens used here (tokens.css; light / dark)

| Token | Light | Dark |
| --- | --- | --- |
| `--bg-elevated` | `#ffffff` | `#2a2a2d` |
| `--bg-window` | `#f5f5f4` | `#1e1e20` |
| `--bg-sunken` | `rgba(0, 0, 0, 0.035)` | `rgba(255, 255, 255, 0.04)` |
| `--bg-hover` | `rgba(0, 0, 0, 0.05)` | `rgba(255, 255, 255, 0.07)` |
| `--bg-pressed` | `rgba(0, 0, 0, 0.09)` | `rgba(255, 255, 255, 0.12)` |
| `--text-primary` | `rgba(0, 0, 0, 0.88)` | `rgba(255, 255, 255, 0.9)` |
| `--text-secondary` | `rgba(0, 0, 0, 0.56)` | `rgba(255, 255, 255, 0.58)` |
| `--text-tertiary` | `rgba(0, 0, 0, 0.36)` | `rgba(255, 255, 255, 0.36)` |
| `--text-on-accent` | `#ffffff` | `#ffffff` |
| `--border-strong` | `rgba(0, 0, 0, 0.18)` | `rgba(255, 255, 255, 0.18)` |
| `--separator` | `rgba(0, 0, 0, 0.07)` | `rgba(255, 255, 255, 0.07)` |
| `--accent` | `#007aff` | `#0a84ff` |
| `--accent-hover` | `#0068d9` | `#3396ff` |
| `--focus-ring` | `rgba(0, 122, 255, 0.5)` | `rgba(10, 132, 255, 0.55)` |
| `--status-working` | `#ff9500` | `#ff9f0a` |
| `--status-waiting` | `#ff3b30` | `#ff453a` |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.06), 0 0 0 0.5px rgba(0,0,0,0.08)` | `0 1px 2px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.08)` |
| `--shadow-md` | `0 4px 14px rgba(0,0,0,0.08), 0 0 0 0.5px rgba(0,0,0,0.1)` | `0 4px 14px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(255,255,255,0.1)` |
| `--shadow-lg` | `0 18px 50px rgba(0,0,0,0.18), 0 0 0 0.5px rgba(0,0,0,0.12)` | `0 18px 50px rgba(0,0,0,0.5), 0 0 0 0.5px rgba(255,255,255,0.12)` |
| `--shadow-modal` | `0 30px 90px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(0,0,0,0.2)` | `0 30px 90px rgba(0,0,0,0.6), 0 0 0 0.5px rgba(255,255,255,0.14)` |
| `--radius-sm` / `--radius-lg` / `--radius-xl` | `6px` / `14px` / `18px` | same |
| `--ease-out` | `cubic-bezier(0.2, 0.8, 0.2, 1)` | same |
| `--dur-fast` / `--dur-med` | `120ms` / `220ms` | same |
| `--text-xs` / `sm` / `md` / `lg` | `11px` / `12px` / `13px` / `15px` | same |
| `--font-ui` | `-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI Variable Text', 'Segoe UI', 'Helvetica Neue', sans-serif` | same |
| `--font-display` | `-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI Variable Display', 'Segoe UI', 'Helvetica Neue', sans-serif` | same |
| `--font-mono` | `'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', Consolas, monospace` | same |

`color-mix(in srgb, X p%, transparent)` = X at alpha p (premultiplied). Derived values used below:

| Expression | Light | Dark |
| --- | --- | --- |
| `color-mix(in srgb, var(--accent) 7%, var(--bg-elevated))` (approvals strip) | `rgb(237.15, 245.69, 255)` ≈ `#edf6ff` | `rgb(39.76, 48.3, 59.7)` ≈ `#28303c` |
| `color-mix(in srgb, var(--status-waiting) 45%, transparent)` (waiting ring) | `rgba(255, 59, 48, 0.45)` | `rgba(255, 69, 58, 0.45)` |
| `color-mix(in srgb, var(--status-working) 22%, transparent)` (working dot halo) | `rgba(255, 149, 0, 0.22)` | `rgba(255, 159, 10, 0.22)` |
| `… --status-working 10% …` (halo at 50 %) | `rgba(255, 149, 0, 0.10)` | `rgba(255, 159, 10, 0.10)` |
| `… --status-waiting 55% …` (pulse start) | `rgba(255, 59, 48, 0.55)` | `rgba(255, 69, 58, 0.55)` |
| `color-mix(in srgb, var(--accent) 12%, transparent)` (active example chip) | `rgba(0, 122, 255, 0.12)` | `rgba(10, 132, 255, 0.12)` |
| `color-mix(in srgb, var(--status-waiting) 10%, transparent)` (dialog error box) | `rgba(255, 59, 48, 0.10)` | `rgba(255, 69, 58, 0.10)` |

### The view (`.view`, views.css; no card since 1.0.12)

- Node: cursor `default` over the whole node.
- Node: a flex column, the caption over the panel. Panel: flex 1; radius `14px` (`--radius-lg`); background `--bg-elevated`; shadow `--shadow-sm`; overflow hidden; own stacking context; `transition: box-shadow 220ms cubic-bezier(0.2, 0.8, 0.2, 1)`. The page's own corners round with it where nothing of the panel is above or below it.
- Panel states (later rule wins): hover (the node or its page) → `--shadow-md`; waiting → `0 0 0 1px <waiting 45%>, --shadow-md`; selected → `0 0 0 2px var(--accent), --shadow-md`; dragging → `0 0 0 2px var(--accent), --shadow-lg`.
- Caption (`.view-head`): on the board, no background or border; flex row, items centred, gap `6px`, height `28px` (fixed), padding `0 4px 4px`. It is the drag handle.
- Icon (`LayoutDashboard`, 12 px, stroke 2): colour `--text-tertiary`, no shrink.
- Status dot (`.status-dot.is-small`, global.css): `7 × 7px` circle. Working: fill `--status-working`, halo `0 0 0 2px <working 22%>`, animation `status-breathe-small 1.6s ease-in-out infinite` (at 50 %: `0 0 0 3.5px <working 10%>`). Waiting: fill `--status-waiting`, animation `status-pulse 1.1s ease-out infinite` (0 %: `0 0 0 0 <waiting 55%>` → 100 %: `0 0 0 8px transparent`). Tooltips per VIEW-125.
- Title (`.view-title`, `h3`): min-width 0, margin 0, `--font-display` 12px weight 500, letter-spacing `-0.005em`, `--text-secondary` (`--text-primary` while hovered or selected), single line with ellipsis.
- Title input (`.view-title-input`): flex 1, height `26px`, padding `0 8px`, border `0.5px solid --accent`, radius `6px`, background `--bg-window`, shadow `0 0 0 3px --focus-ring`, font `600 13px --font-display`, `--text-primary`, no outline.
- Refreshing spinner: `.spin` = `rotate(360deg)` over `0.9s linear infinite`.
- Approvals strip (`.view-approvals`): flex column, gap `8px`, no shrink, max-height `45%` of the panel, padding `10px 12px`, top corners rounded with the panel's, bottom border `0.5px --separator`, background accent-7 % mix, scrolls (`overflow: auto`). Each item: column, gap `8px`. Text (`.view-approve-text`): margin 0, 12px, `--text-secondary`. Command (`.view-approve-cmd`, `code`): block, max-height `72px`, margin-top `5px`, padding `6px 8px`, radius `6px`, background `--bg-sunken`, `--font-mono` 11px, line-height 1.5, `--text-primary`, `white-space: pre-wrap`, `word-break: break-all`, scrolls, text selectable. Actions row: right-aligned, gap `6px`; buttons `.btn.btn-ghost` (`Don’t allow`) and `.btn.btn-primary` (`Allow`).
- Body (`.view-body`): flex 1, min-height 0, positioned. Frame (`.view-frame`): absolute, fills the body, no border, background `--bg-elevated`.
- Empty state (`.view-empty`): absolute fill, flex column centred both ways, gap `10px`, padding `20px 28px`, centred text, colour `--text-secondary`. Title (`.view-empty-title`): 13px weight 600 `--text-primary`, margin 0. Spinner `LoaderCircle` 18 px spinning. Buttons: `Watch it work` = `.btn.btn-ghost`; `Open its session` and `Build it` = `.btn` (Build it has `Wand2` = lucide `wand-sparkles`, 14 px, stroke 2, before the label, gap 6px). Description (`.view-empty-description`): max-width `44ch`, 12px, line-height 1.45, `--text-tertiary`, clamped to 4 lines, margin 0.
- Error line (`.view-error`): no shrink, margin 0, padding `7px 12px`, top border `0.5px --separator`, 12px, colour `--status-waiting`.
- Ask bar (`.view-ask`): flex row, gap `6px`, padding `8px`, top border `0.5px --separator`. Input (`.view-ask-input`): flex 1, height `28px`, padding `0 10px`, border `0.5px solid --border-strong`, radius `6px`, background `--bg-window`, font `13px --font-ui`, `--text-primary`, no outline, transitions `box-shadow 120ms ease, border-color 120ms ease`; focused → border `--accent`, shadow `0 0 0 3px --focus-ring`. Button `.btn.btn-primary` `Send` (spinner 14 px while sending).
- Vertical order: the caption, then in the panel approvals (if any), body, error line (if any), ask bar (if open).
- Resize (when selected and not renaming): edge hit areas `10px` thick (z 2, invisible); corner handles `9 × 9px`, border `1.5px solid --accent`, radius `2px`, background `--bg-elevated`, z 3 (board.css `.note-resize-*`, shared with notes).
- Lineage handle (`.card-handle`): `1 × 1px`, transparent, invisible, no pointer events, on the right edge.

### Shared buttons (`.btn`, global.css; owned by 08, listed for completeness)

Height `28px`, padding `0 12px`, gap `6px`, radius `6px`, border `0.5px solid --border-strong`, background `--bg-elevated`, shadow `--shadow-sm`, 13px weight 500, no wrap, `transition: background 120ms ease`; hover `--bg-hover`; active `--bg-pressed`; disabled opacity `0.45` and no pointer events. `.btn-primary`: background `--accent`, transparent border, colour `--text-on-accent`, hover `--accent-hover`. `.btn-ghost`: transparent background and border, no shadow.

### Large modal (`.view-modal` in the shared Modal, 08)

- Panel: `width: min(1400px, 92vw)`, `height: 88vh`, capped by the Modal's `max-width: calc(100vw - 48px)` / `max-height: calc(100vh - 48px)`; radius `18px`, background `--bg-elevated`, `--shadow-modal`; backdrop `rgba(0,0,0,0.28)` light / `rgba(0,0,0,0.45)` dark with `blur(3px)`, z-index 500; enter animations 220 ms ease-out (08).
- Header (`.view-modal-head`): flex row centred, gap `9px`, height `44px`, padding `0 8px 0 16px`, bottom border `0.5px --separator`; `LayoutDashboard` 15 px stroke 2 in `--text-tertiary`; title (`h2`, id `view-modal-<view id>`, the dialog's label) `--font-display` 15px weight 600, single line ellipsis, flex 1; close `.card-tool` with `X` 15 px.
- Body (`.view-modal-body`): flex 1, min-height 0, positioned; the frame fills it.

### New view dialog (classes from actions.css, owned by 05; values used)

- Panel `.action-dialog`: `width: min(620px, calc(100vw - 48px))`, `max-height: min(86vh, 760px)`; inner column filling it.
- Header: flex, items top, gap `12px`, padding `20px 22px 16px`, bottom border `0.5px --separator`. Icon tile `32 × 32px`, radius `9px`, `linear-gradient(135deg, var(--accent), #5e5ce6)`, icon colour `#fff`. Title 15px weight 600 margin 0. Subtitle margin `2px 0 0`, 12px, `--text-secondary`.
- Body: column, gap `14px`, padding `16px 22px`, scrolls. Textarea `.action-input.action-description`: full width, border `0.5px solid --border-strong`, radius `8px`, background `--bg-window`, padding `10px 12px`, line-height 1.45, vertical resize, focus border `--accent` + `0 0 0 3px --focus-ring` (120 ms transitions), disabled opacity `0.6`.
- Examples: column gap `8px`; label 12px weight 600 `--text-secondary`; chips wrap with gap `6px`; chip height `26px`, padding `0 11px`, border `0.5px solid --border-strong`, radius `13px`, background `--bg-elevated`, `--text-primary`, 12px; hover `--bg-hover`; active → border `--accent`, background accent 12 %, colour `--accent`.
- Hint: 12px, line-height 1.4, `--text-secondary`, margin 0.
- Error box `.action-error.selectable`: margin `0 22px 12px`, padding `8px 12px`, radius `8px`, background waiting 10 %, `--text-primary`, 12px, `pre-wrap`, `break-word`, text selectable.
- Footer: flex, gap `8px`, padding `12px 22px`, top border `0.5px --separator`; spacer then `Cancel` (`.btn.btn-ghost`) and `Create view` (`.btn.btn-primary`).

### Menus, toasts, confirm (08/09 components; copy here)

- View menu items and icons: VIEW-143. Hint glyph via `keys('⌫')`: `⌫` on macOS, `Del` elsewhere.
- Toasts (09, ~4.2 s): `Couldn't open its session: <message>`, `Couldn't refresh it: <message>`, `“<view title>” tried to start a session. Ask its brain if you want it to be able to.` (note the straight `'` in `Couldn't` here vs the curly one in the dialog's `Couldn’t start building it:`).
- Confirm: VIEW-144.

## Keyboard & mouse

| Where | Input | Effect |
| --- | --- | --- |
| View caption | drag | moves the view (only the caption drags) |
| View caption | click | selects the view (React Flow selection, 09) |
| View title | double-click | rename (VIEW-127) |
| Title input | Enter / blur · Escape | commit · cancel |
| Selected view | drag a corner/edge | resize, min 240 × 160 |
| Page | click / type / scroll | goes to the page at once; unused scroll pans the board; pinch zooms the board |
| Page | click a link | `http(s)` opens in the browser; `#…` stays; others do nothing |
| View, page included | right-click (⌃-click) | view menu at the pointer (VIEW-143) |
| Ask bar | Enter · Escape | send · close |
| Board (view(s) selected, focus not in a field or page) | Backspace / Delete | delete views, asking first |
| Board | Escape | clear selection |
| Large modal | Escape (focus not in the page) · backdrop click · ✕ | close |
| New view dialog | ⌘Enter (Ctrl+Enter on Windows) | create |
| New view dialog | Escape · backdrop click · Cancel | close |
| Board canvas | right-click › `New view here…` | dialog, view placed around that point |

## Platform differences

- **Commands' shell.** macOS: the login shell (`$SHELL` from the login env when absolute, else the process's `$SHELL` when absolute, else `/bin/zsh`) with `-c <run>`. Windows: `pwsh.exe` when on `PATH`, else `<SystemRoot>\System32\WindowsPowerShell\v1.0\powershell.exe` (`SystemRoot` from the env, default `C:\Windows`), with `-NoProfile -NonInteractive -Command "[Console]::OutputEncoding = [Text.Encoding]::UTF8; <run>"` (01's `shellCommand`).
- **Process tree.** macOS: the command runs detached in its own process group; timeout sends `SIGKILL` to the group (`kill(-pid)`), falling back to the child. Windows: not detached (a detached process would get no console, so each program it runs would open a window), `windowsHide`; timeout runs `taskkill /pid <pid> /t /f` (hidden).
- **Guide text.** `VIEW_GUIDE` (thus the brain's system prompt and `create_view`'s description) has the extra PowerShell sentence on Windows.
- **Shortcut labels.** `⌫` vs `Del` in the view menu hint; ⌘Enter vs Ctrl+Enter in the dialog (modKey).
- **Fonts in pages.** The injected theme names only Apple fonts (`--nodus-font`: `-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Helvetica Neue', sans-serif`; `--nodus-mono`: `'SF Mono', ui-monospace, Menlo, Monaco, monospace`), so on Windows pages fall back to the engine's `sans-serif`/`monospace`. Keep verbatim.
- **Paths.** `~/Library/Application Support/Nodus/views/<id>/` vs `%APPDATA%\Nodus\views\<id>\`. `create_project` expands `~` followed by `/` or `\`.
- **Webview engine (Rust).** Electron renders pages with Chromium on both OSes; Rust uses WKWebView (macOS) and WebView2/Chromium (Windows). See risks R3, R4, R11.

## Edge cases & errors

### Verbatim error strings (all returned to Claude as tool errors, to pages as rejections, or shown on the card/dialog after VIEW-053 stripping)

| Message | Where |
| --- | --- |
| `Nodus isn’t open, so the board can’t be reached right now.` | host, app closed/timeout |
| `Parse error` (code -32700) | host, bad JSON |
| `Unknown method <method>` (code -32601) | answerMcp |
| `The board is not available` | tools, before configure |
| `This session is no longer on a Nodus board` | tools, caller not on a board |
| `<tool> is only for action runs` | session tools for non-runs |
| `Unknown tool <name>` | tools |
| `note_id is required` | update_note |
| `project must be a project’s id or name, from list_projects` | findProject |
| `More than one project is called “<ref>”; use its id from list_projects` | findProject |
| `There’s no project called “<ref>”; list_projects lists them` | findProject |
| `path is required` · `path must be absolute` | create_project |
| `Folder not found: <folder>` · `Not a folder: <real path>` | addProject |
| `This session is no longer on the board` · `Terminals can’t be archived` · `This session is already archived` · `A view’s brain stays with its view; deleting the view stops it` · `An action run leaves the board by itself when its turn ends` | archive_self |
| `not on the board` · `it is the session running this action` · `it is a view’s brain, which goes with its view` · `terminals can’t be archived` · `already archived` · `it is waiting for the user` · `it is working` | archive_sessions skip reasons |
| `A note needs some text` | create/update note |
| `There’s no note with that id on this board` | update_note |
| `Not a view id` | view folder path |
| `There’s no view with that id on this board` | findView |
| `view_id is required: this session isn’t a view’s brain` | update_view/get_view |
| `A view needs a title` | create_view |
| `html must be the page, as one HTML document` · `The page is too big (over 2 MB)` | html |
| `The data must be JSON` · `The data is too big (over 1024 KB)` | data |
| `The state must be JSON` · `The state is too big (over 256 KB)` | setState |
| `commands must be a list of { name, run, every }` · `A view can have at most 12 commands` · `Each command needs a name and a run (its command line)` · `There are two commands called “<name>”` | commands |
| `refresh needs every (minutes) and a prompt, or null to stop refreshing` | refresh |
| `Describe what the view should show` · `A view needs a place on the board` | views:create |
| `Unknown project <id>` | views:create (getProject) |
| `That command is gone` | approve |
| `This view has no command called “<name>”` · `The user didn’t allow “<name>” to run` · `“<name>” is waiting for the user to allow it on the view` | run |
| `It took longer than 2 minutes, so it was stopped` | command result `error` |
| `Say what the view’s brain should do` | ask with blank prompt |
| `Asked too soon: a view can ask its brain once every 15 seconds` | page ask |
| `Only http and https links open` | page openUrl |
| `nodus.<method> isn’t something a view can do` | page unknown method |
| `nodus.openSession needs a session id` · `nodus.startSession needs a prompt` · `A view can only start a session within a few seconds of the user clicking something in it` · `The board isn’t open` | relay |
| `This view isn’t allowed to start sessions. The user can ask its brain to let it.` · `This view just started a session; wait a moment before starting another` | board startSession |
| `“<name>” failed: <error>` · `“<name>” exited with code <code>` | card after ⟳ |
| `Couldn’t start building it: <reason>` | New view dialog |
| `The board isn’t showing this view right now, so the page couldn’t be checked for errors.` | `checked` in tool results |

### Edge cases

- **Docs vs code.** docs § Views says "Until you click a view, the page is a picture" and the shortcuts table says "double-click it to open it large"; neither is in 0.9.4 (shield removed in 64d9dc4; there is no double-click handler for views). docs says "Click **View** in the title bar"; 0.9.4 has `New view…` in the New Session split button's menu. The code wins (VIEW-142, VIEW-151); docs to be fixed at cutover.
- **Errors toasts with the IPC prefix.** `Couldn't refresh it:` and `Couldn't open its session:` use `err.message` without VIEW-053 stripping, so Electron shows e.g. `Couldn't refresh it: Error invoking remote method 'views:refresh': Error: Say what the view’s brain should do`. unclear: reproduce the prefix or not (BoardView.tsx:1291, 1327); recommend showing the stripped message and recording the decision.
- **Away-note signature on user save.** `board:save-note` (ipc.ts:540–556, 10's) rebuilds the note without `fromProject`/`fromTitle`, so moving or editing a note written from another project drops its away signature from `state.json`. unclear whether intended; port as-is (10 owns) unless the plan says otherwise.
- **Page check without a board.** `create_view`/`update_view` with html while the board of that project isn't open (or the view's node isn't rendered: React Flow renders only visible nodes when the board has more than 150 cards) → after 3 s the `checked` message instead of `errors`.
- **Two frames.** With the large modal open both frames load, report `loaded` and report the same JS errors, so errors can be stored twice.
- **Errors window.** `errors` returned by the page check are every error reported since the tool call started, from any frame of the view.
- **Data before page.** `update_view` with only `data` on a version-0 view stores it; nothing is shown until a page exists; the page gets it at boot.
- **`commands: []`** removes the commands key but keeps `allowedRuns`/`deniedRuns`.
- **Scheduled refresh to a busy brain** is dropped, yet `refreshedAt` already advanced, so the next try is a full interval later.
- **Failing page ask** (blank prompt) still uses up the 15 s window.
- **Command killed by a signal** without timeout → `code: null`, no `error`; ⟳ shows `“<name>” exited with code null`.
- **Unknown view in `runCommand`** throws synchronously instead of rejecting (callers are inside try/promise chains, so the effect is the same rejection).
- **`typeInto`** clears the idle timer but leaves its map entry, so `brainInput` keeps recording input until the next status change (harmless).
- **`views` from a page** lists other views in board insertion order, not by `createdAt`.
- **`update_view` / `create_view` `width: null`** → minimum width 240 (Number(null) = 0).
- **Removing a view** while its command is running: the result is not stored (view gone) but the promise still resolves for the caller.
- **App closed:** schedules don't run; host answers tool calls with `APP_CLOSED`; brains keep running in the host until idle-killed by a running app (no idle stop while the app is closed).
- **Note ids from Claude** like `__proto__` never match (own-property lookups everywhere: notes, views, nodes in `archive_sessions`).
- **Lone UTF-16 surrogates** can exist in JS strings (and in `JSON.stringify` output as `\udxxx`); Rust strings can't hold them (R8).

## Tests to port

### `src/main/boardTools.test.ts` (mocks: app version `9.9.9`; every session is on `p1` except `gone`; `run-session` is the only run; handler mocks return fixed values)

1. protocol › `answers initialize in the version asked for`: `initialize` with `protocolVersion: '2025-03-26'` → `{jsonrpc:'2.0', id:1, result:{protocolVersion:'2025-03-26', capabilities:{tools:{}}, serverInfo:{name:'nodus', version:'9.9.9'}, instructions: contains 'create_note'}}`.
2. protocol › `answers ping, and unknown methods with an error`: `ping` → `{jsonrpc:'2.0', id:2, result:{}}`; `resources/list` → `{jsonrpc:'2.0', id:3, error:{code:-32601, message:'Unknown method resources/list'}}`.
3. protocol › `gives every session the note and view tools, and action runs the session tools too`: tool names for `s1` = `[create_note, update_note, list_notes, list_projects, create_project, archive_self, create_view, update_view, get_view, list_views]`; for the run, the same + `list_sessions, archive_sessions`. Since 0.9.4 the four drawing tools come after `archive_self` (TOOL-101).
4. protocol › `allows every tool by name and keeps the token out of the config`: `BOARD_TOOL_RULES` equals the 12 `mcp__nodus__…` names in order (16 since 0.9.4, TOOL-101); config url `http://127.0.0.1:${NODUS_PORT}/mcp`; headers `{Authorization: 'Bearer ${NODUS_TOKEN}', 'X-Nodus-Session': '${NODUS_SESSION}'}`.
5. tools › `creates a note, dropping a colour that is not one of the board’s`: `create_note {text:'Findings', color:'chartreuse'}` → handler `('p1','s1',{text:'Findings', color: undefined})`; reply id 7; parsed text `{noteId:'n1'}`.
6. tools › `updates a note by id`: `update_note {note_id:'n1', color:'green'}` → `('p1','n1',{text: undefined, color:'green'})`; without `note_id` → `isError: true`, text `note_id is required`.
7. tools › `lists notes and archives the calling session`: `list_notes` → `('p1','s1')`; `archive_self` → `('p1','s1')`.
8. tools › `writes, changes and reads notes on another project’s board, by its id or name`: `project:'Docs'` → createNote on `p2`; `project:'p2'` → updateNote/listNotes on `p2`.
9. tools › `says so when the project isn’t one, and writes nothing`: `project:'Nowhere'` → `{content:[{type:'text', text:'There’s no project called “Nowhere”; list_projects lists them'}], isError:true}`; createNote not called.
10. tools › `lists projects and adds one`: listProjects called with `'p1'`; `create_project {path:'~/Code/new-app', name:'New app'}` → handler called with `('~/Code/new-app','New app')` (unexpanded; expansion is the handler's), result `{projectId:'p3'}`.
11. tools › `keeps the session tools to action runs`: `list_sessions` from `s1` → isError, text `list_sessions is only for action runs`; `archive_sessions` → isError; neither handler called.
12. tools › `lets an action run list and archive sessions`: `include_archived:'yes'` → `listSessions('p1', RUN, false)`; `session_ids:['s2',3,null,'s3'], force:true` → `archiveSessions('p1', RUN, ['s2','s3'], true)`.
13. tools › `turns failures into tool errors Claude can read`: a throwing createNote → `{content:[{type:'text', text:'A note needs some text'}], isError:true}`; session `gone` → text `This session is no longer on a Nodus board`; `delete_everything` → `Unknown tool delete_everything`.
14. tools › `creates and updates views, answering once the handler is done`: `create_view {title:'Deploys', html:'<p>hi</p>'}` → handler args passed through unchanged, text `{view_id:'v1'}`; `update_view {data:{count:3}}` → passed through, text `{view_id:'v1', version:2}`.
15. tools › `reads a view by id, or the caller’s own when it leaves the id out`: `get_view {view_id:'v1'}` → `('p1','s1','v1')`; no args → `('p1','s1', undefined)`; `list_views` → `('p1','s1')`.
16. tools › `reports a view tool that fails, as it does any other`: rejected updateView → `{content:[{type:'text', text:'There’s no view with that id on this board'}], isError:true}`.

### `src/main/notes.test.ts` (real store, `save` mocked; each test a fresh project with card `s1` at (2000, 0))

1. createNote › `puts a session’s first note beside its card`: text `'  Findings  '` → `{text:'Findings', color:'yellow', from:'s1', x: 2000+288+32, y:0}`; stored on the board as the same object.
2. createNote › `puts the next one under the last`: second note at `{x: first.x, y: first.y + 216 + 32}`, colour `green`.
3. createNote › `keeps clear of the board’s cards`: with card `s2` at (2000+288+32, 0), the note doesn't overlap s2's 288 × 216 rect.
4. createNote › `stays square for short text and grows to a limit for long text`: `'Short'` → `w`, `h` undefined; `'word '.repeat(400)` → `{w:360, h:480}`.
5. createNote › `needs some text, and keeps no more than the limit`: `'   '` and `{}` throw `A note needs some text`; 20050 `x`s → text length 20000.
6. a note from another project’s board › `goes beside the rightmost card, under the last one that session wrote here, and says where it came from`: with cards at x 2000 and 0, visitor note → `{from:'visitor', fromProject:'elsewhere', fromTitle:'Release checklist', x: 2000+288+32, y:0}`; second → under it.
7. a note from another project’s board › `leaves the fields out for a note from this board`: `fromProject`, `fromTitle` undefined.
8. updateNote › `changes the text and colour, and keeps the size`: note `w = 500`; update `{text:' After ', color:'red'}` → `{text:'After', color:'red', w:500}`.
9. updateNote › `leaves out what the patch doesn’t say`: `{}` keeps `{text:'Keep', color:'blue'}`; `{text:' '}` throws `A note needs some text`.
10. updateNote › `only finds the board’s own notes`: `missing` throws `There’s no note with that id on this board`; `__proto__`, `constructor`, `toString`, `hasOwnProperty` throw `There’s no note`; `Object.prototype` is not polluted.
11. listNotes › `lists notes oldest first and says which the session wrote`: `[{id:'claude', text:'Claude’s', color:'blue', writtenByYou:true}, {id:'user', text:'Mine', color:'yellow', writtenByYou: undefined}]`.

### `src/main/ink.test.ts`

1. `keeps pen strokes, arrows and labels, rounding their points`: pen `[0.123, 1.987, 5, 5]` → `[0.1, 2, 5, 5]`; arrow kept; text `'  needs a design pass '` → `'needs a design pass'`; exact output objects as listed in the test.
2. `drops anything malformed`: of `null`, `'pen'`, kind `laser`, colour `pink`, 3 points, `NaN`, empty points, 6-number arrow, blank text, empty id, `h`, duplicate `h` → only `['h']`; `cleanInk('nope')` → `[]`.
3. `keeps sizes sensible`: sizes 100, -1, undefined → 10, 0.25, 1.
4. `caps how much a board keeps`: 2005 strokes → 2000 kept, first is `s5`; a pen with 4020 numbers → 4000; a 550-char label → 500.

### `src/main/viewPage.test.ts`

1. buildViewPage › `puts what every view gets at the top of the head`: injected block starts right after `<head>`; `window.nodus` appears before the page's `<title>`.
2. buildViewPage › `keeps a page with no doctype out of quirks mode`: `<html><body>hi</body></html>` → starts `<!doctype html><html><head>`; a fragment → `^<!doctype html><html><head>.*<body><p>just a fragment</p></body></html>$`.
3. buildViewPage › `never puts anything in front of the doctype`: `<!DOCTYPE html><body>hi</body>` → starts `<!DOCTYPE html><head>`.
4. buildViewPage › `carries the content security policy, with no network`: page contains `content="<VIEW_CSP>"`; CSP contains `connect-src 'none'` and `default-src 'none'`; no `http:`/`https:`.
5. buildViewPage › `boots the page with its data, which can’t close the script early`: data containing `</script><script>alert(1)</script>` → the first script contains `"viewId":"v1"` and `\u003c/script>` and no `<script>alert`.
6. buildViewPage › `is not fooled by a <header> into thinking it found the head`: `<html><header>top</header></html>` → injected before `<header>`, page contains `<html><head><meta charset`.
7. scriptJson › `escapes what would end a script or a JS string`: `'</script>'` → `"\u003c/script>"`; `'a\u2028b\u2029c'` → `"a\u2028b\u2029c"` (escaped); `undefined` → `null`.

### `src/main/views.test.ts` (temp userData; real store with `save` mocked; pty mocked: spawn resolves, `isLive` false; `getStatus` → `done`; `hasTranscript` false; `sessionList` → `[{id:'s1', title:'Card'}]`; `gitStatus` → null; `shellCommand(run)` → `/bin/sh -c run`; the `boardChanged` hook reports the newest view loaded at its version on the next tick; each test a new project whose folder is the temp dir, card `s1` at (2000, 0))

1. normalizeCommands › `keeps a command line the user allowed allowed, whatever it is called now`: `{name:'list', run:'vercel ls --json', every:10}` with that line allowed → `{name:'list', run:'vercel ls --json', every:10, approved:'vercel ls --json', denied: undefined}`.
2. normalizeCommands › `asks again for a command line that changed`: `vercel ls; rm -rf ~` vs allowed `vercel ls` → `approved` undefined.
3. normalizeCommands › `runs no more than every minute, nor less than weekly`: every `0.2`, `99999`, `-5` → `1`, `10080`, `undefined`.
4. normalizeCommands › `turns down commands it can’t use, saying why`: `'ls'` → `commands must be a list`; `[{name:'x'}]` → `needs a name and a run`; duplicate `x` → `two commands called “x”`.
5. normalizeRefresh › `asks the brain at most every ten minutes`: `{every:1, prompt:' Fetch my tickets '}` → `{every:10, prompt:'Fetch my tickets'}`; `null` → undefined; `{every:60}` → throws `refresh needs every`.
6. create_view and update_view › `puts a view up beside the card of the session that made it`: view `{title:'Deploys', description:'Deploys', from:'s1', version:1, x:2000+288+32, y:0}`; reply has `waiting_for_approval: []`, `errors: []`; `views/<id>/index.html` equals the html.
7. … › `reports the errors the page hit as it loaded`: an error reported 5 ms in → `errors: ['TypeError: rows is undefined']`.
8. … › `sends new data to the page in place, without reloading it`: update with `data:{users:42}` → version stays 1; `notify(project, id, 'data', {users:42})`; get_view data equals it.
9. … › `reloads the page when its html changes`: update html `<p>v2</p>` → version 2; get_view html `<p>v2</p>`.
10. … › `lets a brain leave out its own view’s id, and nobody else`: brain renames its view without `view_id`; `s1` without id → `view_id is required`; `view_id:'__proto__'` → `no view with that id`.
11. … › `needs a title and a page`: missing title → `needs a title`; html `'  '` → `html must be the page`.
12. starting sessions › `stays off unless Claude turns it on`: `startsSessions` undefined; get_view `start_sessions:false`; served page lacks `"startsSessions":true`.
13. starting sessions › `lets the page know when it may start sessions`: `start_sessions:true` → get_view true, page contains `"startsSessions":true`; update `false` → undefined.
14. starting sessions › `takes only "yes" for an answer, and keeps it across other changes`: `start_sessions:'yes'` → undefined; set `true` then rename → still `true`.
15. starting sessions › `lets the user stop it from the board, but not turn it on`: `saveView({startsSessions:true})` doesn't turn it on; after the tool turns it on, `saveView({startsSessions:false})` turns it off.
16. commands › `waits for the user to allow a command before it runs`: get_view `allowed: 'waiting for the user'`; run rejects `waiting for the user to allow it`.
17. commands › `runs an allowed command in the project folder, and sends the page its output`: `printf "%s" "$(pwd)"` after allowing → `{name:'count', code:0, stdout: <realpath of the project folder>}`; `notify(…, 'result', result)`.
18. commands › `remembers a command line the user allowed when Claude puts it back`: allow `echo one`; change to `echo two` → waiting; back to `echo one` → `yes`.
19. commands › `keeps a command the user turned down from running`: deny → run rejects `didn’t allow`.
20. commands › `reports a failing command’s exit code and what it printed`: `echo oops >&2; exit 3` → `{code:3, stderr:'oops\n'}`.
21. the page’s window.nodus › `shows a page the other views, with their data, and not itself`: `views` from A → `[{id:B, title:'B', description:'B', data:{b:2}}]`; `sessions` → `[{id:'s1', title:'Card'}]`.
22. … › `keeps a page’s state, within bounds`: `setState {filter:'open'}` → get_view state; 300000-char string → `too big`.
23. … › `opens web links only, and refuses anything else`: `file:///etc/passwd` → `Only http`; method `readFile` → `isn’t something a view can do`.
24. … › `asks the brain no more than every 15 seconds`: second ask right away → `Asked too soon`.
25. serving and removing › `serves a view’s page with the policy, and nothing else`: `nodus-view://<id>/?v=1` → 200, CSP header contains `connect-src 'none'`, body contains `<p id="n"></p>`; unknown uuid → 404; `nodus-view://<id>/../state.json` → 404.
26. serving and removing › `takes a view off the board with its files and its brain`: after `removeView`, the view and brain node are gone, `hidden` contains `brain`, the folder doesn't exist.

Port notes: Rust tests need fakes for the session API, status, env (`/bin/sh -c`), git, and a board-load simulator for the page check; test 17 needs a real shell (skip or adapt on Windows with PowerShell, as Electron's suite only runs on macOS CI for these). Add golden-fixture tests for `tools/list` (macOS + Windows), `BRIDGE`/`THEME`/`VIEW_CSP` byte equality, and JS-number formatting.

## Rust mapping

### Crates

- **`nodus-tools`**: `answer_mcp` (TOOL-016–TOOL-025), the static tool schemas (store the exact JSON from Data & formats, with `VIEW_GUIDE` spliced in per OS; serde_json with `preserve_order`), `BOARD_MCP_CONFIG`, `BOARD_TOOL_RULES`, `INSTRUCTIONS`, and the `BoardToolHandlers` trait. Pure apart from awaiting handlers. The host side of the relay (TOOL-004–TOOL-013, `APP_CLOSED`, 20 s, 1 MiB) lives in `nodus-host` (02) using constants from `nodus-host-proto`.
- **`nodus-board`**: notes placement and sizing (NOTE-*), `clean_ink` (INK-*), on top of the shared geometry (`find_free_spot`, `dock_rect`, sizes).
- **`nodus-views`**: storage, validation, page serving (`serve_page`, `build_view_page`, `script_json`, `VIEW_CSP`, `THEME`, `BRIDGE`, `VIEW_GUIDE`), commands (tokio `process::Command`; unix: `process_group(0)` + `killpg(SIGKILL)`; Windows: `CREATE_NO_WINDOW`, `taskkill /pid <pid> /t /f`), schedules (30 s tokio interval), brains (idle timers, queue, bracketed paste), page-check waiters, tool handlers for views. Depends on `nodus-store`, `nodus-sessions` (spawn/write/kill/is_live/status), `nodus-platform` (open URL), `nodus-git`.
- **`nodus-core`**: implements `BoardToolHandlers` (the ipc.ts logic: `find_project`, `create_project` + `add_project`, `list_sessions`, `archive_sessions`, `archive_self` + the archive-when-done set, `create_note` with `away`, `session_title`), the `views_*` commands and events, wiring `brain_status` to status changes and `brain_input` to terminal writes, `flush_views` on quit.
- **`nodus-ui`**: view card, frame host, relay, ask bar, title editor, large modal, New view dialog, menu items, confirm text.

### wry: the frame

- **One webview per frame.** A `wry::WebView` built as a child of the GPUI window (`build_as_child`) for each mounted frame: the card's (when `version > 0`) and the large modal's. Bounds follow the card body's on-screen rect every frame of pan/zoom/drag/resize.
- **Scale.** The page must lay out in board units (a 480 × 322 body is a 480 × 322 CSS-px viewport at any zoom) and render scaled by the board zoom: set the webview's page zoom to the board zoom (`WebView::zoom`; WKWebView `pageZoom`, WebView2 `ZoomFactor`) and its bounds to `body rect × zoom`.
- **Custom protocol.** `with_custom_protocol("nodus-view", …)` (or the asynchronous variant) → `serve_page`, with the headers of VIEW-020. On Windows, WebView2 exposes custom schemes as `http(s)://nodus-view.<host>/…`; `serve_page` must accept that form too (R11).
- **Recommended shell page.** Load a tiny internal shell document per webview that contains only `<iframe sandbox="allow-scripts" src="nodus-view://<id>/?v=<version>" title="<view title>">` filling it, plus a relay script. This keeps, without re-implementing: the opaque-origin sandbox (no storage, cookies, popups, `alert`, forms, downloads, top navigation), the unchanged `BRIDGE` (`parent.postMessage`, `e.source === parent`), and navigation blocking (the shell's own CSP `frame-src nodus-view://<id>` blocks the frame from going anywhere else, matching VIEW-028; plus `with_navigation_handler` allowing only the shell, and `with_new_window_req_handler` denying all).
- **Relay mapping.** Shell → Rust: `window.ipc.postMessage(JSON.stringify({…page message, activated}))` for each message from the frame (`e.source === frame.contentWindow`, `nodus === 1`), where `activated = document.activeElement === frame && navigator.userActivation.isActive` evaluated on arrival (VIEW-051). Rust → page: `evaluate_script("__nodusDeliver(<json>)")`, which does `frame.contentWindow.postMessage(msg, '*')`. Rust keys frames by webview, never by message content (VIEW-044).
- **`loaded`.** Rust reports `{loaded: <version that webview was created/navigated for>}` (VIEW-046).
- **Wheel.** The bridge's `{wheel}` messages become synthetic board wheel input at `webview origin + (x, y) × zoom` through the same pan/zoom path as native wheel events (09).
- **Theme.** The webview's appearance must follow the app's Settings theme: macOS set the WKWebView `appearance` (nil for System, `NSAppearanceNameAqua`/`DarkAqua`) on every theme change; Windows `set_theme`/`PreferredColorScheme`. Background colour = `--bg-elevated` of the current theme so no white flash (VIEW-027).
- **Pointer pass-through** while dragging nodes, marquee-selecting or resizing this view (VIEW-141): the webview must stop taking mouse events (e.g. hide it and draw a snapshot, or put a transparent native view above it).
- **Defaults to switch off:** devtools, default context menus (Electron shows none in frames), file drag-drop navigation, back/forward gestures, zoom by pinch inside the webview (a pinch must go to the board through the bridge), text-size autosizing.

### Risks

- **R1 Z-order and clipping (biggest).** Native webviews draw above all GPUI content. In Electron the frame is an ordinary DOM node: notes draw over views, and menus, toasts, the prompt bar, selection bar, minimap, controls, archive panel, dialogs, the terminal modal and other dragged cards all draw over it; it is clipped by the canvas edges and the card's rounded corners (14 px). Needs a snapshot/overlay strategy (WKWebView `takeSnapshot`, WebView2 `CapturePreview`) and a clipping container (NSView `masksToBounds` + corner radius; Windows region). Must be proven in spike S1.
- **R2 Pointer gating** during drag/marquee/resize (VIEW-141) and focus handoff between GPUI and webviews (keys typed in a page must not reach board shortcuts; Escape in a page must not close the modal; ⌘W menu accelerators still work).
- **R3 User activation.** `startSession` gating relies on Chromium's transient activation (about 5 s in Chromium) propagating from the frame to its parent and on the frame being the focused element. WebKit's `navigator.userActivation` support and duration differ; verify in WKWebView or implement an equivalent native "clicked/pressed in this webview within N s and it still has focus" check with N matching Chromium.
- **R4 Engine differences.** Pages Claude wrote were tested on Chromium; WKWebView differs (CSS/JS features, scrollbars, font fallback). "Same UI" for pages can only be checked case by case; collect sample views (made-up content) as visual fixtures.
- **R5 Theme sync** of webviews with the app's System/Light/Dark setting (VIEW-026).
- **R6 JS number semantics.** `Math.round` rounds halves toward +∞ (`Math.round(-2.5) = -2`; Rust's `f64::round` gives -3): use `(x + 0.5).floor()` with JS's edge cases. `Number(x)` coercion for `width`/`height` (VIEW-104). JSON numbers must print like JS (`10`, `0.1`, `-0` → `0`, exponents as JS): use an ECMAScript number formatter (e.g. `ryu-js`) for tool output, `state.json` and `runtime.json`.
- **R7 String lengths and trimming.** Every limit and `fitNote` counts UTF-16 code units (`clip`, `NOTE_MAX_CHARS`, `MAX_TITLE_CHARS`, error 2000, ink 64/500, stdout/stderr caps, JSON size checks). JS `trim()` also strips U+FEFF and the JS line terminators; Rust's `trim()` doesn't strip U+FEFF. Implement `js_trim` and UTF-16 slicing (and decide how a cut through a surrogate pair is written; JS keeps the lone surrogate).
- **R8 JSON fidelity.** Key order (insertion order) for tool results and `tools/list`; `undefined` → absent keys; lone surrogates from pages/Claude can't round-trip through Rust `String` (serde_json rejects `\ud800` alone): decide a policy (replace with U+FFFD) and document it.
- **R9 Timers and concurrency.** 500 ms runtime debounce per view, 30 s ticker, 5 min idle timers with re-arm, 200 ms Enter after paste, 500 ms queued delivery, 3000 ms load wait + 500 ms settle, 15 s page-ask window, 3 s start gap, shared in-flight command runs, page-check waiters resolved by reports. All need cancellation on view removal and on quit (flush).
- **R10 Process trees and encodings.** Kill whole trees on timeout; no console windows on Windows; per-chunk UTF-8 decoding (VIEW-077) can split multi-byte characters into U+FFFD exactly as Node does; keep it per chunk or decode the whole buffer (unclear which to prefer; the difference is visible only at chunk boundaries).
- **R11 Custom-scheme URLs on WebView2** (`http://nodus-view.<id>/`) and whether `isSecureContext` is true for the scheme in WKWebView (Electron registers it `secure: true`, so pages can use secure-context APIs like `crypto.subtle`). Verify both.
- **R12 Structured clone vs JSON.** Electron passes page args and events by structured clone (Dates, Maps, `undefined` survive in memory until written to `runtime.json`); a JSON relay changes what a page gets back from `setState` events in the same session. Low impact; document.
- **R13 Off-screen culling.** If Rust doesn't keep webviews for off-screen views, `create_view`/`update_view` page checks would time out where Electron reports errors (Electron renders every view unless the board has more than 150 cards).
- **R14 Relay timing.** Host relay timeout 20 s, newest-client selection and `APP_CLOSED` answers must match 02 byte for byte so a 0.9.4 host and a Rust app (and vice versa) interoperate.

## Parity checklist

One line per behavior ID, in the order above. Tick only when the Rust build matches the Electron app and the behavior's ported tests pass.

- [x] TOOL-001 The MCP server's name is nodus
- [x] TOOL-002 BOARD_MCP_CONFIG (passed as --mcp-config to every claude process when the host's welcome said mcp: true; argv owned by 03) is exactly the string…
- [x] TOOL-003 BOARD_TOOL_RULES (passed after --allowedTools) is exactly, in order: mcp__nodus__create_note, mcp__nodus__update_note, mcp__nodus__list_notes, mcp__nodus__list_projects…
- [x] TOOL-004 The host serves the MCP endpoint on its hook HTTP server (127.0.0.1, hook port) at path /mcp (MCP_PATH)
- [x] TOOL-005 A request is authorized only if its Authorization header matches ^Bearer (\S+)$ and the token equals the host's hook token (constant-time compare, length must match)…
- [x] TOOL-006 Authorized, known session, method DELETE → 204 empty
- [x] TOOL-007 The POST body is accumulated as text
- [x] TOOL-008 A body that isn't valid JSON → 200, Content-Type: application/json, body {"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}
- [x] TOOL-009 The parsed body may be one message or an array (batch)
- [x] TOOL-010 Each request is relayed to the app: the newest connected app client (last added to the host's client set; only clients that said hello with the host's PROTOCOL are in…
- [x] TOOL-011 No client, timeout, or a reply of undefined → the host's own answer, with APP_CLOSED = Nodus isn’t open, so the board can’t be reached right now.: for tools/call →…
- [x] TOOL-012 All requests of a batch are relayed concurrently
- [x] TOOL-013 An exception while serving /mcp → logged mcp request failed, 500 if headers weren't sent yet, then the response ends
- [x] TOOL-014 App side: on a host mcp message the app calls answerMcp(sessionId, message) asynchronously (after a microtask), awaits it, logs a rejection as…
- [x] TOOL-015 Because the server lives in the host, a session keeps its tools across app restarts
- [x] TOOL-016 Every response has jsonrpc: "2.0" and id echoed from the request (undefined id → key omitted)
- [x] TOOL-017 initialize → result {protocolVersion, capabilities: {tools: {}}, serverInfo: {name: "nodus", version: <app version, e.g
- [x] TOOL-018 ping → result {}
- [x] TOOL-019 tools/list → result {tools: [...]}: the 6 common tools then the 4 view tools (order in Data & formats) for every session
- [x] TOOL-020 Any other method → error: {code: -32601, message: "Unknown method <method>"} (e.g. Unknown method resources/list)
- [x] TOOL-021 tools/call answers only once the tool's handler finished (view tools wait for the page check, VIEW-108)
- [x] TOOL-022 tools/call arguments = params.arguments when it is a non-null object (arrays included), else {}
- [x] TOOL-023 A successful tool call's result is {content: [{type: "text", text: JSON.stringify(value, null, 2)}]} (2-space pretty JSON, no isError key)
- [x] TOOL-024 A failing tool call's result is {content: [{type: "text", text: <error message>}], isError: true} (a JSON-RPC result, never a JSON-RPC error)
- [x] TOOL-025 Checks run in this order before dispatch: handlers not configured → The board is not available
- [x] TOOL-026 Note patches take text only when it is a string and color only when it is one of FOLDER_COLORS (blue, purple, pink, red, orange, yellow, green, graphite)
- [x] TOOL-027 The note tools work on the caller's own board when project is undefined (absent)
- [x] TOOL-028 Project lookup (findProject): ref must be a string non-empty after trim, else project must be a project’s id or name, from list_projects
- [x] TOOL-029 create_note → createNote(target, caller, patch, away) (NOTE-*), then ev:board-changed(target)
- [x] TOOL-030 sessionTitle(projectId, sessionId): a view's brain → its view's title, or A view if empty/missing
- [x] TOOL-031 update_note: note_id must be a string, else note_id is required (checked before the project is resolved)
- [x] TOOL-032 list_notes → listNotes(target, caller) (NOTE-016)
- [x] TOOL-033 list_projects → every project in state.projects order as {id, name, folder: repoPath, archived?: true, thisSessionsBoard?…
- [x] TOOL-034 create_project: path must be a string non-empty after trim, else path is required
- [x] TOOL-035 list_sessions (runs only) → sessionList(project, caller, include_archived === true): the board's cards except the caller and views' brains, archived ones only when…
- [x] TOOL-036 archive_sessions (runs only): session_ids keeps only string items (non-array → [])
- [x] TOOL-037 archive_self errors, checked in order: node missing → This session is no longer on the board
- [x] TOOL-038 On the next status done for a remembered session (status hook path in ipc.ts, after finishRun), it is forgotten and, if its node still exists and isn't archived…
- [x] TOOL-039 list_views, get_view, create_view, update_view call the views handlers (VIEW-102–VIEW-114) with the caller's own project
- [x] TOOL-040 Claude cannot delete notes or views
- [x] TOOL-101 Every session gets list_board, draw_on_board, update_drawing and erase_from_board, after archive_self (added after 0.9.4)
- [x] TOOL-102 list_board: cards, free notes, views, shapes and drawn links with their boxes, and linked_to_this_session in words
- [x] TOOL-103 draw_on_board checks everything before drawing anything
- [x] TOOL-104 Default sizes, the row, and the drawing beside the caller's card (or at)
- [x] TOOL-105 A link's ends: a key, self, or the id of something drawn
- [x] TOOL-106 Shapes and links saved with by and createdAt, ev:board-changed, ids returned
- [x] TOOL-107 update_drawing changes shapes and only moves cards, notes and views
- [x] TOOL-108 update_drawing changes links' labels, colours and ends
- [x] TOOL-109 erase_from_board takes shapes and links, a shape's links with it, never cards, notes or views
- [x] TOOL-110 INSTRUCTIONS end with a few sentences on shapes and links
- [x] TOOL-111 look_at_board: the board or an area as a PNG drawn from the store, with tags keyed to ids (added after 1.0.17)
- [x] TOOL-112 draw_on_board puts up pictures from files, checked and copied, at their own proportions; not on a server
- [x] TOOL-113 Ink marks (circle, underline, rect, arrow, text, pen) drawn, listed by list_board, erased by id
- [x] TOOL-114 Sessions may draw and write notes unasked, small and near their card; the user's drawings change only when asked
- [x] NOTE-001 Note text is text.trim() then its first 20000 UTF-16 code units (NOTE_MAX_CHARS)
- [x] NOTE-002 fitNote(text): lines = text.split('\n')
- [x] NOTE-003 A short note stays square at 216 × 216: w is stored only when ≠ 216, h only when ≠ 216 (otherwise the keys are absent)
- [x] NOTE-004 Colour defaults to yellow
- [x] NOTE-005 Obstacles for placement: active cards (not archived, no runOf, no viewOf) at cardSize, free notes (no sessionId, i.e. not stickers) at noteSize, and every view at…
- [x] NOTE-006 Preferred spot, first that applies: (1) under the newest (createdAt) free note whose from is the writer: {x: last.x, y: last.y + noteSize(last).h + 32}
- [x] NOTE-007 The final position is findFreeSpot(preferred, obstacles, dock, size) (geometry, 03/09) with the fitted size
- [x] NOTE-008 A run, an archived card and a session on another project have no active card on the target board, so their first note goes beside the rightmost card (NOTE-006 (4))
- [x] NOTE-009 The note record: {id: <uuid v4>, text, color, x, y, w?, h?, from: <writer session id>, fromProject?: <writer's project id>, fromTitle?: <writer's title at write time>…
- [x] NOTE-010 Creating a note writes it into board.notes[id] and saves state.json
- [x] NOTE-011 updateNote finds the note only as an own property of the board's notes (ids like __proto__, constructor, toString, hasOwnProperty are not found), else There’s no note…
- [x] NOTE-012 updateNote with text !== undefined: the new text goes through NOTE-001; blank throws A note needs some text before anything changes (the colour is not applied either)
- [x] NOTE-013 updateNote sets color only when given (and valid, TOOL-026)
- [x] NOTE-014 An empty patch changes nothing but still saves and succeeds
- [x] NOTE-015 clampNoteSize(n) = Math.round(min(1200, max(120, n))), used by board:save-note (10) for user resizes
- [x] NOTE-016 listNotes(project, self): every note on the board (stickers included), oldest createdAt first, as {id, text, color, writtenByYou?…
- [x] INK-001 cleanInk(raw): a non-array gives []
- [x] INK-002 Only the last 2000 items (INK_MAX_STROKES) of the array are considered (raw.slice(-2000)), before any filtering
- [x] INK-003 An item that is falsy or not an object is dropped
- [x] INK-004 id must be a non-empty string of at most 64 UTF-16 units and not seen earlier in this pass (first occurrence wins)
- [x] INK-005 kind must be one of pen, arrow, text, rect and color one of graphite, red, orange, green, blue, purple
- [x] INK-006 points must be an array whose every element is a finite number (no NaN, no strings)
- [x] INK-007 points needs length ≥ 2 and even
- [x] INK-008 A text stroke's text is trim()ed and cut to 500 UTF-16 units (INK_MAX_TEXT)
- [x] INK-009 Every kept point is rounded to one decimal: Math.round(n × 10) / 10 (JS rounding: halves go toward +∞)
- [x] INK-010 size: a finite number → Math.round(min(10, max(0.25, n)) × 100) / 100; anything else (missing, non-number) → 1
- [x] INK-011 Output objects are {id, kind, color, points, size} plus text only for text strokes, in that key order, in input order
- [x] INK-012 board:save-ink(projectId, strokes) replaces board.ink with cleanInk(strokes) and saves (the ink UI is 11's)
- [x] INK-013 board:add-ink / board:remove-ink change single strokes, leaving the rest (hosted, added after 1.0.12)
- [x] VIEW-001 View ids are lowercase UUID v4. Any id that ends up in a path must match ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$, else Not a view id is thrown
- [x] VIEW-002 A view's files live in <userData>/views/<viewId>/: index.html (the page exactly as Claude sent it, UTF-8) and runtime.json
- [x] VIEW-003 Reading index.html returns '' on any error (missing file, bad id)
- [x] VIEW-004 Writing index.html creates the folder recursively first
- [x] VIEW-005 Runtime (data, state, command results, errors, refreshedAt) is cached in memory per view
- [x] VIEW-006 Runtime saves are debounced per view: each save request restarts a 500 ms timer
- [x] VIEW-007 On quit (shutdown) every pending runtime is written immediately (failure logs [views] flush failed), timers cleared, and the 30 s ticker stopped
- [x] VIEW-008 Deleting a view's files cancels its pending runtime write, drops its cache and removes the folder recursively (force)
- [x] VIEW-009 clip(s, max): a non-string gives ''
- [x] VIEW-010 Limits: page ≤ 2 × 1024 × 1024 UTF-8 bytes
- [x] VIEW-011 clampEvery(value, min): a finite number > 0 → min(10080, max(min, Math.round(value))) (10080 = a week in minutes)
- [x] VIEW-012 normalizeCommands(raw, remembered = {}): non-array → commands must be a list of { name, run, every }
- [x] VIEW-013 normalizeRefresh(raw): null/undefined → undefined
- [x] VIEW-014 checkJson(value, max, what): JSON.stringify throws (cycle, BigInt) → <what> must be JSON
- [x] VIEW-015 checkHtml(html): not a string or blank after trim → html must be the page, as one HTML document
- [x] VIEW-016 findView(projectId, id): the id must be a string that is an own property of that board's views, else There’s no view with that id on this…
- [x] VIEW-017 A command is *waiting* when approved !== run && denied !== run
- [x] VIEW-018 The scheme is nodus-view (VIEW_SCHEME), registered before app ready as privileged {standard: true, secure: true}, and every request to it is answered by servePage(url)
- [x] VIEW-019 servePage: the URL's hostname is the view id and the pathname must be exactly / (query ignored, so ?v=<version> is fine)
- [x] VIEW-020 A served page is 200 with headers exactly: Content-Type: text/html
- [x] VIEW-021 The body is buildViewPage(index.html || EMPTY_PAGE, boot) where boot = {viewId, title, startsSessions, data, state, results} from the view's current title/startsSessions…
- [x] VIEW-022 buildViewPage inserts the injected block (VIEW-023) right after the first <head …> tag (/<head(\s[^>]*)?>/i, so <header> doesn't match)
- [x] VIEW-023 The injected block is the concatenation, with no separators: <meta charset="utf-8"> + <meta http-equiv="Content-Security-Policy" content="<VIEW_CSP>"> + <meta…
- [x] VIEW-024 scriptJson(value) = JSON.stringify(value) ?? 'null' with every < replaced by <, U+2028 by  , U+2029 by   (the six-character escapes)
- [x] VIEW-025 The CSP (VIEW_CSP, verbatim in Data & formats) allows inline scripts and styles only, images/media from data:/blob:, fonts from data:, and nothing else: no network, no…
- [x] VIEW-026 The injected theme (Data & formats) defines --nodus-* colours for light and, under prefers-color-scheme: dark, dark
- [x] VIEW-027 A view's page shows in a frame with sandbox="allow-scripts"…
- [x] VIEW-028 A frame may only navigate to a nodus-view:// URL, and once it shows a view page, only to that same view id
- [x] VIEW-029 Each mounted frame registers with the relay under its window: {projectId, viewId, version}
- [x] VIEW-030 The card renders its frame only when version > 0 and a board project is open
- [x] VIEW-031 window.nodus has viewId, title, canStartSessions (Boolean(boot.startsSessions)), data, state, results (an object, boot.results || {}), and methods on, setState, run…
- [x] VIEW-032 A call posts {id: ++seq, method, args: args || {}, nodus: 1} to parent with target origin '*' (seq starts at 0 so the first id is 1) and returns a Promise
- [x] VIEW-033 Incoming messages are accepted only when e.source === parent and e.data.nodus === 1; a message with a truthy id is a reply (VIEW-032) and nothing else
- [x] VIEW-034 On an event message: data → sets current data and nodus.data
- [x] VIEW-035 nodus.on(event, fn) adds fn and returns an unsubscribe function
- [x] VIEW-036 nodus.setState(s) sets current state and nodus.state immediately, then calls setState with {state: s}
- [x] VIEW-037 Method argument shapes: run(name) → {name}
- [x] VIEW-038 Errors are reported to the board as {error: <text>, nodus: 1} where text = err.stack || err.message when either exists, else String(err), cut to 2000 chars
- [x] VIEW-039 On window load the page posts {loaded: true, nodus: 1}
- [x] VIEW-040 A click (capture phase, on document) whose target is inside an a[href]: an href attribute starting with # is left alone
- [x] VIEW-041 Wheel routing (listener on window, non-passive): ignored if defaultPrevented or both deltas are 0. A ctrlKey wheel (pinch) always goes to the board
- [x] VIEW-042 scrolls(el, dx, dy): walks from the target up to (not including) <html>, element nodes only: an element scrolls when its computed overflowY (for dy) / overflowX (for dx)…
- [x] VIEW-043 A wheel that goes to the board is preventDefault()ed in the page and posted as {wheel: {dx, dy, mode, x, y, ctrl, shift, alt, meta}, nodus…
- [x] VIEW-044 The board knows which view a message is from only by the frame window that sent it
- [x] VIEW-045 {error: string} → views.report(projectId, viewId, {error}) (fire-and-forget; main ignores reports for views that have gone)
- [x] VIEW-046 {loaded: truthy} → views.report(projectId, viewId, {loaded: <the version the frame was registered with>})
- [x] VIEW-047 {wheel: object} → replayed on the frame element as a synthetic, bubbling, cancelable wheel event: deltas/mode from the message (non-finite → 0), clientX = frameRect.left…
- [x] VIEW-048 A request needs a numeric id and a string method, else ignored
- [x] VIEW-049 openSession: args.id must be a string, else nodus.openSession needs a session id
- [x] VIEW-050 startSession, in order: prompt (string, trimmed) empty → nodus.startSession needs a prompt
- [x] VIEW-051 *Activated* = at the moment the request arrives, the board document's focused element is this frame AND the board window has transient user…
- [x] VIEW-052 Every other method goes to main as views.call(projectId, viewId, method, args) (VIEW-060–VIEW-071)
- [x] VIEW-053 Error text returned to pages, dialogs and the card is the message with any leading Error invoking remote method '<channel>':  and an optional following Error: …
- [x] VIEW-054 Main's ev:view(projectId, viewId, event, payload) is posted as {nodus: 1, event, payload} to every frame showing that view
- [x] VIEW-055 The board posts {event: 'board'} to all frames 300 ms after its cards, notes or views last changed (trailing debounce; also once 300 ms after the board mounts), and…
- [x] VIEW-056 The view must have startsSessions
- [x] VIEW-057 At most one start per view every 3000 ms (VIEW_START_GAP_MS, in-memory per board): sooner → This view just started a session
- [x] VIEW-058 The new session is an expanded card (720 × 750) placed with findSpotBeside({x: view.x, y: view.y…
- [x] VIEW-059 Main stores fromView only for a Claude session whose fromView is an own property of that board's views
- [x] VIEW-060 callView first resolves the view on that board (VIEW-016)
- [x] VIEW-061 The page's rate-limit and other per-view state are keyed by view id
- [x] VIEW-062 sessions → sessionList(project, null, false) (TOOL-035 shape: active cards, no brains)
- [x] VIEW-063 notes → every note, oldest first, as {id, text, color}
- [x] VIEW-064 views → every other view on the board, in board insertion order (not sorted), as {id, title, description, data} with data from that view's runtime
- [x] VIEW-065 view → {id, title, description, data} of findView(project, args.id) (may be the page's own view)
- [x] VIEW-066 git → waits for the login-shell env, then gitStatus(repoPath) (05)
- [x] VIEW-067 setState → checkJson(args.state, 262144, 'The state')
- [x] VIEW-068 run → runCommand(project, view, String(args.name ?? '')) (VIEW-073)
- [x] VIEW-069 ask: if the previous page ask for this view was less than 15000 ms ago → Asked too soon: a view can ask its brain once every 15 seconds
- [x] VIEW-070 openUrl: args.url must be a string matching /^https?:\/\//i, else Only http and https links open
- [x] VIEW-071 Any other method → nodus.<method> isn’t something a view can do
- [x] VIEW-072 openSession/startSession never reach main (the relay answers them)
- [x] VIEW-073 runCommand(project, view, name): unknown view → throws synchronously (VIEW-016)
- [x] VIEW-074 While a run of the same view + command name is going, another request gets the same promise (no second process)
- [x] VIEW-075 When the run finishes and the view still exists, its result is stored as runtime.results[name], a save is scheduled and ev:view(…, 'result', result) is sent
- [x] VIEW-076 A run waits for the login-shell env, then spawns the user's shell on the command…
- [x] VIEW-077 stdout/stderr chunks are decoded as UTF-8 per chunk and appended while the collected text is shorter than its cap (stdout 1048576, stderr 65536 chars)
- [x] VIEW-078 After 120000 ms (COMMAND_TIMEOUT_MS) the whole process tree is…
- [x] VIEW-079 A spawn that throws, or a child error event, resolves with code: null and error = the error message
- [x] VIEW-080 A result is {name, code, stdout, stderr, at, durationMs, error?}: code is the exit code (or null when killed by a signal / timed out / failed to start), at = finish time…
- [x] VIEW-081 approveCommand(project, view, name, allow): unknown command name → That command is gone
- [x] VIEW-082 Allowing a command runs it straight away (errors ignored)
- [x] VIEW-083 An answer is tied to the exact command line: any change asks again
- [x] VIEW-084 refreshView(project, view): if the view has a refresh, refreshedAt = now, schedule a save, and await askBrain(…, refresh.prompt…
- [x] VIEW-085 A ticker runs every 30000 ms, started once by configureViews at app start, while the app is open (whether or not any board is showing)
- [x] VIEW-086 Each tick visits every non-archived project's views
- [x] VIEW-087 A view with refresh is due when now − (runtime.refreshedAt ?? view.createdAt) ≥ refresh.every × 60000; refreshedAt = now and a save are done before asking
- [x] VIEW-088 A brain is a board node {sessionId: <uuid>, x: view.x, y: view.y, archived: false, title: "View: <view title>", createdByApp: true, viewOf: <view id>, autoMode: true…
- [x] VIEW-089 startBrain(project, view, prompt?): adds the node and sets view.sessionId, then spawns claude: fresh session (resume: false), initialPrompt…
- [x] VIEW-090 brainArgs(project, node) = ['--append-system-prompt', brainPrompt(view)] for a node whose viewOf view exists, else none
- [x] VIEW-091 Renaming a view (tool or user) also sets its brain node's title to View: <new title>
- [x] VIEW-092 ensureBrain(project, view) returns the brain's session id, starting an idle brain (no prompt) when the view has none (its node is missing)
- [x] VIEW-093 askBrain(project, view, prompt, origin): prompt clipped to 4000; empty → Say what the view’s brain should do
- [x] VIEW-094 Queued asks for a brain are joined as <queued>\n\n<new> and only the last 4000 characters are kept
- [x] VIEW-095 Typing into a brain: cancel its idle timer, write ESC[200~<text>ESC[201~ (bracketed paste, \x1b[200~…\x1b[201~), then 200 ms later write \r
- [x] VIEW-096 On every status change of a brain (brainStatus, called from the status machine): cancel its idle timer
- [x] VIEW-097 When the idle timer fires: if the brain is still done and nothing was typed into it for ≥ 5 minutes (time since the last user input recorded while idle, or ever), its…
- [x] VIEW-098 Any terminal input the user sends to a session (session:write) records "last input" for it only while it has an idle timer
- [x] VIEW-099 A brain is not drawn on the board and isn't a card anywhere: it is filtered out of the board's cards, list_sessions, nodus.sessions(), ⌘K search, notifications, and…
- [x] VIEW-100 views:create(projectId, {description, x, y}) waits for the login-shell env, checks the project exists (Unknown project <id>), then createView: description =…
- [x] VIEW-101 The board's create (dialog, context menu, title-bar menu): preferred top-left = {at.x − 240, at.y − 180} for "here" or the viewport centre for a 480 × 360 view
- [x] VIEW-102 create_view checks, in order: title = clip(title, 80) empty → A view needs a title
- [x] VIEW-103 create_view placement (placeView): preferred = right of the caller's active card (x + cardSize.w + 32, same y), else right of the rightmost active card, else {dock.x +…
- [x] VIEW-104 Size from tools: clampViewSize({w: Number(width), h: Number(height)}): non-finite → undefined (default 480/360)
- [x] VIEW-105 The new view: {id: uuid, title, description: clip(description, 4000) || title, x, y, w?, h?, from: <caller>, commands?: (only if non-empty), refresh?, startsSessions?…
- [x] VIEW-106 Then: write index.html; runtime data = args.data
- [x] VIEW-107 A view put up by a tool has no brain until it first needs one (an ask, refresh, ⌨, Build)
- [x] VIEW-108 Page check (checkPage(view, since)): wait up to 3000 ms (LOAD_WAIT_MS) for a loaded report with version ≥ the view's version
- [x] VIEW-109 update_view resolves the view: view_id undefined → the caller's own view when the caller is a brain, else view_id is required: this session isn’t a view’s brain
- [x] VIEW-110 update_view validates everything before changing anything: html (when given) via checkHtml
- [x] VIEW-111 update_view then applies: non-empty title → rename (VIEW-091)
- [x] VIEW-112 update_view returns {view_id, version, waiting_for_approval: [...], ...check} where the page check (VIEW-108) runs only when html was given (else nothing extra)
- [x] VIEW-113 get_view (own view when view_id is undefined, VIEW-109 error otherwise as findView) returns {id, title, description, html: <index.html or ''>, data, state, commands…
- [x] VIEW-114 list_views returns every view oldest first as {id, title, description, yours?: true}, yours when the caller is the view's brain or put it up (from)
- [x] VIEW-115 A view starts sessions only after a tool sets start_sessions: true
- [x] VIEW-116 views:save(projectId, patch) (saveView): findView(patch.id)
- [x] VIEW-117 views:remove(projectId, viewId) (removeView): findView
- [x] VIEW-118 Removing a project deletes the files of all its views (brains stop with the project's sessions)
- [x] VIEW-119 views:report (fire-and-forget; any error, e.g. a view that has gone, is swallowed): a non-empty string error → appended as {message: error.slice(0, 2000), at: now}…
- [x] VIEW-120 views:refresh, views:ask (origin user, prompt String(prompt ?? '')), views:brain wait for the login-shell env first
- [x] VIEW-121 The "how a view works" guide (VIEW_GUIDE) is given to brains (in their system prompt) and to every session (in create_view's description)
- [x] VIEW-122 Board nodes: one per view, type: 'view', position (x, y), size viewSize(view) (480 × 360 default), zIndex: 0, dragHandle: '.view-head' (only the title bar drags)…
- [x] VIEW-123 While a view is being dragged/resized (or the board marks it busy) its node keeps the live position (and the live size while resizing) instead of the saved one
- [x] VIEW-124 The card's brain status = the brain card's status when its process is live, else done
- [x] VIEW-125 Title bar left: when working or waiting, a small status dot (data-status = status) with tooltip Claude is working on this view / Claude needs you to carry on with this…
- [x] VIEW-126 Title: the view's title (single line, ellipsis) with the description as tooltip
- [x] VIEW-127 Renaming: an input (placeholder View name, spellcheck off) with the current title, focused and all selected
- [x] VIEW-128 Tools, right-aligned, in order: ⟳ Refresh (only when the view has a refresh or at least one allowed command), ✎ Change view, ⌨ Open its session, ⤢ Open large, ⋯ More
- [x] VIEW-129 ⟳: tooltip Refresh: run its commands, and ask its brain for fresh data when the view has a refresh, else Refresh: run its commands
- [x] VIEW-130 ✎: tooltip Ask Claude to change this view, aria-label Change view, toggles the ask bar (aria-expanded)
- [x] VIEW-131 Ask bar: an input (placeholder Tell Claude what to change…, spellcheck off) focused on open without scrolling, and a primary Send…
- [x] VIEW-132 ⌨: tooltip Open its brain, the Claude session that builds this view, aria-label Open its session: views.brain then opens that session's terminal
- [x] VIEW-133 ⤢: tooltip/aria-label Open large, disabled while version === 0; opens the large modal (VIEW-147)
- [x] VIEW-134 ⋯: tooltip/aria-label More
- [x] VIEW-135 Approvals: for each waiting command, in command order: This view wants to run[ <every label>]: followed by the full command line in a code block, then Don’t allow…
- [x] VIEW-136 Every label (everyLabel(minutes)): not a whole number of hours → every minute (1) / every <n> minutes
- [x] VIEW-137 With version === 0, the body shows an empty state: working → spinning LoaderCircle (18 px), Claude is building this view…, ghost button Watch it work (opens the brain)
- [x] VIEW-138 Build it asks the brain Build this view
- [x] VIEW-139 The card error line (role="status") clears itself 6000 ms after it was last set
- [x] VIEW-140 A selected view (not renaming) shows resize handles
- [x] VIEW-141 While any node is being dragged, a selection rectangle exists, or this view is being resized, frames don't take the pointer, so drags and resizes pass over pages
- [x] VIEW-142 A page takes clicks, typing and scrolling on the first interaction: there is no "click to use" shield and clicking in the page doesn't select the…
- [x] VIEW-143 View menu (right-click on the view, or ⋯)…
- [x] VIEW-144 Deleting asks first (danger confirm): one view → title Delete view?, message “<title>” and its page go, and the Claude session that keeps it up to date stops
- [x] VIEW-145 Removing in the board: the view and its brain card leave the board state immediately
- [x] VIEW-146 patchView applies the patch's defined fields to the board state immediately and sends views.save (not awaited)
- [x] VIEW-147 Large modal: a modal over the board (view-modal) with a header (LayoutDashboard 15 px, the title as heading, a close button X 15 px with tooltip/aria-label Close) and a…
- [x] VIEW-148 Dragging views: a view dropped onto the archive or the note pad bounces back to where its drag started
- [x] VIEW-149 Keyboard on the board: Backspace/Delete (no ⌘/Ctrl, no ⌥) with views selected → delete them, asking…
- [x] VIEW-150 A source handle on the card's right edge (1 × 1 px, invisible, not connectable) anchors the dashed lines to sessions the view started
- [x] VIEW-151 Opened from the board's context menu New view here… (LayoutDashboard, disabled over the archive; the view is placed around the clicked point) and from the New Session…
- [x] VIEW-152 Content: icon tile (LayoutDashboard 16 px, stroke 2.2), title New view, subtitle Say what it should show and where the data comes from
- [x] VIEW-153 Example chips (label → text), clicking fills the box with the text
- [x] VIEW-154 Create view (or ⌘Enter / Ctrl+Enter anywhere in the dialog) with a non-blank description: everything disables, Create view shows a spinning LoaderCircle (14 px) before…
- [x] VIEW-155 A view records whether its brain runs in auto mode: the user's views do, a tool's only when the calling session does…
- [x] VIEW-156 update_view from a session not in auto mode takes the view out of auto mode, and stops a brain still running in it…
- [x] VIEW-157 start_sessions: true only from a session in auto mode or the view's own brain…
- [x] VIEW-158 A brain stopped at a permission prompt is waiting, so the view shows Claude needs the user…
- [x] VIEW-159 The app writes its nodus plugin into userData and every Claude process with the board tools gets --plugin-dir for it…
- [x] VIEW-160 SKILL.md is one file: which tool, the brief, writing a page, VIEW_GUIDE and four example pages…
- [x] VIEW-161 create_view without html needs a description, makes a version 0 view and starts its brain on building it…
