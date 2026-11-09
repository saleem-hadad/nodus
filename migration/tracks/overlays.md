# Track: overlays (P8, spec 11)

- **Branch:** `rust/overlays`
- **Scope:** the board's overlays in `crates/nodus-ui/src/board/overlays/`: the prompt bar (Sidekick), ⌘K session search, the merge dialog ("New session from context"), the ink layer and its dock, and the selection bar; their pure logic in `nodus-board` (`ink_draw`, `merge`).
- **Spec:** `migration/features/11-board-overlays.md` (BAR, SRCH, MERGE, INKUI, SEL) and the board hooks board-canvas left for them (BOARD-036, 108/109, 131–138, 143–145, 155, 159, 166, 167, 169).
- **Status:** 🟡 in progress.

## Files

| File | What |
| --- | --- |
| `nodus-board/src/ink_draw.rs` | `ink.ts`: paths, `simplify`, boxes, hit tests, `strokes_along`, `translate`, `size_at`, and the undo history (`InkHistory`); ink.test.ts ported |
| `nodus-board/src/merge.rs` | MergeDialog.tsx's logic: `Merge` (items, phases, request ids, dirty/append rule, `can_start`), `default_name`, `section`, `initial_prompt`, `normalize_focus` |

## Done (implemented, IDs in the code, checked against the goldens)

## Deferred

## Edits outside the track's files
