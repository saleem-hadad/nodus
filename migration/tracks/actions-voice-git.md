# Track: actions-voice-git (P3)

- **Branch:** `rust/actions-voice-git`
- **Spec:** `migration/features/05-actions-voice-git.md` (183 behaviors: ACT, VOICE, GIT)
- **Crates:** `nodus-git`, `nodus-actions`, `nodus-voice`
- **Shared files touched (additive):** root `Cargo.toml` `[workspace.dependencies]` (`icu_collator`, `opus1`,
  and the same `block2`, `objc2*`, `reqwest`, `windows` lines platform-updater adds).

## Status

| Module | State |
| --- | --- |
| `nodus-git` (runner, status, log, diff, diff parser, labels) | ✅ |
| `nodus-actions` | ☐ |
| `nodus-voice` (key, xAI request, hint words) | ☐ |
| `nodus-voice` recorder (capture, Opus, WebM, meter, controller) | ☐ |

Each module is checked with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
(macOS and `--target x86_64-pc-windows-msvc`) and `cargo test --workspace`.

## Done (implemented and tested)

nodus-git: GIT-001, GIT-002 (stand-in git: argv, `GIT_TERMINAL_PROMPT=0`, non-zero exit as an answer, a signal and
32 MiB as failures, `spawn git ENOENT`), GIT-004…GIT-009, GIT-011, GIT-012, GIT-013 (`clamp_limit`), GIT-014…GIT-020,
GIT-048…GIT-055. git.test.ts (`tests/git.rs`, real git in temp repos with `GIT_CONFIG_GLOBAL=/dev/null`) and
diff.test.ts (`parse.rs`) are ported.

Pure parts for nodus-ui (`nodus_git::labels`), tested; the drawing is UI: GIT-025 (tooltip, `999+` badge), GIT-026
(label, `↑n`/`↓n`), GIT-029 (head and tracking line), GIT-034 (`<n> changed file(s)` / `No changes`), GIT-036 (labels,
tooltip, folder/name split), GIT-037 (icon names), GIT-038 (selection by path then index), GIT-039 (↑/↓/Home/End),
GIT-041 (cache key, the "same diff" test incl. the empty-patch-turned-too-large quirk), GIT-047 (gutter width, signs).

## Deferred

- GIT-003 Env wait: nodus-core awaits the login-shell env and resolves `repoPath` (`Unknown project <id>`), then calls
  `Git::from_env(&env)`.
- GIT-021…GIT-024, GIT-027, GIT-028, GIT-030…GIT-033, GIT-035, GIT-040, GIT-042…GIT-046: UI (store refresh and
  polling, popover timers, modal layout). GIT-031 uses 10's `timeAgo` (CARD-041).

## What nodus-core implements

- `git:status` = `Git::from_env(&env).status(repo)`; an `Err` reads as `null` in the window (GIT-021).
- `git:diff` = `Git::diff(repo, &file)`; errors go to the window as `Error invoking remote method 'git:diff': Error: <text>`.
- `git:log` = `Git::log(repo, clamp_limit(limit))`.

## Notes for the orchestrator

- **Collation (GIT-009).** Changed files sort with ICU4X's root collation at tertiary strength, which is what V8's
  `localeCompare` uses for `en` (checked against Node: `a.txt`, `B.txt`, `c.txt`; `_x`, `1`, `a`; `src-a/` before
  `src/`; `file10` before `file2`). A user whose system locale tailors collation (Swedish, say) gets English order.
- **Node's `path`.** `nodus_git::nodepath` ports `resolve`, `relative`, `isAbsolute` and `parse().name` for both POSIX
  and win32, checked against Node; voice's hint words use `parse_name` too (PD-8).
- The empty-tree id is cached for the process, as 0.9.4 does (a SHA-256 repository after a SHA-1 one reuses it).
