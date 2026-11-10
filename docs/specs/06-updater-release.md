# 06 · Updater, packaging & release

Sources:
- `src/main/updater.ts` (all 495 lines), `src/main/updater.test.ts` (all 197 lines)
- `src/renderer/src/components/UpdateButton.tsx`; the update parts of `src/renderer/src/components/SettingsDialog.tsx` (`UpdateSettings`, `updateText`, `RELEASES_URL`)
- `src/main/ipc.ts` (`updater.startUpdater`, `settings:update` → `check()`, `updates:status|check|install`), `src/main/menu.ts` (`Check for Updates…`), `src/preload/index.ts` and `src/shared/api.ts` (`updates`, `on.updateStatus`), `src/shared/types.ts` (`UpdateState`, `UpdateStatus`, `AppSettings.autoUpdate`), `src/main/store.ts` (`normalizeSettings` default for `autoUpdate`)
- `electron-builder.yml`, `build/entitlements.mac.plist`, `build/icon.icns`, `build/icon.png`, `build/icon-win.png`, `build/icon.svg`
- `.github/workflows/release.yml`, `.github/workflows/ci.yml`, `scripts/smoke.mjs`, `scripts/postinstall.cjs`, `package.json`
- `docs/how-it-works.md`: Updates, Windows, Development, Releasing
- electron-builder 26.15.3 internals that shape the shipped artifacts (`app-builder-lib/templates/nsis/*`, `out/targets/nsis/NsisTarget.js`, `out/macPackager.js`, `out/electron/electronMac.js`, `out/winPackager.js`, `dmg-builder/out/dmg.js`, `dmg-builder/out/dmgUtil.js`)
- The published v0.9.4 release, inspected on 2026-11-08: the GitHub API answer for `releases/latest`, `Nodus-arm64.dmg` mounted with the updater's own commands, and an installed 0.9.4 `Nodus.app` (Info.plist, `codesign -dv`, entitlements, designated requirement, `spctl`).

## Summary

Nodus updates itself from GitHub releases without electron-updater. 30 s after launch and every 4 h after that (while **Download updates automatically** is on), it asks `api.github.com/repos/saleem-hadad/nodus/releases/latest` for the newest published release, picks the asset for this OS and machine (`Nodus-{arm64|x64}.dmg`, `Nodus-Setup-{x64|arm64}.exe`), and if the tag is newer it downloads the asset into `<userData>/updates/`, checking size and GitHub's SHA-256 `digest`.

- **macOS:** it mounts the DMG hidden, copies the `.app` out with `ditto --noqtn`, then checks `codesign --verify --deep --strict`, the bundle id, the version and the signing team. When the app quits, a detached `/bin/sh` script waits for the app's PID to exit, swaps the bundle in place, rolls back and writes `updates/failed` if that fails, and reopens the app when the user chose **Restart to Update**. Sessions keep running in the session host the whole time.
- **Windows:** it keeps the downloaded installer, which isn't code signed (only the digest vouches for it), between launches. **Restart to Update** asks before stopping the running sessions, then runs `Nodus-Setup-<v>.exe --updated /S --force-run` and quits. The electron-builder NSIS one-click installer replaces the app in `%LOCALAPPDATA%\Programs\nodus` and starts it again.

The UI is a title-bar **Restart to Update** button, a Settings › Updates group and a **Check for Updates…** menu item.

The migration has a hard handoff constraint. Every installed Electron copy (macOS 0.6.0 through 0.9.4, Windows 0.9.0 through 0.9.4) runs its own updater against the first Rust release, so that release's assets, DMG, bundle, Info.plist, signature and NSIS installer must pass every check those copies run. They are listed in the **HANDOFF CONTRACT** under Data & formats. The Rust app's own updater (`nodus-updater`) then has to behave identically, so later Rust releases update the same way.

## Behaviors

### Status and start-up

- **UPD-001** Before `startUpdater` runs, the status is `{ state: 'idle', current: <running version> }`.
- **UPD-002** Every state change replaces the whole status with `{ ...next, current: <running version> }` (no fields carry over from the previous state) and pushes it to the main window on `ev:update-status`. Nothing is sent when no window exists or the window is destroyed.
- **UPD-003** `startUpdater` first works out why updates might be off. When there's a reason, it sets `{ state: 'off', reason }` and stops there: no clean-up, no quit hook, no timers.
- **UPD-004** On a platform that is neither macOS nor Windows, the reason is `Updates are only built for macOS and Windows.`
- **UPD-005** On macOS or Windows, an unpackaged (development) build's reason is `Development builds don’t update themselves.` This check comes after the platform check.
- **UPD-006** A packaged Windows build is never off.
- **UPD-007** A packaged macOS build is on only when the user can write to the app bundle (`<execPath>/../../..`) and to the bundle's parent folder (`access(W_OK)` on both).
- **UPD-008** A macOS bundle that can't be written to while it sits in an Applications folder (`app.isInApplicationsFolder()`) gets the reason `Your account can’t change apps in Applications, so Nodus can’t update itself.`
- **UPD-009** A macOS bundle that can't be written to anywhere else (run from the DMG, or translocated) gets the reason `Move Nodus to your Applications folder so it can update itself.`
- **UPD-010** When updates are on, clean-up of `updates/` (UPD-071…UPD-075) starts immediately and every `check()` awaits it before doing anything.
- **UPD-011** On macOS, updates being on registers the swap (UPD-057) on the app's `will-quit`. Windows registers nothing on quit.
- **UPD-012** The automatic check runs once 30 000 ms after `startUpdater`, then every 14 400 000 ms (4 h) on a fixed interval measured from `startUpdater`, not from the first check. Each tick reads `settings.autoUpdate` at that moment and runs a non-asked `check()` only when it's true.
- **UPD-013** A `settings:update` whose patch has a truthy `autoUpdate` runs a non-asked `check()` after saving. Turning the switch on checks right away.
- **UPD-014** `settings.autoUpdate` defaults to true. `normalizeSettings` sets it to `s.autoUpdate !== false`.

### Checking

- **UPD-015** `check()` returns without changing state while a lookup or download is under way (`busy`), while an update is staged, or while the state is `off`.
- **UPD-016** Otherwise `check()` marks itself busy, sets `checking`, and sends `GET https://api.github.com/repos/saleem-hadad/nodus/releases/latest` with headers `Accept: application/vnd.github+json` and `User-Agent: Nodus/<running version>`, with no token, under a 20 000 ms timeout.
- **UPD-017** When the request fails or times out before a response arrives, the state becomes `error` with `Couldn’t reach GitHub to check for updates.`
- **UPD-018** HTTP 403 or 429 becomes `error` with `GitHub is limiting update checks right now. Try again later.`
- **UPD-019** Any other non-2xx status becomes `error` with `GitHub answered the update check with an error (<status>).`
- **UPD-020** The asset is the first one in `assets` whose `name` equals the expected asset name and whose `state` is `'uploaded'`.
- **UPD-021** The expected asset name uses arch `arm64` when the build is arm64 or runs under ARM64 translation (`app.runningUnderARM64Translation`: Rosetta on a Mac, x64 emulation on Windows on Arm), and `x64` otherwise. It is `Nodus-<arch>.dmg` on macOS and `Nodus-Setup-<arch>.exe` on Windows.
- **UPD-022** The release version is the capture of `/^v?(\d+\.\d+\.\d+)$/` on `tag_name`, without the `v`.
- **UPD-023** When no asset matches, or `tag_name` doesn't match the pattern, the lookup counts as "nothing for this machine" and the state becomes `latest`.
- **UPD-024** The asset's SHA-256 is the capture of `/^sha256:([0-9a-f]{64})$/` on `digest` (lowercase hex only). It is null when `digest` is missing, null or doesn't match.
- **UPD-025** `isNewer(a, b)` splits both on `.`, reads each part with `parseInt(n, 10) || 0`, compares the first three parts numerically (a missing part is 0), and returns false when they're all equal.
- **UPD-026** A release that isn't newer than the running version (equal or older) sets `latest`. There is never a downgrade.
- **UPD-027** A non-asked check that finds a release whose version equals the recorded failed version sets `error` with `Nodus <version> downloaded but couldn’t install itself.` and downloads nothing.
- **UPD-028** An asked check (Check Now, the menu item, `updates:check`) prepares a release even when its version is the failed one.
- **UPD-029** `check()` resolves once the lookup is done. Preparing the release (download and checks) carries on in the background, and busy stays set until it finishes.

### Downloading (both platforms)

- **UPD-030** The download is `GET browser_download_url` with `User-Agent: Nodus/<running version>` (no `Accept`), following redirects.
- **UPD-031** A stall timer of 60 000 ms starts before the request and restarts at every received chunk. When it fires, the download is aborted. There is no limit on total time.
- **UPD-032** `downloading` with `percent: 0` is set before the download starts. After that, `percent = min(100, floor(received / size * 100))` is emitted as `downloading` only when it differs from the last value emitted.
- **UPD-033** A non-OK response, or one without a body, fails with `GitHub answered the download with an error (<status>).`
- **UPD-034** When the bytes received differ from the asset's `size`, or a SHA-256 is known and the file's hex SHA-256 differs from it, the download fails with `The update came through damaged. Try again later.`
- **UPD-035** Any other download failure (connection, stall abort, write error) fails with `The update didn’t finish downloading. Check your connection.`
- **UPD-036** A failure while preparing that isn't one of the named update errors shows `The update couldn’t be prepared. Try again later.` Every preparation failure is logged as `[updater] update failed`.

### macOS: preparing

- **UPD-037** On macOS the paths are `dir = <userData>/updates/<version>`, `dmg = <userData>/updates/Nodus-<version>.dmg` and `target = <dir>/Nodus.app`.
- **UPD-038** Preparing runs `rm -rf dir`, `mkdir -p dir`, the download to `dmg`, copying the app out, the checks, then `staged = { version, app: target }` and `ready`.
- **UPD-039** On macOS a missing SHA-256 doesn't stop the download. Only the size is checked then.
- **UPD-040** The DMG is mounted on a fresh `mkdtemp(<userData>/updates/mount-)` folder with `/usr/bin/hdiutil attach <dmg> -nobrowse -noautoopen -readonly -mountpoint <mount>`.
- **UPD-041** The app copied is the first entry of `readdir(<mount>)` whose name ends in `.app`. When there is none, preparing fails with `The update has no app in it.`
- **UPD-042** The app is copied with `/usr/bin/ditto --noqtn <mount>/<name> <target>`, so the copy carries no quarantine flag.
- **UPD-043** The DMG is always detached with `/usr/bin/hdiutil detach <mount> -force`. A failed detach is logged as `[updater] detach failed` and doesn't fail the update. Then `rmdir <mount>` runs, ignoring errors.
- **UPD-044** The first check is `/usr/bin/codesign --verify --deep --strict <target>`. A non-zero exit fails with `The new version’s signature didn’t check out.`
- **UPD-045** Next, three reads run in parallel: `/usr/bin/plutil -extract <key> raw -o - <app>/Contents/Info.plist` for `CFBundleIdentifier` of the new app, `CFBundleIdentifier` of the running bundle and `CFBundleShortVersionString` of the new app, each with stdout trimmed. When the two identifiers differ, or the new version isn't exactly the release version, preparing fails with `The update isn’t the expected version of Nodus.` A failing `plutil` (a missing key, for one) gives the generic message from UPD-036.
- **UPD-046** Last comes the team. A bundle's team is the capture of `/^TeamIdentifier=(.+)$/m` on the stderr of `/usr/bin/codesign -dv <bundle>`, and `not set` or no match means none. When the running bundle has a team and the new app's team differs, preparing fails with `The update isn’t signed by the same developer.` When the running bundle has no team (ad-hoc signed), this check is skipped.
- **UPD-047** When preparing fails, `rm -rf dir` runs and the state becomes `error` with the message. Whether it succeeds or fails, the DMG file is deleted at the end (`rm` with `force`, errors ignored).

### Windows: preparing

- **UPD-048** On Windows the installer path is `<userData>\updates\Nodus-Setup-<version>.exe`. `downloading` with `percent: 0` is set first.
- **UPD-049** Without a SHA-256 for the asset, preparing fails with `GitHub had no checksum for the update, so it wasn’t downloaded.`
- **UPD-050** After `mkdir -p updates`, an existing file at the installer path whose size equals the asset's `size` and whose SHA-256 matches is used as it is, without downloading. The state then goes straight from `downloading` 0 to `ready`.
- **UPD-051** Otherwise the installer is downloaded to that path, replacing what's there.
- **UPD-052** On success, `staged = { version, app: <installer path> }` and the state becomes `ready`.
- **UPD-053** On failure, the installer file is deleted (errors ignored) and the state becomes `error` with the message.

### Restart to Update and installing

