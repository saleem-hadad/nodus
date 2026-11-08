# Nodus → Rust: migration plan

The single place to see what's done, what's in flight and what's next. **If you are an agent resuming work, read this file top to bottom first**, then the feature spec for your track in `migration/features/`.

## Goal

Rebuild Nodus v0.9.4 (the Electron app in `src/`) as a fully native Rust app with **exactly the same functionality, UI and UX**. No Electron, no Node, no web UI for the app itself. No new features during the migration.

The second goal is shape: the code is split into isolated domains with no UI dependencies, so a headless Nodus (hosted on a server, see `plans/hosted/intent.md`) can later reuse the same core. Building the headless server is **not** part of this migration.

Done means:

- Every checklist item in every `migration/features/*.md` is ticked and verified.
- An installed 0.9.4 updates itself to the Rust release through its own updater and keeps its boards, settings and saved xAI key. Sessions still running in its session host are handed over as decision U1 says.
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
8. **No secrets or personal data in the repo.** Nothing from the user's own `state.json`, boards, transcripts, Keychain, environment or clipboard: no keys, tokens or ciphertext, no real project names or repo paths. Fixtures are synthetic, and secrets appear only by name (`XAI_API_KEY`, `APPLE_APP_SPECIFIC_PASSWORD`). No absolute personal paths: write `~/…`, `<userData>/…` or `<scratch>/…`. Documenting *how* a secret is stored is fine. The app's pre-rename name never appears in docs; code that needs the old folder name keeps the literal where `src/main/store.ts` has it.

## JavaScript semantics to keep

The TypeScript runs on V8, so some of its behavior comes from JavaScript itself. Ports must reproduce these, not Rust's defaults:

