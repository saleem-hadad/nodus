# S1: UI stack spike (GPUI + gpui-component, alacritty_terminal, portable-pty, wry, floating panel)

Status: **done** (2026-11-08). Machine: Apple M4 MacBook (built-in 60 Hz Retina, scale 2), macOS 27.2, Xcode 16.4 (SDK 15.5), Rust 1.99.1.
Source of the spike app: `migration/spikes/s1-src/` (the backdrop-blur proof is in `s1-src/blur/`).

## Verdict

**D1 (GPUI + gpui-component): adopt, but on a GPUI we patch.** Upstream GPUI covers the window, theme, canvas, terminal element, webview embedding and floating panel with no forks (steps 1–6 all pass on stock crates). It does **not** cover two must-haves from the specs: rotated elements (step 7) and region backdrop blur (step 8). Both exist as unmerged/closed upstream work that builds and runs here; owning a small patch set on top of the weekly GPUI snapshot is the price.

**D2 (alacritty_terminal + own GPUI element): adopt.** `ls --color`, 256/truecolor SGR, wide CJK, `top`, `vim`, ctrl-C, resize, a 3 MB output flood at 60 fps, all pass.

**D3 (wry child webview): adopt, with the snapshot swap for z-order.** A native view always sits above GPUI's Metal layer. Swapping the live view for a GPUI-drawn snapshot (2–16 ms per capture, typically 2–6) whenever anything overlaps it, it's dragged, or an ink tool is out works; clipping, rounded corners, pointer pass-through, page zoom and transient user activation all verified.

### Pin these

| Crate | Version / rev | Notes |
| --- | --- | --- |
| `gpui-kit` | `=0.9.0` | Facade: re-exports GPUI (`gpui-pre =0.3.11`), `gpui_platform` (features `font-kit`, `runtime_shaders`, set by gpui-kit), `gpui-base 0.9.0`, `gpui-component 0.9.0`, `gpui-kit-assets` (Lucide icons). Use `extern crate gpui_kit as gpui;` |
| `gpui-pre` | `=0.3.11` | Weekly republish of Zed's GPUI by the gpui-component maintainer: snapshot of `zed@f816e1f` (2026-11-01). gpui-component pins it exactly. |
| `alacritty_terminal` | `=0.26.0` (vte 0.15.0) | Apache-2.0 |
| `portable-pty` | `=0.9.0` | |
| `wry` | `=0.57.1` | upstream, not `lb-wry` (see below) |
| `raw-window-handle` | `0.6.2` | |
| `objc2` / `objc2-app-kit` / `objc2-foundation` / `block2` | `0.6.5` / `0.3.2` / `0.3.2` / `0.6.2` | app-kit features used: NSWindow NSView NSPanel NSResponder NSApplication NSScreen NSGraphics NSRunningApplication NSEvent NSImage NSImageRep NSBitmapImageRep |
| `image` | `0.25` (`png` only for debugging) | snapshot → `RenderImage` |

**Don't use:** crates.io `gpui 0.2.2` (Oct 2025, a year stale). **Don't depend on zed git directly** while using gpui-component: gpui-component requires `gpui-pre =0.3.11`, so a git `gpui` would put two GPUIs in the graph. If we need a newer or patched GPUI, `[patch.crates-io]` the `gpui-pre*` family with a fork that keeps those package names and version (see "Patch set" below).

