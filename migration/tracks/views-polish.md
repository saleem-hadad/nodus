# Track: views-polish (P12, spec 04's pages on the board)

- **Branch:** `rust/views-polish`
- **Scope:** the differences left between views on the native board and 0.9.4, where a view's page was an iframe in
  the DOM, so ink, tooltips, menus and dialogs drew over it naturally (plan.md, Integration debts, "Views (from
  views-ui)"). Code in `crates/nodus-ui/src/views/`, three small GPUI patches (`third_party/gpui`, PATCHES.md 16).
- **Status:** ✅ the six items done on macOS; Windows keeps the gaps views-ui listed (WebView2 never run).

## Items

| # | What was different | Now |
| --- | --- | --- |
| 1 | Ink strokes hid under a live page (0.9.4 drew ink over every iframe, and the page kept the pointer while no ink tool was out). | The marks that cross a live page draw again in a native overlay above its WKWebView, inside the page's clip container, whose `hitTest:` is nil (`mac_ink.rs`): pen lines and arrows (the same paths, `PEN_WIDTH × size × zoom`, round caps and joins), the selection's dashed outline, and labels in the system font at 600 set on GPUI's own baseline (the line GPUI shapes, `chromium_baseline_offset`). Core Graphics and Core Text draw them into an sRGB picture of just the part of the page they cover, at the display's scale; it's drawn again only when what it shows changes (pans move it with the page; zooms, edits, theme and drags redraw it). The ink scene comes from the layer's own state (`board/overlays/ink/overlay.rs`, the marks `render_ink_layer` draws, in its order). A snapshot is taken of the webview alone, so it never holds the overlay; under a snapshot GPUI's ink layer shows, as before. |
| 2 | Another element's tooltip over a live page was hidden. | A tooltip inserts no hitbox, so the coverage check also asks GPUI where the frame's tooltip is (`Window::tooltip_bounds`): one over the page's visible part covers it. Any other floating element (menus, popovers, dialogs, deferred draws) blocks the pointer and so already covered. |
| 3 | A menu or overlay over a live page showed a frame or two late. | Coverage is decided in the frame's paint, from that frame's hitboxes (`Hitbox::is_hovered_at_in_frame`; every hitbox is in by paint), not after it lands: the probe swaps to the snapshot and paints it in the same frame as the menu, and that frame presents in the Core Animation transaction that hid the webview (`Window::present_with_native_views`), so neither shows a frame early. A snapshot is kept ready for this: a live page that changed is pictured again at most once a second (`READY_EVERY`), and a page shown from a picture that may be old is pictured again at once. Only a page never pictured yet (just loaded) still steps aside a frame or two late. |
| 4 | The New view box's placeholder didn't wrap; no resize grip. | `.action-description` as the actions dialog sets it (13px at 1.45, Size::Large's 10px 12px, the 10px wrap margin), the placeholder drawn by the dialog under the field so it wraps (gpui-component draws one line), and Chromium's grip in the corner: its two strokes (as the merge dialog draws them), the up-down cursor, dragging the box taller or shorter, a line at the least. |
| 5 | Views read `git status` every 4 s themselves. | The relay follows the git UI's model (`GitModel`, the store's `git`): `git` goes to pages whenever its status changes, as 0.9.4's BoardView effect broadcast the store's, and on mount with what's known. One read on 0.9.4's rules (route, session status, focus, 4 s while focused). |
| 6 | `render_to_image` sometimes left a menu's or modal's page area blank when snapshots were in the frame. | Not GPUI's capture: in every failing shot (2 in 50 runs) WebKit hadn't delivered a single snapshot of the board's pages yet, so there was nothing to draw; images inside effect layers capture as drawn. Fixed where it was: a snapshot WebKit fails to take is tried again (`RETRY`), and the test driver waits up to 5 s for every page that should show a picture to have a current one before a shot (`swap::pictures_ready`). The large window draws its snapshot in its panel again (the probe paints it), which is closer to the golden. 24 stress runs (views-menu, views-large) then gave identical shots. |

