# Track: board-canvas (P6, spec 09)

- **Branch:** `rust/board-canvas`
- **Scope:** the board route and canvas in `crates/nodus-ui/src/board/` (everything but `board/cards/`, which is the board-cards track's): pan/zoom, selection, drag, sections, edges, menus, shortcuts, HUD (zoom pill, minimap, hint, toast, confirm), the store's board actions, and the node plug-in interface the cards, notes, archive, pad and views are drawn through.
- **Spec:** `migration/features/09-board-canvas.md` (BOARD-001…BOARD-177).
- **Status:** ✅ ready to merge: 164 of 177 behaviors done and checked at canvas level against the goldens; 13 deferred to the tracks that draw what they need (cards 10, overlays 11, terminals 12, views 04, actions 05). Main merged in (409274f).

## Files

| File | What |
| --- | --- |
| `board/mod.rs` | `init` (installs node renderers), the loading surface (BOARD-001), the title bar's split button (BOARD-147), `CoreRuntime`/`on_core` (core commands run on the core's tokio runtime) |
| `board/nodes.rs` | **the node plug-in interface** (below) |
| `board/units.rs` | `fu()` flow units and `Zoomed` (rem = 16px × zoom) |
| `board/viewport.rs` | React Flow/d3 viewport maths: clamp, wheel/pinch formulas, `getViewportForBounds`, `setCenter`, `interpolateZoom` flights eased cubic-in-out, `calcAutoPan` (tested) |
| `board/scene.rs` | `buildNodes` and the build*Nodes helpers: pile, results, clip, sink/rise flights, z rules, draggable/selectable, held positions (tested) |
| `board/layout.rs` | ports of `layout.ts`, `sections.ts`, `notes.ts`, `views.ts` (tested), incl. the `createSectionAt` quirk (BOARD-092) pinned |
| `board/edges.rs` | which edges, facing-side anchors, `getBezierPath` (tested) |
| `board/fuzzy.rs` | `fuzzy.ts` in UTF-16 units (its Vitest cases ported) |
| `board/canvas.rs` | `BoardCanvas`: state, snapshot sync, rebuild, viewport ops and persistence, toasts, confirm, archive stack state, minimap pan/zoom |
| `board/input.rs` | pointer/wheel/pinch/keys: XYDrag, the pane/marquee, clicks/double-clicks, drop resolution, shortcuts and guards, menu commands |
| `board/commands.rs` | the board's commands (create, archive, stop, restore, remove, sections, notes, views, runs, open requests, jump) |
| `board/bar.rs` | the board's side of the overlays: prompt bar submissions (`Submission`, `submit_from_bar`, `bar_into`, `bar_context_count`), the merge dialog's state and session (`start_merge`, `merge_start`, `close_merge`), the selection bar's counts (`selection_bar`), ⌘K search's state (`open_search`, `search_query`, `search_start`), the New view dialog's point (`open_view_dialog`) |
| `board/menus.rs` | every context menu, items/hints/enable rules verbatim |
| `board/render.rs` | drawing: dots, nodes by z with CSS-transition glides, edges (dashes march), selection rect, marquee, HUD |
| `board/section.rs`, `board/naming.rs` | the section node (frame, header, count, status, placeholder, rename field) and the naming glow/sweep/reveal |
| `board/hud.rs` | zoom pill, minimap, empty hint, toast, confirm dialog |
| `board/store.rs` | `store.ts`'s board actions on `AppState` (load, patchCards, upsertCard, removeCard, setArchive, sections, notes, views, statuses, openers) |

Shared files touched (additive): `lib.rs` (`pub mod board`), `app.rs` (board init and the core runtime global), `shell/root.rs` (the board route, key forwarding), `state.rs` (board loading, status/usage/agent events, `StateEvent::{BoardMenu, RunFinished}`), `components/icon.rs` (19 lucide icons appended), `automation.rs` (`open` command, `emit status`; and every command first draws the window, as GPUI stops drawing a covered one and input would hit a stale frame), `parity/scenes.py` + `parity/seed.mjs` (board scenes), `nodus-board/src/compose.rs` (`compose.ts`: `ContextItem`, `with_context`, the slash commands, with its Vitest cases; spec 11 places it there).

## The node plug-in interface (for board-cards and views)

`crates/nodus-ui/src/board/nodes.rs`. The canvas owns everything React Flow did (position, size, z-order, selection, drag, clicks, double-clicks, menus, wheel); a renderer draws what's inside a node's box.

```rust
pub trait NodeRenderers: 'static {           // every method has a placeholder default
    fn card(&self, node: CardNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn note(&self, node: NoteNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn archive(&self, node: ArchiveNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn pad(&self, node: PadNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn view(&self, node: ViewNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn wake(&self, session_id: &str, cx: &mut App);                              // default: nothing
    fn focus_terminal(&self, session_id: &str, window: &mut Window, cx: &mut App); // default: nothing
}
board::nodes::install(MyRenderers, cx);      // once, from board::init (add `pub mod cards;` + `cards::install(cx)` there)
```

