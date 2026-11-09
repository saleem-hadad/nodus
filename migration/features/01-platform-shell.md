# 01 · Platform & shell

Sources:
- `src/main/index.ts` (158 lines): app lifecycle, main window, single instance, userData override, quitting
- `src/main/menu.ts` (208): application menu, focus-menu sync, menu commands, bug-report URL
- `src/main/env.ts` (258) + `src/main/env.test.ts` (39): login-shell environment, `claude` lookup, spawn env, shells, Git Bash
- `src/main/store.ts` (202): `state.json` load/migrate/save/flush, board defaults, lookups
- `src/main/ipc.ts` (1044): plumbing (`registerIpc` order, `send`, `envReady`), handlers `app:*`, `settings:*`, `projects:*`, `groups:*`, `applySettings`, `addProject`, `pickFolderColor`, `shutdown()`; the rest only skimmed for the API table
- `src/preload/index.ts` (156): the `window.api` bridge, `pathForFile`
- `src/shared/types.ts` (714), `src/shared/api.ts` (422): the full command/event contract
- `src/renderer/src/platform.ts` (36) + `platform.test.ts` (65): Mac detection, modifier logic, shortcut labels
- `src/renderer/src/main.tsx` (30), `src/renderer/index.html` (15), `src/renderer/src/env.d.ts` (10)
- `safeStorage` call sites: `src/main/voice.ts:1-52` (only user of secrets)
- Context only (not owned): `package.json` / `electron-builder.yml` (productName `Nodus`, appId `dev.saleem.nodus`), `src/main/notify.ts`, `src/main/capture.ts`, `src/main/pty.ts`, `src/main/updater.ts`, `src/main/views.ts`, `src/main/discovery.ts`, `src/renderer/src/App.tsx`, `src/renderer/src/components/TitleBar.tsx`, `src/renderer/src/styles/tokens.css`
- Checked against an installed 0.9.4 (names, key sets and formats only, never values): the Keychain item's service and account, the `state.json` key sets, the key layout of Chromium's `Preferences` and `Local State`, and the Electron 44.5.1 / Chromium 152.0.7977.130 framework strings.

## Summary

This domain is everything between the OS and the other domains. It gives the process its identity (name `Nodus`, Windows AppUserModelID `dev.saleem.nodus`), resolves the data folder (`userData`, overridable with `NODUS_USER_DATA`), enforces one instance off a Mac, and starts the subsystems in a fixed order before the single main window opens. It owns that window's geometry, chrome and theme colors, the rule that the app never navigates away from itself, and the quit paths. Quitting always leaves sessions running in the session host, except **Quit and Stop Sessions**.

It owns the native application menu on macOS, and the same menu as a popup from the title bar's ☰ button on Windows. Menu shortcuts become *menu commands* sent to the UI. It owns `state.json`: the full schema, tolerant loading, a one-time migration from the app's pre-rename folder, a 300 ms debounced atomic save, and a synchronous flush on quit. It owns settings (normalize, apply, persist), projects and groups (add/update/remove/reveal), the login-shell environment capture, the lookup of the `claude` binary that every spawned process depends on, and the encrypted secret format (`safeStorage`) that the Rust app must decrypt. Last, it defines the command/event contract (`window.api`) that every other spec references, the IPC error string format the UI shows, and the renderer's platform helpers (`isMac`, `modKey`, `shortcutKey`, `keys`).

## Behaviors

### Identity and data folder

- **PLAT-001** The application name is `Nodus` (`app.setName('Nodus')`, also `productName`). It appears as the macOS app-menu title and in `About Nodus`, `Hide Nodus` and `Quit Nodus`.
- **PLAT-002** Windows only: at process start, before any window or notification, the process's explicit AppUserModelID is set to `dev.saleem.nodus`. The NSIS shortcut carries the same ID, so toasts are attributed to it (index.ts:118).
- **PLAT-003** Default userData folder = `<appData>/<productName>`: macOS `~/Library/Application Support/Nodus`, Windows `%APPDATA%\Nodus` (the Roaming AppData known folder). Electron computes it at init as `path.join(app.getPath('appData'), 'Nodus')` (verify in Electron `lib/browser/init.ts`).
- **PLAT-004** If env `NODUS_USER_DATA` is set and non-empty, userData is exactly that string, unnormalized (`app.setPath('userData', value)`). It applies before anything reads userData and before the single-instance lock. An empty string is ignored. unclear: a relative path makes Electron throw `Path must be absolute` (string present in the Electron 44 binary) at module load, so startup fails. Verify what 0.9.4 shows.
- **PLAT-005** The userData string the Rust app uses must equal 0.9.4's byte for byte. Spec 02 hashes `<userData>/host` into the host socket and pipe name (`hostPaths`), and 06 passes it to the relaunch.

### Single instance

