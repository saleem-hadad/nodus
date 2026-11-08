# Track: core (P4)

- **Branch:** `rust/core`
- **Scope:** compose every merged domain into `nodus-core` (the UI-free service that mirrors `window.api` and the
  `ev:*` events), the deferred IDs that are core wiring, the integration debts, headless end-to-end tests.
- **Sources:** `src/main/index.ts` (startup and quit order), `src/main/ipc.ts` (every handler and the glue),
  `src/shared/api.ts`.
- **Owned:** everything in `crates/nodus-core` except `projects.rs`, `groups.rs`, `settings.rs`, `app.rs` and the
  folder-picker `Dialogs` (ui-shell's). Those got one-line hooks only (below).

## Status

| Piece | State |
| --- | --- |
| Debts: claude -p through `nodus_platform::process`; agents on nodus-transcripts' helpers; one `SpawnOptions` | ✅ 5fab53a |
| Tests for HOST-033, 043, 064 (4 s exit), 086 (20 s relay), 090, 098 | ✅ 5de9280 |
| ui-shell's core part of 8a14488 taken as is (stats from boards, reveal through nodus-platform) | ✅ 8608538 |
| Composition: host client, status, notify, boards, views, board tools, updater; launch, shutdown | ✅ 9969b9a |
| API areas: board, sessions, views, updates, menu; projects.stats with real counts; take_open_request | ✅ 9969b9a |
| Unit tests for the fan-out order and the glue | ☐ |
| Headless end-to-end tests (`crates/nodus-core/tests/`) | ☐ |

## How to resume

Read this file, then `git log --oneline main..rust/core`. The next unchecked row is what's left.

## Done (implemented and tested)

(filled in as tests land)

## Deferred

- `actions.*`, `voice.*`, `git.*`, `capture.*`: tracks in flight. Until nodus-actions merges, `run_args` (an action
  run's system prompt, SESS-132) is a private copy in `wiring.rs`; until nodus-git merges, a view's `nodus.git()` gets
  `null`. Capture's `boardOpened` before `board.load` (SESS-071 step 1) and `stopCapture` in `shutdown` (PLAT-032 step
  1) come with capture.

## Integration debts resolved

- ~~`nodus-transcripts::claude::spawn` uses tokio's `Command` directly~~: `claude::run` starts through
  `nodus_platform::process::Command` (timeout, SIGKILL, whole tree, no console window).
- ~~`nodus-sessions::agents` keeps private copies of the discovery.ts helpers~~: it uses nodus-transcripts'
  `read_lines`, `assistant_parts`, `describe_tool_use`, `message_text`, `is_task_notification`, `time_of`, `truncate`,
  `list_agent_files_in`, `is_agent_id`, `AgentFile`, `mtime_ms`.
- ~~`nodus-sessions::agents` reads a timestamp with no zone as no time~~: there's one `Date.parse`
  (`nodus_transcripts::date::parse`). It reads the variants agents.rs had checked against Node 22 (`t`, `z`, a space
  for `T`, `±HHmm`, a date with `Z`) and a time without an offset as UTC, as transcripts did (V8: local time).
- ~~`nodus-sessions::launch::SpawnOptions` may duplicate a type the host client defines~~: the client already takes
  `launch::SpawnOptions`; there is one.
- ~~`nodus-board`'s `BoardSessions` and `Launcher` must not touch the `Store`~~: `CoreSessions` reads only the host
  client and the status machine; `CoreLauncher` reads the store only from its async methods, which the board calls
  after letting go of the state.
- ~~The status machine and the host client rely on `process_started` / `process_exited` being called where pty.ts
  tracks and untracks a session~~: verified in `client/mod.rs`: `track` (spawned, each welcome session before its
  history) calls `process_started`; `exited` (exit message, host lost, `stop_all` leftovers) calls `process_exited`
  only for a session that was running. pty.ts's `startTracking`/`exited` do the same.
- ~~Six host behaviors have no test~~: see the tests row above.

## Small fixes in merged crates

- `nodus-sessions::client::HostClient::kill`: its future is `+ use<>`, so it doesn't borrow the client (edition 2024
  captures every lifetime otherwise).
- `nodus-transcripts::paths::list_agent_files_in(dir)`, factored out of `list_agent_files`.
- `nodus-host::log_panics(&Logger)`, factored out of `main` so HOST-033 has a test.

## Edits in ui-shell's files (one line each)

- `projects.rs` `remove`: `self.nodus.project_removing(id)` (kill the project's processes, unwatch, delete its views'
  folders) in place of the unwatch line.
- `settings.rs` `update_json`: `self.nodus.settings_updated(&patch)` after applying (UPD-013).
- `app.rs` `take_open_request`: `self.nodus.take_open_request()` (notifications' pending card, then
  `set_open_request`'s).

## What the UI implements

- `nodus_core::AppShell`: `is_open`, `is_focused`, `is_loading`, `show` (PLAT-023's `showWindow`), `quit`
  (`app.quit()`: the normal quit path, which calls `Nodus::shutdown`), `other_version()` (HOST-102's box, may come up
  before the window exists), `exit_now` (default `process::exit(0)`).
- `NodusConfig { shell: Some(..), notifications: Notifications::System, dialogs: Some(..), ..NodusConfig::new(user_data,
  runtime) }`, then `Nodus::start`, register settings hooks, `nodus.launch().await` (PLAT-010 steps 8–9; `Err(QuitAtOnce)`
  means exit now), create the window, subscribe.
- Quit paths: `nodus.shutdown()` on every normal quit (before-quit); menu `Quit and Stop Sessions` →
  `nodus.menu().quit_and_stop_sessions()`; `Check for Updates…` → `nodus.menu().check_for_updates()`.
- `nodus.updates().install()` may give `RestartOutcome::Ask(question)`: show it, then `answer_stop_sessions(bool)`.
- The `nodus-view` protocol handler: `nodus.views().serve_page(url)`.
- Stop: `nodus_core::stoppable(&cards)`, `stop_confirmation(..)`, then `nodus.sessions().stop(&ids)` (SESS-153).
