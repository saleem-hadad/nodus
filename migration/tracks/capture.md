# Track: capture (P3)

- **Branch:** `rust/capture`
- **Spec:** `migration/features/07-capture.md` (CAP-001…CAP-177)
- **Crate:** `nodus-capture` (no GPUI; the panel window and its page are a later UI track)
- **Sources:** `src/main/capture.ts`, `captureGuess.ts` (+ test), `doubleTap.ts` (+ test), `screenContext.ts` (+ test), `src/shared/capture.ts` (+ test), the capture handlers in `src/main/ipc.ts`; the non-drawing logic of `CaptureApp.tsx`; libuiohook 1.5.5's `darwin/input_hook.c` and `windows/input_hook.c` (what uiohook-napi hears)

## Modules

| Module | Ports | Tests |
| --- | --- | --- |
| `text` | shared/capture.ts: `title_from`, `describe_attachments`, `with_screen`, `fence` | capture.test.ts (9 cases) + edges checked under Node |
| `double_tap` | doubleTap.ts | doubleTap.test.ts (10 cases) + 3 |
| `guess` | captureGuess.ts: prompt, verbatim argv, `read_guess`, `Guesser` (one at a time, cancel = SIGKILL + `None`, 30 s) | captureGuess.test.ts (6 cases) + stand-in `claude` runs |
| `screen` | screenContext.ts + `describeFiles`: lsappinfo, tab AppleScripts, screencapture, Windows GDI shot, clipboard, thumbnails, files, 30-day sweep | screenContext.test.ts (7 cases) + stand-in `lsappinfo`/`osascript`/`screencapture`, a fake clipboard, synthetic images |
| `hook` | uiohook-napi: CGEventTap (macOS), WH_KEYBOARD_LL/WH_MOUSE_LL (Windows), translation to libuiohook's events, `Listener` | synthetic OS events end to end; no real tap |
| `targets` | `sessionInfo`, `recentOf`, `defaultProject`, `targets`, `remember` | synthetic state.json |
| `submit` | `cleanAttachments`, `spotOnBoard`, `submit` (notes are sized and placed with nodus-board's `notes::fit_note` and `notes::obstacles`) | fake handlers over a real `Store` in a temp folder |
| `service` | the rest of capture.ts: open/hide/toggle/rest/aside, blur rule, resize, shortcut state, hook, permission poll, Allow…, every `capture:*` command | fake panel, screen, hook and permissions, paused clock |
| `page` | CaptureApp.tsx logic: route, pick/cycle kinds, `GuessState`, history, offer key, sizes/ages/clips, copy | spec's suggested tests, arithmetic checked under Node |

Tests never touch the user's screen, clipboard, front app or browser: the system implementations (`SystemScreen`, `SystemClipboard`, `SystemHook`, `SystemPermissions`) are behind traits and only fakes or stand-in scripts run.

## Counts

108 done, 19 with their logic ready for the UI, 45 deferred to the panel UI, 5 deferred for other reasons (177).

## Done (implemented and tested)

- CAP-001 (lazy panel via `PanelWindow::create`, 3 s pre-create), CAP-007 (constants; the UI makes the window that size), CAP-008, CAP-009, CAP-012 (`Capture::on_blur`, `blur_counts`), CAP-014…CAP-020
- CAP-021…CAP-041, CAP-043, CAP-044 (040/041's OS steps are `nodus_platform::permissions::request_accessibility`)
- CAP-045, CAP-046, CAP-048…CAP-050, CAP-052…CAP-055, CAP-057…CAP-061, CAP-063
- CAP-064…CAP-066 (the rules in `page::GuessState`; the UI runs the 700 ms timer), CAP-067…CAP-080
- CAP-084, CAP-113…CAP-116 (`page::route`, `pick_kind`, `cycle_kind`, `pick_project`, `pick_session`)
- CAP-125…CAP-139, CAP-145, CAP-148…CAP-151, CAP-154…CAP-156, CAP-158…CAP-165, CAP-168, CAP-169 (blank name is the page's), CAP-171, CAP-173, CAP-176

## Logic ready, UI wiring left

The page's rules are in `page.rs` (and `service.rs` for timings) with tests; the panel UI track draws and wires them. Tick these when the panel lands.

- CAP-051 (`with_tab`), CAP-092 (`host`), CAP-094 (`bytes`), CAP-098 / CAP-167 (`offer_key`), CAP-103 (`append_transcript`), CAP-106 / CAP-109 (`kind_copy`), CAP-111 (`sessions_count`, `MENU_SESSIONS`), CAP-122 (`history_up`/`history_down`), CAP-123 (`GuessState::must_wait`, `remember_sent`, `MAX_HISTORY`), CAP-141 / CAP-143 (`clip`), CAP-147 (`RECEIPT_MS`), CAP-152 / CAP-170 (`ago`, `recent_status`), CAP-153 (`more_recents`), CAP-166 (`with_tab`), CAP-010 (`MENU_ROOM`), CAP-090 (`MAX_ROWS`), CAP-118 (`POLL_MS`)

## Deferred

- **UI (panel window and page):** CAP-002…CAP-006, CAP-010, CAP-011 (no web page; nothing to navigate), CAP-013 (`PanelWindow::exists` turns false when the window closes), CAP-042 (the File › Capture… item; it calls `Capture::show`), CAP-047 (the page doesn't show `app`), CAP-062 (drag and drop; dropped paths go to `Capture::files`), CAP-081…CAP-083, CAP-085…CAP-091, CAP-093, CAP-095…CAP-097, CAP-099…CAP-102, CAP-104, CAP-105, CAP-107, CAP-108, CAP-110, CAP-112, CAP-117, CAP-119…CAP-121, CAP-124 (show `SubmitError`'s plain text), CAP-140, CAP-142, CAP-144, CAP-146, CAP-177, and the wiring of the "logic ready" list above.
- CAP-056: the Windows screenshot (GDI BitBlt of the monitor under the pointer → PNG) and the Windows clipboard (CF_UNICODETEXT, registered `PNG`, CF_DIB → BMP) pass clippy for `x86_64-pc-windows-msvc` and `dib_to_bmp` is tested here, but they first run in the Windows CI job.
- CAP-157, CAP-172: where the page's `capture.draft`, `capture.attachments`, `capture.history` and `capture.clipboardDone` live is the user's decision in plan.md ("Decisions for the user", localStorage row). Their rules (removed when empty, history ≤ 50 unique newest first, offer key) are in `page.rs`.
- CAP-174, CAP-175: app-lifecycle quirks of the hidden panel; decide with 01 (plan.md "Quirks").
- The real CGEventTap / low-level hooks, NSPasteboard and the real screencapture need Accessibility, the user's clipboard or screen, so they're never run in tests; check them in the packaged app (P10).

## Traits nodus-core must implement

- `CaptureHandlers` (submit.rs), the TS `CaptureHandlers`:
  - `window_size()` → the board window's content size while open (CAP-134);
  - `create_session(project_id, opts)` → `nodus_board::Board::create_session(project_id, &opts, launcher)`, answering the card's `session_id` (errors as their text);
  - `send_to_session(project_id, session_id, text)` → `Board::send_to_session(…, launcher)` (its `BoardError` text, e.g. `It’s waiting for your answer. Open it to reply.`);
  - `session_title(project_id, session_id)` → `Board::session_title` (fall back to `New session` on error);
  - `create_view(project_id, description, at)` → nodus-views' `createView` (title `New view`, brain started), answering the view id;
  - `open_card(request)` → `nodus_sessions::notify::Notify::open_card`;
  - `open_settings()` → bring the board window up (re-created if closed) and send `Event::Menu(Settings)` once it has loaded.
- `CaptureSessions` (targets.rs): `is_live` and `status` from the session host and the status machine, `meta(repo_path, session_id)` from `nodus_transcripts::Transcripts::session(..).map(|d| d.meta)`. Called while capture reads the `Store`: must not touch it.
- `PanelWindow` (service.rs): the UI's panel window (any thread; carry to the UI thread). `pick_files` is the open dialog attached to the panel.
- `ClaudeEnv` is implemented for `nodus_platform::Env`; `Screen`, `Permissions`, `KeyHook` have system implementations (`SystemScreen`, `SystemPermissions`, `SystemHook`).

### Wiring

```rust
let capture = Capture::new(CaptureDeps { store, user_data, handlers, sessions, panel, screen: Arc::new(SystemScreen),
    permissions: Arc::new(SystemPermissions), hook: Arc::new(SystemHook), claude: env, events, runtime, clock });
capture.register();                         // at startup: sweep captures/, sync the shortcut (CAP-038, CAP-063)
capture.sync_shortcut();                    // every settings:update (applySettings)
capture.board_opened(&project_id);          // board:load (CAP-154)
capture.show().await;                       // menu File › Capture…, capture:show
capture.stop();                             // shutdown / will-quit (CAP-039)
// capture:* → ready, targets, guess(text, &attachments), submit(&raw), screenshot, allow_screen, files(&paths),
//             clipboard, hide, resize(h), open(&request), settings, shortcut_state, allow
// The UI calls on_blur() when the panel resigns key.
```

Events: `Event::CaptureShown`, `Event::CaptureContext` (only while the panel's page has loaded) and `Event::BoardChanged` after a capture.

## What the panel UI track needs from here

- `service::PanelWindow` to implement (an NSPanel: non-activating, key without activating, all Spaces + full screen, level 101, transparent, no shadow, accepts first mouse; Windows: tool window, topmost), `Capture::on_blur` on resign-key, `Capture::resize` from its measured height (`MENU_ROOM`), `place`/`resized`/`blur_counts` and the constants (`PANEL_W`, `PANEL_H`, `BLUR_GRACE` 250 ms, `BLUR_RECHECK` 50 ms, `SCREENSHOT_SETTLE`).
- `page.rs` for every rule the page applies, and `kind_copy` for the kind words.
- A home for the draft/history values (CAP-157) once the user decides.

## Notes for integration

- libuiohook reads Shift from the device-independent flag, so letting go of one Shift while the other is held reads as a press; Caps Lock and media keys come as `NX_SYSDEFINED` key events; another mouse button coming up counts as a click. The port does the same. The tap is re-enabled on both timeout and user-input disables (libuiohook only on timeout).
- Thumbnails are PNGs made by the `image` crate (Catmull-Rom down to 360 px), so their bytes differ from Chromium's; an image clipboard already offered under 0.9.4 is offered once more after the upgrade (its offer key is the thumbnail's tail).
- `clean_attachments` reads a file size as `Number(size) || 0` but keeps it a whole, non-negative number (the domain type is `u64`).
- Tests that run a stand-in script give it generous timeouts: macOS checks a script it hasn't run before, which can take over a second on a busy machine.