- **UPD-054** `restartToUpdate()` does nothing when no update is staged.
- **UPD-055** On macOS, `restartToUpdate()` sets `relaunch = true` and quits the app normally (`app.quit()`), which fires `will-quit` and the swap.
- **UPD-056** On macOS, any normal quit while an update is staged installs it, including Quit and Quit and Stop Sessions. Only Restart to Update reopens the app afterwards.
- **UPD-057** At `will-quit`, with an update staged and no swap started yet in this run, the app starts `/bin/sh -c <SWAP_SCRIPT> nodus-update <pid> <bundle path> <staged app> <userData>/updates/previous.app <userData>/updates/failed <version> <'1' or ''> <NODUS_USER_DATA or ''>`, detached, stdio ignored and unref'd. It never waits for it.
- **UPD-058** The 8th argument is `process.env.NODUS_USER_DATA` (or `''` when unset) only when relaunching. Otherwise it is `''`.
- **UPD-059** The swap script ignores HUP, INT and TERM and polls `kill -0 <pid>` every 0.2 s until the app's process has exited.
- **UPD-060** The swap script then runs `rm -rf <previous.app>`, `mv <bundle> <previous.app>`, `mv <staged app> <bundle>` and, when both moves succeed, `rm -rf <previous.app>`.
- **UPD-061** When the new app can't be moved into place, the script runs `rm -rf <bundle>`, moves `<previous.app>` back to `<bundle>`, and writes the version followed by a newline to `updates/failed`.
- **UPD-062** When the running app can't be moved aside, the script writes the version followed by a newline to `updates/failed` and leaves everything else alone.
- **UPD-063** Only when the 7th argument is non-empty does the script run `open -n "<bundle>"`, or `open -n --env "NODUS_USER_DATA=<8th>" "<bundle>"` when the 8th is non-empty. It does this whether or not the swap succeeded, so a failed swap reopens the old version.
- **UPD-064** On Windows, a `restartToUpdate()` that arrives while an install is already under way returns at once. Two clicks ask once and start one installer.
- **UPD-065** On Windows, the installer file has to exist before anything else happens. When it's gone, there's no dialog, no sessions are stopped, the app doesn't quit, and the error path (UPD-069) runs.
- **UPD-066** On Windows, when one or more sessions are live, a native question dialog asks first: message `Stop the running session to update?` for one session, `Stop <n> running sessions to update?` for more; detail `Windows can’t update Nodus while its sessions run. You can resume them from their cards afterwards.`; buttons `Stop and Update` and `Not Now`; default `Stop and Update`; cancel `Not Now`.
- **UPD-067** `Not Now` (and cancelling) leaves the sessions running and the state at `ready`, and starts no installer.
- **UPD-068** Then all sessions and the host are stopped (`stopAll()`, which also runs when no session is live), and the installer is started detached with arguments exactly `['--updated', '/S', '--force-run']` and stdio ignored. Once Windows reports the process started, it is unref'd and the app quits (`app.quit()`). Stopping always comes before the spawn.
- **UPD-069** When the installer can't start (or stopping the sessions throws, or the installer is missing), the failure is logged as `[updater] the installer didn’t start`, the staged update is dropped, the installer file is deleted, the app stays open, and the state becomes `error` with `The installer didn’t start, so Nodus wasn’t updated. Try Check for Updates again.` The next check downloads it again.
- **UPD-070** On Windows an update is never installed on quit. It goes in only through Restart to Update.

### Clean-up of `updates/` (at start-up, when updates are on)

- **UPD-071** Clean-up reads `updates/failed` and trims it. When that version is newer than the running one, it becomes the failed version. Otherwise it's ignored, and the file is deleted by UPD-074.
- **UPD-072** On Windows, clean-up keeps the installer for the newest version newer than the running one, among files named `/^Nodus-Setup-(\d+\.\d+\.\d+)\.exe$/`. Every other installer, older or newer, is deleted.
- **UPD-073** On macOS, every entry whose name starts with `mount-` gets `/usr/bin/hdiutil detach <path> -force` (errors ignored) before it is deleted.
- **UPD-074** Every other entry is deleted (`/bin/rm -rf` on macOS, recursive forced removal on Windows), except `failed` while a failed version is set and the kept Windows installer. Deletion errors are logged as `[updater] cleanup failed` and don't stop clean-up.
- **UPD-075** A missing `updates/` folder is treated as empty.

### IPC, menu

- **UPD-076** `updates:status` (`window.api.updates.status()`) returns the current status.
- **UPD-077** `updates:check` (`window.api.updates.check()`) runs `check(true)` and resolves once the lookup is done.
- **UPD-078** `updates:install` (`window.api.updates.install()`) runs `restartToUpdate()`.
- **UPD-079** The menu item `Check for Updates…` (no accelerator) sends the `settings` menu command, which opens Settings, and then runs `check(true)`. On macOS it sits in the app menu right after About. Elsewhere it's in File, right after `Settings…`.

### UI

- **UPD-080** `useUpdateStatus()` is null until `updates.status()` answers, and then follows every `updateStatus` event. It unsubscribes on unmount and ignores an answer that arrives after unmount.
- **UPD-081** The title bar shows the Update button only while the state is `ready`. It is a `btn btn-primary` button with a `RotateCw` icon (size 14, stroke 2.2) and the label `Restart to Update`. Its tooltip is `Nodus <version> is ready. Sessions keep running while Nodus restarts.` on a Mac and `Nodus <version> is ready. Restarting stops the sessions; you can resume them after.` elsewhere. Clicking it calls `updates.install()`.
- **UPD-082** Settings has a fieldset with the legend `Updates`. Its first row is titled `Nodus <current>`, with a hint for the state: off → the reason; idle → none; checking → `Checking for updates…`; latest → `You’re up to date.`; downloading → `Downloading version <v>… <percent>%`; ready → `Version <v> is ready to install.`; error → the message. With no status yet, the title is `Nodus ` and there is no hint.
- **UPD-083** The first row's button is a primary `Restart to Update` (calls `updates.install()`) when `ready`; no button when `off`; otherwise `Check Now` (calls `updates.check()`), disabled while the status is null, `checking` or `downloading`.
- **UPD-084** The second row, hidden when `off`, is the switch `Download updates automatically` (checkbox `role="switch"`, class `switch`). It is checked when `settings.autoUpdate ?? false`, disabled until settings load, and writes `{ autoUpdate }` on change. Its hint is `They install when you quit Nodus. Sessions keep running.` on a Mac and `They install when you choose Restart to Update, which stops the sessions.` elsewhere.
- **UPD-085** In the `error` state, a hint paragraph after the rows reads `You can also download the latest version yourself.`, where `download the latest version` links to `https://github.com/saleem-hadad/nodus/releases/latest` and opens in the default browser.

### Release, packaging, CI

