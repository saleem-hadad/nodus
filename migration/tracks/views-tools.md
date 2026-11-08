# Track: views-tools (P3)

- **Branch:** `rust/views-tools`
- **Spec:** `migration/features/04-board-tools-views.md` (TOOL, NOTE, INK, VIEW: 222 behaviors)
- **Crates:** `nodus-tools` (all), `nodus-views` (all), `nodus-board` modules `notes` and `ink`
- **Sources:** `src/main/boardTools.ts`, `notes.ts`, `ink.ts`, `views.ts`, `viewPage.ts` (+ their tests), the
  board-tools handlers and `views:*` handlers in `src/main/ipc.ts`

## Status

- [x] nodus-board `notes`, `ink` — notes.test.ts, ink.test.ts ported
- [x] nodus-tools — boardTools.test.ts ported; golden `tools.json`; ipc.ts handlers on a real store and board
- [x] nodus-views — viewPage.test.ts and views.test.ts ported; golden `page.json`; brains, schedules, timeouts

## How it's checked

- `crates/nodus-tools/tests/fixtures/generate.mjs` runs the real boardTools.ts (with the macOS and the Windows
  guide) and writes `tools.json`: `initialize`, `tools/list` for a session and a run, ids, errors, pretty JSON.
  `tests/protocol.rs` compares every answer byte for byte.
- `crates/nodus-views/tests/fixtures/generate.mjs` runs the real viewPage.ts and writes `page.json`: the CSP, the
  guide on both OSes, `scriptJson`, and `buildViewPage` over 20+ page shapes (no head, `<header>`, odd whitespace,
  `$&`, comments, unterminated tags). The theme and bridge are compared through the pages.
- Re-run either with Node 22 from the repo root: `node --experimental-strip-types crates/<crate>/tests/fixtures/generate.mjs`.
- Brains, idle stops, queues, schedules and the runtime's 500 ms save run on a paused tokio clock with fake
  processes (`tests/brains.rs`); commands run real `sh`/`zsh` processes on Unix (`tests/views.rs`), including the
  2-minute timeout, reached on a paused clock.

## Done (implemented and tested)

- **NOTE-001…016** (`nodus_board::notes`). NOTE-015 is transcripts-board's `clamp_note_size`, re-exported.
- **INK-001…012** (`nodus_board::ink`). INK-012: `ink::save_ink(board, raw)`, or `Board::save_ink(p, clean_ink(&raw))`.
- **TOOL-001…003, TOOL-014, TOOL-016…040** (`nodus-tools`, 29). TOOL-030 is `Board::session_title` and TOOL-038
  `Board::archive_if_asked` via `Board::on_done` (transcripts-board, SESS-157/158), set by `archive_self` here and
  tested end to end. TOOL-014: `McpRelay` implements the host client's `BoardTools`.
- **VIEW-001…026, VIEW-031…043, VIEW-060…100, VIEW-102…121** (`nodus-views`, 100). Notes:
  - VIEW-018/VIEW-019: `serve_page` takes `nodus-view://<id>/` and WebView2's `http(s)://nodus-view.<id>/`; registering
    the scheme with wry is the UI track's.
  - VIEW-026: the theme CSS is served verbatim; making the webview's appearance follow the app's theme is UI.
  - VIEW-031…043: the bridge script is served byte for byte as 0.9.4's; what it does runs in the webview.
  - VIEW-099: brains are left out of `list_sessions`/`nodus.sessions()` (`Board::session_list`, SESS-159) and refused
    by `archive_sessions`/`archive_self` here; the board's cards, ⌘K search and notifications are other tracks'.

## Deferred

- **TOOL-004…013, TOOL-015**: the `/mcp` endpoint, auth, batching, `APP_CLOSED` and the relay live in nodus-host
  (track host, merged). The server here answers what it relays.
- **UI (54)**: VIEW-027…030 (the frame), VIEW-044…059 (the relay on the board side, `startSession`, `openSession`,
  wheel replay, posting events to frames), VIEW-101 (placing a view made from the board), VIEW-122…154 (the card,
  ask bar, menus, large modal, New view dialog).

## What nodus-core must implement and wire

- **Board tools.** `nodus_tools::ProjectAdder` (`addProject`: realpath, `Folder not found`/`Not a folder`, existing
  project un-archived, new id/name/colour; returns `(Project, added)`) and `nodus_tools::ViewTools` (four one-liners to
  `Views::{list_views_for_tool, get_view_for_tool, create_view_from_tool, update_view_from_tool}`, errors
  `to_string()`). Then `StoreHandlers::new(StoreHandlersConfig { store, board, sessions, events, projects, views,
  clock, home })`, `McpServer::new(app_version, nodus_views::VIEW_GUIDE, Some(handlers))`, and
  `Wiring.board_tools = Arc::new(McpRelay(Arc::new(server)))`. `BOARD_MCP_CONFIG`/`BOARD_TOOL_RULES` feed
  `nodus_sessions::launch::BoardTools` when the host relays MCP.
- **Views.** `nodus_views::BrainLauncher` (`spawn(SpawnOptions)` = `pty.spawnSession` through the host client with
  `launch::command_for`; `write` = `HostClient::write`) and `nodus_views::ViewHost` (`git_status` from nodus-git;
  `open_url` defaults to `nodus_platform::shell::open_external`). `Views::new(ViewsConfig { user_data, store, board,
  sessions, brains, host, events, env, clock, runtime })`, then `start_schedules()` at startup and `flush()` on quit.
- **Calls into views.** Every status change → `Views::brain_status(id, status, live)` (after `Board::on_done`); every
  `session:write` → `Views::brain_input(id)`; `session:open`/`session:send` pass `brain_args(board, node)` to
  `launch::for_open`/`for_send`; removing a project → `Views::remove_project_views(board)`; `views:*` →
  `create`, `save`, `remove`, `call`, `report`, `approve`, `refresh`, `ask`, `brain` (one to one); `board:save-ink` →
  `clean_ink`.

## What the UI track needs for the webview

- A wry custom protocol `nodus-view` whose handler returns `Views::serve_page(url)` as is: status, the four headers in
  order, the body. It accepts WebView2's `http(s)://nodus-view.<id>/` form too.
- A shell page per frame holding `<iframe sandbox="allow-scripts" src="nodus-view://<id>/?v=<version>">`, relaying
  `{nodus: 1, …}` messages from that frame only (spec 04, "wry: the frame"): `{loaded}` → `Views::report(p, v,
  {loaded: <version the frame was made for>})`, `{error}` → `report(…, {error})`, `{wheel}` → board pan/zoom,
  requests → `Views::call` (except `openSession`/`startSession`, the board's), replies `{nodus:1, id, result|error}`.
  `Event::View {project_id, view_id, event, payload}` is posted to every frame of that view as `{nodus:1, event,
  payload}`.

## Notes for the orchestrator

- Lone UTF-16 surrogates can't exist in Rust strings; text is cut with `nodus_domain::js::slice_utf16` (a cut emoji
  becomes U+FFFD). Commands' output is decoded whole rather than chunk by chunk, which differs only where 0.9.4
  split a character between two pipe chunks (R10).
- `runtime.json` keys are written `data, state, results, errors, refreshedAt`; entries that aren't
  `{message, at}` in `errors` are dropped on read, and a `results` that isn't an object reads as `{}`.
- Writing a view's `index.html` failing in `update_view` now fails before any other change; 0.9.4 applied the title
  and the rest first.
