# Track: audit-shell (P12, first slice)

- **Branch:** `rust/audit-shell`
- **Scope:** every unticked item in specs 01–03 and 05–08 (spec 04 is views-ui's): find the code and its test, or
  check it in the running app, then tick it, implement it, or defer it with a reason. Plus four debts from plan.md:
  selectable static text, an action name's paste cut to 40, the test driver on a Windows named pipe, and the Windows
  window chrome (☰ menu, caption buttons, single instance, quit quirks).
- **Evidence:** one line per ID below: `done` (already implemented; where, and the test when there is one),
  `implemented in <commit>` (this track), or `deferred` (and why). Line numbers are as of this branch.

## Spec 01 · Platform & shell

- PLAT-001 — done: `shell/menus.rs` (`Nodus` app menu, `About Nodus`, `Hide Nodus`, `Quit Nodus`; test
  `has_windows_file_and_help_items`), `packaging/macos/Info.plist` (`CFBundleName`/`CFBundleDisplayName` `Nodus`),
  window title `Nodus` (`shell/window.rs` `open_main_window`).
- PLAT-006 — done: no lock anywhere on a Mac (`crates/nodus/src/main.rs`, `nodus-ui/src/app.rs` `run`); a second
  process started directly runs on its own, as `packaging/smoke.py --open` (`open -n`) does.
- PLAT-012 — done: `nodus-core/src/nodus.rs` `Nodus::start` spawns `Env::init` on the core runtime without waiting,
  and `Inner::env` waits on the `watch` for it (every ⏳env command calls it: `app.info`, `actions.draft`,
  `voice.transcribe`, `git.*`, sessions); the window opens after `launch()`, which doesn't wait for it.
- PLAT-013 — done: by construction; every command is a method on `nodus_core::Nodus`, which exists before
  `open_main_window` (`nodus-ui/src/app.rs` `run`).
- PLAT-020 — done: `shell/window.rs` `open_main_window` (`WindowBackgroundAppearance::Opaque`, no vibrancy, no
  always-on-top, OS shadow and corners).
- PLAT-024 — done: links leave only for `http(s)` (`terminal/host.rs` `activate_link` → `links::browser_url`;
  Settings' and the menu's fixed `https` links); nothing in GPUI opens an in-app window for a link.
- PLAT-025 — done: by construction; GPUI draws the app, there's no page to navigate away from.
- PLAT-033 — done: `nodus-core/tests/headless.rs` `a_view_starts_its_brain_and_a_relaunch_adopts_the_host`
  (quitting lets go of the host; the relaunch adopts its sessions).
- PLAT-054 — done: `shell/menus.rs` `register_actions` (`Quit` → `cx.quit()` → `on_app_quit` → `Nodus::shutdown`,
  PLAT-032; `QuitAndStopSessions` → `core_shell::quit_and_stop_sessions`, PLAT-034).
- PLAT-056 — done: menu Undo/Redo/Cut/Copy/Paste/Select All are gpui-component's input actions with the OS
  selectors (`shell/menus.rs` `app_menus`), handled by the focused text field; a focused terminal takes Copy, Paste
  and Select All (`terminal/host.rs`, TERM-040…043). Paste and Match Style pastes (plain text is all GPUI has).
- PLAT-083 — done: `nodus-store/src/store.rs` (`Store`: one `State`, whole file rewritten); test
  `writes_the_file_back_exactly_as_it_read_it`.
- PLAT-084 — done: `store.rs` `migrate_old_state`; tests `brings_the_old_apps_state_over`,
  `leaves_the_old_app_alone_once_it_has_state`, `brings_nothing_over_from_a_null_state`.
- PLAT-085 — done: `nodus-store/src/state.rs` `State::from_json`; tests `reads_an_older_state`,
  `reads_lists_that_are_not_lists_as_empty`. (A `boards` array reads as `{}`, where V8 kept the array; nothing writes
  one.)
- PLAT-086 — done, deliberately safer (plan.md Quirks, PLAT-086/088): tests `starts_empty_without_a_file`,
  `starts_empty_when_the_file_is_not_json`, `moves_aside_a_file_it_cannot_read`.
- PLAT-087 — done: the `extra` maps on every record; test `keeps_what_a_newer_version_wrote`.
- PLAT-088 — done, deliberately safer (plan.md Quirks): unknown top-level keys are kept (`State::extra`), test
  `keeps_what_a_newer_version_wrote`.
- PLAT-089 — done: entries aren't checked beyond their JSON shape; one this version can't hold moves the file aside
  (the PLAT-086/088 quirk), test `moves_aside_a_file_it_cannot_read`.
- PLAT-090 — done: `state.rs` `NotFound`; test `finds_projects_groups_sessions_and_views`.
- PLAT-091 — done: `state.rs` `State::board_mut` and `BoardState`'s defaults; test `makes_a_board_as_get_board_does`,
  `reads_an_older_state` (getBoard's additions in its order; no save of its own).
- PLAT-092 — done: `state.rs` `project_id_for_view`, `project_id_for_session`; test
  `finds_projects_groups_sessions_and_views`.
- PLAT-093 — done: `store.rs` `Store::save` / `run_saver` (`SAVE_DELAY` 300 ms, restarted by each change;
  `[store] save failed` logged); test `saves_a_moment_after_the_last_change`.
- PLAT-094 — done: `store.rs` `write_atomically` (`state.json.<pid>.tmp`, rename, `JSON.stringify(…, null, 2)` via
  `nodus_domain::json`, no trailing newline); tests `writes_through_a_temporary_file`,
  `writes_the_file_back_exactly_as_it_read_it`.
- PLAT-095 — done: `store.rs` `Store::flush`; tests `flush_writes_what_is_waiting`,
  `flush_does_nothing_when_nothing_is_waiting`.
- PLAT-096 — done: `nodus-platform/src/env.rs` `process_env` / `to_env_map`; test `folds_windows_path_names_into_one`.
- PLAT-098 — done: `env/login_shell.rs` `capture` returns none on Windows.
- PLAT-099 — done: `env.rs` `Env::init` (`base.extend(captured)` over the process env).
- PLAT-100 — done: `env/path_search.rs` `add_fallback_dirs`; tests `adds_the_installers_folders_it_lacks` (both
  OSes), `makes_a_path_when_there_is_none`.
- PLAT-101 — done: `path_search.rs` `find_on_path`; tests `looks_for_each_name_in_every_folder_before_the_next`,
  `skips_empty_folders_and_folders_without_it`, `takes_only_files`, `takes_only_files_it_may_run`.
- PLAT-102 — done: `env/claude.rs` `find`; test `finds_claude_and_its_version`.
- PLAT-103 — done: `claude.rs` `find` (`claude.exe` before `claude.cmd`, the shim's exe or none).
- PLAT-104 — done: `claude.rs` `exe_behind_shim`; tests `finds_the_program_an_npm_shim_starts`,
  `finds_nothing_in_a_shim_that_runs_a_script_with_node`, `reads_the_other_ways_a_shim_can_say_it`.
- PLAT-106 — done: `nodus.rs` `Nodus::start` runs `Env::init` once; nothing reads it again.
- PLAT-107 — done: `env.rs` `Env::spawn_env` (callers add their own last); test
  `spawns_without_a_parent_sessions_markers`, `keeps_the_users_language_and_sets_the_terminal`.
- PLAT-108 — done: `env.rs` `Env::var`; test `reads_variables`.
- PLAT-109 — done: `env.rs` `Env::user_shell`; tests `runs_the_users_shell`, `runs_powershell`.
- PLAT-110 — done: `env.rs` `Env::terminal_shell`; tests `runs_the_users_shell`, `runs_powershell`.
- PLAT-111 — done: `env.rs` `Env::shell_command`; tests `runs_the_users_shell`, `runs_powershell`.
- PLAT-112 — done: `env.rs` `ClaudeNotFound`; test `says_how_to_install_claude`.
- PLAT-113 — done: `env.rs` `Env::git_bash` (once per `Env`; on Windows the env is the process's from the start, so
  the 0.9.4 "first call before the capture" case can't differ); tests `finds_git_bash_where_its_set`,
  `finds_git_bash_beside_git`.
- PLAT-129 — done: by construction; commands take typed Rust arguments (absent = `None`, `voice.transcribe` takes
  bytes), nothing else can be passed.
- PLAT-135 — done: the capture panel is its own window and view (`nodus-ui/src/capture/`), not a hash route.
- PLAT-138 — done: `components/text_field.rs` (no native menu), and GPUI shows none elsewhere.
- PLAT-140 — done: `nodus-core/src/local_storage.rs` (every `file://` key brought over from `Local Storage/leveldb`
  once, kept under `localStorage` in state.json); tests `keeps_what_a_page_stored_across_restarts` and the LevelDB
  reader's. The capture keys use it (capture-panel); `nodus.inkColor` uses it on rust/overlays.
- PLAT-141 — done: nothing deletes Chromium's files; the updater touches only `updates/` (`nodus-updater`), and
  `Local State` (secrets), `Preferences` (zoom) and `Local Storage` are only read.

## Spec 02 · Session host

- HOST-129 — done: `nodus-sessions/src/launch.rs` `command_for`; tests `starts_a_new_named_session_with_a_prompt`,
  `resumes_without_a_name`, `runs_an_action_in_auto_mode_with_its_system_prompt`,
  `leaves_the_board_tools_out_for_an_older_host`.
- HOST-130 — done: `launch.rs` `command_for` (spawn env, Git Bash unless set, hook variables last); test
  `gives_claude_the_hook_environment`.
- HOST-131 — done: `launch.rs` `command_for`; test `starts_a_terminal_as_a_login_shell`.

## Spec 05 · Actions, voice

- ACT-016 — done: `nodus-core/src/actions.rs` `ActionsApi::draft` (waits for the env, `Unknown project`, then
  `draft_action` with `global`); test `act_001_015_actions_are_saved_moved_and_removed` (its `actions:draft` case).
- VOICE-019 — done: `nodus-core/src/voice.rs` `VoiceApi::transcribe` (waits for the env, `Unknown project`, the
  MIME type passed as given); test `voice_001_008_the_key_is_sealed_and_never_given_out`.

## Spec 06 · Updater UI

- UPD-081 — done: `shell/title_bar.rs` `title_bar` (`Restart to Update` only for `Ready`, RotateCw 14/2.2, both
  tooltips, `install_update`).
- UPD-082 — done: `settings/mod.rs` (`Nodus <current>` / `Nodus `, `update_text` hints).
- UPD-083 — done: `settings/mod.rs` (`Restart to Update` when ready, none when off, `Check Now` disabled while
  none/checking/downloading).
- UPD-084 — done: `settings/mod.rs` (`Download updates automatically`, hidden when off, both hints, `switch_row`
  disabled until settings load).
- UPD-085 — done: `settings/mod.rs` (the error state's hint linking `RELEASES_URL`).

## Spec 08 · Shell UI (board route store)

- UI-026 — done: `shell/root.rs` (the board and the rail over it, UI-027's pinned column).
- UI-034 — done: `state.rs` `on_event` (`BoardChanged` reloads only the open board).
- UI-036 — done: `state.rs` `on_event` → `board/store.rs` `set_status` (ignored with no board open).
- UI-037 — done: `state.rs` `on_event` → `store.rs` `set_agent_status`.
- UI-038 — done: `state.rs` `on_event` → `store.rs` `set_usage`.
- UI-050 — done: `store.rs` `fetch_board` (`board_loading` only for another project's board; cleared either way);
  `state.rs` `navigate` clears the snapshot first.
- UI-051 — done: `store.rs` `fetch_board` (dropped unless that board is still the route).
- UI-052 — done: `store.rs` `fetch_board` (closes the terminal modal when its card came back archived).
- UI-053 — done: `store.rs` `patch_cards` (no board, or no patches: nothing).
- UI-054 — implemented in 470c518: `store.rs` `patch_cards` (an empty session leaves instead of archiving, its notes
  fall off, it leaves the selection, and now its terminal is disposed).
- UI-055 — done: `store.rs` `patch_cards` (the patch's fields; `section_id: Some(None)` clears, `None` leaves).
- UI-056 — done: `store.rs` `patch_cards` (archived → `done`, `archived_at` when newly archived, section and
  expanded dropped).
- UI-057 — done: `store.rs` `patch_cards` (all patches to `board.save_nodes` in one call).
- UI-058 — done: `store.rs` `upsert_card`.
- UI-059 — implemented in 470c518: `store.rs` `remove_card` (off the board, notes unstuck, out of the selection, and
  now its terminal disposed, then the core removes it).
- UI-060 — done: `store.rs` `unstick_notes`.
- UI-061 — done: `store.rs` `set_archive`.
- UI-062 — done: `store.rs` `save_section`.
- UI-063 — done: `store.rs` `remove_section`.
- UI-064 — done: `store.rs` `save_note`.
- UI-065 — done: `store.rs` `remove_notes`.
- UI-066 — deferred: `save_ink` (INKUI-024) is the overlays track's, on rust/overlays; it ticks when that merges.
- UI-067 — done: `store.rs` `upsert_view` (no save).
- UI-068 — done: `store.rs` `patch_view` (only the fields given; `ViewPatch` has every field).
- UI-069 — implemented in 470c518: `store.rs` `remove_view` (the view and its brain's card; the terminal modal closes
  when it shows the brain, and now the brain's terminal is disposed).
- UI-070 — done: `store.rs` `set_status`.
- UI-071 — done: `store.rs` `set_status` (`live` false: every agent `done`).
- UI-072 — done: `store.rs` `set_agent_status` (known agents only, only when it changed).
- UI-073 — done: `store.rs` `set_usage`.
- UI-074 — done: `git/mod.rs` `GitModel::refresh` (GIT-021: off a board nothing; a read in flight for the project is
  shared).
- UI-075 — done: `git/mod.rs` `GitModel::refresh` (failure → none; applied only to the same open board, only when it
  differs).
- UI-076 — implemented in 470c518: `store.rs` `open_terminal` / `open_preview` (each closes the other; a preview from
  the board now records no search), `terminal/actions.rs` `open_preview` records a search's.
- UI-077 — done: `store.rs` `open_agents`, `terminal/actions.rs` `open_agents`; `git/mod.rs` sets and clears
  `diff_open`.
- UI-078 — done: `actions/mod.rs` (ACT-017 `setActions`: the project in the list, the open board's, the global list).

## Debts

- Action name paste — implemented in 51a455a: `components/text_field.rs` `fit_max_length` / `paste_within` on the
  name field (`actions/dialog_view.rs`); test `cuts_a_paste_to_what_fits_as_max_length_does`.
