# Track: gpui-text

- **Branch:** `rust/gpui-text`
- **Scope:** text drawn by the patched GPUI (`third_party/gpui/`, D13) indistinguishable from what Chromium drew in 0.9.4: colour space, SF Pro widths, wrapping (hanging spaces, `overflow-wrap`, `word-break`, `-webkit-line-clamp`), baseline and glyph rounding, anti-aliasing; nodus-ui's workarounds removed. Also the Windows `effects` CI step.
- **Reference:** the goldens (`migration/parity/golden/`, from `rust/parity-harness`), and Electron 44.5.1 (0.9.4's runtime) measuring and drawing test pages hidden (no Dock icon, no focus).
- **What changed in GPUI:** `third_party/gpui/PATCHES.md`, patches 6–10.

## Status

| Item | State |
| --- | --- |
| 1. sRGB-tagged layer inside GPUI (Metal; DirectX / wgpu checked) | ✅ PATCHES 6; nodus-ui's tagging removed |
| 2. Widths at every size (the "tracking" gap) | ✅ PATCHES 7: the cause was weight 500 → Regular; tracking matches (CoreText's `trak` is what Chromium applies too) |
| 3. Hanging trailing spaces; `overflow-wrap`, `word-break`, line clamp | 🟡 WIP (committed): spaces hang, rows align without them, `OverflowWrap`/`WordBreak` text options, Chromium's ellipsis for `text-overflow` and `-webkit-line-clamp`. One-line ellipses and wrapped paragraphs now match Chromium's breaks; left: the clamp's centred last line sits ~2.4px left, min-content sizes, PATCHES entry |
| 4. Chromium's baseline and glyph rounding | ✅ PATCHES 9: layout-unit line heights and widths, unrounded text origin, Blink's baseline, Skia's glyph rounding |
| 5. Anti-aliasing and font smoothing | ✅ PATCHES 8: `-webkit-font-smoothing: antialiased` (no CoreGraphics thickening); grayscale AA as Chromium |
| Scene diffs before / after | ☐ |
| Windows `effects` CI step | ☐ |

## How to check text against Chromium

Widths, every size and weight the app uses:

```sh
# a JSON of {size, weight, text, width} measured in Electron 44.5.1 (Range.getBoundingClientRect)
cargo run -p nodus-ui --example text_metrics -- <chromium.json>
```

Drawn text, case by case (lines, wrapping, clamps, ellipses), at a device scale of 2:

```sh
cd crates/nodus-ui/parity/text
node cases.mjs <dir>                                    # page.html + cases.json
<app>/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron chromium.cjs <dir>   # chromium.png, page.json
cargo run -p nodus-ui --example text_parity -- <dir>/cases.json <dir>/gpui.png
NODE_PATH=<migration/parity>/node_modules node compare.mjs <dir>              # per case
NODE_PATH=… node compare.mjs <dir> 520 10 310 110                            # one region -> <dir>/side.png
```

Neither window takes the focus: Electron runs with the `prohibited` activation policy and a hidden window, GPUI's is a non-activating pop-up.

`crates/nodus-ui/tests/text_parity.rs` (no test harness: the Mac text system wants the main thread) checks what doesn't depend on the macOS version's fonts.

## Log

- Started: worktree on `main` (83c9c0d); debug build of `nodus`; Electron measuring page set up in the scratch folder.
- `examples/text_metrics.rs`: weight 400 already matched Chromium within 0.02px at every size; 500 was 1–5px narrower per label (Medium resolved to Regular), 600 slightly narrow, 700 fine. The "tracking" gap was the weight.
- Colour space, weights, smoothing and rounding fixed in GPUI. Single-line text cases are pixel-identical to Chromium; in Settings, text lines land where 0.9.4's do except where the boxes around them sit a fraction of a pixel off (ui-shell's layout).
