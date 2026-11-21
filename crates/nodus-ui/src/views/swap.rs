//! The snapshot swap (decision D3, spike S1 step 4). A native webview always
//! draws above everything GPUI paints, so whenever GPUI must draw over a
//! page (a note, a menu, a dialog, a toast, a tooltip, the marquee), or the
//! page moves with the board in a way the webview can't follow smoothly
//! (dragged, resized, zoomed), or an ink tool is out, the page goes
//! transparent and GPUI draws a snapshot of it in its place. It comes back
//! once nothing covers it. The board's ink is the exception: it draws over a
//! live page in a native overlay (`ink.rs`), as 0.9.4's ink layer drew over
//! its iframes and let the pointer through.
//!
//! What covers a page is found while the frame paints, from GPUI's own hit
//! test of that frame: anything drawn later that blocks the pointer over
//! the body's probe (`frames::probe`), or a tooltip over it. Every hitbox of
//! a frame is in by then, so a page that a menu or a dialog opens over steps
//! aside in the frame that first shows it, and that frame is presented with
//! the webview change (`Window::present_with_native_views`), as an iframe
//! under 0.9.4's menus was covered the frame they opened. A snapshot is kept
//! ready for that, taken again a while after the page last changed; a page
//! with none yet steps aside once one is taken.
//!
//! The board's own chrome (the dock, the prompt bar, the zoom pill, the
//! minimap, a note over the page) only cuts holes in a page (macOS): the page
//! stays live and takes the pointer everywhere else, as a session card under
//! the prompt bar does, and GPUI draws the chrome in the holes, over the
//! page's snapshot. Only what comes and goes over everything (menus,
//! popovers, modals, tooltips), or chrome over all of the page, still puts
//! the page behind its snapshot.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, Bounds, Hitbox, Pixels, RenderImage, Window, point, px};

use super::frames::{Frame, Geometry, Slot, frames};

/// The zoom must hold this long before the page follows it live.
const ZOOM_SETTLE: Duration = Duration::from_millis(180);
/// A page whose body changed size gets this long to lay out again.
const RELAYOUT: Duration = Duration::from_millis(150);
/// A page given a new scale (a zoom, a size) stays behind its snapshot until
/// it has drawn at it, or this long at most.
const RESCALE: Duration = Duration::from_millis(500);
/// A snapshot shown this long while the page may have changed is taken again.
const STALE_AFTER: Duration = Duration::from_millis(400);
/// A live page that changed gets a new snapshot at most this often, ready
/// for whatever covers it next.
const READY_EVERY: Duration = Duration::from_millis(1000);
/// A snapshot WebKit couldn't take (a page not drawn yet) is tried again
/// after this, twice as long after each failure in a row, up to 16 times.
const RETRY: Duration = Duration::from_millis(120);
/// More holes than this in a page and it goes behind its snapshot instead.
const MAX_HOLES: usize = 24;

