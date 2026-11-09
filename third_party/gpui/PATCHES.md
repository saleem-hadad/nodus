# Our patched GPUI (decision D13)

Stock GPUI can't blur what is behind an element, can't rotate or scale an element, and fades a subtree primitive by primitive. Nodus needs all three everywhere (spec 08, 10, 11), so we keep a small patch set on top of the GPUI that `gpui-kit =0.9.0` uses.

## What is here

| Directory | Crate (crates.io) | Zed crate | Changed |
| --- | --- | --- | --- |
| `gpui-pre/` | `gpui-pre 0.3.11` | `crates/gpui` | scene, style, window, hit testing, the API |
| `gpui-pre-apple/` | `gpui-pre-apple 0.3.11` | `crates/gpui_apple` | Metal renderer and shaders |
| `gpui-pre-macos/` | `gpui-pre-macos 0.3.11` | `crates/gpui_macos` | `render_to_image` in every build |
| `gpui-pre-windows/` | `gpui-pre-windows 0.3.11` | `crates/gpui_windows` | DirectX 11 renderer and HLSL |
| `gpui-pre-wgpu/` | `gpui-pre-wgpu 0.3.11` | `crates/gpui_wgpu` | wgpu renderer and WGSL (Linux, web) |

`gpui-pre 0.3.11` is the gpui-component maintainer's republish of `zed@f816e1f9263e3a8ef63c4baaa02e1441ce1c6038` (2026-11-01), with three small publish-time edits (marked "Modified for gpui-pre" at the top of the file). Each crate was first committed exactly as published (commit "Vendor the five gpui-pre 0.3.11 crates we patch, as published"), so `git diff <that commit> -- third_party/gpui` is the whole patch set. Every file we change says so in a comment ("Nodus patch").

The root `Cargo.toml` swaps them in with `[patch.crates-io]`, keeping their names and version, so `gpui-kit` and `gpui-component` resolve to them and the graph holds one GPUI (`cargo tree -d | grep gpui` is empty). They are excluded from the workspace (`exclude = ["third_party"]`), so `cargo fmt --all` and `cargo clippy --workspace` leave them alone; they keep upstream's own `[lints]`. `rustfmt.toml` here is Zed's, for formatting edits by hand (`rustfmt --edition 2024 <file>`). With our toolchain, clippy finds one pre-existing upstream error (`approx_constant` on `phi()` in `geometry.rs`); it only matters if someone lints the crates directly.

`gpui-pre-apple/vendor/gpui/src/` is a copy of gpui-pre's `scene.rs`, `effect_layer.rs`, `geometry.rs`, `color.rs`, `window.rs` and `platform.rs`: its build script runs cbindgen on them to generate the Metal header. **After changing any of those six files in `gpui-pre/src`, copy them there too.**

The other `gpui-pre-*` crates (platform, util, collections, …) come from crates.io unchanged.

## The API

All of these are on `Styled`, so every `div()` has them, and map 1:1 to CSS:

| CSS | GPUI |
| --- | --- |
| `backdrop-filter: blur(30px) saturate(1.8)` | `.backdrop_blur(px(30.)).backdrop_saturate(1.8)` |
| `backdrop-filter: blur(20px) brightness(1.1) saturate(1.6)` | `.backdrop_blur(px(20.)).backdrop_matrix(ColorMatrix::brightness(1.1).then(&ColorMatrix::saturate(1.6)))` |
| `filter: blur(4px)`, `filter: grayscale(1)` | `.blur(px(4.))`, `.color_matrix(ColorMatrix::grayscale(1.))` |
| `mask-image: linear-gradient(…)` | `.mask(linear_gradient(…))` |
| `mix-blend-mode: multiply` | `.blend_mode(BlendMode::Multiply)` |
| `transform: rotate(3deg)` | `.rotate(radians(3f32.to_radians()))` |
| `transform: translate(4px, 0) rotate(-3deg) scale(1.02)` | `.translate(px(4.), px(0.)).rotate(…).scale(1.02)` (CSS order) |
| `transform-origin: 50% 0` | `.transform_origin(0.5, 0.)` (default: the centre) |
| any `transform` | `.transform(ElementTransform::default().matrix([a, b, c, d, e, f]).origin_px(x, y))` |
| `opacity: 0.5` on a parent (one picture) | `.group_opacity(0.5)` |
| `overflow: hidden` + `border-radius` clipping children to the arcs | `.overflow_hidden().rounded(…).clip_rounded()` |

