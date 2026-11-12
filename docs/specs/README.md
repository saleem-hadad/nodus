# Behavior specs

Every behavior of Nodus, one file per domain, each with an ID (`SESS-042`, `UI-105`). They were written from the TypeScript of 0.9.4, the Electron app, before Nodus was rebuilt in Rust, and every one was ported and checked off before 1.0.0. Comments and test names cite the IDs where a behavior is implemented (`// SESS-042`).

When you change a behavior, change its entry in the same commit, and say what 0.9.4 did if it differs (as UI-105 does).

| File | Covers |
| --- | --- |
| `01-platform-shell.md` | app lifecycle, menu, environment, store, settings |
| `02-session-host-status.md` | session host and its protocol, terminals, the status machine, agents, usage, notifications |
| `03-sessions-transcripts.md` | creating, opening and stopping sessions, claude's arguments, transcripts, search, summaries, naming |
| `04-board-tools-views.md` | the board tools' MCP server, where notes go, views, ink storage |
| `05-actions-voice-git.md` | the actions rail, voice prompts, git status, diff and log |
| `06-updater-release.md` | updates, packaging, signing, CI, releases, the smoke test, the 0.9.4 handoff |
| `07-capture.md` | floating capture: the panel, the double tap, screen context, the guess, submitting |
| `08-projects-shell-ui.md` | title bar, settings, projects grid, project rail, theme tokens, menus, modals |
| `09-board-canvas.md` | pan and zoom, selection, dragging, shortcuts, context menus, sections, archive, layout |
| `10-board-cards-notes.md` | session cards, expanded cards, the agent deck, the archive stack, notes and the pad |
| `11-board-overlays.md` | the prompt bar, ⌘K search, the merge dialog, ink, the selection bar |
| `12-terminal-ui.md` | terminals, the terminal modal, transcript preview, the agent viewer |
| `13-feedback.md` | Help › Report a Problem…: the dialog, screenshots, the issue it becomes, the feedback server |
| `14-board-shapes-links.md` | Shapes, stickers, pictures and links on the board, and the dock tools that make them |

## Compatibility with 0.9.4

Installed copies of 0.9.4 update straight to the latest release, so every release keeps:

- **Its data.** The same userData folder (`~/Library/Application Support/Nodus`, `%APPDATA%\Nodus`), the same `state.json` schema (read and write), the same `views/` and `captures/` layout, the same `NODUS_USER_DATA` override, bundle id `dev.saleem.nodus` and release asset names.
- **Its session host's wire format.** The Rust host speaks 0.9.4's messages byte for byte, with `PROTOCOL` 2 where 0.9.4 says 1 (U1 below).

## JavaScript semantics to keep

0.9.4's TypeScript ran on V8, so some of its behavior came from JavaScript itself. The Rust code reproduces these, not Rust's defaults:

