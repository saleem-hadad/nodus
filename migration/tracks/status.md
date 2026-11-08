# Track: status

- **Branch:** `rust/status` (phase P2b)
- **Spec:** `migration/features/02-session-host-status.md`, HOST-132 to HOST-219 (the status/agents/usage/notify half)
- **Crate:** `nodus-sessions`, modules `hooks`, `usage`, `agents`, `notify`, `running` (and the private `jsvalue`)
- **Sources:** `src/main/hooks.ts` (+ test), `src/main/agents.ts`, `src/main/notify.ts` (+ test), `src/renderer/src/components/runningSessions.ts` (+ test), the running counts in `src/main/board.ts` `projectStats`

## How it's checked

`crates/nodus-sessions/tests/fixtures/generate.mjs` runs the real hooks.ts (and agents.ts, discovery.ts) under Node 22 with only the Electron-bound imports stubbed and a fake clock, and writes the fixtures beside it: the three `--settings` variants byte for byte, `hookEnv`, status-line reports → usage, and 20+ status-machine scenarios (every event and answer in order). `tests/hooks.rs` replays them on a paused tokio runtime. Re-run the generator after changing the TypeScript:

```sh
node --experimental-strip-types crates/nodus-sessions/tests/fixtures/generate.mjs
```

## Done (implemented and tested)

- HOST-132 to HOST-159 status machine (`hooks::StatusMachine`): ported hooks.test.ts cases + fixture scenarios
- HOST-160 to HOST-165 keystroke rules, the 1200 ms interrupt timer on `tokio::time`, replayed history
- HOST-166 to HOST-173 usage (`usage::to_usage`, dedupe key, pause/resume)
- HOST-174 to HOST-182 endpoint, `hookEnv`, the three settings variants byte for byte, settings cache, silent hooks (real `sh` + `curl` against a local server)

## Deferred

- (none yet)

## In progress

- agents (HOST-183 to HOST-201), notify (HOST-202 to HOST-215), running counts (HOST-216 to HOST-219)

## Interfaces nodus-core must wire

- `hooks::StatusMachine::new(sink, clock, pending_work)`: `sink` fans `Event::Status` out in HOST-220's order (UI, notifier, finish run / archive when asked, view brain); `clock` = `SystemClock`; `pending_work` = the agents service's `has_pending_work`.
- The session host client (track host) feeds it: `process_started(id, initial)` when it starts tracking a session (spawned, or each welcome session), `process_exited(id)` when it stops (exit, host lost, stopAll), `receive_hook(event, id, payload, at)` for each `hook` message (`at: None`) and each replayed history entry or status line (`at: Some(recorded)`), `note_user_input(id, data, at)` before each `write` and for replayed `input` entries. Liveness is tracked from those calls, so `StatusMachine::is_live` equals the client's `isLive`.
- `hooks::HookEndpoint`: the client calls `set(port, token, config)` on adoption (HOST-106); the claude command builder (track transcripts-board, `launch`) calls `settings(hooks::uses_powershell(&env))` and merges `env(session_id)` into the spawn environment.
- Other domains read `status()`, `usage()`, `forget()`.
