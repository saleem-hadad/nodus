# Track: ui-shell (P4a + P5)

- **Branch:** `rust/ui-shell`
- **Scope:** `nodus-core` skeleton (the `Nodus` service: app, settings, projects, groups), `nodus-ui` (theme, components, title bar, Settings, projects grid, project rail, menus, the test driver), the `nodus` binary launching the GUI.
- **Specs:** 08 (UI-001…UI-305), the shell parts of 01 (window, menus, quit paths, the window.api contract for app/settings/projects/groups).

## How to run it

```sh
cargo build -p nodus
NODUS_USER_DATA=<scratch>/userData target/debug/nodus          # never the real userData
```

Screenshots of the parity scenes: seed a world with the parity harness (`migration/parity/seed`), start the app with `NODUS_AUTOMATION=<socket>` (a short path: macOS caps socket paths at 104 bytes) and drive it with JSON lines (see `crates/nodus-ui/src/automation.rs`): `resize`, `parity`, `core`, `menu`, `key`, `modifiers`, `mouse`, `idle`, `screenshot`, `window`, `quit`.

## nodus-core API (what later tracks extend)

```rust
let nodus = Nodus::start(NodusConfig { user_data, env: EnvSource::Capture(rt), dialogs: Some(dialogs) })?;
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
```

Errors are `CommandError { channel, name, message }`; `ipc_message()` is 0.9.4's `Error invoking remote method '<channel>': Error: <message>`.

## Status

See the commits on the branch; the done/deferred lists below are kept current.

### Deferred (needs another track's API)

- `projects.stats` returns zeros (03 `projectStats`): folder subtitles read "No sessions", no badges, no running count yet.
- Board load and the board's mutations (UI-050…UI-078): a board opens as an empty canvas; the store's board logic comes with the board track.
- Settings rows backed by other tracks: capture shortcut state (07), voice key (05), update status (06): rendered as 0.9.4 renders them before the answer arrives.
- Quit and Stop Sessions stops nothing yet (02); Capture… opens nothing yet (07); Check for Updates… only opens Settings (06).
- Page zoom (Actual Size / Zoom In / Zoom Out) is a no-op: it needs a whole-UI scale.
- Backdrop blur goes through `components::frosted::frosted` (solid surface) until gpui-fork's blur lands.
