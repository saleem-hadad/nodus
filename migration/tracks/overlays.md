# Track: overlays (P8, spec 11)

- **Branch:** `rust/overlays`
- **Scope:** the board's overlays in `crates/nodus-ui/src/board/overlays/`: the prompt bar (Sidekick), ⌘K session search, the merge dialog ("New session from context"), the ink layer and its dock, and the selection bar; their pure logic in `nodus-board` (`ink_draw`, `merge`).
- **Spec:** `migration/features/11-board-overlays.md` (BAR, SRCH, MERGE, INKUI, SEL) and the board hooks board-canvas left for them (BOARD-036, 108/109, 131–138, 143–145, 155, 159, 166, 167, 169).
- **Status:** ✅ ready to merge: 107 of 111 spec 11 behaviors done and checked against the goldens (4 deferred), and 19 of the 20 board hooks spec 09 left for the overlays (BOARD-036's flash is the card's).

## Files

| File | What |
| --- | --- |
| `nodus-board/src/ink_draw.rs` | `ink.ts`: paths, `simplify`, boxes, hit tests, `strokes_along`, `translate`, `size_at`, and the undo history (`InkHistory`); ink.test.ts ported |
| `nodus-board/src/merge.rs` | MergeDialog.tsx's logic: `Merge` (items, phases, request ids, dirty/append rule, `can_start`), `default_name`, `section`, `initial_prompt`, `normalize_focus` |
| `board/overlays/mod.rs` | `Overlays` (per board mount), `kbd`, `spinner`, shadow mixing |
| `board/overlays/icons.rs` | the lucide icons only the overlays draw (`icons/overlays/…`) |
| `board/overlays/ink/` | `mod.rs` state, tools, colours, label measuring; `ops.rs` changes, history, labels; `input.rs` the pointer taken ahead of the board; `draw.rs` strokes, labels, selection, label input, the hitbox; `dock.rs` the dock and palette |
| `board/overlays/bar/` | `mod.rs` state and submit; `keys.rs` the field's keys; `view.rs` column, pill, token, field, hint, send; `popovers.rs` slash menu and colour picker; `voice.rs` the mic (spec 05's `voice::Mic`), orb, status line, notice; `text.rs` placeholder, send and mic labels (tested) |
| `board/overlays/selection.rs` | the toast and the selection bar above the row |
| `board/overlays/search/` | `mod.rs` state, rows, debounce, keys, open/mark/start; `view.rs` the modal; `text.rs` UTF-16 highlight runs and `timeAgo` |
| `board/overlays/merge/` | `mod.rs` the dialog's lifecycle, requests, progress events, start, the scenes' `SummarizeStub`; `view.rs` header, gathering, review, footer; `grip.rs` the instruction's resize grip |
| `parity/scenes11.py` | spec 11's scenes (bar, search, merge, ink) for the Rust app |

## Done (implemented, IDs in the code, checked against the goldens)

- **BAR** (40): 001, 002, 003, 004, 006, 007, 008, 009, 010, 011, 012, 013, 014, 015, 016, 017, 018, 019, 020, 021, 022, 023, 024, 025, 026, 027, 028, 029, 030, 031, 032, 033, 034, 035, 036, 037, 038, 039, 040, 041. The mic, its recording look, status line and notice are spec 05's `voice::Mic`, `voice_status` and `voice_notice` (actions-git-voice-ui); 024–031 are the board's side (`board/bar.rs`).
- **SRCH** (21): 001…021 (the latest-sessions order is tested). 021 is board-canvas's `board/fuzzy.rs` (fuzzy.test.ts ported there), used by the archive stack.
- **MERGE** (17): 001…017.
- **INKUI** (23): 001…015, 019…026. 012: GPUI has no pointer cancel; a gesture is dropped when the window lets go of the pointer (deactivation), as the board drops Space.
- **SEL** (6): 001…006.
- **BOARD** hooks (19): 037 (the preview opens at the match with `PreviewFind`), 108, 109, 131, 132, 133, 134, 135, 136, 137, 138, 143, 144, 145, 155, 159, 166, 167, 169 (board-canvas implemented 108/109, 131–137, 143–145; they are reached from the bar, search and merge dialog now).

