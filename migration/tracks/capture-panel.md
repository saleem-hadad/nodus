# Track: capture-panel (P10)

- **Branch:** `rust/capture-panel2` (from `rust/capture-panel` at adc385b, rebased on main)
- **Spec:** `migration/features/07-capture.md` (the panel UI CAP behaviors), spec 08 UI-131…UI-135 (Settings' capture
  rows), spec 01 PLAT-048 (File › Capture…)
- **Crates:** `nodus-ui` (`src/capture/`: the panel window and page), `nodus-core` (`src/capture.rs`: `nodus.capture()`;
  `src/local_storage.rs`: the pages' `localStorage` values)
- **Sources:** `src/renderer/src/capture/CaptureApp.tsx`, `capture.css`, `src/main/capture.ts`, the capture handlers in
  `src/main/ipc.ts`; backend on main in `nodus-capture` (see `migration/tracks/capture.md`)

## Status

| Piece | State |
| --- | --- |
| Track file | ✅ |
| Core: `nodus.capture()` (`CaptureHandlers`, `CaptureSessions`), `boardOpened` before `board.load`, `stopCapture` in shutdown, the shortcut synced on every settings change | ✅ |
| Core: `nodus.local_storage()` (state.json `localStorage`, 0.9.4's Chromium LevelDB brought over once) | ✅ |
| Panel window (`PanelBridge`/`PanelSlot`, macOS NSPanel, Windows pop-up) | ✅ |
| Panel page: header, field, tiles, footer, mic, menu, banners, recents, receipt, keys, drops | ✅ |
| Settings' capture rows, File › Capture… | ✅ |
| Scenes vs goldens | ✅ (see below) |

## How to resume

Read this file, then `git log --oneline main..rust/capture-panel2`. Scenes: `crates/nodus-ui/parity/capture_scenes.py`
(10 panel scenes plus `settings` and `settings-ready`), compared with `compare.mjs`; give it `--golden`, `--world` and
`--node` (22+) outside the repo and `NODE_PATH=<migration/parity>/node_modules`.

## Done (implemented and verified)

Verified side by side against `migration/parity/golden/capture-*.{light,dark}.png` (pixel diff at threshold 0.1, both
themes): capture-empty 0.09/0.14 %, capture-recents 0.23/0.26 %, capture-guessing 0.07/0.11 %, capture-guess
0.20/0.23 %, capture-kind-menu 0.32/0.36 %, capture-attachments 0.32/0.40 %, capture-receipt-note 0.73/0.27 %,
capture-receipt-session 0.55/0.47 %, capture-permission 0.19/0.51 %, capture-screen-permission 6.08/10.48 % (one
wrap, below); settings 0.59/0.58 %; settings-ready's Floating capture rows 2.2/2.6 % (one wrap; the rest of that shot is
the voice and update rows' stubs, other tracks').

- **Window:** CAP-001…CAP-007 (GPUI's pop-up NSPanel: non-activating, key without activating Nodus, every Space and
  over full-screen apps, level 101, see-through, no shadow, fixed, title `Nodus Capture`, accepts the first click),
  CAP-010 (the page measures the stage and the open menu + 48 and asks `capture:resize`), CAP-011 (no web page: nothing
  navigates), CAP-012 (resigning key calls capture's blur rule: 250 ms grace, 50 ms re-check, busy and resting), CAP-013,
  CAP-042, PLAT-048 (File › Capture… opens it with or without a main window)
- **Context and guess:** CAP-047, CAP-051, CAP-062, CAP-064…CAP-066, CAP-166
- **Page:** CAP-081…CAP-124 (all: header, field with IME, tiles, offer, paperclip, scan and its notice, error line, mic
  with 05's voice controller, its status and notices, Esc for the mic, hint, send, menu, kinds/projects/sessions,
  banner, Return/Tab/Esc/↑/↓, the send flow and its errors)
- **Receipt and recents:** CAP-140…CAP-144, CAP-146, CAP-147, CAP-152, CAP-153, CAP-170, CAP-177
- **Storage:** CAP-157, CAP-167, CAP-172 (draft, attachments with thumbnails, history and the offer key in state.json's
  `localStorage`, as the plan's default has it; 0.9.4's own values are read from `Local Storage/leveldb` the first time,
  checked against a log Electron 44 wrote)
- **Quirks kept (plan.md "Quirks", "Same"):** CAP-174 (the hidden panel counts as a window, so a Dock click doesn't
  reopen a closed board window), CAP-175 (off a Mac, once the panel was made, closing the board window doesn't quit)
- **Settings (spec 08):** UI-131 (switch), UI-132 (state read at once, every 2 s, and when the setting changes),
  UI-133 (Allow… asks and shows the state returned), UI-134, UI-135 (Try It opens the panel)
- **Core tests:** capture follows the setting with its key hook and stops it on quit; hears which board opened before
  it loads; local storage kept across restarts, brought over from Chromium once.

## Deferred

- CAP-056: the Windows screenshot and clipboard (backend) first run in the Windows CI job.
- CAP-008 off macOS: GPUI doesn't say where the pointer is outside a window, so the panel goes on the primary
  display's work area there (macOS uses the screen under the pointer). Windows CI / a Windows machine to check.
- Windows hides the panel by closing its window (GPUI has no hide there) and makes it again on the next show; looks
  the same, unverified until Windows CI.

## Remaining visible differences

- Line wraps a word early in a few places (the screenshot notice: "on" goes to the next line; Settings' capture
  description): GPUI counts a line's trailing space where Chromium lets it hang. That's the gpui-text track's
  "hanging trailing spaces" fix.
- Shadows under see-through boxes (the recents' "more" pill) show through faintly: CSS clips an outer shadow under its
  box, GPUI doesn't.
- The voice notice in the card is laid out as a row with the dismiss button under it; 0.9.4 runs it as inline text
  (no golden; spec 07 "UI details" calls it out as unclear).

## Edits in other tracks' files

(additive only)

- `crates/nodus-core/src/board.rs`: `board.load` tells capture first (on the core's runtime).
- `crates/nodus-core/src/lib.rs`, `nodus.rs`: `mod capture`, `mod local_storage`, `Nodus::capture()`,
  `Nodus::local_storage()`, `Inner.capture`, `shutdown` stops capture.
- `crates/nodus-core/src/voice.rs`: `VoiceApi::run` (a mic's work on the core's runtime).
- `crates/nodus-ui/Cargo.toml`: `nodus-voice` (the same line the actions-git-voice-ui track adds).
- `crates/nodus-ui/src/app.rs`: `capture::register` at startup; CAP-175's condition on the last window closing.
- `crates/nodus-ui/src/shell/menus.rs`: File › Capture… opens the panel.
- `crates/nodus-ui/src/settings/mod.rs`: the capture rows' polling, Allow… and Try It.
- `crates/nodus-ui/src/automation.rs`: the driver hands `capture`, `type` and the panel's shots to `capture::automation`.
