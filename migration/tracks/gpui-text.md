# Track: gpui-text

- **Branch:** `rust/gpui-text`
- **Scope:** text drawn by the patched GPUI (`third_party/gpui/`, D13) indistinguishable from what Chromium drew in 0.9.4: colour space, SF Pro tracking, wrapping (hanging spaces, `overflow-wrap`, `word-break`, `-webkit-line-clamp`), baseline and glyph rounding, anti-aliasing; nodus-ui's workarounds removed. Also the Windows `effects` CI step.
- **Reference:** the goldens (`migration/parity/golden/`, from `rust/parity-harness`), and Electron 44.5.1 (0.9.4's runtime) measuring and drawing test pages headless (no Dock icon, no focus).

## Status

| Item | State |
| --- | --- |
| 1. sRGB-tagged layer inside GPUI (Metal; DirectX / wgpu checked) | ✅ PATCHES 6; nodus-ui's tagging removed |
| 2. Widths at every size (the "tracking" gap) | ✅ PATCHES 7: the cause was weight 500 → Regular; tracking matches (CoreText's `trak` is what Chromium applies too) |
| 3. Hanging trailing spaces; `overflow-wrap`, `word-break`, line clamp | ☐ |
| 4. Chromium's baseline and glyph rounding | ☐ |
| 5. Anti-aliasing and font smoothing | ☐ |
| Scene diffs before / after | ☐ |
| Windows `effects` CI step | ☐ |

## Log

- Started: worktree on `main` (83c9c0d); debug build of `nodus`; Electron measuring page set up in the scratch folder.
- WIP: `crates/nodus-ui/examples/text_metrics.rs` compares GPUI's widths with Chromium's (Electron 44.5.1). First run: weight 400 already matches within 0.02px at 10.5–15px; weight 500 is 1–5px narrower per label (GPUI resolves Medium to Regular), 600 is ~0.15px narrower; 700 matches. So the 13px labels' gap is the weight, not (only) tracking; checking the larger sizes next. "Before" scene shots taken.