## Checked side by side

`python3 crates/nodus-ui/parity/scenes11.py [prefix…] --golden <dir> --world <scratch>` renders spec 11's 20 scenes (the steps of `migration/parity/scenes/overlays.mjs`, plus board-populated and board-selected) in both themes; the merge scenes' summarize answers come from the driver's `stub` command, as 0.9.4's scenes replaced the IPC handler. Whole shots differ 7.6–8.6% from the goldens, as board-populated does: the cards' and notes' contents are board-cards' placeholders here. The overlays' own regions (pixelmatch, threshold 0.1):

| Scene | Light | Dark |
| --- | --- | --- |
| bar-focused, bar-typing, bar-view, bar-note, board-selected | 0.05–0.07% | 0.05–0.07% |
| bar-slash-menu | 0.11% | 0.11% |
| bar-one-selected | 1.3% (the token's tooltip: fixed in the scene, it parks the pointer) | 1.3% |
| bar-several-selected | 4.4% (the golden's board is scrolled 52px left: a Chromium focus-scroll quirk only the light shot has) | 0.07% |
| search-empty, search-results | 0.4%, 1.0% | 0.4%, 1.0% |
| search-none | 0.01% | (typed before the field took the keyboard; fixed) |
| merge-progress | 0.2% | 0.2% |
| merge-ready, merge-failed | 1.6%, 1.5% (text rasterisation, line rounding) | 1.2%, 1.0% |
| ink-pen, ink-arrow, ink-eraser, ink-palette (dock) | 0.00–0.38% | 0.00–0.39% |
| ink-text, ink-selected (label) | 3.1%, 3.1% (SF Pro tracking, gpui-text) | 2.3% |

## Known differences

- Text: GPUI's SF Pro tracking and rounding (gpui-text): labels, the merge context's mono lines (GPUI rounds its line height to device pixels, so 7 lines of 18.6px end ~3px lower).
- Search highlights: GPUI's text highlights have no corner radius (0.9.4's marks are 3px round) and the title's underline sits where GPUI puts it (0.9.4: 3px below the baseline).
- Native tooltips are drawn by GPUI in the window, so a shot with the pointer resting on a titled control shows one; 0.9.4's goldens never do.

## Deferred

- BAR-005: the field grows 1–5 lines then scrolls, as 0.9.4's; the spellcheck underline isn't drawn (GPUI's text input has no spellchecker; plan.md's "Decisions for the user" spellcheck row).
- INKUI-016: hovering a stroke with the pointer finds it, but GPUI has no `move` cursor (`CursorStyle` lacks it and custom cursor images); needs the patched GPUI.
- INKUI-017: pen, arrow and text show crosshair and I-beam over everything; the eraser's 20px circle cursor needs a custom cursor image (patched GPUI), and shows the crosshair meanwhile.
- INKUI-018: views have no live pages yet (spec 04's UI); `BoardCx.ink_tool` says when a tool is out, for the view renderer to ignore the pointer and show its snapshot (D3).
- BOARD-036: the jump selects and centres the card and sets `BoardCx.flash`; the flash is the card's to draw (board-cards).

## Edits outside the track's files

- `board/canvas.rs`: the `overlays` field. `board/render.rs`: ink input/layer/hitbox/dock, the bar column, search and merge drawn; nodes at `RAISED_Z` drawn over the ink; the HUD toast moved into the bar's column (BOARD-138). `board/hud.rs`: `render_toast` removed (the column draws it). `board/input.rs`: ⌘Z/⇧⌘Z, Esc, ⌫, P/A/X, V and `/` wired to the ink and the bar; `release_keys` drops an ink gesture. `board/store.rs`: `save_ink`.
- `board/nodes.rs`: `BoardCx.ink_tool` (and `canvas.rs`'s `board_cx` fills it), for views (INKUI-018).
- `components/icon.rs`: serves `icons/overlays/…`. `components/button.rs`: `height()`.
- `automation.rs`: `stub` (what the merge dialog's summarize answers in a scene).
- `Cargo.toml`: `lyon` (round caps and joins for ink strokes; GPUI tessellates with it).
