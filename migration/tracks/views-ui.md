# Track: views-ui (P9, spec 04's UI)

- **Branch:** `rust/views-ui`
- **Scope:** `crates/nodus-ui/src/views/` (new): the view card, its page in a native webview (wry 0.57.1: WKWebView,
  WebView2) with the snapshot swap, the relay between pages and the core, the large window, the New view dialog.
- **Spec:** `migration/features/04-board-tools-views.md` VIEW-027…030, 044…059, 101, 122…154; spec 09 BOARD-129/130.
- **Status:** ✅ ready to merge on macOS (54 VIEW + BOARD-129/130); Windows builds the same code with the gaps below.

## Files

| File | What |
| --- | --- |
| `views/mod.rs` | `install` (the view renderer), `board_layer` (frames kept in step, the large window, the dialog), `reason` (VIEW-053), `every_label` (VIEW-136) |
| `views/card.rs` | the card: shadow by state, title bar, body (snapshot + probe), brain status (VIEW-124) |
| `views/parts.rs` | status dot, tools ⟳ ✎ ⌨ ⤢ ⋯, approvals strip, empty state, error line, the card's buttons |
| `views/inputs.rs` | the ask bar and the rename field, focus on the render after they first draw |
| `views/resize.rs` | resize zones and corner handles (VIEW-140) |
| `views/frames.rs` | one webview per frame, kept for every view with a page while its board is open (R13); the body probe; the driver's `views` report |
| `views/swap.rs` | the snapshot swap (D3): what covers a page, drag/resize/zoom/ink/marquee/tooltips, snapshots, the keyboard back to the board |
| `views/webview.rs` | a frame's wry webview: custom scheme → `Views::serve_page`, the shell, IPC, the navigation guard, place/scale/snapshot |
| `views/shell.rs` | the shell page (`<iframe sandbox="allow-scripts">` + relay script), URLs, the navigation rule (VIEW-028) |
| `views/relay.rs` | page messages → report/call/board, replies, `ev:view` to frames, `board`/`git` broadcasts |
| `views/large.rs`, `views/dialog.rs` | the large window (VIEW-147) and the New view dialog (VIEW-151…154) |
| `views/mac.rs` | the clip container (rounded, pass-through, resize-edge pass-through), occlusion, no context menu, sRGB snapshots, first responder, driver clicks |
| `views/ink.rs`, `views/mac_ink.rs` | the board's ink over live pages, in a native overlay above each page (track views-polish) |
| `views/icons.rs` | `ellipsis`, `wand-sparkles` |
| `parity/views_scenes.py` | the six golden scenes, plus `try-*` interaction scenes (relay page, rename, refresh, resize, drag, create, wheel, keyboard) |

Shared files touched (additive): `board/nodes.rs` (`ViewRenderer` global used by `NodeRenderers::view`'s default,
`BoardCx.pointer: BoardPointer`, `BoardActions::click_select`), `board/canvas.rs` (fills `pointer`; `click_select`),
`board/render.rs` (`views::board_layer` children), `board/mod.rs` (`views::install`), `lib.rs`,
`components/icon.rs` (one asset loader line), `automation.rs` (`type`, `views`, `native-click`, `hit`),
`Cargo.toml` (wry, CoreGraphics).

## How a page is shown