- **REL-001** The version has one source (0.9.4: `package.json` `version`; Rust: the workspace version). It feeds `UpdateStatus.current`, the `Nodus/<v>` User-Agent, `CFBundleShortVersionString` and `CFBundleVersion`, the DMG volume name, the Windows exe's FileVersion (`<v>`) and ProductVersion (`<v>.0`), NSIS `VIProductVersion` (`<v>.0`), `BrandingText` (`Nodus <v>`), and the uninstall entry's `DisplayName` (`Nodus <v>`) and `DisplayVersion` (`<v>`).
- **REL-002** The release workflow runs when a tag matching `v*` is pushed, with `permissions: contents: write`.
- **REL-003** Each release job fails first when `v<version>` differs from the tag, printing `::error::Tag $GITHUB_REF_NAME doesn't match package.json version $version` and exiting 1.
- **REL-004** The macOS release job runs on `macos-latest`, and runs the type check and the tests before packaging.
- **REL-005** The macOS release job installs the Developer ID certificate from secret `CSC_LINK` (a base64 .p12, password in `CSC_KEY_PASSWORD`) into a throwaway keychain, using the script quoted under Data & formats.
- **REL-006** Only the packaging step receives `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD` and `APPLE_TEAM_ID`. Install scripts never see them.
- **REL-007** The macOS job produces `Nodus-arm64.dmg` and `Nodus-x64.dmg`. Each holds a thin (single-arch) `Nodus.app` for its arch, with `LC_BUILD_VERSION minos 13.0` and `LSMinimumSystemVersion` `13.0`.
- **REL-008** The app is signed with the Developer ID Application certificate under the hardened runtime (`flags=0x10000(runtime)`), with the entitlements in `build/entitlements.mac.plist` on the app and on every nested piece of code, plus a secure timestamp.
- **REL-009** When all three Apple secrets are set, the app is notarized and the ticket is stapled to the `.app`. The DMG itself isn't signed, notarized or stapled.
- **REL-010** Each DMG is UDIF read-only compressed with zlib (UDZO, compression level 9) and an HFS+ file system, with volume name `Nodus <version>-<arch>` (for example `Nodus 0.9.4-arm64`), no licence agreement, and no encryption.
- **REL-011** The DMG root holds `Nodus.app`, the link `Applications -> /Applications`, `.VolumeIcon.icns` (= `build/icon.icns`), `.background.tiff` (dmg-builder's default 540×380 image) and a `.DS_Store`. Finder shows window bounds `{{400, 530}, {540, 380}}` with no toolbar, sidebar, status bar, path bar or tab bar, icon view at icon size 80 and text size 12 with labels at the bottom, arranged by none, `Nodus.app` at (130, 220) and `Applications` at (410, 220).
- **REL-012** The Windows release job runs on `windows-latest` with bash as the default shell. It installs dependencies and builds both installers, `Nodus-Setup-x64.exe` and `Nodus-Setup-arm64.exe`, with no type check or tests.
- **REL-013** Each job attaches its files with `gh release view "$GITHUB_REF_NAME" >/dev/null 2>&1 || gh release create "$GITHUB_REF_NAME" --title "$GITHUB_REF_NAME" --generate-notes || gh release view "$GITHUB_REF_NAME" >/dev/null`, then `gh release upload "$GITHUB_REF_NAME" <files> --clobber` (macOS `dist/*.dmg`, Windows `dist/Nodus-Setup-*.exe`). Whichever job finishes first creates the release.
- **REL-014** The release is published as soon as it's created (not a draft, not a prerelease), so `releases/latest` returns it before every asset is up. Installed copies treat a missing asset as `latest` (UPD-023).
- **REL-015** No `.blockmap`, `latest-mac.yml` or `latest.yml` is built or uploaded (`dmg.writeUpdateInfo: false`, `nsis.differentialPackage: false`).
- **REL-016** The Windows installer is an NSIS one-click, per-user installer (`RequestExecutionLevel user`, no UAC prompt). It installs into `<UserProgramFiles known folder>\nodus` (normally `%LOCALAPPDATA%\Programs\nodus`) unless `HKCU\Software\99c2a492-c8ad-53bf-923b-1df868e91b51` `InstallLocation` names an earlier location. `/D=<dir>` overrides both.
- **REL-017** The installer reads the flags `/S`, `--updated`, `--force-run`, `--keep-shortcuts`, `--no-desktop-shortcut`, `--delete-app-data`, `--allusers` and `--currentuser`, as electron-builder's template defines them (Data & formats).
- **REL-018** Only one installer runs at a time: it takes a mutex named `99c2a492-c8ad-53bf-923b-1df868e91b51`, and a second one brings the first to the front and quits.
- **REL-019** The installer closes every process whose image path starts with the install folder. With `--updated` it waits 300 ms, checks, waits 1000 ms more if any are still running, asks them to close, then force-kills and retries (the template's `_CHECK_APP_RUNNING`).
- **REL-020** The installer removes the previous version by copying the exe named in the `UninstallString` of `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\99c2a492-c8ad-53bf-923b-1df868e91b51` into its plugins folder and running `"<copy>" /S /KEEP_APP_DATA /currentuser [--keep-shortcuts] --updated _?=<install dir>`. It retries up to 5 times, 1 s apart. `--keep-shortcuts` is added when `KeepShortcuts` is `"true"` and `<install dir>\Nodus.exe` exists.
- **REL-021** The installer writes these HKCU values: under `Software\99c2a492-c8ad-53bf-923b-1df868e91b51`, `InstallLocation` (the install dir), `KeepShortcuts` (`true`) and `ShortcutName` (`Nodus`); under `Software\Microsoft\Windows\CurrentVersion\Uninstall\99c2a492-c8ad-53bf-923b-1df868e91b51`, `DisplayName` `Nodus <v>`, `UninstallString` `"<dir>\Uninstall Nodus.exe" /currentuser`, `QuietUninstallString` `"<dir>\Uninstall Nodus.exe" /currentuser /S`, `DisplayVersion` `<v>`, `DisplayIcon` `<dir>\Nodus.exe,0`, `Publisher` `Saleem Hadad`, `Comments` `A spatial board for managing Claude Code sessions per project`, `NoModify` and `NoRepair` (DWORD 1) and `EstimatedSize` (DWORD, KB).
- **REL-022** The install folder holds `Nodus.exe` and `Uninstall Nodus.exe`.
- **REL-023** A fresh install creates `Nodus.lnk` in the Start menu's Programs folder (no subfolder) and on the Desktop. Both point at `<dir>\Nodus.exe` with description `A spatial board for managing Claude Code sessions per project` and AppUserModelID `dev.saleem.nodus`. An update that keeps shortcuts leaves them as they are.
- **REL-024** After installing, when run without `/S` or with `--force-run`, the installer starts the app through the Start menu shortcut (or `<dir>\Nodus.exe` when there's no shortcut) using `ExecShellAsUser … "open"`. The argument is `--updated` when the installer itself got `--updated`, and empty otherwise.
- **REL-025** The installer copies itself to `%LOCALAPPDATA%\nodus-updater\installer.exe`. That is an electron-builder side effect nothing reads.
- **REL-026** Uninstalling keeps `%APPDATA%\Nodus` (`deleteAppDataOnUninstall: false`). Only `--delete-app-data` removes `%APPDATA%\nodus` and `%APPDATA%\Nodus`, which are the same folder since Windows paths ignore case.
- **REL-027** Neither the Windows installer nor `Nodus.exe` is code signed, so SmartScreen asks before the first run of a downloaded installer (**More info › Run anyway**).
- **REL-028** `Nodus.exe` carries the icon made from `build/icon-win.png` as icon index 0, and version strings FileDescription `Nodus`, ProductName `Nodus`, CompanyName `Saleem Hadad` and LegalCopyright `Copyright © <build year> Saleem Hadad`.
- **REL-029** CI runs on every pull request and every push to `main`, with `permissions: contents: read` and concurrency group `ci-${{ github.ref }}` (`cancel-in-progress: true`).
- **REL-030** The CI macOS job (`check`, `macos-latest`) installs dependencies, type-checks, tests and builds.
- **REL-031** The CI Windows job type-checks, runs only the Windows-safe tests (`src/main/hostProtocol.test.ts src/main/hooks.test.ts src/main/env.test.ts src/renderer/src/platform.test.ts`), builds, packages the x64 installer only, installs it with `Start-Process -Wait -FilePath dist/Nodus-Setup-x64.exe -ArgumentList '/S'` (pwsh), runs `node scripts/smoke.mjs "$LOCALAPPDATA/Programs/nodus/Nodus.exe" smoke`, and always uploads the `windows` artifact (`smoke/` and `dist/Nodus-Setup-x64.exe`, `if-no-files-found: ignore`).
- **REL-032** The smoke test starts the packaged app with `NODUS_USER_DATA` set to a fresh temp folder. It waits up to 60 s for the window, saves `1-projects-light.png`, adds a temp project, opens its board, creates an expanded terminal card at (80, 80), opens it at 120×36, types `echo ("nodus-" + (6 * 7))` on Windows or `echo nodus-$((6 * 7))` elsewhere followed by `\r`, and waits up to 30 s for `nodus-42`. It then reloads, saves `2-board-light.png`, switches to dark and saves `3-board-dark.png`.
- **REL-033** The smoke test connects to the session host as the app does: socket `<userData>/host/host.sock` (or `<tmpdir>/nodus-<sha1(dir)[0..12]>.sock` when the path is over 100 bytes), or on Windows the pipe `\\.\pipe\nodus-<sha1(dir)[0..12]>`, sending `{"type":"hello","protocol":0}`. It fails unless the welcome lists the terminal's session, and on Windows unless the welcome's `key` equals `<userData>/host/host.key`.
- **REL-034** On Windows, the smoke test presses keys through Windows. Ctrl+W on the board must leave it, Ctrl+W in a focused terminal must not (it saves `4-screen.png`), and Ctrl+Shift+W in a terminal must leave the board. The steps are skipped when the window can't be brought to the front.
- **REL-035** The smoke test always ends by sending the host `hello` (protocol 0) followed by `{"type":"shutdown"}`, killing the app and copying `host.log` into the screenshot folder. It exits 1 on failure (after saving `failure.png`) and 0 on success.
- **REL-036** Every action is pinned to a commit SHA (checkout `11d5960a…` v4.4.0, pnpm/action-setup `b906affc…` v4.3.0 with pnpm 10, setup-node `49933ea5…` v4.4.0 using `.nvmrc` 22, upload-artifact `ea165f8d…` v4.6.2), and checkout uses `persist-credentials: false`.
- **REL-037** `scripts/postinstall.cjs` only fixes up the Electron toolchain (chmod 755 on node-pty's `spawn-helper` prebuilds, and downloading Electron's binary when `path.txt` is missing). It has no Rust counterpart and goes away at cutover.
- **REL-038** The README and `docs/index.html` link to `https://github.com/saleem-hadad/nodus/releases/latest/download/<asset name>` for all four asset names, so the names never change.

### 0.9.4 → Rust handoff

- **REL-039** The first Rust release's version is newer than 0.9.4 by UPD-025 (for example `0.10.0` or `1.0.0`), and its tag is exactly `v<major>.<minor>.<patch>` with no suffix.
- **REL-040** Every installed macOS copy from 0.6.0 to 0.9.4 runs the same macOS logic (`updater.ts` unchanged in behavior since `b31d09b`), and every Windows copy from 0.9.0 to 0.9.4 runs the same Windows logic (`8839972`). The ad-hoc signed 0.6.0–0.8.0 copies skip the team check (UPD-046). The Rust release has to satisfy all of them.
- **REL-041** The `.app` in each Rust DMG passes 0.9.4's checks: `codesign --verify --deep --strict` succeeds, `CFBundleIdentifier` is `dev.saleem.nodus`, `CFBundleShortVersionString` equals the tag's version, and `codesign -dv` reports the same `TeamIdentifier` as 0.9.4 (the Team ID of the certificate in `CSC_LINK`, which is also `APPLE_TEAM_ID`).
- **REL-042** The Rust app starts when it's opened with `open -n "<bundle>"` at the swapped-in path, and with `open -n --env "NODUS_USER_DATA=<dir>" "<bundle>"` it uses `<dir>` as userData.
- **REL-043** The Rust app's first launch handles whatever the Electron updater left in `updates/` (`failed`, empty `<version>/` folders, `mount-*` folders, `previous.app`, `Nodus-<v>.dmg`, `Nodus-Setup-<v>.exe`) with the UPD-071…UPD-075 rules.
- **REL-044** On Windows, the Rust release's installer, started by 0.9.4 as `Nodus-Setup-<v>.exe --updated /S --force-run`, silently replaces the Electron install in the same folder, with the same registry keys and shortcuts, keeps `%APPDATA%\Nodus`, and starts the Rust app.
- **REL-045** The Rust app ignores an unknown `--updated` argument, which the installer passes when it starts the app after an update.
- **REL-046** The Rust app is signed so that its designated requirement equals 0.9.4's (`identifier "dev.saleem.nodus"` + Developer ID + the same team OU). That keeps the TCC grants (Microphone, Accessibility, Screen Recording, Automation) and the Keychain item `Nodus Safe Storage` working for it.
- **REL-047** From then on, Rust releases ship the same assets under the same contract, and the Rust app's updater implements UPD-001…UPD-085 unchanged, so Rust-to-Rust updates work the way Electron-to-Electron ones did.

## Data & formats

### Constants (src/main/updater.ts)

| Name | Value |
| --- | --- |
| `LATEST_URL` | `https://api.github.com/repos/saleem-hadad/nodus/releases/latest` |
| `FIRST_CHECK_MS` | `30_000` |
| `CHECK_EVERY_MS` | `4 * 60 * 60 * 1000` = `14_400_000` |
| `LOOKUP_TIMEOUT_MS` | `20_000` (covers the request and reading the JSON body; `AbortSignal.timeout`) |
| `STALL_MS` | `60_000` (re-armed per chunk; `setTimeout(...).refresh()`) |
| `RELEASES_URL` (SettingsDialog) | `https://github.com/saleem-hadad/nodus/releases/latest` |
| `REPO_URL` (menu) | `https://github.com/saleem-hadad/nodus` |
| Request headers (lookup) | `Accept: application/vnd.github+json`, `User-Agent: Nodus/<version>` |
| Request headers (download) | `User-Agent: Nodus/<version>` |
| Tag pattern | `/^v?(\d+\.\d+\.\d+)$/` |
| Digest pattern | `/^sha256:([0-9a-f]{64})$/` |
| Kept installer pattern | `/^Nodus-Setup-(\d+\.\d+\.\d+)\.exe$/` |
| Team pattern | `/^TeamIdentifier=(.+)$/m` on `codesign -dv` **stderr**; `not set` means none |

### GitHub API shape used

`GET /repos/saleem-hadad/nodus/releases/latest` gives the newest release that is neither a draft nor a prerelease (by GitHub's ordering, or the one marked "latest"). It answers 404 when there is none, which becomes `GitHub answered the update check with an error (404).` Only these fields are read; everything else is ignored:

```jsonc
{
  "tag_name": "v0.9.4",                      // optional; must match the tag pattern
  "assets": [                                 // optional; missing ⇒ no asset ⇒ 'latest'
    {
      "name": "Nodus-arm64.dmg",             // exact match with the expected asset name
      "state": "uploaded",                   // anything else (e.g. "open" mid-upload) is skipped
      "size": 136446713,                     // bytes; must equal the bytes downloaded
      "digest": "sha256:9112ae36…",          // may be null/absent on assets uploaded before GitHub computed digests
      "browser_download_url": "https://github.com/saleem-hadad/nodus/releases/download/v0.9.4/Nodus-arm64.dmg"
    }
  ]
}
```

As observed for v0.9.4 on 2026-11-08: four assets (`Nodus-arm64.dmg`, `Nodus-Setup-arm64.exe`, `Nodus-Setup-x64.exe`, `Nodus-x64.dmg`), all `state: "uploaded"`, all with `sha256:` digests, `draft: false`, `prerelease: false`. `browser_download_url` redirects to GitHub's asset CDN, and redirects are followed.

### UpdateState / UpdateStatus (IPC JSON; `src/shared/types.ts`)

```ts
type UpdateState =
  | { state: 'off'; reason: string }
  | { state: 'idle' }
  | { state: 'checking' }
  | { state: 'latest' }
  | { state: 'downloading'; version: string; percent: number }   // integer 0..100
  | { state: 'ready'; version: string }
  | { state: 'error'; message: string }
type UpdateStatus = UpdateState & { current: string }            // e.g. {"state":"ready","version":"0.9.5","current":"0.9.4"}
```

Rust: `#[serde(tag = "state", rename_all = "lowercase")]` enum, flattened with `current`, in `nodus-domain`.

Transitions:

```
(start) idle ──whyOff──▶ off (terminal for the run)
idle|latest|error ──check()──▶ checking ──▶ latest | error | downloading
downloading ──▶ downloading(percent…) ──▶ ready | error
ready (mac) ──quit──▶ [swap after exit]           ready (win) ──Restart, installer fails──▶ error
ready (win) ──Not Now──▶ ready
check() is a no-op in: off, ready (staged), and while busy (checking/downloading)
```

### `updates/` folder (`<userData>/updates`)

userData is `~/Library/Application Support/Nodus` on macOS, `%APPDATA%\Nodus` on Windows, or `$NODUS_USER_DATA` (owned by 01).

| Entry | Who writes it | Lifetime |
| --- | --- | --- |
| `Nodus-<v>.dmg` | macOS download | deleted at the end of `prepare` (success or failure); a crash leaves it for clean-up |
| `mount-XXXXXX/` | `mkdtemp` mount point | detached and removed after copying; clean-up force-detaches leftovers |
| `<v>/Nodus.app` | `ditto` copy, the staged app | moved into place by the swap; the empty `<v>/` is removed by the next clean-up |
| `previous.app` | the swap script | deleted by the script; restored on rollback |
| `failed` | the swap script (`echo "$6" > "$5"`): version + `\n` | kept while that version is newer than the running one |
| `Nodus-Setup-<v>.exe` | Windows download | kept (newest newer one) until installed or superseded |

### macOS commands (verbatim argv)

```
/usr/bin/hdiutil attach <dmg> -nobrowse -noautoopen -readonly -mountpoint <mount>
/usr/bin/ditto --noqtn <mount>/<first *.app> <userData>/updates/<v>/Nodus.app
/usr/bin/hdiutil detach <mount> -force
/usr/bin/codesign --verify --deep --strict <app>
/usr/bin/plutil -extract CFBundleIdentifier raw -o - <app>/Contents/Info.plist
/usr/bin/plutil -extract CFBundleShortVersionString raw -o - <app>/Contents/Info.plist
/usr/bin/codesign -dv <app>          # TeamIdentifier=… parsed from stderr
/bin/rm -rf <path>                   # every macOS delete (Electron's fs can't delete app.asar)
/bin/sh -c <SWAP_SCRIPT> nodus-update <8 args>
```

`execFile` semantics: UTF-8, and a non-zero exit is an error. On the macOS this was written on (Darwin 27.2), `hdiutil attach … -nobrowse -readonly -mountpoint` and `hdiutil detach -force` print a deprecation warning on stderr (pointing at `diskutil image attach` / `diskutil eject force`) but still exit 0 and work.

### SWAP_SCRIPT (verbatim; the JS template literal starts with `\n` and ends with `\n`)

```sh

trap '' HUP INT TERM
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
app=$2 new=$3 old=$4
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then
    rm -rf "$old"
  else
    rm -rf "$app"
    mv "$old" "$app"
    echo "$6" > "$5"
  fi
else
  echo "$6" > "$5"
fi
[ -n "$7" ] || exit 0
if [ -n "$8" ]; then open -n --env "NODUS_USER_DATA=$8" "$app"; else open -n "$app"; fi
```

Invocation: `spawn('/bin/sh', ['-c', SWAP_SCRIPT, 'nodus-update', pid, bundlePath, staged.app, updates/previous.app, updates/failed, version, relaunch ? '1' : '', relaunch ? (NODUS_USER_DATA ?? '') : ''], { detached: true, stdio: 'ignore' }).unref()`. `$0` is `nodus-update`. `detached` on POSIX means `setsid()`, a new session and process group.

### Windows installer invocation (verbatim)

`spawn(<userData>\updates\Nodus-Setup-<v>.exe, ['--updated', '/S', '--force-run'], { detached: true, stdio: 'ignore' })`. Node's `detached` on Windows means `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`. "Started" means the `spawn` event (CreateProcess succeeded), and a failure is the `error` event. The upstream comment reads: "/S installs without asking, --updated keeps the shortcuts as they are and --force-run opens Nodus after."

### electron-builder.yml (verbatim, the parts that shape the artifacts)

```yaml
appId: dev.saleem.nodus
productName: Nodus
directories: { output: dist, buildResources: build }
mac:
  target: [{ target: dmg, arch: [arm64, x64] }]
  artifactName: Nodus-${arch}.${ext}
  category: public.app-category.developer-tools
  hardenedRuntime: true
  entitlements: build/entitlements.mac.plist
  entitlementsInherit: build/entitlements.mac.plist
  extendInfo:
    NSMicrophoneUsageDescription: Nodus listens only while you record with the mic button, to turn what you say into a prompt.
    NSAppleEventsUsageDescription: Floating capture reads the link and title of the browser tab in front, to send along with what you write.
dmg: { writeUpdateInfo: false }
win:
  target: [{ target: nsis, arch: [x64, arm64] }]
  icon: build/icon-win.png
nsis:
  oneClick: true
  perMachine: false
  artifactName: Nodus-Setup-${arch}.${ext}
  shortcutName: Nodus
  differentialPackage: false
  deleteAppDataOnUninstall: false
```

`package.json`: `"name": "nodus"` (sets the Windows install folder `nodus` and `APP_PACKAGE_NAME`), `"productName": "Nodus"`, `"version": "0.9.4"`, `"description": "A spatial board for managing Claude Code sessions per project"` (NSIS Comments, shortcut description, installer FileDescription), `"author": "Saleem Hadad"` (Publisher, CompanyName, copyright), `"license": "MIT"`.

### build/entitlements.mac.plist (verbatim keys)

```xml
<key>com.apple.security.cs.allow-jit</key><true/>
<key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
<key>com.apple.security.device.audio-input</key><true/>
<key>com.apple.security.automation.apple-events</key><true/>
```

The file's comment says V8 needs the two `cs.*` keys, the prompt bar's mic needs audio-input, and floating capture's browser-tab reading needs apple-events. Library validation stays on, so there is no `disable-library-validation`. The installed 0.9.4 app and its helpers carry exactly these four entitlements.

### Throwaway keychain (release.yml, verbatim; secret names only)

```sh
keychain="$RUNNER_TEMP/signing.keychain-db"
password=$(openssl rand -base64 32)
printf '%s' "$CSC_LINK" | base64 --decode > "$RUNNER_TEMP/cert.p12"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$RUNNER_TEMP/cert.p12" -k "$keychain" -P "$CSC_KEY_PASSWORD" -T /usr/bin/codesign -T /usr/bin/productbuild
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
rm "$RUNNER_TEMP/cert.p12"
```

The workflow comment explains why it exists: electron-builder 26 unlocks its own keychain with the certificate's password instead of the keychain's, and macOS refuses. The Rust workflow keeps this step as it is, since `codesign` finds the identity in the search list.

### HANDOFF CONTRACT: what the first Rust release must satisfy

Each item names the 0.9.4 code that depends on it. "Parity" marks items nothing checks programmatically but users or macOS/Windows would notice.

#### A. The GitHub release (both platforms)

1. Tag `v<X.Y.Z>`: three numeric parts, no prerelease or build suffix (`/^v?(\d+\.\d+\.\d+)$/`). Otherwise every installed copy reports `latest` forever. (UPD-022/023)
2. `X.Y.Z` is newer than `0.9.4` by numeric major.minor.patch. `0.10.0` counts as newer; `0.9.4.1` isn't allowed. (UPD-025/026)
3. The release is published, not a draft and not marked prerelease, and is the one `releases/latest` returns. Don't publish an Electron release with a higher tag after it unless that's a deliberate rollback, and note that a rollback needs a still higher version. (UPD-016)
4. Assets named exactly `Nodus-arm64.dmg`, `Nodus-x64.dmg`, `Nodus-Setup-x64.exe`, `Nodus-Setup-arm64.exe`, each in `state: "uploaded"`. Apple-silicon Macs running the Intel copy, and x64 copies on Windows on Arm, get the **arm64** asset, so both arm64 assets must work as an in-place replacement for an x64 install. (UPD-020/021)
5. Each asset has GitHub's `digest` (`sha256:<64 lowercase hex>`). GitHub computes this for new uploads. **Windows refuses an asset without it**; macOS checks size only. `size` equals the file's byte length. (UPD-024, UPD-033, UPD-049)
6. Each asset is downloadable unauthenticated from `browser_download_url`, through redirects, with no stall of 60 s or longer. (UPD-030/031)

#### B. macOS DMG

7. Attaches without prompts or errors via `hdiutil attach <dmg> -nobrowse -noautoopen -readonly -mountpoint <dir>`. That rules out a licence agreement (SLA), encryption and an internet-enable prompt. 0.9.4 ships UDZO + HFS+, and APFS would also attach, but parity says UDZO + HFS+. (UPD-040)
8. Exactly one `*.app` at the volume root (0.9.4 takes the first `readdir` entry ending in `.app`). Name it `Nodus.app`. The updater doesn't need that name, since it copies to `updates/<v>/Nodus.app` and the swap moves it to wherever the running app is, but manual installs do (parity). (UPD-041)
9. The `.app` survives `ditto --noqtn` intact: no symlinks pointing outside the bundle, no files that would break the sealed resources. (UPD-042)
10. Parity layout: volume name `Nodus <X.Y.Z>-<arm64|x64>`, `.VolumeIcon.icns` = `build/icon.icns`, the 540×380 background, the `Applications` symlink, and the Finder layout in REL-011. The DMG itself stays unsigned and unstapled.

#### C. macOS app bundle (the copy 0.9.4 verifies and then swaps in)

11. Executable at `Contents/MacOS/<CFBundleExecutable>`. The Rust app derives its own bundle as `<exe>/../../..` the same way, so the executable must sit directly in `Contents/MacOS`. Keep `CFBundleExecutable` = `Nodus` (parity: the process name shown in Activity Monitor and permission prompts).
12. `Contents/Info.plist` readable by `plutil -extract … raw`: `CFBundleIdentifier` = `dev.saleem.nodus` (string) and `CFBundleShortVersionString` = `X.Y.Z` exactly (string, matching the tag without `v`; `X.Y` would fail). (UPD-045)
13. `codesign --verify --deep --strict` passes. Every Mach-O in the bundle is signed (sign inner code first, or sign once if it's the single binary), there are no unsealed files, and the bundle is not modified after signing (stapling is allowed). (UPD-044)
14. Signed with a **Developer ID Application** certificate from the **same team** as 0.9.4: `codesign -dv` must print `TeamIdentifier=<TEAM_ID>`, where `<TEAM_ID>` is the Team ID of the certificate in secret `CSC_LINK` (the same value as secret `APPLE_TEAM_ID`; check with `codesign -dv /Applications/Nodus.app` on a 0.9.4 install). An ad-hoc or differently-teamed signature is rejected by every 0.8.1+ copy. (UPD-046)
15. The signing identifier is `dev.saleem.nodus`, so the designated requirement equals 0.9.4's: `identifier "dev.saleem.nodus" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = <TEAM_ID>`. The updater doesn't check this, but TCC grants (mic, Accessibility, Screen Recording, Automation) and the `Nodus Safe Storage` Keychain item's access list are tied to it. (REL-046)
16. Hardened runtime on, with entitlements including at least `com.apple.security.device.audio-input` and `com.apple.security.automation.apple-events`. Without them the hardened runtime blocks the mic and Apple Events.
17. Notarized, with the ticket stapled to the `.app` (parity: manual downloads must pass Gatekeeper). 0.9.4's own copy has no quarantine (`--noqtn`), so Gatekeeper never assesses the swapped-in app. Notarization is for everyone who downloads by hand.
18. Thin per-arch binaries, deployment target 13.0 (`MACOSX_DEPLOYMENT_TARGET=13.0`, `LSMinimumSystemVersion` `13.0`). A higher minimum would let 0.9.4 swap in an app that can't launch on a supported macOS, and nothing checks for that.
19. Launchable by LaunchServices with `open -n "<bundle>"` and `open -n --env "NODUS_USER_DATA=<dir>" "<bundle>"`. The new process isn't a child of the old one. (UPD-063, REL-042)
20. On first launch it cleans up a 0.9.4 `updates/` folder (REL-043), reads `state.json` and the Keychain key (01, D6), and adopts the 0.9.4 Node session host, which keeps running even though its executable was in the deleted old bundle (02).

#### D. Info.plist: every key in the 0.9.4 bundle (33 entries), and what to do with each

| Key | 0.9.4 value | Source | Depended on by | Rust |
| --- | --- | --- | --- | --- |
| `CFBundleIdentifier` | `dev.saleem.nodus` | appId | **0.9.4 updater**, TCC, Keychain, LaunchServices | keep |
| `CFBundleShortVersionString` | `0.9.4` | version | **0.9.4 updater** | keep (= tag version) |
| `CFBundleVersion` | `0.9.4` | version | macOS (build number) | keep (= version) |
| `CFBundleExecutable` | `Nodus` | productName | macOS launch | keep |
| `CFBundleName` | `Nodus` | productName | menu bar app name, Dock | keep |
| `CFBundleDisplayName` | `Nodus` | productName | Finder, permission prompts | keep |
| `CFBundleIconFile` | `icon.icns` | electron-builder (copies `build/icon.icns` to `Contents/Resources/icon.icns`) | Finder/Dock icon | keep |
| `CFBundlePackageType` | `APPL` | Electron template | macOS | keep (+ `Contents/PkgInfo` = `APPL????`) |
| `CFBundleInfoDictionaryVersion` | `6.0` | Electron template | macOS | keep |
| `LSMinimumSystemVersion` | `13.0` | Electron 44 template | macOS launch gate | keep `13.0` |
| `LSApplicationCategoryType` | `public.app-category.developer-tools` | `mac.category` | Finder/App Store category | keep |
| `NSHumanReadableCopyright` | `Copyright © 2026 Saleem Hadad` | electron-builder default `Copyright © ${year} ${author}` | About panel | keep (build year) |
| `NSMicrophoneUsageDescription` | `Nodus listens only while you record with the mic button, to turn what you say into a prompt.` | extendInfo | **TCC** (the app is killed on mic access without it) | keep verbatim |
| `NSAppleEventsUsageDescription` | `Floating capture reads the link and title of the browser tab in front, to send along with what you write.` | extendInfo | **TCC** (Automation prompt) | keep verbatim |
| `NSCameraUsageDescription` | `This app needs access to the camera` | Electron template | TCC, if a view page (WKWebView) ever asks for the camera | keep verbatim (avoids a TCC kill) |
| `NSAudioCaptureUsageDescription` | `This app needs access to audio capture` | Electron template | TCC (system-audio capture) | keep verbatim |
| `NSBluetoothAlwaysUsageDescription` | `This app needs access to Bluetooth` | Electron template | TCC | keep verbatim |
| `NSBluetoothPeripheralUsageDescription` | `This app needs access to Bluetooth` | Electron template | TCC | keep verbatim |
| `NSHighResolutionCapable` | `true` | Electron template | Retina rendering | keep |
| `NSSupportsAutomaticGraphicsSwitching` | `true` | Electron template | lets dual-GPU Macs stay on the integrated GPU | keep |
| `NSRequiresAquaSystemAppearance` | `false` | Electron template | dark mode | keep |
| `NSQuitAlwaysKeepsWindows` | `false` | Electron template | no window state restoration | keep |
| `NSPrefersDisplaySafeAreaCompatibilityMode` | `false` | Electron template | notch handling | keep |
| `NSAppTransportSecurity` | `NSAllowsArbitraryLoads` true, `NSAllowsLocalNetworking` true, `NSExceptionDomains` `localhost` and `127.0.0.1` each with `NSIncludesSubdomains` false, `NSTemporaryExceptionAllowsInsecureHTTPLoads` true, `NSTemporaryExceptionAllowsInsecureHTTPSLoads` false, `NSTemporaryExceptionMinimumTLSVersion` `1.0`, `NSTemporaryExceptionRequiresForwardSecrecy` false | electron-builder `configureLocalhostAts` | WKWebView loads in view pages (04); not Chromium | keep verbatim, so view pages can load http |
| `LSEnvironment` | `{ MallocNanoZone: "0" }` | electron-builder (Electron allocator workaround) | Electron only | drop (or keep, harmless) — decision for 01 |
| `NSPrincipalClass` | `AtomApplication` | Electron | Electron's NSApplication subclass | **drop** (GPUI creates its own NSApplication; a missing class must not be named) |
| `NSMainNibFile` | `MainMenu` | Electron | Electron's nib | **drop** (no nib in the Rust bundle) |
| `ElectronAsarIntegrity` | `{ "Resources/app.asar": { algorithm: SHA256, hash } }` | electron-builder | Electron only | **drop** |
| `DTCompiler` | `com.apple.compilers.llvm.clang.1_0` | Electron's Xcode build | nothing at runtime | optional |
| `DTSDKBuild` | `24F74` | ditto | nothing at runtime (behavior follows LC_BUILD_VERSION's SDK) | optional |
| `DTSDKName` | `macosx15.5` | ditto | nothing at runtime | optional |
| `DTXcode` | `1640` | ditto | nothing at runtime | optional |
| `DTXcodeBuild` | `16F6` | ditto | nothing at runtime | optional |

Not present, and not to be added: `LSUIElement` (the app has a Dock icon; the `host` subcommand hides its own at runtime, 02/D5), `CFBundleURLTypes`, `CFBundleDocumentTypes`, `SUFeedURL` or any Sparkle key. electron-builder also wrote `Contents/Resources/app-update.yml` (`owner: saleem-hadad`, `repo: nodus`, `provider: github`, `updaterCacheDirName: nodus-updater`). Nothing in the app reads it, so drop it.

The 0.9.4 bundle also contains Electron-only `Frameworks/` (Electron Framework, Mantle, ReactiveObjC, Squirrel, four `Nodus Helper*.app`) and `Resources/` (`app.asar`, `app.asar.unpacked/node_modules/{node-pty,uiohook-napi}`, ~60 `*.lproj`). None of that is checked by the updater, and none of it is needed in Rust. Note that the empty `*.lproj` folders make macOS list the app as localized in those languages. That has no visible effect in an English-only UI, so dropping them is safe. If `en.lproj` is dropped too, set `CFBundleDevelopmentRegion` = `en`.

#### E. Windows installer (`Nodus-Setup-<arch>.exe`, run by 0.9.4 as `--updated /S --force-run`)

21. A Windows PE that `CreateProcess` starts without elevation (`asInvoker` / NSIS `RequestExecutionLevel user`). A `requireAdministrator` manifest fails `spawn` with ERROR_ELEVATION_REQUIRED, which shows `The installer didn’t start…` (UPD-069).
22. With `/S`: no UI, no prompts. Unknown flags must not fail the install.
23. Installs into the existing install's folder: read `HKCU\Software\99c2a492-c8ad-53bf-923b-1df868e91b51` → `InstallLocation`, defaulting to `<FOLDERID_UserProgramFiles>\nodus` (= `%LOCALAPPDATA%\Programs\nodus`). The GUID is UUIDv5(`dev.saleem.nodus`, namespace `50e065bc-3134-11e6-9bab-38c9862bdaf3`), as electron-builder derives it, and it must not change.
24. Closes leftover processes whose image path is under the install folder (REL-019). 0.9.4 has already stopped its sessions and quit, and the session host also runs from `Nodus.exe`.
25. Removes the Electron files by running the **old** `Uninstall Nodus.exe` from the `UninstallString` with `/S /KEEP_APP_DATA /currentuser --keep-shortcuts --updated _?=<dir>` (REL-020). The old uninstaller moves every file out (and aborts and restores if one is busy), keeps shortcuts and AUMID registration because of `--keep-shortcuts`, keeps `%APPDATA%\Nodus` because there's no `--delete-app-data`, and deletes its registry keys. If the Rust installer only copied files over the old ones, `resources\app.asar`, `locales\`, the Chromium DLLs and the old uninstaller would be left behind.
26. Installs `Nodus.exe` (the Rust binary, same name and folder, so existing shortcuts and taskbar pins still work) and `Uninstall Nodus.exe` (a Rust-era uninstaller with the same semantics, so the next update can run it as in 25).
27. Rewrites the registry values in REL-021 so the next update finds them.
28. Keeps shortcuts: on `--updated` with `KeepShortcuts` = `true` and `Nodus.exe` present, it leaves `Nodus.lnk` (Start menu, Desktop) alone. On a fresh install it creates them with AUMI `dev.saleem.nodus`, which the app also sets as its AppUserModelID so Windows matches notifications to the shortcut (01/02).
29. With `--force-run`, starts the app after installing, through the Start menu shortcut, with argument `--updated` (REL-024). The Rust app must accept and ignore that argument (REL-045) and must not take it for a subcommand (D5's `host`).
30. Exit code 0 on success, so the template's `quitSuccess` runs. Nothing reads the code: 0.9.4 doesn't wait for the installer.
31. Leaves `%APPDATA%\Nodus` (state.json, views, captures, host folder, `updates\`) untouched. The installer is running from `%APPDATA%\Nodus\updates\`, so it can't be deleted there.
32. Parity extras from electron-builder: the installer copies itself to `%LOCALAPPDATA%\nodus-updater\installer.exe`; it holds the single-instance mutex `99c2a492-c8ad-53bf-923b-1df868e91b51`; `BrandingText` `Nodus <v>`; `Name` `Nodus`; version info ProductName `Nodus`, ProductVersion `<v>`, FileVersion `<v>`, LegalCopyright `Copyright © <year> Saleem Hadad`, FileDescription `A spatial board for managing Claude Code sessions per project`, CompanyName `Saleem Hadad`; installer and uninstaller icons from `build/icon-win.png`.
33. Unsigned, as 0.9.4 is (REL-027). 0.9.4 doesn't check the installer's Authenticode signature, only the digest. Adding a signature would be allowed but is a change.

#### F. What electron-builder did implicitly, and whether anything depends on it

| Implicit behavior | Observed | Depends on it | Rust action |
| --- | --- | --- | --- |
| Info.plist from Electron's template plus electron-builder defaults | table D | updater: id + version only; macOS: TCC usage strings, `LSMinimumSystemVersion`, `NSHighResolutionCapable`, ATS for WKWebView | write a checked-in Info.plist template with the "keep" rows |
| `ElectronAsarIntegrity` | per-build hash | Electron's asar integrity fuse only | drop |
| `LSMinimumSystemVersion` 13.0, binary `minos 13.0` | Electron 44 | macOS refuses to launch on older systems | deployment target 13.0 |
| Copyright string | `Copyright © <build year> Saleem Hadad` | About panel, exe version info | compute the same way |
| Icon copy | `build/icon.icns` → `Contents/Resources/icon.icns`, `CFBundleIconFile` | Finder, Dock | same |
| Signing order (helpers, frameworks, then app), hardened runtime, timestamp, entitlements and inherit entitlements | `@electron/osx-sign` | `codesign --verify --deep --strict` in 0.9.4 | `codesign --force --timestamp --options runtime --entitlements … --sign "Developer ID Application"` on each nested Mach-O, then the app |
| Notarize the app (`notarytool`), staple the app, then build the DMG | electron-builder `notarize` | Gatekeeper for manual installs | `ditto -c -k --keepParent` → `notarytool submit --wait` → `stapler staple` → dmgbuild |
| DMG via vendored `dmgbuild` (Python), UDZO, HFS+, level 9, default background/layout | dmg-builder 26.15.3 | `hdiutil attach` in 0.9.4; Finder look | dmgbuild with the same settings JSON |
| `dmg.sign` off | DMG unsigned | nothing | keep unsigned |
| `app-update.yml` in Resources | written even with `writeUpdateInfo: false` | nothing (no electron-updater) | drop |
| NSIS GUID from appId | `99c2a492-c8ad-53bf-923b-1df868e91b51` | **registry lookups by the new installer, old uninstaller handoff** | hard-code |
| Install dir name = package `name` (`nodus`), not productName | `getWindowsInstallationDirName(appInfo, false)` → `sanitizedName` | **0.9.4 install location**, CI smoke path | hard-code `nodus` |
| `--updated` passed to the app after an update | `StartApp` macro | app argv | ignore it in the app |
| Uninstaller name `Uninstall Nodus.exe` | `UNINSTALL_FILENAME` | `UninstallString` | same |
| rcedit version strings and icon on `Nodus.exe` | REL-028 | `DisplayIcon` `Nodus.exe,0`, Explorer properties | `winresource`/`embed-resource` |
| Installer copies itself to `%LOCALAPPDATA%\nodus-updater\installer.exe` | `APP_INSTALLER_STORE_FILE` | nothing | optional (parity) |
| Per-user AppUserModelID on shortcuts | `WinShell::SetLnkAUMI … dev.saleem.nodus` | Windows toast attribution (01/02) | same |

## API surface

Electron (the spec):

| Channel | Direction | Shape | Handler |
| --- | --- | --- | --- |
| `updates:status` | invoke | `() => UpdateStatus` | `updater.getStatus()` |
| `updates:check` | invoke | `() => Promise<void>`; resolves after the lookup | `updater.check(true)` |
| `updates:install` | invoke | `() => Promise<void>` | `updater.restartToUpdate()` |
| `ev:update-status` | event | `(status: UpdateStatus)` | `startUpdater(notify)` → `send(IPC.evUpdateStatus, status)` |
| `settings:update` | invoke | side effect: truthy `patch.autoUpdate` → `updater.check()` | ipc.ts (01 owns the handler) |
| menu `Check for Updates…` | — | `send('settings')`, then `check(true)` | menu.ts (01 owns the menu as a whole) |

Module exports (`updater.ts`): `getStatus(): UpdateStatus`, `check(asked = false): Promise<void>`, `restartToUpdate(): Promise<void>`, `startUpdater(notify: (s: UpdateStatus) => void): void`.

Rust (`nodus-updater`, surfaced through `nodus-core`):

```rust
pub struct Updater<E: UpdaterEnv> { … }
impl<E: UpdaterEnv> Updater<E> {
    pub fn start(&self, notify: impl Fn(UpdateStatus) + Send + Sync + 'static); // UPD-003..012
    pub fn status(&self) -> UpdateStatus;                                       // updates:status
    pub async fn check(&self, asked: bool);                                     // updates:check → true
    pub async fn restart_to_update(&self);                                      // updates:install
    pub fn on_will_quit(&self);                                                 // macOS swap (UPD-057); 01 calls it from the quit path
}
pub trait UpdaterEnv: Send + Sync + 'static {
    fn version(&self) -> String;                    // = CARGO_PKG_VERSION = CFBundleShortVersionString
    fn user_data(&self) -> PathBuf;                 // 01
    fn exe_path(&self) -> PathBuf;                  // bundle = exe/../../..
    fn platform(&self) -> Platform;                 // Mac | Windows | Other
    fn is_packaged(&self) -> bool;                  // see Rust mapping
    fn arm64_machine(&self) -> bool;                // cfg(aarch64) || Rosetta (sysctl.proc_translated) || IsWow64Process2 native ARM64
    fn is_in_applications_folder(&self) -> bool;    // nodus-platform
    fn auto_update(&self) -> bool;                  // settings.autoUpdate, read per tick
    fn live_session_count(&self) -> usize;          // 02 liveSessionIds().len()
    async fn stop_all(&self) -> Result<()>;         // 02 stopAll()
    async fn ask_stop_sessions(&self, n: usize) -> bool; // native dialog, UPD-066
    fn quit(&self);                                 // 01 app.quit() equivalent (runs will-quit)
    fn http(&self) -> &dyn Http;                    // GET json / GET stream, injectable for tests
    fn run(&self, cmd: &str, args: &[&str]) -> Result<Output>; // execFile; injectable
    fn spawn_detached(&self, cmd: &Path, args: &[OsString]) -> Result<()>; // injectable
}
```

`nodus-core` exposes `updates_status()`, `updates_check()` and `updates_install()`, and emits `Event::UpdateStatus(UpdateStatus)`, mirroring `window.api.updates.*` and `api.on.updateStatus`.

## UI details

Title bar (`UpdateButton`, placed in `.titlebar-actions` after the Focus switch and before the views' own buttons; layout owned by 08):

| State | Rendered |
| --- | --- |
| anything but `ready`, or no status | nothing |
| `ready` | `<button class="btn btn-primary">` with RotateCw (14, stroke 2.2) and `Restart to Update` |

Tooltip (`title`):
- Mac: `Nodus <version> is ready. Sessions keep running while Nodus restarts.`
- Windows: `Nodus <version> is ready. Restarting stops the sessions; you can resume them after.`

Settings › `Updates` (`fieldset.settings-group`, `legend.settings-label`; last group in the dialog, after Voice; layout owned by 08):

| Status | Row 1 title | Row 1 hint (`.settings-row-hint`) | Row 1 button | Row 2 (switch) | Error paragraph |
| --- | --- | --- | --- | --- | --- |
| null (loading) | `Nodus ` | — | `Check Now` disabled | shown; disabled until settings load | — |
| `off` | `Nodus <current>` | `<reason>` | — | hidden | — |
| `idle` | `Nodus <current>` | — | `Check Now` | shown | — |
| `checking` | `Nodus <current>` | `Checking for updates…` | `Check Now` disabled | shown | — |
| `latest` | `Nodus <current>` | `You’re up to date.` | `Check Now` | shown | — |
| `downloading` | `Nodus <current>` | `Downloading version <v>… <percent>%` | `Check Now` disabled | shown | — |
| `ready` | `Nodus <current>` | `Version <v> is ready to install.` | `Restart to Update` (`btn btn-primary`) | shown | — |
| `error` | `Nodus <current>` | `<message>` | `Check Now` | shown | `You can also ` + link `download the latest version` + ` yourself.` (`p.settings-hint`) |

`Check Now` is `<button type="button" class="btn">`. The switch label is `Download updates automatically`, with hint `They install when you quit Nodus. Sessions keep running.` (Mac) or `They install when you choose Restart to Update, which stops the sessions.` (Windows). The link has `target="_blank"`, and the main window's open handler sends `https:` URLs to the default browser.

Menu: `Check for Updates…` (with `…`, U+2026), no accelerator. On macOS it's in the app menu: About, **Check for Updates…**, separator, Settings…, … On Windows and Linux it's in File: …, Close, separator, Settings…, **Check for Updates…**, separator, Exit, Quit and Stop Sessions.

Windows dialog (Electron `dialog.showMessageBox`, no parent window): type `question` (Windows shows the same icon as info), message, detail and buttons as UPD-066, `defaultId` 0, `cancelId` 1. unclear: the window title Electron gives a parentless TaskDialog with no `title` (probably the app name, `Nodus`); check on a 0.9.4 install (src/main/updater.ts:362).

## Platform differences

| Topic | macOS | Windows |
| --- | --- | --- |
| Asset | `Nodus-{arm64,x64}.dmg` | `Nodus-Setup-{x64,arm64}.exe` |
| Off reasons | platform, unpackaged, bundle or parent not writable (two messages) | platform, unpackaged only |
| Digest | optional (size always checked) | required (`GitHub had no checksum…`) |
| Verification | codesign deep strict, bundle id, version, team | digest only (installer unsigned) |
| Staged item | `updates/<v>/Nodus.app` | `updates/Nodus-Setup-<v>.exe` |
| Kept across launches | nothing; clean-up deletes everything but `failed` | newest newer installer (re-verified by size + SHA-256 before reuse) |
| Install moment | on any quit (swap script); Restart to Update also reopens | only Restart to Update; never on quit |
| Sessions | keep running in the host throughout | dialog asks, then `stopAll()`; resumable from cards |
| Failure memory | `updates/failed` → no automatic re-download of that version | none; the installer is deleted and Check for Updates downloads again |
| Delete primitive | `/bin/rm -rf` | `fs.rm(recursive, force)` |
| Hints and tooltips | "Sessions keep running" wording | "stops the sessions" wording |
| Menu location | app menu after About | File menu after Settings… |
| Release job | signs, notarizes; runs the tests | no tests; unsigned |
| CI | build + tests | build, x64 installer, silent install, smoke |

Linux: `Updates are only built for macOS and Windows.` There is no release asset.

## Edge cases & errors

Verbatim user-visible messages (all shown as the Settings hint, state `error` unless noted):

| Message | When |
| --- | --- |
| `Updates are only built for macOS and Windows.` | off reason, other OS |
| `Development builds don’t update themselves.` | off reason, unpackaged |
| `Your account can’t change apps in Applications, so Nodus can’t update itself.` | off reason, Mac, not writable, in Applications |
| `Move Nodus to your Applications folder so it can update itself.` | off reason, Mac, not writable, elsewhere |
| `Couldn’t reach GitHub to check for updates.` | lookup request threw or timed out before headers |
| `GitHub is limiting update checks right now. Try again later.` | 403/429 |
| `GitHub answered the update check with an error (<status>).` | other non-2xx |
| `Nodus <version> downloaded but couldn’t install itself.` | failed version, automatic check |
| `GitHub answered the download with an error (<status>).` | download non-OK or no body |
| `The update came through damaged. Try again later.` | size or SHA-256 mismatch |
| `The update didn’t finish downloading. Check your connection.` | any other download failure, including the 60 s stall |
| `The update has no app in it.` | no `*.app` in the DMG |
| `The new version’s signature didn’t check out.` | `codesign --verify --deep --strict` failed |
| `The update isn’t the expected version of Nodus.` | bundle id or short version mismatch |
| `The update isn’t signed by the same developer.` | team mismatch |
| `GitHub had no checksum for the update, so it wasn’t downloaded.` | Windows, no digest |
| `The update couldn’t be prepared. Try again later.` | any other preparation error (hdiutil, ditto, plutil, codesign -dv, mkdir…) |
| `The installer didn’t start, so Nodus wasn’t updated. Try Check for Updates again.` | Windows install failed to start, installer missing, or `stopAll` threw |
| Dialog: `Stop the running session to update?` / `Stop <n> running sessions to update?`, `Windows can’t update Nodus while its sessions run. You can resume them from their cards afterwards.`, `Stop and Update`, `Not Now` | Windows Restart to Update with live sessions |

Console-only logs: `[updater] update failed`, `[updater] detach failed`, `[updater] the installer didn’t start`, `[updater] cleanup failed`.

Edge behaviors (each covered by an ID above, or noted as unclear):
- A malformed JSON body, or the 20 s timeout firing while the body is read, throws out of `latestRelease()` as a raw JS error. `check()` then shows its `.message` verbatim (for example a `SyntaxError` text, or `The operation was aborted due to timeout`). unclear: Rust can't reproduce V8's exact wording; propose showing `Couldn’t reach GitHub to check for updates.` for timeouts and the serde error text for bad JSON, and record the choice (src/main/updater.ts:151, 325).
- HTTP 200 with no body on the download gives `GitHub answered the download with an error (200).` (UPD-033).
- Asset `size` 0 makes the percent `min(100, floor(x/0*100))`, so 100 on the first chunk (or `NaN` when 0/0). Harmless; mirror with saturating math.
- While an update is staged, later checks do nothing (UPD-015). A release published after staging is picked up only after a restart.
- `app.exit()` paths (for example `replaceHost` → Quit in pty.ts) skip `will-quit`, so no swap happens on that exit (src/main/pty.ts:314; 02). The staged copy in `updates/<v>/` is deleted by the next launch's clean-up.
- A swap that fails both ways (can't move the old app aside) still reopens when relaunching, and the reopened copy reports `Nodus <v> downloaded but couldn’t install itself.` on its next automatic check (UPD-062/063/071/027).
- `failed` older than or equal to the running version is deleted by clean-up (UPD-071/074).
- Windows: a kept installer for a version that's no longer the latest stays until a newer installer is kept or the app catches up; it's never run unless it's the latest release (UPD-050, UPD-072).
- Windows: Restart to Update with zero live sessions shows no dialog but still calls `stopAll()`, which stops the host.
- Windows: if the new installer can't remove the old version (a file in use), the electron-builder template retries 5 times, then shows `$(appCannotBeClosed)` (`/SD IDCANCEL` when silent) and `$(uninstallFailed): <code>`, sets exit code 2 and quits. The app was already quit by 0.9.4 and isn't restarted. unclear: whether the `uninstallFailed` MessageBox (no `/SD`) shows under `/S` (app-builder-lib `templates/nsis/include/installUtil.nsh:129`).
- macOS: the updater makes no assumption about where the running app is installed (`/Applications`, `~/Applications`, anywhere writable). The swap replaces it in place.
- macOS: `app.isInApplicationsFolder()` (Electron `ElectronBundleMover`) is true when the bundle path starts with any `NSApplicationDirectory` in any domain (`/Applications`, `~/Applications`, `/Network/Applications`) or has a path component named `Applications`. unclear: confirm against Electron 44's `shell/browser/ui/cocoa/electron_bundle_mover.mm` before porting (src/main/updater.ts:88).
- The digest regex accepts lowercase hex only. An uppercase digest counts as missing (Windows then refuses; macOS checks size only).
- `isNewer` treats `1.2.3abc`-style parts as parseInt prefixes. Only the clean `x.y.z` from the tag regex and the file regex ever reaches it, apart from the running version and the `failed` file contents.

## Tests to port

Harness (`src/main/updater.test.ts`): Electron is mocked with `getVersion` `'0.8.9'`, `getPath` → a fresh temp userData per test, `isPackaged: true`, `runningUnderARM64Translation: false`, `quit` mock and `on` mock. `dialog.showMessageBox` is a mock. `./pty` gives `liveSessionIds` (default `[]`) and `stopAll` (resolves). `./store` has `autoUpdate: false`, so no timer ever checks. `child_process.spawn` is a mock. `process.platform` is forced to `win32` for the whole file. `fetch` is stubbed to serve GitHub: the latest release `v0.9.0` with one asset `ASSET` = `Nodus-Setup-<arm64 when process.arch is arm64, else x64>.exe`, `state: 'uploaded'`, `size` = the length of `Buffer('the 0.9.0 installer')`, `digest` = `sha256:<hex of that buffer>`, `browser_download_url` = `https://github.com/saleem-hadad/nodus/releases/download/v0.9.0/<ASSET>`. The download URL returns the buffer and counts `downloads`. `serveRelease({ assets: false })` serves the release with no assets. `launch()` = fresh module + `startUpdater(() => {})`. `staged()` = `launch()`, `check(true)`, wait for `ready`. `child('spawn'|'error')` = an emitter that emits `spawn` or `error(new Error('blocked'))` on the next tick and has `unref`. `console.error` is silenced.

Rust: `nodus-updater/tests/windows.rs` with a fake `UpdaterEnv` (platform Windows, version 0.8.9, temp userData, auto_update false, in-memory HTTP serving the same bodies and counting downloads, recording spawner, scripted dialog, counters for stop_all/quit with call ordering).

describe **the installer between launches**
1. **is downloaded once, and kept for the next launch.** Two `staged()` launches in a row. `downloads` is 1 after the first and still 1 after the second, and `updates/Nodus-Setup-0.9.0.exe` contains exactly the installer bytes.
2. **is downloaded again when the kept one isn’t the release’s.** With `updates/Nodus-Setup-0.9.0.exe` = `half a download` beforehand, `staged()` gives `downloads` 1 and the file now equals the installer bytes.
3. **only the newest newer one is kept.** Release with no assets; `updates/` holds empty files `Nodus-Setup-0.8.0.exe`, `Nodus-Setup-0.8.9.exe`, `Nodus-Setup-0.9.0.exe`, `Nodus-Setup-0.9.1.exe` and `stray.tmp`. After `launch()` and `check(true)`, `readdir(updates)` equals exactly `['Nodus-Setup-0.9.1.exe']`.

describe **Restart to Update on Windows** (beforeEach: `liveSessionIds` → `['s1']`, `showMessageBox` → `{ response: 0 }`)
4. **stops the sessions and quits once Windows has started the installer.** With `spawn` → `child('spawn')`, `restartToUpdate()` calls `stopAll` once, calls `spawn` with `(<updates>/Nodus-Setup-0.9.0.exe, ['--updated', '/S', '--force-run'], { detached: true, stdio: 'ignore' })` and `quit` once, and `stopAll` comes before `spawn`.
5. **stays open and says so when the installer doesn’t start.** With `spawn` → `child('error')`, `restartToUpdate()` doesn't call `quit`, the status matches `{ state: 'error' }` and the installer file no longer exists. Then `check(true)` reaches `ready` again and `downloads` is 2.
6. **stops nothing when the installer has gone.** After `staged()` the installer file is deleted. `restartToUpdate()` shows no dialog, calls neither `stopAll` nor `quit`, and the status matches `{ state: 'error' }`.
7. **leaves the sessions running when the user says not now.** With dialog response 1, `restartToUpdate()` calls neither `stopAll` nor `spawn`, and the status matches `{ state: 'ready' }`.
8. **asks once when the button is clicked twice.** With `spawn` → `child('spawn')`, `Promise.all([restartToUpdate(), restartToUpdate()])` shows the dialog once and calls `spawn` once.

No macOS test exists in TypeScript. Recommended Rust additions, marked as such since they test existing behavior but aren't ports: `is_newer` table (UPD-025); `latest_release` parsing (UPD-020…024, every error status in UPD-017…019); download size, digest and stall (UPD-031…035, paused tokio clock); clean-up on macOS (UPD-071…074); the swap script run for real by `/bin/sh` in a temp dir with a dummy PID that exits (success, new-can't-move rollback, old-can't-move, relaunch flag recorded through a fake `open` on `PATH`) (UPD-059…063); the off reasons (UPD-004…009) with a fake env.

## Rust mapping

**Crate `nodus-updater`** (depends on `nodus-domain`, and on `nodus-platform` for `is_in_applications_folder`, `arm64_machine` and the native dialog; no GPUI):
- HTTP: `reqwest` (rustls, no system proxy surprises; redirects on, which is the default) on `tokio`. Lookup: `.timeout(20s)` on the whole request including the body. Download: `bytes_stream()` with `tokio::time::timeout(60s, next_chunk)` per chunk, plus a 60 s timeout on the initial response.
- Hashing: `sha2::Sha256` + `hex` (lowercase).
- JSON: `serde` with `Option` fields (`tag_name`, `assets`, `digest`) and unknown fields ignored.
- Commands: `std::process::Command::output()` with exact argv, where a non-zero status is an error and stderr is kept (team parse).
- Swap: `Command::new("/bin/sh").args(["-c", SWAP_SCRIPT, "nodus-update", …])`, stdio `Stdio::null()`, `pre_exec(|| { libc::setsid(); Ok(()) })` (Node's `detached` on POSIX), spawned and never awaited. Dropping a `Child` doesn't kill it. `SWAP_SCRIPT` is a `const &str`, byte-identical including the leading and trailing newline.
- Windows spawn: `CommandExt::creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)`, stdio null. `spawn()` returning `Ok` is the `spawn` event, and `Err` is the `error` event.
- Timers: `tokio::time::sleep(30s)` → check, and `interval_at(start + 4h, 4h)` with `MissedTickBehavior::Delay`, because Node timers don't burst after sleep. The busy guard prevents overlap anyway.
- State: a `Mutex<Inner { status, busy, staged, failed_version, relaunch, swapping, installing }>` and a `watch`/broadcast channel for `notify`. `cleaned` is a `Shared` future (or `OnceCell`) every check awaits.
- `is_packaged`: decision needed. Electron's `app.isPackaged` is false only when run through the `electron` binary. Proposed: a compile-time flag set by the packaging xtask (`NODUS_PACKAGED=1` → `option_env!`), so `cargo run`, including `--release`, reports `Development builds don’t update themselves.`
- `arm64_machine`: `cfg!(target_arch = "aarch64")`, or on macOS `sysctlbyname("sysctl.proc_translated") == 1`, or on Windows `IsWow64Process2` with native machine `IMAGE_FILE_MACHINE_ARM64`.
- Dialog (UPD-066): a native Windows TaskDialog (main instruction = message, content = detail, custom buttons `Stop and Update` / `Not Now`, default the first, Esc = the second). Share the helper with 02's `replaceHost` dialog in `nodus-platform`.
- Quit hook: 01 calls `updater.on_will_quit()` from the app's normal quit path (GPUI `on_app_quit` or the equivalent) **after** windows close and **before** the process exits. It must not block, and the process must then actually exit so `kill -0` turns false. It must not run on the `app.exit(0)` paths (edge cases).
- Deletion on macOS: keep `/bin/rm -rf` for parity. `std::fs::remove_dir_all` would also work in Rust, since the asar reason is gone, but parity wins.

**Packaging** (an `xtask` crate, `cargo xtask package`):
- macOS `.app`: hand-assembled, not cargo-bundle (unmaintained, limited plist control) and not cargo-packager (its own plist keys, and a create-dmg flow that drives Finder over AppleScript and differs from dmgbuild's output). Build `aarch64-apple-darwin` and `x86_64-apple-darwin` separately (thin, `MACOSX_DEPLOYMENT_TARGET=13.0`). Lay out `Nodus.app/Contents/{MacOS/Nodus, Info.plist (from packaging/macos/Info.plist template, table D "keep" rows, version substituted), PkgInfo "APPL????", Resources/icon.icns (= build/icon.icns)}`.
- Sign: `codesign --force --timestamp --options runtime --entitlements packaging/macos/entitlements.plist --sign "Developer ID Application" Nodus.app`. The identity is matched by type from the throwaway keychain, so no name or Team ID goes in the repo. Sign nested Mach-Os first if any are added.
- Entitlements: carry `build/entitlements.mac.plist` over. The two `cs.*` keys exist for V8; whether to keep them (no behavior change, weaker hardening) or drop them is a decision for the plan's Decisions table. audio-input and apple-events are required.
- Notarize: `ditto -c -k --keepParent Nodus.app Nodus.zip` → `xcrun notarytool submit Nodus.zip --apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait` → `xcrun stapler staple Nodus.app` → `xcrun stapler validate Nodus.app`. Skip when any of the three is unset, as electron-builder does.
- DMG: `dmgbuild` (pip, pinned; the same tool dmg-builder 26 vendors) with settings `{ title: "Nodus <v>-<arch>", icon: build/icon.icns, "text-size": 12, "icon-size": 80, "compression-level": 9, filesystem: "HFS+", format: "UDZO", background: packaging/macos/background.tiff (copied from dmg-builder 26.15.3 templates, MIT), window: { position: {x: 400, y: 530}, size: {width: 540, height: 380} }, contents: [{x:130,y:220,type:"file",path:"Nodus.app"},{x:410,y:220,type:"link",path:"/Applications"}] }`. Output `dist/Nodus-<arch>.dmg`.
- Windows: build `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` (cross on `windows-latest`, or a `windows-11-arm` runner). Embed the icon (an .ico from `build/icon-win.png` at 16/24/32/48/64/128/256) and the version info (REL-028) with `winresource`, plus a manifest (`asInvoker`, PerMonitorV2). unclear: Electron's own exe manifest beyond `asInvoker`; read it from a 0.9.4 `Nodus.exe` with a resource viewer.
- Installer: **vendor electron-builder 26.15.3's NSIS templates** (`templates/nsis/**`, MIT; keep the licence header) and the plugin set from electron-builder-binaries `nsis-resources-3.4.1` (StdUtils, UAC, WinShell, SpiderBanner, nsProcess; check each plugin's licence), and compile with `makensis` 3.x using the defines electron-builder passes. Those are: `APP_ID=dev.saleem.nodus`, `APP_GUID`/`UNINSTALL_APP_KEY=99c2a492-c8ad-53bf-923b-1df868e91b51`, `PRODUCT_NAME=Nodus`, `PRODUCT_FILENAME=Nodus`, `APP_FILENAME=nodus`, `APP_PRODUCT_FILENAME=Nodus`, `APP_PACKAGE_NAME=nodus`, `APP_DESCRIPTION=…`, `VERSION`, `COMPANY_NAME=Saleem Hadad`, `ONE_CLICK`, `RUN_AFTER_FINISH`, `SHORTCUT_NAME=Nodus`, `UNINSTALL_DISPLAY_NAME=Nodus <v>`, `APP_INSTALLER_STORE_FILE=nodus-updater\installer.exe`, `APP_64` or `APP_ARM64`, `MUI_ICON`/`MUI_UNICON`, `ESTIMATED_SIZE`. Leave out `DELETE_APP_DATA_ON_UNINSTALL`, `MENU_FILENAME` and `INSTALL_MODE_PER_ALL_USERS`. The generated flag macros (`isUpdated`, `isForceRun`, `isKeepShortcuts`, `isNoDesktopShortcut`, `isDeleteAppData`, …) come from `StdUtils.TestParameter` as `nsisScriptGenerator.flags()` writes them. Embed files with `APP_BUILD_DIR` (`File /r`) instead of the 7z archive (same result, fewer tools), and do the uninstaller as the same two passes (compile with `BUILD_UNINSTALLER`, run it to write `Uninstall Nodus.exe`, then compile the installer). Rejected: cargo-packager and Tauri NSIS, which use other registry keys, install into `%LOCALAPPDATA%\Nodus`, and have their own flag set (no `--updated`/`--force-run` semantics). They would strand the Electron install and its shortcuts. <!-- gitleaks:allow — public UUID v5 of the app id that electron-builder writes to every install's registry; not a secret -->

**CI / release workflows (Rust)**:
- `release.yml`: same trigger, permissions, jobs and attach commands. The tag check compares against the workspace version; the message wording names `package.json` today, so decide whether it should say `Cargo.toml` (unclear: .github/workflows/release.yml:25). The macOS job runs `cargo test` (all crates), the keychain script as it is, `cargo xtask package mac --arch arm64` and `--arch x64` with the three Apple secrets in that step only, a **handoff gate** (below), then upload `dist/*.dmg`. The Windows job installs NSIS (pinned), runs `cargo xtask package win --arch x64` and `--arch arm64`, then uploads `dist/Nodus-Setup-*.exe`. Pin actions by SHA and use `persist-credentials: false`.
- Handoff gate, macOS (new CI step, fails the release before upload). Replay 0.9.4's exact commands on each built DMG: `hdiutil attach … -nobrowse -noautoopen -readonly -mountpoint`, first `*.app`, `ditto --noqtn`, `codesign --verify --deep --strict`, the `plutil` id (`dev.saleem.nodus`) and version (= tag), the `codesign -dv` TeamIdentifier (= `$APPLE_TEAM_ID`, compared in the script, never printed), plus `lipo -archs` = the DMG's arch, `vtool -show-build` minos 13.0, `spctl -a -vv` accepted as notarized, and `stapler validate`.
- Handoff gate, Windows (new CI job). Download v0.9.4's `Nodus-Setup-x64.exe` from the release, install it with `/S`, seed `%APPDATA%\Nodus\state.json`, run the new installer with `--updated /S --force-run` and wait. Then assert: `%LOCALAPPDATA%\Programs\nodus\Nodus.exe` is the Rust build (version resource); no `resources\app.asar`; `Uninstall Nodus.exe` exists; HKCU keys have `DisplayVersion` = the new version; `Nodus.lnk` exists in the Start menu with AUMI `dev.saleem.nodus`; `state.json` is unchanged; a `Nodus.exe` process started with `--updated`. Then run the smoke test.
- `ci.yml`: replace the pnpm steps with `cargo fmt --check`, `cargo clippy`, `cargo test` (macOS and Windows) and `cargo xtask package win --arch x64` + silent install + smoke. Keep pnpm alongside until cutover (plan rule 6).
- Smoke: `scripts/smoke.mjs` drives Electron over the DevTools protocol (`--remote-debugging-port=9333`, `Runtime.evaluate` on `window.api`), which a GPUI app doesn't have. Its host-protocol half (REL-033/035) ports unchanged, as a Rust test binary or the same Node script without CDP. The window half (REL-032/034: project grid, terminal output `nodus-42`, light/dark screenshots, Ctrl+W behavior) needs another driver. Decision needed: (a) a test-only automation endpoint compiled behind a cargo feature that is off in releases, or (b) OS-level automation (Windows UI Automation, PowerShell screen capture plus a `nodus-core` command socket).
- `postinstall.cjs`: deleted at cutover (REL-037).

**Risks:**
1. **The Windows installer is the riskiest piece.** 0.9.4 hands control to whatever `Nodus-Setup-<arch>.exe` does with `--updated /S --force-run`. Unless it reproduces electron-builder's NSIS template exactly (GUID registry keys, `%LOCALAPPDATA%\Programs\nodus`, running the old uninstaller with `/KEEP_APP_DATA --keep-shortcuts --updated`, killing leftover processes, `--force-run` relaunch), users end up with two installs, stale Electron files, broken shortcuts or no running app. And because 0.9.4 already quit, nothing reports the failure. Vendoring the template and running the Windows handoff gate in CI are the mitigation.
2. **Signing identity.** Every 0.8.1+ Mac copy rejects an app that isn't signed by the same Team ID, and a different designated requirement silently drops TCC grants and Keychain access to the saved xAI key (D6). The release machine must use the same Developer ID certificate (`CSC_LINK`), with the identifier `dev.saleem.nodus`.
3. **No way to stage the handoff safely.** `LATEST_URL` is hard-coded and ignores drafts and prereleases, so the first real end-to-end run is the public release, and the replay gates have to stand in for it. A local end-to-end run would need intercepting `api.github.com` from the Electron main process (unclear whether Electron 44's main process honours `NODE_EXTRA_CA_CERTS`). There's also no rollback short of an even higher Electron version.
4. **A Rust app that can't run where 0.9.4 ran.** The minimum macOS (13.0), both thin arches (arm64 is what Rosetta copies fetch), the WebView2 runtime on Windows (Electron bundled Chromium; wry needs WebView2, which a stripped Windows 10 may lack, and a silent update can't prompt; consider bundling the Evergreen bootstrapper in the installer), and the GPU (GPUI Metal/DirectX vs Chromium's fallbacks). Any of these turns a silent auto-update into an app that doesn't start.
5. **The session-host handoff on macOS.** The swap deletes the 0.9.4 bundle while its Node host keeps running from it, and the Rust app must adopt that host (02, wire-compatible `PROTOCOL` 1). Separately, `hdiutil … -mountpoint`/`-force` already print deprecation warnings on Darwin 27. A future macOS that removes them breaks 0.9.4's updater (unfixable) and the Rust one (fixable with `diskutil image attach`, at the cost of parity).
6. **Smoke and CI** lose their DevTools-protocol driver (decision above).

## Parity checklist

- [x] UPD-001 Initial status `idle` with `current` = running version
- [x] UPD-002 Each state change replaces the status (+ `current`) and is sent on `ev:update-status` to the main window
- [x] UPD-003 An off reason sets `off` and skips clean-up, quit hook and timers
- [x] UPD-004 Other OS → `Updates are only built for macOS and Windows.`
- [x] UPD-005 Unpackaged → `Development builds don’t update themselves.` (after the platform check)
- [x] UPD-006 Packaged Windows is never off
- [x] UPD-007 macOS on only when the bundle and its parent folder are writable
- [x] UPD-008 Not writable, in Applications → `Your account can’t change apps in Applications, so Nodus can’t update itself.`
- [x] UPD-009 Not writable, elsewhere → `Move Nodus to your Applications folder so it can update itself.`
- [x] UPD-010 Clean-up starts at start-up; every check awaits it
- [x] UPD-011 macOS registers the swap on will-quit; Windows registers nothing
- [x] UPD-012 Auto check at 30 000 ms, then every 14 400 000 ms from start, only while `autoUpdate` is on at that tick
- [x] UPD-013 `settings:update` with truthy `autoUpdate` runs a non-asked check
- [x] UPD-014 `autoUpdate` defaults to true (`!== false`)
- [x] UPD-015 `check()` is a no-op while busy, staged or off
- [x] UPD-016 Lookup: `checking`, GET LATEST_URL with Accept + `Nodus/<v>` UA, 20 s timeout, no token
- [x] UPD-017 Lookup throws/times out → `Couldn’t reach GitHub to check for updates.`
- [x] UPD-018 403/429 → `GitHub is limiting update checks right now. Try again later.`
- [x] UPD-019 Other non-2xx → `GitHub answered the update check with an error (<status>).`
- [x] UPD-020 Asset = first with exact name and `state: 'uploaded'`
- [x] UPD-021 Asset name per OS; arm64 when the build is arm64 or runs under ARM64 translation
- [x] UPD-022 Version = `/^v?(\d+\.\d+\.\d+)$/` capture of `tag_name`
- [x] UPD-023 No asset or bad tag → `latest`
- [x] UPD-024 SHA-256 from `digest` `/^sha256:([0-9a-f]{64})$/`, else null
- [x] UPD-025 `isNewer`: numeric major.minor.patch, `parseInt || 0`, equal = not newer
- [x] UPD-026 Not newer (equal/older) → `latest`, never a downgrade
- [x] UPD-027 Failed version on an automatic check → `Nodus <v> downloaded but couldn’t install itself.`, no download
- [x] UPD-028 An asked check retries a failed version
- [x] UPD-029 `check()` resolves after the lookup; the download continues while busy
- [x] UPD-030 Download GET with `Nodus/<v>` UA, redirects followed
- [x] UPD-031 60 s stall timer re-armed per chunk; no total limit
- [x] UPD-032 `downloading` 0 first, then `min(100, floor(received/size*100))` only on change
- [x] UPD-033 Non-OK/no body → `GitHub answered the download with an error (<status>).`
- [x] UPD-034 Size or SHA mismatch → `The update came through damaged. Try again later.`
- [x] UPD-035 Other download failure → `The update didn’t finish downloading. Check your connection.`
- [x] UPD-036 Unnamed preparation error → `The update couldn’t be prepared. Try again later.`; logs `[updater] update failed`
- [x] UPD-037 macOS paths `updates/<v>/`, `updates/Nodus-<v>.dmg`, `updates/<v>/Nodus.app`
- [x] UPD-038 macOS order: rm dir, mkdir, download, copy, verify, staged, `ready`
- [x] UPD-039 macOS downloads without a digest (size check only)
- [x] UPD-040 Mount with `hdiutil attach <dmg> -nobrowse -noautoopen -readonly -mountpoint <mkdtemp mount->`
- [x] UPD-041 First root `*.app` copied; none → `The update has no app in it.`
- [x] UPD-042 Copy with `ditto --noqtn`
- [x] UPD-043 Always `hdiutil detach -force` (failure only logged), then rmdir the mount
- [x] UPD-044 `codesign --verify --deep --strict` fails → `The new version’s signature didn’t check out.`
- [x] UPD-045 plutil id (new vs running) and short version vs tag → `The update isn’t the expected version of Nodus.`
- [x] UPD-046 Team from `codesign -dv` stderr; a teamed running copy needs the same team → `The update isn’t signed by the same developer.`
- [x] UPD-047 macOS failure removes `<v>/`; the DMG is always deleted
- [x] UPD-048 Windows installer path `updates\Nodus-Setup-<v>.exe`, `downloading` 0 first
- [x] UPD-049 Windows without digest → `GitHub had no checksum for the update, so it wasn’t downloaded.`
- [x] UPD-050 Existing installer with matching size + SHA is reused without downloading
- [x] UPD-051 Otherwise the installer is downloaded over the path
- [x] UPD-052 Windows success: staged installer, `ready`
- [x] UPD-053 Windows failure deletes the installer, `error`
- [x] UPD-054 `restartToUpdate()` with nothing staged does nothing
- [x] UPD-055 macOS Restart to Update sets relaunch and quits normally
- [x] UPD-056 macOS: any normal quit with a staged update installs it (reopens only via Restart)
- [x] UPD-057 will-quit spawns `/bin/sh -c SWAP_SCRIPT nodus-update` with the 8 args, detached, once per run
- [x] UPD-058 Arg 8 = `NODUS_USER_DATA` only when relaunching
- [x] UPD-059 Script ignores HUP/INT/TERM and waits for the PID (0.2 s poll)
- [x] UPD-060 Script swaps: rm old, mv app→old, mv new→app, rm old
- [x] UPD-061 New can't go in → restore old, write version to `failed`
- [x] UPD-062 Old can't move aside → write version to `failed`
- [x] UPD-063 Reopen with `open -n` (+ `--env NODUS_USER_DATA=…`) only when relaunching, swap or not
- [x] UPD-064 Windows install is not re-entrant (two clicks, one dialog, one installer)
- [x] UPD-065 Missing installer: no dialog, no stop, no quit, error path
- [x] UPD-066 Live sessions → question dialog (`Stop … to update?`, detail, `Stop and Update` / `Not Now`)
- [x] UPD-067 `Not Now` leaves sessions and `ready` as they were
- [x] UPD-068 `stopAll()` then spawn `--updated /S --force-run` detached, unref, quit (stop before spawn)
- [x] UPD-069 Start failure: log, drop staged, delete installer, `The installer didn’t start, so Nodus wasn’t updated. Try Check for Updates again.`
- [x] UPD-070 Windows never installs on quit
- [x] UPD-071 Clean-up reads `failed`; kept only when newer than running
- [x] UPD-072 Windows keeps only the newest newer `Nodus-Setup-x.y.z.exe`
- [x] UPD-073 macOS force-detaches leftover `mount-*` before deleting
- [x] UPD-074 Everything else deleted (rm -rf / fs.rm); errors logged `[updater] cleanup failed`
- [x] UPD-075 Missing `updates/` is fine
- [x] UPD-076 `updates:status` returns the status
- [x] UPD-077 `updates:check` runs `check(true)`, resolves after the lookup
- [x] UPD-078 `updates:install` runs `restartToUpdate()`
- [x] UPD-079 Menu `Check for Updates…` opens Settings then `check(true)`; Mac app menu after About, else File after Settings…
- [x] UPD-080 `useUpdateStatus` null until the first answer, then follows events
- [x] UPD-081 Title-bar `Restart to Update` only in `ready`, with the platform tooltip
- [x] UPD-082 Settings `Updates`: `Nodus <current>` + state hint strings
- [x] UPD-083 Button: `Restart to Update` (ready), none (off), else `Check Now` disabled while null/checking/downloading
- [x] UPD-084 Switch `Download updates automatically` hidden when off, platform hint, disabled until settings load
- [x] UPD-085 Error state shows `You can also download the latest version yourself.` linking to releases/latest
- [x] REL-001 One version source feeds status, UA, plist versions, DMG name, exe/NSIS version fields
- [ ] REL-002 Release workflow on `v*` tags, `contents: write`
- [x] REL-003 Tag/version mismatch fails with the `::error::Tag …` message
- [x] REL-004 macOS release job on macos-latest runs the type check and tests first
- [x] REL-005 Certificate from `CSC_LINK`/`CSC_KEY_PASSWORD` into a throwaway keychain (script verbatim)
- [x] REL-006 Only the packaging step sees `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`
- [x] REL-007 `Nodus-arm64.dmg` and `Nodus-x64.dmg`, thin per arch, minimum macOS 13.0
- [x] REL-008 Developer ID signature, hardened runtime, entitlements on all code, timestamp
- [x] REL-009 App notarized and stapled; DMG unsigned and unstapled
- [x] REL-010 DMG UDZO zlib level 9, HFS+, volume `Nodus <v>-<arch>`, no SLA, unencrypted
- [x] REL-011 DMG contents and Finder layout (540×380 window at 400,530; icons 80/12; app 130,220; Applications 410,220)
- [x] REL-012 Windows release job on windows-latest builds both installers, no tests
- [ ] REL-013 Race-tolerant `gh release view || create || view`, upload `--clobber`, generated notes
- [ ] REL-014 Release published immediately (not draft/prerelease)
- [x] REL-015 No blockmaps or latest*.yml
- [x] REL-016 NSIS one-click per-user into `%LOCALAPPDATA%\Programs\nodus` (or registry InstallLocation, or `/D=`)
- [x] REL-017 Installer flags `/S`, `--updated`, `--force-run`, `--keep-shortcuts`, `--no-desktop-shortcut`, `--delete-app-data`, `--allusers`, `--currentuser`
- [x] REL-018 Single-instance mutex `99c2a492-c8ad-53bf-923b-1df868e91b51`
- [x] REL-019 Installer closes or kills processes running from the install folder
- [x] REL-020 Old version removed via its uninstaller `/S /KEEP_APP_DATA /currentuser [--keep-shortcuts] --updated _?=<dir>`, 5 retries
- [x] REL-021 HKCU `Software\<GUID>` and `…\Uninstall\<GUID>` values as listed
- [x] REL-022 Install folder has `Nodus.exe` and `Uninstall Nodus.exe`
- [x] REL-023 Start menu + Desktop `Nodus.lnk` with AUMI `dev.saleem.nodus`; kept as they are on update
- [x] REL-024 `--force-run` launches via the Start menu shortcut with `--updated`
- [x] REL-025 Installer copies itself to `%LOCALAPPDATA%\nodus-updater\installer.exe`
- [x] REL-026 Uninstall keeps `%APPDATA%\Nodus` unless `--delete-app-data`
- [x] REL-027 Installer and exe unsigned (SmartScreen: More info › Run anyway)
- [x] REL-028 `Nodus.exe` icon (index 0) and version strings
- [x] REL-029 CI triggers, `contents: read`, concurrency `ci-${{ github.ref }}` cancel-in-progress
- [x] REL-030 CI macOS job: install, typecheck, test, build
- [x] REL-031 CI Windows job: subset tests, x64 installer, silent `/S` install, smoke, `windows` artifact
- [x] REL-032 Smoke: isolated userData, projects/board/terminal `nodus-42`, light and dark screenshots
- [x] REL-033 Smoke: host welcome lists the terminal; Windows key matches `host.key`
- [ ] REL-034 Smoke (Windows): Ctrl+W board/terminal and Ctrl+Shift+W behaviors
- [x] REL-035 Smoke cleanup: host shutdown, kill app, copy `host.log`, exit code
- [x] REL-036 Actions pinned by SHA, `persist-credentials: false`
- [x] REL-037 `postinstall.cjs` is Electron-only and is removed at cutover
- [x] REL-038 README/site links `releases/latest/download/<asset>` keep working
- [x] REL-039 First Rust release > 0.9.4, tag `vX.Y.Z` with no suffix
- [ ] REL-040 Contract holds for every Mac 0.6.0–0.9.4 and Windows 0.9.0–0.9.4 updater
- [x] REL-041 Rust .app passes deep-strict verify, id, version and same TeamIdentifier
- [x] REL-042 Rust app starts via `open -n` and honors `open --env NODUS_USER_DATA`
- [x] REL-043 Rust first launch cleans up the 0.9.4 `updates/` leftovers
- [x] REL-044 Rust installer run as `--updated /S --force-run` replaces the Electron install in place and starts the app
- [x] REL-045 Rust app ignores the `--updated` argument
- [ ] REL-046 Same designated requirement as 0.9.4 (TCC grants and Keychain item survive)
- [x] REL-047 Rust releases keep the same contract; the Rust updater implements UPD-001…UPD-085