- **PLAT-006** macOS: no single-instance lock. Re-opening the app through LaunchServices activates the running copy (PLAT-030). A second process started directly (binary, `open -n`, as the updater's relaunch does) runs as an independent instance.
- **PLAT-007** Windows (all non-Mac): at startup, after PLAT-004, the app requests a single-instance lock keyed to the userData folder. If another instance holds it, the new process exits immediately with code 0 (`app.exit(0)`). No quit handlers run and nothing else happens.
- **PLAT-008** The running instance, notified of a second launch, calls `showWindow()` (PLAT-023) if it has finished starting (`app.isReady()`). Before that it ignores the notification. The second launch's argv, cwd and extra data are ignored.

### Startup sequence

- **PLAT-009** Before ready, the scheme `nodus-view` is registered as privileged `{ standard: true, secure: true }`, so each view page is its own secure origin (spec 04 serves it).
- **PLAT-010** On ready, in this exact order:
  1. Register the `nodus-view` handler (`servePage`, 04).
  2. `loadState()` (PLAT-084 to PLAT-089).
  3. `applySettings(settings)` (PLAT-066), so the window opens in the chosen theme.
  4. Start the login-shell env capture without awaiting it (`envReady`; a failure logs `[env] init failed`).
  5. Configure the pty, views, notify and status hooks (02/04).
  6. `installMenu` (PLAT-037 onward).
  7. `updater.startUpdater` (06).
  8. **Await** `ptys.connectHost(false)`: adopt a running session host without starting one. A failure logs `[host] connect failed` and startup continues.
  9. `notify.armNotifications()` (02).
  10. Register every command handler, the board tools (04) and capture (07).
  11. Create the main window.
  12. Subscribe to theme changes (PLAT-021).
  13. Subscribe to `activate` (PLAT-030).
- **PLAT-011** The main window is not created until the host connect attempt (step 8) has settled, whether it succeeded or failed.
- **PLAT-012** The env capture never blocks the window. The commands marked ⏳env in the API table await it, and resolve only after the capture and the `claude --version` read have finished or failed.
- **PLAT-013** Every command handler is registered before the window is created, so the UI never calls a command that does not exist yet.

### Main window

- **PLAT-014** The window is 1440 × 920 logical px by default and at least 960 × 620. It is resizable, centered on screen (no x/y given) and titled `Nodus`. The page's `<title>` is also `Nodus`, and nothing changes it.
- **PLAT-015** Window bounds are not persisted. Every new main window opens at 1440 × 920, centered, neither maximized nor full screen.
- **PLAT-016** The window is created hidden and shown once its first frame is ready (`ready-to-show`, once), so it never shows a blank or flashing frame.
- **PLAT-017** macOS: the title bar style is `hiddenInset`. Content runs under a transparent title bar, no title text is drawn, and the traffic lights are visible at `trafficLightPosition { x: 18, y: 18 }` (pt from the window's top-left; verify against a screenshot). The app draws its own 52 px title bar (08).
- **PLAT-018** Windows: the title bar style is `hidden`, with a Window Controls Overlay. The minimize, maximize and close buttons are drawn over the right end of the app's title bar at height 52. Colors: light `{ color: '#f6f6f5', symbolColor: '#1f1f1f' }`, dark `{ color: '#252527', symbolColor: '#e6e6e6' }`. These equal `--bg-toolbar` composited over `--bg-window`. The app's title bar leaves the overlay room (08, TitleBar.css).
- **PLAT-019** The window background color is `#1e1e20` when the effective theme is dark and `#f5f5f4` when it is light (= `--bg-window`). Resizing never flashes the other theme.
- **PLAT-020** No vibrancy, transparency, custom shadow, corner radius or always-on-top: OS defaults for a standard window.
- **PLAT-021** On every change of the effective theme (the Settings theme, or the OS appearance while on `system`), the background color updates (PLAT-019). On Windows the overlay colors update too (PLAT-018).
- **PLAT-022** There is at most one main window. When it closes, the app drops its reference, and events sent while no window exists are dropped (PLAT-123).
- **PLAT-023** `showWindow()` creates the main window if there is none. If it is minimized it is restored first. It is then shown, which also focuses it, and returned. Callers: second launch (PLAT-008), notification clicks (02), capture's open and settings (07).

### Links and navigation

- **PLAT-024** Any request from the app's UI to open a new window (`window.open`, `target=_blank`, links clicked in terminals) is always denied in-app. If the URL matches `/^https?:/` (case-sensitive), it opens in the default browser. Any other scheme (`mailto:`, `file:`…) is silently ignored.
- **PLAT-025** The main window never leaves the app's own page. A navigation whose URL, ignoring the `#fragment`, differs from the app page's URL is cancelled. If it matches `/^https?:/`, it opens in the default browser instead. Fragment-only changes are allowed.
- **PLAT-026** A view's frame only ever shows its own page. Sub-frame navigations are cancelled unless the target starts with `nodus-view://`. When the frame is already on a `nodus-view://` page, they are also cancelled if the target's host (the view id) differs from the current host. Cancelled frame navigations are **not** opened in the browser. A frame not yet on a view page (initial load) may load any view page. Rust equivalent: each view's webview may load only `nodus-view://<its viewId>/…` (04).
- **PLAT-027** The command bridge (`window.api`) is exposed only to the app's own pages (main window and capture panel), never to view frames. The preload runs in the main frame only.

### Theme

- **PLAT-028** `settings.theme`: `system` follows the OS appearance, and `light`/`dark` force it. It applies to the app UI, the capture panel, native menus and dialogs, and the window chrome (traffic lights, caption overlay) (`nativeTheme.themeSource`). The effective theme is dark iff `theme == 'dark'` or (`theme == 'system'` and the OS is dark).

### Lifecycle and quitting

- **PLAT-029** macOS: closing the main window does not quit. The app keeps its menu bar and Dock icon, and sessions keep running.
- **PLAT-030** macOS: when the app is activated (Dock click, re-open from Finder) and **no window of any kind** exists, a new main window is created. QUIRK, to reproduce: the floating capture panel is created on first use and only hidden afterwards, so it counts as a window. Once the panel exists, a Dock click does not reopen a closed main window. The window comes back through `showWindow()` paths only (notification click, capture's open/settings).
- **PLAT-031** Windows: when every window has been closed, the app quits. QUIRK, to reproduce: the capture panel is never closed, so after it has been opened once, closing the main window leaves Nodus running with no window. Double-tap Shift still opens the panel, and a second launch shows the window (PLAT-008).
- **PLAT-032** Before quitting, on every path that emits `before-quit` (⌘Q / `Quit Nodus`, `Exit`, last window closed on Windows, the updater's restart, Dock › Quit, `Quit and Stop Sessions`), `shutdown()` runs synchronously in this order:
  1. `stopCapture()`: stops the key hook and the permission poller (07).
  2. `ptys.disconnect()`: lets go of the session host without stopping sessions (02).
  3. `unwatchAll()`: stops every transcript watcher (03).
  4. `views.flushViews()`: writes pending view runtime state and stops the views' ticker (04).
  5. `flush()`: writes a pending `state.json` save now (PLAT-095).
- **PLAT-033** Quitting never stops sessions. They keep running in the session host, and the next launch adopts them (PLAT-010 step 8).
- **PLAT-034** `Quit and Stop Sessions` calls `ptys.stopAll()`. If there is no host connection it resolves at once. Otherwise it sends `{ type: 'shutdown' }` to the host and waits until the connection closes or 5000 ms pass (`KILL_WAIT_MS` 4000 + 1000), whichever is first. Then `app.quit()` runs, even if stopping failed (`finally`).
- **PLAT-035** Windows logoff or shutdown: Electron does not emit `before-quit`, so `shutdown()` does not run, and a state change saved within the last 300 ms may be lost (Electron documented behavior; verify).
- **PLAT-036** With no main window, menu items that send menu commands do nothing; for example ⌘N, ⌘, and ⌘W are dropped. `Capture…` still opens the panel, `Check for Updates…` still runs its check (its `settings` command is dropped), and Quit, `Quit and Stop Sessions`, `Star on GitHub`, `Report a Bug…`, Hide, Services and the other native items still work.

### Application menu: structure

- **PLAT-037** macOS menu bar, in order: **Nodus** (app menu), **File**, **Edit**, **View**, **Window**, **Help**. Windows has no visible menu bar. The same menu, with Windows' item set (**File**, **Edit**, **View**, **Window**, **Help**), opens as a popup from the title bar's ☰ button (PLAT-060).
- **PLAT-038** macOS app menu (`Nodus`), in order:
  1. `About Nodus`
  2. `Check for Updates…`
  3. separator
  4. `Settings…` (⌘,)
  5. separator
  6. `Services` (system submenu)
  7. separator
  8. `Hide Nodus` (⌘H)
  9. `Hide Others` (⌥⌘H)
  10. `Show All`
  11. separator
  12. `Quit Nodus` (⌘Q)
  13. `Quit and Stop Sessions` (⌥⌘Q)
- **PLAT-039** macOS **File**:
  1. `Capture…` (no shortcut: the double tap of Shift can't be shown in a menu)
  2. separator
  3. `New Session` (⌘N)
  4. `New Terminal` (⌘T)
  5. `New Project…` (⇧⌘N)
  6. separator
  7. `Close` (⌘W)
- **PLAT-040** Windows **File**:
  1. `Capture…`
  2. separator
  3. `New Session` (Ctrl+N, shown, not registered)
  4. `New Terminal` (Ctrl+T, shown, not registered)
  5. `New Project…` (Ctrl+Shift+N, registered)
  6. separator
  7. `Close` (Ctrl+W, shown, not registered)
  8. separator
  9. `Settings…` (Ctrl+,, registered)
  10. `Check for Updates…`
  11. separator
  12. `Exit` (role quit, no shortcut)
  13. `Quit and Stop Sessions` (no shortcut: Ctrl+Alt is AltGr on many Windows keyboards)
- **PLAT-041** **Edit**, both platforms:
  1. `Undo` (⌘Z / Ctrl+Z)
  2. `Redo` (⇧⌘Z / Ctrl+Y)
  3. separator
  4. `Cut` (⌘X / Ctrl+X)
  5. `Copy` (⌘C / Ctrl+C)
  6. `Paste` (⌘V / Ctrl+V)
  7. macOS only: `Paste and Match Style` (⌥⇧⌘V). It is absent off a Mac, where Ctrl+Shift+V is a terminal's paste.
  8. `Delete` (no shortcut)
  9. `Select All` (⌘A / Ctrl+A)
  10. separator
  11. `Search Sessions…` (⌘K / Ctrl+K, shown, **never registered on either platform**: the board takes ⌘K itself so a focused terminal can keep it for clearing)

  Off a Mac, Undo, Redo, Cut, Copy, Paste and Select All show their shortcut but don't register it (PLAT-055).
- **PLAT-042** **View**, both platforms:
  1. `Focus Mode` (checkbox, ⇧⌘F / Ctrl+Shift+F, registered on both; PLAT-052)
  2. separator (hidden with Focus Mode)
  3. `Reload` (⌘R / Ctrl+R, the latter not registered)
  4. `Force Reload` (⇧⌘R / Ctrl+Shift+R, registered on both)
  5. `Toggle Developer Tools` (⌥⌘I / Ctrl+Shift+I, registered on both)
  6. separator
  7. `Actual Size` (⌘0 / Ctrl+0, the latter not registered)
  8. `Zoom In` (⌘+ / Ctrl+Plus, the latter not registered)
  9. `Zoom Out` (⌘- / Ctrl+-, the latter not registered)
  10. separator
  11. `Toggle Full Screen` (⌃⌘F / F11, registered on both)
- **PLAT-043** **Window**. On macOS it is the standard window menu: `Minimize` (⌘M), `Zoom`, separator, `Bring All to Front`, plus whatever AppKit appends (the window list and, on recent macOS, tiling items; verify). On Windows it holds only `Minimize` (Ctrl+M, shown, not registered). There is no `Close` item off a Mac, since its Ctrl+W would close the window.
- **PLAT-044** **Help**. On macOS: `Star on GitHub`, `Report a Bug…`. It is registered as the app's Help menu, so macOS adds its search field. On Windows: `Star on GitHub`, `Report a Bug…`, separator, `About Nodus`.
- **PLAT-045** macOS adds its own items to menus titled Edit and View (for example `AutoFill`, `Start Dictation…`, `Emoji & Symbols`, maybe `Show Tab Bar`). The Rust menu must end up with the same visible set as 0.9.4 (verify by listing 0.9.4's menus on macOS 26).

### Application menu: actions

- **PLAT-046** `Check for Updates…` sends the `settings` menu command (Settings shows how the check goes) and starts a user-asked update check (`check(true)`, 06).
- **PLAT-047** `Settings…` sends the `settings` command.
- **PLAT-048** `Capture…` opens the floating capture panel (`openPanel()`, 07). It works with no main window.
- **PLAT-049** `New Session` sends `new-session`, `New Terminal` sends `new-terminal`, `New Project…` sends `new-project`, `Close` sends `close`, and `Search Sessions…` sends `search`. `Close` never closes the window. What each command does is the UI's business (08, 09).
- **PLAT-050** `Star on GitHub` opens `https://github.com/saleem-hadad/nodus` in the default browser.
- **PLAT-051** `Report a Bug…` opens `https://github.com/saleem-hadad/nodus/issues/new?body=<form-encoded body>` in the default browser. The body format is in Data & formats › Bug-report URL.
- **PLAT-052** `Focus Mode` is visible **and** enabled iff `settings.focusFeature`, so its shortcut can't fire while it is hidden. The separator below it is visible iff `settings.focusFeature`. The item is checked iff `settings.focusMode`. On click, the tick is first reset to the current `settings.focusMode` (undoing the toggle the click made), then `toggle-focus` is sent. The tick changes only when the UI saves the new setting (PLAT-053).
- **PLAT-053** The Focus Mode visibility, enabled state and tick are re-synced from settings every time settings are applied (PLAT-066). The initial state comes from the loaded settings when the menu is built.
- **PLAT-054** `Quit Nodus` (macOS) and `Exit` (Windows) quit normally (PLAT-032). `Quit and Stop Sessions` follows PLAT-034.
- **PLAT-055** Off a Mac, Ctrl plus a letter belongs to a focused terminal. So `New Session`, `New Terminal`, `Close`, `Undo`, `Redo`, `Cut`, `Copy`, `Paste`, `Select All`, `Reload`, `Actual Size`, `Zoom In`, `Zoom Out` and `Minimize` show their shortcut but don't register it. The window takes Ctrl+N, Ctrl+T and Ctrl+W outside terminals (08, App.tsx), and text inputs and terminals do their own editing keys. Registered off a Mac: Ctrl+Shift+N, Ctrl+,, Ctrl+Shift+F (while offered), Ctrl+Shift+R, Ctrl+Shift+I and F11. On macOS every listed shortcut is registered except ⌘K.
- **PLAT-056** The edit items `Undo`, `Redo`, `Cut`, `Copy`, `Paste`, `Paste and Match Style` (paste as plain text) and `Delete` (delete the selection) act on whatever has keyboard focus: a text field, or a terminal (12). `Select All` selects all in the focused field or terminal.
- **PLAT-057** Web-specific View items, as they behave in 0.9.4. **Decision needed** for the Rust app (see Rust mapping):
  - `Reload` and `Force Reload` restart the whole UI from scratch: route, modals and in-memory UI state reset. Sessions keep running and terminals reattach with replay.
  - `Toggle Developer Tools` opens Chromium DevTools for the main window.
  - `Actual Size`, `Zoom In` and `Zoom Out` set the page zoom level to 0, +0.5 and −0.5 levels from the current one. Zoom factor = 1.2^level. The zoom is persisted (PLAT-139), and the capture panel shares it (same page URL; verify).
  - `Toggle Full Screen` toggles native full screen.
- **PLAT-058** `About Nodus`. macOS shows the standard About panel from Info.plist (icon, `Nodus`, version, copyright; verify the exact text on a 0.9.4 build). Windows shows a modal information message box. Electron builds its text from the .exe's FileDescription and ProductVersion, each followed by CRLF; verify the exact text and title on Windows 0.9.4.
- **PLAT-059** `Hide Nodus`, `Hide Others`, `Show All`, `Services`, `Minimize`, `Zoom` and `Bring All to Front` are the native macOS actions.

### Menu commands and the ☰ popup

- **PLAT-060** `app:show-menu(x, y)` does nothing when there is no main window or x or y is not a finite number. Otherwise it opens the **whole application menu** as a popup in the main window at (`round(x)`, `round(y)`), in window-content logical coordinates. The title bar's ☰ button (Windows only, 08) passes its left edge and its bottom + 4.
- **PLAT-061** `app:menu-command(command)`. If `command` is one of the seven MenuCommands (PLAT-062), the `menu` event carrying it is sent to the main window. Anything else is silently ignored, and the call resolves either way. The UI uses it for the shortcuts a terminal would otherwise take off a Mac: Ctrl+N/T/W outside terminals (App.tsx) and Ctrl+Shift+W/T inside terminals (12).
- **PLAT-062** MenuCommand values (exact strings): `close`, `new-session`, `new-terminal`, `new-project`, `settings`, `search`, `toggle-focus`.

### Settings

- **PLAT-063** Normalizing settings fills in defaults and drops unknown keys. The input is any value, and non-objects count as `{}`.
  - `theme`: kept if it is one of `system`, `light`, `dark`, else `system`.
  - `autoUpdate`, `notify`, `dockBadge`, `capture`: `true` unless the value is exactly `false`.
  - `pinProjectRail`, `focusFeature`, `focusMode`: `true` only if the value is exactly `true`.

  Output key order: `theme, autoUpdate, notify, dockBadge, pinProjectRail, focusFeature, focusMode, capture`.
- **PLAT-064** `settings:get` returns the current settings object.
- **PLAT-065** `settings:update(patch)`:
  1. settings = normalize({…current, …patch}).
  2. `applySettings`.
  3. Schedule a save (PLAT-093).
  4. If `patch.autoUpdate` is truthy, start a background update check (`check()`, not user-asked, 06).
  5. Return the full settings.

  An invalid `theme` in a patch resets the theme to `system`. A non-boolean value is coerced as PLAT-063 says (for example `autoUpdate: 0` becomes `true`).
- **PLAT-066** `applySettings(settings)`, run once at startup before the window exists and after every update:
  1. Set the theme source (PLAT-028).
  2. `notify.updateBadge()`: recount the Dock badge, which does nothing until notifications are armed (02).
  3. `syncFocusMenu` (PLAT-053), which does nothing before the menu is installed.
  4. `syncShortcut()`: start or stop capture's key hook (07), which does nothing before capture is registered.

### Projects

- **PLAT-067** `projects:list` returns the stored projects array in stored order, archived ones included.
- **PLAT-068** `projects:add(repoPath?)` without a path (undefined or empty) opens a native folder picker:
  - title `Add Project`
  - button label `Add Project`
  - message `Choose the local repository for this project` (shown on macOS only)
  - folders only, with a New Folder button (macOS)

  The picker is window-modal (a sheet on macOS) when the main window exists, and app-modal otherwise. Cancel or no selection resolves `null`. Otherwise, and when a path was given, the result is `addProject(path).project`.
- **PLAT-069** `addProject(folder, name?)` resolves `real = realpath(folder)`, using Node's `fs.realpathSync` semantics (Rust mapping, risk R2). If that fails it throws `Folder not found: <folder>` (the input as given). If `real` is not a directory it throws `Not a folder: <real>`.
- **PLAT-070** "Focusing" means `settings.focusFeature && settings.focusMode && some project has focused == true && archived != true` (the same as `isFocusing` in the renderer's focus.ts).
- **PLAT-071** An existing project is the first one, in stored order, where `realpath(p.repoPath)` equals `real`. If realpath of the stored path fails, the stored string is compared as is.
  - If it is archived, or focusing is on and it isn't focused: `archived` is removed, `focused = true` is set when focusing, and a save is scheduled.
  - It is returned with `added: false`, and the IPC call returns that project.
- **PLAT-072** A new project, with keys in this order:
  - `id`: random UUID v4, lowercase, hyphenated
  - `name`: `name.trim()` if non-empty, else the basename of `real`
  - `repoPath`: `real`
  - `color`: from PLAT-073
  - `createdAt`: now in ms
  - `focused`: `true` only when focusing, and absent otherwise

  It is appended to the projects array, a save is scheduled, and it is returned with `added: true`.
- **PLAT-073** The folder color is random among the colors used by the fewest existing projects. Archived projects count. Colors are taken in `FOLDER_COLORS` order, and the pick is uniform among the tied ones.
- **PLAT-074** `projects:update(id, patch)`. The project must exist, or the call throws `Unknown project <id>`. Changes apply in this order:
  1. `name`: trimmed, applied only if non-empty.
  2. `color`: applied only if it is one of `FOLDER_COLORS`.
  3. `archived: true` sets `archived: true`. `archived: false` **deletes** the key.
  4. `focused`: `true` sets, `false` deletes.
  5. `groupId: null` deletes `groupId`. A truthy `groupId` must name an existing group, or the call throws `Unknown group <id>`. An empty string is ignored.
  6. Schedule a save and return the project.

  If `groupId` throws, the earlier changes stay in memory without a save scheduled, and they are written by the next save of anything.
- **PLAT-075** `projects:remove(id)`, in order:
  1. Kill every running process of the project (02).
  2. Stop its transcript watcher (03).
  3. Delete the `views/<viewId>` folders of its board's views (04).
  4. Remove it from projects and delete `boards[id]`.
  5. Schedule a save.

  An unknown id only schedules a save. Groups, `captures` records and the transcripts on disk are untouched.
- **PLAT-076** `projects:reveal(id)` shows the project folder selected in Finder or Explorer (`shell.showItemInFolder(repoPath)`). Unknown id: `Unknown project <id>`.
- **PLAT-077** `projects:stats` returns `{ [projectId]: projectStats(project) }` for every project (computed by 03).
- **PLAT-078** `projects:add` and `projects:update` don't emit `projectsChanged`; the UI refreshes itself. Only the board tools' `create_project` emits it (04). That path calls `addProject` with a `~`-expanded absolute path, creating the folder first if it is missing.

### Groups

- **PLAT-079** `groups:list` returns the stored groups array in stored order.
- **PLAT-080** `groups:create(name)` creates `{ id: uuid v4, name: name.trim() || 'New Group', createdAt: now }`, appends it, schedules a save and returns it.
- **PLAT-081** `groups:update(id, patch)`. The group must exist, or the call throws `Unknown group <id>`. `name` is trimmed and applied if non-empty. `collapsed: true` sets the key and `collapsed: false` deletes it. Then a save is scheduled and the group returned.
- **PLAT-082** `groups:remove(id)` removes the group and deletes `groupId` from every project that had it, then schedules a save. An unknown id is a no-op apart from the save.

### State store

- **PLAT-083** All persistent app state is one JSON file, `<userData>/state.json`, held in memory as a single object. The app is its only writer, and every save rewrites the whole file.
- **PLAT-084** When `state.json` doesn't exist at load, the app looks for `<parent of userData>/<OLD_APP_NAME>/state.json`. If it parses, its `voiceKey` is deleted and the rest is written to `<userData>/state.json`: create the folder, then a direct write (not atomic), 2-space JSON. Any failure (missing file, bad JSON) is silently ignored. The saved voice key stays behind because the old Keychain entry can't be read. This runs at every launch, but only acts when `state.json` is missing.
- **PLAT-085** Loading reads `state.json` and keeps only these top-level fields:
  - `projects` if it is an array, else `[]`
  - `groups` if an array, else `[]`
  - `boards` if truthy and of type object (an array passes too), else `{}`
  - `settings` normalized (PLAT-063)
  - `actions` if an array, else absent
  - `voiceKey` if a non-empty string, else absent
  - `captures` if an array, else absent

  Resulting key order: `projects, groups, boards, settings, actions, voiceKey, captures` (absent keys omitted).
- **PLAT-086** If the file is missing or unreadable, isn't valid JSON, or is `null`, state is the defaults `{ projects: [], groups: [], boards: {}, settings: <defaults> }`. The next save overwrites the bad file. No backup is made.
- **PLAT-087** Entries inside `projects`, `groups`, `boards`, `actions` and `captures` are kept verbatim, **including fields this version doesn't know**, and written back unchanged. Only `settings` is normalized at load.
- **PLAT-088** Top-level keys other than the seven in PLAT-085 are dropped from memory at load, so the next save removes them from the file.
- **PLAT-089** Nothing is validated per entry at load: a malformed entry stays and may fail later where it is used.
- **PLAT-090** `getProject(id)` throws `Unknown project <id>` and `getGroup(id)` throws `Unknown group <id>`. Most handlers in other domains surface these.
- **PLAT-091** `getBoard(projectId)`, when `boards[projectId]` is missing, creates `{ nodes: {}, archive: { x: 0, y: 0, w: 1928, h: 320 }, hidden: [] }` in memory. 1928 = 2·40 + 6·288 + 5·24: six 288 px card columns, 24 px gaps, 40 px side padding. On every call it backfills missing `hidden: []`, `sections: {}`, `notes: {}` and `views: {}` in place. It never backfills `ink` or `viewport`. It does **not** schedule a save, so the backfilled keys are written by the next save of anything.
- **PLAT-092** `projectIdForView(viewId)` is the first board, in `boards` key order, whose `views` has `viewId` as an own key. `projectIdForSession(sessionId)` is the first board whose `nodes[sessionId]` is truthy. Both return `null` when nothing matches.
- **PLAT-093** `save()` is debounced: each call restarts a 300 ms timer, and when the timer fires the state is written (PLAT-094). A write error logs `[store] save failed <err>`, and the app carries on with its state in memory.
- **PLAT-094** A write creates the userData folder (recursively), writes `JSON.stringify(state, null, 2)` to `<userData>/state.json.<pid>.tmp`, then renames it over `state.json`, replacing it atomically. There is no fsync and no trailing newline.
- **PLAT-095** `flush()` writes synchronously, and only when a debounced save is pending. Otherwise it does nothing. An error logs `[store] flush failed <err>`. It runs only from `shutdown()`.

### Login-shell environment and the `claude` binary

- **PLAT-096** Until the capture finishes, the base environment is the app's own process environment, string values only. On Windows, every key whose upper case is `PATH` is folded into a single `PATH` key: the first one found wins, later ones are dropped. Elsewhere keys are kept as they are.
- **PLAT-097** macOS (all non-Windows): the login-shell capture runs `$SHELL` (the app's own `SHELL`, or `/bin/zsh` if empty) with args `-ilc` and the script
  `printf '%s' '__NODUS_ENV_START__'; env -0; printf '%s' '__NODUS_ENV_END__'`.
  It runs with the app's environment, a 10 000 ms timeout and an 8 MiB output cap. The exit code and errors are ignored (interactive shells may print noise or exit non-zero).

  Parsing:
  1. Take the text between the **first** `__NODUS_ENV_START__` and the **last** `__NODUS_ENV_END__`. If either marker is missing, or END isn't after START, the capture is null.
  2. Split it on NUL.
  3. Each entry with `=` at an index > 0 becomes key = text before the first `=`, value = the rest.
  4. If no entries come out, the capture is null.
- **PLAT-098** Windows: no capture. The app's environment is already the user's.
- **PLAT-099** After the capture, base env = process env (PLAT-096), overlaid key by key with the captured env when there is one. Keys only in the process env are kept.
- **PLAT-100** Fallback PATH: `PATH` is split on the OS delimiter (`:` / `;`), with empty parts dropped. Each fallback directory not already present (exact string match) is appended in order, then the list is re-joined.
  - macOS: `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`, `/usr/sbin`, `/sbin`. `~` is the OS home directory.
  - Windows: `<home>\.local\bin`; `%LOCALAPPDATA%\Microsoft\WinGet\Links` if `LOCALAPPDATA` is set; `%APPDATA%\npm` if `APPDATA` is set.
- **PLAT-101** `findOnPath(bins, PATH)` searches **each name across every PATH directory before the next name**. Empty directories are skipped. The candidate is `join(dir, name)`, and it counts if it is executable (`X_OK`; on Windows this is mere existence) and stat, following symlinks, says it is a regular file. It returns the candidate path as joined, **not** symlink-resolved, or null.
- **PLAT-102** `claude` lookup on macOS: `findOnPath(['claude'])`.
- **PLAT-103** `claude` lookup on Windows: `findOnPath(['claude.exe', 'claude.cmd'])`, so a `claude.exe` anywhere on PATH beats any `claude.cmd`. If the result ends in `.cmd` (case-insensitive), the shim is read as UTF-8 and `exeBehindShim` is applied. The result is that exe if it exists, else **null**: a shim that runs `node … cli.js` means Claude is "not found". A read error also gives null.
- **PLAT-104** `exeBehindShim(shim, text)`: the first match of `/"%~?dp0%?\\([^"]+?\.exe)"/i` captures a relative exe path. With no match, or a target matching `/(^|\\)node\.exe$/i`, it returns null. Otherwise it returns Windows-joined `dirname(shim)` + target.
- **PLAT-105** The `claude` version comes from running `<claudePath> --version` with the spawn env (PLAT-107), a 10 000 ms timeout and no console window on Windows. On error it is null. Otherwise it is the first line of the trimmed stdout (split on `\r?\n`), or null if that is empty.
- **PLAT-106** The capture, the lookup and the version read happen **once per launch**. Installing Claude while the app runs needs a restart (see the PLAT-112 message).
- **PLAT-107** `spawnEnv(extra)` builds the environment for every spawned process:
  1. Start from the base env minus the stripped keys. Exact keys stripped: `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_EXECPATH`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_PID`, `CLAUDE_EFFORT`, `ELECTRON_RUN_AS_NODE`. Prefixes stripped: `CLAUDE_CODE_SESSION`, `CLAUDE_CODE_MESSAGING`, `CLAUDE_CODE_SSE`, `CLAUDE_AGENT_SDK`, `NODUS_`, `ELECTRON_`.
  2. Set `TERM=xterm-256color` and `COLORTERM=truecolor`, always overriding.
  3. Set `LANG=en_US.UTF-8` only if LANG is unset or empty.
  4. Apply `extra` last, overriding everything.
- **PLAT-108** `envVar(name)` returns the **unstripped** base env value, for example `XAI_API_KEY` (05) or `CLAUDE_CODE_GIT_BASH_PATH`.
- **PLAT-109** `userShell()`:
  - Windows: `pwsh.exe` found on PATH, else `<SystemRoot>\System32\WindowsPowerShell\v1.0\powershell.exe`, where SystemRoot is `SystemRoot`, then `SYSTEMROOT`, then `C:\Windows`.
  - macOS: base `SHELL`, else the app's `SHELL`. It is used only if it is an absolute path, else `/bin/zsh`.
- **PLAT-110** `terminalShell()` returns `{ file: userShell(), args }` with args `['-l']` (a login shell, as Terminal.app starts one) or `['-NoLogo']` on Windows.
- **PLAT-111** `shellCommand(run)`:
  - macOS: `{ userShell(), ['-c', run] }`.
  - Windows: `{ userShell(), ['-NoProfile', '-NonInteractive', '-Command', '[Console]::OutputEncoding = [Text.Encoding]::UTF8; ' + run] }`.
- **PLAT-112** `requireClaude()` returns the path, or throws:
  - macOS: `Claude Code CLI not found. Install it (npm i -g @anthropic-ai/claude-code) and restart Nodus.`
  - Windows: `Claude Code CLI not found. Install it (irm https://claude.ai/install.ps1 | iex, in PowerShell) and restart Nodus.`
- **PLAT-113** `gitBash()` returns null off Windows. On Windows the result of the first call is cached for the life of the process, even if that call came before the capture finished:
  1. If `CLAUDE_CODE_GIT_BASH_PATH` (base env) is set, return it if it exists, else null, with no fallback.
  2. Otherwise build the candidates. For `git.exe` found on PATH, for each of its first 3 ancestor directories `dir` (dirname, then the next two parents), add `dir\bin\bash.exe`. Then add `%ProgramFiles%\Git\bin\bash.exe`, `%ProgramFiles(x86)%\Git\bin\bash.exe` and `%LOCALAPPDATA%\Programs\Git\bin\bash.exe`, each only when its variable is set.
  3. Return the first candidate that exists, else null.
- **PLAT-114** `app:info` awaits the env and returns `{ claudePath, claudeVersion, platform }`. `platform` takes Node's values (`darwin`, `win32`, `linux`). No 0.9.4 UI calls it, but the command must exist.

### Secrets (`safeStorage`)

- **PLAT-115** The only secret is the xAI key for voice prompts. It is stored as `state.voiceKey`: the standard base64 (with padding) of Electron `safeStorage.encryptString(key.trim())`. It is never sent to the UI. The save, forget and source logic belongs to 05.
- **PLAT-116** macOS ciphertext = ASCII `v10` + AES-128-CBC(PKCS#7) of the UTF-8 plaintext.
  - Key: PBKDF2-HMAC-SHA1(password = the Keychain password **as its UTF-8 string bytes**, salt = `saltysalt`, iterations = 1003, length = 16 bytes).
  - IV: 16 × 0x20 (space).
  - Confirmed on 0.9.4 data: the stored blob starts with `v10`, and its length after the prefix is a multiple of 16. The `saltysalt` and 16-space IV literals are in the Electron 44 binary. Iterations 1003 and SHA1: verify in Chromium `components/os_crypt/sync/os_crypt_mac.mm` (Chromium 152).
- **PLAT-117** The macOS Keychain item is a generic password in the login keychain with service `Nodus Safe Storage` and account `Nodus Key` (both names confirmed on an installed 0.9.4). Chromium creates it on first use. Its password is a random string: Chromium generates 16 random bytes, base64 encoded, so 24 characters (verify in `keychain_password_mac.mm`).
- **PLAT-118** Windows ciphertext = ASCII `v10` + 12-byte nonce + AES-256-GCM ciphertext + 16-byte tag.
  - The 32-byte key is in `<userData>\Local State` (JSON) at `os_crypt.encrypted_key`: base64 of ASCII `DPAPI` + `CryptProtectData(key)` (user scope, no entropy).
  - So DPAPI protects the key, not the string. Electron's `decryptString` rejects any blob without a `v10`/`v11` prefix (`Ciphertext does not appear to be encrypted.`).
  - Not confirmable on this Mac: verify in Chromium `components/os_crypt/sync/os_crypt_win.cc` (Chromium 152) against a Windows 0.9.4 install.
- **PLAT-119** If encryption isn't available when a key is saved, saving fails with `The Keychain isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` (macOS) or `Encrypted storage isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` (Windows). The text is owned by 05 and listed here for the secrets API.
- **PLAT-120** Decrypting is lenient base64 (Node `Buffer.from(…, 'base64')`) followed by decrypt. On failure it logs `[voice] could not decrypt the saved key` and acts as if no key were saved. An empty plaintext counts as no key. The source still reads `settings`, because the field is present (05).
- **PLAT-121** By ground rule 3, the Rust app writes the same format, the same Keychain item and the same `Local State` key, so a 0.9.4 reinstall can still read a key saved by the Rust app.

### Command bridge and IPC plumbing

- **PLAT-122** The UI reaches the core only through the commands and events in the API table (API surface). Names, argument order, return shapes and rejection messages are the contract other specs reference.
- **PLAT-123** Every event goes **only to the main window**, and is dropped if the window doesn't exist or is destroyed. Nothing is queued, except the open-card request (02, `takeOpenRequest`) and the settings command capture defers until the window loads (07). The two capture events go **only to the capture panel**.
- **PLAT-124** UI-visible error format. A command that throws makes the UI's promise reject with an Error whose message is `Error invoking remote method '<channel>': <err.toString()>`. For an `Error` that is `Error: <message>` (`TypeError: …` for type errors). Many UI toasts show this whole string. For example `Couldn't start a session: Error invoking remote method 'session:create': Error: Claude Code CLI not found. …` (BoardView.tsx:1249). Other places strip the prefix with `/^Error invoking remote method '[^']+': (Error: )?/` (SettingsDialog, CaptureApp, ViewNode, ViewDialog, viewRelay). The core's error type must carry the channel name and error name, so the UI can render both forms exactly. The main process also logs `Error occurred in handler for '<channel>': <err>` (verify the exact format in Electron).
- **PLAT-125** Fire-and-forget commands have no reply and can't reject: `session:write`, `session:resize`, `views:report`, `capture:hide`, `capture:resize`. Keystrokes and resizes go this way to keep typing latency low.
- **PLAT-126** `pathForFile(file)` gives the absolute path of a file dropped from the OS, or `''` when it has none (e.g. dragged from a browser). It is used for drops on the projects grid, terminals and the capture panel. In Rust, OS drops arrive as paths.
- **PLAT-127** Every `on.*` subscription returns an unsubscribe function. Any number of listeners may subscribe to the same event, and each gets every event.
- **PLAT-128** Commands that await the env (⏳env in the table) wait for PLAT-097 to PLAT-105 to finish. A failed capture still resolves (logged `[env] init failed`).
- **PLAT-129** Command arguments are structured-cloned. `undefined` fields arrive absent, a `Uint8Array` arrives as bytes (`voice:transcribe`), and functions and DOM objects can't be sent.

### Renderer platform helpers (platform.ts)

- **PLAT-130** `isMac` is true when running on macOS (`/^Mac/` on `navigator.platform`). `platform` is `'mac'`, `'windows'` (`/^Win/`) or `'linux'`.
- **PLAT-131** `modKey(e)` is `e.metaKey` on a Mac and `e.ctrlKey` elsewhere. `otherModKey(e)` is the opposite (`ctrlKey` on a Mac, `metaKey` elsewhere): a shortcut must not have it.
- **PLAT-132** `shortcutKey({ key, keyCode })` gives the lowercase shortcut key on any layout. If `key` is a single ASCII letter, or `keyCode` < 65 or > 90, it returns `key.toLowerCase()`. Otherwise it returns the letter of `keyCode`, lowercased. So Arabic `ر` with keyCode 86 gives `v`, and Russian `с` with 67 gives `c`.
- **PLAT-133** `keys(mac)` writes a Mac-style shortcut the way this platform does. On a Mac it is unchanged. Elsewhere, each code point is mapped: `⌘`→`Ctrl`, `⌃`→`Ctrl`, `⌥`→`Alt`, `⇧`→`Shift`, `↩`→`Enter`, `⌫`→`Del`, others unchanged. The key is the non-modifier parts joined with `''`. The output is the modifiers present, in the order `Ctrl`, `Alt`, `Shift` (each at most once), then the key if non-empty, joined with `+`. Examples: `⇧⌘F` → `Ctrl+Shift+F`, `⌘↩` → `Ctrl+Enter`, `⌘` → `Ctrl`, `⌫` → `Del`, `↩` → `Enter`, `⌥⌘K` → `Ctrl+Alt+K`.

### Renderer entry

- **PLAT-134** The root element carries `data-platform="mac" | "windows" | "linux"`. CSS uses it to leave room for the traffic lights or for Windows' buttons (08).
- **PLAT-135** One page serves two windows. If the URL hash is `#capture`, it renders the capture panel app (07). Otherwise it renders the main app (08). The capture panel loads `index.html#capture`.
- **PLAT-136** The app page's Content-Security-Policy, kept for reference: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; frame-src nodus-view:; object-src 'none'; base-uri 'none'; form-action 'none'`. Natively, the only part that carries over is that embedded frames may only be `nodus-view:` pages (PLAT-026).

### Behaviors Chromium provides

- **PLAT-137** Spellcheck is on (Electron's default). Text fields not marked `spellCheck={false}` underline misspelled words: the macOS system spellchecker, or Hunspell on Windows (dictionaries per `Preferences` › `spellcheck.dictionaries`, e.g. `["<locale>"]`). Which fields opt in or out belongs to their own specs; the capture text box and the Sidekick prompt explicitly opt in.
- **PLAT-138** There is no native right-click menu anywhere: the main process never handles `context-menu`. Right-clicking a text field shows nothing unless the UI draws its own menu (09 to 11).
- **PLAT-139** The page zoom from PLAT-057 persists across launches. Chromium stores it in `<userData>/Preferences` at `partition.per_host_zoom_levels.<partition hash>["file:///…/out/renderer/index.html"]`. A user who zoomed has a non-zero level there; for example −0.5 is about 91.3 %. The Rust app must start at the same UI scale (Data & formats › Chromium files).
- **PLAT-140** The UI keeps a few conveniences in Chromium `localStorage` (origin `file://`): `capture.draft`, `capture.attachments`, `capture.clipboardDone`, `capture.history` (07) and `nodus.inkColor` (11). They survive restarts in 0.9.4. The Rust app needs a home for them, plus a migration or an accepted loss (decision).
- **PLAT-141** Other Chromium files in userData (`Cache`, `Code Cache`, `GPUCache`, `Dawn*Cache`, `DIPS*`, `Session Storage`, `Shared Dictionary`, `Trust Tokens*`, `blob_storage`, `Network Persistent State`, `declarative_performance_observer.db*`) belong to the engine. The Rust app must not depend on them, and must not delete `Local State` (it holds the Windows key, PLAT-118), `Preferences` (PLAT-139) or `Local Storage` (PLAT-140) before migrating from them.

## Data & formats

### userData layout

| Path (under userData) | Format | Owner |
| --- | --- | --- |
| `state.json` | JSON, schema below | 01 |
| `state.json.<pid>.tmp` | transient write target, renamed over `state.json` | 01 |
| `host/` (`host.sock` or a pipe, `host.log`, `hook.curlrc`, …) | session host files (`hostPaths`) | 02 |
| `views/<viewId>/` | a view's page, data and runtime | 04 |
| `captures/` | screenshots and copied images from capture | 07 |
| `updates/` | downloaded updates | 06 |
| `Local State` | Chromium JSON; Windows: `os_crypt.encrypted_key` | 01 (read only) |
| `Preferences` | Chromium JSON; page zoom, spellcheck dictionaries | 01 (read only) |
| `Local Storage/leveldb/` | Chromium localStorage for `file://` | 01 (read only) |

Default userData: macOS `~/Library/Application Support/Nodus`, Windows `%APPDATA%\Nodus`. Override: `NODUS_USER_DATA` (non-empty). The old app's folder is `<parent of userData>/<OLD_APP_NAME>/state.json`, where `OLD_APP_NAME` is the pre-rename product name, spelled exactly as the literal at store.ts:87 (keep it out of docs and user-facing text).

### state.json writing rules

- `JSON.stringify(state, null, 2)`: 2-space indent, `"key": value` with one space after the colon, LF line endings, no trailing newline, UTF-8. Non-ASCII is written raw, including U+2028/U+2029. Control characters use short escapes (`\n`, `\t`, `\r`, `\b`, `\f`), others `\u00xx` in lowercase hex. `"` and `\` are escaped.
- Numbers are JS numbers. Integers have no fraction (`1`, not `1.0`). Other values use the shortest round-trip form (`0.30000000000000004`). Magnitudes ≥ 1e21 use the exponent form (`1e+21`), and `-0` is written `0`. Coordinates may be fractional, since board x/y are saved as the UI sends them.
- Keys absent or `undefined` are omitted. `null` is written only where code stores null. Key order is JS insertion order: integer-like keys come first in ascending order (no current ids are integer-like).
- Every save rewrites the whole object.

### state.json schema

Top level (`State`):

| Field | Type | Default / when absent | Notes |
| --- | --- | --- | --- |
| `projects` | `Project[]` | `[]` | stored order = add order |
| `groups` | `ProjectGroup[]` | `[]` | |
| `boards` | `{ [projectId]: BoardState }` | `{}` | created lazily (PLAT-091) |
| `settings` | `AppSettings` | all defaults | normalized at load |
| `actions` | `ProjectAction[]` | absent | global actions; key deleted when the list empties (05) |
| `voiceKey` | string | absent | base64 `safeStorage` ciphertext (PLAT-115) |
| `captures` | `CaptureRecord[]` | absent | newest first, at most 20 (07) |

`AppSettings`:

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `theme` | `'system' \| 'light' \| 'dark'` | `'system'` | PLAT-028 |
| `autoUpdate` | bool | `true` | download in the background, install on quit (06) |
| `notify` | bool | `true` | notifications while in the background (02) |
| `dockBadge` | bool | `true` | waiting count on the Dock icon (02) |
| `pinProjectRail` | bool | `false` | keep the project rail out beside the board (08) |
| `focusFeature` | bool | `false` | offer focus mode (switch, menu item, Add to Focus) |
| `focusMode` | bool | `false` | the Focus switch: only focused projects show |
| `capture` | bool | `true` | double-tap Shift opens floating capture (07) |

`Project`:

| Field | Type | Req | Notes |
| --- | --- | --- | --- |
| `id` | string (uuid v4) | ✓ | |
| `name` | string | ✓ | trimmed, non-empty |
| `repoPath` | string | ✓ | realpath at add time (absolute) |
| `color` | FolderColor | ✓ | `blue, purple, pink, red, orange, yellow, green, graphite` |
| `createdAt` | number (ms) | ✓ | |
| `openedAt` | number (ms) | | set when its board loads (03, board.ts:154) |
| `archived` | `true` | | deleted rather than set false |
| `groupId` | string | | |
| `focused` | `true` | | deleted rather than set false |
| `actions` | `ProjectAction[]` | | the project's own actions, deleted when empty (05) |

`ProjectAction` (05): `id` string, `name` string, `icon` string (one of `ACTION_ICONS`, src/shared/actions.ts), `description` string, `prompt` string, `global?` bool, `createdAt` number.

`ProjectGroup`: `id` string, `name` string, `createdAt` number, `collapsed?` `true` (deleted rather than set false).

`BoardState`:

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `nodes` | `{ [sessionId]: BoardNode }` | `{}` | |
| `archive` | `Rect` | `{x:0,y:0,w:1928,h:320}` | archive area (09) |
| `sections` | `{ [id]: BoardSection }` | `{}` (backfilled) | |
| `notes` | `{ [id]: BoardNote }` | `{}` (backfilled) | |
| `views` | `{ [id]: BoardView }` | `{}` (backfilled) | |
| `ink` | `InkStroke[]` | absent | oldest first (04/11) |
| `viewport` | `Viewport` | absent | |
| `hidden` | `string[]` | `[]` | session ids removed from the board, never re-imported |

`BoardNode` (03 owns semantics): `sessionId` string; `kind?` `'claude'|'terminal'` (absent = claude); `x`, `y` number; `archived` bool (required); `archivedAt?` number; `title?` string; `parents?` string[]; `createdByApp?` bool; `sectionId?` string; `expanded?` `true`; `w?`, `h?` number (clamped 200..4000, rounded); `runOf?` string (action id); `viewOf?` string (view id); `fromView?` string; `autoMode?` `true`; `createdAt` number.

`BoardSection` (03/09): `id`, `name` (may be `''`), `color` FolderColor (default `blue`), `x`, `y`, `createdAt`.

`BoardNote` (04/10): `id`, `text` (≤ `NOTE_MAX_CHARS`), `color` FolderColor (default `yellow`), `x`, `y` (rounded), `w?`, `h?` (clamped by `clampNoteSize`), `sessionId?`, `from?`, `fromProject?`, `fromTitle?`, `createdAt`.

`BoardView` (04): `id`, `title`, `description`, `x`, `y`, `w?`, `h?`, `sessionId?`, `from?`, `commands?: ViewCommand[]`, `allowedRuns?: string[]`, `deniedRuns?: string[]`, `refresh?: ViewRefresh`, `startsSessions?: bool`, `version` number (0 until there's a page), `createdAt`, `updatedAt`.
`ViewCommand`: `name`, `run`, `every?` (minutes), `approved?`, `denied?`. `ViewRefresh`: `every` (minutes), `prompt`.

`InkStroke` (04/11): `id`, `kind` `'pen'|'arrow'|'text'`, `color` `'graphite'|'red'|'orange'|'green'|'blue'|'purple'`, `points` number[] (flat x,y pairs), `size` number (multiple of the default), `text?`.

`Rect`: `x, y, w, h` numbers. `Viewport`: `x, y, zoom` numbers.

`CaptureRecord` (07): `id`, `kind` `'session'|'into'|'note'|'view'`, `projectId`, `title`, `createdAt`, `sessionId?`, `noteId?`, `viewId?`.

Checked against a real 0.9.4 `state.json`, by key names only: the fields listed above occur, and no unknown fields do.

### Shared types used by the API (not persisted)

| Type | Shape |
| --- | --- |
| `SessionStatus` | `'working' \| 'waiting' \| 'done'` |
| `SessionKind` | `'claude' \| 'terminal'` |
| `FOLDER_COLORS` | `['blue','purple','pink','red','orange','yellow','green','graphite']` (order matters, PLAT-073) |
| `INK_COLORS` / `INK_KINDS` | `['graphite','red','orange','green','blue','purple']` / `['pen','arrow','text']` |
| `ProjectStats` | `{ total, active, working, waiting: number, running: RunningCounts }` |
| `RunningCounts` | `{ sessions, background, terminals: number }` |
| `SessionMeta` | `{ sessionId, title, firstPrompt?, lastPrompt?, lastAssistantText?, gitBranch?, messageCount, createdAt, updatedAt, transcriptPath }` |
| `SessionCard` | `BoardNode & { meta?: SessionMeta, status: SessionStatus, live: boolean, agents?: AgentInfo[], agentCount?: number, usage?: SessionUsage }` |
| `AgentInfo` | `{ agentId, agentType, description, status: SessionStatus, parentAgentId?, stopped?, tool?, activity?, background?: number, toolCount, startedAt, updatedAt }` |
| `SessionUsage` | `{ model?, contextPercent?, contextTokens?, contextSize?, costUsd?, durationMs?, at }` |
| `BoardSnapshot` | `{ project: Project, cards: SessionCard[], archive: Rect, sections: BoardSection[], notes: BoardNote[], views: BoardView[], ink: InkStroke[], viewport?: Viewport }` |
| `NodePatch` | `{ sessionId, x?, y?, archived?, title?, sectionId?: string \| null, expanded?, w?, h? }` |
| `CreateSessionOptions` | `{ x, y, kind?, name?, initialPrompt?, parents?: string[], actionId?, expanded?, autoMode?, fromView? }` |
| `SummaryResult` | `{ sessionId, title, summary, error? }` |
| `SummarizePhase` | `'queued' \| 'running' \| 'done' \| 'error'` |
| `SummarizeProgress` | `{ requestId, sessionId, phase, error? }` |
| `SearchSnippetKind` | `'prompt' \| 'reply' \| 'tool'` |
| `SearchSnippet` | `{ kind, tool?, file?, agent?: boolean, text, ranges: [number, number][], timestamp }` |
| `SessionSearchHit` | `{ sessionId, matches: number, snippets: SearchSnippet[] }` |
| `ToolCall` | `{ name, detail? }` |
| `TranscriptMessage` | `{ role: 'user' \| 'assistant', text, timestamp, tools?: string[], calls?: ToolCall[], notice?: boolean }` |
| `OpenSessionResult` | `{ replay: string, live: boolean }` |
| `AppInfo` | `{ claudePath: string \| null, claudeVersion: string \| null, platform: string }` |
| `ThemePreference` | `'system' \| 'light' \| 'dark'` |
| `OpenCardRequest` | `{ projectId, sessionId? }` |
| `UpdateState` | tagged on `state`: `{off, reason}` · `{idle}` · `{checking}` · `{latest}` · `{downloading, version, percent}` · `{ready, version}` · `{error, message}` |
| `UpdateStatus` | `UpdateState & { current: string }` |
| `VoiceKeySource` | `'settings' \| 'env' \| null` |
| `TranscribeResult` | `{ text }` or `{ error, keyProblem? }` (untagged) |
| `GitFileStatus` | `'modified' \| 'added' \| 'deleted' \| 'renamed' \| 'conflicted' \| 'untracked'` |
| `GitFileChange` | `{ path, oldPath?, status }` |
| `GitStatus` | `{ branch: string \| null, oid: string \| null, upstream?, ahead, behind, files: GitFileChange[] }` |
| `GitFileDiff` | `{ patch, binary?, tooLarge? }` |
| `GitCommit` | `{ oid, subject, author, time }` |
| `ActionLists` | `{ project: Project, global: ProjectAction[] }` |
| `ActionDraft` | `{ name, icon, prompt }` |
| `ActionRunResult` | `{ projectId, sessionId, actionName, message? }` |
| `ViewEvent` | `'data' \| 'state' \| 'result'` |
| `ViewPatch` | `{ id, x?, y?, w?, h?, title?, startsSessions?: false }` |
| `CreateViewOptions` | `{ description, x, y }` |
| `CommandResult` | `{ name, code: number \| null, stdout, stderr, at, durationMs, error? }` |
| `CaptureKind` | `'session' \| 'into' \| 'note' \| 'view'` |
| `CaptureAttachment` | tagged on `kind`: `{tab, url, title, app}` · `{image, path, thumb, copied?}` · `{file, path, name, size}` · `{text, text}` |
| `CaptureSession` | `{ projectId, sessionId, title, status, live, updatedAt }` |
| `CaptureRecent` | `{ id, kind, projectId, title, createdAt, sessionId?, noteId?, viewId?, status? }` |
| `CaptureTargets` | `{ projects: {id, name, color}[], sessions: CaptureSession[], recents: CaptureRecent[], defaultProjectId: string \| null }` |
| `CaptureShortcut` | `'ready' \| 'needs-permission' \| 'off' \| 'unavailable'` |
| `CaptureContext` | `{ app?, tab?: tab attachment, clipboard?: {kind:'text', text} \| {kind:'image', thumb}, shortcut: CaptureShortcut }` |
| `CaptureGuess` | `{ kind, projectId, sessionId?, reason }` |
| `CaptureSubmission` | `{ text, kind, projectId, sessionId?, attachments: CaptureAttachment[] }` |
| `CaptureReceipt` | `{ kind, projectId, projectName, title, sessionId?, noteId?, viewId? }` |

### Secret formats

| | macOS | Windows |
| --- | --- | --- |
| Stored | `state.voiceKey` = base64(blob) | same |
| Blob | `"v10"` + AES-128-CBC/PKCS7(plaintext) | `"v10"` + nonce[12] + AES-256-GCM(plaintext) + tag[16] |
| Key | PBKDF2-HMAC-SHA1(pw, `"saltysalt"`, 1003, 16 B) | 32 B from `Local State` › `os_crypt.encrypted_key` = base64(`"DPAPI"` + CryptProtectData(key)) |
| IV / nonce | 16 × 0x20 | random 12 B, stored in the blob |
| Key source | Keychain generic password: service `Nodus Safe Storage`, account `Nodus Key` (login keychain) | DPAPI, current user |
| Confidence | prefix, names, salt and IV confirmed; iterations and hash: verify in Chromium | verify in Chromium source and on a Windows install |

### Login-shell capture wire format

stdout = `[noise]__NODUS_ENV_START__KEY=VALUE\0KEY=VALUE\0…__NODUS_ENV_END__[noise]`. The first START and the last END delimit the body. Values may contain `=` and newlines, never NUL.

### Bug-report URL

The body is these lines joined with `\n`:

```
**What happened?**
<empty>
<empty>
**What did you expect to happen?**
<empty>
<empty>
**Steps to reproduce**
<empty>
<empty>
---
Nodus <app version> · <system> <OS version> · <arch>
```

- `<system>` is `macOS`, `Windows` or `Linux`.
- `<OS version>` is Electron's `process.getSystemVersion()`: `major.minor.patch` on macOS (e.g. `26.2.0`), `10.0.<build>` on Windows.
- `<arch>` uses Node arch names, `arm64` or `x64`.
- The `·` separators are U+00B7 with a space on each side.

The URL is `https://github.com/saleem-hadad/nodus/issues/new?` + `URLSearchParams({ body })`, which is form encoding: space → `+`; `*-._` and alphanumerics kept; everything else percent-encoded as UTF-8 with uppercase hex. Example:
`https://github.com/saleem-hadad/nodus/issues/new?body=**What+happened%3F**%0A%0A%0A**What+did+you+expect+to+happen%3F**%0A%0A%0A**Steps+to+reproduce**%0A%0A%0A---%0ANodus+0.9.4+%C2%B7+macOS+26.2.0+%C2%B7+arm64`

### Chromium files to read for migration

- `Preferences` (JSON): `partition.per_host_zoom_levels.<hash>.<file URL ending /out/renderer/index.html>` = zoom level (number; factor 1.2^level). `spellcheck.dictionaries` = string[].
- `Local Storage/leveldb`: keys for origin `file://`. Chromium's key encoding is `_file://` + `\0` + `\x01` + key, and values are prefixed `\x01` (Latin-1) or `\x00` (UTF-16LE). Verify in Chromium `components/services/storage/dom_storage`. Keys:

  | Key | Value | Owner |
  | --- | --- | --- |
  | `capture.draft` | JSON string | 07 |
  | `capture.attachments` | JSON `CaptureAttachment[]` | 07 |
  | `capture.clipboardDone` | JSON string | 07 |
  | `capture.history` | JSON `string[]` | 07 |
  | `nodus.inkColor` | raw ink color name, **not** JSON | 11 |

## API surface

The bridge is `window.api`. Kind: `invoke` = request/response (async, can reject); `send` = fire-and-forget; `local` = no main process. ⏳env = awaits the login-shell env (PLAT-128). Rejection texts are the ones thrown in `ipc.ts`. The UI sees them wrapped per PLAT-124.

### Commands

| `window.api.…` | Channel | Kind | Args | Returns | Rejects / notes | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| `appInfo()` | `app:info` | invoke ⏳env | — | `AppInfo` | no UI caller in 0.9.4 | 01 |
| `takeOpenRequest()` | `app:take-open-request` | invoke | — | `OpenCardRequest \| null` | returns the pending request once, then null | 01 / 02 |
| `pathForFile(file)` | — | local, sync | `File` | string (`''` if none) | PLAT-126 | 01 |
| `showMenu(x, y)` | `app:show-menu` | invoke | number, number | void | PLAT-060 | 01 |
| `menuCommand(command)` | `app:menu-command` | invoke | `MenuCommand` | void | invalid values ignored | 01 |
| `settings.get()` | `settings:get` | invoke | — | `AppSettings` | | 01 |
| `settings.update(patch)` | `settings:update` | invoke | `Partial<AppSettings>` | `AppSettings` | PLAT-065 | 01 |
| `projects.list()` | `projects:list` | invoke | — | `Project[]` | | 01 |
| `projects.add(repoPath?)` | `projects:add` | invoke | string? | `Project \| null` | `Folder not found: …`, `Not a folder: …` | 01 |
| `projects.update(id, patch)` | `projects:update` | invoke | string, `{name?, color?, archived?, groupId?: string\|null, focused?}` | `Project` | `Unknown project …`, `Unknown group …` | 01 |
| `projects.remove(id)` | `projects:remove` | invoke | string | void | | 01 |
| `projects.revealInFinder(id)` | `projects:reveal` | invoke | string | void | `Unknown project …` | 01 |
| `projects.stats()` | `projects:stats` | invoke | — | `Record<id, ProjectStats>` | | 01 / 03 |
| `groups.list()` | `groups:list` | invoke | — | `ProjectGroup[]` | | 01 |
| `groups.create(name)` | `groups:create` | invoke | string | `ProjectGroup` | | 01 |
| `groups.update(id, patch)` | `groups:update` | invoke | string, `{name?, collapsed?}` | `ProjectGroup` | `Unknown group …` | 01 |
| `groups.remove(id)` | `groups:remove` | invoke | string | void | | 01 |
| `board.load(projectId)` | `board:load` | invoke | string | `BoardSnapshot` | `Unknown project …` | 03 |
| `board.saveNodes(projectId, patches)` | `board:save-nodes` | invoke | string, `NodePatch[]` | void | | 03 |
| `board.saveArchive(projectId, rect)` | `board:save-archive` | invoke | string, `Rect` | void | | 03 |
| `board.saveViewport(projectId, viewport)` | `board:save-viewport` | invoke | string, `Viewport` | void | | 03 |
| `board.removeNode(projectId, sessionId)` | `board:remove-node` | invoke | string, string | void | | 03 |
| `board.saveSection(projectId, section)` | `board:save-section` | invoke | string, `BoardSection` | void | | 03 |
| `board.removeSection(projectId, sectionId)` | `board:remove-section` | invoke | string, string | void | | 03 |
| `board.nameSection(projectId, sessionIds)` | `board:name-section` | invoke ⏳env | string, string[] | string | `These sessions have nothing to name the section after yet` | 03 |
| `board.saveNote(projectId, note)` | `board:save-note` | invoke | string, `BoardNote` | void | | 04 |
| `board.removeNotes(projectId, noteIds)` | `board:remove-notes` | invoke | string, string[] | void | | 04 |
| `board.saveInk(projectId, strokes)` | `board:save-ink` | invoke | string, `InkStroke[]` | void | | 04 |
| `sessions.create(projectId, opts)` | `session:create` | invoke ⏳env | string, `CreateSessionOptions` | `SessionCard` | `That action no longer exists`; the `requireClaude` messages | 03 |
| `sessions.open(projectId, sessionId, cols, rows)` | `session:open` | invoke (⏳env when not live) | string, string, number, number | `OpenSessionResult` | | 03 |
| `sessions.write(sessionId, data)` | `session:write` | send | string, string | — | | 02 / 03 |
| `sessions.send(projectId, sessionId, text)` | `session:send` | invoke (⏳env when not live) | string, string, string | void | `That session is no longer on the board`; `It’s waiting for your answer. Open it to reply.` | 03 |
| `sessions.resize(sessionId, cols, rows)` | `session:resize` | send | string, number, number | — | | 02 |
| `sessions.kill(sessionId)` | `session:kill` | invoke | string | void | | 02 |
| `sessions.transcript(projectId, sessionId)` | `session:transcript` | invoke | string, string | `TranscriptMessage[]` | | 03 |
| `sessions.summarize(projectId, sessionIds, requestId, focus?)` | `session:summarize` | invoke ⏳env | string, string[], string, string? | `SummaryResult[]` (in `sessionIds` order) | per-session error `This session has no conversation yet` | 03 |
| `sessions.search(projectId, query)` | `session:search` | invoke | string, string | `SessionSearchHit[]` | | 03 |
| `sessions.agents(projectId, sessionId)` | `session:agents` | invoke | string, string | `AgentInfo[]` | | 02 |
| `sessions.agentTranscript(projectId, sessionId, agentId)` | `session:agent-transcript` | invoke | string, string, string | `TranscriptMessage[]` | `Not an agent of this session` | 03 |
| `views.create(projectId, opts)` | `views:create` | invoke ⏳env | string, `CreateViewOptions` | `BoardView` | | 04 |
| `views.save(projectId, patch)` | `views:save` | invoke | string, `ViewPatch` | void | | 04 |
| `views.remove(projectId, viewId)` | `views:remove` | invoke | string, string | void | | 04 |
| `views.call(projectId, viewId, method, args)` | `views:call` | invoke | string, string, string, unknown | unknown | | 04 |
| `views.report(projectId, viewId, report)` | `views:report` | send | string, string, `{error?: string, loaded?: number}` | — | errors swallowed | 04 |
| `views.approve(projectId, viewId, name, allow)` | `views:approve` | invoke | string, string, string, boolean | void | `allow` coerced to `=== true` | 04 |
| `views.refresh(projectId, viewId)` | `views:refresh` | invoke ⏳env | string, string | `CommandResult[]` | | 04 |
| `views.ask(projectId, viewId, prompt)` | `views:ask` | invoke ⏳env | string, string, string | void | | 04 |
| `views.brain(projectId, viewId)` | `views:brain` | invoke ⏳env | string, string | string (sessionId) | | 04 |
| `git.status(projectId)` | `git:status` | invoke ⏳env | string | `GitStatus \| null` | | 05 |
| `git.diff(projectId, file)` | `git:diff` | invoke ⏳env | string, `GitFileChange` | `GitFileDiff` | | 05 |
| `git.log(projectId, limit?)` | `git:log` | invoke ⏳env | string, number? | `GitCommit[]` | an integer limit is clamped to 1..50; anything else becomes 5 | 05 |
| `actions.global()` | `actions:global` | invoke | — | `ProjectAction[]` | | 05 |
| `actions.save(projectId, action)` | `actions:save` | invoke | string, `ProjectAction` | `ActionLists` | | 05 |
| `actions.remove(projectId, actionId)` | `actions:remove` | invoke | string, string | `ActionLists` | | 05 |
| `actions.draft(projectId, description, global)` | `actions:draft` | invoke ⏳env | string, string, boolean | `ActionDraft` | | 05 |
| `voice.keySource()` | `voice:key-source` | invoke ⏳env | — | `VoiceKeySource` | | 05 |
| `voice.setKey(key)` | `voice:set-key` | invoke ⏳env | string \| null | `VoiceKeySource` | PLAT-119 messages | 05 |
| `voice.microphone()` | `voice:microphone` | invoke | — | boolean | | 05 |
| `voice.openMicrophoneSettings()` | `voice:microphone-settings` | invoke | — | void | | 05 |
| `voice.transcribe(projectId, audio, mimeType)` | `voice:transcribe` | invoke ⏳env | string, `Uint8Array`, string | `TranscribeResult` | | 05 |
| `updates.status()` | `updates:status` | invoke | — | `UpdateStatus` | | 06 |
| `updates.check()` | `updates:check` | invoke | — | void | user-asked check | 06 |
| `updates.install()` | `updates:install` | invoke | — | void | | 06 |
| `capture.ready()` | `capture:ready` | invoke | — | `CaptureContext \| null` | | 07 |
| `capture.targets()` | `capture:targets` | invoke | — | `CaptureTargets` | | 07 |
| `capture.guess(text, attachments)` | `capture:guess` | invoke | string, `CaptureAttachment[]` | `CaptureGuess \| null` | | 07 |
| `capture.submit(submission)` | `capture:submit` | invoke | `CaptureSubmission` | `CaptureReceipt` | | 07 |
| `capture.screenshot()` | `capture:screenshot` | invoke | — | image attachment \| `'needs-permission'` \| null | | 07 |
| `capture.files(paths?)` | `capture:files` | invoke | string[]? | `CaptureAttachment[]` | | 07 |
| `capture.clipboard()` | `capture:clipboard` | invoke | — | `CaptureAttachment \| null` | | 07 |
| `capture.hide()` | `capture:hide` | send | — | — | | 07 |
| `capture.resize(height)` | `capture:resize` | send | number | — | | 07 |
| `capture.open(request)` | `capture:open` | invoke | `OpenCardRequest` | void | | 07 |
| `capture.openSettings()` | `capture:settings` | invoke | — | void | emits `ev:menu` `settings` | 07 |
| `capture.shortcut()` | `capture:shortcut` | invoke | — | `CaptureShortcut` | | 07 |
| `capture.allow()` | `capture:allow` | invoke | — | `CaptureShortcut` | | 07 |
| `capture.allowScreen()` | `capture:allow-screen` | invoke | — | void | | 07 |
| `capture.show()` | `capture:show` | invoke | — | void | | 07 |

### Events (`window.api.on.*`, main → UI)

| `on.…` | Channel | Payload (positional) | Target | Emitted by |
| --- | --- | --- | --- | --- |
| `ptyData` | `ev:pty-data` | `sessionId: string, data: string` | main window | 02 |
| `ptyExit` | `ev:pty-exit` | `sessionId: string, exitCode: number` | main window | 02 |
| `status` | `ev:status` | `sessionId, status: SessionStatus, live: boolean` | main window | 02 |
| `agentStatus` | `ev:agent-status` | `sessionId, agentId, status: SessionStatus` | main window | 02 |
| `usage` | `ev:usage` | `sessionId, usage: SessionUsage` | main window | 02 |
| `boardChanged` | `ev:board-changed` | `projectId` | main window | 03, 04, 05, 07 |
| `projectsChanged` | `ev:projects-changed` | — | main window | 04 (`create_project`) |
| `summarizeProgress` | `ev:summarize-progress` | `progress: SummarizeProgress` | main window | 03 |
| `runFinished` | `ev:run-finished` | `result: ActionRunResult` | main window | 05 |
| `menu` | `ev:menu` | `command: MenuCommand` | main window | 01 (menu, `app:menu-command`), 07 (`capture:settings`) |
| `updateStatus` | `ev:update-status` | `status: UpdateStatus` | main window | 06 |
| `view` | `ev:view` | `projectId, viewId, event: ViewEvent, payload: unknown` | main window | 04 |
| `openCard` | `ev:open-card` | `request: OpenCardRequest` | main window | 02 (notification click), 07 |
| `captureShown` | `ev:capture-shown` | `context: CaptureContext` | **capture panel only** | 07 |
| `captureContext` | `ev:capture-context` | `context: Partial<CaptureContext>` | **capture panel only** | 07 |

Handlers registered outside `ipc.ts`: the `capture:*` channels (capture.ts, `registerCapture`), registered last.

### What this domain consumes from others

- 02: `connectHost(false)`, `disconnect()`, `stopAll()`, `killProject(id)`, `notify.updateBadge/armNotifications/takeOpenRequest`, `configureStatus`
- 03: `realRepoPath`, `projectStats`, `unwatchProject/unwatchAll`
- 04: `servePage`, `VIEW_SCHEME = 'nodus-view'`, `flushViews`, `removeProjectViews`
- 06: `startUpdater`, `check(asked)`
- 07: `openPanel`, `syncShortcut`, `stopCapture`, `registerCapture`
- 05: `voice` (sole consumer of secrets)

## UI details

Window (PLAT-014 to PLAT-021):

| Item | Value |
| --- | --- |
| Default size | 1440 × 920 logical px, centered |
| Minimum size | 960 × 620 |
| Title | `Nodus` |
| Background, light | `#f5f5f4` (`--bg-window` light) |
| Background, dark | `#1e1e20` (`--bg-window` dark) |
| macOS traffic lights | `hiddenInset`, position `{ x: 18, y: 18 }` |
| Windows caption overlay height | 52 (`--titlebar-h: 52px`) |
| Windows overlay, light | color `#f6f6f5` = `--bg-toolbar` light `rgba(246,246,245,0.82)` over `#f5f5f4`; symbols `#1f1f1f` |
| Windows overlay, dark | color `#252527` = `--bg-toolbar` dark `rgba(38,38,41,0.82)` over `#1e1e20`; symbols `#e6e6e6` |
| Vibrancy / transparency | none |

- Menus are native: macOS NSMenu, Windows the Electron/Chromium views popup, themed by the effective theme. The ☰ popup opens with its top-left at (button left, button bottom + 4) in window coordinates (08 computes them).
- Folder picker (PLAT-068): title `Add Project`, button `Add Project`, message `Choose the local repository for this project`, a New Folder button on macOS.
- Verbatim menu labels (the ellipsis is U+2026): `Check for Updates…`, `Settings…`, `Quit and Stop Sessions`, `Capture…`, `New Session`, `New Terminal`, `New Project…`, `Close`, `File`, `Exit`, `Edit`, `Search Sessions…`, `View`, `Focus Mode`, `Window`, `Star on GitHub`, `Report a Bug…`.
- Role labels from Electron 44: `About Nodus`, `Services`, `Hide Nodus`, `Hide Others`, `Show All`, `Quit Nodus` (macOS) / `Exit` (Windows), `Undo`, `Redo`, `Cut`, `Copy`, `Paste`, `Paste and Match Style`, `Delete`, `Select All`, `Reload`, `Force Reload`, `Toggle Developer Tools`, `Actual Size`, `Zoom In`, `Zoom Out`, `Toggle Full Screen`, `Minimize`, `Zoom`, `Bring All to Front`, `Help`.

## Keyboard & mouse

| Item | macOS | Windows: shown | Windows: registered |
| --- | --- | --- | --- |
| Settings… | ⌘, | Ctrl+, | yes |
| Hide Nodus / Hide Others | ⌘H / ⌥⌘H | — | — |
| Quit Nodus / Exit | ⌘Q | (none) | — |
| Quit and Stop Sessions | ⌥⌘Q | (none) | — |
| New Session | ⌘N | Ctrl+N | no (App.tsx outside terminals) |
| New Terminal | ⌘T | Ctrl+T | no (App.tsx outside terminals; Ctrl+Shift+T in terminals, 12) |
| New Project… | ⇧⌘N | Ctrl+Shift+N | yes |
| Close | ⌘W | Ctrl+W | no (App.tsx outside terminals; Ctrl+Shift+W in terminals, 12) |
| Undo / Redo | ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Y | no |
| Cut / Copy / Paste | ⌘X / ⌘C / ⌘V | Ctrl+X / Ctrl+C / Ctrl+V | no |
| Paste and Match Style | ⌥⇧⌘V | (absent) | — |
| Select All | ⌘A | Ctrl+A | no |
| Search Sessions… | ⌘K (not registered) | Ctrl+K | no |
| Focus Mode | ⇧⌘F (only while offered) | Ctrl+Shift+F | yes (only while offered) |
| Reload | ⌘R | Ctrl+R | no |
| Force Reload | ⇧⌘R | Ctrl+Shift+R | yes |
| Toggle Developer Tools | ⌥⌘I | Ctrl+Shift+I | yes |
| Actual Size / Zoom In / Zoom Out | ⌘0 / ⌘+ / ⌘- | Ctrl+0 / Ctrl+Plus / Ctrl+- | no |
| Toggle Full Screen | ⌃⌘F | F11 | yes |
| Minimize | ⌘M | Ctrl+M | no |

- The shortcut text the Windows popup shows is Electron's. Verify `Zoom In`, which Electron may render as `Ctrl+Shift+=`, against 0.9.4.
- Mouse:
  - The ☰ title-bar button opens the app menu popup (PLAT-060).
  - On macOS, a Dock icon click re-creates the window when no window exists (PLAT-030).
  - Clicking a notification shows the window (02).
  - Dropping files from the OS yields their paths (PLAT-126). App.tsx blocks drops outside drop targets, so the window never navigates to `file://` (08).
  - Links: PLAT-024.
- Shortcut matching helpers for every UI spec: `modKey`, `otherModKey`, `shortcutKey`, `keys` (PLAT-131 to PLAT-133).

## Platform differences (macOS vs Windows)

| Topic | macOS | Windows |
| --- | --- | --- |
| Single instance | none (LaunchServices activates the running copy) | lock per userData; a second launch exits 0, the first shows its window |
| AppUserModelID | — | `dev.saleem.nodus` |
| Title bar | `hiddenInset`, traffic lights at (18, 18) | `hidden` + caption overlay, h 52, themed colors |
| Menu | menu bar: Nodus / File / Edit / View / Window / Help | no bar; ☰ popup: File / Edit / View / Window / Help |
| Settings / Check for Updates | app menu | File menu |
| Quit | `Quit Nodus` ⌘Q; `Quit and Stop Sessions` ⌥⌘Q | `Exit`; `Quit and Stop Sessions` (no shortcuts) |
| Paste and Match Style | present | absent |
| Window menu | standard (Minimize, Zoom, Bring All to Front, window list) | Minimize only |
| About | app menu, standard About panel | Help menu, message box |
| Ctrl+letter shortcuts | ⌘ shortcuts all registered (except ⌘K) | shown but not registered (PLAT-055) |
| Last window closed | app stays open; Dock reopens it (quirk PLAT-030) | app quits (quirk PLAT-031) |
| Shell env | captured from `$SHELL -ilc` | process env as is; PATH key folded |
| `claude` lookup | `claude` on PATH | `claude.exe`, then `claude.cmd` shim → exe |
| Fallback PATH | 8 Unix directories | `.local\bin`, WinGet Links, `npm` |
| User shell | `$SHELL` (absolute) or `/bin/zsh`; terminal `-l` | `pwsh.exe` or Windows PowerShell; terminal `-NoLogo` |
| `shellCommand` | `-c run` | `-NoProfile -NonInteractive -Command` + UTF-8 prefix |
| Git Bash | — | `gitBash()` lookup |
| Install hint | `npm i -g @anthropic-ai/claude-code` | `irm https://claude.ai/install.ps1 \| iex, in PowerShell` |
| Secrets | Keychain `Nodus Safe Storage` / `Nodus Key`, AES-128-CBC | `Local State` DPAPI-wrapped key, AES-256-GCM |
| Reveal | Finder | Explorer |
| Picker message / New Folder | shown | ignored |
| Logoff / shutdown | `before-quit` runs | `before-quit` doesn't run (PLAT-035) |
| `app:info.platform` | `darwin` | `win32` |

## Edge cases & errors

Errors thrown by this domain (exact text; the UI sees them wrapped per PLAT-124):
- `Folder not found: <folder as given>`: realpath failed (missing, or not permitted).
- `Not a folder: <real path>`
- `Unknown project <id>`
- `Unknown group <id>`
- `Claude Code CLI not found. Install it (npm i -g @anthropic-ai/claude-code) and restart Nodus.` (macOS)
- `Claude Code CLI not found. Install it (irm https://claude.ai/install.ps1 | iex, in PowerShell) and restart Nodus.` (Windows)
- `The Keychain isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` / `Encrypted storage isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` (05; U+2019 apostrophes)

Log lines, which the user never sees (stderr):
- `[store] save failed <err>`
- `[store] flush failed <err>`
- `[env] init failed <err>`
- `[host] connect failed <err>`
- `[voice] could not decrypt the saved key <err>`
- Electron's `Error occurred in handler for '<channel>': <err>`

Edge cases:
- A corrupt or unreadable `state.json` silently becomes the defaults, and the next save overwrites it (PLAT-086).
- The old-app migration copies even when `NODUS_USER_DATA` points elsewhere. It looks next to the override folder (`dirname(userData)/<OLD_APP_NAME>`).
- `projects:update` with an unknown `groupId` throws after applying name, color, archived and focused in memory (PLAT-074).
- `projects:add` with a path that is already a project (after realpath) returns the existing project unchanged, unless it is archived or focusing applies (PLAT-071). The UI can't tell "added" from "existing" through this command.
- Two projects can't share a real path through `addProject`. Projects whose stored paths have since become symlinked to the same place are matched by the first in order.
- A `claude.cmd` shim that runs Node (npm's script shim) means "not found" on Windows (PLAT-103), even though `claude` works in a terminal.
- If the shell capture times out (10 s) or exceeds 8 MiB, it falls back to the process env plus the fallback PATH. Shells that print the markers in their own startup output are tolerated, since the first START and the last END are used.
- `gitBash()` called before the capture finishes caches a result computed from the process env (PLAT-113).
- `app:show-menu` with NaN or Infinity coordinates, or with no window, does nothing.
- `app:menu-command` with an unknown command does nothing and resolves.
- Settings patch: `{ theme: 'blue' }` sets `system`, and any unknown key is dropped.
- A second instance on Windows before the first is ready: the first doesn't show anything (PLAT-008).
- A Windows second instance gets focus: Chromium's ProcessSingleton lets the first process take the foreground. The Rust app needs an equivalent (e.g. `AllowSetForegroundWindow`; verify) or the window may only flash in the taskbar.
- Menu commands sent while the window is still loading are dropped. Capture's openSettings defers its `settings` command until the reopened window has loaded (07).
- If the host connect hangs, the window never appears (PLAT-011). Connect timeouts belong to 02.
- `groups:create` with a non-string name throws a JS TypeError (`Cannot read properties of undefined (reading 'trim')`). The UI never sends one.

## Tests to port

From `src/main/env.test.ts` (`describe('exeBehindShim')`):
1. **finds the program an npm shim starts**: shim at `C:\Users\me\AppData\Roaming\npm\claude.cmd`, contents (CRLF-joined) `@ECHO off`, `GOTO start`, `:find_dp0`, `SET dp0=%~dp0`, `EXIT /b`, `:start`, `SETLOCAL`, `CALL :find_dp0`, `"%dp0%\node_modules\@anthropic-ai\claude-code\bin\claude.exe"   %*`, `` → `C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe`.
2. **finds nothing in a shim that runs a script with Node**: contents `@ECHO off`, `SETLOCAL`, `CALL :find_dp0`, `IF EXIST "%dp0%\node.exe" (`, `  SET "_prog=%dp0%\node.exe"`, `) ELSE (`, `  SET "_prog=node"`, `)`, `endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\node_modules\@anthropic-ai\claude-code\cli.js" %*` → `null`.

From `src/renderer/src/platform.test.ts` (each test reloads the module with a stubbed `navigator.platform`; in Rust, parameterize the functions by platform):
3. `keys` › **writes a Mac shortcut as Windows does** (`Win32`): `platform == 'windows'`; `keys('⇧⌘F') == 'Ctrl+Shift+F'`, `keys('⌘↩') == 'Ctrl+Enter'`, `keys('⌘') == 'Ctrl'`, `keys('⌫') == 'Del'`, `keys('↩') == 'Enter'`.
4. `keys` › **leaves it be on a Mac** (`MacIntel`): `platform == 'mac'`; `keys('⇧⌘F') == '⇧⌘F'`.
5. `modKey` › **is ⌘ on a Mac and Ctrl elsewhere**: Mac, `modKey({metaKey:true, ctrlKey:false}) == true`; Windows, the same input gives `false`; Windows, `modKey({metaKey:false, ctrlKey:true}) == true`; Windows, `otherModKey({metaKey:true, ctrlKey:false}) == true`.
6. `shortcutKey` › **reads a Latin layout’s letter as it is** (Windows): `{key:'c',keyCode:67}` → `c`; `{key:'W',keyCode:87}` → `w`; `{key:'a',keyCode:65}` → `a` (AZERTY A).
7. `shortcutKey` › **reads the letter from the key code on another alphabet’s layout**: `{key:'ر',keyCode:86}` → `v`; `{key:'с',keyCode:67}` → `c`.
8. `shortcutKey` › **leaves keys that aren’t letters be**: `{key:'Enter',keyCode:13}` → `enter`; `{key:'é',keyCode:50}` → `é`.

No tests exist in 0.9.4 for store, menu, ipc or index. Suggested Rust tests, which are not ports:
- A real `state.json` round trip that is byte-identical when nothing changes.
- PLAT-063 normalization.
- Unknown nested fields preserved (PLAT-087).
- The shell-capture parser on noisy output.
- Fallback PATH ordering.
- The bug-report URL encoding example.
- `v10` macOS decryption with a known test password.

## Rust mapping

Crates (plan.md):
- `nodus-domain`: every type above, serde-compatible.
  - Use `#[serde(rename_all = "camelCase")]` where the names already match, `skip_serializing_if = "Option::is_none"` everywhere optional, and `#[serde(flatten)] extra: serde_json::Map<String, Value>` on every persisted record (Project, ProjectGroup, ProjectAction, BoardState, BoardNode, BoardSection, BoardNote, BoardView, ViewCommand, ViewRefresh, InkStroke, CaptureRecord) for PLAT-087.
  - `Settings` gets **no** `extra`: unknown keys are dropped (PLAT-063).
  - Maps keyed by id are `IndexMap` (serde_json `preserve_order`), to keep the JS insertion order.
  - `UpdateState`: `#[serde(tag = "state")]`. `CaptureAttachment`: `#[serde(tag = "kind")]`. `TranscribeResult`: untagged.
- `nodus-store`:
  - userData resolution: `dirs::config_dir()` gives `~/Library/Application Support` and the Roaming AppData folder, + `Nodus`; else `NODUS_USER_DATA` as the raw `OsString`.
  - load, migrate, normalize; save debounced 300 ms (tokio sleep, reset on each call); `flush()`.
  - Atomic write: `state.json.<pid>.tmp` then `std::fs::rename`, which on Windows uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` like Node.
  - JSON writer that matches `JSON.stringify(…, null, 2)`: serde_json `PrettyFormatter::with_indent(b"  ")` (it writes `": "` the same way), plus a custom f64 serializer that prints integral values without `.0` (`ryu-js` formats numbers the JS way).
- `nodus-platform`:
  - `env`: `std::process::Command` with a 10 s kill-on-timeout (`wait-timeout`, or tokio `timeout` + `kill`) and an 8 MiB cap.
  - `which`-like lookup implemented exactly as PLAT-101: don't use the `which` crate, whose search order and extension handling differ.
  - `exeBehindShim` with the `regex` crate, using the same pattern and `(?i)`.
  - `realpath`: **port Node's JS `fs.realpathSync`** (resolve to absolute, then lstat each component, readlink symlinks, keep the input's case) rather than `std::fs::canonicalize`. That returns `\\?\` verbatim paths on Windows and may normalize case on macOS (R2).
  - Secrets:
    - macOS: `security-framework` (`get_generic_password("Nodus Safe Storage", "Nodus Key")`, `set_generic_password`), `pbkdf2` + `sha1` + `aes` + `cbc`.
    - Windows: the `windows` crate `CryptUnprotectData` / `CryptProtectData` plus `aes-gcm`, reading and writing `Local State` JSON. Use `serde_json` with `preserve_order`, and only touch `os_crypt.encrypted_key`.
  - Open URL: `NSWorkspace openURL` / `ShellExecuteW` (or the `open` crate).
  - Reveal: `NSWorkspace activateFileViewerSelectingURLs` / `SHOpenFolderAndSelectItems`.
  - Folder picker:
    - macOS: `NSOpenPanel` through `objc2-app-kit`, because `rfd` can't set `prompt`, `message` or `canCreateDirectories` exactly: `setPrompt("Add Project")`, `setMessage`, `setTitle`, `canChooseDirectories`, `!canChooseFiles`, `canCreateDirectories`, `beginSheetModalForWindow`.
    - Windows: `IFileOpenDialog` with `FOS_PICKFOLDERS`, `SetTitle`, `SetOkButtonLabel`.
  - Windows AUMID: `SetCurrentProcessExplicitAppUserModelID`.
  - Single instance on Windows: a named mutex keyed by a hash of userData, plus a named pipe or window message to notify the first instance, and `AllowSetForegroundWindow`. It need not interoperate with Chromium's ProcessSingleton: 0.9.4 and the Rust app never run at once, because the updater quits first (06).
- `nodus-core`: commands and events mirroring the API tables one to one. The error type carries `{ channel, name, message }`, with a `display_ipc()` that yields `Error invoking remote method '<channel>': <name>: <message>` (PLAT-124). Event fan-out is per window target: main versus capture panel (PLAT-123).
- `nodus-ui`:
  - GPUI `WindowOptions`: `titlebar: TitlebarOptions { appears_transparent: true, traffic_light_position: Some(point(px(18.), px(18.))) }`, `window_min_size` 960×620, centered 1440×920 bounds, `show: false` until the first frame, background per theme.
  - Menus: `cx.set_menus` with `MenuItem::action` and `MenuItem::os_submenu("Services", SystemMenuType::Services)`. Keybindings registered only where PLAT-055 says. Focus Mode's visibility and enabled state are synced from settings.
  - On Windows the ☰ popup is drawn in-app (GPUI deferred popover) or `TrackPopupMenuEx`. Match Chromium's views menu look (verify with screenshots).
  - Appearance: set `NSApp.appearance` (theme source), and on Windows draw the caption buttons in the title bar.
- Not ported: `ELECTRON_RENDERER_URL` (dev server), the CSP meta, contextIsolation and sandbox flags. The `nodus-view` handling moves to wry (04).

Risks and unknowns:
- **R1 Secrets.**
  - macOS: the Keychain item's ACL trusts the 0.9.4 app's designated requirement. The Rust .app must be signed with the same Team ID and bundle id `dev.saleem.nodus`, or users get a Keychain prompt (or the read fails). Unsigned dev builds will prompt.
  - Windows: the format is AES-GCM with a key in `Local State`, not bare DPAPI on the string. D6 in plan.md should say so.
  - Verify iterations, hash and key sizes in Chromium 152 sources.
- **R2 realpath semantics.** Project identity (PLAT-071) and the transcript-folder encoding derived from `repoPath` (03 `encodeRepoPath`) depend on Node's JS realpath: case kept, no `\\?\` prefix. Any difference re-creates projects or loses sessions.
- **R3 Web-only menu items.** `Reload`, `Force Reload`, `Toggle Developer Tools` and page zoom (`Actual Size`/`Zoom In`/`Zoom Out`, persisted) have no native meaning. Needs a decision: keep the items, with reload meaning a UI reset, devtools meaning something or hidden, and zoom meaning a GPUI UI scale migrated from `Preferences`. Existing installs may have a non-zero zoom level saved.
- **R4 Chromium-provided state and behavior.** localStorage keys (PLAT-140), page zoom (PLAT-139) and spellcheck underlines (PLAT-137) need native replacements and migration. GPUI has no spellchecker (NSSpellChecker on macOS; Windows `ISpellChecker`).
- **R5 Windows chrome.** The caption overlay and popup menu are Chromium-drawn in 0.9.4. GPUI must draw lookalikes (sizes, hover colors, Win11 rounded menu). Also verify whether pressing Alt shows anything in 0.9.4.
- **R6 IPC error strings.** About 15 UI sites show the full `Error invoking remote method …` text. The core must keep channel names for parity.
- **R7 Startup gating.** The window waits for the host connect (PLAT-011). The Rust host connect must time out the same way (02).
- **R8 QUIRKS PLAT-030 and PLAT-031.** The hidden capture panel blocks Dock reopen on macOS and keeps the app alive on Windows. Reproduce them unless the plan owner rules them bugs.
- **R9 Keychain and AppKit menu extras.** The items macOS appends to Edit, View and Window depend on how menus are built. Compare against 0.9.4.

## Parity checklist

- [x] PLAT-001 App name `Nodus` in OS and menus
- [x] PLAT-002 Windows AUMID `dev.saleem.nodus` at start
- [x] PLAT-003 Default userData folder per OS
- [x] PLAT-004 `NODUS_USER_DATA` override (non-empty, raw)
- [ ] PLAT-005 userData string byte-identical to 0.9.4
- [x] PLAT-006 No single-instance lock on macOS
- [ ] PLAT-007 Windows lock; second launch exits 0
- [ ] PLAT-008 Second launch shows the first's window when ready
- [ ] PLAT-009 `nodus-view` registered standard+secure
- [x] PLAT-010 Startup order (13 steps)
- [x] PLAT-011 Window waits for host connect to settle
- [x] PLAT-012 Env capture non-blocking; ⏳env commands wait
- [x] PLAT-013 All commands registered before the window
- [x] PLAT-014 Window 1440×920, min 960×620, centered, title `Nodus`
- [x] PLAT-015 No window bounds persistence
- [x] PLAT-016 Hidden until first frame
- [x] PLAT-017 macOS hiddenInset, traffic lights (18,18)
- [ ] PLAT-018 Windows caption overlay h52 + colors
- [x] PLAT-019 Window background `#1e1e20` / `#f5f5f4`
- [x] PLAT-020 No vibrancy/transparency
- [x] PLAT-021 Live theme change updates background/overlay
- [x] PLAT-022 Single main window; events dropped without it
- [x] PLAT-023 showWindow: create / restore / show+focus
- [x] PLAT-024 New-window requests: http(s) → browser, else ignored
- [x] PLAT-025 Main window never navigates away; http(s) → browser
- [ ] PLAT-026 View frames only load their own `nodus-view://<id>`
- [ ] PLAT-027 Bridge never exposed to view frames
- [x] PLAT-028 Theme source system/light/dark everywhere
- [x] PLAT-029 macOS: closing window keeps app running
- [x] PLAT-030 macOS activate recreates window only with zero windows (panel quirk)
- [ ] PLAT-031 Windows quits on all windows closed (panel quirk)
- [x] PLAT-032 shutdown() order on before-quit
- [x] PLAT-033 Quitting leaves sessions running
- [x] PLAT-034 Quit and Stop Sessions: shutdown host, ≤5000 ms, then quit
- [ ] PLAT-035 Windows logoff skips shutdown()
- [x] PLAT-036 Menu commands dropped with no window; others still work
- [x] PLAT-037 Menu bar order / Windows popup
- [x] PLAT-038 macOS app menu items
- [x] PLAT-039 macOS File menu
- [x] PLAT-040 Windows File menu
- [x] PLAT-041 Edit menu
- [x] PLAT-042 View menu
- [x] PLAT-043 Window menu
- [x] PLAT-044 Help menu
- [ ] PLAT-045 AppKit-added menu items match 0.9.4
- [x] PLAT-046 Check for Updates… → settings + check(true)
- [x] PLAT-047 Settings… → settings
- [x] PLAT-048 Capture… → openPanel
- [x] PLAT-049 File/Edit commands → menu commands
- [x] PLAT-050 Star on GitHub URL
- [x] PLAT-051 Report a Bug… URL
- [x] PLAT-052 Focus Mode item visibility/enabled/tick/click
- [x] PLAT-053 Focus menu re-sync on applySettings
- [x] PLAT-054 Quit/Exit behavior
- [x] PLAT-055 Off-Mac shown-not-registered accelerators
- [x] PLAT-056 Edit roles act on focused field/terminal
- [ ] PLAT-057 Reload/DevTools/zoom/full screen semantics (decision)
- [x] PLAT-058 About panel (macOS) / message box (Windows)
- [x] PLAT-059 Native Hide/Show All/Services/Minimize/Zoom/Front
- [x] PLAT-060 app:show-menu popup at rounded (x,y)
- [x] PLAT-061 app:menu-command validation + echo
- [x] PLAT-062 Seven MenuCommand values
- [x] PLAT-063 Settings normalization + key order
- [x] PLAT-064 settings:get
- [x] PLAT-065 settings:update merge/apply/save/autoUpdate check
- [x] PLAT-066 applySettings four effects
- [x] PLAT-067 projects:list
- [x] PLAT-068 projects:add picker strings, modality, cancel → null
- [x] PLAT-069 addProject realpath + errors
- [x] PLAT-070 "Focusing" definition
- [x] PLAT-071 Existing project match + unarchive/join focus
- [x] PLAT-072 New project fields
- [x] PLAT-073 pickFolderColor least-used random
- [x] PLAT-074 projects:update rules + partial-mutation edge
- [x] PLAT-075 projects:remove effects
- [x] PLAT-076 projects:reveal
- [x] PLAT-077 projects:stats
- [x] PLAT-078 projectsChanged only from create_project
- [x] PLAT-079 groups:list
- [x] PLAT-080 groups:create default `New Group`
- [x] PLAT-081 groups:update
- [x] PLAT-082 groups:remove ungroups projects
- [x] PLAT-083 Single state.json, full rewrite
- [x] PLAT-084 Old-app state migration (voiceKey dropped)
- [x] PLAT-085 Load: per-field acceptance + key order
- [x] PLAT-086 Corrupt/missing → defaults, overwritten later
- [x] PLAT-087 Nested records kept verbatim (unknown fields)
- [x] PLAT-088 Unknown top-level keys dropped
- [x] PLAT-089 No per-entry validation at load
- [x] PLAT-090 getProject/getGroup error texts
- [x] PLAT-091 getBoard lazy default + backfill without save
- [x] PLAT-092 projectIdForView/Session lookup
- [x] PLAT-093 300 ms debounced save; error logged
- [x] PLAT-094 Atomic write via `.<pid>.tmp` + rename, 2-space JSON
- [x] PLAT-095 flush only when pending
- [x] PLAT-096 Initial env; Windows PATH fold
- [x] PLAT-097 macOS login-shell capture + parse
- [x] PLAT-098 Windows: no capture
- [x] PLAT-099 Merge captured over process env
- [x] PLAT-100 Fallback PATH dirs appended
- [x] PLAT-101 findOnPath order + checks
- [x] PLAT-102 macOS claude lookup
- [x] PLAT-103 Windows claude.exe / .cmd shim lookup
- [x] PLAT-104 exeBehindShim regex
- [x] PLAT-105 claude --version read
- [x] PLAT-106 Lookup once per launch
- [x] PLAT-107 spawnEnv strip + TERM/COLORTERM/LANG + extra
- [x] PLAT-108 envVar unstripped
- [x] PLAT-109 userShell
- [x] PLAT-110 terminalShell args
- [x] PLAT-111 shellCommand args
- [x] PLAT-112 requireClaude messages
- [x] PLAT-113 gitBash lookup + memo
- [x] PLAT-114 app:info contract
- [x] PLAT-115 voiceKey = base64 safeStorage blob
- [x] PLAT-116 macOS v10 AES-128-CBC/PBKDF2 format
- [x] PLAT-117 Keychain `Nodus Safe Storage` / `Nodus Key`
- [x] PLAT-118 Windows v10 AES-256-GCM + Local State DPAPI key
- [x] PLAT-119 Encryption-unavailable messages
- [x] PLAT-120 Decrypt failure → no key (source still settings)
- [x] PLAT-121 Write the same secret format
- [x] PLAT-122 Command/event contract per API tables
- [x] PLAT-123 Event routing (main vs panel), no queue
- [x] PLAT-124 `Error invoking remote method '…': …` format
- [x] PLAT-125 Fire-and-forget commands
- [x] PLAT-126 pathForFile / dropped paths
- [x] PLAT-127 Subscriptions + unsubscribe, multi-listener
- [x] PLAT-128 ⏳env commands wait; failure still resolves
- [x] PLAT-129 Structured-clone argument semantics
- [x] PLAT-130 isMac / platform value
- [x] PLAT-131 modKey / otherModKey
- [x] PLAT-132 shortcutKey layout handling
- [x] PLAT-133 keys() label conversion
- [x] PLAT-134 data-platform root attribute
- [x] PLAT-135 `#capture` page routing
- [ ] PLAT-136 CSP: frames only nodus-view
- [ ] PLAT-137 Spellcheck underlines in opted-in fields
- [x] PLAT-138 No native right-click menu in fields
- [ ] PLAT-139 Persisted page zoom (migrate from Preferences)
- [x] PLAT-140 localStorage keys (migrate or accept loss)
- [x] PLAT-141 Leave Chromium files; keep Local State/Preferences/Local Storage
