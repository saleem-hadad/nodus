# 07 · Floating capture

Sources:
- `src/main/capture.ts` (panel window, key hook, permission, targets, recents, submit)
- `src/main/captureGuess.ts`, `src/main/captureGuess.test.ts` (Claude's guess)
- `src/main/doubleTap.ts`, `src/main/doubleTap.test.ts` (double tap of ⇧)
- `src/main/screenContext.ts`, `src/main/screenContext.test.ts` (front app, browser tab, screenshot, clipboard, `captures/` cleanup)
- `src/shared/capture.ts`, `src/shared/capture.test.ts` (kinds, titles, the prompt's "From my screen" block)
- `src/renderer/src/capture/CaptureApp.tsx`, `src/renderer/src/capture/capture.css` (the panel page)
- `src/renderer/src/styles/tokens.css`, `src/renderer/src/styles/global.css` (tokens, base element styles, reduced motion)
- `src/renderer/src/components/board/board.css` lines 36–59 (global `.spin` and `kbd` rules, which also apply inside the panel because the panel loads the same CSS bundle)
- `src/renderer/src/components/voice/*` (mic hook and parts, used by the panel; owned by 05)
- `src/renderer/src/components/FolderIcon.tsx` (`FOLDER_HUES`, owned by 08)
- `src/renderer/src/main.tsx` (the `#capture` route), `src/renderer/src/platform.ts` (`keys()`)
- `src/main/ipc.ts` (`registerCapture` handlers, `createSession`, `sendToSession`, `sessionTitle`, `boardOpened`, `applySettings` → `syncShortcut`, `shutdown` → `stopCapture`)
- `src/main/menu.ts` (File › Capture…), `src/main/notify.ts` (`openCard`), `src/main/store.ts` (`captures`, `settings.capture`)
- `src/shared/types.ts` lines 533–534 and 612–716 (capture types), `src/shared/api.ts` (`capture.*`, `on.captureShown`, `on.captureContext`)
- `docs/how-it-works.md` § Floating capture

## Summary

Double-tapping ⇧ in any app (or File › Capture…) floats a panel up over whatever is in front, on the screen the pointer is on, over full-screen apps and on every Space, without bringing Nodus forward. The user types (or speaks) something. On each pause in the typing, a headless `claude -p` call to Haiku guesses what it is (a new session, the next prompt **into** an existing session, a **note**, or a **view**) and which project it belongs to; the header shows the guess, Tab/⇧Tab or a menu changes it. The browser tab in front, a screenshot, copied text or image and files come along as attachment tiles. Return makes it on the project's board (session in auto mode as an expanded card with a yellow note stuck on it; note; view; or a prompt sent into a running session), a receipt card shows for 2.6 s while the keyboard goes back to the previous app, and the panel hides. Recent captures (up to 20, stored in `state.json`) show under the empty panel with live status. Screenshots and copied images live in `<userData>/captures/` and are deleted after 30 days.

The panel is a second window loading the renderer at `#capture`; in Rust it is a second GPUI window (an `NSPanel` on macOS) driven by `nodus-capture` through `nodus-core`.

## Behaviors

### Window

- **CAP-001** The panel is one window, created lazily the first time it is needed (first open, or 3000 ms after the key hook starts, see CAP-036) and reused afterwards; if it was destroyed, the next use creates a new one.
- **CAP-002** The panel window is created hidden, frameless, fully transparent (background `#00000000`), with no window shadow (the card draws its own), not resizable, not movable, not minimizable, not maximizable, not full-screenable, not shown in the taskbar / app switcher, always on top, and with the title `Nodus Capture`.
- **CAP-003** The panel accepts the first mouse click (acceptFirstMouse): a click on the receipt's link works even while the panel does not have the keyboard.
- **CAP-004** macOS: the window is an `NSPanel` (Electron `type: 'panel'`): it becomes key (takes the keyboard) without activating Nodus, so Nodus's board window and the Dock do not come forward and the app in front stays the active app.
- **CAP-005** macOS: the panel is visible on all workspaces (Spaces) and over full-screen apps, without changing Nodus's activation policy (Nodus keeps its Dock icon; Electron's `skipTransformProcessType: true`).
- **CAP-006** The panel's always-on-top level is `pop-up-menu` (macOS `NSPopUpMenuWindowLevel` = 101; on Windows Electron maps every level to HWND_TOPMOST).
- **CAP-007** The panel's width is always 652 px (`PANEL_W`). Its initial height is 320 px (`PANEL_H`), used until the page reports its height.
- **CAP-008** Each time the panel opens it is placed on the display nearest the mouse pointer, using that display's work area (excluding menu bar / Dock / taskbar): `x = round(workArea.x + (workArea.width − 652) / 2)`, `y = round(workArea.y + workArea.height × 0.2 − 40)`, width 652, height unchanged from the window's current height. (0.2 = `PANEL_TOP`, 40 = `CARD_TOP`, the stage's top padding, so the card's top sits a fifth of the way down the work area.)
- **CAP-009** The window's height follows the page: on `capture:resize(height)` the main side ignores the call when there is no panel, it is destroyed, or `height` is not a finite number; otherwise it clamps to `[120, 900]` (`MIN_PANEL_H`, `MAX_PANEL_H`), rounds, and, only if that differs from the current height, sets the height keeping x, y and width (the window grows/shrinks downward from a fixed top).
- **CAP-010** The page measures the height it needs as `ceil(max(stage.offsetTop + stage.offsetHeight, bottom of each open .cap-menu + 48))` (48 = `MENU_ROOM`, room for the menu's shadow) and sends it only when it differs from the last value it sent. It re-measures after every render and whenever the stage's size changes (ResizeObserver). There is no click-through anywhere in the window: the window is exactly as tall as what it shows, so clicks below it land in the app underneath.
- **CAP-011** The panel's page never navigates: a request to open an `http:`/`https:` URL in a new window opens it in the default browser, any other new-window request is denied, and every in-page navigation is cancelled.
- **CAP-012** Clicking away hides the panel: when the panel loses focus, the blur is ignored if a screenshot/file dialog is up (`busy`), if the receipt is resting (`resting`, CAP-019), or if the panel was shown less than 250 ms ago (`BLUR_GRACE_MS`). Otherwise, 50 ms later, if still not `busy`, the window still exists and it still does not have focus, the panel hides (CAP-016).
- **CAP-013** When the panel window is closed/destroyed, the reference to it is dropped (next use creates a new one).
- **CAP-014** Telling the page what the panel is up over: when the panel is shown, the stored "shown context" is replaced with the new context; for later partial updates it is merged into the stored one. If the page is still loading, nothing is sent (the page asks with `capture:ready` once loaded); otherwise a show sends `ev:capture-shown` with the full context and an update sends `ev:capture-context` with only the new fields. `capture:ready` returns the stored shown context (with any merged updates) while the panel is open (CAP-018), else `null`.
- **CAP-015** Opening the panel (`openPanel`), in order:
  1. Note whether any Nodus window currently has focus (`own`; the panel itself counts, e.g. File › Capture… while the panel is key). Only when none does, read the front app (CAP-045). When a Nodus window is focused there is no app and no browser tab.
  2. Get (or create) the panel and place it (CAP-008).
  3. Remember what it opened on: the front app's name and a fresh `targets()` snapshot (used by guesses).
  4. Tell the page it is shown with `{ app: <front app name or absent>, shortcut: <shortcut state> }` (CAP-014).
  5. Record `shownAt = now`, clear `resting`, show the window and give it focus.
  6. Then read the clipboard (CAP-052); if there is something and the panel is still open, send `{ clipboard }` as an update.
  7. Then, if the front app is a readable browser (CAP-048), read its tab (CAP-050); if there is one and the panel is still open, send `{ tab }` as an update. The clipboard read always finishes before the tab read starts.
- **CAP-016** Hiding the panel (`hidePanel`) cancels any guess in progress (CAP-074), clears `resting`, and hides the window if it exists and is visible. The window is never closed.
- **CAP-017** Toggle (used by the double tap): if the panel is open (visible) **and** focused, hide it; otherwise open it (CAP-015). A double tap while the receipt is resting (visible but not focused) therefore re-opens the panel with focus.
- **CAP-018** "Open" means visible: the panel counts as open when it exists, is not destroyed and is visible (including while resting with the receipt).
- **CAP-019** Resting: after a successful submit (CAP-131) the panel sets `resting = true`, hides, and immediately shows again **without** taking focus (Electron `showInactive`), so the keyboard goes back to the app the user was in while the receipt stays on screen. While resting, blurs never hide it (CAP-012).
- **CAP-020** Working "aside" (screenshot, file dialog): `busy = true` for the duration of the work; afterwards (success or failure) `busy = false` and, if the window still exists, it is shown and focused again.

### Shortcut (double tap, hook, permission)

- **CAP-021** Constants: a tap may be held at most 350 ms (`TAP_MS`, key-down to key-up, inclusive); the gap from the first tap's key-up to the second tap's key-down may be at most 400 ms (`GAP_MS`, inclusive). Shift keys are libuiohook key codes 42 (left Shift, `VC_SHIFT_L`) and 54 (right Shift, `VC_SHIFT_R`), on every platform; either Shift key may make either tap.
- **CAP-022** Detector state: `downAt` (time Shift went down, while down and nothing else pressed), `spoiled` (Shift is down but something else was pressed meanwhile), `lastTapAt` (when the last clean tap ended, if the next tap may complete a double tap). All start empty/false.
- **CAP-023** Key down of a non-Shift key behaves exactly as `other()` (CAP-028).
- **CAP-024** Key down of Shift while `downAt` is set or `spoiled` is true is ignored (auto-repeat while held, or the other Shift key pressed during a spoiled hold). Otherwise `downAt = at`.
- **CAP-025** Key up of a non-Shift key is ignored.
- **CAP-026** Key up of Shift: the tap is clean when `downAt` is set, not `spoiled`, and `at − downAt ≤ 350`. `downAt` and `spoiled` are then reset. If not clean, `lastTapAt` is cleared and nothing else happens.
- **CAP-027** On a clean tap: if `lastTapAt` is set and `downAt − lastTapAt ≤ 400` (measured from the previous key-up to this tap's key-down), `lastTapAt` is cleared and the double tap fires. Otherwise `lastTapAt = at` (this key-up). A third quick tap therefore starts a new pair rather than firing again.
- **CAP-028** `other()` (any non-Shift key down, or any mouse button down): if Shift is down, mark `spoiled`; clear `downAt` and `lastTapAt`. Typing a capital, ⇧-click and shortcuts containing ⇧ never count as taps, and any key or click between two taps breaks the pair.
- **CAP-029** Mouse button presses (any button) call `other()`. Mouse moves, drags, wheel events and key-ups of other keys do not affect the detector.
- **CAP-030** Times are taken from a monotonic millisecond clock (`performance.now()`) at the moment the hook event is received, never from the hook's own event timestamps (libuiohook's are raw clock ticks on macOS).
- **CAP-031** The double tap calls the toggle (CAP-017). It is the only global shortcut; there is no key-combination alternative.
- **CAP-032** Shortcut state, in priority order: setting `settings.capture` is false → `'off'`; the key hook failed to start → `'unavailable'`; macOS and the process is not trusted for Accessibility (checked without prompting) → `'needs-permission'`; otherwise `'ready'`. On Windows the trust check is always true.
- **CAP-033** `settings.capture` defaults to `true`: any value other than literal `false` in `state.json` reads as `true` (store normalization, owned by 01).
- **CAP-034** Starting the hook is a no-op if it is already running or has failed before. The hook listens to key down, key up and mouse down globally. If starting throws (native hook unavailable), the state becomes `'unavailable'` for the rest of the run (never retried until relaunch) and `[capture] the key hook failed to start` is logged with the error.
- **CAP-035** Stopping the hook removes its listeners and stops it; a failure is logged as `[capture] the key hook failed to stop` and swallowed. The hook reference is dropped either way.
- **CAP-036** 3000 ms after the hook starts, the panel window is pre-created (not shown) so the first double tap does not wait for the page to load.
- **CAP-037** `syncShortcut` (match the hook to the setting and permission): does nothing until capture is registered; clears any running permission poll; if the setting is off, stops the hook; else if trusted, starts the hook; else polls every 3000 ms and, the first time the process is trusted, stops polling and starts the hook.
- **CAP-038** `syncShortcut` runs once when capture is registered at startup and again on every settings update, whatever field changed (`applySettings` in `ipc.ts`).
- **CAP-039** On quit (`will-quit`, and the app's `shutdown()` on `before-quit`) the permission poll is cleared and the hook is stopped.
- **CAP-040** `capture:allow` (the **Allow…** buttons in the panel banner and in Settings): on macOS when not trusted: if the app is a packaged build, run `/usr/bin/tccutil reset Accessibility dev.saleem.nodus` (timeout 5000 ms; its result and errors are ignored) and wait for it to finish; then ask macOS to prompt for Accessibility trust (`AXIsProcessTrustedWithOptions` with prompt = true); open `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` in System Settings; then `syncShortcut()` (starts polling). It always resolves to the current shortcut state (CAP-032). When already trusted, or on Windows, it does nothing but return the state.
- **CAP-041** In a development (unpackaged) build, Allow… never runs `tccutil reset` (a dev build's entry is not cleared).
- **CAP-042** File › Capture… is the first item of the File menu, followed by a separator, with no accelerator, on macOS and Windows (Windows: inside the ☰ menu's File). It always **opens** the panel (CAP-015), never toggles, and works whatever the shortcut state.
- **CAP-043** `capture:show` (Settings' button, owned by 08) opens the panel exactly as File › Capture… does.
- **CAP-044** `capture:shortcut` returns the current shortcut state (CAP-032).

### Context (front app, browser tab, screenshot, clipboard, files)

- **CAP-045** Front app (macOS only; Windows returns none): run `/usr/bin/lsappinfo front` (timeout 1000 ms); its trimmed output is the ASN; if empty, none. Then run `/usr/bin/lsappinfo info -only bundleid -only name <ASN>` (timeout 1000 ms) and parse it (CAP-046). Any error or timeout → none. Needs no permission.
- **CAP-046** Parsing `lsappinfo info`: the bundle id is the first match of `/bundleID="([^"]+)"/`; without one → none. The name is the first match of `/^\s*"([^"]+)"\s+ASN:/m` (a quoted name at the start of a line followed by `ASN:`); without one, the bundle id's last dot-separated segment (`com.mitchellh.ghostty` → `ghostty`).
- **CAP-047** The front app's name is shown nowhere in the panel; it is only used in the guess prompt (`The app in front: …`) and to decide whether to read a browser tab.
- **CAP-048** Readable browsers, by bundle id. Safari-family: `com.apple.Safari`, `com.apple.SafariTechnologyPreview`. Chromium-family: `com.google.Chrome`, `com.google.Chrome.beta`, `com.google.Chrome.canary`, `org.chromium.Chromium`, `company.thebrowser.Browser` (Arc), `company.thebrowser.dia` (Dia), `com.brave.Browser`, `com.microsoft.edgemac`, `com.vivaldi.Vivaldi`, `com.operasoftware.Opera`. Anything else (Firefox included) is not a readable browser.
- **CAP-049** The tab script for a bundle id: `null` unless the id matches `/^[\w.-]+$/` (nothing odd ever reaches a script); the Safari script for Safari-family ids; the Chromium script for Chromium-family ids; otherwise `null`. Scripts verbatim in Data & formats.
- **CAP-050** Reading the tab: run `/usr/bin/osascript -e <script>` with a 60 000 ms timeout (long enough for the user to answer macOS's Automation prompt the first time, once per browser). Trim the output and split on `\n`; the first line is the URL and must match `/^https?:\/\//i`, else no tab; the title is the remaining lines joined with a single space and trimmed. Result: `{ kind: 'tab', url, title, app: <front app name> }`. Any error, refusal or timeout → no tab.
- **CAP-051** When a tab arrives on the page (`context.tab` changes to a value), it is inserted at the **front** of the attachments, unless the attachments already contain a tab (e.g. one restored from the draft, CAP-157), in which case it is not added.
- **CAP-052** Reading the clipboard to offer it (`readClipboard`): text first: plain text, trimmed, cut to 20 000 chars (`MAX_COPIED_CHARS`); if non-empty → `{ kind: 'text', text }`. Otherwise the first clipboard item that has any `image/*` type, decoded as an image; if non-empty → `{ kind: 'image', thumb }` (CAP-053). Otherwise nothing. Read errors count as nothing. Text wins over an image (a file copied in Finder also puts its icon there).
- **CAP-053** Thumbnails: an image wider than 360 px is resized to 360 px wide (aspect kept, good quality); otherwise kept as is; then encoded as a PNG `data:image/png;base64,…` URL.
- **CAP-054** Taking the clipboard (`capture:clipboard`, clicking the offer tile): text (same rules as CAP-052) → `{ kind: 'text', text }`; else an image is written as PNG to `<userData>/captures/<random UUID v4>.png` → `{ kind: 'image', path, thumb, copied: true }`; else `null`. The clipboard is read afresh at click time, so what is attached can differ from what was offered.
- **CAP-055** Screenshot, macOS: the file is `<userData>/captures/<random UUID v4>.png`. Run `/usr/sbin/screencapture -i -x -o <file>` (interactive crosshair, Space toggles window mode; `-x` no sound; `-o` no window shadow) with a 5-minute timeout; errors are ignored. If no file was written **and** Nodus's Screen Recording access is not granted (`CGPreflightScreenCaptureAccess` / Electron `getMediaAccessStatus('screen') !== 'granted'`), the result is `'needs-permission'`. The permission is read only after a shot that didn't come. The env var `NODUS_SCREENCAPTURE`, when set and non-empty, replaces the screencapture binary path (tests).
- **CAP-056** Screenshot, Windows: no interaction; capture the whole display nearest the pointer at its physical pixel size (`round(size.width × scaleFactor)` × `round(size.height × scaleFactor)`), choosing the screen source whose display id matches, else the first screen source; if the captured image is non-empty, write it as PNG to the new file.
- **CAP-057** After either: no file → `null` (the user cancelled); the file can't be decoded or is empty → `null`; else `{ kind: 'image', path, thumb }` (no `copied`).
- **CAP-058** `capture:screenshot`: works aside (CAP-020): hides the panel, waits 150 ms so it has left the screen, takes the screenshot, then the panel shows and takes focus again.
- **CAP-059** `capture:allow-screen` (**Open System Settings** in the screenshot notice): macOS opens `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture`; Windows does nothing. The panel is not hidden.
- **CAP-060** `capture:files(paths)`: when `paths` is an array, keep only strings that are absolute paths and describe them (CAP-061). Otherwise work aside (CAP-020): show the system open dialog attached to the panel with "open file" and "multiple selection" (files only, no directories); cancelled → `[]`; else describe the picked paths.
- **CAP-061** Describing files: at most the first 20 paths; each is `stat`ed; a regular file or a directory becomes `{ kind: 'file', path, name: <basename>, size: <bytes for a file, 0 for a directory> }`; anything else, or a stat error (gone), is skipped silently.
- **CAP-062** Dropping files onto the card: while something is dragged over the card it shows the dropping outline (CAP-097); leaving the card (to an element outside it) removes it; on drop the outline is removed, each dropped file's absolute path is taken (Electron `webUtils.getPathForFile`; empty paths dropped) and, if any, `capture:files(paths)` is called and the result attached.
- **CAP-063** Old captures files are deleted at startup (when capture registers): every entry of `<userData>/captures/` (created if missing) whose modification time is more than 30 days (2 592 000 000 ms, `KEEP_MS`) before now is removed. Any error stops the sweep silently (a subdirectory older than 30 days throws on removal and ends the sweep at that point; unclear whether intended, `src/main/screenContext.ts:198-208`).

### Guess (Claude's guess)

- **CAP-064** The page asks for a guess 700 ms (`GUESS_AFTER_MS`) after the last change of the text or the attachments, only when: the trimmed text is at least 3 characters (`MIN_GUESS_CHARS`), the user has not made their own choice (CAP-084), and the trimmed text differs from the text the shown guess was made for. Each change restarts the 700 ms timer.
- **CAP-065** Clearing the text (trimmed empty) resets the guess: in-flight answers are invalidated, the "guessed for" text is cleared, the pending guess is forgotten, the guess and the guessing flag are cleared, and the user's choice is cleared.
- **CAP-066** Asking: a run counter is bumped; `guessing = true`; `capture.guess(text, attachments)` is called with the untrimmed text; a rejection counts as `null`. When it settles and is still the latest run: the pending guess is cleared, `guessing = false`, and if the answer is non-null the "guessed for" text becomes the trimmed text and the guess is replaced. A `null` answer keeps the previous guess on screen. Answers from older runs are dropped entirely.
- **CAP-067** Main side of `capture:guess`: uses the targets remembered when the panel opened (refreshed by every `capture:targets` call, CAP-149), or a fresh `targets()` if the panel has not opened; the text cut to 20 000 chars; the attachments cleaned (CAP-135); the front app's name from when the panel opened.
- **CAP-068** `guessCapture`: first cancels any guess still running (CAP-074). If the trimmed text is empty or there are no projects, resolves `null` without spawning. Then finds the claude binary (domain 01 `requireClaude`); if Claude Code isn't found the call rejects (the page treats it as `null`).
- **CAP-069** The guess spawns the claude binary with the exact argv in Data & formats (`-p --model haiku …`), working directory = the OS temp dir (outside any repository), the login-shell environment (domain 01 `spawnEnv`), stdin closed (`ignore`, otherwise `claude -p` waits for piped input), stdout and stderr piped, no console window on Windows. stderr is read and discarded.
- **CAP-070** A guess that hasn't finished after 30 000 ms (`TIMEOUT_MS`) is killed with SIGKILL and resolves `null`.
- **CAP-071** A spawn error logs `[capture] guess failed` with the error and resolves `null`.
- **CAP-072** When the process closes, stdout (UTF-8, trimmed) is parsed as JSON; if `is_error` is truthy → `null`; otherwise `readGuess(structured_output, targets)` (CAP-080). Invalid JSON → `null`. Exit code is not checked.
- **CAP-073** A guess resolves exactly once (later settle calls are ignored), clears its timeout, and stops being "the running guess" if it still was.
- **CAP-074** `cancelGuess`: if a guess is running, it stops being the running one, its process is killed with SIGKILL and its promise resolves `null` immediately. Called before every new guess and whenever the panel hides.
- **CAP-075** The guess prompt (user message) is built exactly as in Data & formats: a `Projects` header naming the default project, each project with up to 8 (`SESSIONS_PER_PROJECT`) of its sessions in `targets.sessions` order (most recently active first) or `  (no sessions)`, then (if any) a blank line and the screen lines, then a blank line and the capture text (trimmed, cut to 4000 chars) between `"""` fences.
- **CAP-076** Relative time in the prompt (`ago`): `min = round(ms / 60 000)`; `min < 1` → `just now`; `min < 60` → `<min> min ago`; else `h = round(min / 60)`; `h < 48` → `<h> h ago`; else `<round(h / 24)> days ago`.
- **CAP-077** Flattening (`flat(s, max)`): every run of whitespace becomes one space, trimmed; if longer than `max`, cut to `max − 1` chars plus `…`. Session titles are flattened to 80, tab titles to 120, copied text to 200, reasons to 120.
- **CAP-078** Session status words: status `waiting` → `waiting for the user`; `working` → `working`; otherwise `idle` if its process is live, else `stopped`.
- **CAP-079** Screen lines in the prompt, in order: `The app in front: <app>` (only when known), then one line per attachment in attachment order: tab → `Its browser tab: <flat(title,120)> (<url>)`; image → `An image the user copied is attached.` when `copied`, else `A screenshot is attached.`; file → `A file is attached: <name>`; text → `Copied text is attached: <flat(text,200)>`. Paths and image data are never sent in the guess.
- **CAP-080** Reading Claude's answer (`readGuess`): a non-object → `null`. Project: `project` if it is one of the targets' project ids, else the default project id; if that is also empty → `null`. Kind: `kind` if it is one of `session`/`into`/`note`/`view`, else `session`. For `into`: the session must exist in targets and not be `waiting`; otherwise the kind becomes `session` with no session. Reason: `flat(reason, 120)` when a string, else `''`. When a session was taken, the project is that session's own project, whatever project Claude named.

### Panel UI (page state, header, field, attachments, menu, banners, keys)

- **CAP-081** Page start: the panel page is the renderer at `#capture` (in Rust: the capture panel view). It marks the document as the capture window (transparent page background, see UI details › Stage), subscribes to `ev:capture-shown` and `ev:capture-context`, then calls `capture:ready`; a non-null answer is handled exactly as a "shown" event (the panel came up while the page was loading).
- **CAP-082** Initial page state: context `{ shortcut: 'ready' }`; text = saved draft (CAP-157) or `''`; attachments = saved draft attachments or `[]`; no guess, not guessing, no choice, menu closed, not sending, no error, screenshot not blocked, no receipt, not dragging; "offer done" key = saved value or none; history = saved list or `[]`; history position −1.
- **CAP-083** On "shown" (each time the panel comes up): cancel the receipt timer and clear the receipt, close the menu, clear the error, clear the screenshot-blocked notice, **replace** the context with the new one (a previous tab/clipboard offer is dropped), re-read targets, and focus the field with the caret at the end of the text, now and again on the next frame. The draft text, attachments, guess and choice are **kept** across shows. On a context update, the new fields are merged into the context.
- **CAP-084** The route (where it goes) is: the user's **choice** if any; else Claude's **guess** (its kind, project and session) if any; else the **fallback**: kind `session`, project = targets' default project, else the first project, else none. The user's choice stands over the guess until the text is cleared (CAP-065) or a capture is sent.
- **CAP-085** "No projects" is true once targets have loaded and there are zero (non-archived) projects. "Has text" is true when the trimmed text is non-empty.
- **CAP-086** Header button content (left of the header):
  - No text, or targets not loaded yet: folder swatch in the route project's colour (blue when none) + the project's name, or `No projects yet` when there is no project + chevron-down.
  - Has text, a guess is in flight, and there is neither a guess nor a choice yet: a spinning loader + `Working out where this goes…` (the whole line pulses).
  - Otherwise: a kind badge with the kind's icon, then for `into`: bold `Into “<session title>”` (title of the route's session from targets, or `a session` if not found), ellipsized; for other kinds: bold `New session` / `Note` / `View`, then faint `on` (note) or `in` (session, view), the project's folder swatch and the project's name; then the chevron-down.
- **CAP-087** The header button is disabled until targets have loaded and while there are no projects. Clicking it toggles the menu and returns focus to the field.
- **CAP-088** Header right side: when there is text and (a guess or a choice): `Your choice` if the user chose, else a small accent dot (pulsing while a guess is in flight) + `Claude's guess`; in both cases followed by a `Tab` key cap. When there is no text: `Claude works out what it is as you type`. When there is text but neither a guess nor a choice: nothing.
- **CAP-089** The field: a multi-line text area labelled `What to capture`, 3 rows by default, autofocused, spell-checked; placeholder `Add a project in Nodus to capture into` when there are no projects, else `Ask Claude to do something, or jot a note`. Typing sets the text, resets the history position to −1 and clears the error.
- **CAP-090** The field grows with its content: height = min(content height, line height × 8 + 24) where the line height is 22 px (so at most 200 px), never below its 90 px min-height; beyond that it scrolls. Recomputed whenever the text or the receipt changes.
- **CAP-091** The attachments row shows when there is at least one attachment or a clipboard offer: one tile per attachment in order, then the offer tile last.
- **CAP-092** Tab tile: globe icon + the URL's host without a leading `www.` (the whole URL if it doesn't parse) on top, ellipsized; the title (or the URL if the title is empty) below, at most 2 lines; tooltip = the URL.
- **CAP-093** Image tile: the thumbnail (if non-empty) covering the tile, anchored top-left; a label `Screenshot`, or `Copied image` when copied, bottom-left; tooltip = the file path. Images restored from a draft keep their thumbnails (thumbnails are part of the saved draft). After cleaning on the main side thumbnails are dropped (`thumb: ''`), which never reaches the page.
- **CAP-094** File tile: file-text icon (18 px, faint), then the name (ellipsized) and, when size > 0, the size: `< 1024` → `<n> B`; `< 1 048 576` → `<round(n/1024)> KB`; else `<(n/1048576) to 1 decimal> MB`; tooltip = the path. Directories show no size.
- **CAP-095** Copied-text tile: `Copied text` on top, the text in monospace below, at most 2 lines.
- **CAP-096** Each attachment tile has a remove button (×, label and tooltip `Remove`) in its top-right corner, invisible until the tile is hovered or the button has keyboard focus. Removing drops that tile and returns focus to the field.
- **CAP-097** While files are dragged over the card, the card shows a 2 px accent outline around its shadow.
- **CAP-098** Clipboard offer: shown when the context has a clipboard value whose key differs from the saved "offer done" key. Key: text → `t:` + its first 2000 chars; image → `i:` + the last 120 chars of its thumbnail data URL. The offer tile is a dashed button: plus icon + `Copied text` / `Copied image` on top; below, the text (monospace, 2 lines) or a 34 px tall thumbnail. Clicking it: takes the clipboard (CAP-054), marks the **offered** key done (saved), attaches what was taken if anything, and focuses the field.
- **CAP-099** The paperclip opens the file dialog (CAP-060) and attaches what was picked; focus returns to the field.
- **CAP-100** The scan button takes a screenshot (CAP-058): `'needs-permission'` shows the screenshot notice (CAP-101); an image is attached; `null` (cancelled) attaches nothing. The notice is shown exactly when the result is `'needs-permission'` and hidden for any other result; the field is focused in every case.
- **CAP-101** Screenshot notice (role alert, inside the card under the tiles): alert icon + `macOS kept the screenshot from Nodus. Turn Nodus on in Screen & System Audio Recording, then take it again.` + button `Open System Settings` (CAP-059). Cleared on the next show or a successful/cancelled screenshot.
- **CAP-102** Error line (role alert): shows the last error message (CAP-124) in red under the tiles; cleared by typing, by picking a kind, by starting a send, and on the next show.
- **CAP-103** Voice: the mic button records with the voice hook owned by 05 (the transcript's project is the route's project, or `''`). A transcript is appended to the text with one space after the trimmed-end text (or replaces an all-whitespace text) and the field is focused. The mic button: `Speak` (title and label), mic icon 16; while recording `Stop and transcribe`, filled square icon 13, red background; disabled while the mic is opening or transcribing. While voice is not idle the footer hint is replaced by the voice status (clock and meter while recording, `Transcribing…` while transcribing, nothing while opening). Voice notices (owned by 05) show inside the card under the error/notice line. A voice notice's **Settings** button, and a missing voice key, call `capture:settings` (CAP-159) instead of opening a dialog in the panel.
- **CAP-104** Esc while the voice hook is opening, recording or transcribing cancels the voice (05) and does not reach the field: the panel stays up and the menu stays as it is.
- **CAP-105** Footer hint (when voice is idle): key cap `⇧↩` (Windows: `Shift+Enter`) + ` new line`.
- **CAP-106** Send button: label is the route kind's verb (`Start session`, `Send`, `Put up note`, `Build view`), followed by a key cap `↩` (Windows: `Enter`). While sending, a spinning loader precedes the label, and the label is `Working it out…` when there is no choice and the shown guess was not made for the current trimmed text. Disabled when there is no text, while sending, or when there are no projects.
- **CAP-107** The menu (role dialog, `Where it goes`) opens under the header when the header is clicked and targets are loaded. Any pointer-down outside the menu and outside the header button closes it (the click still goes on to its target). Esc in the field closes it (CAP-121). Picking a project or a session closes it; picking a kind does not.
- **CAP-108** Menu reason: when there is no choice and the guess's reason is non-empty, the menu starts with an accent dot + bold `Claude's guess:` + ` ` + the reason.
- **CAP-109** Menu left column `Make` (listbox `What to make`): one option per kind in order session, into, note, view: icon (16), label (`New session`, `Send into a session`, `Note`, `View`) and hint (`Claude starts on it, in auto mode`, `Adds to its conversation`, `A sticky on the board, nothing runs`, `A live page Claude keeps up to date`). When no session anywhere is sendable (status ≠ `waiting`), the into option is disabled and its hint is `No sessions are running`. The route's kind is selected. Clicking an option picks that kind (CAP-113) and returns focus to the field.
- **CAP-110** Menu right column, for kinds other than into (listbox `Which project`): label `Project`, then every project (targets order): folder swatch, name (ellipsized), a check mark on the route's project (selected). Clicking picks the project (CAP-115).
- **CAP-111** Menu right column, for into (listbox `Which session`): label = folder swatch + the route project's name; then up to the first 8 sessions of that project in targets order (most recently active first), **waiting ones included**: status dot, title (ellipsized), and for a waiting session `Needs you` (red), the option disabled with tooltip `It’s waiting for your answer. Answer it on the board first.`; the selected session shows a check mark. If the project has none: `No sessions on this board`. Then a separator, then every **other** project: folder swatch, name, and `<n> sessions` where n counts all of its sessions (waiting included) or the word `no` when zero (so `1 sessions`, `no sessions`, verbatim). Clicking a session picks it (CAP-116); clicking a project picks it (CAP-115).
- **CAP-112** Menu footer: `Tab` key cap + ` next kind`, `Esc` key cap + ` close` (literal `Tab`/`Esc` on every platform).
- **CAP-113** Picking a kind: for into: take the route's project and the route's session if it is still a sendable session of that project, else that project's first sendable session; if the project has none, take the first sendable session anywhere and its project; if there is none at all, nothing happens. For other kinds: choice = that kind with the route's project. The error is cleared (except when nothing happened).
- **CAP-114** Tab / ⇧Tab cycle kinds: the order is session, into, note, view, skipping into when no session anywhere is sendable; step +1 (Tab) or −1 (⇧Tab) from the route's kind, wrapping; then pick that kind (CAP-113). If the route's kind is into but into is skipped, Tab goes to the first kind (session) and ⇧Tab to the second-to-last (note). Only with text; without text Tab does nothing (focus does not move).
- **CAP-115** Picking a project: if the route is into, choose that project's first sendable session; with none, the choice becomes a new session in that project. Otherwise keep the kind and change the project. Closes the menu, focuses the field.
- **CAP-116** Picking a session: choice = into that session, with the session's project. Closes the menu, focuses the field.
- **CAP-117** Banner under the card (only with no text and shortcut state `needs-permission` or `unavailable`): for `needs-permission`: `To open this from any app with a double tap of ⇧, allow Nodus in Accessibility. Already on there? Click Allow… anyway.` + button `Allow…` (CAP-040; the returned state replaces the context's shortcut, hiding the banner once `ready`); for `unavailable`: `The double tap of ⇧ couldn’t start on this computer. Use File › Capture… instead.`. Not shown for `ready` or `off`.
- **CAP-118** The page re-reads targets every 2000 ms (`POLL_MS`) while it is visible, so recents' and sessions' statuses stay live.

### Panel UI: field keys

- **CAP-119** While an IME composition is in progress, the field's key handling does nothing.
- **CAP-120** Return (without ⇧) sends (CAP-123); ⇧Return inserts a new line.
- **CAP-121** Esc: closes the menu if open; otherwise hides the panel (`capture:hide`). The draft stays.
- **CAP-122** ↑ when (the text is empty or the history position ≥ 0) and history is non-empty: position = min(position + 1, history length − 1); text = that history entry (newest first). ↓ when the position ≥ 0: position − 1; text = that entry, or empty at −1. Otherwise ↑/↓ move the caret normally. Typing resets the position to −1. History recall does not itself reset the position.

### Submit

- **CAP-123** Sending from the page (Return or the send button): nothing happens when the trimmed text is empty or a send is in progress. Otherwise: mark sending, clear the error, close the menu; then:
  1. Destination = the route (CAP-084). If there is no choice and the shown guess was not made for the current trimmed text, wait for the guess in flight, or ask for one now (CAP-066) and wait; a non-null answer becomes the destination; a `null` answer leaves the route as it was (the previous guess or the fallback).
  2. No project → error `Add a project in Nodus first`.
  3. Kind into without a session → error `There’s no session to send into. Pick another kind.`
  4. `capture:submit({ text: <trimmed text>, kind, projectId, sessionId: <only for into>, attachments })`.
  5. On success: put the trimmed text at the front of the history, removing any earlier identical entry, keep at most 50 (`MAX_HISTORY`), and save it; if the context had a clipboard offer, mark its key done (taken or not) and save it; clear the text, the attachments and the choice; reset the history position to −1; show the receipt (CAP-140) with the receipt and the trimmed text; re-read targets; start the 2600 ms receipt timer (CAP-147).
  6. Finally (success or error) clear sending.
- **CAP-124** A failed send shows the error's message in the error line; the Electron IPC prefix `Error invoking remote method '<channel>': ` and an optional following `Error: ` are stripped (Rust: show the plain message). The text, attachments and choice are kept.
- **CAP-125** Main side of `capture:submit`, validation in order: text = the submitted text if a string, cut to 20 000 chars, then trimmed; empty → error `Type something first`. Kind not one of `session`/`into`/`note`/`view` → error `Pick what to make`. Project unknown → error `Unknown project <id>` (store, 01). The board is fetched (created empty if missing). Attachments are cleaned (CAP-135). The prompt = `withScreen(text, attachments)` (CAP-138).
- **CAP-126** Session: position = `spotOnBoard(board, 720 × 480, window size)` (expanded card size, CAP-133). Create a session through the board's own create path (domain 03 `createSession`) with `{ x, y, initialPrompt: <prompt>, expanded: true, autoMode: true }` (Claude starts on the prompt in auto mode, as an expanded card). Then add a board note `{ id: <UUID v4>, text: <the typed text only, without the screen block>, color: 'yellow', x, y: <same position as the card>, sessionId: <new session id>, createdAt: <now> }` (no `w`/`h`; it shows stuck on the card, as the prompt bar's does, owned by 10), and save. Receipt title = `titleFrom(text)` (CAP-136), with `sessionId`.
- **CAP-127** Into: session id = the submitted `sessionId` if a string, cut to 100 chars. It must be non-empty and pass the session check (CAP-148: on the project's board, not archived, not a terminal, not an action run, not a view's brain), else error `That session is no longer on the board`. Then send the prompt through the board's send path (domain 03 `sendToSession`): a live session that is `waiting` rejects with `It’s waiting for your answer. Open it to reply.`; a live one gets the prompt typed and submitted; a stopped one is resumed with the prompt as its first prompt. Receipt title = the session's title as its card shows it (03 `sessionTitle`: node title, else transcript title, else `New session`).
- **CAP-128** Note: size = `fitNote(prompt)` (owned by 04/10: width from the longest line, clamped; height from wrapped rows, clamped); note `{ id: <UUID v4>, text: <prompt, including the screen block>, color: 'yellow', x, y: spotOnBoard(board, size, window size), w, h, createdAt: <now> }`, saved. Receipt title = `titleFrom(text)`, with `noteId`.
- **CAP-129** View: position = `spotOnBoard(board, 480 × 360, window size)` (`VIEW_W` × `VIEW_H`); create a view with `description = prompt` at that position (domain 04 `createView`: title `New view`, its brain is started with `Build this view:\n\n<description>`). Receipt title = `titleFrom(text)`, with `viewId`.
- **CAP-130** After any kind succeeds: remember it in recents (CAP-155), emit board-changed for the project (`ev:board-changed`, so an open board reloads), and return the receipt `{ kind, projectId, projectName, title, sessionId?, noteId?, viewId? }`.
- **CAP-131** After `capture:submit` resolves, the panel rests (CAP-019) before the page gets the receipt. A failed submit doesn't rest: the panel keeps focus.
- **CAP-132** A submit that fails part-way leaves nothing behind for that kind (the session/view creators undo their own board entry on spawn failure, 03/04); nothing is remembered.
- **CAP-133** Where it goes on the board (`spotOnBoard(board, size, windowSize)`): if the board has a saved viewport with zoom > 0, the preferred top-left is the centre of the board as the window last showed it: `x = (windowW / 2 − vp.x) / vp.zoom − size.w / 2`, `y = (windowH / 2 − vp.y) / vp.zoom − size.h / 2`. Without a usable viewport: right of the right-most card (cards = nodes not archived, not runs, not view brains; the one with the largest x; on a tie the earlier one in the board's node order): `{ x: card.x + cardSize(card).w + 32, y: card.y }`; with no cards: `{ x: dock.x + dock.w + 80, y: dock.y }` where dock = the archive dock rect (09). Then the nearest free spot of that size avoiding the board's obstacles (cards, free notes, views; 04 `obstacles`) and the dock (09 `findFreeSpot`), rounded to integers.
- **CAP-134** Window size for placement: the board window's content size if it is open, else 1440 × 920.
- **CAP-135** Cleaning attachments from the page: not an array → none. At most the first 20 entries; non-objects skipped. Any non-string field reads as `''`. Tab: kept when `url` (cut to 4000) matches `/^https?:\/\//i` → `{ kind: 'tab', url: <cut 4000>, title: <cut 500>, app: <cut 100> }`. Image: kept when `path` (cut 4000) is non-empty → `{ kind: 'image', path, thumb: '', copied: <true only if copied === true> }`. File: kept when `path` non-empty → `{ kind: 'file', path: <cut 4000>, name: <cut 500>, size: <Number(size), or 0 if NaN/0> }`. Text: kept when the text (cut 20 000) is non-blank → `{ kind: 'text', text: <cut 20 000, untrimmed> }`. Anything else is dropped. Paths are not checked against the filesystem.
- **CAP-136** `titleFrom(text, max = 48)`: take the first line of the trimmed text; take its first sentence (shortest prefix ending in `.`, `!` or `?` followed by whitespace or the end), or the whole line; strip trailing `.!?` and trim. If ≤ 48 chars, that's the title. Else cut to 47 chars; if the last space in the cut is past position 24 (`> max / 2`), cut there; trim the end and add `…`.
- **CAP-137** `describeAttachments(attachments)`: lines in attachment order: tab → `- The <app> tab I had open: <title> (<url>)`, or `- The <app> tab I had open: <url>` when the title is empty; image → `- An image I copied: <path>` when copied, else `- A screenshot: <path>`; file → `- A file: <path>`. Copied text (trimmed, cut to 20 000; blank skipped) is collected separately. With no lines and no copied text → `''`. Otherwise the parts `From my screen as I wrote this:`, then the lines joined by `\n` (if any), then for each copied text `Text I copied:\n<fence>`, all joined by a blank line (`\n\n`).
- **CAP-138** `withScreen(text, attachments)`: `<trimmed text>\n\n<describeAttachments>` when that is non-empty, else the trimmed text.
- **CAP-139** Fencing copied text: the fence is N backticks, N = max(2, longest run of backticks in the text) + 1 (so at least 3); the block is fence, `\n`, the text, `\n`, fence.

### Receipt

- **CAP-140** While a receipt is shown, the page shows only the receipt (no card, no banner, no recents); the window shrinks to its height (CAP-010).
- **CAP-141** Note receipt: a sticky-note square (yellow paper, tilted −1.5°) showing the trimmed text clipped to 160 chars (`clip`: longer than max → first max−1 chars, end-trimmed, + `…`), whitespace and line breaks preserved; under it a pill `On <projectName>’s board · ` + link `Open`.
- **CAP-142** Session / into / view receipt card (288 px wide): top line: for a view a spinning loader + `Building view` (purple view ink); otherwise an orange working dot + `Working` (session) or `Sent` (into) in bold orange; then the receipt's title (bold); then a faint line: session `Claude is starting on it, expanded on the board.`, into `Your message is in. Claude is on it.`, view `Its brain is putting it together.`; then a footer, separated by a line: link `Open on board` on the left; on the right the project's folder swatch (colour from targets; blue if unknown), the project name and, for a session only, faint `· auto mode`.
- **CAP-143** A session receipt also wears a sticker: a small yellow note at its top-right corner (tilted 4°, overhanging the card) with the trimmed text clipped to 70 chars; the title leaves 56 px on its right for it.
- **CAP-144** Clicking `Open` / `Open on board`: cancel the receipt timer, clear the receipt, and call `capture:open({ projectId, sessionId })` with the receipt's session id (none for note and view, which opens the project's board).
- **CAP-145** `capture:open(request)`: hides the panel; if `request.projectId` is a string, brings the board window up on that card or board (domain 02/08 `openCard`: an unknown project only shows the window; a closed window is re-created and the request is picked up once it loads).
- **CAP-146** Receipt entrance: the receipt (card or note) scales in from 1.5× and 18 px lower at 40 % opacity over 280 ms with the spring easing, as if the panel shrank into it.
- **CAP-147** 2600 ms (`RECEIPT_MS`) after it appears, the receipt is cleared and the panel hides (`capture:hide`). The panel coming up again in the meantime cancels the timer and clears the receipt.

### Recents

- **CAP-148** Session check (`sessionInfo(projectId, sessionId)`): none unless the node exists on that project's board and is not archived, not a terminal, not an action run (`runOf`), not a view's brain (`viewOf`). Otherwise `{ projectId, sessionId, title, status, live, updatedAt }` with title = node title, else the transcript's title, else the title of the newest `session` capture record for that session id, else `Untitled session`; status = the status machine's current status (02); live = whether its process runs; updatedAt = the transcript's last activity, else the node's `createdAt`, else 0.
- **CAP-149** `capture:targets` → `{ projects, sessions, recents, defaultProjectId }`: projects = non-archived projects in `state.json` order, as `{ id, name, color }`; sessions = for each of those projects, each board node passing the session check, all sorted by `updatedAt` descending (stable); recents = the capture records (newest first) that still resolve (CAP-150); defaultProjectId (CAP-151). A project without a board gets an empty one in memory (store `getBoard`, saved with the next save). It also replaces the targets remembered for guesses (CAP-067).
- **CAP-150** A capture record resolves to a recent only while its project exists in `state.json` (archived included) and has a board: session/into → only while its session passes the session check, with the session's current title and status; note → only while the note is on the board; view → only while the view is on the board, titled with the view's title unless that is still `New view`, in which case the record's title. Others are skipped (not deleted).
- **CAP-151** Default project: among non-archived projects, take the newest capture record whose project is one of them, and the project opened most recently (`openedAt`, missing = 0; ties keep `state.json` order; with no `openedAt` anywhere, the first project). If there is such a capture and (that most recently opened project has no `openedAt`, i.e. no project has one, or the capture's `createdAt` is later than its `openedAt`) → the capture's project. Else, if a board was opened in the board window this run (CAP-154) and its project is non-archived → that project. Else the most recently opened project, else none.
- **CAP-152** Recents on the page: only with no text and at least one recent: a 3-column row of the first 3 recents, each a button with tooltip `Open it on the board`: top line: a mark (session/into: a status dot, `done` when no status; note: a yellow square; view: a purple square), the project's name (from targets; blank if the project is archived), and the age (`min = round(Δms / 60 000)`; `min < 1` → `now`; `min < 60` → `<min>m`; `h = round(min / 60)`; `h < 24` → `<h>h`; else `<round(h / 24)>d`); the title (bold, 2 lines max); the status at the bottom: note → `Note`, view → `View`, waiting → `Needs you` (red), working → `Working` (orange), else `Done` (faint). A waiting session's recent has a red 2 px ring. Clicking opens it (CAP-144 path: clears receipt timer, `capture:open({ projectId, sessionId })`; notes and views open the board).
- **CAP-153** With more than 3 recents, a pill under them: `<n − 3> more · <titles of the 4th and 5th recents joined by ", ">` + faint ` · ↑ for earlier captures`.
- **CAP-154** Every `board:load` in the board window records that project as "the board opened last" (in memory only, not persisted).

### Storage

- **CAP-155** Remembering a capture: a record `{ id: <UUID v4>, kind, projectId, title: <receipt title>, createdAt: <now ms>, sessionId?, noteId?, viewId? }` is put at the front of `state.json` `captures`, which is cut to 20 (`MAX_RECORDS`), and the state is saved (debounced save, 01).
- **CAP-156** `captures` in `state.json` is read only when it is an array (otherwise absent) and written back as is; records that no longer resolve stay in the file until pushed out by newer ones.
- **CAP-157** The page keeps, per machine, across panel shows and app restarts: `capture.draft` (the text; removed when empty), `capture.attachments` (the attachment list including image thumbnails; removed when empty), `capture.history` (up to 50 sent texts, newest first), `capture.clipboardDone` (the key of the last clipboard offer seen). They are saved on every change; unreadable or missing values fall back to defaults; write failures are ignored. In 0.9.4 they live in the renderer's `localStorage` as JSON values.
- **CAP-158** Screenshots and copied images are PNGs named `<UUID v4>.png` in `<userData>/captures/` (`<userData>` honours `NODUS_USER_DATA`), created on demand. Claude gets them by path (CAP-137). They are never deleted on send or removal from the panel; only by the 30-day sweep (CAP-063).
- **CAP-159** `capture:settings` (voice notice's Settings, missing voice key): hides the panel, brings the board window up (re-creating it if closed) and opens its Settings dialog at Capture & Voice, where the voice key goes (menu command `voice-settings`, sent once the window has loaded if it was re-created).
- **CAP-160** `capture:hide` (fire-and-forget) hides the panel (CAP-016).

## Data & formats

### The guess call (captureGuess.ts)

argv, one element per line (the first element is the resolved claude binary path, domain 01):

```
<claude>
-p
--model
haiku
--settings
{"alwaysThinkingEnabled":false}
--tools

--strict-mcp-config
--no-session-persistence
--json-schema
{"type":"object","properties":{"kind":{"type":"string","enum":["session","into","note","view"]},"project":{"type":"string"},"session":{"type":"string"},"reason":{"type":"string"}},"required":["kind","project","session","reason"],"additionalProperties":false}
--output-format
json
--system-prompt
<SYSTEM_PROMPT, below>
<user prompt, below>
```

- The element after `--tools` is the empty string `""` (no tools).
- `--settings` is exactly `{"alwaysThinkingEnabled":false}` (thinking off).
- `--json-schema` is exactly the single-line JSON above (`JSON.stringify`, no spaces). Pretty form:

```json
{
  "type": "object",
  "properties": {
    "kind": {
      "type": "string",
      "enum": [
        "session",
        "into",
        "note",
        "view"
      ]
    },
    "project": {
      "type": "string"
    },
    "session": {
      "type": "string"
    },
    "reason": {
      "type": "string"
    }
  },
  "required": [
    "kind",
    "project",
    "session",
    "reason"
  ],
  "additionalProperties": false
}
```

- Process: cwd = OS temp dir (macOS `$TMPDIR`, Windows `%TEMP%`), env = login-shell env (01 `spawnEnv`), stdin ignored, no console window. Timeout 30 000 ms → SIGKILL. Output read: stdout parsed as one JSON object; `is_error` (bool) and `structured_output` (the schema's object) are the only fields used.

SYSTEM_PROMPT, verbatim (the text between the fences, curly quotes and `…` included, no trailing newline):

````text
You route quick captures on Nodus, a board of Claude Code sessions for each of the user's projects. In another app, the user opened a small panel and typed something. Decide what it becomes and which project it goes to:
- session: a new Claude Code session in the project's repository that does what the text asks. The default for work to be done.
- into: the text is sent to a session that already exists, as the user's next message to it. Only when the text clearly carries on that session's work: it follows up on it ("also…", "and add tests for it"), replies to it, or names it. Never a session that is waiting for the user.
- note: a sticky note on the project's board, and nothing runs. For ideas, reminders and things to do later or to ask someone, not work to start now ("remember to…", "idea:", "todo:", "ask finance about…").
- view: a live page on the board that Claude builds and keeps up to date. Only when the text asks for something to watch or show: a dashboard, a table, a chart, a list or the status of something.

Take the project the text names (a project, a repository, a file or one of its sessions), else the one the browser tab or the app in front points to, else the default project.

Answer with kind, project (an id from the list), session (an id from the list, only for into; otherwise ""), and reason: one short sentence for the user, under 80 characters, saying why. For example: "It follows on from “Fix checkout total rounding”." or "It's an idea to keep, not work to start now."
````

User prompt (the last argv element), built line by line and joined with `\n`:

```
Projects (default: <default name>, <default id>):        ← "Projects:" when the default id matches no project
- project <id>: <name>
  - session <sessionId>: “<flat(title, 80)>”, <status word>, active <ago>
  (no sessions)                                            ← instead of session lines when the project has none
<empty line, then one screen line each>                    ← only when there are screen lines
<empty line>
The capture:
"""
<text trimmed, cut to 4000 chars>
"""
```

Exactly: `["Projects…:", projectsBlock, screen.length ? "\n" + screen.join("\n") : "", "\nThe capture:\n\"\"\"\n" + text + "\n\"\"\""].join("\n")`. With no screen lines that third element is `""`, so the project block is followed by `\n\n\nThe capture:`: **two** empty lines before `The capture:` (with screen lines: one empty line before them and one after). Each project's block is its header line followed by up to 8 session lines (or `  (no sessions)`), joined by `\n`; projects are joined by `\n`. Session lines start with two spaces; `(no sessions)` with two spaces.

Example (made-up data, `now` fixed):

```
Projects (default: storefront, p-store):
- project p-store: storefront
  - session s-round: “Fix checkout total rounding”, idle, active 25 min ago
- project p-api: api
  - session s-uuid: “Migrate orders to UUIDs”, waiting for the user, active 2 min ago
  - session s-stripe: “Bump Stripe SDK”, working, active just now

The app in front: Safari
Its browser tab: Refund fails #482 (https://example.com/acme/storefront/issues/482)
A file is attached: checkout.log

The capture:
"""
also cover the coupon path
"""
```

### What a capture sends to Claude (shared/capture.ts)

`withScreen("Fix it", [tab, screenshot, copied image, file, text])` with made-up values:

````
Fix it

From my screen as I wrote this:

- The Safari tab I had open: Refund fails #482 (https://example.com/acme/storefront/issues/482)
- A screenshot: <userData>/captures/1b9d6bcd-….png
- An image I copied: <userData>/captures/6ec0bd7f-….png
- A file: ~/logs/checkout.log

Text I copied:
```
TypeError: amount.minus
```
````

Several copied texts each get their own `Text I copied:` part, separated by blank lines. This is the session's initial prompt, the into prompt, the note's text (kind note) and the view's description. The session kind's sticky note gets only the typed text.

### Front app and browser tab (screenContext.ts)

- `/usr/bin/lsappinfo front` → e.g. `ASN:0x0-0x1a01a:` (trimmed).
- `/usr/bin/lsappinfo info -only bundleid -only name <ASN>` → e.g.
  ```
  "Safari" ASN:0x0-0x1a01a: (in front)
      bundleID="com.apple.Safari"
  ```
- AppleScript for Safari-family ids (`<bundleId>` substituted verbatim):
  ```applescript
  tell application id "<bundleId>" to tell front document to return (URL as text) & linefeed & (name as text)
  ```
- AppleScript for Chromium-family ids:
  ```applescript
  tell application id "<bundleId>" to tell active tab of front window to return (URL as text) & linefeed & (title as text)
  ```
- Run as `/usr/bin/osascript -e <script>` (one argument), timeout 60 000 ms. Output: URL line, then title line(s).

### Other commands and URLs

| What | Exact |
| --- | --- |
| Screenshot (macOS) | `/usr/sbin/screencapture -i -x -o <userData>/captures/<uuid>.png`, timeout 300 000 ms; binary overridable by env `NODUS_SCREENCAPTURE` |
| Reset a stale Accessibility entry (packaged macOS only) | `/usr/bin/tccutil reset Accessibility dev.saleem.nodus`, timeout 5000 ms |
| Accessibility pane | `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` |
| Screen Recording pane | `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture` |

### state.json

```jsonc
{
  "settings": { "capture": true },            // any value but false reads as true
  "captures": [                                // newest first, at most 20; absent until the first capture
    {
      "id": "<uuid v4>",
      "kind": "session" | "into" | "note" | "view",
      "projectId": "<project id>",
      "title": "<titleFrom(text), or the session's title for into>",
      "createdAt": 1800000000000,             // ms since epoch
      "sessionId": "<uuid>",                  // session, into
      "noteId": "<uuid>",                     // note
      "viewId": "<uuid>"                      // view
    }
  ],
  "boards": { "<projectId>": { "notes": { "<noteId>": { "id": "…", "text": "…", "color": "yellow", "x": 0, "y": 0, "w": 0, "h": 0, "sessionId": "…", "createdAt": 0 } } } }
}
```

Absent optional fields are omitted (not `null`). A session capture's note has `sessionId` and no `w`/`h`; a note capture's note has `w`/`h` and no `sessionId`.

### Page-local values (0.9.4: renderer `localStorage`, JSON-encoded)

| Key | Value | Default |
| --- | --- | --- |
| `capture.draft` | string (removed when empty) | `""` |
| `capture.attachments` | `CaptureAttachment[]`, thumbnails as data URLs (removed when empty) | `[]` |
| `capture.history` | `string[]`, newest first, ≤ 50, unique | `[]` |
| `capture.clipboardDone` | `"t:<first 2000 chars>"` or `"i:<last 120 chars of thumb>"` | `null` |

### Files on disk

- `<userData>/captures/<uuid v4>.png`: screenshots and copied images; swept after 30 days by mtime at launch.
- Nothing else is written by capture (the guess runs with `--no-session-persistence` in the temp dir).

### Types (shared/types.ts, serde-compatible)

```ts
type CaptureKind = 'session' | 'into' | 'note' | 'view'           // Tab order
type CaptureAttachment =
  | { kind: 'tab'; url: string; title: string; app: string }
  | { kind: 'image'; path: string; thumb: string; copied?: boolean }
  | { kind: 'file'; path: string; name: string; size: number }
  | { kind: 'text'; text: string }
interface CaptureSession { projectId; sessionId; title; status: SessionStatus; live: boolean; updatedAt: number }
interface CaptureRecent { id; kind; projectId; title; createdAt; sessionId?; noteId?; viewId?; status?: SessionStatus }
interface CaptureTargets { projects: { id; name; color: FolderColor }[]; sessions: CaptureSession[]; recents: CaptureRecent[]; defaultProjectId: string | null }
type CaptureShortcut = 'ready' | 'needs-permission' | 'off' | 'unavailable'
interface CaptureContext { app?: string; tab?: <tab attachment>; clipboard?: { kind: 'text'; text } | { kind: 'image'; thumb }; shortcut: CaptureShortcut }
interface CaptureGuess { kind; projectId; sessionId?; reason: string }
interface CaptureSubmission { text; kind; projectId; sessionId?; attachments: CaptureAttachment[] }
interface CaptureReceipt { kind; projectId; projectName; title; sessionId?; noteId?; viewId? }
```

Constants: `CAPTURE_KINDS = ['session','into','note','view']`, `MAX_COPIED_CHARS = 20000`, `TAP_MS = 350`, `GAP_MS = 400`, `SHIFT_KEYS = [42, 54]`, panel `652 × 320` (height 120–900), `CARD_TOP = 40`, `PANEL_TOP = 0.2`, `MAX_RECORDS = 20`, main `MAX_TEXT_CHARS = 20000`, `BLUR_GRACE_MS = 250`, blur re-check 50 ms, hook pre-create 3000 ms, permission poll 3000 ms, screenshot settle 150 ms, guess `TIMEOUT_MS = 30000`, guess `MAX_TEXT_CHARS = 4000`, `SESSIONS_PER_PROJECT = 8`, `MAX_REASON_CHARS = 120`, page `GUESS_AFTER_MS = 700`, `MIN_GUESS_CHARS = 3`, `RECEIPT_MS = 2600`, `POLL_MS = 2000`, `MAX_HISTORY = 50`, `MAX_ROWS = 8`, `MENU_ROOM = 48`, `KEEP_MS = 30 days`, thumbnail width 360.

## API surface

All channels are `nodus-core` commands/events in Rust (names kept for the mapping table in 01).

| Channel | Kind | Args → result | Behavior |
| --- | --- | --- | --- |
| `capture:ready` | invoke | `() → CaptureContext \| null` | CAP-014 |
| `capture:targets` | invoke | `() → CaptureTargets` | CAP-149 |
| `capture:guess` | invoke | `(text, attachments) → CaptureGuess \| null` | CAP-067 |
| `capture:submit` | invoke | `(CaptureSubmission) → CaptureReceipt` (rejects with message) | CAP-125…CAP-131 |
| `capture:screenshot` | invoke | `() → image attachment \| 'needs-permission' \| null` | CAP-058 |
| `capture:allow-screen` | invoke | `() → void` | CAP-059 |
| `capture:files` | invoke | `(paths?: string[]) → CaptureAttachment[]` | CAP-060 |
| `capture:clipboard` | invoke | `() → CaptureAttachment \| null` | CAP-054 |
| `capture:hide` | send | `()` | CAP-160 |
| `capture:resize` | send | `(height: number)` | CAP-009 |
| `capture:open` | invoke | `(OpenCardRequest) → void` | CAP-145 |
| `capture:settings` | invoke | `() → void` | CAP-159 |
| `capture:shortcut` | invoke | `() → CaptureShortcut` | CAP-044 (used by Settings, 08) |
| `capture:allow` | invoke | `() → CaptureShortcut` | CAP-040 (panel and Settings) |
| `capture:show` | invoke | `() → void` | CAP-043 (Settings) |
| `ev:capture-shown` | event → panel | `CaptureContext` | CAP-014, CAP-083 |
| `ev:capture-context` | event → panel | `Partial<CaptureContext>` | CAP-014, CAP-083 |

Main-process hooks into other domains (`CaptureHandlers`): `mainWindow()`, `createSession(projectId, opts)` (03), `sendToSession(projectId, sessionId, text)` (03), `sessionTitle(projectId, sessionId)` (03), `boardChanged(projectId)` (→ `ev:board-changed`), `openCard(request)` (notify, 02), `openSettings()` (01/08). Exported to other main modules: `boardOpened(projectId)` (called by `board:load`), `openPanel()` (menu), `syncShortcut()` (settings), `stopCapture()` (shutdown), `registerCapture(handlers)`.

## UI details

Values are CSS px at 1×. Token values are given as `light / dark`. The panel follows the app's theme setting (system / light / dark, applied app-wide, 01/08), not only the OS appearance. There is **no** blur, vibrancy or backdrop filter anywhere in the panel: the window is fully transparent and every surface is an opaque fill with drop shadows. `prefers-reduced-motion: reduce` cuts every animation and transition to 0.01 ms (global.css).

### Base (inherited from global.css and board.css)

- Page background: transparent (html and body), so only the card, banner, recents and their shadows are visible.
- Text: `--text-primary` `rgba(0,0,0,0.88)` / `rgba(255,255,255,0.9)`; font `--font-ui` (`-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI Variable Text', 'Segoe UI', 'Helvetica Neue', sans-serif`), 13 px, line-height 1.4, antialiased; text not selectable except in the field; default arrow cursor.
- Buttons inherit font and colour. The field inherits font and colour and is selectable.
- Keyboard focus ring (`:focus-visible`, not on the field, which sets `outline: 0`): 2 px solid `--focus-ring` `rgba(0,122,255,0.5)` / `rgba(10,132,255,0.55)`, offset 2 px.
- Scrollbars (menu columns): 10 px wide, thumb `--border-strong` with a 3 px transparent border (≈ 4 px visible), radius 10, no track.
- Monospace (`.cap-mono`): `--font-mono` (`'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', Consolas, monospace`), 11 px.
- Every `<kbd>` also gets board.css's global `kbd` rule underneath its own class: `margin: 0 1px`, `border: 0.5px solid --border-strong` (`rgba(0,0,0,0.18)` / `rgba(255,255,255,0.18)`), `text-align: center`, and (where the class doesn't override them) `display: inline-block`, `min-width: 18px`, `box-shadow: 0 1px 0 --border`, `color: --text-secondary`. The resolved key caps are given below.
- Spinners: lucide `Loader2` rotating 360° every 0.9 s, linear, forever.
- Pulse (`cap-pulse`): opacity 1 → 0.55 at 50 % → 1, ease-in-out, forever.

### Stage (the transparent area the window shows)

- Column, items centred, 10 px gap between card, banner and recents; padding 40 top, 56 left/right, 64 bottom (receipt: 52 top); overflow hidden. 652 − 2 × 56 = 540 = card width.
- Stage colours: `--into-ink` = `--accent` `#007aff` / `#5eacff`; `--view-ink` `#8944ab` / `#d9a3f7`; `--working-ink` `#b25000` / `#ffb340`; `--waiting-ink` `#d70015` / `#ff7a72`.

### Card

- 540 px wide, radius `--radius-lg` 14, background `--bg-elevated` `#ffffff` / `#2a2a2d`.
- Shadow: light `0 16px 40px rgba(0,0,0,0.24), 0 0 0 0.5px rgba(0,0,0,0.16)`; dark `0 16px 40px rgba(0,0,0,0.55), 0 0 0 0.5px rgba(255,255,255,0.16)`.
- Dropping files: the same shadow plus `0 0 0 2px --accent` (`#007aff` / `#0a84ff`).
- Entrance `cap-rise`: from opacity 0, `translateY(-6px) scale(0.985)` to rest, 220 ms (`--dur-med`), `cubic-bezier(0.2, 0.8, 0.2, 1)` (`--ease-out`). It plays when the card is created (page load, or the card coming back after a receipt), **not** on every show; showing a hidden window does not replay it. In practice it is seen on a first open that happens while the page is still loading.
- Role `dialog`, label `Nodus capture`.

### Header (`.cap-head`)

- Row, items centred, gap 10, min-height 48, padding 8 12 8 10, bottom border 1 px `--separator` `rgba(0,0,0,0.07)` / `rgba(255,255,255,0.07)`.
- Route button: inline row, gap 8, max-width 360, height 32, padding 0 8 0 6, no border, radius 8, transparent, 13 px, no wrapping, pointer cursor. Hover (enabled): `--bg-hover` `rgba(0,0,0,0.05)` / `rgba(255,255,255,0.07)`. Menu open: `--bg-selected` `rgba(0,122,255,0.14)` / `rgba(10,132,255,0.24)`; hovering while open shows `--bg-hover` (the hover rule is more specific). Disabled: no hover change, no other style. `<strong>` 600. The `Into “…”` text ellipsizes. ARIA: `aria-haspopup="dialog"`, `aria-expanded`.
- Folder swatch: 13 × 10, radius 2 2 3 3 (top-left, top-right, bottom-right, bottom-left), fill = the colour's `frontBottom`: blue `#2da4f3`, purple `#a77beb`, pink `#f074aa`, red `#f76760`, orange `#fba343`, yellow `#f9cb45`, green `#5dcd6b`, graphite `#9a9aa0` (same in both themes).
- Kind badge: 24 × 24, radius 6, `--bg-sunken` `rgba(0,0,0,0.035)` / `rgba(255,255,255,0.04)`, icon 14 centred. Kind icons: session lucide `SquareTerminal` (text colour); into `CornerDownRight` in `--into-ink`; note a `#ffcc00` square, radius 3, (icon size − 2) px (12 in the header, 14 in the menu); view `LayoutDashboard` in `--view-ink`.
- Chevron: lucide `ChevronDown` 14, `--text-tertiary` `rgba(0,0,0,0.36)` / `rgba(255,255,255,0.36)`.
- Thinking line: inline row, gap 8, `--text-secondary` `rgba(0,0,0,0.56)` / `rgba(255,255,255,0.58)`, `Loader2` 14 spinning, the whole line pulsing every 1.2 s.
- Right side (`.cap-who`): inline row, no shrink, gap 6, 11 px, `--text-tertiary`. Dot: 6 × 6 circle `--accent` (`#007aff` / `#0a84ff`); while thinking it pulses every 0.9 s.
- Key cap (`.cap-kbd`, also used in the footer hint and menu footer): inline-flex centred, min-width 20, height 20 (border included), padding 0 5, radius 5, background `--bg-sunken`, inset ring `0 0 0 1px --border` (`rgba(0,0,0,0.1)` / `rgba(255,255,255,0.1)`) plus the 0.5 px `--border-strong` border and 0 1 px margin from the global `kbd` rule, font 500 11 px `--font-ui` (normal line height), `--text-secondary`.

### Field

- Block, full width, min-height 90, padding 14 16 10, no border, no outline, not resizable, transparent, 15 px (`--text-lg`), line-height 22, placeholder `--text-tertiary`. Auto height up to 200 px (8 × 22 + 24), then scrolls.

### Attachment tiles

- Row: wrapping, gap 8, padding 0 16 14.
- Tile: 150 × 70, column, gap 3, padding 8 10, overflow hidden, radius 9, `--bg-sunken`, inset ring 1 px `--border`, 12 px / 16 px, left-aligned.
- Tile top line: row, gap 5, 11 px, `--text-tertiary`. Icons 12 (`Globe`, `Plus`).
- Tab tile: 168 wide. Image tile: 96 wide, no padding, background `--bg-window` `#f5f5f4` / `#1e1e20`; image fills (cover, top-left). Image label: bottom-left at (6, 5), padding 1 5, radius 4, `rgba(0,0,0,0.6)`, `#fff`, 10 px / 14 px. File tile: row, centred, gap 8, 52 high; `FileText` 18 `--text-tertiary`; name and size stacked.
- Remove button: top-right at (4, 4), 20 × 20, radius 5, background `--bg-elevated` at 80 % (`rgba(255,255,255,0.8)` / `rgba(42,42,45,0.8)`), `--text-secondary`, `X` 11; opacity 0, 1 on tile hover or its own focus-visible, fade 120 ms (`--dur-fast`). On image tiles: `rgba(0,0,0,0.5)` background, `#fff` icon.
- Offer tile (button): same box, 1 px dashed `--border-strong` border, transparent, no ring, pointer; hover `--bg-hover`; top line `--text-secondary`; image preview 34 high, max full width, radius 3, cover top-left.

### Error, notices

- Error: margin 0 16 12, 12 px, `--waiting-ink`.
- Screenshot notice: row, centred, gap 10, margin 0 16 12, 12 px / 17 px, `--text-secondary`; `AlertCircle` 14 in `--status-waiting` (`#ff3b30` / `#ff453a`); text fills; then the accent button (as the banner button below).
- Voice notice inside the card: placed in the flow with margin 0 12 10 and no transform. Unclear: the panel gives `.voice-notice` no layout of its own (its pill style is scoped to `.sidekick`/`.session-mic` in `voice.css:4-5`; `capture.css:394-398` only resets position/transform), so it renders as a plain block: inline `AlertCircle` 14, message text (`--text-primary`, line-height 1.35), `.btn` action buttons (24 high, padding 0 8, 12 px, gap 4) and a 24 × 24 round dismiss button. Reproduce as rendered or confirm with the owner (05).

### Footer

- Row, centred, gap 2, padding 8 10, top border 1 px `--separator`.
- Icon buttons: 32 × 32, radius 8, transparent, `--text-secondary`, pointer; hover (enabled) `--bg-hover` and `--text-primary`; recording: `--status-waiting` background, `#fff`. Icons: `Scan` 16, `Paperclip` 16, `Mic` 16, recording `Square` 13 filled with the current colour. Disabled: no hover, no other change.
- Hint: margin-right 10, 12 px, `--text-tertiary`: key cap `⇧↩` + ` new line`.
- Voice status (05) in the hint's place, margin-right 10.
- Send button: inline row, centred, gap 8, height 32, padding 0 8 0 14, radius 8, `--accent` background, `--text-on-accent` `#ffffff`, 13 px 600, pointer; hover `--accent-hover` `#0068d9` / `#3396ff`; disabled `--bg-sunken` background, `--text-tertiary` text, default cursor. Its key cap (`.cap-send-key`): padding 1 5, radius 4, background `rgba(255,255,255,0.22)` (disabled: `--bg-sunken`), font 500 11 px `--font-ui`, plus the global `kbd` rule: inline-block, min-width 18, margin 0 1, 0.5 px `--border-strong` border, `box-shadow: 0 1px 0 --border`, centred text, and colour `--text-secondary` (the `↩` glyph is **not** white).

### Menu

- Absolute at top 50, left 8 inside the card, 524 wide, above the card's content (z 5), overflow hidden, radius 12, `--bg-elevated`, shadow `--shadow-lg`: light `0 18px 50px rgba(0,0,0,0.18), 0 0 0 0.5px rgba(0,0,0,0.12)`; dark `0 18px 50px rgba(0,0,0,0.5), 0 0 0 0.5px rgba(255,255,255,0.12)`. Entrance `cap-rise` 120 ms `--ease-out`.
- Reason: row, top-aligned, gap 8, padding 10 14, bottom border `--separator`, 12 px / 17 px, `--text-secondary`; dot 6 × 6 accent with 6 px top margin; `Claude's guess:` 600 `--text-primary`.
- Two equal columns; each a column with gap 2, padding 8, max-height 300, scrolls vertically; the second has a 1 px `--separator` left border.
- Column label: row, gap 6, padding 2 8 6, 11 px 600 `--text-tertiary`.
- Option: row, top-aligned, gap 10, padding 7 8, radius 8, transparent, 13 px, left, pointer; hover (enabled) `--bg-hover` (also over the selected one); selected `--bg-selected`; disabled opacity 0.55, default cursor. Kind option: icon box 16 wide, 1 px top margin, icon 16; text column gap 1, hint 11 px `--text-tertiary`. Row options (projects, sessions): centred, gap 8, secondary text 11 px.
- Check: lucide `Check` 14 in `--into-ink`. `Needs you`: 11 px 600 `--waiting-ink`. Session status dot: as `.cap-dot` below.
- Separator: 1 px `--separator`, margin 4 8. Empty line: padding 6 8, 12 px, `--text-tertiary`.
- Footer: row, gap 14, padding 8 14, top border `--separator`, 11 px `--text-tertiary`.
- ARIA: `role="dialog"` `aria-label="Where it goes"`; columns `role="listbox"` labelled `What to make` and `Which project` / `Which session`; options `role="option"` with `aria-selected`.

### Status marks

- Mark square: 9 × 9, radius 2; note `#ffcc00`, view `#af52de` (both themes).
- Dot: 8 × 8 circle; done `--status-done` `#007aff` / `#0a84ff`; working `--status-working` `#ff9500` / `#ff9f0a` with a 3 px ring of the same at 28 % (`rgba(255,149,0,0.28)` / `rgba(255,159,10,0.28)`); waiting `--status-waiting` `#ff3b30` / `#ff453a` with a 3 px ring at 28 % (`rgba(255,59,48,0.28)` / `rgba(255,69,58,0.28)`). No animation.

### Banner (under the card)

- Row, centred, gap 12, 540 wide, padding 10 12 10 14, radius 10, `--bg-elevated`, `--shadow-md` (light `0 4px 14px rgba(0,0,0,0.08), 0 0 0 0.5px rgba(0,0,0,0.1)`; dark `0 4px 14px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(255,255,255,0.1)`), 12 px / 17 px, `--text-secondary`.
- Accent button (`Allow…`, `Open System Settings`): no shrink, 28 high, padding 0 12, radius 7, `--accent`, `#fff`, 600, pointer, 12 px (inherits), no hover change.

### Recents

- Column, centred, gap 8, 540 wide. Grid of 3 equal columns, gap 8.
- Recent card (button): column, gap 5, 104 high, padding 10 12, radius 10, `--bg-elevated`, `--shadow-md`, left-aligned, pointer. Hover: `--shadow-lg`. Waiting: `0 0 0 2px --status-waiting, --shadow-md` (kept on hover).
- Entrance: `cap-rise` 220 ms `--ease-out`, fill both, delays 0 / 30 / 60 ms for the 1st / 2nd / 3rd; plays when they appear (e.g. the text is cleared).
- Top line: row, gap 6, 11 px `--text-tertiary`: mark, project name (fills, ellipsized), age. Title: 13 px 600, line-height 17, at most 2 lines. Status: pushed to the bottom, 11 px 600, `--text-tertiary`; working `--working-ink`; waiting `--waiting-ink`.
- More pill: padding 4 10, fully round, `--bg-elevated` at 86 % (`rgba(255,255,255,0.86)` / `rgba(42,42,45,0.86)`), `--shadow-sm` (light `0 1px 2px rgba(0,0,0,0.06), 0 0 0 0.5px rgba(0,0,0,0.08)`; dark `0 1px 2px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.08)`), 12 px `--text-secondary`; the ` · ↑ for earlier captures` part `--text-tertiary`.

### Receipt

- Card: 288 wide, column, gap 8, padding 12 14, radius 12, `--bg-elevated`, shadow `0 16px 40px rgba(0,0,0,0.24), 0 0 0 0.5px rgba(0,0,0,0.16)` in **both** themes. Entrance `cap-shrink`: from opacity 0.4 and `scale(1.5) translateY(18px)`, 280 ms, `cubic-bezier(0.34, 1.4, 0.64, 1)` (`--ease-spring`). Role `status`.
- Top line: row, gap 7, 11 px; view: `Loader2` 12 spinning + text, both `--view-ink`; others: working dot + text in `--working-ink` 600.
- Title: 15 px 600, line-height 20; with the sticker, 56 px right padding.
- Body line: 12 px / 17 px `--text-tertiary`.
- Footer: row, space-between, padding-top 7, top border 1 px `--separator`, 11 px; right part inline row, gap 6, `--text-secondary`, folder swatch, project name, `· auto mode` in `--text-tertiary`.
- Link button: no padding/border/background, `--into-ink`, 12 px 500, pointer, underline on hover.
- Sticker: absolute at top −20, right −36, 104 wide, padding 7 8, radius 2, background = 72 % `#ffe27a` (yellow `frontTop`) + 28 % `--note-paper` (`#ffffff` / `#b4b4b8`) = `#ffea9f` / `#ead58b`, `--note-shadow` (light `0 1px 1.5px rgba(0,0,0,0.08), 0 8px 16px -8px rgba(0,0,0,0.3)`; dark `0 1px 1.5px rgba(0,0,0,0.3), 0 8px 18px -8px rgba(0,0,0,0.6)`), text `--note-ink` `rgba(0,0,0,0.82)` (both), 10 px / 13 px, rotated 4°.
- Note receipt: 216 wide, min-height 160, padding 16 18, radius 4, background a top shade `linear-gradient(rgba(0,0,0,0.035), transparent 22px)` over the same yellow paper (`#ffea9f` / `#ead58b`), `--note-shadow-lifted` (light `0 2px 4px rgba(0,0,0,0.08), 0 20px 32px -12px rgba(0,0,0,0.36)`; dark `0 2px 4px rgba(0,0,0,0.3), 0 20px 34px -12px rgba(0,0,0,0.7)`), `--note-ink`, 15 px, line-height 1.4, pre-wrap, breaks anywhere, rotated −1.5°, `cap-shrink` 280 ms spring (animating towards its rotation).
- Caption pill: padding 4 10, fully round, `--bg-elevated`, `--shadow-sm`, 12 px `--text-secondary`.

### Copy (verbatim)

| Where | Text |
| --- | --- |
| Window title | `Nodus Capture` |
| Card label | `Nodus capture` |
| Header, no project | `No projects yet` |
| Header, thinking | `Working out where this goes…` |
| Header, kinds | `New session`, `Note`, `View`, `Into “<title>”`, fallback title `a session`, joiners `on` (note), `in` |
| Header right | `Your choice`, `Claude's guess`, `Claude works out what it is as you type`, key `Tab` |
| Field label / placeholders | `What to capture`; `Add a project in Nodus to capture into`; `Ask Claude to do something, or jot a note` |
| Tiles | `Screenshot`, `Copied image`, `Copied text`, remove `Remove` |
| Screenshot notice | `macOS kept the screenshot from Nodus. Turn Nodus on in Screen & System Audio Recording, then take it again.`, button `Open System Settings` |
| Footer buttons (title = aria-label) | `Take a screenshot`, `Attach files`, `Speak`, `Stop and transcribe` |
| Footer hint | `⇧↩` / `Shift+Enter` + ` new line` |
| Send verbs | `Start session`, `Send`, `Put up note`, `Build view`, sending `Working it out…`, key `↩` / `Enter` |
| Menu | `Claude's guess:`, `Make`, `Project`, `New session` / `Claude starts on it, in auto mode`, `Send into a session` / `Adds to its conversation` (or `No sessions are running`), `Note` / `A sticky on the board, nothing runs`, `View` / `A live page Claude keeps up to date`, `Needs you`, tooltip `It’s waiting for your answer. Answer it on the board first.`, `No sessions on this board`, `<n> sessions` / `no sessions`, footer `Tab` ` next kind`, `Esc` ` close` |
| Banner | `To open this from any app with a double tap of ⇧, allow Nodus in Accessibility. Already on there? Click Allow… anyway.`, button `Allow…`; `The double tap of ⇧ couldn’t start on this computer. Use File › Capture… instead.` |
| Recents | tooltip `Open it on the board`, ages `now` / `<m>m` / `<h>h` / `<d>d`, statuses `Note`, `View`, `Needs you`, `Working`, `Done`, label `Recent captures`, more `<n> more · <t4>, <t5>` + ` · ↑ for earlier captures` |
| Receipt | `Working`, `Sent`, `Building view`, `Claude is starting on it, expanded on the board.`, `Your message is in. Claude is on it.`, `Its brain is putting it together.`, `Open on board`, `· auto mode`; note: `On <project>’s board · ` + `Open` |
| Session fallback title | `Untitled session` |
| Menu item | `Capture…` (File menu) |

Lucide icons: `AlertCircle` (circle-alert), `Check`, `ChevronDown`, `CornerDownRight`, `FileText`, `Globe`, `LayoutDashboard`, `Loader2` (loader-circle), `Mic`, `Paperclip`, `Plus`, `Scan`, `Square`, `SquareTerminal`, `X`; via voice parts (05): `Loader2`, `AlertCircle`, `RotateCcw`, `Settings`, `X`. Stroke width 2 (lucide default).

## Keyboard & mouse

| Input | Where | Effect | IDs |
| --- | --- | --- | --- |
| ⇧ tapped twice (either Shift, each ≤ 350 ms held, ≤ 400 ms apart, nothing else pressed or clicked) | anywhere, any app | toggle the panel | CAP-021…CAP-031, CAP-017 |
| File › Capture… | menu (macOS menu bar, Windows ☰) | open the panel | CAP-042 |
| Return | field | send (waits for Claude's guess first if needed) | CAP-120, CAP-123 |
| ⇧Return | field | new line | CAP-120 |
| Tab / ⇧Tab | field, with text | next / previous kind; without text does nothing and focus stays | CAP-114 |
| Esc | field | close the menu, else hide the panel (draft kept); while the mic is busy: cancel voice only | CAP-121, CAP-104 |
| ↑ / ↓ | field, empty or browsing history | earlier / later sent captures | CAP-122 |
| IME composing | field | all of the above ignored | CAP-119 |
| Click header | panel | toggle the menu, focus back to field | CAP-087 |
| Pointer down outside menu and header | panel | close the menu (the click also acts) | CAP-107 |
| Click outside the panel | other app / desktop | hide the panel (unless resting/busy/within 250 ms of showing) | CAP-012 |
| Hover a tile | panel | show its × | CAP-096 |
| Drag files over / drop on the card | panel | blue outline / attach | CAP-062, CAP-097 |
| Click dashed clipboard tile | panel | attach the clipboard | CAP-098 |
| Click a recent | panel | hide panel, open it on the board | CAP-152 |
| First click on the resting receipt (panel not key) | panel | works immediately (`Open on board` / `Open`) | CAP-003, CAP-144 |

The menu's options are buttons: Tab from a focused button would move focus normally, but every menu click returns focus to the field, so in practice the keys above always reach the field. There is no arrow-key navigation inside the menu.

## Platform differences

| Aspect | macOS | Windows |
| --- | --- | --- |
| Window | `NSPanel`, non-activating, all Spaces + over full-screen apps, level 101 (`pop-up-menu`) | Top-most tool window (no taskbar button); Electron's `type: 'panel'` and all-workspaces calls are skipped; the panel activates to take the keyboard |
| Double-tap hook | needs Accessibility; polled every 3 s until granted | needs nothing; starts at launch |
| Allow… | `tccutil reset` (packaged only) + prompt + opens Privacy_Accessibility | returns the state, does nothing |
| Front app / browser tab | `lsappinfo`; tab via `osascript` for Safari/Chromium family | none (no app, no tab ever) |
| Screenshot | `screencapture -i -x -o` (crosshair, Space for window); `'needs-permission'` without Screen Recording | whole display under the pointer, at native resolution, no interaction, never `'needs-permission'` |
| Open System Settings (screen) | Privacy_ScreenCapture pane | no-op (the notice never shows) |
| Key caps | `⇧↩`, `↩` | `Shift+Enter`, `Enter` (`keys()` in platform.ts: ⌘/⌃→Ctrl, ⌥→Alt, ⇧→Shift, ↩→Enter, ⌫→Del; modifiers ordered Ctrl, Alt, Shift, joined by `+`) |
| Literal key caps | `Tab`, `Esc` (menu footer, header) unchanged on both | same |
| Fonts | SF Pro Text / Menlo (`--font-mono`) | Segoe UI Variable Text / Cascadia Mono |
| Hook key codes | libuiohook 42/54 on both (map native Shift codes to these) | same |
| Guess process | `claude` | `claude.exe`, `windowsHide` (no console flash) |
| Screen Recording / Accessibility copy in banner/notice | shown | `needs-permission` never occurs, so the Accessibility banner never shows; `unavailable` banner can |

## Edge cases & errors

Errors shown in the panel's error line (verbatim):

| Message | When |
| --- | --- |
| `Add a project in Nodus first` | send with no project in the route |
| `There’s no session to send into. Pick another kind.` | send with kind into and no session |
| `Type something first` | main: submitted text empty after trimming |
| `Pick what to make` | main: unknown kind |
| `Unknown project <id>` | main: project gone (store, 01) |
| `That session is no longer on the board` | main: into a session that fails the session check (CAP-148), or 03's send path finds it gone/archived/terminal |
| `It’s waiting for your answer. Open it to reply.` | 03's send path: into a live session that is waiting |
| any other message from 03 (`createSession`) or 04 (`createView`) | a spawn failure |

Logged only: `[capture] the key hook failed to start`, `[capture] the key hook failed to stop`, `[capture] guess failed`. The guess never shows an error: every failure (no Claude Code, timeout, bad JSON, `is_error`, cancelled) is a `null` guess, and the header keeps the previous guess or falls back.

Other edge cases (each covered by the behavior cited):

- **CAP-161** The panel hides while a guess is in flight (Esc, click away): the guess process is killed and resolves `null`; the page keeps whatever guess it showed (CAP-016, CAP-066).
- **CAP-162** Return while the guess is still thinking: the send waits for it; if the panel is hidden meanwhile, the guess resolves `null`, the send continues with the route as it was and the capture is still made; `rest()` does nothing because the panel isn't visible; the receipt is set on the hidden page and is cleared on the next show (CAP-123, CAP-131, CAP-083).
- **CAP-163** A session picked for into that started waiting before send: main rejects with `It’s waiting for your answer. Open it to reply.`; the text stays (CAP-127).
- **CAP-164** The route's into session vanished from targets: the header shows `Into “a session”`; sending fails with `That session is no longer on the board` (CAP-086, CAP-127).
- **CAP-165** Allow… while the switch is still off in System Settings: the state stays `needs-permission`, the banner stays until the next show, even after the poll starts the hook (the page is only told on show) (CAP-117, CAP-037).
- **CAP-166** A draft restored with a tab: the front browser's tab is not added; the restored tab may be stale (CAP-051).
- **CAP-167** Same clipboard as one already offered (or seen at a send): no offer tile, across restarts (CAP-098, CAP-157).
- **CAP-168** The panel opened while a Nodus window is focused: no app and no tab; the clipboard is still offered (CAP-015).
- **CAP-169** A capture record whose project was archived still lists in recents, with a blank project name (targets omit archived projects) (CAP-150, CAP-152).
- **CAP-170** A recent's `createdAt` in the future (clock change) shows `now` (CAP-152).
- **CAP-171** Dropped directories become file tiles with no size; Claude gets the folder path (CAP-061).
- **CAP-172** Thumbnails are saved with the draft, so a draft with screenshots carries their data URLs in page storage until sent or removed (CAP-157).
- **CAP-173** A `captures/` subdirectory older than 30 days stops the sweep (CAP-063).
- **CAP-174** macOS: once the hidden panel window exists, clicking the Dock icon after closing the board window does not bring the board window back, because 01's `activate` handler re-creates it only when there are no windows at all (`src/main/index.ts:146-148`). Unclear whether to reproduce; flag to 01. The board window still comes back through capture:open / capture:settings / notifications.
- **CAP-175** Windows: once the hidden panel window exists (3 s after launch with the setting on), closing the board window no longer quits Nodus (`window-all-closed` needs every window closed, `src/main/index.ts:151-154`); Nodus keeps running with the double tap live, and a second launch, a recent or a voice Settings click brings the board back. This contradicts docs/how-it-works.md ("Closing the window quits Nodus"). Unclear: likely unintended; decide with 01 before porting.
- **CAP-176** Two guesses can't run at once: a new one kills the old (CAP-074).
- **CAP-177** The page polls targets only while visible; a hidden window doesn't poll (Chromium reports a hidden window's page as hidden) (CAP-118).

## Tests to port

`src/main/doubleTap.test.ts` (`DoubleTap`; helper `press(key, at, hold = 60)` = down at `at`, up at `at + hold`, returns `at + hold`; SHIFT = 42, RIGHT_SHIFT = 54, A = 30):

1. `fires on two quick taps of Shift`: press(SHIFT, 0) → end 60; press(SHIFT, 180) → fired exactly once.
2. `takes either Shift key`: press(SHIFT, 0); press(RIGHT_SHIFT, 180) → fired once.
3. `doesn't fire on one tap`: press(SHIFT, 0) → never fired.
4. `doesn't fire when the taps are too far apart`: press(SHIFT, 0) → 60; press(SHIFT, 60 + 401) → never.
5. `doesn't count Shift held too long`: press(SHIFT, 0, hold 351) → 351; press(SHIFT, 451) → never.
6. `doesn't count typing capitals`: down SHIFT@0; press(A, 10, 30); up SHIFT@60; down SHIFT@150; press(A, 160, 30); up SHIFT@210 → never.
7. `a key between the taps breaks it`: press(SHIFT, 0) → 60; press(A, 100, 20); press(SHIFT, 180) → never.
8. `a mouse click between the taps breaks it`: press(SHIFT, 0); `other()`; press(SHIFT, 180) → never.
9. `ignores Shift repeating while held`: down SHIFT@0, @30, @60; up SHIFT@90; press(SHIFT, 200) → fired once.
10. `a third tap starts over rather than firing again`: press(SHIFT, 0) → 60; press(SHIFT, 160) → 220; press(SHIFT, 320) → fired once.

`src/main/captureGuess.test.ts` (fixture: now = 1 800 000 000 000; projects `p-store` "storefront" blue, `p-api` "api" purple; sessions `s-round` (p-store, "Fix checkout total rounding", done, live, now − 25 min), `s-uuid` (p-api, "Migrate orders to UUIDs", waiting, live, now − 2 min), `s-stripe` (p-api, "Bump Stripe SDK", working, live, now); recents []; default `p-store`):

11. `buildGuessPrompt` › `lists projects with their sessions, the screen and the text` (text `also cover the coupon path`, app `Safari`, a Safari tab "Refund fails #482" with an issue URL, a file `checkout.log`): contains `Projects (default: storefront, p-store):`, `- project p-store: storefront`, `  - session s-round: “Fix checkout total rounding”, idle, active 25 min ago`, `  - session s-uuid: “Migrate orders to UUIDs”, waiting for the user, active 2 min ago`, `The app in front: Safari`, `Its browser tab: Refund fails #482 (<url>)`, `A file is attached: checkout.log`; ends with `The capture:\n"""\nalso cover the coupon path\n"""`.
12. `readGuess` › `takes a valid answer`: `{kind:'into', project:'p-store', session:'s-round', reason:'It follows on.'}` → `{kind:'into', projectId:'p-store', sessionId:'s-round', reason:'It follows on.'}`.
13. `falls back to the default project for one it made up`: `{kind:'note', project:'nope', session:'', reason:'An idea.'}` → kind `note`, projectId `p-store`.
14. `starts a session instead of sending into one that is waiting or missing`: into `s-uuid` (waiting) in `p-api` → `{kind:'session', projectId:'p-api', sessionId: undefined}`; into `gone` → kind `session`.
15. `takes the session's own project`: into `s-stripe` naming `p-store` → projectId `p-api`.
16. `turns down nonsense`: `null` → `null`; `{kind:'party', project:'p-api', session:'', reason: 7}` → `{kind:'session', projectId:'p-api', sessionId: undefined, reason: ''}`.

`src/main/screenContext.test.ts`:

17. `parseAppInfo` › `reads the name and bundle id from lsappinfo`: the 4-line Safari output → `{name:'Safari', bundleId:'com.apple.Safari'}`.
18. `falls back to the bundle id when there is no name`: `[ NULL ] ASN:0x0-0x1:\n    bundleID="com.mitchellh.ghostty"` → `{name:'ghostty', bundleId:'com.mitchellh.ghostty'}`.
19. `has nothing without a bundle id`: `[ NULL ] ASN:0x0-0x1:\n    bundleID=[ NULL ]` → `null`.
20. `tabScript` › `asks Safari for its front document`: exact Safari script for `com.apple.Safari`.
21. `asks a Chromium browser for its active tab`: `company.thebrowser.Browser` → contains `tell active tab of front window`.
22. `has no script for apps that are not browsers it can read`: `org.mozilla.firefox` → `null`; `isBrowser('com.microsoft.VSCode')` → false.
23. `never puts an odd bundle id into a script`: `com.apple.Safari" to do shell script "x` → `null`.

`src/shared/capture.test.ts`:

24. `titleFrom` › `takes the first sentence`: `Refunds fail with a coupon. Write a test, then fix it` → `Refunds fail with a coupon`.
25. `takes the first line`: `Bump Stripe\nand run the tests` → `Bump Stripe`.
26. `cuts a long sentence at a word`: `Look into why the checkout total is rounded down when the customer has a coupon applied` → length ≤ 48 and equals `Look into why the checkout total is rounded…`.
27. `doesn't take a dot inside a word as the end of a sentence`: `Fix README.md links` → unchanged.
28. `describeAttachments` › `is empty with nothing attached`: `[]` → `''`; `[{text:'   '}]` → `''`.
29. `lists the tab, images and files, then the copied text`: text first in the input still comes last; exact output `From my screen as I wrote this:\n\n- The Safari tab I had open: Refund fails #482 (<url>)\n- A screenshot: /tmp/shot.png\n- An image I copied: /tmp/copied.png\n- A file: /logs/checkout.log\n\nText I copied:\n` + "```\nTypeError: amount.minus\n```".
30. `fences copied text with more backticks than it holds`: text "```js\nx()\n```" → output contains "````\n```js\nx()\n```\n````".
31. `withScreen` › `leaves text alone with nothing attached`: `'  Fix it  '` → `Fix it`.
32. `puts the screen after the text`: `Fix it` + file `/a.log` → `Fix it\n\nFrom my screen as I wrote this:\n\n- A file: /a.log`.

Suggested new Rust tests (not in 0.9.4, for behaviors with no test): `ago`/`flat`/`statusWord`, prompt with no screen lines (two blank lines), `cleanAttachments`, `spotOnBoard` (viewport / right-most card / dock), `defaultProject` ordering, `recentOf` (`New view` title), `sessionInfo` filters, recents cap 20, `capture:resize` clamping, `offerKey`, page `ago`, `bytes`, `clip`, `cycleKind` with into skipped, `pickKind` fallbacks, history ↑/↓.

## Rust mapping

**Crates**

- `nodus-capture` (no UI): `double_tap.rs` (pure `DoubleTap` with `key_down(code, at_ms)`, `key_up`, `other`, constants), `prompt.rs` (`titleFrom`, `describeAttachments`, `withScreen`, fence; port of shared/capture.ts), `guess.rs` (`build_guess_prompt`, `read_guess`, the process runner with one running child behind a mutex; cancel = kill + resolve `None`; 30 s timeout via `tokio::time::timeout`; argv verbatim), `context.rs` (front app, tab script, browser tab, clipboard, screenshot, prune), `targets.rs` (`session_info`, `recent_of`, `default_project`, `targets`), `submit.rs` (`clean_attachments`, `spot_on_board`, `submit` over traits for 03's `create_session` / `send_to_session` / `session_title` and 04's `create_view`), `hook.rs` (`ShortcutState`, start/stop/poll, permission ask), `records.rs` (recents in `state.json` via `nodus-store`).
- `nodus-platform`: `accessibility::is_trusted(prompt: bool)` (`AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt`), `screen_capture::preflight()` (`CGPreflightScreenCaptureAccess`), `open_url` (`x-apple.systempreferences:` URLs), clipboard (`NSPasteboard`: `NSPasteboardTypeString`, then PNG/TIFF image data; Windows: `CF_UNICODETEXT`, then `CF_DIB`/`PNG` (e.g. `clipboard-win`/`arboard`)), `display_under_pointer()` (work area + scale), global input hook (below), `run(file, args, timeout)` without a console window.
- `nodus-core`: commands `capture_ready`, `capture_targets`, `capture_guess`, `capture_submit`, `capture_screenshot`, `capture_allow_screen`, `capture_files`, `capture_clipboard`, `capture_hide`, `capture_resize`, `capture_open`, `capture_settings`, `capture_shortcut`, `capture_allow`, `capture_show`; events `CaptureShown(CaptureContext)`, `CaptureContext(Partial)`, and `BoardChanged`. `board_load` calls `capture.board_opened`. Settings updates call `sync_shortcut`. Menu "Capture…" calls `open_panel`.
- `nodus-ui`: `capture_panel` (the second window and its view: header, field, tiles, menu, banner, recents, receipt, all values in UI details), using shared theme tokens and the voice components from 05. Page-local values (`capture.draft`, `capture.attachments`, `capture.history`, `capture.clipboardDone`) need a home: Decision needed (none in plan): e.g. a small JSON beside `state.json`; carrying 0.9.4's values over means reading Chromium's `Local Storage/leveldb` for the renderer origin (optional; the plan's data-compat rule names state.json, views/ and captures/ only).

**macOS window (objc2 / objc2-app-kit)**

- An `NSPanel` subclass: style mask borderless + `NSWindowStyleMaskNonactivatingPanel`; `canBecomeKeyWindow = YES`; `level = NSPopUpMenuWindowLevel` (101); `collectionBehavior = CanJoinAllSpaces | FullScreenAuxiliary` (Electron's `setVisibleOnAllWorkspaces(true, {visibleOnFullScreen:true})`; never change the activation policy); `opaque = NO`, clear background, `hasShadow = NO`, `hidesOnDeactivate = NO`, not movable by background, no resize; content view `acceptsFirstMouse: YES`. Unclear: Electron's exact style bits for `type: 'panel'` — verify against Electron 44's `NativeWindowMac` in spike S1.
- Show + focus = `makeKeyAndOrderFront:`; rest = `orderOut:` then `orderFrontRegardless` (no key); hide = `orderOut:`. Blur = `windowDidResignKey:`; "focused" = `isKeyWindow`. Never `NSApp.activate`.
- Placement in Cocoa coordinates: convert the top-left rule (CAP-008) from the top-left-origin work area (`NSScreen.visibleFrame` flipped against the primary screen height); `capture:resize` keeps the top edge fixed (adjust origin.y when height changes).
- GPUI: a second window with transparent background (`WindowBackgroundAppearance::Transparent`) and a pop-up/panel kind (`WindowKind::PopUp` creates an NSPanel in GPUI's mac backend; confirm it can become key without activating and accepts the collection behavior; otherwise wrap the NSWindow GPUI creates). The spike S1 item "a transparent floating panel over full-screen apps" covers this.

**macOS hook (CGEventTap)**

- `CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionDefault, mask(keyDown | keyUp | flagsChanged | leftMouseDown | rightMouseDown | otherMouseDown), cb)` on its own thread with a `CFRunLoop`; the callback returns the event unchanged and only forwards `(kind, keycode, performance-clock ms)` to the detector over a channel. Use an active (default) tap, not listen-only: listen-only taps fall under Input Monitoring, a different permission from the Accessibility one 0.9.4 asks for (verify; libuiohook gates on `AXIsProcessTrusted`). Re-enable the tap on `kCGEventTapDisabledByTimeout` / `kCGEventTapDisabledByUserInput`.
- Shift arrives as `flagsChanged`, not keyDown/keyUp: keycode 56 (`kVK_Shift`) → 42, 60 (`kVK_RightShift`) → 54; down when its device bit is now set (`NX_DEVICELSHIFTKEYMASK` 0x2 / `NX_DEVICERSHIFTKEYMASK` 0x4), up when cleared. Other modifiers' `flagsChanged` presses (⌘, ⌥, ⌃, Fn, Caps Lock) are key downs of non-Shift keys → `other()` (libuiohook reports them as key presses). Key auto-repeat key-downs of ordinary keys also call `other()`.
- Times: `std::time::Instant` at callback time, in ms (CAP-030).
- Start fails (tap creation returns null although trusted) → `unavailable` for the run (CAP-034).

**Windows**

- Window: `WS_POPUP` with `WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED` (or DirectComposition transparency as GPUI does), no taskbar button; show + focus = `ShowWindow(SW_SHOW)` + `SetForegroundWindow`; rest = hide then `SW_SHOWNOACTIVATE`; blur = `WM_ACTIVATE(WA_INACTIVE)`.
- Hook: `SetWindowsHookExW(WH_KEYBOARD_LL)` + `WH_MOUSE_LL` on a thread with a message loop (as libuiohook does). `VK_LSHIFT` (0xA0) → 42, `VK_RSHIFT` (0xA1) → 54; `WM_KEYDOWN`/`WM_SYSKEYDOWN` → down, `WM_KEYUP`/`WM_SYSKEYUP` → up; `WM_LBUTTONDOWN`/`WM_RBUTTONDOWN`/`WM_MBUTTONDOWN`/`WM_XBUTTONDOWN` → `other()`. Always call `CallNextHookEx`.
- Screenshot: `Windows.Graphics.Capture` or GDI `BitBlt` of the monitor under the pointer at physical size → PNG.
- File dialog: `IFileOpenDialog` with `FOS_ALLOWMULTISELECT` (macOS: `NSOpenPanel` `canChooseFiles`, `allowsMultipleSelection`, no directories), parented to the panel.

**Risks**

1. The NSPanel behaviour (key without activation, over full-screen Spaces, transparent, first-mouse) through GPUI; a regular GPUI window would activate Nodus and pull the board window forward.
2. The CGEventTap must reproduce libuiohook's modifier handling (Shift via `flagsChanged`, other modifiers as `other()`), and the tap type decides which TCC permission is needed (Accessibility vs Input Monitoring); a wrong choice silently breaks the double tap or changes the permission prompt.
3. The Accessibility/TCC flow (`tccutil reset` for packaged builds only, then the prompt) must run under the same bundle id and a code signature that macOS attributes to Nodus; osascript/screencapture/lsappinfo children must be attributed to Nodus as the responsible process (Automation and Screen Recording consent).
4. The guess's stale-answer handling spans two layers (page run counter + single running child); a port that cancels only one side shows stale routes.
5. Page-local draft/history storage has no Rust home yet (see Decision needed above), and the global `kbd`/unstyled voice-notice quirks must be reproduced deliberately.

## Parity checklist

- [x] CAP-001 One panel window, created lazily (first open or 3 s after hook start), reused, recreated if destroyed
- [x] CAP-002 Window: hidden, frameless, transparent `#00000000`, no shadow, fixed, no taskbar, always on top, title `Nodus Capture`
- [x] CAP-003 Accepts first mouse (receipt link works without focus)
- [x] CAP-004 macOS NSPanel: key without activating Nodus
- [x] CAP-005 macOS: all Spaces + over full-screen apps, Dock icon kept
- [x] CAP-006 Level `pop-up-menu` (101)
- [x] CAP-007 Width 652, initial height 320
- [x] CAP-008 Placed on pointer's display: centred, top = 20 % of work area − 40, height kept
- [x] CAP-009 `capture:resize` clamps 120–900, rounds, keeps top/width, ignores bad input
- [x] CAP-010 Page reports `ceil(max(stage bottom, menu bottom + 48))` on change; no click-through
- [x] CAP-011 No navigation; http(s) new-window → browser
- [x] CAP-012 Blur hides after 50 ms re-check, except busy / resting / < 250 ms after show
- [x] CAP-013 Closed window reference dropped
- [x] CAP-014 Shown/context events; nothing sent while loading; `capture:ready` returns stored context while open
- [x] CAP-015 Open order: own-focus check, front app, place, targets snapshot, shown event, show+focus, then clipboard, then tab
- [x] CAP-016 Hide cancels the guess, clears resting, hides (never closes)
- [x] CAP-017 Toggle: hide only if open and focused, else open
- [x] CAP-018 Open = exists and visible (resting counts)
- [x] CAP-019 Rest after submit: hide + show without focus; blur ignored
- [x] CAP-020 Aside: busy during screenshot/dialog, then show + focus
- [x] CAP-021 TAP_MS 350, GAP_MS 400 (inclusive), Shift codes 42/54, either key
- [x] CAP-022 Detector state downAt / spoiled / lastTapAt
- [x] CAP-023 Non-Shift key down = other()
- [x] CAP-024 Shift repeat / Shift while spoiled ignored
- [x] CAP-025 Non-Shift key up ignored
- [x] CAP-026 Shift up: clean check, reset, unclean clears lastTapAt
- [x] CAP-027 Second clean tap within gap fires and resets; else remembers key-up time
- [x] CAP-028 other(): spoil a held Shift, clear downAt and lastTapAt
- [x] CAP-029 Any mouse button down = other(); moves/wheel ignored
- [x] CAP-030 Monotonic ms clock at receipt, not hook timestamps
- [x] CAP-031 Double tap toggles; no other global shortcut
- [x] CAP-032 Shortcut state priority off > unavailable > needs-permission > ready
- [x] CAP-033 `settings.capture` defaults true
- [x] CAP-034 Hook start once; failure → unavailable for the run + log
- [x] CAP-035 Hook stop; failure logged and swallowed
- [x] CAP-036 Panel pre-created 3000 ms after hook start
- [x] CAP-037 syncShortcut: off → stop, trusted → start, else poll 3000 ms
- [x] CAP-038 syncShortcut at registration and on every settings update
- [x] CAP-039 Quit clears poll and stops hook
- [x] CAP-040 Allow…: tccutil reset (packaged), AX prompt, open Accessibility pane, sync, return state
- [x] CAP-041 Dev builds never run tccutil reset
- [x] CAP-042 File › Capture… first File item + separator, no accelerator, always opens
- [x] CAP-043 `capture:show` opens like the menu
- [x] CAP-044 `capture:shortcut` returns state
- [x] CAP-045 Front app via lsappinfo (1000 ms timeouts), macOS only
- [x] CAP-046 lsappinfo parsing: bundleID, quoted name or last id segment
- [x] CAP-047 Front app name only feeds the guess and tab reading
- [x] CAP-048 Readable browser bundle ids (2 Safari + 10 Chromium)
- [x] CAP-049 Tab script per family; odd ids → none
- [x] CAP-050 osascript 60 s; URL must be http(s); title = rest joined by space
- [x] CAP-051 Tab attachment inserted first unless one exists
- [x] CAP-052 Clipboard offer: trimmed text ≤ 20 000 first, else first image/* item
- [x] CAP-053 Thumbnail ≤ 360 px wide, PNG data URL
- [x] CAP-054 Taking clipboard: text, or image saved to captures/ with copied: true
- [x] CAP-055 macOS screenshot `screencapture -i -x -o`, 5 min, needs-permission only when no file and not granted; NODUS_SCREENCAPTURE
- [ ] CAP-056 Windows screenshot: whole display under pointer at physical size
- [x] CAP-057 Missing/empty image → null, else image attachment
- [x] CAP-058 Screenshot hides panel, waits 150 ms, shows + focuses after
- [x] CAP-059 Allow-screen opens Privacy_ScreenCapture (macOS only)
- [x] CAP-060 Files: absolute dropped paths, or multi-select open dialog aside
- [x] CAP-061 Describe ≤ 20 files/dirs: name, size (0 for dirs); errors skipped
- [x] CAP-062 Drag over/leave/drop on the card; paths from dropped files
- [x] CAP-063 Launch sweep of captures/ older than 30 days by mtime
- [x] CAP-064 Guess after 700 ms pause, ≥ 3 chars, no choice, text changed
- [x] CAP-065 Empty text resets guess, choice and in-flight state
- [x] CAP-066 Ask: run counter, stale answers dropped, null keeps previous guess
- [x] CAP-067 Main guess uses opened-on targets and app, text ≤ 20 000, cleaned attachments
- [x] CAP-068 guessCapture cancels previous; empty text/no projects → null; no claude → reject
- [x] CAP-069 Spawn: exact argv, temp cwd, login env, stdin closed, hidden window
- [x] CAP-070 30 s timeout → SIGKILL → null
- [x] CAP-071 Spawn error logged → null
- [x] CAP-072 Close: parse JSON, is_error → null, else readGuess(structured_output)
- [x] CAP-073 Settles once, clears timer and running slot
- [x] CAP-074 cancelGuess kills (SIGKILL) and resolves null
- [x] CAP-075 Guess prompt layout, ≤ 8 sessions per project, text ≤ 4000
- [x] CAP-076 Prompt ages `just now` / `N min ago` / `N h ago` (< 48) / `N days ago`
- [x] CAP-077 flat(): collapse whitespace, cut with `…` (80/120/200/120)
- [x] CAP-078 Status words waiting/working/idle/stopped
- [x] CAP-079 Screen lines: app, tab, image, file, text wording
- [x] CAP-080 readGuess fallbacks: project, kind, into→session, reason, session's project
- [x] CAP-081 Page start: transparent doc, subscribe, ask ready
- [x] CAP-082 Initial state from saved draft, attachments, history, offer key
- [x] CAP-083 On shown: clear receipt/menu/error/notice, replace context, refresh, focus caret end (twice); draft kept
- [x] CAP-084 Route = choice > guess > fallback (default or first project)
- [x] CAP-085 No projects / has text definitions
- [x] CAP-086 Header contents: project / thinking line / kind badge + label + project
- [x] CAP-087 Header disabled until targets load and when no projects; click toggles menu
- [x] CAP-088 Header right: `Your choice` / `Claude's guess` + Tab cap / empty-state line
- [x] CAP-089 Field: label, placeholders, 3 rows, autofocus, spellcheck; typing resets history/error
- [x] CAP-090 Field auto-height max 200 px, min 90
- [x] CAP-091 Tiles row when attachments or offer; offer last
- [x] CAP-092 Tab tile: host without www, title or URL, tooltip URL
- [x] CAP-093 Image tile: thumb, `Screenshot` / `Copied image`, tooltip path
- [x] CAP-094 File tile: name, size B/KB/MB, no size for 0
- [x] CAP-095 Copied-text tile
- [x] CAP-096 Remove × on hover/focus, refocus field
- [x] CAP-097 Drop outline 2 px accent
- [x] CAP-098 Clipboard offer key, dashed tile, take on click, mark offered key done
- [x] CAP-099 Paperclip → dialog → attach
- [x] CAP-100 Scan → attach / notice / nothing; refocus
- [x] CAP-101 Screenshot notice copy + Open System Settings
- [x] CAP-102 Error line and when it clears
- [x] CAP-103 Mic: append transcript, labels, disabled states, status replaces hint, Settings → capture:settings
- [x] CAP-104 Esc while mic busy cancels voice only
- [x] CAP-105 Hint `⇧↩` / `Shift+Enter` new line
- [x] CAP-106 Send button verbs, `Working it out…`, ↩ cap, disabled rules
- [x] CAP-107 Menu open/close rules (outside pointer-down, Esc, project/session pick)
- [x] CAP-108 Menu reason when guessed and non-empty
- [x] CAP-109 Make column: 4 kinds, hints, into disabled `No sessions are running`
- [x] CAP-110 Project column with check
- [x] CAP-111 Into column: ≤ 8 sessions incl. waiting (disabled, `Needs you`), empty line, other projects `<n> sessions` / `no sessions`
- [x] CAP-112 Menu footer Tab/Esc
- [x] CAP-113 Pick kind rules (into fallbacks, nothing when no sendable session)
- [x] CAP-114 Tab/⇧Tab cycle, into skipped when none sendable, only with text
- [x] CAP-115 Pick project (into → first sendable or new session)
- [x] CAP-116 Pick session
- [x] CAP-117 Permission/unavailable banner copy and Allow…
- [x] CAP-118 Targets polled every 2000 ms while visible
- [x] CAP-119 IME composition ignored
- [x] CAP-120 Return sends, ⇧Return new line
- [x] CAP-121 Esc closes menu, else hides
- [x] CAP-122 ↑/↓ history browsing rules
- [x] CAP-123 Send flow: wait for guess, checks, submit, history (50), offer done, clear, receipt, refresh, timer
- [x] CAP-124 Send error message (IPC prefix stripped); draft kept
- [x] CAP-125 Submit validation and errors in order; prompt = withScreen
- [x] CAP-126 Session: 720×480 spot, auto mode expanded, yellow note with typed text stuck on it
- [x] CAP-127 Into: session check, 03 send path, title from sessionTitle
- [x] CAP-128 Note: fitNote(prompt), yellow, w/h, prompt text
- [x] CAP-129 View: 480×360 spot, createView(description = prompt)
- [x] CAP-130 Remember, board-changed event, receipt shape
- [x] CAP-131 Rest after successful submit only
- [x] CAP-132 Failed submit leaves nothing, remembers nothing
- [x] CAP-133 spotOnBoard: viewport centre / right of right-most card + 32 / dock + 80; free spot; rounded
- [x] CAP-134 Window size or 1440×920
- [x] CAP-135 cleanAttachments shapes and limits (20, 4000, 500, 100, 20 000)
- [x] CAP-136 titleFrom 48 chars, sentence, word cut, `…`
- [x] CAP-137 describeAttachments wording and order
- [x] CAP-138 withScreen
- [x] CAP-139 Fence longer than any backtick run, min 3
- [x] CAP-140 Receipt replaces the page
- [x] CAP-141 Note receipt: sticky with 160-char text, caption `On <project>’s board · Open`
- [x] CAP-142 Receipt card: top line, title, body line, footer, `· auto mode`
- [x] CAP-143 Session sticker 70 chars, title padding 56
- [x] CAP-144 Open link clears receipt and opens card/board
- [x] CAP-145 `capture:open` hides then openCard
- [x] CAP-146 Receipt entrance 280 ms spring from 1.5× / +18 px / 0.4
- [x] CAP-147 Receipt hides after 2600 ms; re-show cancels
- [x] CAP-148 Session check filters and title fallbacks (`Untitled session`)
- [x] CAP-149 Targets: non-archived projects, sessions by activity, recents, default
- [x] CAP-150 Record → recent resolution rules (`New view` title)
- [x] CAP-151 Default project rule (capture vs openedAt vs last board)
- [x] CAP-152 Recent cards: mark, project, age, title, status, waiting ring, open
- [x] CAP-153 More pill with 4th/5th titles and ↑ hint
- [x] CAP-154 board:load records last board (memory only)
- [x] CAP-155 Capture record prepended, cap 20, saved
- [x] CAP-156 `captures` array read/write as is
- [x] CAP-157 Draft, attachments, history, clipboardDone persisted per machine
- [x] CAP-158 captures/<uuid>.png, never deleted except by the sweep
- [x] CAP-159 `capture:settings` hides and opens Settings in the board window
- [x] CAP-160 `capture:hide` hides
- [x] CAP-161 Hide during guess → null, previous guess kept
- [x] CAP-162 Send while hidden still makes it; receipt cleared on next show
- [x] CAP-163 Into a session that became waiting → error, text kept
- [x] CAP-164 Vanished into session → `a session`, then error on send
- [x] CAP-165 Banner stays after Allow… until next show
- [x] CAP-166 Restored tab blocks the new front tab
- [x] CAP-167 Already-seen clipboard not offered, across restarts
- [x] CAP-168 Opened over Nodus: no app, no tab, clipboard still
- [x] CAP-169 Archived project's recents show a blank name
- [x] CAP-170 Future createdAt shows `now`
- [x] CAP-171 Dropped directories as size-less file tiles
- [x] CAP-172 Draft thumbnails kept in page storage
- [x] CAP-173 Old subdirectory stops the sweep
- [x] CAP-174 macOS Dock-click quirk with the hidden panel (decide with 01)
- [x] CAP-175 Windows close-doesn't-quit quirk with the hidden panel (decide with 01)
- [x] CAP-176 One guess process at a time
- [x] CAP-177 No polling while hidden
