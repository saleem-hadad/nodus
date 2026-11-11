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
//! the body's probe (`frames::probe`), or a tooltip over it, covers it. Every
//! hitbox of a frame is in by then, so a page that a menu or a dialog opens
//! over steps aside in the frame that first shows it, and that frame is
//! presented with the webview change (`Window::present_with_native_views`),
//! as an iframe under 0.9.4's menus was covered the frame they opened. A
//! snapshot is kept ready for that, taken again a while after the page last
//! changed; a page with none yet steps aside once one is taken.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, Bounds, Hitbox, Pixels, RenderImage, Window, point, px};

use super::frames::{Frame, Geometry, Slot, frames};

/// The zoom must hold this long before the page follows it live.
const ZOOM_SETTLE: Duration = Duration::from_millis(180);
/// A page whose body changed size gets this long to lay out again.
const RELAYOUT: Duration = Duration::from_millis(150);
/// A snapshot shown this long while the page may have changed is taken again.
const STALE_AFTER: Duration = Duration::from_millis(400);
/// A live page that changed gets a new snapshot at most this often, ready
/// for whatever covers it next.
const READY_EVERY: Duration = Duration::from_millis(1000);
/// A snapshot WebKit couldn't take (a page not drawn yet) is tried again
/// after this, twice as long after each failure in a row, up to 16 times.
const RETRY: Duration = Duration::from_millis(120);

/// A frame's probe painted: its webview follows the body, what covers the
/// page this frame is decided, and the snapshot to draw in the page's place
/// (if it's behind one) comes back for the probe to paint.
pub fn placed(
    view_id: &str,
    slot: Slot,
    g: Geometry,
    hitbox: Hitbox,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    let parity = crate::parity::frozen(cx);
    let rc = frames(cx);
    let mut f = rc.borrow_mut();
    let schedule = !f.check_scheduled;
    f.check_scheduled = true;
    let ink = f.ink.clone();
    let mut paint = None;
    let mut swapped = false;
    if let Some(fr) = f.get_mut(view_id, slot) {
        let now = Instant::now();
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
            fr.web.place(g.document, g.clip, g.zoom, g.radius, window.ui_zoom());
        }
        fr.web.set_edge(g.edge);
        let visible = g.document.intersect(&g.clip);
        fr.occluded = !fr.web.offscreen() && covered(&hitbox, visible, window);
        fr.probe = Some(hitbox);
        fr.want = parity || g.forced || fr.occluded || fr.tooltip || zoom_moving;
        if fr.want && fr.live && fr.snapshot.is_some() {
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
        if !fr.live {
            paint = fr.snapshot.clone();
        }
    }
    drop(f);
    if swapped {
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
                fr.web.place(g.document, Bounds::default(), g.zoom, g.radius, ui);
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
            if (needed || ready) && !fr.pending {
                let retrying = fr.retry_at.is_some_and(|t| now < t);
                // A body that just changed size lays its page out again
                // first; a picture now would be of the old layout.
                let relaying = fr.resized.is_some_and(|t| now.duration_since(t) < RELAYOUT);
                let stale = fr.snapshot_at.is_none_or(|t| now.duration_since(t) >= STALE_AFTER);
                if retrying || relaying || (needed && !fr.live && !stale && !parity) {
                    settle = true;
                } else {
                    shoot.push(fr.id);
                }
            }
            if !fr.want && !fr.live {
                fr.web.place(g.document, g.clip, g.zoom, g.radius, ui);
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

/// Whether, in the frame being drawn, anything drawn after the probe blocks
/// the pointer somewhere over the visible part of the page, or a tooltip
/// is over it.
fn covered(probe: &Hitbox, visible: Bounds<Pixels>, window: &Window) -> bool {
    let (x, y) = (f32::from(visible.origin.x), f32::from(visible.origin.y));
    let (w, h) = (f32::from(visible.size.width), f32::from(visible.size.height));
    if w < 2.0 || h < 2.0 {
        return false;
    }
    // GPUI's tooltips take no pointer, so no hitbox says they're there.
    if window.tooltip_bounds().is_some_and(|tip| tip.intersects(&visible)) {
        return true;
    }
    let cols = ((w / 14.0).ceil() as usize).clamp(2, 64);
    let rows = ((h / 14.0).ceil() as usize).clamp(2, 64);
    let inset = 0.75;
    for r in 0..=rows {
        let py = y + inset + (h - 2.0 * inset) * r as f32 / rows as f32;
        for c in 0..=cols {
            let px_ = x + inset + (w - 2.0 * inset) * c as f32 / cols as f32;
            if !probe.is_hovered_at_in_frame(point(px(px_), px(py)), window) {
                return true;
            }
        }
    }
    false
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
