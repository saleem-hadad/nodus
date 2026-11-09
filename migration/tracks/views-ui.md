# Track: views-ui (P9, spec 04's UI)

- **Branch:** `rust/views-ui`
- **Scope:** `crates/nodus-ui/src/views/` (new): the view card, its page in a native webview (wry 0.57.1: WKWebView,
  WebView2) with the snapshot swap, the relay between pages and the core, the large window, the New view dialog.
- **Spec:** `migration/features/04-board-tools-views.md` VIEW-027…030, 044…059, 101, 122…154; spec 09 BOARD-129/130.
- **Status:** 🟡 in progress.

## Files

| File | What |
| --- | --- |
| `views/mod.rs` | `install` (the view renderer), `board_layer` (frames kept in step, the large window, the dialog), `reason` (VIEW-053), `every_label` (VIEW-136) |
| `views/card.rs` | the card: shadow by state, title bar, body (snapshot + probe), brain status (VIEW-124) |
| `views/parts.rs` | status dot, tools ⟳ ✎ ⌨ ⤢ ⋯, approvals strip, empty state, error line, card buttons |
| `views/inputs.rs` | the ask bar and the rename field |
| `views/resize.rs` | resize zones and corner handles (VIEW-140) |
| `views/frames.rs` | one webview per frame, kept for every view with a page while its board is open (R13), the body probe |
| `views/swap.rs` | the snapshot swap (D3): what covers a page, drag/resize/zoom/ink/marquee/tooltips, snapshots |
| `views/webview.rs` | a frame's wry webview: custom scheme → `Views::serve_page`, shell, IPC, navigation guard, place/zoom/snapshot |
| `views/shell.rs` | the shell page (sandboxed iframe + relay script), URLs, the navigation rule (VIEW-028) |
| `views/relay.rs` | page messages → report/call/board, replies, `ev:view` to frames, `board`/`git` broadcasts |
| `views/large.rs`, `views/dialog.rs` | the large window (VIEW-147) and the New view dialog (VIEW-151…154) |
| `views/mac.rs` | the clip container (rounded, pass-through, resize-edge pass-through) and sRGB snapshots |
| `views/icons.rs` | `ellipsis`, `wand-sparkles` |

Shared files touched (additive): `board/nodes.rs` (`ViewRenderer` global used by `NodeRenderers::view`'s default,
`BoardCx.pointer: BoardPointer`), `board/canvas.rs` (fills `pointer`), `board/render.rs` (`views::board_layer`
children), `board/mod.rs` (`views::install`), `lib.rs`, `components/icon.rs` (one asset loader line),
`automation.rs` (`type` command), `Cargo.toml` (wry, CoreGraphics).

## Done

(filled in as behaviors are checked)

## Deferred

(filled in)
