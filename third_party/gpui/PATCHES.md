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
| `overflow-wrap: anywhere` (also `break-word`; default `normal`) | `.overflow_wrap(OverflowWrap::Anywhere)` |
| `word-break: break-all` (default `normal`) | `.word_break(WordBreak::BreakAll)` |

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

### 9. Where text sits: Chromium's layout units and rounding (Nodus, track gpui-text)

Chromium (Blink at device scale, with Skia) lays text out and rounds it differently from GPUI, which put some lines one device pixel (half a point) off and let others drift down a Settings dialog:

- **Line boxes keep their fractional height.** Blink keeps lengths in layout units (1/64 of a device pixel): a `line-height` of 18.2px is 36.390625 device pixels and lines stack that far apart; only what is painted is rounded. GPUI rounded the line height to a device pixel (18.2 → 18) and then rounded every measured text box *up* to a device pixel (18.2 → 18.5). `TextLayout` now keeps the line height in layout units (`layout_unit_floor`) and its width rounded up to layout units (`layout_unit_ceil`, Blink's `SnappedWidth`), and asks for a measured layout that isn't rounded (`Window::request_measured_layout_unsnapped`; other measured elements are unchanged).
- **Text paints from the unrounded origin.** GPUI rounds every box's edges to device pixels after layout; text now starts from the box's origin before that rounding (`Window::unrounded_layout_origin`, from the layout's unrounded absolute origins), as Blink paints text from its layout position.
- **The baseline is Blink's.** `chromium_baseline_offset`: the font's ascent and descent rounded to whole device pixels, the half-leading above the text floored (`CalculateLeadingSpace`), and the baseline rounded to a device pixel, halves up. GPUI centred the unrounded ascent and descent in the line and rounded halves toward zero. Fitted on Electron 44.5.1: the system font at 10.5–26px, eight line heights and four sub-pixel tops, 320 of 320 the same.
- **Glyph positions round as Skia's do**: to the nearest quarter device pixel horizontally and the nearest pixel vertically, halves up (`SkGlyphPositionRoundingSpec`), where GPUI rounded halves toward zero.

`crates/nodus-ui/examples/text_parity.rs` draws a page of text cases that `migration/tracks/gpui-text.md` also draws in Electron; every single-line case (sizes 11–26px, eight line heights, fractional tops) is pixel-identical. In the shell scenes, Settings' text lines land where 0.9.4's do; what is left is the boxes' own sub-pixel layout.

### 10. Wrapping, ellipses and min-content as CSS does them (Nodus, track gpui-text)

GPUI wrapped text its own way: a space at the end of a line could push it to wrap, a word too long for its line was always cut, centred rows counted their trailing spaces, its ellipsis dropped punctuation and was placed by its own rules, and a min-content probe got the whole line. Each now follows CSS as Blink implements it (`white-space: normal`):

- **Spaces hang.** A space at the end of a line never makes it wrap (`compute_wrap_boundaries` skips spaces when it measures), and the spaces at a row's end don't count when the row is centred or right-aligned (`RowAlign`).
- **`overflow-wrap` and `word-break`** are text styles: `Styled::overflow_wrap(OverflowWrap::Normal | Anywhere)` and `Styled::word_break(WordBreak::Normal | BreakAll)`, CSS's `normal` by default (upstream behaved like `anywhere`). With `normal`, a word longer than its line overflows it and the line breaks at the next opportunity; `Anywhere` (also CSS's `break-word`, and `word-break: break-word`) breaks it between any two characters when it doesn't fit alone; `BreakAll` may break between any two graphemes. The wrap cache keys on them and on the line limit. `WindowTextSystem::shape_text_wrapped` takes a `WrapStyle`; `shape_text` keeps upstream's signature with CSS's defaults.
- **The ellipsis is Chromium's** (`chromium_ellipsis` in `elements/text.rs`), for `text-overflow: ellipsis` on one line and for `-webkit-line-clamp` (`line_clamp` with `text_ellipsis`). The text is wrapped as usual; when it overflows, or runs past the last line shown, that line keeps what fits before `…`, in the style of the text it follows. For a clamp, the line first loses its trailing spaces and the ellipsis follows its last word when there's room; otherwise characters go from its end, mid-word, until there is. Nothing else is trimmed (GPUI dropped punctuation before the ellipsis).
- **Aligned rows use Blink's numbers.** A clamped last row is centred or right-aligned by the width it was wrapped at, before its end gave way to the ellipsis (`WrappedLine::last_row_align_width`); Blink aligns the line, then truncates it. Centring and right-aligning work in layout units: the row's width rounded up to one, the space left over halved by truncating (`aligned_origin_x`).
- **Min-content is the widest unbreakable piece.** A min-content probe of wrapping text (`AvailableSpace::MinContent`, no known width) wraps it at every opportunity and answers with its widest row (`WrappedLine::widest_row`), so a flex item with text shrinks to its longest word, or to a character with `Anywhere`, where it used to hold its whole line. The probe's layout isn't kept for painting.

On the case page (`examples/text_parity.rs` against Electron 44.5.1), wrapped paragraphs at three widths, centred and right-aligned, a path with `normal`, `anywhere` and `break-all`, six two-line clamps of folder names, a four-line clamp, six one-line ellipses and two min-content flex rows: 69 of 71 cases pixel-identical. The other two are clamps whose first row breaks inside a word: GPUI measures that row inside the whole word's shaping (the kerning across the break included), Blink shapes it apart, and two or three glyphs land in the next quarter-pixel bin (1 and 13 anti-aliased pixels).

### 11. Page zoom (Nodus, track audit-shell)

0.9.4's View › Zoom In, Zoom Out and Actual Size are Chromium's page zoom: the whole page drawn 1.2^level times larger, laid out in a viewport that much smaller. `Window::set_ui_zoom(zoom, cx)` does the same for a GPUI window (`Window::ui_zoom` reads it). The window's `scale_factor` becomes the display's times the zoom, so everything is drawn larger at full resolution, and `viewport_size` and the pointer are divided by it (`Window::read_platform_metrics`, used by `bounds_changed`). Input arrives in the platform's points and is divided by the zoom before dispatch (`PlatformInput::unzoomed`: mouse, scroll positions and pixel deltas, pinch, touch, file drops); the IME's character rectangles go back multiplied by it (`PlatformInputHandler::bounds_for_range`, `character_index_for_point`). At 1.0, the default, nothing changes. Window-control hit testing follows, as it reads the last pointer position.

### 12. Custom cursors (Nodus, track gpui-polish)

GPUI's `CursorStyle` had no `move`, no `cell` and no image cursors, which the ink layer needs (spec 11 INKUI-016/017: the pointer tool shows `move` over a stroke; the eraser is `cursor: url(<20×20 SVG circle>) 10 10, cell`).

- `CursorStyle::Move` (CSS `move`) and `CursorStyle::Cell` (CSS `cell`). macOS has no public `NSCursor` for either; Chromium (as WebKit before it) gets them from CoreCursor, an `NSCursor` subclass whose `_coreCursorType` names a system cursor (39 is the four-way move arrow, 41 the cell plus), and so does the patch (`gpui-pre-macos/src/cursor.rs`), so they are the same pictures 0.9.4 showed. Windows: `IDC_SIZEALL` for `move`, as Chromium; `IDC_CROSS` for `cell`.
- `CursorStyle::Custom(CustomCursor)`: `CustomCursor::register(CursorImage { size, hotspot, bitmaps, fallback })` registers an image once (bitmaps at one or more display scales, straight sRGB RGBA) and returns a `Copy` handle, so `CursorStyle` stays `Copy + Eq + Hash`. macOS makes an `NSCursor` over an `NSImage` holding every bitmap (AppKit picks the one for the display), sized in points with the hotspot from the top left, as Chromium's `CreateCustomCursor`; Windows makes an `HCURSOR` with `CreateIconIndirect` from the bitmap for the system DPI. Either is made the first time it's shown and kept. Where one can't be made, the image's `fallback` style shows.
- `ns_cursor` in `gpui-pre-macos/src/window.rs` is the old match in `reset_cursor_rects`, moved into a function so an image cursor can fall back to another style.
- `gpui_macos.rs` re-exports `MetalHeadlessRenderer` only with the `bench-support`/`test-support` features, not under `cfg(test)`, so `cargo test -p gpui-pre-macos --lib cursor` builds (gpui-apple has the type only with those features).

Linux and web (not vendored, not built for Nodus) match `CursorStyle` exhaustively and would need arms for the three new variants if they're ever built. Checked: `cargo test -p gpui-pre-macos --lib cursor` (the move and cell cursors exist and are 24×24 with the hotspot at 12,12; an image cursor keeps its size, hotspot and both bitmaps), nodus-ui's `ink::cursor` tests (the eraser's SVG at 1× and 2×), and `cargo check -p gpui-pre-windows --target x86_64-pc-windows-msvc`.

### 13. Highlights as CSS draws them: rounded marks, the underline's offset (Nodus, track gpui-polish)

GPUI filled a text run's background over the whole line height with square corners, and put underlines where it computes them from the line box. 0.9.4's highlights are inline elements: ⌘K's snippet marks and the transcript preview's are `mark { border-radius: 3px }`, the diff's changed words 2px, and a search title's matches are underlined `text-underline-offset: 3px` below the baseline (spec 11 SRCH-019/020). Two text styles, so nothing changes for text that doesn't ask:

- `Styled::inline_background_radius(px)` (`TextStyle::inline_background: Option<InlineBackground>`): runs' backgrounds cover the font's content area as Blink's inline boxes do (from Chromium's baseline, `chromium_baseline_offset`, the ascent above and the descent below, each rounded to device pixels) instead of the line height, as a quad with that corner radius. A run that wraps is rounded only where it starts and ends (CSS's default `box-decoration-break: slice`). `paint_run_background` in `text_system/line.rs`.
- `Styled::underline_offset(px)` (`TextStyle::underline_offset: Option<Pixels>`): an underline's top edge sits that far below the row's (Chromium) baseline.

`TextRun`, `HighlightStyle` and `UnderlineStyle` are unchanged: gpui-component builds them as struct literals, so new fields there would break it; the styles are read from the text style in effect when a line paints, and only when one of its runs has a background or an underline.

Checked on `search-results` against 0.9.4's golden, both themes: the snippet's mark covers the same 28 device rows (375–402) with the same rounded ends, and the title's underline the same two rows (364–365; the golden's underline is one CSS pixel thick, though `search.css` asks for 2px).

### 13. Shadows drawn only outside their box (Nodus, track gpui-polish)

CSS clips an outer `box-shadow` to outside the element's border box; GPUI drew the whole blurred rectangle under the element, so a see-through box (0.9.4's frosted menus, rail, prompt bar pill, toasts, selection bar, ink dock, the open archive panel, the capture panel's "N more" pill, a terminal's loading pill) showed its own shadow through its fill, and a backdrop blur blurred it into the panel. `Styled::shadow_outside()` (`Style::box_shadow_outside`) asks for CSS's behaviour: `Window::paint_drop_shadows_clipped(…, outside: true)` marks the `Shadow` primitives `inset: 2`, and the shadow fragment shader (Metal, HLSL, WGSL) multiplies the shadow's coverage by one minus the element's rounded-box coverage (`saturate(0.5 + sdf)`), with the same one-pixel anti-aliased edge as the box. `paint_drop_shadows` keeps its signature (unclipped) for other callers. Opt-in, because a Rust element may carry a shadow for a box drawn by a child with other corners; nodus-ui's `frosted()` sets it for every frosted surface. board-cards' sliced `outer_shadow` (the open archive panel) is now one element with it.

