# Track: host (P2a)

- **Branch:** `rust/host`
- **Spec:** `migration/features/02-session-host-status.md`, HOST-001 to HOST-128 (host protocol, host process, the app's client). HOST-129–131 (the claude command) are `launch`'s (track transcripts-board); HOST-132 onwards are track status's.
- **Crates:** `nodus-host-proto`, `nodus-host`, `nodus-sessions::client`, the `host` subcommand in `nodus`.

## Progress

| Module | State |
| --- | --- |
| `nodus-host-proto`: messages, reader, paths, Windows command line, replay buffer | done |
| `nodus-host::pty`: macOS (fork + exec, as node-pty's spawn helper) and Windows (ConPTY) | done; Windows compiled and clippy-clean, its tests run on the Windows CI job only |
| `nodus-host`: socket/pipe, sessions, hook endpoint, MCP relay, lifecycle, `host.log` | done |
| `nodus`: `Nodus host <socket> <key>` | done |
| `nodus-sessions::client` | done |
| Wire tests against the 0.9.4 Node host, both directions | done |

## How wire compatibility is checked

- **Rust host = Node host.** `crates/nodus-host/tests/wire.rs` runs one scripted conversation (hello/welcome, spawns incl. a duplicate and one node-pty refuses, output, status keys, ignored messages, every hook case, the status line, a second app's welcome with history, every `/mcp` case incl. a batch and a client leaving mid-relay, a protocol-0 probe, kill, shutdown from the probe) and writes down everything the host says on the socket, over HTTP and in its log, with port/token/times replaced. `fixtures/wire-transcript.txt` was recorded from the real Node host; the Rust host reproduces it line for line. The Node half re-runs when `NODUS_NODE_HOST_JS` and `NODUS_NODE_BIN` point at a build (instructions in the file); `NODUS_UPDATE_GOLDEN=1` re-records.
- **Rust client → Node host.** `crates/nodus-sessions/tests/client.rs` `node_host_…` starts the Node host through `ProcessLauncher` exactly as 0.9.4 starts it (detached, `host.log`, `ELECTRON_RUN_AS_NODE=1`), spawns, writes, resizes, has a relaunched Rust client adopt the session with its history, kills, stops everything. Same env vars.
- **TypeScript client → Rust host.** Checked once by hand (not committed: it has to sit in `src/main` to import `pty.ts`): a vitest file with `electron`, `./env` and `./boardTools` stubbed drove the `target/debug/nodus host` binary with the real `pty.ts` and `hooks.ts`: connect, terminal and claude-shaped spawns, writes, resize, the curl config and hook env, a hook moving status, the MCP relay, a relaunch adopting output and replaying a prompt hook and an Esc (status `done` 1200 ms later), `stopAll`, idle exit. All passed.

Build for the Node runs: `corepack pnpm install --frozen-lockfile && npx electron-vite build --outDir node_modules/.cache/host` (Node 22). Every test kills the hosts it starts.

## Done (implemented and tested)

- HOST-001–021: protocol, paths, private files, trust, framing (hostProtocol.test.ts ported in full, plus golden bytes for every message).
- HOST-022–032, 034, 035: arguments, usage error, one host per socket, stale socket, hook server first, listen, socket mode, Windows key (Windows CI), hook token, ready log, 30 s unused exit, log format, no UI in the host path.
- HOST-036–047: hello first, welcome, app clients, other protocols, ignored messages, `spawned` to the asker, broadcasts, disconnect, idle checks, newest app, idle rule.
- HOST-048–063, 065–067: duplicate spawn, PTY spawn and env, argv, Windows command line, spawn errors, late spawn failure, logs, output decoding, buffer limit in UTF-16 units, exit ordering, exit code 0 on signal, write, status keys, resize, kill (SIGTERM to the pid, SIGKILL after 3 s; Windows console kill on CI), stopping host exits, history cap, slim payloads.
- HOST-064: shutdown (golden); see Deferred for the 4 s hard exit.
- HOST-068–085, 087–089: the hook endpoint (every case, golden) and the MCP relay (every case, golden; 413 tested on its own).
- HOST-086: settles on a reply and on the client leaving (golden); see Deferred for the 20 s timeout.
- HOST-091–097, 099–128: the client: starting a host (detached, through `nodus_platform::process`), polling 10 s, one attempt at a time, connect order, launch adopts, hello, welcome timeout and early close, other protocol, adoption in order (incl. a message in the same read), replace dialog both ways, hook config, endpoint, board tools flag, history and status line replay, host lost, no reconnect, quit, stopAll (both outcomes), spawn once / shared starts, spawn request bytes, spawned, unknown output, app buffer, 16 ms coalescing, snapshot, exit order, write, sendPrompt, resize, kill (and its 4 s), killProject, board tools replies, sends while disconnected, live list.

## Implemented, not separately tested

- HOST-033: a panic is logged as `uncaught Error: <message>` by the binary's panic hook, and tokio keeps a panicking task to itself. Rust has no unhandled rejections, so there's no `unhandled` line.
- HOST-043: socket errors end the connection quietly (no test makes a socket error).
- HOST-064's hard exit 4000 ms after `shutdown`: only visible with a process that outlives SIGKILL.
- HOST-086's 20 s relay timeout (each relay's own deadline from when it was sent).
- HOST-090: a failing MCP request (body read error) logs `mcp request failed` and answers 500.
- HOST-098: the untrusted-pipe error is reachable only on Windows; `trusts_host` is unit-tested on every platform, and the client check is one call to it.

## Deferred

- Windows behaviours (HOST-005, 029, 051, 052, 063, ConPTY output/exit timing) are compiled and clippy-clean for `x86_64-pc-windows-msvc`, with tests in `pty_windows.rs`, `windows_host.rs` and `nodus/tests/host_subcommand.rs`, but have not run yet: there's no Windows machine here. First run is the Windows CI job.
- Decision U1 (adopting a 0.9.4 Node host across the bundle swap): default (a) since the cutover, `PROTOCOL` is 2; a 0.9.4 host or app is another version (`migration/tracks/cutover.md`).

## Where 0.9.4 and the spec differ (the port follows 0.9.4)

- Spec §13 / HOST-051: with no arguments node-pty 1.1.0's `argsToCommandLine(file, '')` returns `file` exactly as given, not quoted.
- Spec "Edge cases": a malformed `spawn` gets node-pty's own message (`args as a string is not supported on unix.`, recorded from the Node host; wrong types give node-pty's `Usage: pty.fork(…)`).
- Spec "Edge cases" (Windows): in node-pty 1.1.0 `CreateProcessW` runs inside `pty.spawn`, so its failure is a `spawned` error (`Cannot create process, error code: N`), not an `uncaught` log.
- Windows kill: 0.9.4 can report an exit without `exitCode` (the console closes before the process is reaped); the Rust host waits for the real code (1, from TerminateProcess). The window ignores the code; the client maps a missing one to 0.
- An MCP body over 1 MiB: Node destroys the socket mid-reply; the Rust host answers 413 and closes. Bodies are decoded across chunks (plan: fixed).
- Hooks are processed before the 204 is written; in Node both happen in the same tick, so nothing can tell.

## Client interface (for nodus-core)

`nodus_sessions::client::HostClient::new(user_data, Wiring { events, status, endpoint, board_tools, prompt, launcher })`:

- `events: Arc<dyn EventSink>` gets `Event::PtyData` (coalesced, 16 ms) and `Event::PtyExit` (`-1` when the host was lost).
- `status: Arc<dyn hooks::SessionInput>` (the `StatusMachine`): `process_started` when a session is tracked (spawned, or each welcome session before its history), `process_exited` when it stops, `hook` for each `hook` message (`at: None`) and replayed hook/status line (`at: Some`), `user_input` before every `write` and for replayed keystrokes. Its `is_live` therefore equals the client's.
- `endpoint: Arc<hooks::HookEndpoint>`: `set(port, token, config)` on each adoption.
- `board_tools: Arc<dyn BoardTools>`: `answer(session_id, message) -> Result<Option<Value>, String>`; errors are logged as `[board-tools]`.
- `prompt: Arc<dyn HostPrompt>`: `other_version() -> ReplaceChoice` shows the "Sessions from another version of Nodus are still running" box. `ClientError::Quit` from `connect_host`/`spawn_session` means the user chose Quit: exit at once with code 0, no quit handlers.
- `launcher: Arc<dyn HostLauncher>`: `ProcessLauncher::current_exe()` (`Nodus host <socket> <key>`) in the app.

Methods (pty.ts): `connect_host(start)` (launch: `connect_host(false)`, log `[host] connect failed <err>` on error), `disconnect()` (quit), `stop_all()` (Quit and Stop Sessions, Windows update), `spawn_session(&launch::SpawnOptions, |opts, HostInfo { relays_mcp }| launch::command_for(…))` (the command is built after connecting; board tools when `relays_mcp`), `snapshot`, `write`, `send_prompt`, `resize`, `kill` (sends at call time; the future resolves on exit or 4 s), `kill_project`, `is_live`, `live_session_ids`, `relays_mcp`, `paths`.

Errors shown to the user are 0.9.4's strings, in `ClientError::Failed`.
