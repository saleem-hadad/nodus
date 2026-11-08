# Track: actions-voice-git (P3)

- **Branch:** `rust/actions-voice-git`
- **Spec:** `migration/features/05-actions-voice-git.md` (183 behaviors: ACT, VOICE, GIT)
- **Crates:** `nodus-git`, `nodus-actions`, `nodus-voice`
- **Shared files touched (additive):** root `Cargo.toml` `[workspace.dependencies]` (`icu_collator`, `objc2-avf-audio`,
  `opus1`; the `windows` comment).

## Status

| Module | State |
| --- | --- |
| `nodus-git` (runner, status, log, diff, diff parser, labels) | ✅ |
| `nodus-actions` (normalize, lists, draft, run args, rail and dialog logic) | ✅ |
| `nodus-voice` (key, xAI request, hint words) | ✅ |
| `nodus-voice` recorder (capture, Opus, WebM, meter) | ✅ (controller next) |

Each module is checked with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
(macOS and `--target x86_64-pc-windows-msvc`) and `cargo test --workspace`.

## Done (implemented and tested)

**nodus-git:** GIT-001, GIT-002 (stand-in git: argv, `GIT_TERMINAL_PROMPT=0`, non-zero exit as an answer, a signal and
32 MiB as failures, `spawn git ENOENT`), GIT-004…GIT-009, GIT-011, GIT-012, GIT-013 (`clamp_limit`), GIT-014…GIT-020,
GIT-048…GIT-055. git.test.ts (`tests/git.rs`, real git in temp repos with `GIT_CONFIG_GLOBAL=/dev/null`) and
diff.test.ts (`parse.rs`) are ported.

Pure parts for nodus-ui (`nodus_git::labels`), tested; the drawing is UI: GIT-025 (tooltip, `999+` badge), GIT-026
(label, `↑n`/`↓n`), GIT-029 (head and tracking line), GIT-034 (`<n> changed file(s)` / `No changes`), GIT-036 (labels,
tooltip, folder/name split), GIT-037 (icon names), GIT-038 (selection by path then index), GIT-039 (↑/↓/Home/End),
GIT-041 (cache key, the "same diff" test incl. the empty-patch-turned-too-large quirk), GIT-047 (gutter width, signs).

**nodus-actions:** ACT-001…ACT-012 (`normalize_action` over the JSON the window sends; actions.test.ts ported),
ACT-002, ACT-013…ACT-015 (`Actions` over the `Store`: scopes, moving between them, empty lists left out),
ACT-019…ACT-027 (`draft_with`/`draft_action`: argv, prompt, schema and system prompt byte-checked against
actions.ts, cwd, stdin closed, SIGKILL at 2 min on a paused clock, spawn failure text, JSON and fallback reasons;
against a stand-in `claude`), ACT-028 (`run_args`; runArgs test ported), ACT-083 (`remote_error`).

Logic for nodus-ui (`nodus_actions::rail`, `nodus_actions::dialog`), tested; the drawing is UI: ACT-031 and ACT-039
(toast texts), ACT-047, ACT-048, ACT-049…ACT-052 (`Rail`: reveal 100 ms, hide 300 ms / 0 ms, keep open, shown with
the menu), ACT-054/055 (hint text), ACT-056 (`press`), ACT-057, ACT-059, ACT-060 (labels), ACT-062, ACT-063,
ACT-064 (`fan_top`), ACT-065 (step and status label; the duration is 08's `formatDuration`), ACT-066, ACT-067…ACT-078
except drawing (`ActionDialog`: steps, chips, scope, request counter, stale drafts, save payload, footer buttons,
⌘/Ctrl+Enter), ACT-080.

**Done in other crates (already on main, tested there):** ACT-029, ACT-030 (nodus-board `new_session` /
`create_session`, nodus-sessions `launch::for_create`), ACT-034…ACT-038 (nodus-board `on_done`, the load sweep,
`finish_run` emitting `Event::RunFinished`), ACT-040/041 (nodus-sessions `launch::is_takeover`; the spec's quirk
table is also run from `nodus-actions/tests/actions.rs`), ACT-043 (`Board::claim_run`), ACT-044 (`send_to_session`).
ACT-033: `nodus_actions::is_run` is the predicate; the session tools are 04's (nodus-tools).

## Deferred

- GIT-003, ACT-016 (env part), ACT-042: nodus-core wiring (wait for the login-shell env, resolve `repoPath`, check
  `launch::is_takeover` → `Board::claim_run` before each `session:write`).
- GIT-021…GIT-024, GIT-027, GIT-028, GIT-030…GIT-033, GIT-035, GIT-040, GIT-042…GIT-046: UI (store refresh and
  polling, popover timers, modal layout). GIT-031 uses 10's `timeAgo` (CARD-041).
- ACT-017, ACT-018 (window store), ACT-032, ACT-045 (canvas, 09), ACT-046, ACT-053, ACT-058, ACT-061, ACT-079,
  ACT-081, ACT-082: UI.

## What nodus-core implements

- `git:status` = `Git::from_env(&env).status(repo)`; an `Err` reads as `null` in the window (GIT-021).
- `git:diff` = `Git::diff(repo, &file)`; errors go to the window as `remote_error("git:diff", …)`.
- `git:log` = `Git::log(repo, clamp_limit(limit))`.
- `actions:global` / `actions:save` / `actions:remove` = `Actions::new(store)` `.global()` / `.save(id, &raw)` /
  `.remove(id, action_id)`; `raw` is the JSON the window sends (`null` is fine).
- `actions:draft` = after the env, `draft_action(&env, repo, description, global == Some(true))`.
- A run's launch: `StartNew.action` → `launch::ActionRun { prompt: &action.prompt, args: run_args(&action) }`.
- `nodus-tools`' `isRun` = `store.read(|s| is_run(s, session_id))`.

## Notes for the orchestrator

- **Collation (GIT-009).** Changed files sort with ICU4X's root collation at tertiary strength, which is what V8's
  `localeCompare` uses for `en` (checked against Node: `a.txt`, `B.txt`, `c.txt`; `_x`, `1`, `a`; `src-a/` before
  `src/`; `file10` before `file2`). A user whose system locale tailors collation (Swedish, say) gets English order.
- **Node's `path`.** `nodus_git::nodepath` ports `resolve`, `relative`, `isAbsolute` and `parse().name` for both POSIX
  and win32, checked against Node; voice's hint words use `parse_name` too (PD-8).
- The empty-tree id is cached for the process, as 0.9.4 does (a SHA-256 repository after a SHA-1 one reuses it).
- **Draft output decoding.** 0.9.4 decodes each stdout chunk of `claude -p` on its own, so a character split across
  chunks turns into U+FFFD; the port decodes the whole output once (data corruption, not a behavior).
- **Key order.** `Project` and `State` are structs, so a project's `actions` and the top-level `actions` are written
  where the struct puts them, not where JavaScript appends a key it adds later (P1's choice, invisible to readers).
