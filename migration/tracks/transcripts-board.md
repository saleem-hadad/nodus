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
- [ ] nodus-board
- [ ] nodus-sessions::launch

## Done (implemented and tested)

nodus-transcripts: SESS-001…070, SESS-109…113, SESS-161, SESS-163 (id check), SESS-165…195 (summaries' and search's
transcript side; the handlers that pick targets are in nodus-board).

## Deferred

(none yet)

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
- `summarize_sessions` and `name_section` take the `claude` path and `spawnEnv()` map, so the caller runs
  `Env::require_claude()` (SESS-174's order: after the no-conversation errors).