## Files

| File | What |
| --- | --- |
| `views/ink.rs` | `BoardInk` (the board's ink scene and where the board is, each render), `page_ink` (the marks over a page, in its px, with the picture's rect) |
| `views/mac_ink.rs` | the overlay view (`NodusViewInkView`, hit test nil) and its picture: Core Graphics paths, Core Text labels |
| `views/swap.rs` | coverage decided in paint (hitboxes and tooltip), the same-frame swap, ready and retried snapshots, the overlays kept current, `pictures_ready` |
| `views/frames.rs` | the probe paints the snapshot; `want`, `retry_at`, `ink` per frame, the board's ink; the relay follows `GitModel` |
| `views/mac.rs`, `views/webview.rs` | the container holds the overlay above the webview and places it with the page (`set_ink`) |
| `views/dialog.rs` | the box's style, wrapped placeholder and grip |
| `views/relay.rs` | `git_changed` from the model; no reads of its own |
| `views/card.rs`, `views/large.rs` | the snapshot is the probe's to paint (the large window's back in its panel) |
| `parity/views_scenes.py` | `try-ink`, `try-ink-select`, `try-tooltip`, `try-new-empty`, `try-new-grip` |

Edits outside `views/` (additive): `board/overlays/ink/overlay.rs` (new) and its `mod` line; `automation.rs` (the
screenshot waits for pages' pictures); `Cargo.toml`s (`objc2-core-text`, CoreGraphics/CoreFoundation features);
`third_party/gpui` (PATCHES.md 16: `Hitbox::is_hovered_at_in_frame`, `Window::tooltip_bounds`,
`Window::present_with_native_views` with `PlatformWindow::set_presents_with_transaction`; a note under 5 on item 6).

## Checked

`python3 crates/nodus-ui/parity/views_scenes.py views- --golden <dir> --world <scratch>`, then `compare.mjs`:

| Scene (light / dark) | Before | After |
| --- | --- | --- |
| views-board | 0.49% / 0.49% | 0.49% / 0.49% |
| views-brain-waiting | 0.54% / 0.54% | 0.54% / 0.54% |
| views-menu | 0.53% / 0.52% | 0.53% / 0.52% |
| views-change | 0.56% / 0.55% | 0.56% / 0.55% |
| views-large | 0.13% / 0.10% | 0.06% / 0.06% (snapshot in the panel) |
| views-new | 0.09% / 0.08% | 0.05% / 0.05% (the dialog's region 0.40% → 0.17%: the box's text size and the grip) |

Spec 11's `ink-*` scenes (`scenes11.py`) are unchanged by this (0.39–0.43% whole shots). Live, with
`VIEWS_LIVE=1 VIEWS_SCREENCAPTURE=<png>` (the screen's pixels, webviews included): `try-ink` and `try-ink-select` show
the pen line, arrow, label and selection outline continuing across the page's edge without a seam, in both themes (the
label's glyphs land on the same device rows inside and outside the page), and `hit` over a stroke on the page still
reaches the page (`WryWebView`); `try-tooltip` puts the page behind its snapshot under a note's ⤢ tooltip;
`views-menu` live swaps in the menu's own frame (logged while testing: the swap happened in paint, with the ready
snapshot).

## Left

- Windows: no ink overlay (the ink stays under a live WebView2 page) and the gaps views-ui listed (no snapshots,
  rounded clipping, clip to board or pass-through).
- The overlay's anti-aliasing is Core Graphics' where GPUI's ink is lyon + MSAA: identical geometry, edges within a
  fraction of a device pixel; labels are Core Text's line at GPUI's baseline (glyph positions unquantized, GPUI's are
  at quarter pixels).
- A page never pictured yet (the first moments after it loads) still steps aside a frame or two after what covers it.
- `present_with_native_views` with `CAMetalDisplayLink` is checked by use (menus, tooltips, scenes), not with a
  high-speed capture of the swap frame.