- **Props.** Every node gets `NodeState { id, size (flow units), selected, dragging, zoom }`. `CardNode` adds the `SessionCard`, the archive search's `matched` title indices, its `stickers`, and for archived cards a `StackStyle { place: Pile { depth } | Result { slot }, rise: Option<offset>, nowheel }` (0.9.4's `stack-d*`, `is-result`, `--stack-i`, `stack-enter`, `--fly-x/y`). `NoteNode` adds the note and `draft` (the pad's top sheet). `ArchiveNode` adds `count`. `ViewNode` adds the `BoardView`. The canvas places the box, glides it (420/620 ms) and clips results at the panel's band; the card draws its own depth, rise and deal-out.
- **Board context.** `BoardCx` = 0.9.4's contexts: `renaming`, `archive_hot`, `section_hot`, `naming`, `stack: ArchiveStack { open, hover, query, result_count, scroll, scroll_max }`, the canvas classes (`moving_archive`, `moving_section`, `scrolling_stack`, `stack_lift`), `zoom`, `flash` (the card a search jump flashes and since when: two 800 ms rounds of `0 0 0 2px accent, 0 0 0 2px→18px accent 55%→0%`, BOARD-036), and `actions: BoardActions`.
- **Hooks.** `wake(session_id)` is `wakeCardTerminal` (XCARD-025): the canvas calls it when an expanded card is opened (Enter, a click's open, a notification, BOARD-044) and for every card it expands (E, the menus, `actions.expand`, BOARD-150/168), so board-cards' `CanvasActions::expand` no longer needs to wake by itself. `focus_terminal(session_id)` is the terminal registry's `focus`, called on a click on an expanded card (BOARD-042).
- **Actions.** `BoardActions` has 0.9.4's `BoardActions` (`rename`, `set_renaming`, `rename_section`, `expand`, `archive`, `stop`, `edit_note`, `start_note`, `open_session`, `open_view_session`, `rename_view`, `view_menu`), the archive stack's (`set_stack_query`, `open_stack`, `close_stack`, `submit_stack`), the store actions nodes called (`patch_cards`, `save_note`, `remove_notes`, `patch_view`), and `hold_rect` (a node's live box while it's resized), `raise` (z 1400 while the agent fan is out), `toast`, `press_title`.
- **Units.** Contents are drawn inside `units::Zoomed` (rem = 16px × zoom); write every length as `units::fu(px)` — **line heights too** (`.line_height(fu(18.2))`; a px line height doesn't scale and lines overlap when zoomed in) — and they scale with the board, with crisp text. Custom elements read `units::scale(window)`. Shadows, icon sizes and `px()` APIs take pixels: multiply by `node.zoom`.
- **Input.** GPUI bubbles from the innermost element out, so contents get each event first: a control that must not start a drag/click/menu stops its mouse-down's propagation (`nodrag`); an element that scrolls itself stops the scroll's propagation (`nowheel`); a card's title text calls `board.actions.press_title()` in its own mouse-down without stopping it (`.card-title`, BOARD-043/047). Drag handles are the canvas's: the section header (44), the archive header (52), the view head (38).
- **State.** Keep per-node state with `window.use_keyed_state((node.id.clone(), "…"), cx, …)`.
- **Core calls** from node code: run them on the core's runtime with `board::on_core(&board::core_rt(cx), async move { … })` (tokio processes and the host client panic on GPUI's executor).
- **Changes since board-cards merged a2f48b8** (all additive; nothing it uses changed): `BoardCx.flash`, `NodeRenderers::{wake, focus_terminal}` with no-op defaults.

## For the overlays (11) and views (04)

The canvas holds the overlays' state and does what 0.9.4's BoardView did for them (`board/bar.rs`); the overlay tracks draw them and call these:

- **Prompt bar:** `bar_into()` (BAR-024's `into`), `bar_context_count()`, `submit_from_bar(Submission::{Session, Into, View, Note}) -> Task<bool>` (true = clear the bar). The toast is `canvas.toast` (drawn by `hud.rs` at `bottom: 84px` until the bar's `above` slot takes it, BOARD-138). V, `/`, P/A/X and ⌘Z/⇧⌘Z are already taken by `on_key_down` (default prevented, guards applied) and do nothing yet: wire them to the bar and ink there.
- **Selection bar:** `selection_bar() -> Option<SelectionCounts>` (shown when `Some`); its buttons call `start_merge(&selected_cards(), None)`, `group_cards`, `archive_cards`, `clear_selection`.
- **Merge dialog:** shown while `canvas.merge` is `Some(sources)`, with `merge_focus`; `merge_start(MergeStart) -> Task<Result<…>>` creates the session (errors go to the dialog), `close_merge()` cancels.
- **⌘K search:** shown while `search_open` (⌘K and Edit › Search set it); `search_query` persists; `jump_to_card(id)` and `search_start(ids)` close it; `close_search()`.
- **New view dialog (04):** shown while `view_dialog` is `Some(at)` (pane menu: a flow point; title bar: `None`, in view); create with `create_view(description, at)`, then `close_view_dialog()`.
- `overlay_open()` already counts the merge dialog, search and the view dialog (BOARD-142).

## Checked side by side

Rendered with `python3 crates/nodus-ui/parity/scenes.py board- --golden <dir> --world <scratch>` (statuses emitted as `storefront()` starts them; 23 board scenes × 2 themes) and compared with `compare.mjs`. Every scene matches its golden at canvas level: the fit (117% on mobile-app, 80% saved on storefront, 72% on infra), 40%/160% zooms, archive/pad/section frames, card and note boxes, edges and their anchors, the marquee and the selection rectangle, the zoom pill, minimap (node colours by status, the mask), the empty hint, the pane/card/note/section/archived-card menus, the confirm dialog, the toast, the naming glow and sweep, the section header and its rename field. What differs belongs to other tracks: card, note, archive and pad contents (placeholders here, 10), ink and the prompt bar (11), the expanded cards' terminals (12), the title bar's git chips (05), and text rasterisation (gpui-text). Whole-shot diff with the goldens: zoomed-out 2.4–2.5%, empty 5.9–6.4%, the storefront scenes 7.6–9.3%, marquee/selected 9.1–10.2%, zoomed-in 16% (the cards' contents fill the screen), expanded-stopped 2.5% dark / 27.7% light (the terminals' black bodies).

## Done (implemented, IDs in the code, checked against the goldens)

BOARD-001, 002, 003, 004, 005, 006, 007, 008, 009, 010, 011, 012, 013, 014, 015, 016, 017, 018, 019, 020, 021, 022, 023, 024, 025, 026, 027, 028, 029, 030, 031, 032, 033, 034, 035, 038, 039, 040, 041, 043, 045, 046, 047, 048, 049, 050, 051, 052, 053, 054, 055, 056, 057, 058, 059, 060, 061, 062, 063, 064, 065, 066, 067, 068, 069, 070, 071, 072, 073, 074, 075, 076, 077, 078, 079, 080, 081, 082, 083, 084, 085, 086, 087, 088, 089, 090, 091, 092, 093, 094, 095, 096, 097, 098, 099, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 131, 132, 133, 134, 135, 136, 137, 139, 140, 141, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 156, 157, 158, 160, 161, 162, 163, 164, 165, 168, 170, 171, 172, 173, 174, 175, 176, 177 (164).

108/109, 131–137 and 143–145 are the board's side (`board/bar.rs`); the dialogs, bar and search that call them are 11's and 04's. BOARD-092 keeps 0.9.4's `createSectionAt` quirk (pinned by a test), as the spec asks until the owner decides.

## Deferred (13)

- BOARD-036: selects and centres the card and sets `BoardCx.flash`; the flash is drawn by the card (10).
- BOARD-037: opens the archived card's transcript preview; opening it at the match needs the preview's `{query, snippet}` (12).
- BOARD-042: calls `NodeRenderers::focus_terminal`; focusing needs the terminal registry (12) through board-cards (10).
- BOARD-044: archived → preview and compact → terminal modal are in; expanded calls `NodeRenderers::wake`, which board-cards implements (10).
- BOARD-129, BOARD-130: `start_view_session` (VIEW-056…058) is in; registering the view relay's handlers and broadcasting `board`/`git` to view pages need the webviews (04).
- BOARD-138: toasts and the selection bar stacked in the prompt bar's `above` slot (11); the toast sits at `bottom: 84px` until then.
- BOARD-142: the context menu, confirm, merge dialog, New view dialog and search count; the actions rail's dialogs and menu join with its UI (05).
- BOARD-155: the ink layer clearing the selection and toasting (11).
- BOARD-159, BOARD-166, BOARD-167, BOARD-169: ⌘Z/⇧⌘Z, V, `/`, P/A/X are taken with 0.9.4's guards (default prevented) but act once the ink layer and prompt bar exist (11).

## Known differences

- Text: GPUI's SF Pro tracking and rounding (see ui-shell; gpui-text fixes it).
- The rename field's text isn't drawn selected in the driven (unfocused) window.
- Dots below zoom ≈ 0.15 aren't drawn (Chromium's pattern is an invisible wash there).
- Trackpad pinch arrives as GPUI `PinchEvent`s; they're converted to Chromium's `deltaY = −100·ln(scale)` ctrl-wheel, so the feel matches if Chromium's conversion is as read.