- **String lengths are UTF-16 code units.** Every truncation, clip, window and highlight range (`slice`, `length`, `substring`) counts UTF-16 units, not bytes or chars. Use the helpers in `nodus-domain`.
- **Numbers are f64.** Times and coordinates can be fractional (file mtimes become `createdAt`), so they're `f64`, and they serialize like `JSON.stringify`: `1700000000000`, not `1700000000000.0`. **Write every JSON through `nodus_domain::json`**, never `serde_json::to_string` directly.
- **`delete` is `shift_remove`.** With `preserve_order`, serde_json's `Map::remove` swaps the last key into the hole; use `shift_remove` (on `IndexMap` too) so the order stays as JavaScript leaves it.
- **`Math.round` rounds .5 up**, toward +∞ (`Math.round(-2.5) === -2`); Rust's `f64::round` rounds away from zero.
- **Objects keep insertion order.** serde_json uses `preserve_order`; maps keyed by id are `IndexMap`. Sorts are stable.
- **Regexes:** JS `\s` includes U+FEFF and other Unicode spaces; JS `\w`, `\d` and `\b` without the `u` flag are ASCII-only. Check every ported regex.
- **Paths:** Node's `realpath` keeps the case it's given and never adds `\\?\` on Windows; Rust's `canonicalize` may do both. The transcript folder name depends on this.
- **Dates:** `Date.parse` is lenient; match what the code actually feeds it.

## Before every push

The orchestrator is the only one who pushes. Before each push to `main`:

```sh
sh migration/prepush.sh && git diff --stat origin/main..HEAD && git push origin main
```

`prepush.sh` runs gitleaks on the new commits and refuses added lines with personal paths or the app's pre-rename name (placeholders like `/Users/example` are fine). It exits non-zero on any finding, so the push never starts. Check the `--stat` lists only the files you meant to ship.

Any finding blocks the push until the commit is rewritten without it. A secret that reached GitHub is rotated, not just deleted.

## Decisions

| # | Decision | Status |
| --- | --- | --- |
| D1 | **UI: GPUI** (Zed's GPU-native UI framework) plus `gpui-component` where it fits. Native Metal / DirectX rendering, no webview for the app's own UI. | Pending spike (S1) |
| D2 | **Terminal: `alacritty_terminal`** (Apache-2.0) for VT parsing and the grid, wrapped GPUI-free in `nodus-terminal` and drawn by our own GPUI element in `nodus-ui`. Zed's `terminal_view` is GPL, so nothing is copied from it. | Pending spike (S1) |
| D3 | **Views' pages: the system webview through `wry`** (WKWebView on macOS, WebView2 on Windows), embedded in the board. Views are HTML that Claude writes, so they need a browser engine; everything else is native. | Pending spike (S1) |
| D4 | **PTYs: `portable-pty`**. Async runtime: `tokio`. HTTP (hook server, MCP relay): `axum`/`hyper`. | Proposed |
| D5 | **One binary.** `Nodus` runs the app; `Nodus host …` runs the session host (no Dock icon, outlives the app), as the Electron app runs its own binary as Node for `host.js`. | Proposed |
| D6 | **Secrets:** read the 0.9.4 `safeStorage` ciphertext so a saved xAI key survives the upgrade. macOS: Chromium OSCrypt, password in the Keychain item service `Nodus Safe Storage`, account `Nodus Key`; the Rust app is signed with the same Team ID and bundle id so the Keychain doesn't prompt. Windows: AES-256-GCM with a key kept in `Local State` and protected by DPAPI. Exact parameters per spec 01. | Proposed |
| D8 | **No visible difference for a 0.9.4 user (user, 2026-11-08).** Someone moving from 0.9.4 sees no difference in how the app looks or how it's used: same layout, colours, type, motion, shortcuts, gestures, copy and flows. Golden screenshots of 0.9.4 for every screen and state, light and dark, from synthetic seeded state, live in `migration/parity/`; every Rust UI slice renders the same scenes and is compared side by side (with a perceptual diff as an aid) before it merges. Same fonts (system UI font, SF Mono / Cascadia) and metrics. | Adopted |
| D9 | **Windows installer: electron-builder's NSIS templates (MIT), built with `makensis`.** Every installed copy since 0.9.0 runs `Nodus-Setup-<arch>.exe --updated /S --force-run`, and expects the same registry id, the same `%LOCALAPPDATA%\Programs\nodus` folder, the old uninstaller run with data and shortcuts kept, and the app started again. cargo-packager's installer does none of that. | Proposed (spec 06) |
| D10 | **Same Developer ID certificate (`CSC_LINK`), bundle id, macOS 13.0 minimum, arm64 and x64 on both OSes.** Copies since 0.8.1 reject an app signed by another team, and another identity loses the macOS permission grants and the Keychain item. Intel copies on Apple silicon and x64 copies on Windows on Arm download the arm64 asset. | Proposed (spec 06) |
| D11 | **A test driver built into the app, off unless `NODUS_AUTOMATION` names a socket.** It runs core commands, sends input, and takes screenshots of scenes. It replaces the DevTools-protocol smoke test (`scripts/smoke.mjs`) and renders the Rust side of the golden screenshots. | Proposed |
| D12 | **Handoff gates in the release workflow.** Before upload, CI replays 0.9.4's exact updater checks on each DMG, and installs a real 0.9.4 and updates it to the new build on Windows. `releases/latest` is the first place 0.9.4 sees a release, so there's no rehearsal after that. | Proposed (spec 06) |
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
  nodus-terminal/           the terminal model, GPUI-free: alacritty_terminal's grid, xterm-compatible key encoding,
                            query answers, selection, replay; nodus-ui draws it
  nodus-ui/                 GPUI: theme tokens, components, title bar, projects, board, overlays, capture panel
  nodus/                    the binary: app, and the `host` subcommand
```

### Layers