**Not used:** `gpui-webview 0.9.0` (longbridge) — it wraps `lb-wry 0.53.3`, a one-off 2025 fork of wry; our ~150-line `webview.rs` on upstream wry 0.57.1 does the same plus clipping/rounding/pass-through/snapshots. `gpui-fast` (longbridge's GPUI fork, 0.1.13) — has native "window composition" (ported from the open upstream PR zed#62379) that would put GPUI overlays above a WKWebView, but it's a whole-GPUI fork; the snapshot swap gets the same UX on stock GPUI.

## Results

| Step | Result |
| --- | --- |
| 1 Window, title bar, theme, gpui-component | **PASS** |
| 2 Canvas: pan, pinch/ctrl-wheel zoom at pointer, drag, rubber band, smoothness | **PASS** (60 fps = display refresh) |
| 3 Terminal (PTY + alacritty + own element), in a zoomed card and a panel | **PASS** |
| 4 wry webview child, custom protocol, IPC, z-order, clip, zoom, sandbox, user activation | **PASS** with the snapshot swap |
| 5 Floating panel (NSPanel, level 101, all Spaces + full-screen aux, key, hide on blur) | **PASS** (see notes: over a real full-screen app not exercised) |
| 6 Build time, size, CPU, memory | measured, below |
| 7 Rotated elements | **PASS on stock GPUI via SVG layers** (limited fidelity); full fidelity needs a GPUI patch |
| 8 Region backdrop blur | **PASS on a patched GPUI** (closed upstream PR stack); **FAIL on stock GPUI** |

How it was verified: the app has an autopilot (`S1_AUTOPILOT=1`) that drives **synthetic** input through `Window::dispatch_event` / `dispatch_keystroke` (and, for the webview, `-[NSWindow sendEvent:]`), asserts state, and screenshots only the app's own window with a ScreenCaptureKit single-window helper. It never moves the real pointer and never activates the app (`S1_NO_ACTIVATE=1`, Stage-Manager-exempt window). Screenshots were reviewed but are not committed (the shell prompt in them shows a local user name).

### Step 1: window, title bar, theme, components — PASS

- `WindowOptions { titlebar: Some(TitlebarOptions { appears_transparent: true, traffic_light_position: Some(point(px(18.), px(18.))), .. }), app_owns_titlebar_drag: true, window_min_size: Some(size(px(960.), px(620.))), .. }` gives exactly Electron's `hiddenInset` + `trafficLightPosition {18,18}`: measured close/min/zoom at x = 18/38/58, 18 px from the top, `titlebarAppearsTransparent = YES`, style mask `0x800f` (titled, closable, miniaturizable, resizable, full-size content).
- The 52 px title bar is drawn by us: `gpui_component::TitleBar::new().h(px(52.)).pl(px(88.))…` (it handles drag-to-move via `start_window_move` and double-click via `titlebar_double_click`).
- Theme follows the appearance: `cx.observe_window_appearance(window, |_, window, cx| theme::sync(window, cx))`, where `sync` reads `window.appearance()` and sets both our token global (a slice of `tokens.css`) and `gpui_component::Theme::sync_system_appearance`. Verified by flipping the window's `NSAppearance` (Aqua ↔ DarkAqua, which is what a system switch does to a window) — tokens and gpui-component colors switched (screenshots light + dark checked).
- gpui-component `Button` (primary, small, click counter) and `Input` (`InputState::new(window, cx).placeholder(..)`) render and take keystrokes (`"hello gpui"` typed via `dispatch_keystroke`).
- API churn note: weekly releases rename things (`ScrollWheelEvent` → `ScrollEvent`, `on_scroll_wheel` → `on_scroll`, `InputState::value(cx)` → `value()`); expect small fixes on every bump.

### Step 2: board canvas — PASS

Transform approach (reuse this):
- Board state is `pan: Point<f32>` (screen offset of the world origin, board-local) and `zoom: f32`. A card is an absolutely positioned div at `pan + pos * zoom`, size `size * zoom`.
- **Everything inside a card is styled in `rems`, and the card subtree is wrapped in a tiny `Zoomed` element that calls `window.with_rem_size(Some(px(16. * zoom)), …)` in `request_layout`, `prepaint` and `paint`.** Text, padding, radii scale with the board; shadows are given `px(… * zoom)` explicitly. GPUI has no element transform, so this is how "the whole board scales" works. Custom elements read `window.rem_size() / 16` as their scale.
- Zoom around the pointer: `new = clamp(zoom * f, 0.2, 3.0); pan = p - (p - pan) * (new / zoom)`. Verified: the world point under the pointer is unchanged to 0.01 px after a pinch (`PinchEvent.delta`, `zoom *= 1 + delta`) and after ctrl+wheel (`exp(-dy * 0.01)`). Two-finger scroll (`ScrollDelta::Pixels`) pans 1:1.
- Drags (cards, rubber band) register window-level `window.on_mouse_event::<MouseMoveEvent/MouseUpEvent>` listeners during paint while a drag is active, so tracking doesn't depend on hover. Card drag moved by exactly `delta / zoom`; rubber band selected exactly the 4 intersecting cards.
- Dot grid: painted in a `canvas` paint closure in screen space (one `paint_quad` per dot, culled to the viewport).
- Smoothness (window inactive, `inactive_frame_interval: None`; frame clock = timestamps from a paint closure):
  - continuous pan, 180 frames: **60.0 fps**, gap p50 16.67 ms, p95 17.1 ms, max 19.7 ms, 0 gaps > 25 ms; CPU per frame (render+layout+paint) p50 1–3 ms.
  - continuous zoom (new font sizes every frame, terminal card + webview card + rotated sticker on the board), 180 frames: 60.0 fps, p95 19–21 ms, 1–3 gaps > 25 ms (max 27–34 ms) — glyph and SVG rasterization at new sizes. Quantize font sizes during gestures if this grows with many cards.
  - **GPUI throttles inactive windows to ~30 fps by default** (`WindowOptions::inactive_frame_interval`, also low-power and thermal limits). First runs measured 30–33 fps for that reason. Electron doesn't throttle; decide per window (terminals streaming in a background window will look like 30 fps).

### Step 3: terminal — PASS

Pattern (reuse; nothing copied from Zed's GPL `terminal_view`):
- `portable_pty::native_pty_system().openpty(PtySize{..})`, spawn `$SHELL -l` with `TERM=xterm-256color`, `COLORTERM=truecolor`.
- A reader thread reads 64 KiB chunks, locks `Arc<FairMutex<Term<Listener>>>` and runs `vte::ansi::Processor::advance`, then `try_send(())` on a `smol::channel::bounded(1)` (coalesces bursts). A GPUI task awaits the channel and `cx.notify()`s, so painting happens at most once per frame.
- `Listener: EventListener` writes `Event::PtyWrite` replies (DA, DSR) back to the PTY and records `Title`.
- `TerminalElement` (custom `Element`): in `prepaint` it computes the grid **in unscaled units** (`cols = (w/scale - 2·pad) / advance('m' @ 13px)`, `rows = … / round(13·1.2)`) and only recomputes it when the unscaled size moved by more than 1.5 px — layout snapping to device pixels and fixed-px decorations make `bounds / scale` wobble while zooming, and without that hysteresis a zoom gesture caused 37 PTY resizes (SIGWINCH storm); with it, a 180-frame zoom animation causes none. Better still in the real app: derive the grid from the card's world size, and keep card borders/selection rings from changing the inner size. It calls `master.resize` + `term.resize` only when cols/rows change, then walks `renderable_content().display_iter`, merges cells into style runs and shapes each run with `text_system.shape_line(text, font_size * scale, &[run], Some(cell_w))` (`force_width` keeps the grid monospace even for fallback glyphs; wide chars are their own run with `2·cell_w`). Backgrounds are merged quads; cursor block/beam/underline/hollow (hollow when unfocused).
- Colors: Nodus' xterm theme (`TERMINAL_THEME`), 256-color cube, truecolor, OSC color overrides from `term.colors()`, bold-as-bright (xterm.js default), inverse, dim, hidden.
- Keys → bytes: enter/backspace/tab/shift-tab/esc, arrows/home/end with app-cursor mode and xterm modifier codes, pgup/pgdn/del/ins, F1–F12, ctrl-letters, option composes (`macOptionIsMeta: false` like Nodus), ⌘V pastes with bracketed paste. Wheel: scrollback, or arrow keys in the alternate screen.

Verified (autopilot, screenshots reviewed): `ls --color=auto -la` (colored dirs/executables), `printf` with bold red, green background, underline, `38;5;208`, `38;2;r;g;b`, and `中文` correctly two cells wide; `sleep 30` + ctrl-C; `top -o cpu -s 1` (alt screen, live refresh, inverse header); `vim -u NONE -c 'syntax on' -c 'set number'` (syntax colors, insert mode typed text, `-- INSERT --`); window resize → `stty size` reports the new `rows cols`; zooming the card to 135 % keeps `69×19` and the text scales crisply; `seq 1 400000` (3.1 MB) streamed into a card while the board animated: UI stayed at 60 fps (p95 16.9 ms, render p50 1.1 ms).
Font: Nodus asks for `'SF Mono', ui-monospace, Menlo`. "SF Mono" is not an enumerable family for GPUI/CoreText, but the system ships it: `cx.text_system().add_fonts(vec![Cow::Owned(std::fs::read("/System/Library/Fonts/SFNSMono.ttf")?)])` at startup registers it as family **`.SF NS Mono`**, which renders correctly in the grid (verified by screenshot). Load it from the system at runtime; never bundle it (Apple's font license). Windows: Cascadia Mono / Consolas as in the CSS stack.

Not done (port work for P7): selection + copy, mouse reporting, IME/marked text (needs an `InputHandler`), cursor blink, links, bell, search.

### Step 4: wry webview — PASS (with snapshot swap)

- Build: `WebViewBuilder::new().with_custom_protocol("nodus-view".into(), serve).with_url("nodus-view://demo/").with_ipc_handler(|req| tx.try_send(req.into_body())).with_accept_first_mouse(true).build_as_child(&*window)` — GPUI's `Window` implements `HasWindowHandle` (the AppKit handle is GPUI's content NSView).
- **Sandbox plan verified**: the custom scheme serves a tiny shell page holding the view's page in `<iframe sandbox="allow-scripts" src="nodus-view://demo/page">` (own CSP per response); the shell relays `postMessage` ⇄ `window.ipc.postMessage` / `evaluate_script`. Page origin inside the iframe is `null` (opaque), as in 0.9.4. Three JS→Rust→JS round trips: RTT 0–1 ms.
- **Transient user activation** is observable in WKWebView inside the sandboxed iframe: a real click (an `NSEvent` sent through `-[NSWindow sendEvent:]`) on a button in the iframe reported `navigator.userActivation.isActive = true` (and in the shell); a `setTimeout` message reported `isActive: false, hasBeenActive: false`. So "startSession only right after a click" can be enforced exactly like in Electron.
- **Positioning**: a `canvas` in the view card's body calls `web.place(bounds, window.content_mask().bounds, zoom, radius)` in its prepaint — the native view follows the card in the same frame (panning the card at 60 fps, p95 16.9 ms, no visible lag in screenshots).
- **Clipping + rounded corners**: the WKWebView is re-parented into our own container NSView (`S1WebHostView`, layer-backed, `masksToBounds`). Container frame = visible rect (card body ∩ board viewport), webview frame = full document rect inside it (no reflow when the card is half off-screen); `cornerRadius = 10·zoom` with `maskedCorners` set only for the bottom corners that are actually visible. Verified with the card cut by the board's top-left edge.
- **Zoom**: board zoom → `webview.zoom(z)` (WKWebView `pageZoom`) with frame = body·z. CSS viewport stays the card's unscaled size: zoom 1.0 → `358×226 dpr 2`, zoom 0.7 → `357×224 dpr 1.4` — same layout as the 0.9.4 iframe under a CSS transform, and sharper (it rasterizes at the real device scale).
- **Z-order**: the native view is always above everything GPUI paints in the window (single CAMetalLayer below). Proven approach: when anything GPUI draws overlaps the view (a later card, a menu, a dialog, a toast, the prompt bar), while the view is dragged/resized, or while an ink tool is out, call `takeSnapshotWithConfiguration:completionHandler:` (2–16 ms here, typically 2–6), convert to BGRA → `RenderImage`, hide the container and paint `img(ImageSource::Render(..))` in the card; restore the live view when unobstructed. Verified: overlapped → snapshot shown and a click on the overlapping note hit-tests to GPUI's view; unobstructed → hit-test reaches `WryWebView`; ink mode → snapshot. Invalidate the snapshot whenever the page reports a change (IPC) — a stale capture was the only artifact seen.
- **Pointer pass-through**: the container (an objc2 `define_class!` NSView subclass) overrides `hitTest:` and returns nil while the board drags (`passthrough` flag), so the pointer goes to GPUI over the page; verified (`GPUIView` during a card drag, `WryWebView` after). For a two-finger pan that starts on the board and slides over a view, set the same flag for the whole scroll gesture (`TouchPhase::Started` → end of momentum); that path wasn't exercised with real trackpad events.
- Reordering native subviews doesn't help: GPUI paints everything into one CAMetalLayer, so no GPUI element can be placed above a native sibling. Only GPUI "window composition" (open PR zed#62379, shipped in `gpui-fast`) splits GPUI into surfaces above/below native views. The snapshot swap is the stock-GPUI answer and matches what the specs need (views are interactive only when nothing covers them).
- Gotcha: AppKit calls that dispatch events synchronously (`sendEvent:`, `orderFront`, `setFrameOrigin` → `windowDidMove`) must not run inside a GPUI `update` closure — GPUI re-enters and logs `RefCell already borrowed`. Run them outside (`cx.defer` / after the update).

### Step 5: floating panel — PASS (notes)

- `WindowOptions { kind: WindowKind::PopUp, titlebar: None, window_background: WindowBackgroundAppearance::Transparent, is_movable: false, is_resizable: false, focus: true, .. }`. On macOS GPUI's PopUp is already an `NSPanel` subclass with `NonactivatingPanel`, `canBecomeKeyWindow = YES`, level 101 and `CanJoinAllSpaces | FullScreenAuxiliary` — i.e. Electron's `type: 'panel'` + `setVisibleOnAllWorkspaces(true, {visibleOnFullScreen})` + `setAlwaysOnTop(true, 'pop-up-menu')`.
- objc2 calls on the raw NSWindow (reuse; **order matters — `setFloatingPanel:YES` resets the level to 3, so call it before `setLevel:`**):
  ```rust
  msg_send![&*w, setFloatingPanel: Bool::YES];
  w.setLevel(101); // NSPopUpMenuWindowLevel
  w.setCollectionBehavior(CanJoinAllSpaces | FullScreenAuxiliary | Stationary | IgnoresCycle);
  w.setHasShadow(false); w.setOpaque(false); setBackgroundColor: NSColor.clearColor;
  msg_send![&*w, setHidesOnDeactivate: Bool::NO];
  msg_send![&*w, setBecomesKeyOnlyIfNeeded: Bool::NO];
  ```
  Measured: `level=101 collectionBehavior=0x151 isPanel=true styleMask=0x8081`.
- Rounded card + shadow are drawn by GPUI inside a transparent margin (Electron's `.cap-stage`); screenshot shows the soft shadow on a fully transparent window.
- Showing it: open with `focus: false, show: false`, then (outside the GPUI update, e.g. `window.defer`) `orderFrontRegardless()` + `makeKeyWindow()`. Launched from the background, the panel became key (`isKeyWindow = YES`, GPUI `is_window_active() = true`) while **the other app stayed frontmost** (`NSWorkspace.frontmostApplication` unchanged — Nodus doesn't take the menu bar or raise its main window); note `NSApp.isActive` reads YES while the panel is key. The gpui-component Input inside took keystrokes (`"buy milk"`, screenshot checked).
- Hide on blur: `cx.observe_window_activation(window, …)` → when `!window.is_window_active()`, `orderOut:`; verified both when another window of ours became key and when the panel resigned key (what a click in another app does): `key=false visible=false`. Re-show = the same two calls (what the double-⇧ hotkey will do).
- Not exercised: drawing over another app's full-screen Space (would take over the user's screen); the window flags are the ones Electron uses for that.

### Step 6: measurements

| | |
| --- | --- |
| Clean build, dev profile (deps opt-level 2), all deps | 3 m 12 s (10 cores, ~800 % CPU) |
| Clean build, release (thin LTO, 16 CGUs, strip) | 1 m 45 s |
| Incremental, dev (touch one file) | 2.0 s |
| Incremental, release | ~60 s (thin LTO dominates) |
| Binary, release (stripped) | 20.6 MB (debug 40 MB) |
| Startup to first frame, release (board + 2 shells + webview) | ~0.5 s |
| Idle CPU, release, board open | 0.0 % app, 0.0 % WebKit helpers (`top`, 20 s) |
| Memory, release, idle | app ~85–90 MB (top MEM; RSS 122 MB) + WebKit GPU 27 MB + WebContent 39 MB + Networking 13 MB + 2 shells ~5.5 MB each |
| Memory after the full autopilot (debug build, zoom churn + 3 MB flood) | 227 MB RSS |

The blur proof (step 8), which depends on the Zed workspace crates, cold-builds in 2 m 20 s.

### Step 7: rotated elements — stock GPUI: SVG only; prototype works

Confirmed: in GPUI only `MonochromeSprite`s carry a `TransformationMatrix` and only `paint_svg` exposes it; quads, shadows, images and glyphs are axis-aligned. (A community `ElementTransform` — translate + scale, explicitly **no rotation** — exists in a fork: remorses/zed `gpuix` branch, commit `1da884b3b9`.)

Prototype on stock GPUI (`rotated.rs`): a sticker/tilted note is a stack of single-color SVG layers (shadow, paper, ink incl. text via resvg) painted with `window.paint_svg(bounds, key, Some(svg), matrix, color, cx)` where `matrix = translate(c)·rotate(θ)·scale(residual)·translate(-c)`. Rasters are cached in the sprite atlas per (content, zoom bucket), so changing only the angle is free. Zoom is quantized to half-octave buckets with the remainder applied by the matrix; the blurred shadow is rasterized at ¼ size and scaled up. Result: a note tilting 3° → 7° while dragged with Nodus' 29-stop `--ease-stack` spring and settling back, at 60 fps (zoom animation p95 19.7 ms). Fidelity: good for paper + shadow; text is resvg-rendered (grayscale AA, not CoreText, slightly different metrics), only single-color layers (gradients need one layer per color), no live content (a terminal can't tilt this way).

Full fidelity needs a GPUI patch. Best route: the effect-layers patch below already renders a subtree into an offscreen texture and composites it; adding a 2×3 transform to the composite quad's vertex shader (Metal, HLSL, WGSL) rotates any element — text rendered by CoreText/DirectWrite at full resolution, children, images. Estimate: 2–3 days on top of effect layers, plus inverse-transform hit testing (or keep hit-testing the unrotated box, which is what the board needs while dragging).

Animation: GPUI has per-element `.opacity()` and `with_animation(Animation::new(d).with_easing(f))` with **any** `Fn(f32) -> f32`, so CSS `cubic-bezier()` and multi-stop `linear()` are a few lines each (`rotated.rs::cubic_bezier`, `css_linear`, used in the spike); there is also `gpui::spring`. Caveat: `.opacity()` multiplies each primitive's alpha (overlapping children show through each other) — CSS `opacity` is *group* opacity; the effect-layer patch's layer opacity gives true group opacity.

### Step 8: region backdrop blur — stock GPUI: no; patched GPUI: works

Stock GPUI has only window-level blur (`WindowBackgroundAppearance::Blurred`). Nodus uses `backdrop-filter: blur(20–30px) saturate(1.6–1.8)` on the prompt bar, mic, context menus, toasts, selection bar, ink dock, modal backdrops, archive panel — so this is a hard requirement.

Working approach: the **"effect layers" PR stack by mateo-m, zed-industries/zed #63773 (core: scene/style/window) → #63777 (Metal) → #63778 (wgpu) → #63779 (DirectX 11) → #63780 (variable blur)**, head `e343ea6471ce`, closed unmerged on 2026-09-04 (split from remorses/zed#7, also closed). It adds `Styled::backdrop_blur(px)`, `backdrop_matrix(ColorMatrix)`, `blur`, `color_matrix`, `mask`, `blend_mode`; the renderer draws the element's backdrop into an offscreen texture, runs two separable Gaussian passes on a downscaled copy (cost ~independent of radius) and composites inside the rounded box. `saturate(s)` is a 4×5 color matrix (in the proof).

Proof (`s1-src/blur/`, built against that head with `runtime_shaders`): an "archive panel" with `backdrop_blur(px(30.)).backdrop_matrix(saturate(1.8))` and a "toast" with 20 px / 1.6 over colored cards, text and a moving gradient pill — correct frosted glass, content behind is blurred and saturated, rounded corners clip, at **60 fps** (p50 16.7 ms, p95 18.7 ms) — no measurable difference vs blur off at this size.

Blockers found and worked around (must be fixed properly in our patch):
- On macOS 27.2 / M4, creating two of its pipelines **crashes the Metal backend compiler** (`MTLCompilerService` SIGSEGV in libLLVM, surfaced as `XPC_ERROR_CONNECTION_INTERRUPTED`): `variable_blur_fragment` and the full `layer_composite_fragment` (mask + 17 blend modes). Spike patch: compile the variable-blur pipeline lazily (only masked backdrops need it) and drop masks / non-normal blend modes from the composite shader. Real fix: split the composite into function-constant specializations; file a Feedback with Apple.
- The stack is ~890 commits behind Zed main; it must be rebased onto the gpui-pre snapshot we pin (35 commits, +5.3k/−0.5k lines, 26 files). The DirectX 11 HLSL was never compiled by its author's CI — unverified on Windows.

Alternatives considered: `NSVisualEffectView` children (within-window blending can blur the Metal layer below, but it sits above all GPUI content, so the panel's own content would need GPUI composition surfaces — gpui-fast / zed#62379 — and it has no Windows counterpart); CPU blur of a snapshot (too slow, stale). Not pursued.

### Patch set this implies

Maintain `nodus-gpui` = the weekly `gpui-pre` snapshot + (1) effect layers (blur, backdrop blur, color matrix, group opacity, rounded clip), with the Metal compiler workaround, (2) a composite transform for rotated elements, optionally (3) window composition (zed#62379) if snapshot-swapping webviews proves insufficient. Consume it with `[patch.crates-io]` on the `gpui-pre*` crate names so gpui-component keeps resolving. Estimated cost: ~1–1.5 weeks to land (1)+(2) on Metal + DX11 + wgpu with screenshot tests, then ~½ day per weekly rebase (or rebase monthly). Upstreaming is worth trying again (the PRs were closed without merging; Zed's CLA was the only blocker visible in the thread).

## Windows risks

- GPUI on Windows (DirectX 11 renderer) wasn't exercised here; gpui-component claims Windows support. CI should build and smoke-test the spike on Windows before P5.
- Effect layers on DX11 are unverified (HLSL not compiled by the PR's CI).
- wry on Windows is WebView2: a child HWND, also always above GPUI's swap chain — the snapshot swap applies (`ICoreWebView2::CapturePreview`, async PNG). Custom schemes are `http://nodus-view.localhost/` (or `https://` with `with_https_scheme`), so the page origin differs from macOS; the sandboxed iframe keeps it opaque. Clipping to rounded corners needs a region on the child HWND or WebView2 composition (`ICoreWebView2CompositionController`) — not available through wry's `build_as_child`; may need `webview2-com` directly.
- Page zoom: `ICoreWebView2Controller::ZoomFactor` (what `wry::WebView::zoom` uses) also changes layout CSS px like WKWebView's pageZoom — should match, untested.
- portable-pty uses ConPTY on Windows: needs Windows 10 1809+; resize and ctrl-C semantics differ; `$SHELL -l` has no meaning (Nodus already picks shells per platform).
- The floating panel's NSPanel behavior (non-activating key window over full-screen apps) has no direct Win32 equivalent; Electron uses a topmost tool window there — replicate with `WS_EX_TOOLWINDOW | WS_EX_TOPMOST` and `WS_EX_NOACTIVATE` handling via the raw HWND.

## Gotchas for the real implementation

- Call AppKit APIs that synchronously dispatch events or delegate callbacks outside GPUI `update` closures.
- `setFloatingPanel:` resets the panel level; set the level after it.
- A normal window opened with `focus: true` activates the app; for the capture panel open with `focus: false, show: false` and use `orderFrontRegardless` + `makeKeyWindow` on the non-activating panel.
- The terminal grid must not follow zoom-induced pixel noise (see step 3): hysteresis or world-size-derived grid.
- `PinchEvent` listeners require the hitbox to be hovered (the platform sends a mouse move first in practice).
- `Keystroke`/`Input` APIs churn weekly; pin `=` versions and bump deliberately.

## Reproduce

```sh
cd migration/spikes/s1-src            # copy it somewhere outside the repo first if you like
cargo build                           # ~3 min cold
swiftc -O tools/wincap.swift -o wincap   # optional: single-window capture via ScreenCaptureKit
./target/debug/s1 board               # or: window | panel | all
tools/run.sh board 170                # autopilot: prints "S1 STEPn … pass=…" lines, screenshots to shots/
```

Env: `S1_AUTOPILOT=1` (scripted synthetic input + checks), `S1_QUIT=1`, `S1_NO_ACTIVATE=1` (never steal focus), `S1_STAGE_EXEMPT=1` (keep the test window out of Stage Manager), `S1_NO_THROTTLE=1` (disable GPUI's 30 fps inactive-window cap), `S1_APPEARANCE=light|dark`, `S1_TERM_FONT=<family>`.

Files worth reading first: `src/zoom.rs` (rem-scaling wrapper), `src/board.rs` (pan/zoom/drag/rubber band, snapshot swap), `src/terminal.rs` (PTY + alacritty + element), `src/webview.rs` + `src/mac.rs` (wry child, clip container, pass-through, snapshot, panel flags), `src/rotated.rs` (rotated SVG layers, cubic-bezier / linear() easing), `blur/` (backdrop blur on the effect-layers stack, plus the Metal workaround patch).
