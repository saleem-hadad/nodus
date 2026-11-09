# 09 · Board canvas

Sources:
- `src/renderer/src/views/BoardView.tsx` (all 2683 lines: `BoardView`, `buildNodes`, `BoardCanvas`)
- `src/renderer/src/components/board/layout.ts`, `sections.ts`, `BoardContext.ts`, `BoardControls.tsx`, `ConfirmDialog.tsx`, `LineageEdge.tsx`, `SectionNode.tsx`, `SelectionBar.tsx` (wiring only), `board.css` (canvas-level rules; card/note/archive internals are spec 10)
- `src/shared/geometry.ts` (constants; behaviour owned by 03)
- `src/renderer/src/styles/tokens.css`, `styles/global.css` (`.btn`, body type), `components/Modal.tsx` + `Modal.css`, `components/ContextMenu.tsx` + `.css` (chrome owned by 08; used here)
- Read for wiring: `components/board/notes.ts`, `views.ts`, `stopSessions.ts`, `InkLayer.tsx` (handle API), `sidekick/Sidekick.tsx` + `sidekick.css` (the `above` slot), `store.ts` (board actions), `App.tsx` (menu/Ctrl routing, `BoardView key={projectId}`), `main/menu.ts` (accelerators), `platform.ts`
- React Flow as installed: `@xyflow/react` 12.12.0 and `@xyflow/system` 0.0.83 (`node_modules/@xyflow/react/dist/esm/index.mjs`, `dist/style.css`, `@xyflow/system/dist/esm/index.mjs`). Every "React Flow default" below was read from that source, not from docs.
- `docs/how-it-works.md` (§Expanded cards, §Notes, §Shortcuts › Board), `docs/screenshots/board*.png`, `expanded*.png`, `context*.png`, `archive*.png`, `views*.png` (older screenshots: their title bar shows separate `Terminal`/`View` buttons; the code's split button wins).

## Summary

The board is one project's infinite, pannable, zoomable canvas. It draws, back to front: a dotted background; section frames; the archive frame (a stack of archived cards that opens into a searchable panel) and the note pad under it; lineage edges; session cards (compact or expanded with a live terminal), views and sticky notes; then HUD chrome (zoom pill bottom-left, minimap bottom-right, the empty-board hint at the top, the prompt bar with toasts and the selection bar at the bottom centre, the actions rail on the right). It is built on React Flow today; the Rust port re-implements every React Flow mechanic it relies on: wheel/trackpad pan, pinch and modifier-wheel zoom, middle-drag and Space-drag pan, marquee selection with partial overlap, Shift/⌘ multi-select, a 1px drag threshold, selection elevation, auto-pan at the edges, fit-view maths, the minimap and the post-marquee selection rectangle.

On top of that the board orchestrates: where new cards, notes, views and sections go (free-spot search), drag-and-drop resolution (into/out of sections, into/out of the archive, bounce-backs), every context menu, every board shortcut and its guards, viewport persistence (400ms debounce), the archive stack's open/close/scroll/flight state, the confirm dialog, toasts, and the wiring of overlays owned by 11 (prompt bar, ⌘K search, merge dialog, ink, selection bar) and nodes owned by 10 (cards, notes, archive) and 04 (views).

Layering of ownership: this spec owns the canvas, the scene (which nodes exist, their positions, sizes, z-order, flags), all input routing and all board-level commands. Specs 10/04 own what is painted inside a node and its internal controls; 11 owns overlay internals; 08 owns the title bar, `ContextMenu`/`Modal` chrome and tokens; 03 owns geometry semantics and data.

## Canvas mechanics

All values are from the installed React Flow + the props in `BoardView.tsx:2533-2571`. "Flow units" = board coordinates; "px" = screen pixels. Viewport = `{x, y, zoom}`: screen = flow × zoom + (x, y), relative to the canvas element's top-left.

### Viewport and limits

| Item | Value |
| --- | --- |
| `minZoom` / `maxZoom` | `0.1` / `2` (clamp for wheel, pinch, buttons, minimap, initial viewport) |
| `translateExtent` | infinite (no pan limits) |
| `nodeExtent` | infinite |
| Initial viewport | the board's saved `viewport` (`board.viewport`, state.json) clamped to [0.1, 2]; if none, React Flow's `{x: 0, y: 0, zoom: 1}` then `fitActive(0)` (no animation) once the canvas initialises (`onInit`, fired via `setTimeout(…, 1)` after the pan-zoom is ready). |
| Persistence | every viewport change end (`onMoveEnd`) restarts a 400ms timer; when it fires, `board.saveViewport(projectId, {x, y, zoom})`. Pending timer is cancelled on unmount (a change in the last 400ms before leaving is lost). |
| `onMoveStart` | fires for user pans/zooms **and** programmatic animations (fit, `setCenter`, zoom buttons, minimap); closes the context menu. `onMoveEnd` likewise fires for both (so programmatic moves are saved too). |
| Move start/end for wheel-pan | start on the first wheel event; end 150ms after the last wheel event. For drags/pinches end fires on gesture end, then `onMoveEnd` is delivered after a further 150ms timeout (`panOnScroll` is on, so React Flow always delays end by 150ms). |
| Animated viewport changes | d3 transition: duration as given, easing cubic-in-out (`t<0.5 ? 4t³ : …`, d3 `easeCubicInOut`), interpolation d3 `interpolateZoom` (van Wijk & Nuij smooth zoom, ρ = √2) between the view centres and view widths. Duration ≤ 0 or non-number → instant. |
| `zoomIn` / `zoomOut` | scale by `1.2` / `1/1.2` about the canvas centre, clamped. |
| `zoomTo(z)` | scale to z about the canvas centre. |
| `setCenter(cx, cy, {zoom, duration})` | viewport = `{x: W/2 − cx·zoom, y: H/2 − cy·zoom, zoom}` (W,H = canvas size). Not clamped by min/max. |
| `fitView({nodes?, padding, maxZoom?, duration})` | bounds = union of the listed nodes (or all) that are measured and not hidden; zoom = clamp(min((W − 2·px)/bw, (H − 2·py)/bh), minZoom, maxZoom ?? 2) with `px = floor((W − W/(1+padding))·0.5)`, `py` likewise with H; centred on the bounds' centre; then shifted so no side's applied padding is smaller than required. Queued until nodes are measured. |
| `fitBounds(rect, {padding, duration})` | same maths for an explicit rect, with the store's [0.1, 2]. |
| `screenToFlowPosition(client)` | `((clientX − canvasLeft − x)/zoom, (clientY − canvasTop − y)/zoom)`; no snapping. |

### Background

`Background` variant Dots, gap `22` flow units, size `1.6` → each dot is a circle of radius `0.8 × zoom` px, spaced `22 × zoom` px; dot centres at flow coordinates `(22·i − 10.2, 22·j − 10.2)` (pattern origin `x % gap` shifted by −gap/2, circle at (r, r)); colour `--canvas-dot`, fill `--bg-canvas`. If `22 × zoom` evaluates to 0 the gap is treated as 1 (cannot happen within [0.1, 2]).

### Wheel, trackpad and pinch

| Input | Condition | Effect |
| --- | --- | --- |
| Wheel / two-finger scroll | no zoom-activation key held, target not inside a `nowheel` region | pan: translate by `−deltaX/zoom × 0.5`, `−deltaY/zoom × 0.5` flow units (screen moves `0.5 × delta` px). `deltaMode === 1` (lines) multiplies deltas by 20. Off a Mac, Shift+wheel pans horizontally by deltaY (deltaY becomes deltaX, deltaY = 0). No inertia beyond what the OS sends. `preventDefault` on the event. |
| Pinch (wheel with `ctrlKey`) | `zoomOnPinch` true | zoom about the pointer: `zoom' = zoom × 2^(−deltaY × m × f)` with `m = 0.05` for line mode, `1` for page mode, `0.002` for pixel mode, and `f = 10` on macOS (ctrlKey + Mac) else `1`; clamped to [0.1, 2]. Instant (no transition). |
| ⌘+wheel (Mac) / Ctrl+wheel (Windows/Linux) | zoom-activation key (`Meta` on Mac, `Control` elsewhere) held | the same zoom-about-pointer formula through d3's wheel zoom (Ctrl on Windows also counts as the pinch path; both give factor 1 off Mac). d3 ends the gesture 150ms after the last wheel event. |
| Wheel over a `nowheel` region | — | the board ignores it (the element scrolls natively); with ctrlKey the event is `preventDefault`ed so the page doesn't zoom, and the board does **not** zoom. |
| Wheel over the open archive panel with overflow | panel/results carry `nowheel` | board handler scrolls the results instead (see Archive stack orchestration). |

`nowheel` regions (owned by their specs): the open archive panel and its result cards while results overflow (here), the agent fan (10), a note being edited (10), a selected view's page (04/10). Expanded-card terminals take the wheel themselves except a pinch, and except while a board-pan gesture that started elsewhere continues over them (10/12).

### Drag-to-pan

| Input | Effect |
| --- | --- |
| Middle button (`button 1`) drag | pans, starting anywhere — on the pane **and** over nodes, edges or the selection rectangle (React Flow's filter lets middle-button mousedown through on nodes), unless the node's content stops the event (e.g. an editing field). |
| Space held + left drag | pans (left drag on the pane pans instead of starting a marquee; node drags still drag nodes). Space only counts when it is the only key down, focus is not in an input/textarea/select/contenteditable/`.nokey`; its keydown is `preventDefault`ed unless the target is a `<button>`/`<a>`. Space-held also forces wheel = pan. Released on keyup, window blur, or any `contextmenu` event. |
| Left drag on the pane (no Space) | marquee (below), never pan. |
| Right drag | nothing (right button never pans or drags). |
| Pan cursor | pane cursor is `default`; while a pan drag is in progress `grabbing`. (React Flow's `grab` for `.draggable` panes is overridden by `board.css` and never shows.) |

### Selection

| Mechanic | Value |
| --- | --- |
| Marquee start | left button, primary pointer, pointer-down **on the pane itself** (not on a node; section frame interiors are click-through so they count as pane). Starts once the pointer moves **more than 1px** (`paneClickDistance = 1`, screen px) from the down point. |
| Marquee start effect | clears the whole selection (cards, notes, views) first — a marquee always replaces, even with Shift/⌘ held. |
| Marquee hit rule | `SelectionMode.Partial`: any node whose rect overlaps the marquee with area > 0; only selectable, non-hidden nodes (active cards, archived result cards, notes, views; never sections, archive, pad, pad's top sheet, pile cards). Live while dragging. |
| Marquee auto-pan | while the pointer is within 40px of a canvas edge the board pans by `clamp(distanceIntoBand, 1, 40)/40 × 15` px per animation frame per axis (towards the edge), extending the marquee. |
| Marquee visual | fill accent 8%, border 1px solid accent 55%, no radius, drawn above all nodes (z 6). |
| After a marquee that selected ≥ 1 node | a **selection rectangle** (React Flow `NodesSelection`) covers the bounding box of all selected nodes with the same fill and border, **above** the nodes (z 3 over the node layer). It captures the pointer: dragging it moves every selected node (and fires the same drag-start/drag/drop handling as dragging a node, with the first selected node as "the node"); clicks, double-clicks and right-clicks on it do nothing (no menu, no open). Cursor `grab`. It follows the selection's bounds and disappears when nothing is selected, on a pane click, or on a click on any node. Not shown for selections made by click, ⌘A, search jump or section `Select cards`. |
| Click on a node (selectable) | not selected → it becomes the only selected node (or is added, with multi-select held); selected + multi-select held → it is deselected; selected without modifier → nothing changes (a multi-selection stays). Clicking also clears the selection rectangle. |
| Multi-select keys | `Meta` or `Shift` held alone (`multiSelectionKeyCode = ['Meta', 'Shift']`, on every platform; Windows key counts as Meta; Ctrl does not). Tracked on window keydown/keyup; reset on blur/`contextmenu`/Meta keyup. |
| Click on empty pane | deselects every node (cards, notes, views), clears the selection rectangle, closes the context menu. A pane click is a pointer-up on the pane with no marquee started (≤ 1px movement). |
| Selection elevation | `elevateNodesOnSelect`: a selected node's z-index is its own + 1000. Edges are not elevated. |
| Store sync | after every selection change, the store's `selection` = ids of selected **session** nodes (notes/views stay local to the canvas). |
| Keyboard a11y | disabled: no arrow-key nudging, no auto-pan on focus, no focus of the selection rectangle. Cards, notes and views are still `tabIndex=0` (focus outline suppressed). |

### Node drag

| Mechanic | Value |
| --- | --- |
| Threshold | drag starts when the pointer has moved **> 1px** (screen, Euclidean) from pointer-down (`nodeDragThreshold = 1`). |
| Click suppression | `nodeClickDistance = 0`: if the pointer moved at all (> 0px) between down and up on a draggable node, no click fires (no select-on-click, no open). Movement in (0, 1] px therefore yields neither click nor drag. |
| Buttons | left only (`!event.button`). |
| Handles | sections drag only by `.section-header` (44px strip), the archive only by `.archive-header` (52px strip), views only by `.view-head` (04); cards and notes anywhere except `nodrag` regions (10). |
| Select on drag | starting a drag on an unselected selectable node selects it (replacing the selection unless multi-select held) before moving. Starting a drag on a non-selectable node (section, archive, pile top card) while no multi-select key is held **clears the whole selection**. |
| What moves | the grabbed node plus every selected draggable node (pile cards and non-draggable nodes never). Positions = pointer − grab offset, unsnapped (`snapToGrid` off), unclamped. |
| Auto-pan | as for the marquee: within 40px of an edge, up to 15px/frame, moving the dragged nodes with it. |
| Cursor | `grabbing` while a node is dragged; otherwise nodes show `default` (React Flow's `grab`/`pointer` are overridden by `board.css`), section and archive headers show `grab` (`grabbing` while dragging). |
| Abort | a node deleted mid-drag aborts the drag. |

### Z-order (back to front)

Within the transformed node layer, ordering is z-index, then document order `[edges…] [archive, pad, sections (oldest first), cards (board order), views, free notes, pad top sheet]`.

| z | What |
| --- | --- |
| −2 | section frames |
| −1 | archive frame (when not raised), note pad |
| 0 | all edges (drawn before nodes of equal z, so under them); compact active cards; views |
| 1 | expanded cards |
| 0…3 | folded-stack cards while not raised: `3 − depth` for depth 0–2, 0 deeper (hidden) |
| 2 | free notes and the pad's top sheet |
| +1000 | any selected node (added to its own z) |
| 1400 | a card node whose agent fan is open (`!important`, 10) |
| 1500 | the archive frame while open and for 700ms after it closes ("raised") |
| 1500 + (3 − depth) | folded-stack cards while raised |
| 1510 | archive result cards (only exist while open) |

Above the node layer: selection rectangle (z 3 inside the pane), marquee (z 6), then HUD: minimap and zoom pill (z 5 over the whole flow), empty-board hint (z 4, outside the flow), prompt bar column (z 20), context menu (fixed, z 1000), confirm/other modals (backdrop z 500). Ink layer: 11.

### Hit-testing / pointer-events

- Edges are never hit (`selectable: false`, no click handler → `pointer-events: none`); clicks pass through to whatever is under them.
- Section frames are click-through (`pointer-events: none !important`) except their 44px header; clicks, double-clicks, right-clicks and marquees inside a frame act on the pane.
- Pile cards (archived, not results, depth ≥ 1) are click-through (`pointer-events: none !important`); the archive frame takes those points.
- The note pad frame takes the pointer (it has a click handler) but does nothing on click/double-click.
- Hidden nodes are neither drawn nor hit nor counted (minimap, fit, marquee).
- `onlyRenderVisibleElements` is on when the board has more than 150 cards (cards after removing runs and view brains, archived included): only nodes (and edges between visible nodes) overlapping the canvas rect are rendered.

### Not used / off

Connections (`nodesConnectable={false}`), edge selection/focus, React Flow's delete key (`deleteKeyCode={null}`), selection key (`selectionKeyCode={null}`), zoom on double-click (`zoomOnDoubleClick={false}`), snapping, attribution (`hideAttribution`).

## Behaviors

### Mounting, loading and teardown

- **BOARD-001** The board for project P renders a loading surface (fills the board area, background `--bg-canvas`) while the store's board is missing or belongs to another project; it shows a 20px spinning `Loader2` (`--text-tertiary`, one turn per 0.9s, linear) only while the store's `boardLoading` is true, and is `aria-busy`.
- **BOARD-002** The board view is keyed by project id: switching projects tears the canvas down and builds a new one (fresh viewport from that board's saved viewport, fresh local state).
- **BOARD-003** Cards with `runOf` (action runs) or `viewOf` (view brains) are removed from the snapshot before anything on the board (nodes, edges, counts, search list, hints) sees it.
- **BOARD-004** On unmount the board unregisters its view-relay handlers, its keydown listener and menu-command subscription, and cancels its title-click, stack-scroll and viewport-save timers.
- **BOARD-005** A new board snapshot (from `ev:board-changed` → reload, status, usage or agent-status updates, or any local store action) re-derives the scene while keeping, for every node, its local selection and measurements, and — for nodes being dragged, resized or marked busy — their live position (and size while resizing).
- **BOARD-006** When the reloaded board shows the session open in the terminal modal as archived (archived from outside, e.g. by its own board tools), the store closes the modal (store behaviour; the board just re-renders).

### Scene derivation (`buildNodes`)

- **BOARD-007** The scene is, in order: the archive node, the note-pad node, one section node per section (oldest first), one card node per card in board order, one view node per view, one node per free note, and the pad's top sheet.
- **BOARD-008** The archive node sits at the archive's stored corner with the size of the stack zone (folded stack, or the open panel); while the archive is being dragged (or its drop is pending) it keeps its live position; it is not selectable, focusable or deletable; its drag handle is the archive header; it carries `is-open` while open and `nowheel` while results overflow; its data is the archived-card count.
- **BOARD-009** An active card's node sits at its stored `x, y` with `cardSize(card)` (288×216, or its expanded `w×h`, default 720×480); a card being resized keeps the size under the pointer.
- **BOARD-010** Every archived card that is not a search result is placed at the stack top (archive x + 28, y + 52), regardless of its stored position; result cards are placed at their result slots; a dragged/resizing/busy card keeps its live position.
- **BOARD-011** Pile order is most recent first by `max(archivedAt ?? 0, meta.updatedAt ?? createdAt)`; depth = index in the pile (results excluded).
- **BOARD-012** Pile cards at depth ≥ 3 are hidden; one that was visible on the previous build instead starts a 700ms "sink" flight and stays drawn until it ends, then hides.
- **BOARD-013** A card that was hidden on the previous build, is now drawn, is not cut off by the result band and was not a result before, starts a 700ms "rise" flight from the stack top (offset = stack top − its position) and gets the `stack-enter` class.
- **BOARD-014** Result cards whose top or bottom is a full card height (216) or more outside the results band are hidden; ones crossing a band edge are clipped at that edge with `inset(max(cutTop,−24)px −24px max(cutBottom,−24)px)` (24px of shadow bleed left/right and on uncut edges). Band = archive y + 52 … + 464.
- **BOARD-015** Card node classes: `stack-card` for archived cards; `is-result` for results, else `stack-d<min(depth,3)>` for pile cards; `nowheel` on results while the results overflow; `stack-enter` during a rise. Style vars on results/rising cards: `--stack-i = min(slot, 4)`, `--fly-x/--fly-y` = rise offset px, plus the clip.
- **BOARD-016** Card z-index: active compact cards none (0); expanded cards 1; result cards base + 10; pile cards base + max(0, 3 − depth), where base = 1500 while the archive is raised, else 0.
- **BOARD-017** Archived cards are draggable only if they are a result or the pile's top card (depth 0), and selectable only if they are a result; a card that is not selectable is forced unselected.
- **BOARD-018** An archived card's sticker notes are not passed to it (hidden until restored); an active card gets the notes stuck to it (notes whose `sessionId` is a card on the board).
- **BOARD-019** A card node is reused unchanged when its card data (deep-equal), match indices, stickers, position, size, hidden flag, classes, style, z, draggable, selectable and selected are all unchanged (render-skip optimisation; not observable).
- **BOARD-020** Section nodes: one per section, framing its active members (see Sections), z −2, not selectable/focusable/deletable, drag handle `.section-header`; a section being dragged keeps its live position; data = section, member count and aggregated status.
- **BOARD-021** The pad node sits under the folded stack (archive x, archive y + 386, 344×296), z −1, not draggable/selectable/focusable; it keeps its live position while the archive is dragged.
- **BOARD-022** The pad's top sheet is a note node whose id is the board's current "draft id" (a fresh UUID per board mount and after each peel), 216×216 at (pad x + 64, pad y + 52), yellow, empty, z 2, not selectable/focusable/deletable; while dragged or busy it keeps its live position.
- **BOARD-023** Free-note nodes (notes not stuck to a card on the board) sit at their stored position and size, z 2, selectable; view nodes at their stored position and size (default 480×360), z 0, selectable, drag handle `.view-head`.
- **BOARD-024** A 700ms-later re-derivation is scheduled (soonest flight end + 20ms) whenever any flight is pending, so sinking cards hide and rising cards lose `stack-enter` on time.

### Edges

- **BOARD-025** For every active card, one lineage edge is drawn from each of its `parents` that is also an active card on the board (id `<parent>-><child>`); edges involving archived cards are not drawn.
- **BOARD-026** A lineage edge is "animated" (dashes moving, stroke `--status-working`) while the child card's status is `working`; otherwise solid `--border-strong` 1.5px.
- **BOARD-027** For every active card with `fromView` naming a view on the board, an always-animated dashed view edge is drawn from that view to the card (id `<viewId>-><sessionId>`), stroke accent 70%, width 2, dashes 6 on 4 off, round caps, one 10px dash cycle per 1s, marching away from the view.
- **BOARD-028** Edges are not selectable, focusable or hit-testable.
- **BOARD-029** Each edge attaches to the midpoint of the side of each node that faces the other: with centres a, b and the source's size (w, h), if `|dx|/w > |dy|/h` the right (dx > 0) or left side, else the bottom (dy > 0) or top; the same rule from the target's side with the target's size.
- **BOARD-030** The edge path is a cubic Bézier from source anchor S to target anchor T with control points offset along each anchor's side normal by `0.5·d` when the other end lies ahead on that axis (`d` = distance along the normal) or `0.25 × 25 × √(−d)` when it lies behind (React Flow `getBezierPath`, curvature 0.25).
- **BOARD-031** Anchors use the nodes' logical positions and measured sizes: an edge jumps straight to a card's new position while the card itself glides there over 420ms (edges don't animate with node glides).

### Viewport commands

- **BOARD-032** `fitActive(duration = 320)`: if the board has any active cards, free notes (notes without `sessionId`) or views, fit the viewport to those nodes plus every section frame that has members, with padding 0.2, max zoom 1, the given duration; otherwise fit the rect `{x: dock.x, y: dock.y, w: dock.w + 760, h: min(dock.h, 784)}` (dock = archive zone ∪ pad) with padding 0.06 and the given duration.
- **BOARD-033** The zoom pill's fit button calls `fitActive` with the click event as its argument, which is not a number, so that fit is **instant** (no animation); the pane menu's `Zoom to fit` animates over 320ms.
- **BOARD-034** `ensureVisible(p, size = 288×216)`: if the rect is already fully inside the visible flow rect (canvas size / zoom) nothing happens; otherwise the viewport animates (320ms, zoom unchanged) to centre it; on an axis where the rect is longer than the view, it instead puts the rect's start 24px (`24/zoom` flow) from that view edge.
- **BOARD-035** `viewportCentre(size)` = the top-left that centres something of `size` in the visible flow rect; `(0, 0)` when the canvas element is not available.
- **BOARD-036** Jumping to a search result for an active card: closes search, makes that card the only selected card (notes/views keep their selection), animates the viewport (320ms) to centre the card at zoom `max(current, 0.8)`, then flashes the card: two iterations of 800ms (ease cubic-bezier(0.2, 0.8, 0.2, 1)) of a box-shadow going from `0 0 0 2px <accent>, 0 0 0 2px <accent 55%>` to `0 0 0 2px <accent>, 0 0 0 18px <accent 0%>` (accent read from the card's computed `--accent`).
- **BOARD-037** Jumping to a search result for an archived card closes search and opens its transcript preview at the match (`{query: trimmed search query, snippet}`); no viewport change. Unknown ids do nothing (search stays open).

### Pointer: clicks and double-clicks

- **BOARD-038** A click on the pad's top sheet peels a note beside the pad (see Notes) only when it is the first click of a sequence (`detail === 1`); the second click of a double-click does nothing.
- **BOARD-039** A click on a pile card (archived, not a result) opens the stack (clearing any dismissal); nothing else happens.
- **BOARD-040** A click on a card with Shift, ⌘ (Meta) or Ctrl held only changes selection (React Flow rules above); it never opens the card.
- **BOARD-041** A click on the card currently being renamed does nothing.
- **BOARD-042** A click on an expanded card (anywhere that isn't a stopped control) focuses its terminal, like clicking a window, and cancels any pending title-click open.
- **BOARD-043** A click on a compact card's title text waits 240ms and then opens the card; any other click on a compact card opens it at once; every click first cancels a pending title-click open.
- **BOARD-044** Opening a card: archived → transcript preview; expanded → wake its on-board terminal (10/12); otherwise open the terminal modal.
- **BOARD-045** A double-click on a section header starts renaming that section.
- **BOARD-046** A double-click on a free note starts writing in it; on the pad's top sheet it does nothing.
- **BOARD-047** A double-click on a card's title text (active card or result, not a pile card) cancels the pending title-click open and starts renaming the card.
- **BOARD-048** A double-click whose target is the pane itself creates a session centred on the pointer (top-left = pointer − (144, 108)) unless the point is inside the dock (archive zone — folded or open — ∪ pad); double-clicks on nodes, HUD or the selection rectangle never do. Section interiors count as pane, so the session lands (and joins) there.
- **BOARD-049** A click on the empty pane closes the context menu (and deselects everything per Canvas mechanics).

### Pointer: drag and drop

- **BOARD-050** Any drag start closes the context menu and cancels a pending title-click open.
- **BOARD-051** Dragging the archive: marks the archive busy and the board `is-moving-archive` (card, note and pad glides off); every archived card, the pad and the pad's top sheet are marked busy and move by the same delta as the archive each frame.
- **BOARD-052** Dropping the archive saves the archive rect `{x: round(x), y: round(y), w, h}` (stored w/h unchanged) to the store and main; archived cards keep their stored positions (they are drawn at the new stack top); all busy marks are cleared.
- **BOARD-053** Dragging a section by its header marks it and its members busy, sets `is-moving-section` (card glides off), and moves every member card by the same delta each frame.
- **BOARD-054** Dropping a section: delta = round(node position − frame corner); member cards are saved at `x + dx, y + dy`; every ungrouped active card whose centre now lies inside the moved frame joins the section; the section's corner is saved at the moved frame's corner; busy marks are cleared.
- **BOARD-055** Dragging cards, notes or views records each dragged node's start position and marks it busy.
- **BOARD-056** While dragging, the archive is "hot" when any dragged non-terminal card's centre lies inside the current stack zone (folded or open).
- **BOARD-057** While dragging, the "hot" section is the section the lead card would join: lead = the grabbed node if it is a card, else the first dragged card; `sectionAt(lead position, sections, lead's current section, lead's size)`; none when the archive is hot or no card is dragged (notes never pick a section).
- **BOARD-058** On any drop, the archive and section hot states clear.
- **BOARD-059** Dropping the pad's top sheet with its centre on the pad or on the stack zone sends it back to its sheet spot (gliding 420ms); anywhere else it becomes a note at the drop position (rounded), written immediately (rename mode), and a fresh top sheet appears.
- **BOARD-060** A terminal card dropped with its centre in the stack zone returns to its drag-start position (terminals are never archived).
- **BOARD-061** A note (size `noteSize`) or view (size `viewSize`) dropped with its centre in the stack zone or on the pad returns to its drag-start position.
- **BOARD-062** Every other dropped card is saved at its rounded drop position with `archived` = (non-terminal and centre in the stack zone) and `sectionId` = null if archived, else the section under the lead card (computed as for the hot section, with the lead's current section preferred) — so all cards dropped together join the lead's section, and dropping outside any frame removes them from their sections.
- **BOARD-063** Dropping an archived result card outside the stack zone restores it (`archived: false`) where it was dropped (drag out of the archive).
- **BOARD-064** If the drop would archive any card that was active, live and not `done`, the board first asks with the archive-running confirm; on Cancel every dragged card/note/view goes back to its drag-start position and nothing is saved; while the dialog is up the nodes stay where they were dropped.
- **BOARD-065** After a confirmed (or confirm-free) drop: cards are patched in one `patchCards` call, each moved note not bounced is saved at its rounded position, each moved view not bounced is patched with its rounded position, and busy marks are released.
- **BOARD-066** Archiving by drop stores the drop position (not the stack top) in the card; the card then glides into the stack (420ms) because archived cards are drawn at the stack top.

### Archive stack orchestration

- **BOARD-067** The stack is open only while the open flag is set and there is at least one archived card.
- **BOARD-068** Results: with an empty (trimmed) query, every archived card in pile order; otherwise cards whose title (`cardTitle`) fuzzy-matches the query (`fuzzy.ts`), sorted by score desc then pile rank, keeping only scores ≥ 50% of the best score; each result keeps its matched character indices.
- **BOARD-069** The stack zone is the folded stack (344×362) when closed, or the open panel sized for the result count (see layout.ts); the dock (what new things keep clear of, and where the pane menu and double-click are disabled) is the bounding box of the stack zone and the pad.
- **BOARD-070** "Raised" is true while open and for 700ms after closing (the timer restarts if it reopens), so cards fly back in above the board.
- **BOARD-071** `openStack` clears the dismissal and sets open; `closeStack` clears open, the query and the scroll, and sets dismissed.
- **BOARD-072** Setting the stack query resets the scroll to 0 and opens the stack when the trimmed query is non-empty.
- **BOARD-073** Submitting the stack search (Enter) blurs the focused element and opens the first result's transcript preview; with no results it does nothing.
- **BOARD-074** Pointer movement over the canvas with no button pressed sets stack hover = pointer inside the stack zone; moving outside also clears the dismissal; leaving the canvas clears both hover and dismissal.
- **BOARD-075** The canvas gets `stack-lift` while the stack is closed and either (hovered and not dismissed) or the archive is hot.
- **BOARD-076** While open, a pointer-down anywhere inside the board canvas (capture phase) that is not on the archive node or an archived card node closes the stack.
- **BOARD-077** While open with overflow (scroll max > 0), a wheel event without ctrlKey whose pointer lies in the stack zone scrolls the results by `deltaY × (16 if line mode else 1) / zoom` flow units, clamped to [0, scroll max]; the canvas gets `is-scrolling-stack` until 160ms after the last such wheel event.
- **BOARD-078** The effective scroll is `min(scroll, scrollMax)` for the current result count.
- **BOARD-079** Archive context exposed to the archive node (10): `open`, `hover` (= hover and not dismissed), `query`, `resultCount`, `scroll`, `scrollMax`, `setQuery`, `openStack`, `closeStack`, `submit`.

### Sections

- **BOARD-080** A section's members are the active cards whose `sectionId` is that section; archived cards never belong to a section.
- **BOARD-081** A section's frame wraps its members: left = min x − 24, top = min y − 44, right = max right + 24, bottom = max bottom + 24 (each member's `cardSize`); a section with no members is a 336×284 frame at its stored corner.
- **BOARD-082** Whenever a section with members has a frame corner different from its stored corner, the stored corner is saved to the frame corner (so an emptied section stays where it last was).
- **BOARD-083** A section's status is `waiting` if any member is waiting, else `working` if any member is working, else none.
- **BOARD-084** The section a card at a position belongs to is the one whose frame holds the card's centre; where frames overlap, the card's current section wins, else the newest (last in the list).
- **BOARD-085** A new section takes the first colour (blue, purple, pink, red, orange, yellow, green, graphite) not used by another section; if all are used, `FOLDER_COLORS[usedCount % 8]` (i.e. blue once all eight are in use).
- **BOARD-086** A new section has a fresh UUID, empty name, rounded corner and `createdAt = now`, is saved immediately, and (unless Claude will name it) starts renaming.
- **BOARD-087** Grouping (⌘G, card menu, selection bar) takes the active cards among the ids (none → nothing happens), computes the other sections' frames as they will be without those cards, and lays the group out (layout.ts › `layoutGroup`) against the other active cards and the zones [dock, other frames].
- **BOARD-088** `layoutGroup`: if the frame around the cards where they are intersects no other active card (margin 0) and no zone (margin 32), the cards stay put; otherwise they are tidied into a grid of `cols = min(4, ceil(√n))` columns, cells sized to the largest card, at a free spot near the original frame corner, in reading order (rows by `round(y / (cellH + 32))`, then x), each position rounded.
- **BOARD-089** A group gets Claude to name it when at least one grouped card is a non-terminal with a title or transcript meta; then the new section does not start renaming, its header shows `Naming…` with a glow, and `board.nameSection(projectId, those ids)` runs.
- **BOARD-090** When naming resolves with a name, it is saved only if the section still exists on this project's board, still has an empty name, and is not being renamed; the landed name ('' if none or failed) then plays the reveal for 1200ms before the naming state is removed. A naming failure logs a warning and settles with no name.
- **BOARD-091** After grouping: cards are patched with the section id (and grid positions when tidied), the selection is cleared, and `ensureVisible(frame corner, 288×216)` runs (the frame's corner with the default card size, not the frame's size).
- **BOARD-092** `New section here` at point p: the preferred corner is (p.x − 24, p.y − 22); obstacles are active cards, the dock and every section frame. Because the empty frame (a rect carrying the click point as x/y) is passed as the search size, every candidate is tested at the click point itself: if a 336×284 rect cornered at p is clear (16px margin) the section goes to (p.x − 24, p.y − 22), otherwise every candidate is blocked and it lands at (p.x − 24, p.y − 22 + 9920). **Bug in 0.9.4 (BoardView.tsx:1057-1068 + geometry.ts:109); flag to the owner before porting.** It starts renaming; no viewport change.
- **BOARD-093** Renaming a section commits the trimmed value on Enter or blur if it differs from the original (empty → untitled); Escape cancels; the input swallows key, pointer, click and double-click events; it is focused with its text selected on open.
- **BOARD-094** `Remove from section` moves the card to a free card-sized spot near (frame right + 32, card y), avoiding the section's own frame (margin 32), clear of other active cards, notes, views, the dock and other frames; saves it with `sectionId: null`; pans to show it (card-sized).
- **BOARD-095** `Ungroup` / `Delete section` removes the section immediately (no confirm); its cards stay where they are, ungrouped.
- **BOARD-096** Changing a section's colour from its menu closes the menu and saves the section with the new colour.
- **BOARD-097** `New session in section` / `New terminal in section` start a session at the first free card slot inside the frame (slots step 320×248 from (frame x + 24, frame y + 44), as many columns/rows as fit, a slot free when no active card intersects it with margin 15); if none, the nearest free card spot to (frame right + 8, frame y + 44) (no area avoided); that point then goes through the new-session search (720×480, clear of cards/notes/views, avoiding the dock), and the session joins the section whatever spot that search picks.
- **BOARD-098** While a dragged card hovers a section, that section's frame shows its hot style.

### Creating sessions

- **BOARD-099** New sessions reserve room for an expanded card (720×480): with a point, the nearest free spot to it (clear of every card, free notes and views with a 16px margin, and of the dock with 32px); without one, the spot right of the rightmost active card (its x + width + 64, its y), or 80 right of the dock's right edge at the dock's top when there are no active cards. Positions are rounded.
- **BOARD-100** A new session (from a point or the default spot) joins the section whose frame holds the centre of a 288×216 card at its final position, unless a section was given explicitly.
- **BOARD-101** Creating a session calls `sessions.create(projectId, {x, y, kind, autoMode: true})`, adds the card, joins the section, pans to show the reserved 720×480 rect, and opens the terminal modal.
- **BOARD-102** A failed create shows the toast `Couldn't start a session: <message>` (or `Couldn't start a terminal: <message>` for a terminal).
- **BOARD-103** When the terminal modal closes or switches away from a session whose card is on the board, active and not expanded, that card is expanded on the board and the board pans to show it at its expanded size.
- **BOARD-104** Running an action (from the actions rail) creates a session with `actionId` at the new-session spot for a 288×216 card; it is not drawn on the board; a failure toasts `Couldn't run <action name>: <message>`.
- **BOARD-105** When a run is taken over (its card stops having `runOf`), the card is moved to the nearest free spot to where it is (clear of other active cards, notes, views; avoiding the dock) if that differs, and saved.
- **BOARD-106** When a run of this project finishes (`ev:run-finished`), its terminal is disposed, the terminal modal closes if it shows that run, and a toast shows `<action name>: <message truncated to 180>` or `<action name> finished` when there is no message.
- **BOARD-107** A notification/open request for this project and a session: the request is consumed; if the card is on the board the board pans to show it (its size) and opens it; else if it is a run on this board, its terminal modal opens; else nothing.
- **BOARD-108** Merging ("New session from context") starts the new session below its sources (average x of the sources, 96 below their lowest bottom; if that 720×480 rect comes within 32 of the dock, 80 right of the dock instead, y clamped to the dock's height), or at the new-session spot when no source is on the board, then the nearest free spot; it is created with `{x, y, name, initialPrompt, parents}` (no auto mode), closes the merge dialog, clears the merge focus and the selection, pans to it and opens its terminal modal. Errors propagate to the dialog (11).
- **BOARD-109** Starting a merge takes the non-terminal cards among the ids (in id order, with their titles); none → nothing; it records the focus query (trimmed; empty → none) and opens the merge dialog.

### Archive, stop, restore, remove

- **BOARD-110** Archiving ids takes the active non-terminal cards among them (none → nothing); if any is live and not `done` it asks first (archive-running confirm) and stops on Cancel; then every target is saved with `archived: true` and its position set to the stack top, and the moved cards are deselected.
- **BOARD-111** The store turns an archive of a session with nothing said in it into removal from the board instead, sets archived cards to `done`, stamps `archivedAt`, drops them from sections and un-expands them (store, 03/10).
- **BOARD-112** Stopping ids takes the live non-terminal non-run cards among them (none → nothing); asks with the stop confirm when any of them is not `done`; then kills each and waits for all.
- **BOARD-113** `Restore to board` saves the card at a free card-sized spot to the right of the dock (dock right + 80, dock top), clear of every other card and of notes/views, with `archived: false`, and pans to show it (card-sized).
- **BOARD-114** `Remove terminal` asks first only when the shell is live (`Remove terminal?`), then removes the card from the board.
- **BOARD-115** `Remove from board` always asks (`Remove from board?`); on confirm a live session is killed first (awaited), then the card is removed; its transcript is untouched.

### Notes orchestration

- **BOARD-116** A new note is saved immediately as `{id, text: '', color: 'yellow', x, y (rounded), createdAt: now}` and opens for writing (rename state).
- **BOARD-117** Peeling a note uses the current top sheet's id for the new note and gives the pad a fresh top-sheet id.
- **BOARD-118** Clicking the pad's top sheet (or the pad/top-sheet menu's `New note`) peels a note to the nearest free 216×216 spot to (pad right + 32, pad y + 52) — clear of every card, notes and views, avoiding the dock — and pans to show it.
- **BOARD-119** `New note here` at p places a note at the nearest free spot to (p.x − 108, p.y − 108) and pans to show it.
- **BOARD-120** Recolouring from the note menu closes the menu and saves every target note with the new colour.
- **BOARD-121** Deleting notes removes them immediately with no confirmation, written on or not (the docs claim otherwise; the code wins).
- **BOARD-122** Starting a session from a note requires that the note exists, has no session, has non-blank text and is not already starting (a second start while one is pending is ignored).
- **BOARD-123** A note's session reserves 720×480 at the nearest free spot to the note's position (clear of every card, the other notes and views; avoiding the dock), joins the section there, is created with `{x, y, initialPrompt: note text, autoMode: true}`, and the note — as it is now, in case it moved or changed colour meanwhile — is saved with that `sessionId`; the board pans to show a 288×216 rect there. Failure toasts `Couldn't start a session: <message>`.
- **BOARD-124** Editing a note's text saves it through the store when this board is still the open board, else straight to main (`board.saveNote`) so typing survives the board closing (⌘W) mid-note.

### Views orchestration (04 owns view semantics)

- **BOARD-125** Creating a view from a description places a 480×360 view centred on the given point, or in the middle of the visible area, at the nearest free spot (clear of every card, notes and views, avoiding the dock), creates it, adds it to the board and pans to show it; errors propagate to the caller.
- **BOARD-126** Opening a view's session asks main for the view's brain and opens its terminal modal; failure toasts `Couldn't open its session: <message>`.
- **BOARD-127** Deleting views asks first (`Delete view?` / `Delete N views?`), then removes them one by one.
- **BOARD-128** The view menu (from right-click or the view's own menu button at a screen point) matches § Context menus › View; `Refresh` is enabled when the view has a brain refresh schedule or any command whose approved line equals its current line; a failed refresh toasts `Couldn't refresh it: <message>`.
- **BOARD-129** The board registers relay handlers for view pages: `openSession(id)` opens the card if it is on the board (else ignored); `startSession(viewId, prompt, title)` follows 04 (VIEW-056…058), including its toast and error strings.
- **BOARD-130** 300ms after the board's cards, notes or views last changed the board broadcasts `board` to every view page; it broadcasts `git` with the git status whenever that changes (and on mount).

### Prompt bar wiring (11 owns the bar)

- **BOARD-131** The bar's "into" target is the single selected card when exactly one card is selected, no selected note has text, no view is selected, and that card is active and not a terminal; otherwise none.
- **BOARD-132** The bar's context count = selected non-terminal cards + selected notes with non-blank text + selected views.
- **BOARD-133** A `session` submission builds context from the selected non-terminal cards (title, git branch, last reply, transcript path), selected notes with text, and selected views (title, description); the session reserves 720×480 below the selected sessions (as for merge) or centred in view; it is created with the context-wrapped prompt, `parents` = those sessions (or none), `name` = the prompt's first line truncated to 64 when there is any context, `expanded: true`, `autoMode: true`; it joins the section there; the prompt is stuck to it as a yellow note; the selection is cleared when there was context; the board pans to it.
- **BOARD-134** If that session can't start, the prompt is saved as a free yellow note at the reserved position, the board pans to it (note-sized) and toasts `Couldn't start a session, so your prompt is on a note: <message>`.
- **BOARD-135** An `into` submission: if the card is gone or archived toast `That session is no longer on the board.` (false); if it is waiting toast `“<title>” is waiting for your answer. Answer it, then send this.` and open it (false); else send the text; on failure toast `Couldn't send it: <message>` (false); on success pan to the card (true).
- **BOARD-136** A `view` submission creates a view from the text in the middle of the view (true); failure toasts `Couldn't create the view: <message>` (false).
- **BOARD-137** A `note` submission saves a note with the submitted colour at the nearest free 216×216 spot to the middle of the view and pans to it (true).
- **BOARD-138** Toasts and the selection bar render stacked above the prompt bar (its `above` slot), toast first.

### Overlays, toasts and dialogs

- **BOARD-139** One toast shows at a time; a new message replaces the current one; each message clears itself after 4200ms unless a newer message replaced it.
- **BOARD-140** Confirms are one at a time, shown in the confirm dialog; the board awaits the answer (true = confirm).
- **BOARD-141** The confirm dialog focuses its confirm button when it opens, so Enter (or Space) confirms; Escape, a pointer-down on the backdrop or `Cancel` resolve false.
- **BOARD-142** "Overlay open" (which blocks board shortcuts and menu commands) = a context menu, confirm, merge dialog, actions-rail dialog/menu, New view dialog, or ⌘K search is up.
- **BOARD-143** The selection bar shows while at least one card is selected and the merge dialog is closed; counts/disabled rules and actions per § UI details › Selection bar wiring.
- **BOARD-144** The New view dialog (04) opens from the pane menu with a point (the view is centred there) or from the title-bar split button with none (centred in view); it closes via its own close.
- **BOARD-145** ⌘K search (11) receives the non-terminal cards (active and archived) and a query that persists between openings; `onStart(ids)` closes search and starts a merge with the query as focus.
- **BOARD-146** The empty-board hint shows while the board has no active cards (terminals count as active).
- **BOARD-147** The title bar shows the board's split button (see UI details); its main part creates a session at the default spot.

### Board actions exposed to nodes (10/04 call these)

- **BOARD-148** Only one thing is renamed or written in at a time: a single "renaming" id is shared by cards, sections, notes and views; starting one ends any other.
- **BOARD-149** `rename(sessionId, title)` saves the card's custom title (empty clears the override) via `patchCards`.
- **BOARD-150** `expand(ids, expanded)` affects only active cards among the ids whose expanded state differs (none → nothing); expanding wakes each card's terminal; the cards are patched in one call; when exactly one card is expanded the board pans to show it at its expanded size. Used by E, the card menus and the card's own button.
- **BOARD-151** `renameView(viewId, title)` patches the view's title; `openSession(id)` opens a card as a click would; `openViewSession`, `startNote`, `archive`, `stop`, `editNote`, `viewMenu(viewId, x, y)` run the board commands described in this spec.

### Selection commands

- **BOARD-152** Select all active: every active card selected, every archived card deselected; notes and views keep their selection.
- **BOARD-153** Clear selection: every node deselected and the store selection emptied.
- **BOARD-154** `Select cards` (section menu) selects exactly that section's members among cards (notes/views unchanged).
- **BOARD-155** The ink layer clears the board's selection when it selects a stroke, and shows its messages through the board toast (11).

### Keyboard

- **BOARD-156** The board listens to `keydown` on the window for its whole lifetime (one listener; current handlers read through refs).
- **BOARD-157** ⌘K (Ctrl+K off Mac; no Shift/Alt/other modifier): ignored when already handled (`defaultPrevented`) or the target is inside a terminal (`.xterm`), or when the terminal modal, agents viewer, diff panel or a board overlay is open; otherwise prevents default, closes the transcript preview if open, and opens search.
- **BOARD-158** Every other board key is ignored while the terminal modal, transcript preview, agents viewer, diff panel or a board overlay is open, or when the target is inside an input, textarea, select or `[contenteditable="true"]`.
- **BOARD-159** ⌘Z / ⇧⌘Z (mod, no Alt, no other modifier) ask the ink layer to undo / redo, and prevent default only when it did something; nothing else on the board has undo.
- **BOARD-160** Escape: first lets the ink layer put its tool down or drop its selected stroke (and stops there if it did); otherwise closes the stack if open, and clears the selection if any card, note or view is selected. Default not prevented.
- **BOARD-161** ⌘A (mod+A, no Shift/Alt): prevents default and selects all active cards.
- **BOARD-162** ⌘G (mod+G, no Shift/Alt): with at least one card selected, prevents default and groups the selected cards; otherwise nothing.
- **BOARD-163** ⌫ or Delete (no mod, no Alt): if the ink layer deleted its selected stroke, prevent default and stop; else if any card, note or view is selected, prevent default, delete the selected notes, archive the selected cards (asks if running; terminals and archived cards are skipped) and delete the selected views (asks).
- **BOARD-164** Enter (no mod, no Shift) with exactly one card selected opens it; with no card and exactly one note selected starts writing in that note; both prevent default.
- **BOARD-165** N (no ⌘/Ctrl/Alt; Shift allowed) prevents default and creates a session at the default spot; T likewise creates a terminal.
- **BOARD-166** V (no ⌘/Ctrl/Alt, not auto-repeat) prevents default and toggles voice in the prompt bar.
- **BOARD-167** `/` (no ⌘/Ctrl/Alt) prevents default and focuses the prompt bar with `/` typed (its slash menu).
- **BOARD-168** E (no ⌘/Ctrl/Alt): when any active card is selected, prevents default and expands all selected active cards if any of them is collapsed, else collapses them all; expanding wakes each card's terminal and, for exactly one card, pans to show it expanded.
- **BOARD-169** P, A, X (no ⌘/Ctrl/Alt) prevent default and pick the ink tool pen, arrow, eraser.
- **BOARD-170** Letter keys are compared by `event.key` lower-cased (not by key code), so on non-Latin layouts N/T/V/E/P/A/X/⌘A/⌘G/⌘K/⌘Z do not fire.
- **BOARD-171** App-menu commands (`ev:menu`) are ignored while a board overlay, the diff panel or the agents viewer is open — but not while the terminal modal or preview is open; `new-session` / `new-terminal` create at the default spot; `search` closes the terminal modal and preview and opens search.

### Context menus

- **BOARD-172** Every menu opens at the pointer's client position (clamped on screen by ContextMenu, 08) and replaces any open menu; choosing an item closes the menu, then runs it; a disabled item does nothing.
- **BOARD-173** Right-clicking the pane (including inside a section frame) shows the pane menu (§ Context menus › Pane), with `inArchive` = point inside the dock and `inSection` = point inside any section frame (empty ones included).
- **BOARD-174** Right-clicking the pad or its top sheet shows the pad menu; a free note the note menu; a view the view menu; a section header the section menu; a terminal card the terminal menu; any other card the card menu; the archive frame, edges and the selection rectangle show nothing.
- **BOARD-175** Right-click never changes the selection; a card or note menu acts on the whole selection when the clicked item is part of a multi-selection (for the card menu, the selection minus terminals).
- **BOARD-176** Pane-menu `New session here` / `New terminal here` create a session centred on the clicked point (top-left = p − (144, 108)).
- **BOARD-177** Pane-menu `Show everything` fits every drawn node (archive, pad, sections, all visible cards, notes, views, top sheet) with padding 0.12 over 320ms, allowing zoom up to 2.

## UI details

### Resolved tokens used on the canvas

`color-mix(in srgb, C p%, transparent)` = C with alpha × p. `color-mix(in srgb, A p%, B)` = premultiplied mix: α = p·αA + (1−p)·αB; rgb = (p·αA·A + (1−p)·αB·B)/α. The port should implement this once and evaluate token expressions with it.

| Token / expression | Light | Dark |
| --- | --- | --- |
| `--bg-canvas` | `#f3f3f1` | `#19191b` |
| `--canvas-dot` | `rgba(0,0,0,0.13)` | `rgba(255,255,255,0.11)` |
| `--bg-elevated` | `#ffffff` | `#2a2a2d` |
| `--bg-window` | `#f5f5f4` | `#1e1e20` |
| `--bg-hover` | `rgba(0,0,0,0.05)` | `rgba(255,255,255,0.07)` |
| `--bg-pressed` | `rgba(0,0,0,0.09)` | `rgba(255,255,255,0.12)` |
| `--text-primary` | `rgba(0,0,0,0.88)` | `rgba(255,255,255,0.9)` |
| `--text-secondary` | `rgba(0,0,0,0.56)` | `rgba(255,255,255,0.58)` |
| `--text-tertiary` | `rgba(0,0,0,0.36)` | `rgba(255,255,255,0.36)` |
| `--border` | `rgba(0,0,0,0.1)` | `rgba(255,255,255,0.1)` |
| `--border-strong` | `rgba(0,0,0,0.18)` | `rgba(255,255,255,0.18)` |
| `--separator` | `rgba(0,0,0,0.07)` | `rgba(255,255,255,0.07)` |
| `--accent` | `#007aff` | `#0a84ff` |
| `--accent-hover` | `#0068d9` | `#3396ff` |
| `--focus-ring` | `rgba(0,122,255,0.5)` | `rgba(10,132,255,0.55)` |
| `--status-working` | `#ff9500` | `#ff9f0a` |
| `--status-waiting` | `#ff3b30` | `#ff453a` |
| `--status-done` | `#007aff` | `#0a84ff` |
| `--archive-bg` | `rgba(120,120,128,0.07)` | `rgba(160,160,170,0.06)` |
| `--archive-border` | `rgba(120,120,128,0.35)` | `rgba(160,160,170,0.3)` |
| marquee / selection rect fill (accent 8%) | `rgba(0,122,255,0.08)` | `rgba(10,132,255,0.08)` |
| marquee / selection rect border (accent 55%) | `rgba(0,122,255,0.55)` | `rgba(10,132,255,0.55)` |
| view edge (accent 70%) | `rgba(0,122,255,0.7)` | `rgba(10,132,255,0.7)` |
| minimap mask (bg-canvas 70%) | `rgba(243,243,241,0.7)` | `rgba(25,25,27,0.7)` |
| minimap mask stroke (accent 50%) | `rgba(0,122,255,0.5)` | `rgba(10,132,255,0.5)` |
| zoom pill / selection bar bg (bg-elevated 86%) | `rgba(255,255,255,0.86)` | `rgba(42,42,45,0.86)` |
| hint bg (bg-elevated 80%) | `rgba(255,255,255,0.8)` | `rgba(42,42,45,0.8)` |
| toast bg (bg-elevated 92%) | `rgba(255,255,255,0.92)` | `rgba(42,42,45,0.92)` |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.06), 0 0 0 0.5px rgba(0,0,0,0.08)` | `0 1px 2px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.08)` |
| `--shadow-md` | `0 4px 14px rgba(0,0,0,0.08), 0 0 0 0.5px rgba(0,0,0,0.1)` | `0 4px 14px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(255,255,255,0.1)` |
| `--shadow-lg` | `0 18px 50px rgba(0,0,0,0.18), 0 0 0 0.5px rgba(0,0,0,0.12)` | `0 18px 50px rgba(0,0,0,0.5), 0 0 0 0.5px rgba(255,255,255,0.12)` |
| `--shadow-modal` | `0 30px 90px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(0,0,0,0.2)` | `0 30px 90px rgba(0,0,0,0.6), 0 0 0 0.5px rgba(255,255,255,0.14)` |

Easings: `--ease-out` = cubic-bezier(0.2, 0.8, 0.2, 1); `--ease-spring` = cubic-bezier(0.34, 1.4, 0.64, 1); `--ease-stack` = piecewise-linear through 29 evenly spaced points `0, 0.05, 0.1697, 0.3218, 0.4803, 0.6281, 0.7553, 0.8579, 0.9355, 0.9902, 1.0254, 1.0452, 1.0534, 1.0536, 1.0487, 1.0411, 1.0326, 1.0242, 1.0166, 1.0104, 1.0055, 1.0019, 0.9995, 0.998, 0.9973, 0.9971, 0.9972, 0.9975, 1`. `--dur-fast` 120ms, `--dur-med` 220ms. Type: `--font-ui` (SF Pro Text / Segoe UI Variable Text), 11/12/13/15px scale; body 13px, line-height 1.4.

### Canvas surface

- Board area: absolutely fills the main area below the title bar; background `--bg-canvas`; dots per Canvas mechanics.
- Loading surface: same background; spinner `Loader2` 20px, `--text-tertiary`, rotate 360° per 0.9s linear, centred.

### Node motion (canvas-level transitions)

| Node | Property | Duration / easing | Off when |
| --- | --- | --- | --- |
| Card (any) | position | 420ms `--ease-out` | the card is being dragged; the archive is being dragged; a section is being dragged |
| Result card | position | 620ms `--ease-stack`, delay `--stack-i × 30ms` | being dragged; archive dragged; a section dragged; results wheel-scrolling |
| Section frame | position, width, height | 420ms `--ease-out` | the section is being dragged |
| Archive frame | width, height | 460ms `--ease-out` (position never animates) | — |
| Free note / top sheet | position | 420ms `--ease-out` | being dragged; archive dragged |
| Note pad | — (no transition) | — | — |
| Views | (04/10) | | |

Animations start from the node's current drawn position, so an interrupted glide continues from where it is. Card internals (stack depth transforms, rise keyframe, hover lift, `stack-lift`) are spec 10.

### Sections

Section hue = `FOLDER_HUES[color].backBottom`: blue `#1d8be0`, purple `#8b5bd9`, pink `#e0508f`, red `#e0443e`, orange `#ee8420`, yellow `#e6b020`, green `#3db352`, graphite `#76767b` (same in both themes).

| Part | Spec |
| --- | --- |
| Frame | radius 22px; border 1.5px solid hue at 30% alpha; fill hue at 6% alpha; transitions background and border-color 220ms ease. |
| Frame, hot (a dragged card would join) | border hue (100%); fill hue at 12% alpha. |
| Header | full width, 44px tall, padding 0 18px, top radius 22px; takes the pointer; cursor `grab` (`grabbing` while dragging); tooltip `Drag to move the section and its cards`. Text colour `color-mix(in srgb, hue 68%, --text-primary)`: light ≈ hue × 0.7071 at α 0.9616 (e.g. blue `rgba(21,98,158,0.96)`, graphite `rgba(83,83,87,0.96)`); dark ≈ hue × 0.7025 + 75.9 at α 0.968 (blue `rgba(96,174,233,0.97)`, graphite `rgba(159,159,162,0.97)`). |
| Label | flex row, gap 8px, 13px weight 600, letter-spacing 0.01em, no wrap; scaled by `s = min(2.2, max(1, 1/zoom))` from its left-centre so it stays legible when zoomed out; max width `100/s %` of the header. |
| Name | ellipsis; `Untitled section` when empty, weight 500 at 70% opacity. |
| Count pill | the member count; padding 0 7px, radius 999px, fill hue at 14% alpha, 11px, line-height 18px, tabular numbers. |
| Status dot | 8×8 dot after the count, `data-status` working/waiting (dot styling: 10 › Status dot); tooltip `A session in this section is working` / `A session in this section needs you`. |
| Empty placeholder | when count is 0: at (24, 44) inside the frame, 288×216, radius 14px, border 1.5px dashed hue at 35%, text `Drag cards here` 12px centred, colour `color-mix(in srgb, hue 55%, --text-tertiary)`; not interactive. |
| Rename input | height 24px, margin −2px 0, padding 0 6px, radius 6px, background `--bg-window`, ring `0 0 0 2px hue`, text `--text-primary`, font inherited, no outline; width = `max(14, length + 1)` characters; placeholder `Section name`; no spellcheck. |
| Naming state | label shows a `Sparkles` icon (13px, stroke 2, `#bf5af2`, twinkle: 1.6s ease-in-out infinite, at 50% opacity 0.55, scale 0.82, rotate 12°) and `Naming…` (weight 500, gap 6px) with a light sweep: text filled with a 90° gradient `currentColor 45%α @0% → currentColor @40% → mix(#bf5af2 80%, currentColor) @50% → currentColor @60% → currentColor 45%α @100%`, background size 250%×100%, position animating 100% → 0% over 1.8s ease-in-out infinite. Tooltip `Claude is naming this section`. |
| Naming glow | a ring 1.5px outside the frame edge (inset −1.5px, radius 22px) filled with a conic gradient rotating once per 3.2s linear (`#0a84ff, #5e5ce6, #bf5af2, #ff375f, #ff9f0a, #bf5af2, #0a84ff`, masked to a 1.5px border), plus a halo copy (3px ring, blur 9px, opacity 0.55); fades in over 600ms `--ease-out`; when a name lands (or naming gives none) it fades out over 700ms ease; removed 1200ms after landing. Not interactive. |
| Name reveal | the landed name's letters each fade in over 460ms `--ease-out` from opacity 0, blur 5px, translateY 3px; letter i starts at `round(i × min(28, 480/max(1, n − 1)))` ms. Accessible name = the whole name. |
| Reduced motion | glow, sparkle and sweep animations off (`prefers-reduced-motion: reduce`). |

### Edges

As in Behaviors › Edges. Lineage stroke `--border-strong` 1.5; animated: stroke `--status-working`, dash 5 on 5 off, offset 10 → 0 every 0.5s linear (forward along source → target). View edge: accent 70%, width 2, dash 6/4, round caps, 1s per cycle. No arrowheads. (Visual duplicate of 10 › Edges.)

### Marquee and selection rectangle

Fill accent 8%, border `1px solid` accent 55%, square corners. Selection rectangle cursor `grab`.

### Zoom pill (`BoardControls`)

- Position: 16px from the canvas's left and bottom; z 5; `role="group"`, label `Zoom`.
- Pill: flex row, centred, height 34px, padding 0 3px, radius 11px, fill bg-elevated 86%, backdrop blur 20px + saturate 1.6, `--shadow-md`.
- Buttons, in order: `−` (lucide `Minus` 14px stroke 2.2, tooltip `Zoom out`), zoom label (tooltip `Reset to 100%`), `+` (`Plus` 14px stroke 2.2, tooltip `Zoom in`), a 1×16px separator (`--separator`, margin 0 3px), fit (`Maximize` 13px stroke 2.2, tooltip `Zoom to fit`).
- Each button: min 28×28, padding 0 4px, radius 8px, transparent, icon `--text-secondary`, cursor default; hover fill `--bg-hover` + text `--text-primary`; pressed fill `--bg-pressed`.
- Zoom label: `round(zoom × 100)` + `%` (e.g. `78%`), min width 50px, 12px weight 500, tabular; live during gestures.
- Actions: `−` zooms out ×1/1.2 over 160ms; label zooms to 100% over 200ms; `+` zooms in ×1.2 over 160ms (about the canvas centre); fit = `fitActive` (instant, see Behaviors).

### Minimap

- Bottom-right of the canvas, 16px margin, 176×118px, radius 12px, clipped, `--shadow-md`, z 5; fill `--bg-elevated`.
- Shows every non-hidden node as a rect (radius 6 flow units) over the union of all nodes' bounds and the current view rect, scaled to fit (`viewScale = max(boundsW/176, boundsH/118)`, centred, plus a margin of `5 × viewScale` flow units each side).
- Node fills: archive and pad `--archive-bg`; section hue + alpha `0x24` (≈ 0.14); note its colour's `frontTop` (blue `#6dd0fc`, purple `#c8a2f7`, pink `#fba0c8`, red `#ff9189`, orange `#ffc374`, yellow `#ffe27a`, green `#93e19c`, graphite `#bdbdc2`); view `#c8a2f7`; terminal card `--text-tertiary`; Claude card its status colour (working/waiting/done). Strokes: archive and pad `--archive-border`; section its hue; others transparent. Stroke width 3 **flow units** (so usually a sub-pixel hairline; Chromium draws with `crispEdges`, unclear: exact on-screen weight — match screenshots).
- Mask: everything outside the current view rect is covered with bg-canvas 70%; its edge is stroked accent 50% at 2 flow units (sub-pixel at typical scales).
- Pannable: dragging anywhere in the minimap moves the view by `pointerDelta × viewScale × max(zoom, ln zoom)` screen px per move (relative, never jumps to the click point). Zoomable: wheel over it zooms about the canvas centre by `2^(−deltaY × m × f)` (same m, f as pinch). Clicks do nothing else.

### Empty-board hint

- Shown while there are no active cards; not interactive, `aria-hidden`.
- 22px from the top, centred, z 4, max width 440px, padding 12px 18px, radius 14px, fill bg-elevated 80%, backdrop blur 20px saturate 1.6, `--shadow-sm`, centred text; enters with `board-rise` (220ms `--ease-out` from opacity 0 and +6px).
- Title `No active sessions` (13px weight 600, `--text-primary`, margin-bottom 2px).
- Body (12px `--text-secondary`): `Type in the bar below, double-click anywhere or press ` [N] ` to start a Claude session, ` [V] ` to speak one, or ` [T] ` for a terminal. Drag cards out of the archive to reactivate them.` — keys are literal letters on every platform.
- Key caps: inline-block, min width 18px, padding 0 5px, margin 0 1px, radius 4px, border 0.5px `--border-strong`, fill `--bg-elevated`, shadow `0 1px 0 --border`, 11px UI font, line-height 17px, centred, `--text-secondary`.

### Toast

Rendered in the prompt bar's `above` slot (static position, not absolute): padding 9px 14px, radius 10px, fill bg-elevated 92%, backdrop blur 20px, `--shadow-lg`, 12px `--text-primary`, max width `min(560px, 100% − 48px)` of the bar column, `role="status"`; enters with the bar's rise (220ms `--ease-out` from opacity 0, +6px).

Toast strings (verbatim; `<m>` = the error's message, or `String(err)` for non-Errors; quotes are typographic where shown):
- `Couldn't start a session: <m>` · `Couldn't start a terminal: <m>`
- `Couldn't run <action name>: <m>`
- `<action name>: <message ≤180>` · `<action name> finished`
- `Couldn't open its session: <m>`
- `Couldn't refresh it: <m>`
- `“<view title>” tried to start a session. Ask its brain if you want it to be able to.`
- `Couldn't start a session, so your prompt is on a note: <m>`
- `That session is no longer on the board.`
- `“<card title>” is waiting for your answer. Answer it, then send this.`
- `Couldn't send it: <m>`
- `Couldn't create the view: <m>`

### Title-bar actions (component chrome: 08)

Split button in the title bar's action slot: main part `New Session` with a `Plus` icon (15px, stroke 2.2), tooltip `New session (⌘N)` (Windows `New session (Ctrl+N)`); its menu, labelled `More ways to start`, has `New terminal` (`SquareTerminal`, hint `⌘T` / `Ctrl+T`) and `New view…` (`LayoutDashboard`, no hint).

### Selection bar wiring (content and look: 11)

Props from the board: `count` = selected cards (store selection: active, results and terminals); `sessionCount` = non-terminal among them; `activeCount` = non-archived non-terminal; `groupCount` = non-archived. Actions: `New session from context` → start merge with all selected ids (merge drops terminals); `Group` → group all selected ids (archived dropped); `Archive` → archive all selected ids; clear (×) → clear selection. Disabled: merge when `sessionCount` = 0; Group when `groupCount` = 0; Archive when `activeCount` = 0. Tooltips `Group into a section (⌘G)`, `Archive selected (⌫)` (Windows `Ctrl+G`, `Del`), `Clear selection (Esc)`; count text `<n> selected`. Placed as the second item of the prompt bar's `above` stack (static position, rise animation) — its own absolute `bottom: 20px` is overridden there.

### Confirm dialog

- Modal: backdrop fixed full window, z 500, `rgba(0,0,0,0.28)` light / `rgba(0,0,0,0.45)` dark, backdrop blur 3px, fades in 220ms `--ease-out`; panel centred, radius 18px, `--bg-elevated`, `--shadow-modal`, rises in 220ms `--ease-out` from opacity 0, +8px, scale 0.985; max size window − 48px; `role="dialog"`, `aria-modal`, labelled by its title.
- Panel content: width 380px, padding 22px 22px 18px. Title 15px weight 600, margin-bottom 6px. Message `--text-secondary`, line-height 1.45, margin-bottom 20px. Actions right-aligned, gap 8px: `Cancel` (`.btn`) then the confirm button (`.btn-danger` when danger, else `.btn-primary`), auto-focused.
- `.btn`: height 28px, padding 0 12px, radius 6px, border 0.5px `--border-strong`, fill `--bg-elevated`, `--shadow-sm`, 13px weight 500, gap 6px; hover `--bg-hover`; pressed `--bg-pressed`. `.btn-danger`: fill `--status-waiting`, transparent border, text `#fff`; hover `color-mix(in srgb, --status-waiting 88%, #000)` (light ≈ `#e0342a`, dark ≈ `#e03d33`). `.btn-primary`: fill `--accent`, text `--text-on-accent`, hover `--accent-hover`. Focus ring: `:focus-visible` outline 2px `--focus-ring`, offset 2px (unclear whether Chromium shows it for this programmatic focus after a mouse action — ConfirmDialog.tsx:17).

Confirm requests (verbatim; all `danger: true`):

| Trigger | Title | Message | Confirm |
| --- | --- | --- | --- |
| Archive running (1) | `Archive a running session?` | `“<title>” is still working. The Claude process will be stopped.` / when waiting: `“<title>” is still waiting for you. The Claude process will be stopped.` | `Stop & Archive` |
| Archive running (n > 1) | `Archive <n> running sessions?` | `Some of these sessions are still working. Their Claude processes will be stopped.` | `Stop & Archive` |
| Stop (1 card) | `Stop a running session?` | `“<title>” is still working. Stopping it cuts off this turn. The transcript stays, so you can resume it.` (`is still waiting for you.` in place of `is still working.` when waiting; same in the next row) | `Stop` |
| Stop (n cards, one busy) | `Stop <n> sessions?` | `“<title>” is still working. Stopping it cuts off this turn. The transcripts stay, so you can resume them.` | `Stop` |
| Stop (n cards, several busy) | `Stop <n> sessions?` | `Some of these sessions are still working. Stopping them cuts off their turns. The transcripts stay, so you can resume them.` | `Stop` |
| Remove terminal (live) | `Remove terminal?` | `“<title>” will be removed from this board. Its shell and anything running in it will be stopped.` | `Stop & Remove` |
| Remove from board | `Remove from board?` | `“<title>” will be removed from this board and its process stopped. The transcript stays on disk.` (without ` and its process stopped` when not live) | `Remove` |
| Delete view (1) | `Delete view?` | `“<view title>” and its page go, and the Claude session that keeps it up to date stops. Its transcript stays on disk.` | `Delete` |
| Delete views (n) | `Delete <n> views?` | `Their pages go, and the Claude sessions that keep them up to date stop. Their transcripts stay on disk.` | `Delete` |

`<title>` = `cardTitle`: custom title, else transcript title, else `Terminal` (terminals) / `New session`.

## Keyboard & mouse

Guard **G** (applies unless noted): no terminal modal, transcript preview, agents viewer, diff panel or board overlay open, and the focused target is not an input/textarea/select/contenteditable. "mod" = ⌘ on Mac, Ctrl elsewhere.

| Key / gesture | Condition / guard | Effect |
| --- | --- | --- |
| mod+K | no Shift/Alt/other mod; not handled already; target not in a terminal; no terminal modal, agents viewer, diff or overlay (preview allowed) | close preview if open; open ⌘K search |
| mod+Z / mod+Shift+Z | G; no Alt/other mod | ink undo / redo (prevent default only if it acted) |
| Esc | G | ink escape, else close stack (if open) + clear selection (if any) |
| mod+A | G; no Shift/Alt | select all active cards |
| mod+G | G; no Shift/Alt; ≥1 card selected | group selected cards into a new section |
| ⌫ / Delete | G; no mod/Alt | ink delete-stroke, else delete selected notes + archive selected cards + delete selected views (asks) |
| Enter | G; no mod/Shift; exactly 1 card selected | open it |
| Enter | G; no mod/Shift; 0 cards, exactly 1 note selected | write in the note |
| N | G; no ⌘/Ctrl/Alt | new session (default spot) |
| T | G; no ⌘/Ctrl/Alt | new terminal (default spot) |
| V | G; no ⌘/Ctrl/Alt; not repeat | toggle voice prompt |
| / | G; no ⌘/Ctrl/Alt | focus prompt bar with `/` |
| E | G; no ⌘/Ctrl/Alt; ≥1 active card selected | expand all selected, or collapse all if all expanded |
| P / A / X | G; no ⌘/Ctrl/Alt | ink tool pen / arrow / eraser |
| ⌘N, ⌘T (Mac menu accelerators); Ctrl+N, Ctrl+T (Windows, outside terminals, via App → main → `ev:menu`) | no overlay, diff or agents viewer (modal/preview allowed) | new session / new terminal at the default spot |
| Edit › Search Sessions… (menu click; its ⌘K label isn't a registered accelerator) | same as above | close terminal modal + preview, open search |
| Space (held alone) | focus not in an input | left-drag pans; wheel pans |
| Shift or ⌘/Win (held) | — | click toggles selection; drag keeps selection when grabbing non-selectable nodes |
| ⌘ (Mac) / Ctrl (others) held + wheel | — | zoom about pointer |
| Wheel / two-finger scroll | not over `nowheel` | pan (0.5× delta) |
| Pinch | — | zoom about pointer (×10 sensitivity on Mac) |
| Middle-drag | anywhere | pan |
| Left-click empty pane | — | deselect all, close menu |
| Left-drag empty pane (>1px) | no Space | marquee (replaces selection, partial overlap) |
| Drag inside the post-marquee selection rect | — | move all selected |
| Double-click empty pane | point not in dock | new session centred at point |
| Right-click empty pane / section interior | — | pane menu |
| Click compact card | no Shift/⌘/Ctrl | select; open (title: after 240ms) |
| Click expanded card | no Shift/⌘/Ctrl | select; focus its terminal |
| Shift/⌘-click card | — | toggle in selection; no open |
| Ctrl-click card (Windows) | — | select only it (unless already selected); no open |
| Double-click card title | not a pile card | rename |
| Drag card | >1px | move (selected ones together); drop rules per Behaviors |
| Click / drag pile top card | — | open stack / drag it out |
| Drag section header | — | move section + members |
| Double-click section header | — | rename section |
| Drag archive header | — | move archive, pad and archived cards |
| Click pad top sheet | single click | new note beside the pad |
| Drag pad top sheet | — | peel a note where dropped |
| Double-click note | not the top sheet | write in it |
| Right-click node | — | its menu (§ Context menus) |
| Hover stack | no button down | stack lift (10) |
| Wheel over open panel with overflow | no ctrl | scroll results |
| Pointer-down outside archive/archived cards | stack open | close stack |
| Minimap drag / wheel | — | pan / zoom about canvas centre |
| Zoom pill buttons | — | zoom out / 100% / zoom in / fit (instant) |

## Context menus

Items are listed in order; `—` = separator; hints are shown as written on Mac, with Windows equivalents in brackets (`keys()`: ⌘→Ctrl, ⌫→Del, ↩→Enter, joined with `+`). Icons are lucide icons by name.

### Pane (right-click empty canvas or inside a section frame)

p = clicked flow point. `inArchive` = p inside the dock (stack zone ∪ pad); `inSection` = p inside any section frame.

1. `New session here` — Plus — hint `N` — disabled if inArchive — session at p − (144, 108)
2. `New terminal here` — SquareTerminal — hint `T` — disabled if inArchive — terminal at p − (144, 108)
3. `New section here` — SquareDashed — disabled if inArchive or inSection — empty section near p (see Sections bug)
4. `New note here` — StickyNote — disabled if inArchive — note centred near p
5. `New view here…` — LayoutDashboard — disabled if inArchive — New view dialog for a view centred at p
6. —
7. `Select all active` — BoxSelect — hint `⌘A` [`Ctrl+A`]
8. `Zoom to fit` — Maximize — `fitActive` (320ms)
9. `Show everything` — Maximize — fit all drawn nodes, padding 0.12, 320ms

### Card (right-click a Claude session card: active, pile top, or result)

`group` = the selection minus terminals when the card is in a multi-selection, else just this card. `running` = live, non-terminal, non-run cards of `group`.

1. Archived card: `Preview` — Eye — hint `↩` [`Enter`] — transcript preview.
   Active expanded card: `Open in a window` — AppWindowMac — terminal modal; then `Collapse` — Minimize2 — hint `E` — collapse it.
   Active compact card: `Open` — SquareTerminal — hint `↩` [`Enter`] — terminal modal; then `Expand on board` — Maximize2 — hint `E` — expand it.
2. Only if the card has agents: `Agents (<agentCount>)` — Bot — agents viewer for it.
3. `Rename` — PencilLine — rename the card.
4. Archived: `Restore to board` — ArchiveRestore. Active: `Archive <n> sessions` when group has n > 1, else `Archive` — Archive — hint `⌫` [`Del`] — archive the group.
5. Only if running non-empty: `Stop <n> sessions` (n > 1) / `Stop session` — Square — stop them.
6. `New session from <n> contexts…` (group n > 1) / `New session from context…` — Sparkles — start merge with the group.
7. Active cards only: `Group <n> into section` (n = active cards among the selection when the card is in a multi-selection, n > 1) / `Group into section` — Group — hint `⌘G` [`Ctrl+G`] — group the selection (terminals included) or the card.
8. Active card in a section only: `Remove from section` — Ungroup.
9. Only if the card wears stuck notes: `Delete its notes` (2+) / `Delete its note` — StickyNote — delete them (no confirm).
10. —
11. `Remove from board` — Trash2 — danger — confirm, kill if live, remove.

### Terminal card

1–2. Same open items as an active card (`Open`/`Expand on board` or `Open in a window`/`Collapse`).
3. `Rename` — PencilLine.
4. `Group <n> into section` / `Group into section` — Group — hint `⌘G` [`Ctrl+G`]; then `Remove from section` — Ungroup — if in a section.
5. —
6. `Remove terminal` — Trash2 — danger — confirm only if live; remove.

(Archived terminals don't exist; the terminal menu never shows Archive, Stop, Agents or merge.)

### Section (right-click its header)

`sessions` = non-terminal members.

1. `Rename` — PencilLine
2. Colour swatches row (08 `ColorSwatches`, label `Section colour`, current colour marked) — picking closes the menu and recolours
3. —
4. `New session in section` — Plus
5. `New terminal in section` — SquareTerminal
6. `Select cards` — BoxSelect — disabled when no members
7. `Archive <n> sessions` (n > 1) / `Archive session` — Archive — disabled when no non-terminal members — archive those sessions
8. —
9. `Ungroup` (has members) / `Delete section` (empty) — Ungroup — remove the section, no confirm

### Note (right-click a free note)

`ids` = the selected notes when this note is in a multi-note selection, else this note.

1. `Edit` — PencilLine — hint `↩` [`Enter`] — write in this note
2. `Start session` — Play — disabled when its text is blank — start a session from this note
3. Colour swatches row (label `Note colour`) — closes the menu, recolours all `ids`
4. —
5. `Delete <n> notes` (n > 1) / `Delete note` — Trash2 — hint `⌫` [`Del`] — danger — delete `ids` immediately

### Note pad (right-click the pad or its top sheet)

1. `New note` — StickyNote — peel a note beside the pad

### View (right-click a view, or its own menu button)

1. `Refresh` — RefreshCw — disabled unless it has a refresh schedule or an approved current command
2. `Open its session` — SquareTerminal
3. `Rename` — PencilLine — rename the view inline
4. Only if the view may start sessions: `Stop it starting sessions` — Ban — turn that off (no confirm, no Claude call)
5. —
6. `Delete view` — Trash2 — hint `⌫` [`Del`] — danger — delete this view (asks); acts on this view only, never the selection

### Archive frame, edges, selection rectangle

No menu.

## Platform differences

- Shortcut modifier: ⌘ on Mac, Ctrl elsewhere, for ⌘K, ⌘A, ⌘G, ⌘Z; the "other" modifier (Ctrl on Mac, ⌘/Win elsewhere) must not be held for ⌘K and ⌘Z.
- Hints and tooltips via `keys()`: `⌘A`→`Ctrl+A`, `⌘G`→`Ctrl+G`, `⌘N`→`Ctrl+N`, `⌘T`→`Ctrl+T`, `⌫`→`Del`, `↩`→`Enter`; `N`, `T`, `E` and `Esc` stay as written.
- ⌘N/⌘T: Mac menu accelerators (work even with focus in a terminal). Windows/Linux: the window catches Ctrl+N / Ctrl+T itself (not in a terminal, not repeated, no Shift/Alt/Meta, letter by key code) and routes them through main back as `ev:menu`.
- Multi-select keys are Shift and Meta on every platform: on Windows, Ctrl-click does **not** add to the selection (it selects just that card and, because Ctrl is held, doesn't open it); Shift-click or Win-click do. (docs say "Ctrl stands in for ⌘": not true for selection.)
- Zoom-activation key: ⌘ on Mac, Ctrl elsewhere (Ctrl+wheel zooms on Windows).
- Pinch sensitivity: ×10 on Mac (trackpad pinch arrives as ctrl+wheel); ×1 on Windows (precision-touchpad pinch also arrives as ctrl+wheel, so it is 10× slower than on Mac).
- Shift+wheel pans horizontally only off Mac (macOS already turns it into deltaX).
- React Flow's Mac detection is `navigator.userAgent` containing `Mac`; the app's is `navigator.platform` starting with `Mac`.
- Fonts: SF Pro on Mac, Segoe UI Variable on Windows (tokens, 08).

## Edge cases & errors

- Board snapshot for another project (navigation race) → loading surface, never a mixed board.
- `nameSection` result arrives after the user left the board → reads the store; saves only if the store's board is still this project and the section is unnamed and not being renamed.
- Typing in a note when the board was closed mid-edit → saved directly to main.
- `startNote` double-click on ▶ → second start ignored while the first is pending.
- View start rate limit / permission → errors returned to the page (04).
- A section whose members all leave keeps its last stored corner (the auto-follow only runs with members).
- Creating a section near a card hits the `findFreeSpot` size bug (lands 9920 flow units below; Behaviors › Sections). Flag before porting.
- `ensureVisible(frame)` after grouping and `ensureVisible(spot)` after restore/leave use a 288×216 rect, so a large frame or an expanded card may not be fully brought into view.
- `fitActive` from the zoom pill is instant because of the event argument.
- Notes with a `sessionId` whose card is no longer on the board are drawn free but excluded from `fitActive` (it only takes notes with no `sessionId`).
- Fit with no measured nodes is queued until measurement; `fitBounds` (empty board) is immediate.
- Grouping when every selected card is archived → nothing; ⌘G with only notes/views selected → nothing (no preventDefault).
- E with only archived cards selected → nothing (no preventDefault); terminals are active cards and expand like sessions.
- ⌫ with only terminals selected → nothing happens to them (archive skips terminals; no removal); default still prevented.
- Enter with one card plus notes selected → opens the card.
- Pane click immediately after a marquee: React Flow swallows the click that ends a marquee.
- Dropping several cards where only some are over the archive: those over it archive, the rest land and join the lead's section.
- A drop that both archives a running card and bounces a terminal: the terminal returns immediately; the confirm then applies to the rest; Cancel returns everything recorded at drag start.
- Dragging a section or the archive with Shift/⌘ held keeps the selection, so selected nodes are dragged too while the section's members are also moved by delta — unclear: members that are also selected may move twice (BoardView.tsx:1703-1738); verify in Electron before porting.
- Archive drag drop stores `w/h` from the stored archive rect, not the open panel size.
- An archived card dropped back inside the open panel stays archived (stored at the drop position, drawn in the stack).
- The stack closes on a pointer-down anywhere else on the board — including the minimap/zoom pill, which are inside the board canvas.
- The view-relay `openSession` for an archived card opens its preview.
- Run-finished for another project is ignored.
- Open request for a session not on the board and not a run → consumed, nothing shown.
- `onlyRenderVisibleElements` above 150 cards: off-screen cards are not mounted (their terminals unmount; 10/12).
- Pinch over a `nowheel` region (e.g. the open, overflowing archive panel or a note being edited) does nothing at all.
- Window blur closes the context menu and resets Space/multi-select key state; any `contextmenu` event also resets them.
- `truncate` counts UTF-16 code units (JS `length`/`slice`); the port must too, or an emoji at the cut can be split differently.

## Tests to port

None of `BoardView.tsx` or the owned board components have tests, and `layout.ts` / `sections.ts` have none either. `src/shared/geometry.test.ts` covers `intersects`, `cardSize`, `findFreeSpot`, `findSpotBeside` — owned by 03.

New Rust tests to add in `nodus-board` (pure functions, ported from layout.ts/sections.ts and `buildNodes`):
- `containsPoint` (inclusive edges), `boundsOf`, `isInArchive` (centre rule), `cardObstacles` (exclusions, expanded sizes).
- `stackZone` closed (344×362) and open for 0, 1, 2, 3, 4, 5 results (w 672, 672, 992, 992, 992, 992; h 362, 362, 362, 544, 544, 544); `stackTop`; `stackResultSpot` with scroll; `stackResultsBand` (464 tall); `stackScrollMax` (0 for ≤ 4 results, 248 per extra row); `stackRank`.
- `spotRightOfArchive`, `spotForNewSession` (no active cards; rightmost tie keeps the first), `spotBelow` (both branches, clamp).
- `timeAgo` thresholds (`just now`, `Nm ago`, `Nh ago`, `Nd ago`, dates with and without year after 300 days), `truncate` (≤ max unchanged, trimEnd before `…`).
- `frameAround` (empty → 336×284; with cards), `sectionViews` (archived excluded, unknown section ids ignored, order kept), `sectionAt` (centre rule, current section preferred, newest otherwise, custom size), `sectionStatus`, `layoutGroup` (clear → no moves; blocked → grid cols/rows, reading order, rounding, largest-cell sizing), `spotInSection` (first free slot; full → right of the frame).
- `buildNodes`: pile placement and depth hiding, sink/rise flights with a fake clock, result clipping/hiding, z rules, draggable/selectable rules, forced deselect, busy/dragging position retention.
- Drop resolution (pure function over a drag result): archive/terminal bounce/note bounce/view bounce/section target/running-confirm list.
- The `findFreeSpot`-with-rect bug in `createSectionAt` (pin current behaviour or the agreed fix).

## Rust mapping

### Module layout (`nodus-ui/src/board/`)

- `mod.rs` — `BoardView` (loading vs canvas, keyed by project id) and `BoardCanvas` (GPUI view holding all local state listed below).
- `viewport.rs` — `Viewport {x, y, zoom}` (f64), clamp [0.1, 2], screen↔flow, `set_center`, `fit_bounds`, `fit_nodes` (React Flow padding maths), `zoom_by(factor, anchor)`, animated transitions (van Wijk ρ=√2 + cubic-in-out over a duration; instant when duration ≤ 0), move-start/move-end events (wheel end debounce 150ms, +150ms end delay), save debounce 400ms through `nodus-core`'s `board.save_viewport`.
- `scene.rs` — pure port of `buildNodes` producing `Vec<SceneNode>` with `{id, kind, rect, z, hidden, draggable, selectable, drag_handle, classes (stack depth, is_result, stack_enter, nowheel), clip, flight}`; lives in `nodus-board` if it can stay UI-free (it only needs board data, stack view, busy set, flights and a clock) — recommended, so it is unit-testable headless.
- `layout.rs` / `sections.rs` — ports of layout.ts / sections.ts (in `nodus-board`, beside 03's geometry).
- `input.rs` — the interaction state machine: `Idle → PendingNodeDrag{start, node} → NodeDrag`, `PendingMarquee → Marquee`, `Pan{button: Middle|SpaceLeft}`, `SelectionRectDrag`, `MinimapPan`; thresholds (marquee > 1px, drag > 1px, click suppressed on any movement), auto-pan (40px band, 15px/frame), key state (Space alone, Shift/Meta multi-select, zoom-activation key), wheel routing (nowheel regions, ctrl = pinch).
- `hit_test.rs` — z-sorted hit testing over scene nodes with per-kind rules: section = header strip only; pile cards depth ≥ 1 transparent; edges never; hidden never; plus node-provided sub-regions (`DragHandle`, `NoDrag`, `NoWheel`, `NoPan`, `Interactive`) reported by the node views of 10/04.
- `selection.rs` — selection set over ids (cards, notes, views) with the store sync rule (cards only), elevation (+1000), and the post-marquee selection rectangle flag (`nodes_selection_active`).
- `edges.rs` — facing-side anchors + Bézier (curvature 0.25) and dash animation (dashoffset 10 → 0 over 0.5s / 1s).
- `hud/` — `zoom_pill.rs`, `minimap.rs`, `hint.rs`, `toast.rs`, `confirm.rs`.
- `menus.rs` — builders for every menu in § Context menus returning `Vec<MenuItem>` for 08's `ContextMenu`.
- `commands.rs` — the board actions (create session/terminal/note/view/section, group, archive, stop, restore, remove, merge start, run action, start note, bar submissions), each taking the current scene snapshot and calling `nodus-core` commands; port 1:1 from BoardView so strings and placement match.

### Render layers (one GPUI element tree, painted in this order)

1. Canvas background fill + dot grid (paint dots per visible cell; at zoom 0.1 that is ≈ 0.08px dots every 2.2px — draw as a cheap quad pattern or skip dots below ~0.3px radius only if screenshots match).
2. Transformed world layer: nodes and edges sorted by (z, document order); edges at z 0 painted before z-0 nodes. Node rects animate via per-node position tweens (spring-free cubic-bezier / `linear()` easings above).
3. Selection rectangle (when active), then marquee.
4. Ink layer (11) — above the world, below HUD (verify z with 11).
5. HUD: zoom pill, minimap, hint, actions rail (05), prompt bar column with toasts and selection bar (11).
6. Context menu, then modals (confirm, merge, search, view dialog).

### Plug-in seams for 10/11/04

- `trait BoardNodeView` implemented by 10 (card, expanded card, archive, pad, note) and 04 (view): `fn paint(&self, rect, zoom, cx)`, `fn regions(&self) -> Vec<(Bounds, RegionKind)>`, `fn on_click/on_double_click` hooks the canvas calls only after its own routing (title-click timer, rename state, etc.), plus read-only access to board context (`RenamingContext`, `ArchiveHot`, `SectionHot`, `SectionNaming`, `ArchiveStack`) and the `BoardActions` command set (rename, setRenaming, renameSection, expand, archive, stop, editNote, startNote, openSession, openViewSession, renameView, viewMenu).
- Overlays (11): `PromptBar` gets `{into, context_count, on_submit}` and an `above` slot; `InkLayer` exposes `set_tool/escape/delete_selected/undo/redo` and reports `on_select`, `on_toast`; `SessionSearch`, `MergeDialog` get the props listed in Behaviors.
- Terminals (12): expanded cards embed the terminal element; it must render scaled by zoom and claim wheel events per 10/12.

### Risks

1. **Input parity with React Flow/d3**: the 1px thresholds, click suppression on any movement, middle-drag over nodes, Space-pan rules, marquee that always replaces, the post-marquee selection rectangle that swallows clicks, selection elevation and the 150ms move-end debounce are subtle and easy to "improve" by accident. Port from the tables above and test with recorded input traces.
2. **Pinch and smooth scrolling in GPUI**: GPUI must deliver trackpad pinch (macOS magnify / ctrl-wheel) and precise pixel deltas with phases; if it only offers line deltas or no pinch, zoom feel diverges. Verify in spike S1.
3. **Scaled live content**: expanded-card terminals (12) and view webviews (04, `wry`) must render at arbitrary zoom inside the transformed world. Native webviews can't be CSS-scaled; a snapshot-when-not-selected approach (as the docs describe "the page is a picture") may be required.
4. **Visual effects**: `backdrop-filter` blur/saturate (zoom pill, hint, toast, open archive panel, menus), CSS `color-mix`, conic-gradient rotating glow with blur, `linear()` spring easing and per-letter blur reveal all need GPUI implementations or approximations; screenshots in light and dark must match.
5. **Animation model**: every programmatic position change glides (420/620/460ms) while drags, archive moves, section moves and stack scrolling disable glides; edges intentionally don't glide; flights are timer-driven. A retained per-node tween map keyed by id, reset on interruption from the current drawn position, is needed.
6. **Bugs to decide on**: `createSectionAt` size bug, instant fit from the zoom pill, `ensureVisible` with card size for frames, keys by `event.key` on non-Latin layouts. Ground rule 2 says keep them; flag the first one explicitly.

## Parity checklist

- [x] BOARD-001 The board for project P renders a loading surface (fills the board area, background `--bg-canvas`) while the store's…
- [x] BOARD-002 The board view is keyed by project id: switching projects tears the canvas down and builds a new one (fresh viewport…
- [x] BOARD-003 Cards with `runOf` (action runs) or `viewOf` (view brains) are removed from the snapshot before anything on the…
- [x] BOARD-004 On unmount the board unregisters its view-relay handlers, its keydown listener and menu-command subscription, and…
- [x] BOARD-005 A new board snapshot (from `ev:board-changed` → reload, status, usage or agent-status updates, or any local store…
- [x] BOARD-006 When the reloaded board shows the session open in the terminal modal as archived (archived from outside, e.g. by its…
- [x] BOARD-007 The scene is, in order: the archive node, the note-pad node, one section node per section (oldest first), one card…
- [x] BOARD-008 The archive node sits at the archive's stored corner with the size of the stack zone (folded stack, or the open…
- [x] BOARD-009 An active card's node sits at its stored `x, y` with `cardSize(card)` (288×216, or its expanded `w×h`, default…
- [x] BOARD-010 Every archived card that is not a search result is placed at the stack top (archive x + 28, y + 52), regardless of…
- [x] BOARD-011 Pile order is most recent first by `max(archivedAt ?? 0, meta.updatedAt ?? createdAt)`; depth = index in the pile…
- [x] BOARD-012 Pile cards at depth ≥ 3 are hidden; one that was visible on the previous build instead starts a 700ms "sink" flight…
- [x] BOARD-013 A card that was hidden on the previous build, is now drawn, is not cut off by the result band and was not a result…
- [x] BOARD-014 Result cards whose top or bottom is a full card height (216) or more outside the results band are hidden; ones…
- [x] BOARD-015 Card node classes: `stack-card` for archived cards; `is-result` for results, else `stack-d<min(depth,3)>` for pile…
- [x] BOARD-016 Card z-index: active compact cards none (0); expanded cards 1; result cards base + 10; pile cards base + max(0, 3 −…
- [x] BOARD-017 Archived cards are draggable only if they are a result or the pile's top card (depth 0), and selectable only if they…
- [x] BOARD-018 An archived card's sticker notes are not passed to it (hidden until restored); an active card gets the notes stuck…
- [x] BOARD-019 A card node is reused unchanged when its card data (deep-equal), match indices, stickers, position, size, hidden…
- [x] BOARD-020 Section nodes: one per section, framing its active members (see Sections), z −2, not selectable/focusable/deletable…
- [x] BOARD-021 The pad node sits under the folded stack (archive x, archive y + 386, 344×296), z −1, not…
- [x] BOARD-022 The pad's top sheet is a note node whose id is the board's current "draft id" (a fresh UUID per board mount and…
- [x] BOARD-023 Free-note nodes (notes not stuck to a card on the board) sit at their stored position and size, z 2, selectable…
- [x] BOARD-024 A 700ms-later re-derivation is scheduled (soonest flight end + 20ms) whenever any flight is pending, so sinking…
- [x] BOARD-025 For every active card, one lineage edge is drawn from each of its `parents` that is also an active card on the board…
- [x] BOARD-026 A lineage edge is "animated" (dashes moving, stroke `--status-working`) while the child card's status is `working`…
- [x] BOARD-027 For every active card with `fromView` naming a view on the board, an always-animated dashed view edge is drawn from…
- [x] BOARD-028 Edges are not selectable, focusable or hit-testable.
- [x] BOARD-029 Each edge attaches to the midpoint of the side of each node that faces the other: with centres a, b and the source's…
- [x] BOARD-030 The edge path is a cubic Bézier from source anchor S to target anchor T with control points offset along each…
- [x] BOARD-031 Anchors use the nodes' logical positions and measured sizes: an edge jumps straight to a card's new position while…
- [x] BOARD-032 `fitActive(duration = 320)`: if the board has any active cards, free notes (notes without `sessionId`) or views, fit…
- [x] BOARD-033 The zoom pill's fit button calls `fitActive` with the click event as its argument, which is not a number, so that…
- [x] BOARD-034 `ensureVisible(p, size = 288×216)`: if the rect is already fully inside the visible flow rect (canvas size / zoom)…
- [x] BOARD-035 `viewportCentre(size)` = the top-left that centres something of `size` in the visible flow rect; `(0, 0)` when the…
- [ ] BOARD-036 Jumping to a search result for an active card: closes search, makes that card the only selected card (notes/views…
- [ ] BOARD-037 Jumping to a search result for an archived card closes search and opens its transcript preview at the match…
- [x] BOARD-038 A click on the pad's top sheet peels a note beside the pad (see Notes) only when it is the first click of a sequence…
- [x] BOARD-039 A click on a pile card (archived, not a result) opens the stack (clearing any dismissal); nothing else happens.
- [x] BOARD-040 A click on a card with Shift, ⌘ (Meta) or Ctrl held only changes selection (React Flow rules above); it never opens…
- [x] BOARD-041 A click on the card currently being renamed does nothing.
- [ ] BOARD-042 A click on an expanded card (anywhere that isn't a stopped control) focuses its terminal, like clicking a window…
- [x] BOARD-043 A click on a compact card's title text waits 240ms and then opens the card; any other click on a compact card opens…
- [ ] BOARD-044 Opening a card: archived → transcript preview; expanded → wake its on-board terminal (10/12); otherwise open the…
- [x] BOARD-045 A double-click on a section header starts renaming that section.
- [x] BOARD-046 A double-click on a free note starts writing in it; on the pad's top sheet it does nothing.
- [x] BOARD-047 A double-click on a card's title text (active card or result, not a pile card) cancels the pending title-click open…
- [x] BOARD-048 A double-click whose target is the pane itself creates a session centred on the pointer (top-left = pointer − (144…
- [x] BOARD-049 A click on the empty pane closes the context menu (and deselects everything per Canvas mechanics).
- [x] BOARD-050 Any drag start closes the context menu and cancels a pending title-click open.
- [x] BOARD-051 Dragging the archive: marks the archive busy and the board `is-moving-archive` (card, note and pad glides off)…
- [x] BOARD-052 Dropping the archive saves the archive rect `{x: round(x), y: round(y), w, h}` (stored w/h unchanged) to the store…
- [x] BOARD-053 Dragging a section by its header marks it and its members busy, sets `is-moving-section` (card glides off), and…
- [x] BOARD-054 Dropping a section: delta = round(node position − frame corner); member cards are saved at `x + dx, y + dy`; every…
- [x] BOARD-055 Dragging cards, notes or views records each dragged node's start position and marks it busy.
- [x] BOARD-056 While dragging, the archive is "hot" when any dragged non-terminal card's centre lies inside the current stack zone…
- [x] BOARD-057 While dragging, the "hot" section is the section the lead card would join: lead = the grabbed node if it is a card…
- [x] BOARD-058 On any drop, the archive and section hot states clear.
- [x] BOARD-059 Dropping the pad's top sheet with its centre on the pad or on the stack zone sends it back to its sheet spot…
- [x] BOARD-060 A terminal card dropped with its centre in the stack zone returns to its drag-start position (terminals are never…
- [x] BOARD-061 A note (size `noteSize`) or view (size `viewSize`) dropped with its centre in the stack zone or on the pad returns…
- [x] BOARD-062 Every other dropped card is saved at its rounded drop position with `archived` = (non-terminal and centre in the…
- [x] BOARD-063 Dropping an archived result card outside the stack zone restores it (`archived: false`) where it was dropped (drag…
- [x] BOARD-064 If the drop would archive any card that was active, live and not `done`, the board first asks with the…
- [x] BOARD-065 After a confirmed (or confirm-free) drop: cards are patched in one `patchCards` call, each moved note not bounced is…
- [x] BOARD-066 Archiving by drop stores the drop position (not the stack top) in the card; the card then glides into the stack…
- [x] BOARD-067 The stack is open only while the open flag is set and there is at least one archived card.
- [x] BOARD-068 Results: with an empty (trimmed) query, every archived card in pile order; otherwise cards whose title (`cardTitle`)…
- [x] BOARD-069 The stack zone is the folded stack (344×362) when closed, or the open panel sized for the result count (see…
- [x] BOARD-070 "Raised" is true while open and for 700ms after closing (the timer restarts if it reopens), so cards fly back in…
- [x] BOARD-071 `openStack` clears the dismissal and sets open; `closeStack` clears open, the query and the scroll, and sets dismissed.
- [x] BOARD-072 Setting the stack query resets the scroll to 0 and opens the stack when the trimmed query is non-empty.
- [x] BOARD-073 Submitting the stack search (Enter) blurs the focused element and opens the first result's transcript preview; with…
- [x] BOARD-074 Pointer movement over the canvas with no button pressed sets stack hover = pointer inside the stack zone; moving…
- [x] BOARD-075 The canvas gets `stack-lift` while the stack is closed and either (hovered and not dismissed) or the archive is hot.
- [x] BOARD-076 While open, a pointer-down anywhere inside the board canvas (capture phase) that is not on the archive node or an…
- [x] BOARD-077 While open with overflow (scroll max > 0), a wheel event without ctrlKey whose pointer lies in the stack zone…
- [x] BOARD-078 The effective scroll is `min(scroll, scrollMax)` for the current result count.
- [x] BOARD-079 Archive context exposed to the archive node (10): `open`, `hover` (= hover and not dismissed), `query`…
- [x] BOARD-080 A section's members are the active cards whose `sectionId` is that section; archived cards never belong to a section.
- [x] BOARD-081 A section's frame wraps its members: left = min x − 24, top = min y − 44, right = max right + 24, bottom = max…
- [x] BOARD-082 Whenever a section with members has a frame corner different from its stored corner, the stored corner is saved to…
- [x] BOARD-083 A section's status is `waiting` if any member is waiting, else `working` if any member is working, else none.
- [x] BOARD-084 The section a card at a position belongs to is the one whose frame holds the card's centre; where frames overlap…
- [x] BOARD-085 A new section takes the first colour (blue, purple, pink, red, orange, yellow, green, graphite) not used by another…
- [x] BOARD-086 A new section has a fresh UUID, empty name, rounded corner and `createdAt = now`, is saved immediately, and (unless…
- [x] BOARD-087 Grouping (⌘G, card menu, selection bar) takes the active cards among the ids (none → nothing happens), computes the…
- [x] BOARD-088 `layoutGroup`: if the frame around the cards where they are intersects no other active card (margin 0) and no zone…
- [x] BOARD-089 A group gets Claude to name it when at least one grouped card is a non-terminal with a title or transcript meta…
- [x] BOARD-090 When naming resolves with a name, it is saved only if the section still exists on this project's board, still has an…
- [x] BOARD-091 After grouping: cards are patched with the section id (and grid positions when tidied), the selection is cleared…
- [x] BOARD-092 `New section here` at point p: the preferred corner is (p.x − 24, p.y − 22); obstacles are active cards, the dock…
- [x] BOARD-093 Renaming a section commits the trimmed value on Enter or blur if it differs from the original (empty → untitled)…
- [x] BOARD-094 `Remove from section` moves the card to a free card-sized spot near (frame right + 32, card y), avoiding the…
- [x] BOARD-095 `Ungroup` / `Delete section` removes the section immediately (no confirm); its cards stay where they are, ungrouped.
- [x] BOARD-096 Changing a section's colour from its menu closes the menu and saves the section with the new colour.
- [x] BOARD-097 `New session in section` / `New terminal in section` start a session at the first free card slot inside the frame…
- [x] BOARD-098 While a dragged card hovers a section, that section's frame shows its hot style.
- [x] BOARD-099 New sessions reserve room for an expanded card (720×480): with a point, the nearest free spot to it (clear of every…
- [x] BOARD-100 A new session (from a point or the default spot) joins the section whose frame holds the centre of a 288×216 card at…
- [x] BOARD-101 Creating a session calls `sessions.create(projectId, {x, y, kind, autoMode: true})`, adds the card, joins the…
- [x] BOARD-102 A failed create shows the toast `Couldn't start a session: <message>` (or `Couldn't start a terminal: <message>` for…
- [x] BOARD-103 When the terminal modal closes or switches away from a session whose card is on the board, active and not expanded…
- [x] BOARD-104 Running an action (from the actions rail) creates a session with `actionId` at the new-session spot for a 288×216…
- [x] BOARD-105 When a run is taken over (its card stops having `runOf`), the card is moved to the nearest free spot to where it is…
- [x] BOARD-106 When a run of this project finishes (`ev:run-finished`), its terminal is disposed, the terminal modal closes if it…
- [x] BOARD-107 A notification/open request for this project and a session: the request is consumed; if the card is on the board the…
- [ ] BOARD-108 Merging ("New session from context") starts the new session below its sources (average x of the sources, 96 below…
- [ ] BOARD-109 Starting a merge takes the non-terminal cards among the ids (in id order, with their titles); none → nothing; it…
- [x] BOARD-110 Archiving ids takes the active non-terminal cards among them (none → nothing); if any is live and not `done` it asks…
- [x] BOARD-111 The store turns an archive of a session with nothing said in it into removal from the board instead, sets archived…
- [x] BOARD-112 Stopping ids takes the live non-terminal non-run cards among them (none → nothing); asks with the stop confirm when…
- [x] BOARD-113 `Restore to board` saves the card at a free card-sized spot to the right of the dock (dock right + 80, dock top)…
- [x] BOARD-114 `Remove terminal` asks first only when the shell is live (`Remove terminal?`), then removes the card from the board.
- [x] BOARD-115 `Remove from board` always asks (`Remove from board?`); on confirm a live session is killed first (awaited), then…
- [x] BOARD-116 A new note is saved immediately as `{id, text: '', color: 'yellow', x, y (rounded), createdAt: now}` and opens for…
- [x] BOARD-117 Peeling a note uses the current top sheet's id for the new note and gives the pad a fresh top-sheet id.
- [x] BOARD-118 Clicking the pad's top sheet (or the pad/top-sheet menu's `New note`) peels a note to the nearest free 216×216 spot…
- [x] BOARD-119 `New note here` at p places a note at the nearest free spot to (p.x − 108, p.y − 108) and pans to show it.
- [x] BOARD-120 Recolouring from the note menu closes the menu and saves every target note with the new colour.
- [x] BOARD-121 Deleting notes removes them immediately with no confirmation, written on or not (the docs claim otherwise; the code…
- [x] BOARD-122 Starting a session from a note requires that the note exists, has no session, has non-blank text and is not already…
- [x] BOARD-123 A note's session reserves 720×480 at the nearest free spot to the note's position (clear of every card, the other…
- [x] BOARD-124 Editing a note's text saves it through the store when this board is still the open board, else straight to main…
- [x] BOARD-125 Creating a view from a description places a 480×360 view centred on the given point, or in the middle of the visible…
- [x] BOARD-126 Opening a view's session asks main for the view's brain and opens its terminal modal; failure toasts…
- [x] BOARD-127 Deleting views asks first (`Delete view?` / `Delete N views?`), then removes them one by one.
- [x] BOARD-128 The view menu (from right-click or the view's own menu button at a screen point) matches § Context menus › View…
- [ ] BOARD-129 The board registers relay handlers for view pages: `openSession(id)` opens the card if it is on the board (else…
- [ ] BOARD-130 300ms after the board's cards, notes or views last changed the board broadcasts `board` to every view page; it…
- [ ] BOARD-131 The bar's "into" target is the single selected card when exactly one card is selected, no selected note has text, no…
- [ ] BOARD-132 The bar's context count = selected non-terminal cards + selected notes with non-blank text + selected views.
- [ ] BOARD-133 A `session` submission builds context from the selected non-terminal cards (title, git branch, last reply…
- [ ] BOARD-134 If that session can't start, the prompt is saved as a free yellow note at the reserved position, the board pans to…
- [ ] BOARD-135 An `into` submission: if the card is gone or archived toast `That session is no longer on the board.` (false); if it…
- [ ] BOARD-136 A `view` submission creates a view from the text in the middle of the view (true); failure toasts…
- [ ] BOARD-137 A `note` submission saves a note with the submitted colour at the nearest free 216×216 spot to the middle of the…
- [ ] BOARD-138 Toasts and the selection bar render stacked above the prompt bar (its `above` slot), toast first.
- [ ] BOARD-139 One toast shows at a time; a new message replaces the current one; each message clears itself after 4200ms unless a…
- [ ] BOARD-140 Confirms are one at a time, shown in the confirm dialog; the board awaits the answer (true = confirm).
- [ ] BOARD-141 The confirm dialog focuses its confirm button when it opens, so Enter (or Space) confirms; Escape, a pointer-down on…
- [ ] BOARD-142 "Overlay open" (which blocks board shortcuts and menu commands) = a context menu, confirm, merge dialog…
- [ ] BOARD-143 The selection bar shows while at least one card is selected and the merge dialog is closed; counts/disabled rules…
- [ ] BOARD-144 The New view dialog (04) opens from the pane menu with a point (the view is centred there) or from the title-bar…
- [ ] BOARD-145 ⌘K search (11) receives the non-terminal cards (active and archived) and a query that persists between openings…
- [x] BOARD-146 The empty-board hint shows while the board has no active cards (terminals count as active).
- [x] BOARD-147 The title bar shows the board's split button (see UI details); its main part creates a session at the default spot.
- [x] BOARD-148 Only one thing is renamed or written in at a time: a single "renaming" id is shared by cards, sections, notes and…
- [x] BOARD-149 `rename(sessionId, title)` saves the card's custom title (empty clears the override) via `patchCards`.
- [x] BOARD-150 `expand(ids, expanded)` affects only active cards among the ids whose expanded state differs (none → nothing)…
- [x] BOARD-151 `renameView(viewId, title)` patches the view's title; `openSession(id)` opens a card as a click would…
- [x] BOARD-152 Select all active: every active card selected, every archived card deselected; notes and views keep their selection.
- [x] BOARD-153 Clear selection: every node deselected and the store selection emptied.
- [x] BOARD-154 `Select cards` (section menu) selects exactly that section's members among cards (notes/views unchanged).
- [ ] BOARD-155 The ink layer clears the board's selection when it selects a stroke, and shows its messages through the board toast…
- [x] BOARD-156 The board listens to `keydown` on the window for its whole lifetime (one listener; current handlers read through refs).
- [x] BOARD-157 ⌘K (Ctrl+K off Mac; no Shift/Alt/other modifier): ignored when already handled (`defaultPrevented`) or the target is…
- [x] BOARD-158 Every other board key is ignored while the terminal modal, transcript preview, agents viewer, diff panel or a board…
- [ ] BOARD-159 ⌘Z / ⇧⌘Z (mod, no Alt, no other modifier) ask the ink layer to undo / redo, and prevent default only when it did…
- [x] BOARD-160 Escape: first lets the ink layer put its tool down or drop its selected stroke (and stops there if it did)…
- [x] BOARD-161 ⌘A (mod+A, no Shift/Alt): prevents default and selects all active cards.
- [x] BOARD-162 ⌘G (mod+G, no Shift/Alt): with at least one card selected, prevents default and groups the selected cards; otherwise…
- [x] BOARD-163 ⌫ or Delete (no mod, no Alt): if the ink layer deleted its selected stroke, prevent default and stop; else if any…
- [x] BOARD-164 Enter (no mod, no Shift) with exactly one card selected opens it; with no card and exactly one note selected starts…
- [x] BOARD-165 N (no ⌘/Ctrl/Alt; Shift allowed) prevents default and creates a session at the default spot; T likewise creates a…
- [ ] BOARD-166 V (no ⌘/Ctrl/Alt, not auto-repeat) prevents default and toggles voice in the prompt bar.
- [ ] BOARD-167 `/` (no ⌘/Ctrl/Alt) prevents default and focuses the prompt bar with `/` typed (its slash menu).
- [x] BOARD-168 E (no ⌘/Ctrl/Alt): when any active card is selected, prevents default and expands all selected active cards if any…
- [ ] BOARD-169 P, A, X (no ⌘/Ctrl/Alt) prevent default and pick the ink tool pen, arrow, eraser.
- [x] BOARD-170 Letter keys are compared by `event.key` lower-cased (not by key code), so on non-Latin layouts…
- [x] BOARD-171 App-menu commands (`ev:menu`) are ignored while a board overlay, the diff panel or the agents viewer is open — but…
- [x] BOARD-172 Every menu opens at the pointer's client position (clamped on screen by ContextMenu, 08) and replaces any open menu…
- [x] BOARD-173 Right-clicking the pane (including inside a section frame) shows the pane menu (§ Context menus › Pane), with…
- [x] BOARD-174 Right-clicking the pad or its top sheet shows the pad menu; a free note the note menu; a view the view menu; a…
- [x] BOARD-175 Right-click never changes the selection; a card or note menu acts on the whole selection when the clicked item is…
- [x] BOARD-176 Pane-menu `New session here` / `New terminal here` create a session centred on the clicked point (top-left = p −…
- [x] BOARD-177 Pane-menu `Show everything` fits every drawn node (archive, pad, sections, all visible cards, notes, views, top…