A crate depends only on crates in lower layers. When a crate needs something from a crate in its own layer or above (views starting a brain session, the board asking a session's status), it declares a small trait for it, and `nodus-core` implements that trait by calling the other crate. That keeps every domain testable on its own and lets a headless server wire them differently.

| Layer | Crates |
| --- | --- |
| L0 | `nodus-domain` (types, events, JSON as JavaScript writes it, UTF-16 helpers) |
| L1 | `nodus-store`, `nodus-platform`, `nodus-host-proto`, `nodus-terminal` |
| L2 | `nodus-transcripts`, `nodus-git`, `nodus-host` |
| L3 | `nodus-sessions`, `nodus-board` |
| L4 | `nodus-views`, `nodus-tools`, `nodus-actions`, `nodus-voice`, `nodus-capture`, `nodus-updater` |
| L5 | `nodus-core` |
| L6 | `nodus-ui` |
| L7 | `nodus` (the binary) |

### Conventions

- **One module per TypeScript file** where that reads naturally (`discovery.ts` → `discovery.rs`), with the TS doc comments carried over. Each spec's behavior IDs appear in the test names or a comment where the behavior is implemented (`// SESS-042`), so the parity audit can find them.
- **Async is tokio.** Services are plain structs holding their dependencies; no global state. Timers go through `tokio::time` so tests can pause the clock.
- **Events.** Domain crates emit `nodus_domain::Event` values (one variant per `api.on.*` event) into an `EventSink` they're given; `nodus-core` fans them out.
- **Errors.** Each crate has its own `thiserror` error type. The text a user can see is the 0.9.4 text, verbatim.
- **Processes.** Child processes are spawned through `nodus-platform` (environment, Windows quoting, killing the whole tree).
- **Tests.** Every Vitest case is ported, named after it. Behaviors a test can reach get a test even if 0.9.4 had none.
- **Dependencies.** Add shared ones to `[workspace.dependencies]`; keep the list small and well known.

The UI only talks to the core through `nodus-core`'s API: async commands that mirror `src/shared/api.ts` one to one, and a subscription to events that mirror `api.on.*`. Nothing in `nodus-ui` reads `state.json` or spawns a process.

## Feature specs

One file per domain in `migration/features/`. Each lists every behavior with an ID and a parity checklist. A track is done when its checklist is all ticked.

| File | Covers (Electron sources) | Spec | Ported |
| --- | --- | --- | --- |
| 01-platform-shell.md | app lifecycle, menu, env, store, settings, preload/IPC plumbing | ✅ 141 | ☐ |
| 02-session-host-status.md | host, host protocol, pty client, hooks status machine, agents, usage, notifications | ✅ 225 | ☐ |
| 03-sessions-transcripts.md | session create/open/send/kill, claude args, discovery, board merge, search, summarize, naming, geometry | ✅ 205 | ☐ |
| 04-board-tools-views.md | board tools MCP, notes placement, views (main + renderer), view page, ink storage | ✅ 222 | ☐ |
| 05-actions-voice-git.md | actions rail, voice prompts, git status/diff/log and their UI | ✅ 183 | ☐ |
| 06-updater-release.md | updater, packaging, signing, CI, release, smoke test, 0.9.4 → Rust handoff | ✅ 132 | ☐ |
| 07-capture.md | floating capture: panel window, double tap, screen context, guess, submit | ✅ 177 | ☐ |
| 08-projects-shell-ui.md | App, title bar, settings, projects grid, project rail, tokens and global styles, menus, modals | ✅ 305 | ☐ |
| 09-board-canvas.md | BoardView: pan/zoom, selection, drag, shortcuts, context menus, sections, archive, layout, edges | ✅ 177 | ☐ |
| 10-board-cards-notes.md | session cards, expanded cards, agent deck, archive stack, notes and the pad | ✅ 203 | ☐ |
| 11-board-overlays.md | prompt bar (sidekick), ⌘K search, merge dialog, ink layer, selection bar | ✅ 111 | ☐ |
| 12-terminal-ui.md | terminal registry, modal, transcript preview, agent viewer, xterm behavior | ✅ 185 | ☐ |

## Phases

Tracks after P1 run in parallel where the dependency column allows.

| Phase | What | Depends on | Status |
| --- | --- | --- | --- |
| P0 | Feature specs and this plan; UI stack spike (S1) | — | 🟡 specs done (2,266 behaviors); S1 running |
| P1 | Workspace, Rust CI job (macOS + Windows), `nodus-domain`, `nodus-store` (round-trips a real state.json), `nodus-platform` env | P0 | ✅ merged (93 tests) |
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

## Quirks to confirm with the user

Places where the code and the docs disagree, or where matching 0.9.4 exactly means copying something odd. The port follows the code (ground rule 1) unless the user decides otherwise before cutover.

| Spec | Quirk | Port does |
| --- | --- | --- |
| 10 NOTEUI-030 | how-it-works.md says deleting a note with writing asks first; the code deletes it at once. | Deletes at once |
| 10 | The agents tooltip says `1 agents, all done` for a single agent. | Same text |
| 05 | Recordings under 600 ms are thrown away; the docs say "about half a second". | 600 ms |
| 05 | Some errors shown to the user carry Electron's prefix `Error invoking remote method '<channel>': Error: …`. | Same text |
| 05 | Esc doesn't cancel a session mic recording while a terminal has focus. | Same |
| 12 | Clicking an OSC 8 link shows xterm's confirm, then most likely opens nothing; a right-click on a URL may open it. | Check in 0.9.4, then match |
| 04 | how-it-works.md says a view is a picture until clicked, double-click opens it large, and there's a View button in the title bar; none is in 0.9.4 (the click shield went in 64d9dc4, "New view…" is in the New Session button's menu). | Follows the code |
| 04 | Saving a note from the board drops `fromProject` and `fromTitle`, so a note another project's session wrote loses its signature once moved or edited. | Same (it's a bug; flag for after cutover) |
| 02 | The host decodes an MCP request body chunk by chunk, so a multibyte character split across chunks is corrupted. | Fixed: data corruption isn't a behavior anyone relies on |
| 02 | A malformed `spawn` message's error is unclear in the code. | Decide in P2 from the Node host's actual reply |
| 11 | Fuzzy matching can throw on titles with characters like `İ` (lowercasing changes the length), and a negative top score makes the archive stack show no results. | Same, unless the user rules them bugs |
| 09 | "New section here" beside a card puts the section about 9,920 units below the click. | Same, unless the user rules it a bug |
| 09 | The zoom pill's fit button jumps instead of gliding; on Windows Ctrl-click doesn't add to the selection; single-letter shortcuts don't work on non-Latin layouts. | Same |
| 01 PLAT-086/088 | The TS store drops unknown top-level and settings keys, and overwrites a `state.json` that parses but doesn't fit on the next save. | Deliberately safer, invisible to users: unknown keys are kept, and a misfit file is moved aside to `state.json.unreadable-<ms>` |
| 01 | A login shell that hangs ignores the SIGTERM Node sends at the 10 s timeout, so 0.9.4 never stops it. | Killed at 10 s |
| 01, 07 CAP-174/175 | The hidden capture panel stops the Dock from reopening a closed window on macOS, and keeps the app running after its window closes on Windows (the docs say closing the window quits). | Same, unless the user rules them bugs |