- **String lengths are UTF-16 code units.** Every truncation, clip, window and highlight range (`slice`, `length`, `substring`) counts UTF-16 units, not bytes or chars. Use the helpers in `nodus-domain`.
- **Numbers are f64.** Times and coordinates can be fractional (file mtimes become `createdAt`), so they're `f64`, and they serialize like `JSON.stringify`: `1700000000000`, not `1700000000000.0`. **Write every JSON through `nodus_domain::json`**, never `serde_json::to_string` directly.
- **`delete` is `shift_remove`.** With `preserve_order`, serde_json's `Map::remove` swaps the last key into the hole; use `shift_remove` (on `IndexMap` too) so the order stays as JavaScript leaves it.
- **`Math.round` rounds .5 up**, toward +∞ (`Math.round(-2.5) === -2`); Rust's `f64::round` rounds away from zero.
- **Objects keep insertion order.** serde_json uses `preserve_order`; maps keyed by id are `IndexMap`. Sorts are stable.
- **Regexes:** JS `\s` includes U+FEFF and other Unicode spaces; JS `\w`, `\d` and `\b` without the `u` flag are ASCII-only. Check every ported regex.
- **Paths:** Node's `realpath` keeps the case it's given and never adds `\\?\` on Windows; Rust's `canonicalize` may do both. The transcript folder name depends on this.
- **Dates:** `Date.parse` is lenient; match what the code actually feeds it.

## Layers

A crate depends only on crates in lower layers. When a crate needs something from a crate in its own layer or above (views starting a brain session, the board asking a session's status), it declares a small trait for it, and `nodus-core` implements that trait by calling the other crate. That keeps every domain testable on its own and lets a headless server wire them differently.

| Layer | Crates |
| --- | --- |
| L0 | `nodus-domain` (types, events, JSON as JavaScript writes it, UTF-16 helpers) |
| L1 | `nodus-store`, `nodus-platform`, `nodus-host-proto`, `nodus-terminal` |
| L2 | `nodus-transcripts`, `nodus-git`, `nodus-host` |
| L3 | `nodus-sessions`, `nodus-board` |
| L4 | `nodus-views`, `nodus-tools`, `nodus-actions`, `nodus-voice`, `nodus-capture`, `nodus-updater`, `nodus-feedback` |
| L5 | `nodus-core` |
| L6 | `nodus-ui` |
| L7 | `nodus` (the binary) |

## What replaced Chromium

Chromium gave 0.9.4 things a native app has no direct equivalent for. What the Rust app does instead:

| Spec | In 0.9.4 | In the Rust app |
| --- | --- | --- |
| 01 | View menu: Reload, Force Reload, Toggle Developer Tools. | Force Reload and Toggle Developer Tools are gone. Reload is only in debug builds. |
| 01 | Page zoom (⌘+ / ⌘- / ⌘0), saved in Chromium's `Preferences`. | The same shortcuts scale the whole UI. The saved zoom is read once, on first launch, and kept in `state.json` after. |
| 01, 07, 11 | `localStorage` keys in Chromium's LevelDB: the capture draft, its attachments, sent-text history, the "clipboard already offered" marker, the ink colour. | Read once from `Local Storage/leveldb` on first launch and moved into `state.json`. |
| 01 | Spellcheck underlines in text fields. | The OS's own spellchecker where it has one (NSSpellChecker on macOS). |
| 12 | Terminals on a zoomed board are a scaled bitmap, so their text goes soft. | The same: drawn at 1× and scaled, with the same columns and rows. |

## Decisions

What the Rust app was built on. Comments cite them by number (`decision D3`).

| # | Decision |
| --- | --- |
| D1 | **UI: GPUI through `gpui-kit =0.9.0`** (`gpui-pre =0.3.11`, a weekly snapshot of Zed's GPUI, plus `gpui-component 0.9.0`), with `runtime_shaders`. Native Metal / DirectX rendering, no webview for the app's own UI. Steps 1–6 of S1 pass on stock crates. |
| D2 | **Terminal: `alacritty_terminal =0.26.0`** (Apache-2.0) for VT parsing and the grid, wrapped GPUI-free in `nodus-terminal` and drawn by our own GPUI element in `nodus-ui`. Zed's `terminal_view` is GPL, so nothing is copied from it. |
| D3 | **Views' pages: the system webview through upstream `wry =0.57.1`** (WKWebView on macOS, WebView2 on Windows), embedded in the board. A native view always draws above GPUI, so whenever anything overlaps a view, it's dragged, or an ink tool is out, the board shows a snapshot of it instead (2–16 ms). Views are HTML that Claude writes; everything else is native. |
| D5 | **One binary.** `Nodus` runs the app; `Nodus host …` runs the session host (no Dock icon, outlives the app), as the Electron app runs its own binary as Node for `host.js`. |
| D6 | **Secrets:** read the 0.9.4 `safeStorage` ciphertext so a saved xAI key survives the upgrade. macOS: Chromium OSCrypt, password in the Keychain item service `Nodus Safe Storage`, account `Nodus Key`; the Rust app is signed with the same Team ID and bundle id so the Keychain doesn't prompt. Windows: AES-256-GCM with a key kept in `Local State` and protected by DPAPI. Exact parameters per spec 01. |
| D8 | **No visible difference for someone moving from 0.9.4.** Same layout, colours, type, motion, shortcuts, gestures, copy and flows. Golden screenshots of 0.9.4 for every screen and state, light and dark, from a synthetic seeded world, come from `crates/nodus-ui/parity/harness/`; the Rust app renders the same scenes and was compared with them side by side (with a perceptual diff as an aid). Same fonts (system UI font, SF Mono / Cascadia) and metrics. |
| D9 | **Windows installer: electron-builder's NSIS templates (MIT), built with `makensis`.** Every installed copy since 0.9.0 runs `Nodus-Setup-<arch>.exe --updated /S --force-run`, and expects the same registry id, the same `%LOCALAPPDATA%\Programs\nodus` folder, the old uninstaller run with data and shortcuts kept, and the app started again. cargo-packager's installer does none of that. |
| D10 | **Same Developer ID certificate (`CSC_LINK`), bundle id, macOS 13.0 minimum, arm64 and x64 on both OSes.** Copies since 0.8.1 reject an app signed by another team, and another identity loses the macOS permission grants and the Keychain item. Intel copies on Apple silicon and x64 copies on Windows on Arm download the arm64 asset. |
| D11 | **A test driver built into the app, off unless `NODUS_AUTOMATION` names a socket.** It runs core commands, sends input, and takes screenshots of scenes. It replaces 0.9.4's DevTools-protocol smoke test (`scripts/smoke.mjs`) and renders the Rust side of the golden screenshots. |
| D12 | **Handoff gates in the release workflow.** Before upload, CI replays 0.9.4's exact updater checks on each DMG, and installs a real 0.9.4 and updates it to the new build on Windows. `releases/latest` is the first place 0.9.4 sees a release, so there's no rehearsal after that. |
| D13 | **We own a patched GPUI.** Stock GPUI can't blur behind an element (the prompt bar, menus, toasts, modals and archive panel all do) or rotate boxes and text (dragged notes, stickers, the pad, the agent fan), and fades elements primitive by primitive rather than as a group. The patch set: the effect-layers PR stack (zed#63773–#63780, backdrop blur on Metal, wgpu and DirectX 11) with a real fix for the two shaders Apple's Metal compiler crashes on, transforms on offscreen layers, and group opacity. The patched `gpui-pre*` crates are vendored under `third_party/gpui/`, keeping their package names and version, and swapped in with `[patch.crates-io]`. |

**U1 · Sessions running in a 0.9.4 session host at the update.** A running 0.9.4 host starts every new session through node-pty's `spawn-helper` inside the installed bundle, so once the Rust app replaces the bundle, new sessions on that host would fail. The first Rust release is therefore a protocol change: `PROTOCOL` is 2 (0.9.4 says 1), and an updated app that finds a 0.9.4 host asks to stop its sessions through the existing flow, and they resume from their cards. Windows is unaffected, since 0.9.4 stops its sessions before an update.

## History

The rewrite's plan, status log, per-track notes and UI-stack spike (S1) were removed after 1.0.0. They're in git at 85fc86a: `git show 85fc86a:migration/plan.md`, and `git ls-tree -r --name-only 85fc86a migration/` for the rest.
