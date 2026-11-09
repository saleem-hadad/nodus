# Track: gpui-polish

- **Branch:** `rust/gpui-polish`
- **Scope:** the visual and UX gaps between the native UI and 0.9.4 that need a change in our patched GPUI (`third_party/gpui/`, D13) or in shared nodus-ui components: custom cursors, spellcheck underlines, rounded search highlights, native-looking tooltips, shadows clipped to outside their box.
- **What changed in GPUI:** `third_party/gpui/PATCHES.md`, patches 12 onwards.

## Status

| Item | State |
| --- | --- |
| 1. Custom cursors: the ink's `move` and the eraser's circle | ✅ PATCHES 12; INKUI-016, INKUI-017 |
| 2. Spellcheck underline in text fields | ☐ |
| 3. Search highlights: rounded marks, the title's underline offset | ☐ |
| 4. Tooltips as Chromium showed them | ☐ |
| 5. Shadows drawn only outside a see-through box | ☐ |

## Done (behavior IDs)

- **INKUI-016**: with the pointer tool and no button held, the pointer over a stroke (on the board's surface) shows CSS `move` over the whole board, checked on each pointer move (GPUI coalesces them to one a frame); switching tools drops it (`ink/input.rs` `ink_hover`, `ink/draw.rs`).
- **INKUI-017**: the eraser's cursor is `ink.css`'s SVG circle rasterized at 1× and 2× (`ink/cursor.rs`), an image cursor with its hotspot at the centre and `cell` as the fallback; pen and arrow crosshair, text I-beam, as before.

## Log

- Started from main (f4467cd).
- Item 1: CoreCursor's move arrow checked on this macOS (24×24, hotspot 12,12, the same picture Chromium shows); `_moveCursor` exists but throws, so it's not used.
