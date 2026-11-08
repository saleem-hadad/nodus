# Track: status

- **Branch:** `rust/status` (phase P2b)
- **Spec:** `migration/features/02-session-host-status.md`, HOST-132 to HOST-219 (the status/agents/usage/notify half)
- **Crate:** `nodus-sessions`, modules `hooks`, `usage`, `agents`, `notify`, `running` (and the private `jsvalue`)
- **Sources:** `src/main/hooks.ts` (+ test), `src/main/agents.ts`, `src/main/notify.ts` (+ test), `src/renderer/src/components/runningSessions.ts` (+ test), the running counts in `src/main/board.ts` `projectStats`

## How it's checked

`crates/nodus-sessions/tests/fixtures/generate.mjs` runs the real hooks.ts, agents.ts and discovery.ts under Node 22 with only the Electron-bound imports stubbed and a fake clock, and writes the fixtures beside it: the three `--settings` variants byte for byte, `hookEnv`, status-line reports → usage, 20+ status-machine scenarios (every event and answer in order), and agents read from synthetic transcripts and meta files. `tests/hooks.rs` and `tests/agents.rs` replay them on a paused tokio runtime. Re-run the generator after changing the TypeScript:

```sh
node --experimental-strip-types crates/nodus-sessions/tests/fixtures/generate.mjs
```

## Done (implemented and tested)

- HOST-132 to HOST-159 status machine (`hooks::StatusMachine`): ported hooks.test.ts cases + fixture scenarios
- HOST-160 to HOST-165 keystroke rules, the 1200 ms interrupt timer on `tokio::time`, replayed history
- HOST-166 to HOST-173 usage (`usage::to_usage`, dedupe key, pause/resume)
- HOST-174 to HOST-182 endpoint, `hookEnv`, the three settings variants byte for byte, settings cache, silent hooks (real `sh` + `curl` against a local server; the PowerShell run is skipped without `pwsh`, as in 0.9.4's test)
- HOST-183 to HOST-200 agents (`agents::Agents`, `list_agent_files`, `agent_transcript_file`): replayed against real agents.ts (`fixtures/agents.json`: background tasks, task notifications, handback, end_turn, headings, incremental growth, shrink, same size with a new mtime, meta mid-rewrite, hold and settle)
- HOST-201 nothing watches agent files: by design, documented in agents.rs (the project watcher, track transcripts-board, covers it)
- HOST-202 to HOST-215 notifications and Dock badge (`notify::Notify`, `notice_for`, `waiting_count`): notify.test.ts ported, plus the service with a fake window and notification centre
- HOST-216 to HOST-219 running counts (`running::running_counts`, `working_and_waiting`, `running_total`, `running_by_project`): runningSessions.test.ts ported

## Deferred

- (none in scope) HOST-220 to HOST-225 are nodus-core's wiring (ipc.ts); the interfaces they need are below.

## Interfaces nodus-core must wire

- `hooks::StatusMachine::new(sink, clock, pending_work)`: `sink` fans `Event::Status` out in HOST-220's order (UI, `Notify::status_changed`, finish run / archive when asked, view brain); `clock` = `SystemClock`; `pending_work` = `Agents::has_pending_work`. It emits only after letting go of its state, so handlers may call back into it.
- The session host client (track host) feeds it through `hooks::SessionInput` (implemented by `StatusMachine`): `process_started(id, initial)` when it starts tracking a session (spawned, or each welcome session before its history), `process_exited(id)` when it stops (exit, host lost, stopAll), `hook(event, id, payload, at)` for each `hook` message (`at: None`) and each replayed history entry or status line (`at: Some(recorded)`), `user_input(id, data, at)` before each `write` and for replayed `input` entries. Liveness is tracked from those calls, so `StatusMachine::is_live` equals the client's `isLive`.
- `hooks::HookEndpoint`: the client calls `set(port, token, config)` on adoption (HOST-106); the claude command builder (track transcripts-board, `launch`) calls `settings(hooks::uses_powershell(&env))` and merges `env(session_id)` into the spawn environment.
- Other domains read `status()`, `usage()`, `forget()`, `is_live()`.
- `agents::Agents::new(home, clock)`: `session:agents` is `read_agents(&status, session_id, &list_agent_files(&subagents_dir))` and the board load calls `attach_agents(&status, &mut card, &files, last_prompt_at)`, with `subagents_dir` from nodus-transcripts (`<transcript dir>/<session id>/subagents`); `session:agent-transcript` checks ids with `agent_transcript_file` and reads it with nodus-transcripts' `readTranscript(…, agent)`.
- `notify::Notify::new(store, status, notifier, window, metas, sink)`: `notifier` adapts track platform-updater's `nodus_platform::notify` to `notify::Notifier` (`is_supported`, `show(id, &Notification)`, `close(id)`, `has_dock`, `set_badge`) and passes clicks to `Notify::clicked(id)`; `window` implements `notify::AppWindow` (open, focused, loading, show); `metas` implements `notify::SessionMetas` with nodus-transcripts' `readSession`; `sink` takes `Event::OpenCard`. Call `arm()` after the launch adoption, `update_badge()` on every settings change, `open_card()` from capture, `take_open_request()` for `app:take-open-request`.
- `running`: `projectStats` (nodus-board) takes `running` from `running_counts(board.nodes.values(), is_live)` and `working`/`waiting` from `working_and_waiting(…, is_live, status)`; the title bar uses `running_total` and `running_by_project`.

## Notes for integration

- agents.rs carries private copies of the discovery.ts helpers it uses (`truncate`, `assistantParts`, `describeToolUse`, `toolName`, `messageText`, `isTaskNotification`, `timeOf`/`Date.parse`, `readLines`); swap them for nodus-transcripts' once both tracks are merged. Its `Date.parse` reads what Claude Code writes (`toISOString()`) and V8's UTC-equivalent variants; a date-time without an offset (local time to V8) reads as no time.
- A meta file field that isn't a string (`agentType: 7`) is written as `String()` would write it; 0.9.4 passed the raw value to the renderer. Claude Code writes strings.
- `nodus_domain::json` writes a non-finite number as `Infinity`/`NaN`, where `JSON.stringify` writes `null`. Usage can only get there from absurd status-line numbers (e.g. a context size of 1e-300), but the fix belongs in nodus-domain.
