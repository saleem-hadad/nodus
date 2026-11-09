# Track: core (P4)

- **Branch:** `rust/core`
- **Scope:** compose every merged domain into `nodus-core` (the UI-free service that mirrors `window.api` and the
  `ev:*` events), the deferred IDs that are core wiring, the integration debts, headless end-to-end tests; then (phase
  2) the app's wiring in `nodus-ui`: `AppShell`, launch, host, notifications, updater, Settings rows, live counts.
- **Sources:** `src/main/index.ts` (startup and quit order), `src/main/ipc.ts` (every handler and the glue),
  `src/shared/api.ts`.
- **Owned:** everything in `crates/nodus-core` except `projects.rs`, `groups.rs`, `settings.rs`, `app.rs` and the
  folder-picker `Dialogs` (ui-shell's; one-line hooks only, below), and `capture.rs` (the capture-panel track's).
  Phase 2 may edit `nodus-ui`'s `app.rs`, `window.rs`, the Settings view and the title bar; not `nodus-ui/src/board/`
  or the capture panel.

## Status

| Piece | State |
| --- | --- |
| Debts: claude -p through `nodus_platform::process`; agents on nodus-transcripts' helpers; one `SpawnOptions` | ✅ merged (49fc575) |
| Tests for HOST-033, 043, 064 (4 s exit), 086 (20 s relay), 090, 098 | ✅ merged (e3c4fef) |
| Composition: host client, status, notify, boards, views, board tools, updater; launch, shutdown | ✅ merged (7cf6661) |
| API areas: board, sessions, views, updates, menu; projects.stats with real counts; take_open_request | ✅ merged (7cf6661) |
| Unit tests for the fan-out order and the glue | ✅ merged (4266dca) |
| Headless end-to-end tests (`crates/nodus-core/tests/headless.rs`) | ✅ merged (3cd11cf) |
| Phase 2a: `git.*`, `actions.*`, `voice.*` in the core; views' `nodus.git()`; nodus-actions' `run_args` | ✅ |
| Phase 2b: app wiring in nodus-ui (AppShell, launch, host launcher, notifications, updater) | ☐ |
| Phase 2c: Settings voice-key and update rows, live counts and badges, Check for Updates…, Quit and Stop Sessions | ☐ |

## How to resume

Read this file, then `git log --oneline main..rust/core`. The next unchecked row is what's left.

## Done (implemented and tested)

- 02: HOST-033, 043, 064, 086, 090, 098 (host tests); HOST-220 (fan-out order), 221 (launch before the window),
  222 (open), 223 (write), 224 (resize, kill), 225 (quit paths).
- 03: SESS-130, 131, 134, 135, 138, 146, 147, 148, 153, 164, 203 (in `sessions.rs`, `wiring.rs`,
  `core_tests.rs` and `tests/headless.rs`).
- 06: UPD-013, UPD-014 (store default, checked through settings), UPD-076…079.
- 05 (phase 2a): GIT-003 (env wait and project lookup), VOICE-002, ACT-042 (takeover → `claim_run` before each
  `session:write`, since phase 1); `git:*`, `actions:*`, `voice:*` commands with 0.9.4's channels.
- 01 (core side): PLAT-032 (steps 2–5), PLAT-034, PLAT-046 (core side), PLAT-075, PLAT-077, PLAT-114, PLAT-128.

## Deferred

- `capture.*`: the capture-panel track adds `crates/nodus-core/src/capture.rs` and the few lines it needs in
  `nodus.rs` (`board:load`'s `boardOpened`, SESS-071 step 1; `stopCapture` in `shutdown`, PLAT-032 step 1;
  `syncShortcut` on settings).
- nodus-terminal's `Sessions` (open, write, resize) is the terminal-ui track's to implement over
  `nodus.sessions()`; it needs nothing more from the core.

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
  after letting go of the state. Status side effects run on a task of their own, so a status change reported while
  the board or the host client holds its state never waits on it.
- ~~The status machine and the host client rely on `process_started` / `process_exited` being called where pty.ts
  tracks and untracks a session~~: checked against pty.ts: `track` (spawned; each welcome session before its
  history) and `exited` (exit message, host lost, `stopAll` leftovers, only for a session that was running).
- ~~Six host behaviors have no test~~: see above.

## Small fixes in merged crates

- `nodus-sessions::client::HostClient::kill`: its future is `+ use<>`, so it doesn't borrow the client.
- `nodus-transcripts::paths::list_agent_files_in(dir)`, factored out of `list_agent_files`.
- `nodus-host::log_panics(&Logger)`, factored out of `main` so HOST-033 has a test.
- `nodus_platform::Env::with_vars(vars, home)`: an environment resolved as the login shell's is (a server's own, tests).
- `nodus_platform::permissions::ask_for_microphone` lets go of its block before waiting, so `voice:microphone`'s
  future is `Send`.

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
- `NodusConfig { shell, notifications: Notifications::System, dialogs, ..NodusConfig::new(user_data, runtime) }`,
  then `Nodus::start`, settings hooks, `nodus.launch().await` (PLAT-010 steps 8–9; `Err(QuitAtOnce)` means exit
  now), create the window, subscribe.
- Quit paths: `nodus.shutdown()` on every normal quit; `Quit and Stop Sessions` → `nodus.menu().quit_and_stop_sessions()`;
  `Check for Updates…` → `nodus.menu().check_for_updates()`.
- `nodus.updates().install()` may give `RestartOutcome::Ask(question)`: show it, then `answer_stop_sessions(bool)`.
- The `nodus-view` protocol handler: `nodus.views().serve_page(url)`.
- Stop: `nodus_core::stoppable(&cards)`, `stop_confirmation(..)`, then `nodus.sessions().stop(&ids)` (SESS-153).
- Mics: `nodus.voice().backend(current_project, open_settings)` is a `VoiceBackend` for `VoiceController::new`.