`ColorMatrix` has the CSS filter functions: `saturate`, `grayscale`, `sepia`, `brightness`, `contrast`, `invert`, `opacity`, `hue_rotate`; `a.then(&b)` is `a b` in a filter list.

For custom elements: `Window::paint_effect_layer(bounds, corner_radii, &LayerEffects, |window| …)` in paint, and `Window::with_element_transform(bounds, Some(&transform), |window| …)` around the same subtree in prepaint (`div` does both).

Window capture: `Window::render_to_image() -> anyhow::Result<image::RgbaImage>`, in every build (upstream gates it behind `test-support`). It draws the last frame's scene again into an offscreen texture of the drawable's size (device pixels: 2× on Retina) and reads it back; no Screen Recording permission, nothing to pay until it's called (10–30 ms when called, for a 1920×1200 frame). Metal and DirectX 11; wgpu returns an error. It holds only what GPUI draws: not the traffic lights, not a wry webview.

### Rules worth knowing

- **Layout ignores transforms**, as in CSS. The picture moves; the box doesn't.
- **Hit testing follows transforms**, as in CSS: a hitbox inserted under a transform is tested against the pointer mapped back through the inverse (nested transforms compose), and the content masks outside the transform still clip it. What doesn't follow: positions handed to mouse listeners stay in window coordinates, and code that checks `bounds.contains(window.mouse_position())` itself (tooltips, drag sources) tests the untransformed box. Map the position with `ElementTransform::to_matrix(bounds).inverse()` where that matters.
- **Under a transform the content draws untransformed** into a frame-sized texture, then the layer moves it. Content mask clipping happens after the move, but whatever the untransformed content has outside the window isn't drawn. Text and images are rasterized at 1× the device scale before the transform, so a scale above 1 softens them (Chromium does the same for animated transforms). Edges are anti-aliased: the layer reads two clear pixels around what it painted and samples between pixels.
- **`group_opacity` costs a layer only while the opacity is below 1.** `.opacity()` stays GPUI's per-primitive fade, which is free; use it for leaves where nothing overlaps.
- **A layer reads only what its content painted** (the scene tracks the painted extent, shadows included, so a rotated note keeps its shadow and overflowing children).
- **The backdrop blur is clipped to the border box and mirrored at its edges**, which is what Chromium does for `backdrop-filter`: colour from outside the panel doesn't bleed in.
- **`clip_rounded` is opt-in.** zed#63794 gives every rounded box with `overflow: hidden` a layer; gpui-component rounds and clips nearly every control, and each layer costs a frame-sized offscreen pass, so we ask for it where a child reaches into a corner.

## The patches

### 1. Effect layers (zed#63773, #63777, #63778, #63779, #63780; #63794 made opt-in)

Origin: the "effect layers" PR stack by mateo-m, closed unmerged on 2026-09-04, head `e343ea6471ce2afdae5603702282301acd2eff70` on `zed@ba6b83f`. #63773 is the core (scene `LayerBegin`/`LayerEnd` batches, `LayerEffects` in `Style`, `Window::paint_effect_layer`, `BlurPlan`), #63777 Metal, #63778 wgpu, #63779 DirectX 11, #63794 the rounded overflow clip, #63780 the variable blur under a gradient mask.