/// A frame's probe painted: its webview follows the body, what covers the
/// page this frame is decided, and the snapshot comes back for the probe to
/// paint, with whether it's in the page's place (else it's under the live
/// page, and shows only while the webview shows nothing: WebKit between two
/// drawings of the page, or the webview hidden a frame before GPUI's frame
/// with the snapshot lands).
pub fn placed(
    view_id: &str,
    slot: Slot,
    g: Geometry,
    hitbox: Hitbox,
    window: &mut Window,
    cx: &mut App,
) -> Option<(Arc<RenderImage>, bool)> {
    let parity = crate::parity::frozen(cx);
    let rc = frames(cx);
    let mut f = rc.borrow_mut();
    let schedule = !f.check_scheduled;
    f.check_scheduled = true;
    let ink = f.ink.clone();
    let mut paint = None;
    let mut swapped = false;
    let mut moved = false;
    if let Some(fr) = f.get_mut(view_id, slot) {
        let now = Instant::now();
        let changed = fr
            .geometry
            .is_none_or(|prev| prev.document != g.document || prev.clip != g.clip || prev.rounding != g.rounding);
        let was_offscreen = fr.web.offscreen();
        if let Some(prev) = fr.geometry {
            if (prev.zoom - g.zoom).abs() > 1e-4 {
                fr.zoom_changed = Some(now);
            }
            if prev.document.size != g.document.size {
                fr.dirty = true;
                fr.resized = Some(now);
            }
        }
        fr.drawn = true;
        fr.geometry = Some(g);
        let zoom_moving = zooming(fr, now);
        // While the zoom moves, the page waits behind its snapshot; it lays
        // out once, at the zoom the board settles on.
        if !zoom_moving || fr.live {
            fr.web.place(g.document, g.clip, g.zoom, g.rounding, window.ui_zoom());
        }
        // A live page that moved on screen (the board panned under it) moves
        // with this frame: presented on GPUI's own clock, the frame lands a
        // few vsyncs after the webview has moved, and the page swims ahead
        // of its card while the board moves.
        moved |= fr.live && changed && !(was_offscreen && fr.web.offscreen());
        fr.web.set_edge(g.edge);
        let visible = g.document.intersect(&g.clip);
        let cover = if fr.web.offscreen() {
            Cover::default()
        } else {
            cover(&hitbox, visible, window)
        };
        fr.occluded = cover.whole;
        if fr.holes != cover.holes {
            // Cut in the frame that first draws what's over the page.
            fr.web.set_holes(&cover.holes, window.ui_zoom());
            fr.holes = cover.holes;
            moved |= fr.live;
        }
        fr.probe = Some(hitbox);
        fr.want = parity || g.forced || g.reveal || fr.occluded || fr.tooltip || zoom_moving;
        if g.reveal && fr.live && fr.snapshot.is_none() {
            // MOTION-033: a first page stays hidden until it's pictured.
            go_snapshot(fr);
            swapped = true;
        } else if fr.want && fr.live && fr.snapshot.is_some() {
            // In this frame, under what covers it. The picture may be a
            // little old: a new one replaces it as soon as it's taken.
            go_snapshot(fr);
            fr.dirty = true;
            swapped = true;
        }
        if fr.live || !zoom_moving {
            update_ink(fr, ink.as_ref(), window);
        }
        fr.web.set_passthrough(!fr.live || g.passthrough);
        // Under a live page too, where its picture shows round the chrome
        // drawn in its holes.
        paint = fr.snapshot.clone().map(|image| (image, !fr.live));
    }
    drop(f);
    if swapped || moved {
        window.present_with_native_views();
    }
    if schedule {
        window.on_next_frame(after_frame);
    }
    paint
}

/// Keeps a card's ink overlay showing the board's ink over its page (the
/// large window is above the ink, so it has none).
fn update_ink(fr: &mut Frame, ink: Option<&super::ink::BoardInk>, window: &Window) {
    let wanted = match (fr.slot, ink, fr.geometry) {
        (Slot::Card, Some(ink), Some(g)) if !fr.web.offscreen() => super::ink::page_ink(ink, g.document, window),
        _ => None,
    };
    if fr.ink != wanted {
        fr.web.set_ink(wanted.as_ref());
        fr.ink = wanted;
    }
}

fn zooming(fr: &Frame, now: Instant) -> bool {
    fr.zoom_changed.is_some_and(|t| now.duration_since(t) < ZOOM_SETTLE)
}

