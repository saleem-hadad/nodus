# Nodus → Rust: migration plan

The single place to see what's done, what's in flight and what's next. **If you are an agent resuming work, read this file top to bottom first**, then the feature spec for your track in `migration/features/`.

## Goal

Rebuild Nodus v0.9.4 (the Electron app in `src/`) as a fully native Rust app with **exactly the same functionality, UI and UX**. No Electron, no Node, no web UI for the app itself. No new features during the migration.

The second goal is shape: the code is split into isolated domains with no UI dependencies, so a headless Nodus (hosted on a server, see `plans/hosted/intent.md`) can later reuse the same core. Building the headless server is **not** part of this migration.

Done means:

- Every checklist item in every `migration/features/*.md` is ticked and verified.
- An installed 0.9.4 updates itself to the Rust release through its own updater, keeps its boards, settings and saved xAI key, and adopts the sessions still running in its session host.
- Side-by-side screenshots of every screen, in light and dark, match the Electron app.
- `src/`, `package.json` and the rest of the Electron toolchain are gone from `main`.

## Ground rules

1. **The Electron app is the spec.** When a feature file and the TypeScript disagree, the TypeScript wins, and the feature file gets fixed.
2. **No new features, no "improvements".** Same strings, same timings, same sizes, same shortcuts, same file formats. Bugs that users rely on are features.
3. **Data compatibility is not optional.** Same userData folder (`~/Library/Application Support/Nodus`, `%APPDATA%\Nodus`), same `state.json` schema (read and write), same `views/` and `captures/` layout, same `NODUS_USER_DATA` override, same bundle id `dev.saleem.nodus`, same release asset names.
4. **Wire compatibility with the 0.9.4 session host.** The Rust host speaks the same protocol (same `PROTOCOL` number, same messages), so a Rust app adopts a running Node host and vice versa.
5. **Domains don't depend on the UI.** Only `nodus-ui` and the `nodus` binary know GPUI exists. Domain crates depend on `nodus-domain` and, where they must touch the OS, `nodus-platform`.
6. **Push early.** Every slice that builds, passes tests and doesn't break the Electron app goes to `main`. Until cutover, `main` ships the Electron app; the Rust workspace grows beside it.
7. **Port the tests.** Every Vitest case in `src/**/*.test.ts` gets a Rust counterpart in the crate that owns the behavior.

## Decisions

