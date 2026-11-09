# Track: board-canvas (P6, spec 09)

- **Branch:** `rust/board-canvas`
- **Scope:** the board route and canvas in `crates/nodus-ui/src/board/` (everything but `board/cards/`, which is the board-cards track's): pan/zoom, selection, drag, sections, edges, menus, shortcuts, HUD (zoom pill, minimap, hint, toast, confirm), the store's board actions, and the node plug-in interface the cards, notes, archive, pad and views are drawn through.
- **Spec:** `migration/features/09-board-canvas.md` (BOARD-001…BOARD-177).
- **Status:** 🟡 work in progress. The plug-in interface is committed first so board-cards can target it; the canvas fills in behaviour group by behaviour group.

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
board::nodes::install(MyRenderers, cx);      // once, from board::init (add `cards::install(cx)` there)
```

- **Props.** Every node gets `NodeState { id, size (flow units), selected, dragging, zoom }`. `CardNode` adds the `SessionCard`, the archive search's `matched` title indices, its `stickers`, and for archived cards a `StackStyle { place: Pile { depth } | Result { slot }, rise: Option<offset>, nowheel }` (0.9.4's `stack-d*`, `is-result`, `--stack-i`, `stack-enter`, `--fly-x/y`). `NoteNode` adds the note and `draft` (the pad's top sheet). `ArchiveNode` adds `count`. `ViewNode` adds the `BoardView`.
- **Board context.** `BoardCx` = 0.9.4's contexts: `renaming`, `archive_hot`, `section_hot`, `naming`, `stack: ArchiveStack { open, hover, query, result_count, scroll, scroll_max }`, the canvas classes (`moving_archive`, `moving_section`, `scrolling_stack`, `stack_lift`), `zoom`, and `actions: BoardActions`.
- **Actions.** `BoardActions` has 0.9.4's `BoardActions` (`rename`, `set_renaming`, `rename_section`, `expand`, `archive`, `stop`, `edit_note`, `start_note`, `open_session`, `open_view_session`, `rename_view`, `view_menu`), the archive stack's (`set_stack_query`, `open_stack`, `close_stack`, `submit_stack`), the store actions nodes called (`patch_cards`, `save_note`, `remove_notes`, `patch_view`), and `hold_rect` (a node's live box while it's resized), `raise` (z 1400 while the agent fan is out), `toast`, `press_title`.
- **Units.** Contents are drawn inside `units::Zoomed` (rem = 16px × zoom); write lengths as `units::fu(px)` and they scale with the board, crisp. Custom elements read `units::scale(window)`. Shadows take `Pixels`: multiply by `node.zoom`.
- **Input.** GPUI bubbles from the innermost element out, so contents get each event first: a control that must not start a drag/click/menu stops its mouse-down's propagation (`nodrag`); an element that scrolls itself stops the scroll's propagation (`nowheel`); a card's title text calls `board.actions.press_title()` in its own mouse-down without stopping it (`.card-title`, BOARD-043/047). Drag handles (section header 44, archive header 52, view head) are the canvas's.
- **State.** Keep per-node state with `window.use_keyed_state((node.id.clone(), "…"), cx, …)`.

## Core API used

The board's commands (`board.load`, `saveNodes`, `saveArchive`, `saveViewport`, `removeNode`, `saveSection`, `removeSection`, `nameSection`, `saveNote`, `removeNotes`, `saveInk`) come from `nodus-core`'s `BoardApi`. It isn't on `main` yet, so this branch carries `crates/nodus-core/src/board.rs` **byte-identical to `rust/core`'s** (it merges cleanly) plus the `board()` accessor and its `mod`/`pub use` lines. Sessions and views commands (`sessions.create/kill/send`, `views.create/save/remove/brain/refresh`) go through an adapter in `board/core_api.rs` until core merges.

## Done

(none yet; IDs land here as each group is checked side by side with the goldens)

## Deferred

(to be filled in)