/// After a frame lands: hides frames nothing drew, takes the snapshots
/// pages need, and brings back pages nothing covers any more.
fn after_frame(window: &mut Window, cx: &mut App) {
    let rc = frames(cx);
    let now = Instant::now();
    let parity = crate::parity::frozen(cx);
    let ui = window.ui_zoom();
    let mut refresh = false;
    let mut settle = false;
    let mut shoot: Vec<u64> = Vec::new();
    {
        let mut f = rc.borrow_mut();
        f.check_scheduled = false;
        let ink = f.ink.clone();
        for fr in &mut f.frames {
            let Some(g) = fr.geometry else { continue };
            if !fr.drawn {
                // Culled or not drawn: off screen, but the page keeps running.
                fr.web.place(g.document, Bounds::default(), g.zoom, g.rounding, ui);
                if fr.ink.take().is_some() {
                    fr.web.set_ink(None);
                }
                continue;
            }
            let zoom_moving = zooming(fr, now);
            settle |= zoom_moving;
            // The page may need a picture: to show now, or to have ready for
            // the frame something first covers it (a frame that draws: a
            // page that changes redraws the board).
            let needed = fr.want && fr.dirty;
            let ready = cfg!(target_os = "macos")
                && !fr.want
                && fr.live
                && !fr.web.offscreen()
                && fr.dirty
                && fr.snapshot_at.is_none_or(|t| now.duration_since(t) >= READY_EVERY);
            // MOTION-033: a first page being revealed is pictured once it
            // has loaded (`page_loaded` draws again then), not blank.
            let unloaded = g.reveal && !fr.loaded;
            if (needed || ready) && !fr.pending && !unloaded {
                let retrying = fr.retry_at.is_some_and(|t| now < t);
                // A body that just changed size lays its page out again
                // first, and a new scale draws first; a picture now would be
                // of the old layout, or the old scale.
                let relaying = fr.resized.is_some_and(|t| now.duration_since(t) < RELAYOUT) || !fr.web.scaled(RESCALE);
                let stale = fr.snapshot_at.is_none_or(|t| now.duration_since(t) >= STALE_AFTER);
                if retrying || relaying || (needed && !fr.live && !stale && !parity) {
                    settle = true;
                } else {
                    shoot.push(fr.id);
                }
            }
            if !fr.want && !fr.live {
                fr.web.place(g.document, g.clip, g.zoom, g.rounding, ui);
                // The page waits behind its snapshot until it shows the zoom
                // it was just given (`page_scaled` draws again then): the
                // webview would show the old one for a frame or two, and the
                // page would flicker as the board's zoom settles.
                if !fr.web.scaled(RESCALE) {
                    settle = true;
                    continue;
                }
                update_ink(fr, ink.as_ref(), window);
                fr.live = true;
                fr.web.set_live(true);
                fr.web.set_passthrough(g.passthrough);
                refresh = true;
            }
        }
        if settle && !f.settle_scheduled {
            f.settle_scheduled = true;
        } else {
            settle = false;
        }
    }
    for id in shoot {
        take_snapshot(id, window, cx);
    }
    if refresh {
        window.refresh();
    }
    if settle {
        // Look again once the zoom has settled, or the snapshot can be
        // taken again, whether or not anything draws meanwhile.
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(ZOOM_SETTLE + Duration::from_millis(20))
                .await;
            cx.update(|cx| {
                if let Some(rc) = super::frames::existing(cx) {
                    rc.borrow_mut().settle_scheduled = false;
                }
                let _ = handle.update(cx, |_, window, _| window.refresh());
            });
        })
        .detach();
    }
}

/// Shows the snapshot in the page's place.
fn go_snapshot(fr: &mut Frame) {
    fr.live = false;
    fr.web.set_live(false);
    fr.web.set_passthrough(true);
}