Why: every `backdrop-filter` in Nodus (prompt bar, mic, menus, toasts, selection bar, ink dock, modal backdrops, archive panel) needs a blur of what GPUI already drew behind the element. The stack renders a subtree into its own texture, blurs a downscaled copy of what's under it with two separable Gaussian passes (cost roughly independent of radius), and composites inside the rounded box.

How it was ported: the stack also carries three prerequisite PRs (#63771 isolated layout, #63792 corner shapes, #63793 multi-stop gradients), which we left out; the effect layers' `corner_shapes` went with them, and their shaders use this snapshot's two-stop fills (`prepare_fill_color`/`fill_color` in Metal, `prepare_gradient_color`/`gradient_color` in HLSL and WGSL), with the mask's colours prepared in the vertex shader like a quad's. The snapshot is ~890 commits past the stack's base, and in between the Metal renderer moved from metal-rs to objc2-metal, so `metal_effects.rs` is rewritten for objc2-metal. `background_image` and `background_blend_mode` (also from #63773) came along unchanged. #63781 (drop-shadow filter) is not ported.

Changes we made to the stack:

- A layer reads only the pixels its content painted (`EffectLayer::painted`, filled in by `Scene::pop_effect_layer`), not the box grown by three sigmas, so overflow and shadows survive and small layers stay cheap.
- Layers that read nothing under them (no backdrop, mask or blend mode: plain transforms, group opacity, clips) composite with hardware premultiplied blending and skip the copy of the target.
- The backdrop blur reads only the border box and mirrors at its edges (`BlurParams::edge`), as Chromium does; upstream blurred a padded region with clamped edges, which pulled outside colour into the panel.
- `framebufferOnly` is off on the `CAMetalLayer` in every build (upstream: test builds only), because a backdrop copies the frame out of the drawable.
- The overflow clip of #63794 is opt-in (`clip_rounded`), see above.

### 2. The Metal compiler crash (macOS 27.2, M4)

Symptom: building the `layer_composite` and `variable_blur` pipelines killed `MTLCompilerService` (`XPC_ERROR_CONNECTION_INTERRUPTED`, a SIGSEGV in its LLVM), so GPUI couldn't start. The spike's workaround dropped masks and blend modes.

Root cause, found by compiling variants of the upstream shader with a harness that builds each pipeline from source the way `runtime_shaders` does: the AGX backend compiler crashes when a fragment function that reads a `Background` from a `constant` struct passes it **by value** into the multi-stop gradient code of #63793 (`linear_gradient_color` → `gradient_stop_color`), which walks the colour stops with a data-dependent index (`while (i + 1 < last && t > colors[i + 1].percentage) i++;`, `last` from `stop_count`). Removing just that loop, or making `last` a constant, compiles; so does passing the `Background` as `thread const Background &`. Masks and the 17 blend modes are innocent: with the gradient code present, the composite crashes even without them, and compiles with them once the gradient is fixed.

Fix: our port doesn't take #63793, so the fills the effect shaders call are this snapshot's two-stop ones, which index the stops with constants and get their colours from the vertex stage; both pipelines build (35 ms and 22 ms), with masks, variable blur and all blend modes intact. **If multi-stop gradients are ported later, pass `Background` by `thread const &` to the functions that index its stops** (the shader comment says so too). Worth a Feedback to Apple with the reduced shader.

### 3. Transforms (Nodus)

`LayerEffects::transform: Option<ElementTransform>` (CSS `matrix(a, b, c, d, e, f)` plus a `transform-origin` as fractions and pixels), set by `Styled::transform/rotate/scale/translate/transform_origin`. Paint draws the subtree untransformed into the layer (with a fresh content mask, since the outer masks mean nothing in the layer's space) and the composite maps each target pixel through the inverse into the layer's texture, sampling bilinearly; the box clip, mask and backdrop are evaluated in the layer's space, so a rotated frosted panel blurs inside its turned outline (the stress mode of the example has one). `EffectLayer` carries the device-pixel transform; `LayerComposite` the inverse and the anti-aliasing scale. Prepaint wraps the subtree in `Window::with_element_transform`, which records an `Arc<HitTransform>` chain for each hitbox inserted there; `Frame::hit_test` maps the pointer through it (`hitbox_transforms`, kept in step with `hitboxes` through prepaint reuse and `transact`). `TransformationMatrix` gains `inverse`, `determinant` and `apply_f32`.

### 4. Group opacity (Nodus)

`LayerEffects::group`, set with the opacity by `Styled::group_opacity`. When it's set and the element's opacity is below 1, the element paints through a layer whose composite applies the opacity once (the content inside paints at full opacity), so overlapping children don't show through each other. Combined with a transform it's one layer.

### 5. Window capture (Nodus)

`Window::render_to_image` and the `PlatformWindow::render_to_image` it calls existed upstream behind `test-support`; the patch removes the gates in gpui-pre, gpui-pre-apple, gpui-pre-macos and gpui-pre-windows and documents the method. The renderer's offscreen path goes through the same effect layers.

### 6. sRGB colour space (Nodus, track gpui-text)

GPUI's colours are sRGB values, but upstream left the window's `CAMetalLayer` untagged unless a window asks for Display P3, so on a wide-gamut display the window server showed them unconverted and every saturated colour came out more vivid than the same CSS colour in Chromium (`#007aff` reached a P3 screen as `#007aff` instead of Chromium's `#3579fa`). `MetalRenderer::configure_layer` now tags the layer `kCGColorSpaceSRGB` (still Display P3 when `use_display_p3` is set), so macOS converts for the display as it does for a browser. DirectX: a flip-model `B8G8R8A8_UNORM` swap chain is already sRGB (`DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709`); `tag_swap_chain_srgb` sets it explicitly through `IDXGISwapChain3::SetColorSpace1` so Windows' colour management treats it as a browser's. wgpu has no colour-space API: it picks a non-sRGB surface format, so the values reach the compositor as sRGB, which is what Linux compositors and a web canvas assume.

### 7. The system font's weights (Nodus, track gpui-text)

GPUI picked a weight of the system font (`.SystemUIFont`) among the faces font-kit lists for its family, and the family has no Medium: 500 came out Regular, so every 13px label set at 500 was 1–1.5px narrower and lighter than Chromium's, and 600 landed on a face lighter than Chromium's. `AppleTextSystem::font_id` now makes the system font (`.SystemUIFont`, `.AppleSystemUIFont`, `system-ui`, `-apple-system`, `BlinkMacSystemFont`) the way Blink's `MatchSystemUIFont` does: `CTFontCreateUIFontForLanguage(kCTFontUIFontSystem)`, the bold trait from 600 and the italic trait when slanted, then the CSS weight set exactly on the `wght` axis (clamped to the axis), one font per weight and style, with the run's features and fallbacks as before. Variable instances share a PostScript name (400 and 500 are both `.SFNS-Regular`), so the map from a laid-out run's font back to a font id is keyed by the name plus the variation, leaving out the optical size (CoreText sets it from the point size); otherwise a 500 run would come back as Regular and be drawn at 400.

Tracking needed no change: CoreText applies SF's size-specific tracking (`trak`), and so does Chromium on a Mac (its advances come from CoreText), so with the weights right the widths match at every size the app uses. Checked with `crates/nodus-ui/examples/text_metrics.rs` against Electron 44.5.1 (0.9.4's Chromium) at 10.5, 11, 12, 12.5, 13, 14, 15, 17, 20 and 26px in weights 400–700: the largest difference over a 43-character line is 0.065px (Chromium's widths are in 1/64px units). `crates/nodus-ui/tests/text_parity.rs` checks that each weight is a font of its own and is drawn at its weight.

### 8. Font smoothing (Nodus, track gpui-text)

0.9.4's `global.css` sets `-webkit-font-smoothing: antialiased`, so Chromium drew every glyph with plain grayscale coverage. GPUI asks CoreGraphics for font smoothing whenever the user allows it, which thickens the strokes by an amount that depends on the text colour: labels came out visibly bolder (15% more ink on light, 29% on dark). `App::set_font_smoothing(FontSmoothing::Antialiased)` turns it off for every window (`FontSmoothing::PlatformDefault` keeps upstream's behaviour), and the rasterizer then sets `shouldSmoothFonts` off explicitly. nodus-ui sets it in `theme::fonts::register`. Anti-aliasing stays grayscale, as Chromium's is on macOS (no LCD text since 10.14). With it, a label's ink is within 0.1% of Chromium's in both themes, and lines drawn at the same position are pixel-identical (`examples/text_parity.rs` against Electron 44.5.1). Windows has no smoothing to turn off; its subpixel or grayscale choice is unchanged.

## Verification

- `crates/nodus-ui/examples/effects.rs` (see `migration/tracks/gpui-fork.md`): frosted panels at blur 30/saturate 1.8 and 20/1.6 over moving cards, a note rotated 3–7° with CoreText-rendered text, a 3° sticker, the pad's sheet turning about its top edge, `group_opacity` beside `.opacity`, hit testing through the rotation (synthetic mouse moves; the window never takes focus), and a capture written to PNG. `EFFECTS_STRICT=1` exits 1 if a check fails; CI runs it on macOS and Windows.
- Metal (M4, 960×600 window at 2×): 60 fps in every mode. GPU time per frame, p50/p95: plain scene with effects 5.4/8.0 ms; stress (6 blurred panels, 23 rotated layers, 2 groups) 7.7/8.8 ms against 4.3/5.3 ms for the same scene with every effect off. CPU per frame (render to end of paint) under 2 ms. Each transform or group layer costs roughly 0.1 ms of GPU at this size (a frame-sized texture clear and store); a region-sized layer texture would need every fragment shader to take an origin offset, which is the next step if boards ever need dozens of them at once.
- wgpu: the effect shaders validate with naga, and a scene with a frosted panel and a rotated, faded note renders correctly through `WgpuHeadlessRenderer` on wgpu's Metal backend.
- DirectX 11: `cargo clippy --target x86_64-pc-windows-msvc` is clean (on macOS, gpui's manifest resource needs an `llvm-rc`; a stand-in that writes an empty file is enough for a check). **The HLSL hasn't been compiled or run yet**: fxc runs only in release builds on Windows (debug builds compile it at runtime with `D3DCompileFromFile`). The CI step builds the example in release on `windows-latest`, which compiles every shader with fxc, then runs it on the runner's adapter (WARP when there's no GPU); it is `continue-on-error` until it's been seen green. Things to look at if it fails: the `TransformationMatrix` read in `layer_point` follows the sprites' `mul(position, rotation_scale)` convention, and `t1` holds two `StructuredBuffer`s as the rest of the file does.

## Re-applying on a newer snapshot

1. Copy the new `gpui-pre*-<version>` crates from `~/.cargo/registry/src/*/` into `third_party/gpui/`, unchanged, for the five crates above; commit.
2. Re-apply the patch set: `git diff <old vendor commit> <last patch commit> -- third_party/gpui | git apply -3` usually goes through, since gpui-pre republishes Zed's sources nearly verbatim. Where it doesn't, resolve by hand; the "Nodus patch" comments mark every hunk.
3. Copy the six shader-facing sources into `gpui-pre-apple/vendor/gpui/src/`.
4. Build the example, run it (`EFFECTS_STRICT=1`), look at the capture, and run the Metal pipeline check if `shaders.metal` changed: compile `target/*/build/gpui-pre-apple-*/out/stitched_shaders.metal` with `MTLDevice.makeLibrary(source:)` and build every pipeline (a 40-line Swift harness).
5. Check `cargo tree -d | grep gpui` is empty, then the workspace's fmt, clippy (also `--target x86_64-pc-windows-msvc`) and tests.

If Zed merges effect layers upstream, drop patch 1 and keep 3–5 on top of theirs.
