# Track: gpui-fork (D13)

Branch `rust/gpui-fork`. Our patched GPUI: the `gpui-pre*` crates we change, vendored under
`third_party/gpui/` and swapped in with `[patch.crates-io]` in the root `Cargo.toml`. What each
patch does, where it comes from and how to re-apply it: `third_party/gpui/PATCHES.md`.

## Status

| # | Deliverable | Status |
| --- | --- | --- |
| 1 | Vendor `gpui-pre`, `-apple`, `-macos`, `-wgpu`, `-windows` 0.3.11 unmodified, wire `[patch.crates-io]`, one GPUI in the graph | done (commits "Vendor the five…", "Build GPUI from the vendored crates…") |
| 2 | Backdrop blur: effect-layers stack (zed#63773, #63777, #63780; #63794 opt-in) on the snapshot, Metal crash fixed | Metal done; DirectX 11 and wgpu next |
| 3 | Transforms (rotate, scale, translate, origin) through layers, hit testing through the inverse | Metal done |
| 4 | Group opacity | Metal done |
| 5 | Window capture (`Window::render_to_image`, ungated) | Metal done; DirectX 11 next |
| 6 | Proof: `crates/nodus-ui/examples/effects.rs` | runs; numbers below |

## How to verify

```sh
cargo run -p nodus-ui --example effects --release            # prints EFFECTS HIT/CAPTURE/FRAMES lines
EFFECTS_STRESS=1 cargo run -p nodus-ui --example effects --release
EFFECTS_CAPTURE=<scratch>/effects.png cargo run -p nodus-ui --example effects --release
```

The window is a non-activating pop-up in the bottom-right corner of the main display; it never
takes focus and quits by itself after `EFFECTS_SECONDS` (8).

## Resuming

- The port was done in a sparse zed worktree at the snapshot (`<scratch>/gf/zed-port`, branch
  `nodus-port`: `zed@f816e1f` + gpui-pre's publish edits + the stack squashed and resolved), then
  copied into `third_party/gpui/*/src`. From here on the vendored crates are the source of truth.
- After editing `gpui-pre/src/{scene,effect_layer,geometry,color,window,platform}.rs`, copy them to
  `gpui-pre-apple/vendor/gpui/src/` (its build script runs cbindgen on that copy).
- The Metal pipeline check: compile the stitched shader in
  `target/*/build/gpui-pre-apple-*/out/stitched_shaders.metal` and build every pipeline (a small
  Swift harness did this; see PATCHES.md, "Metal compiler crash").