/// Takes a picture of a frame's page; once it's in, the next frame shows it
/// if the page should be behind it (the swap itself is the frame's, so it
/// lands with what covers the page).
fn take_snapshot(id: u64, window: &mut Window, cx: &mut App) {
    let rc = frames(cx);
    let handle = window.window_handle();
    let mut f = rc.borrow_mut();
    let Some(fr) = f.by_id_mut(id) else { return };
    fr.pending = true;
    fr.dirty = false;
    let rc2 = rc.clone();
    let async_cx = cx.to_async();
    fr.web.snapshot(move |image| {
        // WebKit's completion runs from the run loop, outside GPUI: hop in.
        async_cx.update(|cx| {
            let (old, redraw) = {
                let mut f = rc2.borrow_mut();
                let Some(fr) = f.by_id_mut(id) else { return };
                fr.pending = false;
                let now = Instant::now();
                fr.snapshot_at = Some(now);
                let Some(image) = image else {
                    if cfg!(target_os = "macos") {
                        // Not drawn yet, or WebKit couldn't: try again shortly.
                        fr.dirty = true;
                        fr.retry_at = Some(now + RETRY * 2u32.pow(fr.failed.min(4)));
                        fr.failed += 1;
                    } else if fr.live && fr.want {
                        // Where pages can't be pictured (WebView2 for now),
                        // the page still steps aside so what covers it shows.
                        go_snapshot(fr);
                    }
                    drop(f);
                    let _ = handle.update(cx, |_, window, _| window.refresh());
                    return;
                };
                fr.retry_at = None;
                fr.failed = 0;
                let old = fr.snapshot.replace(image);
                (old, fr.want || !fr.live)
            };
            // The old picture's texture goes from the atlas.
            if let Some(old) = old {
                cx.drop_image(old, None);
            }
            if redraw {
                let _ = handle.update(cx, |_, window, _| window.refresh());
            }
        });
    });
}

/// What GPUI draws over a page in the frame being drawn.
#[derive(Debug, Default, PartialEq)]
struct Cover {
    /// The page goes behind its snapshot.
    whole: bool,
    /// Else, where the board's chrome lies over the page (window px): the
    /// page is cut away there and lets the pointer through.
    holes: Vec<Bounds<Pixels>>,
}

/// What, in the frame being drawn, blocks the pointer from the probe over
/// the visible part of the page (`Hitbox::blocks_in_frame`), or a tooltip
/// over it: a menu, a popover, a modal or a tooltip covers the page; the
/// board's chrome cuts holes in it, where a page can have holes.
fn cover(probe: &Hitbox, visible: Bounds<Pixels>, window: &Window) -> Cover {
    let (w, h) = (f32::from(visible.size.width), f32::from(visible.size.height));
    if w < 2.0 || h < 2.0 {
        return Cover::default();
    }
    // GPUI's tooltips take no pointer, so no hitbox says they're there.
    if window.tooltip_bounds().is_some_and(|tip| tip.intersects(&visible)) {
        return Cover {
            whole: true,
            holes: Vec::new(),
        };
    }
    let blocks = probe.blocks_in_frame(visible, window);
    if blocks.is_empty() {
        return Cover::default();
    }
    // WebView2 pages can't have holes yet.
    let whole = !cfg!(target_os = "macos")
        || blocks.iter().any(|b| b.deferred)
        || blocks.len() > MAX_HOLES
        || uncovered(visible, blocks.iter().map(|b| b.bounds)).is_empty();
    Cover {
        whole,
        holes: if whole {
            Vec::new()
        } else {
            blocks.into_iter().map(|b| b.bounds).collect()
        },
    }
}

/// What of `area` the `holes` leave, as boxes that don't overlap (none
/// thinner than a pixel).
pub fn uncovered(area: Bounds<Pixels>, holes: impl IntoIterator<Item = Bounds<Pixels>>) -> Vec<Bounds<Pixels>> {
    let mut left = vec![area];
    for hole in holes {
        let mut next = Vec::with_capacity(left.len() + 3);
        for piece in left {
            if !piece.intersects(&hole) {
                next.push(piece);
                continue;
            }
            let (top, bottom) = (piece.top(), piece.bottom());
            let (inner_top, inner_bottom) = (hole.top().max(top), hole.bottom().min(bottom));
            // Above the hole, below it, then beside it between the two.
            let strips = [
                Bounds::from_corners(piece.origin, point(piece.right(), inner_top)),
                Bounds::from_corners(point(piece.left(), inner_bottom), piece.bottom_right()),
                Bounds::from_corners(
                    point(piece.left(), inner_top),
                    point(hole.left().max(piece.left()), inner_bottom),
                ),
                Bounds::from_corners(
                    point(hole.right().min(piece.right()), inner_top),
                    point(piece.right(), inner_bottom),
                ),
            ];
            next.extend(
                strips
                    .into_iter()
                    .filter(|s| s.size.width >= px(1.) && s.size.height >= px(1.)),
            );
        }
        left = next;
    }
    left
}