- **Frames.** Every view with `version > 0` on the open board has a webview (the card's), and the view open large a
  second one; they go with the board (VIEW-029/030). Each loads its own shell at `nodus-view://<id>/__nodus_shell?f=<n>`
  (served by the frame itself), which holds the page in `<iframe sandbox="allow-scripts" src="nodus-view://<id>/?v=<v>">`.
  The page itself comes from `Views::serve_page`, status and headers unchanged. Only the shell's own document may post
  to the app (the page's frame could reach the webview's handler; its messages are dropped), and the frame may only
  navigate within its view (VIEW-028).
- **Scale.** The webview is the card's body on screen; the shell lays the page's frame out at the body's size in flow
  units and scales it by the board's zoom with a CSS transform (`__nodusScale`), as 0.9.4's board scaled its iframe.
  WebKit's page zoom laid out the same but bumped zoomed-out text to its 9px "smart minimum" (rows 12% taller), and its
  private view scale lost a pixel to rounding and left a white sliver.
- **Swap (D3).** As each frame paints, the body's probe hitbox is hit-tested over the page's visible rect in that
  frame (views-polish): anything drawn later that blocks the pointer (notes, menus, the minimap, dialogs), or a
  tooltip, covers it. Covered, dragged, resized, zooming, an ink tool out, the marquee over it, or parity mode → the
  page goes transparent and takes no pointer, and GPUI draws a WebKit snapshot (sRGB, 2–16 ms) in its place, in that
  same frame when one is ready. The board's ink draws over a live page in a native overlay. Pages keep drawing while the window
  is covered (`_setWindowOcclusionDetectionEnabled:NO`) and off screen (the container shrinks rather than hides), so
  snapshots are never blank and pages load for page checks (R13).
- **Pointer.** While nodes are dragged, a marquee or selection rectangle is up, the viewport moves, or this view is
  resized, every page lets the pointer through (VIEW-141); the resize handles reach 5 units into the page. Unused
  wheels come back from the bridge and pan/zoom the board (VIEW-047); a pinch over a page goes up the responder chain
  to GPUI.
- **Activation (VIEW-051).** `startSession` needs the shell's `activeElement === frame && userActivation.isActive`
  and the page's webview to be the window's first responder; a press on the board gives the keyboard back to GPUI.

## Checked side by side

`python3 crates/nodus-ui/parity/views_scenes.py views- --golden <dir> --world <scratch>` (node 22+ first on PATH), then
`compare.mjs` (after main's cards and overlays merged in, so the whole shot is meaningful):

| Scene (light / dark) | Whole shot | Docs build status | Broken links | Search index | Scene's own region |
| --- | --- | --- | --- | --- | --- |
| views-board | 0.49% / 0.49% | 1.80% / 1.75% | 1.25% / 1.23% | 2.79% / 2.76% | |
| views-brain-waiting | 0.54% / 0.54% | 1.80% / 1.75% | 2.16% / 2.04% | 2.79% / 2.76% | |
| views-menu | 0.53% / 0.52% | 2.24% / 2.07% | 1.55% / 1.28% | 2.79% / 2.76% | menu 1.57% / 1.24% |
| views-change | 0.56% / 0.55% | 2.95% / 2.76% | 1.25% / 1.23% | 2.79% / 2.76% | |
| views-large | 0.13% / 0.10% | | | | modal 0.15% / 0.11% |
| views-new | 0.09% / 0.08% | | | | dialog 0.28% / 0.28% |

What differs inside the cards is the page's text (WebKit vs Chromium antialiasing and metrics, rows within 1px) and
GPUI's text (gpui-text). Interaction scenes (`try-*`, looked at, not goldens): every `window.nodus` method round-trips
(notes, views, view, sessions, git, setState + its event, openUrl/openSession/startSession errors), a clicked button's
`startSession` starts an expanded card beside the view with the dashed edge, `board`/`git` events arrive, rename,
⟳ with the error line, resize, drag with pass-through, ⌘Enter create, wheel replay, keyboard handoff.

## Done

VIEW-027, 028, 029, 030, 044, 045, 046, 047, 048, 049, 050, 051, 052, 053, 054, 055, 056, 057, 058, 059, 101, 122,
123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145,
146, 147, 148, 149, 150, 151, 152, 153, 154 (54); BOARD-129, BOARD-130.

VIEW-056/057 (the start rules), 058/059 (placement, edge), 101 (placement), 122/123 (nodes), 132 (⌨), 143–146
(menu, confirm, removal, `patchView`), 148/149 (drop bounce, keys) and 150 (the edge's anchor) are the canvas's code
(board-canvas), reached from here; checked through the scenes above.

## Deferred / gaps

- **Windows (unbuilt here; GPUI's build script needs Windows' resource compiler):** WebView2 child webview with
  `https://nodus-view.<id>/` URLs, bounds, visibility, theme and IPC are in; **no snapshots** (`CapturePreview` not
  written: a covered page steps aside and the body shows its background instead), **no rounded clipping or clip to the
  board** (needs `webview2-com` composition or a window region), **no pointer pass-through** (no `hitTest` there),
  and the activation check is the shell's alone (the page's scale is the shell's CSS transform, as on macOS).
- ~~Ink strokes over a live page draw under it while no tool is out~~ (views-polish: a native overlay above the page).
- ~~Only the card's own tooltips swap the page out~~ (views-polish: any tooltip over the page covers it).
- ~~The New view textarea's placeholder doesn't wrap, and it has no resize grip~~ (views-polish).
- A right-click in a page empties WebKit's context menu (`willOpenMenu:withEvent:`), so none opens, as in 0.9.4; the driver doesn't exercise it.
- ~~An overlay appearing over a live page shows a frame or two later than the overlay~~ (views-polish: decided in the
  frame's paint, presented with the webview change; only a page never pictured yet is still late).

## Notes for the orchestrator

- ~~`Window::render_to_image` sometimes drops a group-opacity layer's contents~~ (views-polish: WebKit hadn't
  delivered the snapshots yet; failed snapshots are retried and the driver waits for pages' pictures; the large
  window's snapshot is in its panel again).
- `Views::serve_page` runs on the main thread from WebKit's scheme handler (it's synchronous and cheap).
- ~~The git broadcast reads `git status` itself~~ (views-polish: the relay follows the git UI's model).
