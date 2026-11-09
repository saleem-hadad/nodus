# Track: gpui-polish

- **Branch:** `rust/gpui-polish`
- **Scope:** the visual and UX gaps between the native UI and 0.9.4 that need a change in our patched GPUI (`third_party/gpui/`, D13) or in shared nodus-ui components: custom cursors, spellcheck underlines, rounded search highlights, native-looking tooltips, shadows clipped to outside their box.
- **What changed in GPUI:** `third_party/gpui/PATCHES.md`, patches 12 onwards.

## Status

| Item | State |
| --- | --- |
| 1. Custom cursors: the ink's `move` and the eraser's circle | ✅ PATCHES 12; INKUI-016, INKUI-017 |
| 2. Spellcheck underline in text fields | ✅ `components::spellcheck` over audit-shell's `nodus_platform::spell`: BAR-005, PLAT-137, PLAT-138 (no menu); prompt bar, capture field, note editor, merge instruction, action dialog |
| 3. Search highlights: rounded marks, the title's underline offset | ✅ PATCHES 13; SRCH-019, SRCH-020 as 0.9.4 draws them; also the transcript preview's marks and the diff's words |
| 4. Tooltips as Chromium showed them | ✅ PATCHES 15: no more one word a line (max-content layout); AppKit's 1.5 s / quick 0.5 s timing on macOS; 11pt system font as `toolTipsFontOfSize:` |
| 5. Shadows drawn only outside a see-through box | ✅ PATCHES 14: `shadow_outside()`; every frosted surface, the archive panel, the "N more" pill, the terminal's loading pill, the note's play button |

## Done (behavior IDs)

- **INKUI-016**: with the pointer tool and no button held, the pointer over a stroke (on the board's surface) shows CSS `move` over the whole board, checked on each pointer move (GPUI coalesces them to one a frame); switching tools drops it (`ink/input.rs` `ink_hover`, `ink/draw.rs`).
- **INKUI-017**: the eraser's cursor is `ink.css`'s SVG circle rasterized at 1× and 2× (`ink/cursor.rs`), an image cursor with its hotspot at the centre and `cell` as the fallback; pen and arrow crosshair, text I-beam, as before.
- **BAR-005** (spellcheck), **PLAT-137**, **PLAT-138**: misspelt words in the fields 0.9.4 spellchecked (the prompt bar, the capture field, a note being written, the merge dialog's instruction, the action dialog's description, name and prompt) get Chromium's red dots; no suggestions menu, as 0.9.4 had none (it never handled `context-menu`). The checker is the system's, through audit-shell's `nodus_platform::spell` (`NSSpellChecker`; `ISpellChecker` on Windows, where Electron used Hunspell); this track's `components::spellcheck` replaced audit-shell's first underline component (estimated dots, every misspelling shown once the field had been focused) with one measured against Electron, and added the fields it hadn't reached (prompt bar, notes, merge instruction). When, measured in Electron 44.5.1 (hidden pages typed into through the input method): checked 50 ms after an edit unless the caret sits in the word just typed (`trimms` gets its dots once a space follows or the caret moves); an edit drops the dots of the word it touches; text a field was given rather than typed isn't judged; a textarea keeps its dots on blur, an input drops them. How they look, measured there: 2px round dots every 4px from half a pixel left of the word, centred 2px below the baseline, `#ff3b30` at 75% (`components/spellcheck.rs`). Views' dialog (views-ui's) can attach the same component.
- **SRCH-019, SRCH-020** (the look): snippet marks cover the font's content area with 3px round ends; title underlines sit 3px below the baseline, 1px thick as the goldens show (`search/text.rs`, `search/view.rs`). The transcript preview's marks (`terminal/messages.rs`, 3px) and the diff's changed words (`diff_modal.rs`, 2px) use the same inline backgrounds.

## Checked side by side

Spec 11's scenes through the real app (`crates/nodus-ui/parity/scenes11.py`, Node 22+ first on `PATH`), region diffs with pixelmatch (threshold 0.1):

| Scene | Region | Light | Dark |
| --- | --- | --- | --- |
| search-results | the modal (760,200 1360×580) | 0.77% | 1.15% |
| capture-recents | the "N more" pill (250,690 800×80) | 0.00% | 0.00% |
| board-archive-open | the whole frame / the open panel (60,240 1680×960) | 0.41% / 0.81% (text only) | 0.40% / 0.78% |
| bar-focused | the prompt bar (400,1580 2080×220) | 0.02% | 0.02% |
| bar-spell (scratch scene: "Fix the coupn feild so it trimms" typed, then ←) | the dots against Electron's 14px/20px textarea | rows baseline+2…+5, 8 device px apart, `trimms` dotted after ← | same |
| bar-typing, bar-view, bar-note | the prompt bar | 0.00–0.03% (no dots where 0.9.4 had none) | |
| capture-guess, capture-attachments | the panel | 0.01%, 0.16% | |
| board-pad-hover | the whole frame (GPUI draws the pad's tooltip, the golden's native one isn't in the shot) | 0.64% (was 1.11%) | |

The highlights match the golden row for row (mark 375–402, underline 364–365 at x 1150–1250). What's left in the modal is text: the monospace snippet line and the footer's key hints sit a little off (not this track's).

## Remaining differences

- Tooltips are still drawn by GPUI inside the window. 0.9.4's were AppKit's own (Chromium hands `title` text to `NSToolTipManager`): a separate panel that can extend past the window, in AppKit's material. The font, delay and quick mode now match; the panel's padding, corners and the wrap width (320px) are ui-shell's estimates. AppKit's private layout calls crash when called directly, so the wrap width wasn't measured. A native tooltip (Chromium's `setToolTipAtMousePoint:` path) was considered: on this macOS, `addToolTipRect:` no longer goes through `addTrackingRect:owner:…`, which that path relies on, and it can't be checked without activating the app, so it wasn't taken.

## Log

- Started from main (f4467cd).
- Item 2: Chromium's spelling behaviour probed with `webContents.insertText` in a hidden Electron 44.5.1 window; NSSpellChecker gives the same words (`Teh` isn't flagged by either).
- Item 4: AppKit's tooltip timing read from `NSToolTipManager` (ivars and `_getDisplayDelay:inQuickMode:forView:` in a probe with no window shown).
- Item 1: CoreCursor's move arrow checked on this macOS (24×24, hotspot 12,12, the same picture Chromium shows); `_moveCursor` exists but throws, so it's not used.
