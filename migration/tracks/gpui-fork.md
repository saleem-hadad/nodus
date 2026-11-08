# Track: gpui-fork (D13)

Branch `rust/gpui-fork`. Our patched GPUI: the `gpui-pre*` crates we change, vendored under
`third_party/gpui/` and swapped in with `[patch.crates-io]` in the root `Cargo.toml`. What each
patch does, where it comes from, the API, its rules and how to re-apply it on a newer snapshot:
`third_party/gpui/PATCHES.md`.

## Status

| # | Deliverable | Status |
| --- | --- | --- |
| 1 | Vendor `gpui-pre`, `-apple`, `-macos`, `-wgpu`, `-windows` 0.3.11 unmodified, wire `[patch.crates-io]`, one GPUI in the graph | done |
| 2 | Backdrop blur: effect-layers stack (zed#63773, #63777, #63778, #63779, #63780; #63794 opt-in) on the snapshot, Metal crash fixed at its cause | done on Metal (verified) and wgpu (verified headless on wgpu-Metal); DirectX 11 ported and cross-clippy clean, HLSL not yet compiled |
| 3 | Transforms (rotate, scale, translate, origin) through layers, hit testing through the inverse | done (same three backends) |
| 4 | Group opacity | done (same three backends) |
| 5 | Window capture: `Window::render_to_image`, ungated | Metal and DirectX 11; wgpu returns an error |
| 6 | Proof: `crates/nodus-ui/examples/effects.rs`, CI step on macOS and Windows | done; CI step informational until seen green |

For ui-shell: `.backdrop_blur(px(30.)).backdrop_saturate(1.8)`, `.rotate(..)`, `.scale(..)`,
`.translate(..)`, `.transform_origin(..)`, `.group_opacity(..)`, `.clip_rounded()` on any `Styled`;
`window.render_to_image()` for captures. The table in PATCHES.md maps each to its CSS.

## How to verify

```sh
cargo run -p nodus-ui --example effects --release            # EFFECTS HIT/CAPTURE/FRAMES lines
EFFECTS_STRESS=1 cargo run -p nodus-ui --example effects --release
EFFECTS_STRICT=1 EFFECTS_CAPTURE=<scratch>/effects.png cargo run -p nodus-ui --example effects --release
```

The window is a non-activating pop-up in the bottom-right corner of the main display; it never
takes focus and quits by itself after `EFFECTS_SECONDS` (8). Measured on the M4 (PATCHES.md,
"Verification"): 60 fps in every mode, GPU p50 7.7 ms in stress mode.

## Open

- Windows: see the CI step "Effects example" on `windows-latest` go green (it compiles the HLSL
  with fxc and draws through DirectX 11 / WARP), then drop its `continue-on-error`. Cross-clippy
  from macOS needs `RC=<an llvm-rc or a stand-in that writes an empty file>`.
- wgpu `render_to_image` (Linux only; not needed for Nodus).
- If boards ever need dozens of transformed layers at once: region-sized layer textures (every
  fragment shader takes an origin offset) instead of frame-sized ones.
- Feedback to Apple with the reduced crashing shader (PATCHES.md, patch 2).

## Resuming

- The port was done in a sparse zed worktree at the snapshot (`<scratch>/gf/zed-port`, branch
  `nodus-port`: `zed@f816e1f` + gpui-pre's publish edits + the stack squashed and resolved), then
  copied into `third_party/gpui/*/src`. From here on the vendored crates are the source of truth.
- After editing `gpui-pre/src/{scene,effect_layer,geometry,color,window,platform}.rs`, copy them to
  `gpui-pre-apple/vendor/gpui/src/` (its build script runs cbindgen on that copy).
- The vendored crates aren't workspace members: lint them by copying `third_party/gpui` into a
  scratch workspace that lists them as members; format with `rustfmt --edition 2024 <file>`.
