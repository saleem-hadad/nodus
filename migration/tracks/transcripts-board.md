# Track: transcripts-board (P3)

- **Branch:** `rust/transcripts-board`
- **Spec:** `migration/features/03-sessions-transcripts.md`
- **Scope:** `crates/nodus-transcripts` (paths, lines, parse, meta, preview, watch, search, summarize, naming),
  `crates/nodus-board` (merge, archive, cards, stats, board handlers, runs, geometry, stop rules),
  `crates/nodus-sessions::launch` (claude and shell command lines).
- **Shared files touched (additive):** root `Cargo.toml` (`notify`, `regex`), `nodus-domain::js` (`is_space`, `trim`,
  `trim_start`, `trim_end`, `collapse_spaces`).

## Status

- [x] nodus-transcripts
- [x] nodus-board
- [ ] nodus-sessions::launch

## Done (implemented and tested)

nodus-transcripts: SESS-001…070, SESS-109…113, SESS-161, SESS-163 (id check), SESS-165…195 (summaries' and search's
transcript side; the handlers that pick targets are in nodus-board).

nodus-board: SESS-052, SESS-071 (steps 2–6), SESS-072…108, SESS-114…118, SESS-120, SESS-140…145 (board side),
SESS-149…152, SESS-154 (`Board::on_done`: finish the run, then archive if asked), SESS-155…160, SESS-162, SESS-163,
SESS-165, SESS-166, SESS-173…175, SESS-196…202, SESS-204. geometry.test.ts and stopSessions.test.ts are ported
(`geometry.rs`, `stop.rs`).

## Deferred

(none yet)

## What nodus-core implements

- `nodus_board::BoardSessions` (pty + hooks + agents, 02): `is_live`, `status`, `usage`, `attach_agents`, `stop`
  (kill, not awaited), `stop_and_forget` (kill, then `forgetStatus` once gone, not awaited), `kill` (awaited, ≤ 4 s),
  `forget_status`. The board may call the queries while it holds the state, so they mustn't use the `Store`.
- `nodus_board::Launcher` (passed per call to `create_session` / `send_to_session`): `start_new` builds
  `nodus_sessions::launch::for_create` and spawns through the host; `resume_with_prompt` builds `launch::for_send`;
  `type_prompt` writes `launch::bracketed_paste(text)` then `\r` after `launch::SEND_RETURN_DELAY`. Wait for the
  login-shell env (`envReady`) inside them.
- Wiring: `board:load` = capture's `boardOpened` then `Board::load`; status change = emit `Status`, notifications,
  then `Board::on_done` when done, then views' `brainStatus`; `session:write` = `launch::is_takeover` →
  `Board::claim_run`, views' `brainInput`, host write; `Board::watcher().unwatch_project` on project removal and
  `unwatch_all` on quit; `board:save-ink` runs views-tools' `cleanInk` before `Board::save_ink`;
  `name_section` / `summarize` take the `Env` after `envReady`.

## Notes for the orchestrator

- **Directory order.** Node's `fs.readdirSync` goes through libuv's `scandir`, which sorts names with `strcmp` on macOS
  and Linux (Windows keeps the file system's order). The spec says "OS directory order (unsorted)" for SESS-005/007/050;
  the port sorts byte-wise on Unix as Node does (`paths::read_dir_names`). Worth fixing in the spec.
- **`Date.parse`.** Only the ECMAScript ISO format is read (what Claude Code writes). A date-time without an offset is
  read as UTC, where V8 reads it as local time, and V8's lenient fallback for other formats isn't ported
  (`date.rs` doc comment).
- **Error text.** A transcript that can't be opened fails with Node's wording (`EACCES: permission denied, open '…'`),
  and a `claude` that can't start with `spawn <path> ENOENT` (`nodeerr.rs`), for the common codes.
- **`claude -p` children** are started in one place, `claude::spawn`, until `nodus_platform::process` lands; switching
  is a one-line change there.
- The Store has no "change without saving": `create_session` adds its node with `Store::update`, so state.json may
  be written with the node before the process has started (0.9.4 saves only after). The content ends up the same.
- `summarize_sessions` and `name_section` take the `claude` path and `spawnEnv()` map, so the caller runs
  `Env::require_claude()` (SESS-174's order: after the no-conversation errors).
