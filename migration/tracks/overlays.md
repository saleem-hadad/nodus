# Track: overlays (P8, spec 11)

- **Branch:** `rust/overlays`
- **Scope:** the board's overlays in `crates/nodus-ui/src/board/overlays/`: the prompt bar (Sidekick), ⌘K session search, the merge dialog ("New session from context"), the ink layer and its dock, and the selection bar; their pure logic in `nodus-board` (`ink_draw`, `merge`).
- **Spec:** `migration/features/11-board-overlays.md` (BAR, SRCH, MERGE, INKUI, SEL) and the board hooks board-canvas left for them (BOARD-036, 108/109, 131–138, 143–145, 155, 159, 166, 167, 169).
- **Status:** 🟡 in progress: ink, bar (with spec 05's mic), selection bar, search drawn and checked; merge dialog being checked.

## Files

| File | What |
| --- | --- |
| `nodus-board/src/ink_draw.rs` | `ink.ts`: paths, `simplify`, boxes, hit tests, `strokes_along`, `translate`, `size_at`, and the undo history (`InkHistory`); ink.test.ts ported |
| `nodus-board/src/merge.rs` | MergeDialog.tsx's logic: `Merge` (items, phases, request ids, dirty/append rule, `can_start`), `default_name`, `section`, `initial_prompt`, `normalize_focus` |
| `board/overlays/mod.rs` | `Overlays` (per board mount), `kbd`, `spinner`, shadow mixing |
| `board/overlays/icons.rs` | the lucide icons only the overlays draw (`icons/overlays/…`) |
| `board/overlays/ink/` | `mod.rs` state, tools, colours, label measuring; `ops.rs` changes, history, labels; `input.rs` the pointer taken ahead of the board; `draw.rs` strokes, labels, selection, label input, the hitbox; `dock.rs` the dock and palette |
| `board/overlays/bar/` | `mod.rs` state and submit; `keys.rs` the field's keys; `view.rs` column, pill, token, field, hint, send; `popovers.rs` slash menu and colour picker; `voice.rs` the mic (spec 05's `voice::Mic`), orb, status line, notice |
| `board/overlays/selection.rs` | the toast and the selection bar above the row |
| `board/overlays/search/` | `mod.rs` state, rows, debounce, keys, open/mark/start; `view.rs` the modal; `text.rs` UTF-16 highlight runs and `timeAgo` |
| `board/overlays/merge/` | `mod.rs` the dialog's lifecycle, requests, progress events, start; `view.rs` header, gathering, review, footer |
| `parity/scenes11.py` | spec 11's scenes (bar, search, merge, ink) for the Rust app |

## Done (implemented, IDs in the code, checked against the goldens)

## Deferred

## Edits outside the track's files

- `board/canvas.rs`: the `overlays` field. `board/render.rs`: ink input/layer/hitbox/dock, the bar column, search and merge drawn; nodes at `RAISED_Z` drawn over the ink; the HUD toast moved into the bar's column (BOARD-138). `board/hud.rs`: `render_toast` removed (the column draws it). `board/input.rs`: ⌘Z/⇧⌘Z, Esc, ⌫, P/A/X, V and `/` wired to the ink and the bar; `release_keys` drops an ink gesture. `board/store.rs`: `save_ink`.
- `components/icon.rs`: serves `icons/overlays/…`. `components/button.rs`: `height()`.
- `automation.rs`: `stub` (what the merge dialog's summarize answers in a scene).
- `Cargo.toml`: `lyon` (round caps and joins for ink strokes; GPUI tessellates with it).
