# Track: ui-shell (P4a + P5)

- **Branch:** `rust/ui-shell`
- **Scope:** `nodus-core` skeleton (the `Nodus` service: app, settings, projects, groups), `nodus-ui` (theme, components, title bar, Settings, projects grid, project rail, menus, the test driver), the `nodus` binary launching the GUI.
- **Specs:** 08 (UI-001…UI-305), the shell parts of 01 (window, menus, quit paths, the window.api contract for app/settings/projects/groups).
- **Ownership (since the core track started):** this track keeps nodus-core's `projects.rs`, `groups.rs`, `settings.rs`, `app.rs` and `Dialogs`; the core track owns the rest of nodus-core. This branch made one change in `nodus.rs`/`lib.rs` that core should keep or replace: a `nodus_board::Board` built with a `NoProcesses` `BoardSessions` (nothing live until the host is wired), so `projects.stats` reads real counts; `NodusConfig` gained `runtime` and `home`, `EnvSource::Capture` lost its handle, and `flush()` became `shutdown()` (unwatch transcripts, then flush).

## How to run it

```sh
cargo build -p nodus
NODUS_USER_DATA=<scratch>/userData target/debug/nodus          # never the real userData
```

Side by side with the goldens (needs the parity harness's `seed/` and `node_modules`, from `rust/parity-harness` until it merges):

```sh
python3 crates/nodus-ui/parity/scenes.py [scene-prefix…] [--theme light|dark] --harness <…/migration/parity>
NODE_PATH=<…/migration/parity>/node_modules node crates/nodus-ui/parity/compare.mjs migration/parity/golden            # all
NODE_PATH=… node crates/nodus-ui/parity/compare.mjs migration/parity/golden settings.light 480 40 480 820             # one, cropped
```

Shots land as `<golden>/<scene>.<theme>.rust.png` (gitignored), side-by-side diffs in `migration/parity/rust-diffs/`. The driver (`NODUS_AUTOMATION=<socket>`, JSON lines, see `crates/nodus-ui/src/automation.rs`): `resize`, `parity`, `core`, `menu`, `key`, `modifiers`, `mouse`, `scroll`, `files`, `emit`, `wait`, `idle`, `screenshot` (GPUI's `render_to_image`, converted to Display P3 as the goldens are), `window`, `quit`. The window ignores the real mouse and never takes focus while driven.

## nodus-core API (what later tracks extend)

```rust
let nodus = Nodus::start(NodusConfig { user_data, env: EnvSource::Capture, dialogs: Some(dialogs), runtime, home })?;
nodus.app().info().await                       // app:info (awaits the login-shell env)
nodus.app().take_open_request().await          // app:take-open-request
nodus.app().menu_command("close").await        // app:menu-command → Event::Menu
nodus.settings().get().await / .update(patch).await / .update_json(value).await
nodus.projects().list() / add(Option<String>) / update(id, ProjectPatch) / remove(id) / reveal_in_finder(id) / stats()
nodus.groups().list() / create(name) / update(id, GroupPatch) / remove(id)
nodus.subscribe() -> broadcast::Receiver<Event>   // window.api.on.*
nodus.sink() -> Arc<dyn EventSink>                 // for domain crates
nodus.on_settings_applied(hook)                    // applySettings hooks (theme, menu; Dock badge and capture go here)
nodus.set_open_request(..)                         // for the notifications track
nodus.shutdown()                                   // quit: unwatch transcripts, flush state.json
```

Errors are `CommandError { channel, name, message }`; `ipc_message()` is 0.9.4's `Error invoking remote method '<channel>': Error: <message>`.

## Status

| Component | State |
| --- | --- |
| nodus-core: app, settings, projects, groups, events, stats through nodus-board | ✅ tested |
| theme tokens (light/dark), motion, fonts, platform helpers (platform.test.ts ported), format helpers, focus (focus.test.ts ported, in nodus-domain) | ✅ tested |
| button, switch, status dot, focus ring, tooltips, icons, folder icon (resvg), letter-spaced text | ✅ |
| context menu, modal, swatches, split button, usage stats, scroll area with gutter, text fields, frosted (real backdrop blur) | ✅ |
| title bar, running count (nodus-sessions' port of runningSessions.ts), menus (mac + Windows item sets), window, root, state | ✅ |
| projects grid, folder item, group picker, remove dialog, toast, drop overlay | ✅ compared: 0.03–0.3% of pixels differ (text rasterization) |
| Settings dialog | ✅ compared: top matches; the Accessibility row waits for the capture API |
| project rail (edge reveal, pinned, ⌘ numbers, hints) | ✅ compared on the rail; the board under it is the board track's |
| test driver + scene runner | ✅ |

## Done (implemented; IDs in the code)

- 08: UI-001…UI-025, UI-027…UI-033, UI-035, UI-039…UI-049, UI-079…UI-138 (UI-106…UI-108 render; the install itself is 06), UI-139…UI-146 (render; saving needs 05), UI-147…UI-305, except as listed below. UI-081…UI-087 are pure functions with tests (`format.rs`).
- 01: PLAT-002, PLAT-003/004 (nodus-store), PLAT-014…PLAT-017, PLAT-019, PLAT-021, PLAT-022, PLAT-023, PLAT-028…PLAT-030, PLAT-032 (steps 3 and 5), PLAT-036…PLAT-044, PLAT-046, PLAT-047, PLAT-049…PLAT-053, PLAT-055, PLAT-058, PLAT-059, PLAT-060 (as the in-window menu off a Mac), PLAT-061…PLAT-082 (nodus-core), PLAT-122…PLAT-127 (core side), PLAT-130…PLAT-134.

## Deferred (needs another track's API, or a decision)

- Board route (UI-026, UI-028, UI-034, UI-036…UI-038, UI-050…UI-078): a board opens as an empty canvas; the store's board logic comes with the board UI track, on core's board API.
- Live counts: nothing reads as running until core connects the session host (`NoProcesses`), so no folder badges, rail dots or running count yet (UI-102…UI-105, UI-234, UI-258 render once stats carry them).
- Settings rows on other tracks' APIs: capture shortcut state (07: the Allow…/unavailable rows, "Ready."), voice key source/save/remove (05: the field stays disabled), update status/check/install (06: rendered from `Event::UpdateStatus`, which the driver can send).
- PLAT-034 Quit and Stop Sessions (02), PLAT-048 Capture… (07), PLAT-046's check (06), PLAT-057 page zoom (decision: whole-UI scale), PLAT-007/008 Windows single instance, PLAT-018 Windows caption buttons (drawn by the OS? not yet: the title bar leaves their 138px), PLAT-031/035 Windows quit quirks, PLAT-137 spellcheck.
- UI-099 off a Mac the ☰ button is drawn but its popup menu isn't built yet.
- UI-013 OS file drops outside the grid are ignored (GPUI never navigates), nothing to do.

## Known differences from 0.9.4 (side by side)

- **Text at 13px is ~0.08px per character narrower**: CoreText (GPUI) applies SF Pro's `trak` table, Chromium's HarfBuzz shaping doesn't. A 13px button label is 1px narrower; 12px (trak 0) and 26px match. Needs a GPUI text-system option (gpui-fork).
- **Wrapping**: GPUI counts the space after a word when deciding to wrap, Chromium lets it hang, so a hint can wrap one word earlier (Settings › Floating capture).
- **Sub-pixel placement**: some text sits one device pixel off (fractional layout, 0.5 logical px) where Chromium rounds the other way.
- **Colours**: the goldens are Display P3 (what Chromium sent the screen); GPUI's layer is now tagged sRGB so macOS converts the same way. Saturated blues land within 4/255 of Chromium's conversion.
- Hover state after a menu opens under the pointer: Chromium keeps the folder's 1px hover lift until the pointer moves; we drop it.

## Next

1. Run every shell scene in both themes after each change; keep the summary under 0.3% (text only) for grid scenes.
2. Off a Mac: the ☰ popup (an in-window menu with Windows' item set) and drawn caption buttons.
3. Wire Settings' capture, voice and update rows as those APIs land in core; the board route as the board UI track lands.
