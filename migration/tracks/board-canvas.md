# Track: board-canvas (P6, spec 09)

- **Branch:** `rust/board-canvas`
- **Scope:** the board route and canvas in `crates/nodus-ui/src/board/` (everything but `board/cards/`, which is the board-cards track's): pan/zoom, selection, drag, sections, edges, menus, shortcuts, HUD (zoom pill, minimap, hint, toast, confirm), the store's board actions, and the node plug-in interface the cards, notes, archive, pad and views are drawn through.
- **Spec:** `migration/features/09-board-canvas.md` (BOARD-001…BOARD-177).
- **Status:** 🟡 canvas in, checked against the goldens at canvas level; overlays (11), views (04), cards (10) and terminals (12) plug in later.

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
| `board/menus.rs` | every context menu, items/hints/enable rules verbatim |
| `board/render.rs` | drawing: dots, nodes by z with CSS-transition glides, edges (dashes march), selection rect, marquee, HUD |
| `board/section.rs`, `board/naming.rs` | the section node (frame, header, count, status, placeholder, rename field) and the naming glow/sweep/reveal |
| `board/hud.rs` | zoom pill, minimap, empty hint, toast, confirm dialog |
| `board/store.rs` | `store.ts`'s board actions on `AppState` (load, patchCards, upsertCard, removeCard, setArchive, sections, notes, views, statuses, openers) |

Shared files touched (additive): `lib.rs` (`pub mod board`), `app.rs` (board init, the core runtime global, `NodusConfig`'s new fields and `launch()` now that core is on main), `shell/root.rs` (the board route, key forwarding), `state.rs` (board loading, status/usage/agent events, `StateEvent::{BoardMenu, RunFinished}`), `components/icon.rs` (19 lucide icons appended), `automation.rs` (`open` command, `emit status`), `parity/scenes.py` + `parity/seed.mjs` (board scenes).

## The node plug-in interface (for board-cards and views)

`crates/nodus-ui/src/board/nodes.rs`. The canvas owns everything React Flow did (position, size, z-order, selection, drag, clicks, double-clicks, menus, wheel); a renderer draws what's inside a node's box.

```rust
pub trait NodeRenderers: 'static {           // every method has a placeholder default
    fn card(&self, node: CardNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn note(&self, node: NoteNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn archive(&self, node: ArchiveNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn pad(&self, node: PadNode, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
    fn view(&self, node: ViewNode<'_>, board: &BoardCx, window: &mut Window, cx: &mut App) -> AnyElement;
}
board::nodes::install(MyRenderers, cx);      // once, from board::init (add `pub mod cards;` + `cards::install(cx)` there)
```

- **Props.** Every node gets `NodeState { id, size (flow units), selected, dragging, zoom }`. `CardNode` adds the `SessionCard`, the archive search's `matched` title indices, its `stickers`, and for archived cards a `StackStyle { place: Pile { depth } | Result { slot }, rise: Option<offset>, nowheel }` (0.9.4's `stack-d*`, `is-result`, `--stack-i`, `stack-enter`, `--fly-x/y`). `NoteNode` adds the note and `draft` (the pad's top sheet). `ArchiveNode` adds `count`. `ViewNode` adds the `BoardView`. The canvas places the box, glides it (420/620 ms) and clips results at the panel's band; the card draws its own depth, rise and deal-out.
- **Board context.** `BoardCx` = 0.9.4's contexts: `renaming`, `archive_hot`, `section_hot`, `naming`, `stack: ArchiveStack { open, hover, query, result_count, scroll, scroll_max }`, the canvas classes (`moving_archive`, `moving_section`, `scrolling_stack`, `stack_lift`), `zoom`, and `actions: BoardActions`.
- **Actions.** `BoardActions` has 0.9.4's `BoardActions` (`rename`, `set_renaming`, `rename_section`, `expand`, `archive`, `stop`, `edit_note`, `start_note`, `open_session`, `open_view_session`, `rename_view`, `view_menu`), the archive stack's (`set_stack_query`, `open_stack`, `close_stack`, `submit_stack`), the store actions nodes called (`patch_cards`, `save_note`, `remove_notes`, `patch_view`), and `hold_rect` (a node's live box while it's resized), `raise` (z 1400 while the agent fan is out), `toast`, `press_title`.
- **Units.** Contents are drawn inside `units::Zoomed` (rem = 16px × zoom); write every length as `units::fu(px)` — **line heights too** (`.line_height(fu(18.2))`; a px line height doesn't scale and lines overlap when zoomed in) — and they scale with the board, with crisp text. Custom elements read `units::scale(window)`. Shadows, icon sizes and `px()` APIs take pixels: multiply by `node.zoom`.
- **Input.** GPUI bubbles from the innermost element out, so contents get each event first: a control that must not start a drag/click/menu stops its mouse-down's propagation (`nodrag`); an element that scrolls itself stops the scroll's propagation (`nowheel`); a card's title text calls `board.actions.press_title()` in its own mouse-down without stopping it (`.card-title`, BOARD-043/047). Drag handles are the canvas's: the section header (44), the archive header (52), the view head (38).
- **State.** Keep per-node state with `window.use_keyed_state((node.id.clone(), "…"), cx, …)`.
- **Core calls** from node code: run them on the core's runtime with `board::on_core(&board::core_rt(cx), async move { … })` (tokio processes and the host client panic on GPUI's executor).

## Checked side by side

Rendered with `python3 crates/nodus-ui/parity/scenes.py board- --golden <dir> --world <scratch>` (statuses emitted as `storefront()` starts them) and compared with `compare.mjs`. Canvas-level geometry matches the goldens: archive/pad/section frames, card boxes, edges, the marquee rect, the fit (117% on mobile-app, 80% saved on storefront), 40%/160% zooms, the zoom pill, minimap (node colours by status, mask), hint, pane/section/archived-card menus (text rasterisation only), the naming glow and sweep, the section header (name, count pill, status dot). What differs in those shots belongs to other tracks: card/note/archive/pad contents (placeholders here), ink, the prompt bar, the title bar's git chips.

## Done (implemented; IDs in the code)

BOARD-001…BOARD-035, 037…041, 043, 045…107, 110…128, 139…141, 146…154, 156…158, 160…165, 168, 170, 172…177.

## Partly done / deferred

- BOARD-036: selects and centres the card; the 2×800ms flash is the card's (10) — needs a `flash` hook on `CardNode`.
- BOARD-042, BOARD-044 (expanded): focusing and waking an expanded card's terminal is 12's registry.
- BOARD-108, BOARD-109: merge sources are computed; the merge dialog is 11's.
- BOARD-129, BOARD-130: `start_view_session` is in; registering relay handlers and broadcasting to view pages wait for 04's webviews.
- BOARD-131…BOARD-138, BOARD-143, BOARD-145, BOARD-155, BOARD-159, BOARD-166, BOARD-167, BOARD-169: the prompt bar, selection bar, ⌘K search and ink layer are 11's; their keys are already taken (default prevented) as 0.9.4 does.
- BOARD-142: overlay = context menu or confirm; merge, dock, view dialog and search join when 11/04/05 land.
- BOARD-144: the New view dialog (04).
- The toast sits at `bottom: 84px` until the prompt bar's `above` slot exists (BOARD-138).
- `onlyRenderVisibleElements` above 150 cards: all nodes are drawn.

## Known differences

- Text: GPUI's SF Pro tracking and rounding (see ui-shell; gpui-text fixes it).
- The rename field's text isn't drawn selected in the driven (unfocused) window.
- Dots below zoom ≈ 0.15 aren't drawn (Chromium's pattern is an invisible wash there).
- Trackpad pinch arrives as GPUI `PinchEvent`s; they're converted to Chromium's `deltaY = −100·ln(scale)` ctrl-wheel, so the feel matches if Chromium's conversion is as read.
