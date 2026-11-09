# Track: gpui-text

- **Branch:** `rust/gpui-text`
- **Scope:** text drawn by the patched GPUI (`third_party/gpui/`, D13) indistinguishable from what Chromium drew in 0.9.4: colour space, SF Pro tracking, wrapping (hanging spaces, `overflow-wrap`, `word-break`, `-webkit-line-clamp`), baseline and glyph rounding, anti-aliasing; nodus-ui's workarounds removed. Also the Windows `effects` CI step.
- **Reference:** the goldens (`migration/parity/golden/`, from `rust/parity-harness`), and Electron 44.5.1 (0.9.4's runtime) measuring and drawing test pages headless (no Dock icon, no focus).

## Status

| Item | State |
| --- | --- |
| 1. sRGB-tagged layer inside GPUI (Metal; DirectX / wgpu checked) | ☐ |
| 2. No `trak` tracking (Chromium's HarfBuzz widths at every size) | ☐ |
| 3. Hanging trailing spaces; `overflow-wrap`, `word-break`, line clamp | ☐ |
| 4. Chromium's baseline and glyph rounding | ☐ |
| 5. Anti-aliasing and font smoothing | ☐ |
| Scene diffs before / after | ☐ |
| Windows `effects` CI step | ☐ |

## Log

- Started: worktree on `main` (83c9c0d); debug build of `nodus`; Electron measuring page set up in the scratch folder.
