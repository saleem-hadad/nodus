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
| D1 | **UI: GPUI through `gpui-kit =0.9.0`** (`gpui-pre =0.3.11`, a weekly snapshot of Zed's GPUI, plus `gpui-component 0.9.0`), with `runtime_shaders`. Native Metal / DirectX rendering, no webview for the app's own UI. Steps 1–6 of S1 pass on stock crates. | Adopted (S1) |
| D2 | **Terminal: `alacritty_terminal =0.26.0`** (Apache-2.0) for VT parsing and the grid, wrapped GPUI-free in `nodus-terminal` and drawn by our own GPUI element in `nodus-ui`. Zed's `terminal_view` is GPL, so nothing is copied from it. | Adopted (S1) |
| D3 | **Views' pages: the system webview through upstream `wry =0.57.1`** (WKWebView on macOS, WebView2 on Windows), embedded in the board. A native view always draws above GPUI, so whenever anything overlaps a view, it's dragged, or an ink tool is out, the board shows a snapshot of it instead (2–16 ms). Views are HTML that Claude writes; everything else is native. | Adopted (S1) |
| D4 | **PTYs: `portable-pty`**. Async runtime: `tokio`. HTTP (hook server, MCP relay): `axum`/`hyper`. | Proposed |
| D5 | **One binary.** `Nodus` runs the app; `Nodus host …` runs the session host (no Dock icon, outlives the app), as the Electron app runs its own binary as Node for `host.js`. | Proposed |
| D6 | **Secrets:** read the 0.9.4 `safeStorage` ciphertext so a saved xAI key survives the upgrade. macOS: Chromium OSCrypt, password in the Keychain item service `Nodus Safe Storage`, account `Nodus Key`; the Rust app is signed with the same Team ID and bundle id so the Keychain doesn't prompt. Windows: AES-256-GCM with a key kept in `Local State` and protected by DPAPI. Exact parameters per spec 01. | Proposed |
| D8 | **No visible difference for a 0.9.4 user (user, 2026-11-08).** Someone moving from 0.9.4 sees no difference in how the app looks or how it's used: same layout, colours, type, motion, shortcuts, gestures, copy and flows. Golden screenshots of 0.9.4 for every screen and state, light and dark, from synthetic seeded state, live in `migration/parity/`; every Rust UI slice renders the same scenes and is compared side by side (with a perceptual diff as an aid) before it merges. Same fonts (system UI font, SF Mono / Cascadia) and metrics. | Adopted |
| D9 | **Windows installer: electron-builder's NSIS templates (MIT), built with `makensis`.** Every installed copy since 0.9.0 runs `Nodus-Setup-<arch>.exe --updated /S --force-run`, and expects the same registry id, the same `%LOCALAPPDATA%\Programs\nodus` folder, the old uninstaller run with data and shortcuts kept, and the app started again. cargo-packager's installer does none of that. | Proposed (spec 06) |
| D10 | **Same Developer ID certificate (`CSC_LINK`), bundle id, macOS 13.0 minimum, arm64 and x64 on both OSes.** Copies since 0.8.1 reject an app signed by another team, and another identity loses the macOS permission grants and the Keychain item. Intel copies on Apple silicon and x64 copies on Windows on Arm download the arm64 asset. | Proposed (spec 06) |
| D11 | **A test driver built into the app, off unless `NODUS_AUTOMATION` names a socket.** It runs core commands, sends input, and takes screenshots of scenes. It replaces the DevTools-protocol smoke test (`scripts/smoke.mjs`) and renders the Rust side of the golden screenshots. | Proposed |
| D12 | **Handoff gates in the release workflow.** Before upload, CI replays 0.9.4's exact updater checks on each DMG, and installs a real 0.9.4 and updates it to the new build on Windows. `releases/latest` is the first place 0.9.4 sees a release, so there's no rehearsal after that. | Proposed (spec 06) |
| D13 | **We own a patched GPUI.** Stock GPUI can't blur behind an element (the prompt bar, menus, toasts, modals and archive panel all do) or rotate boxes and text (dragged notes, stickers, the pad, the agent fan), and fades elements primitive by primitive rather than as a group. The patch set: the effect-layers PR stack (zed#63773–#63780, backdrop blur on Metal, wgpu and DirectX 11) with a real fix for the two shaders Apple's Metal compiler crashes on, transforms on offscreen layers, and group opacity. The patched `gpui-pre*` crates are vendored under `third_party/gpui/`, keeping their package names and version, and swapped in with `[patch.crates-io]`. | Adopted (S1) |
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
| 01-platform-shell.md | app lifecycle, menu, env, store, settings, preload/IPC plumbing | ✅ 141 | 83/141 |
| 02-session-host-status.md | host, host protocol, pty client, hooks status machine, agents, usage, notifications | ✅ 225 | 222/225 |
| 03-sessions-transcripts.md | session create/open/send/kill, claude args, discovery, board merge, search, summarize, naming, geometry | ✅ 205 | 205/205 |
| 04-board-tools-views.md | board tools MCP, notes placement, views (main + renderer), view page, ink storage | ✅ 222 | 168/222 |
| 05-actions-voice-git.md | actions rail, voice prompts, git status/diff/log and their UI | ✅ 183 | 102/183 |
| 06-updater-release.md | updater, packaging, signing, CI, release, smoke test, 0.9.4 → Rust handoff | ✅ 132 | 99/132 |
| 07-capture.md | floating capture: panel window, double tap, screen context, guess, submit | ✅ 177 | 176/177 |
| 08-projects-shell-ui.md | App, title bar, settings, projects grid, project rail, tokens and global styles, menus, modals | ✅ 305 | 270/305 |
| 09-board-canvas.md | BoardView: pan/zoom, selection, drag, shortcuts, context menus, sections, archive, layout, edges | ✅ 177 | 149/177 |
| 10-board-cards-notes.md | session cards, expanded cards, agent deck, archive stack, notes and the pad | ✅ 203 | ☐ |
| 11-board-overlays.md | prompt bar (sidekick), ⌘K search, merge dialog, ink layer, selection bar | ✅ 111 | ☐ |
| 12-terminal-ui.md | terminal registry, modal, transcript preview, agent viewer, xterm behavior | ✅ 185 | 171/185 |

## Phases

Tracks after P1 run in parallel where the dependency column allows.

| Phase | What | Depends on | Status |
| --- | --- | --- | --- |
| P0 | Feature specs and this plan; UI stack spike (S1) | — | ✅ 2,266 behaviors specced; S1 done |
| P1 | Workspace, Rust CI job (macOS + Windows), `nodus-domain`, `nodus-store` (round-trips a real state.json), `nodus-platform` env | P0 | ✅ merged (93 tests) |
| P2 | Session host: `nodus-host-proto`, `nodus-host`, `nodus-sessions` client; wire tests against the Node host both ways | P1 | ✅ merged (host, status) |
| P3 | Data domains: transcripts, board, git, search, summarize, naming, status machine, agents | P1 | ☐ |
| P4 | `nodus-core`: every command and event in api.ts, tested headless | P2, P3 | ✅ all but `capture.*` (capture-panel track) |
| P5 | UI foundation: app shell, theme tokens (light/dark), title bar, menus, settings, projects grid, project rail | P1, S1 | ✅ merged (ui-shell) |
| P6 | Board canvas: pan/zoom/select/drag, cards, sections, archive, notes, edges, controls, context menus, shortcuts | P4, P5 | 🟡 canvas merged; board-cards running |
| P7 | Terminal: element, modal, expanded cards, transcript preview, agent viewer | P2, P5 | ✅ merged; expanded cards plug in through `terminal::card::CardTerminals` |
| P8 | Overlays: prompt bar, ⌘K search, merge dialog, ink, selection bar, agent deck | P6 | 🟡 overlays running (agent deck is board-cards') |
| P9 | Views (wry), board tools, actions, voice, git UI | P6 | ☐ |
| P10 | Capture panel, notifications, Dock badge, updater | P6 | ✅ merged (notifications and the Dock badge need the packaged app to see) |
| P11 | Packaging: .app and DMGs (same bundle id, Info.plist keys, entitlements, signing, notarizing), Windows NSIS (same flags), release workflow; 0.9.4 → Rust update test | P10 | 🟡 merged; waits on Windows CI, the real certificate, and the UI tracks |
| P12 | Parity audit (every checklist item, screenshots light and dark), then cutover: delete Electron, update README and docs, release | P11 | ☐ |

## Tracks

Parallel implementation tracks. Each keeps `migration/tracks/<track>.md` on its branch (done and deferred behavior IDs, interfaces for nodus-core); the orchestrator ticks the spec checklists from it when the branch merges.

| Track | Branch | Scope | Status |
| --- | --- | --- | --- |
| host | `rust/host` | nodus-host-proto, nodus-host, nodus-sessions::client, `Nodus host` | ✅ merged: HOST-001…128 (122 ticked; 033, 043, 064's 4 s exit, 086's 20 s relay timeout, 090, 098 implemented but untested) |
| status | `rust/status` | nodus-sessions hooks, agents, usage, notify, running | ✅ merged: HOST-132…219 (88), 0 deferred |
| transcripts-board | `rust/transcripts-board` | nodus-transcripts, nodus-board, nodus-sessions::launch | ✅ merged: 194/205 SESS; 11 deferred to core (130, 131, 134, 135, 138, 146–148, 153, 164, 203) |
| platform-updater | `rust/platform-updater` | nodus-platform process, secrets, notify, permissions; nodus-updater | ✅ merged: UPD 73, PLAT 11, VOICE 2 (+ OS side of HOST-208…211/215, CAP-032/040/041/055/059); UPD-013/014/076…085 to core/UI |
| parity-harness | `rust/parity-harness` | golden screenshots of 0.9.4 (`migration/parity/`) | ✅ merged: harness, seed and scene list; ~98 scenes × 2 themes captured locally (gitignored) |
| gpui-fork | `rust/gpui-fork` | `third_party/gpui/`: backdrop blur, transforms, group opacity, frame capture (D13) | ✅ merged: Metal + wgpu verified at 60 fps; DirectX 11 HLSL unverified (CI step `continue-on-error` until green) |
| ui-shell | `rust/ui-shell` | nodus-core skeleton (P4a); nodus-ui theme, controls, title bar, menus, settings, projects grid, rail (P5); test driver (D11) | ✅ merged: UI 243, PLAT 64; shell scenes within 0.03–0.3% of the goldens; board route, live counts and Settings rows on other APIs deferred |
| views-ui | `rust/views-ui` | P9, spec 04 UI: view cards, wry webviews with the snapshot swap, the page relay, New view / change dialogs, BOARD-129/130 | 🟡 running |
| views-tools | `rust/views-tools` | nodus-views, nodus-tools, nodus-board notes and ink | ✅ merged: NOTE 16, INK 12, TOOL 40 (004…013/015 by host), VIEW 100; 54 VIEW UI-only to the UI tracks |
| actions-voice-git | `rust/actions-voice-git` | nodus-actions, nodus-voice (incl. the recorder), nodus-git | ✅ merged: 97 (GIT 28, ACT 37, VOICE 32); recorder is Core Audio voice processing + Opus/WebM like MediaRecorder |
| capture | `rust/capture` | nodus-capture backend: double tap, key hook, screen context, guess, submit | ✅ merged: 98 CAP; panel UI and page wiring to the capture-panel track |
| core | `rust/core` | P4: wire every merged domain into nodus-core (sessions, board, views, updates, events, startup/quit), deferred core IDs, debts, headless end-to-end tests | ✅ merged (6243e7c): every domain composed; git/actions/voice wired; the windowed app launches, runs sessions through `Nodus host` and quits through the core |
| overlays | `rust/overlays` | P8, spec 11: prompt bar, ⌘K search, merge dialog, ink layer, selection bar, on the board's `board/bar.rs` hooks | 🟡 running |
| gpui-text | `rust/gpui-text` | patched GPUI text parity with Chromium: sRGB tagging, no SF Pro tracking, hanging trailing spaces, Chromium's rounding; Windows effects CI step | ✅ merged: PATCHES 6–10; 69/71 text cases pixel-identical to Electron 44.5.1; shell scenes now 0.04–0.8% off the goldens |
| board-canvas | `rust/board-canvas` | P6, spec 09: the board route and canvas (pan/zoom/select/drag, sections, edges, menus, shortcuts), node plug-in interface | ✅ merged (f2f5747): 149 ticked; 13 deferred to cards/terminal/views/overlays and 13 board-side overlay hooks (`board/bar.rs`) tick when the overlays land |
| board-cards | `rust/board-cards` | P6, spec 10: session cards, expanded card chrome, agent deck, archive stack, notes, pad (`nodus-ui/src/board/cards/`) | 🟡 running |
| capture-panel | `rust/capture-panel2` | P10, spec 07 UI: the panel window and page, Settings' capture rows, File › Capture…, `nodus.capture()` | ✅ merged (bd3f414): every UI CAP behavior, UI-131…135, PLAT-048; scenes 0.07–0.7% off the goldens; draft/history/clipboard offer kept in state.json and brought over from 0.9.4's Local Storage once |
| packaging | `rust/packaging` | P11: macOS bundle + DMGs, Windows NSIS (electron-builder templates), signing/notarizing, CI packaging + native smoke test, `release-rust.yml` with D12 handoff gates; publishes nothing | ✅ merged (bec0388): `packaging/`, `rust-package.yml`, `release-rust.yml`; arm64 DMG passes 0.9.4's checks locally (ad-hoc); Windows and signing first run in CI / with the certificate |
| actions-git-voice-ui | `rust/actions-git-voice-ui` | P9, spec 05 UI: actions rail and dialog, branch chip + commits popover + diff modal, session mic and recording look | 🟡 running (fresh agent from 4 commits) |
| terminal-ui | `rust/terminal-ui` | P7, spec 12 UI: the GPUI terminal element, session window, transcript preview, agents viewer | ✅ merged (590f9b6): TERM 17, MODAL 27, TRANS 24, AGV 20; windows within 0.04–0.6% of the goldens; expanded-card start/wake rules (TERM-093…108) to board-cards, MODAL-019's recording to the voice UI |
| terminal | `rust/terminal` | nodus-terminal (GPUI-free): alacritty model, xterm key encoder, query answers, widths, registry | ✅ merged: 83 TERM; 26 drawing/focus/timing TERM + MODAL/TRANS/AGV to the terminal UI track |

### Integration debts

Follow-ups a merge left for when the tracks meet in `nodus-core` (P4). Strike them through when done.

- `nodus-sessions::agents` keeps private copies of the discovery.ts helpers it needs; switch it to `nodus-transcripts`' once transcripts-board merges.
- `nodus-sessions::agents` reads a timestamp with no zone as no time, where V8's `Date.parse` reads it as local time. Claude Code always writes UTC, so this only matters if that changes.
- `nodus-sessions::agents` turns a non-string `.meta.json` field into its `String()` form, where 0.9.4 passed the raw value on. Check what the agent viewer shows for one.
- `nodus-board`'s `BoardSessions` and `Launcher` traits are called while the board holds the state, so their nodus-core implementations must not touch the `Store`. `kill` waits up to 4 s.
- `nodus-sessions::launch::SpawnOptions` may duplicate a type the host client defines; keep one.
- "Packaged" is the compile-time flag `NODUS_PACKAGED=1` (`nodus_platform::app::is_packaged`); P11's packaging must set it, or the updater, notifications and the Accessibility reset act as a dev build.
- On Windows, reqwest uses the OS's TLS (SChannel) because ring can't be cross-checked from a Mac; elsewhere rustls. Confirm the Windows CI job builds and the update download works there.
- Windows-only platform code (job objects, DPAPI, toasts, Explorer reveal, AppUserModelID) has passed clippy but first runs in the Windows CI job; watch its first results.
- `nodus-transcripts::claude::spawn` uses tokio's `Command` directly; switch it to `nodus_platform::process` (31013c8).
- `nodus-transcripts` reads only the ISO timestamps Claude Code writes; one with no offset is UTC, where Node reads local time (same as the agents note above).
- The DirectX 11 effect shaders have never been compiled (fxc runs only in Windows release builds). The `effects` example's CI step on `windows-latest` is `continue-on-error`; make it required once it's green.
- `Window::render_to_image()` doesn't include native views (traffic lights, webviews), and wgpu returns an error; parity shots of views need the snapshot path.
- Views need a webview for every view, on screen or not, or `update_view`'s page checks time out (the UI track keeps them alive off screen).
- `update_view` now fails before applying anything when writing `index.html` fails; 0.9.4 applied the title and the rest first. Deliberate, invisible unless the disk is full.
- View command output is decoded whole, not per pipe chunk, so a character split across chunks can differ from 0.9.4 (it was garbled there).
- ~~Text parity: tracking, hanging spaces, rounding, sRGB~~ (gpui-text: PATCHES 6–10; the "tracking" gap was weight 500 → Regular). Two line-clamp cases whose first line breaks inside a word still differ by 1 and 13 anti-aliased pixels.
- The DirectX effect shaders compile with DXC; whether fxc and WARP accept them shows on the next Windows CI run (the step is `continue-on-error` there, required on macOS).
- Thumbnails of copied images are encoded differently from Chromium's, so an image 0.9.4 already offered is offered once more after the upgrade; a non-ASCII host shows as written on a tab tile, where 0.9.4 shows punycode.
- Changed files sort with root collation; 0.9.4 used ICU `localeCompare` in the user's locale.
- ~~Keep one global for the core's runtime~~ (done in f2f5747). Keep one wheel arbiter (board-cards' duplicates the terminal model's; terminal-ui's wheel gate lets only one decide).
- The test driver (`NODUS_AUTOMATION`) listens only on Unix sockets, so the Windows smoke test checks only the window and quit; add a named-pipe listener in `automation.rs`.
- Six host behaviors are implemented but have no test of their own: HOST-033, 043, 090, 098, the 4 s hard exit in HOST-064 and the 20 s relay timeout in HOST-086. Add tests in P4 and tick them.
- The 0.9.4 `pty.ts` → Rust host check was run by hand under vitest and not committed (it has to live in `src/main`); repeat it before cutover.
- The status machine and the host client both rely on `process_started` / `process_exited` being called exactly where pty.ts tracks and untracks a session.

## Spikes

| # | Question | Status | Outcome |
| --- | --- | --- | --- |
| S1 | Does GPUI (+ gpui-component) build and run here, with a pannable canvas, an alacritty terminal on a real PTY, a wry webview inside a GPUI window, and a transparent floating panel over full-screen apps? | ✅ done | Yes for steps 1–6 on stock crates; blur and rotation need the GPUI patch (D13). Report: `migration/spikes/S1-ui-stack.md`, sources in `s1-src/`. |

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
| 06 | When a GitHub lookup times out mid-body the updater says "Couldn't reach GitHub…"; and a non-JSON answer shows serde's parse message where 0.9.4 shows V8's `JSON.parse` message. Both are rare error paths. | Differs; match V8's message if the user asks |
| 12 | alacritty differs from xterm in three small ways: a resize that changes only the column count clears the selection (and reflow is alacritty's), there's no overline (SGR 53), and a selection inside a scroll region moves with it. | Differs; none seen in Claude Code's output |
| 12 | Windows wheel speed in a terminal may be 0.5× where xterm uses 1.25×. | Check against 0.9.4 on Windows |
| 01 PLAT-086/088 | The TS store drops unknown top-level and settings keys, and overwrites a `state.json` that parses but doesn't fit on the next save. | Deliberately safer, invisible to users: unknown keys are kept, and a misfit file is moved aside to `state.json.unreadable-<ms>` |
| 01 | A login shell that hangs ignores the SIGTERM Node sends at the 10 s timeout, so 0.9.4 never stops it. | Killed at 10 s |
| 01, 07 CAP-174/175 | The hidden capture panel stops the Dock from reopening a closed window on macOS, and keeps the app running after its window closes on Windows (the docs say closing the window quits). | Same, unless the user rules them bugs |

## Decisions for the user

Chromium gives 0.9.4 things a native app has no direct equivalent for. Each needs a call before P5 lands; the port's default is in the last column.

| Spec | What Chromium gives today | Proposed native behavior |
| --- | --- | --- |
| 01 | View menu: Reload, Force Reload, Toggle Developer Tools. | **Decided (user, 2026-11-08):** Force Reload and Toggle Developer Tools are dropped. Reload stays only in debug builds. |
| 01 | Page zoom (⌘+ / ⌘- / ⌘0), saved in Chromium's `Preferences`. | Same shortcuts scale the whole UI, read the saved zoom once on first launch and keep it in `state.json` after. |
| 01, 07, 11 | **In place for capture (capture-panel):** `localStorage` keys in Chromium's LevelDB: the capture draft, its attachments, sent-text history, the "clipboard already offered" marker, the ink colour. | Read them once from `Local Storage/leveldb` on first launch and move them into `state.json`. |
| 01 | Spellcheck underlines in text fields. | Native spellcheck where the OS offers it (NSSpellChecker on macOS). |
| 06 | Releasing the Rust app: `release-rust.yml` is started by hand (0.9.4's runs on `v*` tags), creates the release as a draft, checks every asset's digest and only then publishes, and refuses to publish if a handoff gate fails. The JIT entitlements are dropped (WKWebView runs page scripts in WebKit's own process). | Keep the safer flow; switch to tags at cutover if you prefer |
| 12 | Terminals on a zoomed board are a scaled bitmap, so their text goes soft. | Match 0.9.4: draw at 1× and scale the texture, same columns and rows. (Sharp text only if the user asks.) |

**U1 · Sessions running in a 0.9.4 host at the update (specs 02, 06).** Default (a) needs `PROTOCOL` in `nodus-host-proto` bumped to 2 in the first Rust release (it's 1, like 0.9.4, today); that's on the cutover checklist (P12). A running Node host starts every new session through node-pty's `spawn-helper` inside the installed bundle, and may load other files from `app.asar`. Once the Rust app replaces the bundle, new sessions on that host fail with `posix_spawnp failed.` (On Windows, 0.9.4 already stops sessions before an update.) Options: (a) the first Rust release counts as a protocol change, so the existing, documented flow asks to stop those sessions and they resume from their cards; (b) ship a compatible `spawn-helper` at the old path so the Rust app adopts the Node host until it exits. **Default: (a)**, since it's behavior users already know and it ships no Node leftovers.

## Status log

Newest first. One line per event: date, what, branch or commit.

- 2026-11-10: capture-panel merged (bd3f414): the floating capture panel, Settings' capture rows, File › Capture…, the core's `capture.*`; 0.9.4's capture draft and history read from Chromium's Local Storage once; 1,191 tests. A test fixture had been dropped by the repo's `*.log` ignore; committed with an exception.
- 2026-11-09: gpui-text merged: text matches Chromium's (widths, wrapping, clamps, baselines, AA, sRGB); fixed nodus-capture's prune test on Windows, which had kept the Windows effects step from running; 1,170+ tests. views-ui started.
- 2026-11-09: board-canvas merged (f2f5747): the board route and canvas as React Flow did it, with the node plug-in interface and the board's side of the overlays (`board/bar.rs`); one `CoreRuntime` global for board and terminals; 1,170 tests.
- 2026-11-09: terminal-ui merged (590f9b6): terminal element, session window, transcript preview, agents viewer; 1,117 tests.
- 2026-11-09: More watchdog stalls traced to agents reading full-size 2880×1800 screenshots; tracks now compare with the diff tool and look only at small crops. capture-panel continues on `rust/capture-panel2`.
- 2026-11-09: packaging merged (bec0388): macOS bundles and DMGs as 0.9.4's, electron-builder's NSIS templates for Windows, handoff gates replaying 0.9.4's updater, a native smoke test, `rust-package.yml` in CI and a hand-started `release-rust.yml` that refuses to publish on a failed gate. 1,092 tests.
- 2026-11-09: Agents kept dying to the 10-minute no-progress watchdog: with seven tracks building GPUI at once, one `cargo` call ran past it. Now at most five tracks run, each with `CARGO_BUILD_JOBS=4` and long builds in the background; board-cards, terminal-ui, board-canvas and gpui-text restarted as fresh agents from their branches (work committed), capture-panel and actions-git-voice-ui paused. A WIP sweep briefly committed a stale worktree's snapshot onto rust/capture-panel; reset locally, never pushed. The old worktrees `.claude/worktrees/agent-abd95dd991086fde4` and `agent-af174ddfd2ecad8a8` are stale (their work is merged via `rust/capture-panel2`): don't commit there; they can be removed with `git worktree remove --force` (about 15 GB).
- 2026-11-09: All seven UI/packaging tracks stopped at a session limit with work committed (2–7 commits each); resumed after the reset. Disk at ~28 GB free with seven builds.
- 2026-11-09: core phase 2 merged (6243e7c): git/actions/voice in the core; every core command runs on the core's tokio runtime whatever awaits it; the GUI opens after `launch()`, starts hosts through `Nodus host`, has live counts, Settings' voice and update rows, Quit and Stop Sessions; 1,092 tests.
- 2026-11-09: core's first part merged (3cd11cf): the whole app runs headless in tests with a stand-in claude and the real Rust host; integration debts resolved; 1,088 tests. Weekly usage limit hit on one account; resumed on another.
- 2026-11-09: parity-harness merged: `node migration/parity/capture-electron.mjs` regenerates the goldens from 0.9.4 with synthetic seeded data (see migration/parity/README.md). capture-panel restarted fresh after stalling with nothing committed.
- 2026-11-09: terminal merged (75ea2b4): nodus-terminal checked against xterm.js 6's own code (3,083 key presses, 23 screens cell by cell, every code point's width, 18 registry scenarios); 1,070 tests. Every other running agent stopped at a usage limit again; resumed after the reset.
- 2026-11-09: Windows CI ran the Rust tests for the first time: all passed except one quoting test whose expectation was wrong (fixed in cd25612). gpui-text, board-canvas, board-cards and capture-panel started.
- 2026-11-09: ui-shell, capture and actions-voice-git merged (fac662d); 996 tests; 1,160 of 2,266 behaviors ticked. The golden PNGs had been swept into two plan commits by `git add migration`; untracked in 954e195 (synthetic content only; one file named this Mac's temp folder id).
- 2026-11-09: ui-shell's core skeleton cherry-picked to main (a9d528f) so track core (P4) could start; 715 tests.
- 2026-11-09: views-tools merged (2c4bf9d): nodus-tools MCP server, `McpRelay`, nodus-views (files, page, commands, brains, schedules), notes placement and ink; MCP answers and view pages byte-identical to 0.9.4; 687 tests.
- 2026-11-09: gpui-fork merged (9ea68bd): vendored gpui-pre 0.3.11 with effect layers. `.backdrop_blur(px).backdrop_saturate(s)`, `.rotate/.scale/.translate/.transform_origin`, `.group_opacity`, `Window::render_to_image()`; the Metal compiler crash was passing `Background` by value into multi-stop gradient code. Docs in `third_party/gpui/PATCHES.md`.
- 2026-11-09: host merged (f0b6df8): wire protocol, PTYs as node-pty, hook server, MCP relay, `Nodus host`, the app's client; wire-compatible with the 0.9.4 Node host in both directions; 558 tests. Every running agent stopped at a usage limit again; resumed on a new account.
- 2026-11-08: platform-updater merged (6d4dc2b): safeStorage-compatible secrets (OSCrypt parameters checked against Electron 44.5.1), notifications, Dock badge, links, permissions, nodus-updater; 446 tests.
- 2026-11-08: `nodus_platform::process` cherry-picked to main from platform-updater (31013c8) so every track spawns through it; 338 tests. actions-voice-git started.
- 2026-11-08: transcripts-board merged (eee039a): nodus-transcripts, nodus-board, nodus-sessions::launch; 317 workspace tests. Spec 03 fixed: Node's readdir is sorted on Unix.
- 2026-11-08: status merged (3ae719c): status machine, hook settings, usage, agents, notify, running counts; 172 workspace tests. Checklists are ticked with `python3 migration/tick.py <IDs>`.
- 2026-11-08: S1 done (D1–D3, D13 adopted). gpui-fork and ui-shell started.
- 2026-11-08: Every agent stopped at an account usage limit; the account was switched. host, status, transcripts-board and platform-updater restarted fresh (nothing was committed yet); S1 and parity-harness resumed with their work.
- 2026-11-08: P1 merged from `rust/p1-foundations`: workspace with 19 crates, nodus-domain (38 tests), nodus-store (28), nodus-platform env (27), Rust CI on macOS and Windows.
- 2026-11-08: All 12 feature specs on main (2,266 behaviors). Golden screenshot harness started (`rust/parity-harness`).

- 2026-11-08: Plan written. Rust 1.99.1 installed with rustup (no shell profile edits; `source ~/.cargo/env`). Spec agents and spike S1 launched.

## Resuming after an interruption

1. Read this file, then `git log --oneline -20` and `git branch -a | grep rust/`.
2. Find your track's row in **Phases** and its feature file; the unticked checklist items are what's left.
3. A `rust/<track>` branch with commits not on `main` is half-done work: read its last commit message and diff before continuing.
4. Update the status log and the tables here when you finish a slice, in the same commit.