/// Whether every page that should be behind a picture has one that's
/// current (the test driver waits for it before a shot).
pub fn pictures_ready(cx: &App) -> bool {
    let Some(rc) = super::frames::existing(cx).filter(|_| cfg!(target_os = "macos")) else {
        return true;
    };
    let f = rc.borrow();
    f.frames
        .iter()
        .filter(|fr| fr.drawn && fr.want)
        .all(|fr| !fr.live && !fr.pending && !fr.dirty && fr.snapshot.is_some())
}

/// The page changed (a message from it, an event to it): a snapshot shown
/// for it is taken again.
pub fn page_changed(frame: u64, cx: &mut App) {
    let Some(rc) = super::frames::existing(cx) else { return };
    let window = {
        let mut f = rc.borrow_mut();
        let Some(fr) = f.by_id_mut(frame) else { return };
        fr.dirty = true;
        f.window
    };
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
}

/// The page posted `loaded` (VIEW-046): a first page being revealed can be
/// pictured now (MOTION-033).
pub fn page_loaded(frame: u64, cx: &mut App) {
    let Some(rc) = super::frames::existing(cx) else { return };
    let mut f = rc.borrow_mut();
    if let Some(fr) = f.by_id_mut(frame) {
        fr.loaded = true;
    }
}

/// The shell drew its page at scale `n`: a page waiting behind its snapshot
/// for it can come back.
pub fn page_scaled(frame: u64, n: u64, cx: &mut App) {
    let Some(rc) = super::frames::existing(cx) else { return };
    let window = {
        let mut f = rc.borrow_mut();
        let Some(fr) = f.by_id_mut(frame) else { return };
        fr.web.scale_drawn(n);
        if fr.live {
            return;
        }
        f.window
    };
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
}

/// One of the card's tooltips came up over the page, or went.
pub fn set_tooltip(view_id: &str, shown: bool, cx: &mut App) {
    let Some(rc) = super::frames::existing(cx) else { return };
    let window = {
        let mut f = rc.borrow_mut();
        let Some(fr) = f.get_mut(view_id, Slot::Card) else {
            return;
        };
        if fr.tooltip == shown {
            return;
        }
        fr.tooltip = shown;
        f.window
    };
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn area(pieces: &[Bounds<Pixels>]) -> f32 {
        pieces
            .iter()
            .map(|p| f32::from(p.size.width) * f32::from(p.size.height))
            .sum()
    }

    #[test]
    fn leaves_what_the_holes_dont_cover() {
        let page = b(0., 0., 100., 80.);
        assert_eq!(uncovered(page, []), vec![page]);
        // The prompt bar over its bottom edge, the dock over its left.
        let left = uncovered(page, [b(30., 70., 50., 30.), b(-10., 20., 15., 40.)]);
        assert!((area(&left) - (8000. - 500. - 200.)).abs() < 0.01, "{left:?}");
        for (i, p) in left.iter().enumerate() {
            for q in &left[i + 1..] {
                assert!(!p.intersects(q), "{p:?} overlaps {q:?}");
            }
        }
        // A button inside the bar cuts nothing more.
        let more = uncovered(
            page,
            [b(30., 70., 50., 30.), b(-10., 20., 15., 40.), b(40., 72., 10., 6.)],
        );
        assert!((area(&more) - area(&left)).abs() < 0.01);
        // Covered all over: nothing left.
        assert!(uncovered(page, [b(-1., -1., 60., 90.), b(50., -1., 60., 90.)]).is_empty());
    }
}
