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
| 3. Hanging trailing spaces; `overflow-wrap`, `word-break`, line clamp | ✅ PATCHES 10: spaces hang, `OverflowWrap`/`WordBreak` text styles, Chromium's ellipsis for `text-overflow` and `-webkit-line-clamp`, a clamped row aligned by its wrapped width, centring in layout units, min-content = the widest unbreakable piece. 69 of 71 text cases pixel-identical |
| 4. Chromium's baseline and glyph rounding | ✅ PATCHES 9: layout-unit line heights and widths, unrounded text origin, Blink's baseline, Skia's glyph rounding |
| 5. Anti-aliasing and font smoothing | ✅ PATCHES 8: `-webkit-font-smoothing: antialiased` (no CoreGraphics thickening); grayscale AA as Chromium |
| Scene diffs before / after | ✅ below: every shell scene closer, both themes |
| Windows `effects` CI step | 🟡 never ran on Windows (a failing capture test stopped the job first; fixed). The step now runs unless the run is cancelled; required on macOS, still `continue-on-error` on Windows until seen green. All 26 HLSL entry points compile with DXC |
| nodus-ui workarounds | ✅ the layer's sRGB tagging removed (item 1); `spaced_text` (letter-spacing, which GPUI lacks) now puts glyphs on Chromium's baseline. The `min_w_0()` calls mirror 0.9.4's own `min-width: 0` and stay |

## Scene diffs

ui-shell's shell scenes (`crates/nodus-ui/parity/scenes.py`, 18 scenes × 2 themes), rendered by main at b63c4d9 (before) and by this branch (after) from the harness's world, against the goldens of 0.9.4, `compare.mjs` (pixelmatch, threshold 0.1, share of the 2880×1800 frame that differs). No board scenes render on main yet.

| Scene | Light | Dark |
| --- | --- | --- |
| `projects-context-menu-focus` | 0.99% → 0.10% | 1.43% → 0.18% |
| `projects-context-menu` | 1.05% → 0.10% | 1.72% → 0.13% |
| `projects-drop` | 2.57% → 2.54% | 3.78% → 3.79% |
| `projects-empty` | 0.09% → 0.00% | 0.12% → 0.03% |
| `projects-focus-none` | 0.27% → 0.13% | 0.28% → 0.16% |
| `projects-focus` | 0.19% → 0.14% | 0.64% → 0.59% |
| `projects-grid` | 0.31% → 0.17% | 0.33% → 0.21% |
| `projects-group-menu` | 0.39% → 0.36% | 0.32% → 0.49% |
| `projects-group-picker` | 0.03% → 0.03% | 6.25% → 0.02% |
| `projects-remove-confirm` | 91.74% → 0.04% | 5.74% → 0.07% |
| `projects-rename` | 0.39% → 0.15% | 0.94% → 0.18% |
| `projects-selected` | 0.35% → 0.07% | 0.90% → 0.10% |
| `rail-numbers` | 9.23% → 9.14% | 9.37% → 9.30% |
| `rail-open` | 13.84% → 9.70% | 13.85% → 9.73% |
| `rail-pinned` | 13.90% → 9.14% | 14.04% → 9.29% |
| `settings-bottom` | 1.67% → 1.59% | 6.22% → 1.73% |
| `settings` | 0.94% → 0.78% | 7.09% → 0.83% |
| `titlebar-update` | 0.56% → 0.08% | 0.59% → 0.11% |

What the "before" column hides: on main, menu rows drift (rounded line heights), so the scripted clicks on a menu's lower items miss: `projects-remove-confirm` never reaches its dialog, and the group picker missed it once. After, every row lands where 0.9.4's does and the clicks hit.

What is left is not text: the folder icons' own rendering, the overlay scroll bar (shown in some runs, not others), the rail scenes' board (not on main yet), Settings' Accessibility permission row (missing in the Rust dialog, so the rows below sit higher), the group menu (the golden's opens where its harness right-clicks, ~530px to the right of `scenes.py`'s click), and the drop overlay, which the harness's file-drop step shows in some runs only (0.24% in one dark run). Text that both sides draw at the same place is pixel-identical, the letter-spaced headings included.

## How to check text against Chromium

Widths, every size and weight the app uses:

```sh
# a JSON of {size, weight, text, width} measured in Electron 44.5.1 (Range.getBoundingClientRect)
cargo run -p nodus-ui --example text_metrics -- <chromium.json>
```

Drawn text, case by case (lines, wrapping, clamps, ellipses, min-content), at a device scale of 2:

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

Scenes, before and after (the goldens copied into each folder, so `.rust.png` shots land beside them):

```sh
python3 crates/nodus-ui/parity/scenes.py --golden <dir> --bin <nodus> --world <scratch>/world
NODE_PATH=<migration/parity>/node_modules node crates/nodus-ui/parity/compare.mjs <dir>
```

The DirectX shaders without Windows (DXC's Linux build in an amd64 container; it checks syntax and types, not fxc's shader-model-4.1 limits):

```sh
docker create --name dxc --platform linux/amd64 ubuntu:24.04 sh /dxc/check.sh   # check.sh: dxc -HV 2016 -WX -T {vs,ps}_6_0 -E <module>_{vertex,fragment}
docker cp <linux_dxc> dxc:/dxc && docker cp third_party/gpui/gpui-pre-windows/src dxc:/src && docker start -a dxc
```

## Log

- Started: worktree on `main` (83c9c0d); debug build of `nodus`; Electron measuring page set up in the scratch folder.
- `examples/text_metrics.rs`: weight 400 already matched Chromium within 0.02px at every size; 500 was 1–5px narrower per label (Medium resolved to Regular), 600 slightly narrow, 700 fine. The "tracking" gap was the weight.
- Colour space, weights, smoothing and rounding fixed in GPUI. Single-line text cases are pixel-identical to Chromium; in Settings, text lines land where 0.9.4's do except where the boxes around them sit a fraction of a pixel off (ui-shell's layout).
- 2026-11-09, resumed after a stall: rebased on main (b63c4d9) for the parity harness. Wrapping finished: a clamped row is aligned by its wrapped width (Blink aligns, then truncates; a long word cut mid-word sat 5px off), centring in layout units (three paragraph and clamp cases went to 0 px), min-content probes answer with the widest unbreakable piece (two new flex cases, identical). Left on the case page: two clamps whose first row breaks inside a word, 1 and 13 anti-aliased px (GPUI measures that row inside the whole word's shaping, Blink shapes it apart).
- `spaced_text` sat a device pixel low (GPUI's old baseline); now Chromium's.
- Windows CI: the effects step had been skipped on every run since it was added, because `cargo test` failed first on `nodus-capture`'s prune test (`File::open` can't set a time or open a folder on Windows). Test fixed (checked with clippy for `x86_64-pc-windows-msvc`), the step runs unless cancelled. DXC 1.9.2609 compiles all 26 entry points (`-HV 2016 -WX`, SM 6.0). Whether fxc and WARP take them shows on the next run on main.