## Decisions for the user

Chromium gives 0.9.4 things a native app has no direct equivalent for. Each needs a call before P5 lands; the port's default is in the last column.

| Spec | What Chromium gives today | Proposed native behavior |
| --- | --- | --- |
| 01 | View menu: Reload, Force Reload, Toggle Developer Tools. | **Decided (user, 2026-11-08):** Force Reload and Toggle Developer Tools are dropped. Reload stays only in debug builds. |
| 01 | Page zoom (⌘+ / ⌘- / ⌘0), saved in Chromium's `Preferences`. | Same shortcuts scale the whole UI, read the saved zoom once on first launch and keep it in `state.json` after. |
| 01, 07, 11 | `localStorage` keys in Chromium's LevelDB: the capture draft, its attachments, sent-text history, the "clipboard already offered" marker, the ink colour. | Read them once from `Local Storage/leveldb` on first launch and move them into `state.json`. |
| 01 | Spellcheck underlines in text fields. | Native spellcheck where the OS offers it (NSSpellChecker on macOS). |
| 12 | Terminals on a zoomed board are a scaled bitmap, so their text goes soft. | Match 0.9.4: draw at 1× and scale the texture, same columns and rows. (Sharp text only if the user asks.) |

**U1 · Sessions running in a 0.9.4 host at the update (specs 02, 06).** A running Node host starts every new session through node-pty's `spawn-helper` inside the installed bundle, and may load other files from `app.asar`. Once the Rust app replaces the bundle, new sessions on that host fail with `posix_spawnp failed.` (On Windows, 0.9.4 already stops sessions before an update.) Options: (a) the first Rust release counts as a protocol change, so the existing, documented flow asks to stop those sessions and they resume from their cards; (b) ship a compatible `spawn-helper` at the old path so the Rust app adopts the Node host until it exits. **Default: (a)**, since it's behavior users already know and it ships no Node leftovers.

## Status log

Newest first. One line per event: date, what, branch or commit.

- 2026-11-08: P1 merged from `rust/p1-foundations`: workspace with 19 crates, nodus-domain (38 tests), nodus-store (28), nodus-platform env (27), Rust CI on macOS and Windows.
- 2026-11-08: All 12 feature specs on main (2,266 behaviors). Golden screenshot harness started (`rust/parity-harness`).

- 2026-11-08: Plan written. Rust 1.99.1 installed with rustup (no shell profile edits; `source ~/.cargo/env`). Spec agents and spike S1 launched.

## Resuming after an interruption

1. Read this file, then `git log --oneline -20` and `git branch -a | grep rust/`.
2. Find your track's row in **Phases** and its feature file; the unticked checklist items are what's left.
3. A `rust/<track>` branch with commits not on `main` is half-done work: read its last commit message and diff before continuing.
4. Update the status log and the tables here when you finish a slice, in the same commit.