| # | Decision | Status |
| --- | --- | --- |
| D1 | **UI: GPUI** (Zed's GPU-native UI framework) plus `gpui-component` where it fits. Native Metal / DirectX rendering, no webview for the app's own UI. | Pending spike (S1) |
| D2 | **Terminal: `alacritty_terminal`** (Apache-2.0) for VT parsing and the grid, drawn by our own GPUI element in `nodus-terminal`. Zed's `terminal_view` is GPL, so nothing is copied from it. | Pending spike (S1) |
| D3 | **Views' pages: the system webview through `wry`** (WKWebView on macOS, WebView2 on Windows), embedded in the board. Views are HTML that Claude writes, so they need a browser engine; everything else is native. | Pending spike (S1) |
| D4 | **PTYs: `portable-pty`**. Async runtime: `tokio`. HTTP (hook server, MCP relay): `axum`/`hyper`. | Proposed |
| D5 | **One binary.** `Nodus` runs the app; `Nodus host …` runs the session host (no Dock icon, outlives the app), as the Electron app runs its own binary as Node for `host.js`. | Proposed |
| D6 | **Secrets:** read the 0.9.4 `safeStorage` ciphertext (Chromium OSCrypt: Keychain "Nodus Safe Storage" on macOS, DPAPI on Windows) so a saved xAI key survives the upgrade. | Proposed |
| D7 | **Branches:** each track works on a `rust/<track>` branch in its own worktree, and merges to `main` when `cargo test` and `pnpm typecheck/test/build` both pass. | Adopted |

## Architecture

A Cargo workspace at the repo root, beside the Electron sources until cutover.

```
Cargo.toml                  workspace
crates/
  nodus-domain/             the types in src/shared/types.ts, serde-compatible with state.json and the IPC JSON; pure
  nodus-store/              userData resolution, state.json load/save/migrate, views/ and captures/ folders
  nodus-platform/           OS seams: login-shell env, claude lookup, secrets, open/reveal, notifications, Dock badge,
                            permissions (Accessibility, Screen Recording, mic). cfg(macos) / cfg(windows) inside, one API outside
  nodus-host-proto/         the session host wire protocol (src/main/hostProtocol.ts), byte-compatible with 0.9.4
  nodus-host/               the session host process: PTYs, the hook HTTP server, the MCP relay, the socket / named pipe
  nodus-sessions/           the app's side of the host (pty.ts), the status machine (hooks.ts), agents, usage,
                            how claude and shells are started
  nodus-transcripts/        discovery and transcript parsing, search, naming, handoff summaries
  nodus-board/              board merge with sessions on disk, archive, sections, notes placement, geometry, ink
  nodus-views/              views: files, commands, schedules, brains, the page and window.nodus
  nodus-tools/              the board tools' MCP server
  nodus-actions/            the actions rail: drafting and runs
  nodus-voice/              audio capture and xAI speech to text
  nodus-capture/            floating capture: double tap, screen context, Claude's guess, making the capture
  nodus-git/                git status, diff, log
  nodus-updater/            GitHub release updates and the swap on quit
  nodus-core/               composes the domains into one service: commands (= window.api) and an event stream
                            (= the ev:* channels). No UI dependency. This is the seam a headless server will reuse.
  nodus-terminal/           GPUI terminal element over alacritty_terminal
  nodus-ui/                 GPUI: theme tokens, components, title bar, projects, board, overlays, capture panel
  nodus/                    the binary: app, and the `host` subcommand
```

The UI only talks to the core through `nodus-core`'s API: async commands that mirror `src/shared/api.ts` one to one, and a subscription to events that mirror `api.on.*`. Nothing in `nodus-ui` reads `state.json` or spawns a process.

## Feature specs

One file per domain in `migration/features/`. Each lists every behavior with an ID and a parity checklist. A track is done when its checklist is all ticked.

| File | Covers (Electron sources) | Spec | Ported |
| --- | --- | --- | --- |
| 01-platform-shell.md | app lifecycle, menu, env, store, settings, preload/IPC plumbing | ⏳ | ☐ |
| 02-session-host-status.md | host, host protocol, pty client, hooks status machine, agents, usage, notifications | ⏳ | ☐ |
| 03-sessions-transcripts.md | session create/open/send/kill, claude args, discovery, board merge, search, summarize, naming, geometry | ⏳ | ☐ |
| 04-board-tools-views.md | board tools MCP, notes placement, views (main + renderer), view page, ink storage | ⏳ | ☐ |
| 05-actions-voice-git.md | actions rail, voice prompts, git status/diff/log and their UI | ⏳ | ☐ |
| 06-updater-release.md | updater, packaging, signing, CI, release, smoke test, 0.9.4 → Rust handoff | ⏳ | ☐ |
| 07-capture.md | floating capture: panel window, double tap, screen context, guess, submit | ⏳ | ☐ |
| 08-projects-shell-ui.md | App, title bar, settings, projects grid, project rail, tokens and global styles, menus, modals | ⏳ | ☐ |
| 09-board-canvas.md | BoardView: pan/zoom, selection, drag, shortcuts, context menus, sections, archive, layout, edges | ⏳ | ☐ |
| 10-board-cards-notes.md | session cards, expanded cards, agent deck, archive stack, notes and the pad | ⏳ | ☐ |
| 11-board-overlays.md | prompt bar (sidekick), ⌘K search, merge dialog, ink layer, selection bar | ⏳ | ☐ |
| 12-terminal-ui.md | terminal registry, modal, transcript preview, agent viewer, xterm behavior | ⏳ | ☐ |

## Phases

Tracks after P1 run in parallel where the dependency column allows.

| Phase | What | Depends on | Status |
| --- | --- | --- | --- |
| P0 | Feature specs and this plan; UI stack spike (S1) | — | 🟡 in progress |
| P1 | Workspace, Rust CI job (macOS + Windows), `nodus-domain`, `nodus-store` (round-trips a real state.json), `nodus-platform` env | P0 | ☐ |
| P2 | Session host: `nodus-host-proto`, `nodus-host`, `nodus-sessions` client; wire tests against the Node host both ways | P1 | ☐ |
| P3 | Data domains: transcripts, board, git, search, summarize, naming, status machine, agents | P1 | ☐ |
| P4 | `nodus-core`: every command and event in api.ts, tested headless | P2, P3 | ☐ |
| P5 | UI foundation: app shell, theme tokens (light/dark), title bar, menus, settings, projects grid, project rail | P1, S1 | ☐ |
| P6 | Board canvas: pan/zoom/select/drag, cards, sections, archive, notes, edges, controls, context menus, shortcuts | P4, P5 | ☐ |
| P7 | Terminal: element, modal, expanded cards, transcript preview, agent viewer | P2, P5 | ☐ |
| P8 | Overlays: prompt bar, ⌘K search, merge dialog, ink, selection bar, agent deck | P6 | ☐ |
| P9 | Views (wry), board tools, actions, voice, git UI | P6 | ☐ |
| P10 | Capture panel, notifications, Dock badge, updater | P6 | ☐ |
| P11 | Packaging: .app and DMGs (same bundle id, Info.plist keys, entitlements, signing, notarizing), Windows NSIS (same flags), release workflow; 0.9.4 → Rust update test | P10 | ☐ |
| P12 | Parity audit (every checklist item, screenshots light and dark), then cutover: delete Electron, update README and docs, release | P11 | ☐ |

## Spikes

| # | Question | Status | Outcome |
| --- | --- | --- | --- |
| S1 | Does GPUI (+ gpui-component) build and run here, with a pannable canvas, an alacritty terminal on a real PTY, a wry webview inside a GPUI window, and a transparent floating panel over full-screen apps? | 🟡 running | — |

## Status log

Newest first. One line per event: date, what, branch or commit.

- 2026-11-08: Plan written. Rust 1.99.1 installed with rustup (no shell profile edits; `source ~/.cargo/env`). Spec agents and spike S1 launched.

## Resuming after an interruption

1. Read this file, then `git log --oneline -20` and `git branch -a | grep rust/`.
2. Find your track's row in **Phases** and its feature file; the unticked checklist items are what's left.
3. A `rust/<track>` branch with commits not on `main` is half-done work: read its last commit message and diff before continuing.
4. Update the status log and the tables here when you finish a slice, in the same commit.
