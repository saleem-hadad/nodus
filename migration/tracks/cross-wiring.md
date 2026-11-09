# Track: cross-wiring (specs 09, 11, 12 between merged tracks)

- **Branch:** `rust/cross-wiring`
- **Scope:** five behaviors between the board canvas (`board/`), ⌘K search (`board/overlays/search/`) and the
  terminal's windows (`terminal/`) that no track owned: BOARD-037 and MODAL-028…MODAL-031.
- **Status:** ✅ ready to merge: 5 of 5 done, nothing ticked here (the evidence is below).

## How to check

```sh
cargo build -p nodus
python3 crates/nodus-ui/parity/cross_wiring.py [BOARD-037 MODAL-028 …] --world <scratch folder>
```

Each check seeds the parity harness's world (its own HOME and NODUS_USER_DATA, the world's stand-ins, never the real
claude), starts the app with the test driver, drives it and prints PASS or FAIL per step from the driver's `route`
(windows, search, the preview's find, cards, views). The window never takes focus. All five pass.

## Done (ID — where / evidence)

- **BOARD-037** — `board/commands.rs:1280` `jump_to_card(id, find)`: an archived card's preview opens with the
  search's `{query, snippet}` through `AppState::open_preview_at` (`board/store.rs:452`), no viewport change; an
  unknown id returns before closing search. `board/overlays/search/mod.rs:202` opens every row through the board's
  jump and drops the search's state only when the jump closed it; `row_find` (`:73`) builds the trimmed query and
  the best snippet. `open_preview(id)` records no find, so a preview opened from the board after a search isn't
  opened at the old match (TRANS-002); the terminal's `actions::open_preview` routes to the board's store.
  Test: `search::tests::a_row_opens_at_its_best_match_for_the_trimmed_query`. Driver: `BOARD-037` (⌘K, `apple pay`,
  Enter → search closed, sf-arch-01's preview, find `apple pay` + its snippet; then the archived card's menu
  Preview → no find). Seen: the found message centred, sunken with its bar, `Safari` marked.
- **MODAL-028** — `board/store.rs:35` `came_back_archived`, used by `fetch_board` (`:59`), which `BoardChanged`
  reaches. Test: `board::store::tests::a_reload_closes_the_window_whose_card_came_back_archived`. Driver:
  `MODAL-028` (sf-tax archived outside → reloaded, window stays; sf-cart archived outside → window closes).
- **MODAL-029** — `board/commands.rs:1232`: `run_finished` disposes the run's terminal, then closes the window if it
  shows the run. Driver: `MODAL-029` (another run → window stays; its run → window closed, terminal disposed, seen
  as the registry's change with no terminal left). As in 0.9.4 (the core sends `run-finished` before
  `board-changed`), the run's card is still on the board then, so closing its window expands it (BOARD-103) and the
  card attaches a terminal until the reload takes the card away.
- **MODAL-030** — `board/store.rs:368`: `remove_view` disposes the brain's terminal (it already closed the window and
  took the card off). Driver: `MODAL-030` (another view removed → window stays; ds-build removed → view, window,
  card and terminal gone).
- **MODAL-031** — `board/input.rs:45` `Cover`: `search_key` (⌘K: not over the session window, agents viewer, diff,
  overlay or a field; the preview gives way), `shortcuts` (nothing over the board), `menu` (not under an overlay,
  the agents viewer or the diff); used at `:1194`, `:1207`, `:1358`, and the menu's Search closes the window and
  the preview (`:1365`). Test: `board::input::tests::the_session_window_keeps_the_boards_keys_but_not_the_menus_search`.
  Driver: `MODAL-031` (window up: ⌘K, N, T do nothing; menu Search → window closed, search open; over the preview
  likewise).

## Edits outside the board

- `terminal/actions.rs`: `open_preview` calls the board's `open_preview_at` (the integration note terminal-ui left).
- `terminal/driver.rs`: `route` also reports `search`, `find`, `cards`, `views`; `save-nodes` (a change from
  outside, then `board-changed`), `run-finished`, `remove-view`. `shell/root.rs`: `Shell::board()` for the driver.
- `crates/nodus-ui/parity/cross_wiring.py`: the checks above.