Checked against the goldens: `board-archive-open` (0.41% / 0.40% of the frame; the panel's edges and shadow show no differing pixels, what's left is card text), `capture-recents` (the "N more" pill: 0 pixels differ, both themes), `bar-focused` (the frosted pill: 0.02%).

## Verification

- `crates/nodus-ui/examples/effects.rs` (see `migration/tracks/gpui-fork.md`): frosted panels at blur 30/saturate 1.8 and 20/1.6 over moving cards, a note rotated 3–7° with CoreText-rendered text, a 3° sticker, the pad's sheet turning about its top edge, `group_opacity` beside `.opacity`, hit testing through the rotation (synthetic mouse moves; the window never takes focus), and a capture written to PNG. `EFFECTS_STRICT=1` exits 1 if a check fails; CI runs it on macOS and Windows.
- Metal (M4, 960×600 window at 2×): 60 fps in every mode. GPU time per frame, p50/p95: plain scene with effects 5.4/8.0 ms; stress (6 blurred panels, 23 rotated layers, 2 groups) 7.7/8.8 ms against 4.3/5.3 ms for the same scene with every effect off. CPU per frame (render to end of paint) under 2 ms. Each transform or group layer costs roughly 0.1 ms of GPU at this size (a frame-sized texture clear and store); a region-sized layer texture would need every fragment shader to take an origin offset, which is the next step if boards ever need dozens of them at once.
- wgpu: the effect shaders validate with naga, and a scene with a frosted panel and a rotated, faded note renders correctly through `WgpuHeadlessRenderer` on wgpu's Metal backend.
- DirectX 11: `cargo clippy --target x86_64-pc-windows-msvc` is clean (on macOS, gpui's manifest resource needs an `llvm-rc`; a stand-in that writes an empty file is enough for a check). **fxc hasn't compiled the HLSL yet, nor has it run**: fxc runs only in release builds on Windows (debug builds compile it at runtime with `D3DCompileFromFile`). Every entry point of `shaders.hlsl` and `color_text_raster.hlsl` (26) compiles with DXC 1.9.2609 (`-HV 2016 -WX`, shader model 6.0, its Linux build in an amd64 container), which catches syntax and type errors though not fxc's shader-model-4.1 limits. The CI step builds the example in release on `windows-latest`, which compiles every shader with fxc, then runs it on the runner's adapter (WARP when there's no GPU); until 2026-11-09 it never ran there, as a failing test stopped the job first, so it now runs unless the run is cancelled. It is required on macOS and `continue-on-error` on Windows until it's been seen green. Things to look at if it fails: the `TransformationMatrix` read in `layer_point` follows the sprites' `mul(position, rotation_scale)` convention, and `t1` holds two `StructuredBuffer`s as the rest of the file does.

## Re-applying on a newer snapshot

1. Copy the new `gpui-pre*-<version>` crates from `~/.cargo/registry/src/*/` into `third_party/gpui/`, unchanged, for the five crates above; commit.
2. Re-apply the patch set: `git diff <old vendor commit> <last patch commit> -- third_party/gpui | git apply -3` usually goes through, since gpui-pre republishes Zed's sources nearly verbatim. Where it doesn't, resolve by hand; the "Nodus patch" comments mark every hunk.
3. Copy the six shader-facing sources into `gpui-pre-apple/vendor/gpui/src/`.
4. Build the example, run it (`EFFECTS_STRICT=1`), look at the capture, and run the Metal pipeline check if `shaders.metal` changed: compile `target/*/build/gpui-pre-apple-*/out/stitched_shaders.metal` with `MTLDevice.makeLibrary(source:)` and build every pipeline (a 40-line Swift harness).
5. Check `cargo tree -d | grep gpui` is empty, then the workspace's fmt, clippy (also `--target x86_64-pc-windows-msvc`) and tests.

If Zed merges effect layers upstream, drop patch 1 and keep 3–5 on top of theirs.
