//! The snapshot swap (decision D3, spike S1 step 4). A native webview always
//! draws above everything GPUI paints, so whenever GPUI must draw over a
//! page (a note, a menu, a dialog, a toast, a tooltip, the marquee), or the
//! page moves with the board in a way the webview can't follow smoothly
//! (dragged, resized, zoomed), or an ink tool is out, the page goes
//! transparent and GPUI draws a snapshot of it in its place. It comes back
//! once nothing covers it.
//!
//! What covers a page is found after each frame from GPUI's own hit test:
//! anything drawn later that blocks the pointer over the body's probe
//! (`frames::probe`) covers it.

use std::time::{Duration, Instant};

use gpui::{App, Bounds, DispatchPhase, Hitbox, MouseDownEvent, Pixels, Window, point, px};

use super::frames::{Frame, Geometry, Slot, frames};

/// The zoom must hold this long before the page follows it live.
const ZOOM_SETTLE: Duration = Duration::from_millis(180);
/// A page whose body changed size gets this long to lay out again.
const RELAYOUT: Duration = Duration::from_millis(150);
/// A snapshot shown this long while the page may have changed is taken again.
const STALE_AFTER: Duration = Duration::from_millis(400);

/// A frame's probe painted: its webview follows the body, and after the
/// frame lands the swap is decided.
pub fn placed(view_id: &str, slot: Slot, g: Geometry, hitbox: Hitbox, window: &mut Window, cx: &mut App) {
    let rc = frames(cx);
    let mut f = rc.borrow_mut();
    let schedule = !f.check_scheduled;
    f.check_scheduled = true;
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
        fr.probe = Some(hitbox);
        // While the zoom moves, the page waits behind its snapshot; it lays
        // out once, at the zoom the board settles on.
        if !zooming(fr, now) || fr.live {
            fr.web.place(g.document, g.clip, g.zoom, g.radius);
        }
        fr.web.set_edge(g.edge);
        fr.web.set_passthrough(!fr.live || g.passthrough);
    }
    drop(f);
    if schedule {
        window.on_next_frame(after_frame);
        // A press anywhere GPUI draws takes the keyboard off the page the
        // user last clicked, as a click outside a frame blurs it (VIEW-051,
        // VIEW-149: the board's keys are the board's again).
        window.on_mouse_event(|_: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                keyboard_to_board(window, cx);
            }
        });
    }
}

fn keyboard_to_board(window: &mut Window, cx: &mut App) {
    let Some(rc) = super::frames::existing(cx) else { return };
    let any = rc.borrow().frames.iter().any(|fr| fr.web.has_keyboard());
    #[cfg(target_os = "macos")]
    if any && let Some(number) = crate::mac::window_number(window) {
        super::mac::keyboard_to_board(number);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (any, window);
}

fn zooming(fr: &Frame, now: Instant) -> bool {
    fr.zoom_changed.is_some_and(|t| now.duration_since(t) < ZOOM_SETTLE)
}

/// After a frame lands: hides frames nothing drew, finds what covers each
/// page, and swaps pages and snapshots.
fn after_frame(window: &mut Window, cx: &mut App) {
    let rc = frames(cx);
    let parity = crate::parity::frozen(cx);
    let now = Instant::now();
    let mut refresh = false;
    let mut settle = false;
    let mut shoot: Vec<u64> = Vec::new();
    {
        let mut f = rc.borrow_mut();
        f.check_scheduled = false;
        for fr in &mut f.frames {
            let Some(g) = fr.geometry else { continue };
            if !fr.drawn {
                // Culled or not drawn: off screen, but the page keeps running.
                fr.web.place(g.document, Bounds::default(), g.zoom, g.radius);
                continue;
            }
            let visible = g.document.intersect(&g.clip);
            fr.occluded = fr
                .probe
                .as_ref()
                .is_some_and(|probe| !fr.web.offscreen() && covered(probe, visible, window));
            let zoom_moving = zooming(fr, now);
            settle |= zoom_moving;
            let want = parity || g.forced || fr.occluded || fr.tooltip || zoom_moving;
            if want {
                let fresh = !fr.dirty && fr.snapshot.is_some();
                if fr.live && fresh {
                    go_snapshot(fr);
                    refresh = true;
                } else if fr.dirty && !fr.pending {
                    let stale = fr.snapshot_at.is_none_or(|t| now.duration_since(t) >= STALE_AFTER);
                    // A body that just changed size lays its page out again
                    // first; a picture now would be of the old layout.
                    let relaying = fr.resized.is_some_and(|t| now.duration_since(t) < RELAYOUT);
                    if relaying {
                        settle = true;
                    } else if fr.live || stale || parity {
                        shoot.push(fr.id);
                    } else {
                        settle = true;
                    }
                }
            } else if !fr.live {
                fr.web.place(g.document, g.clip, g.zoom, g.radius);
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

/// Takes a picture of a frame's page; once it's in, the frame shows it if
/// it still should.
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
            let old = {
                let mut f = rc2.borrow_mut();
                let Some(fr) = f.by_id_mut(id) else { return };
                fr.pending = false;
                let old = image.and_then(|image| {
                    fr.snapshot_at = Some(Instant::now());
                    fr.snapshot.replace(image)
                });
                // Where pages can't be pictured (WebView2 for now), the page
                // still steps aside so what covers it shows.
                if fr.live && (fr.snapshot.is_some() || cfg!(not(target_os = "macos"))) {
                    go_snapshot(fr);
                }
                old
            };
            // The old picture's texture goes from the atlas.
            if let Some(old) = old {
                cx.drop_image(old, None);
            }
            let _ = handle.update(cx, |_, window, _| window.refresh());
        });
    });
}

/// Whether anything drawn after the probe blocks the pointer somewhere over
/// the visible part of the page.
fn covered(probe: &Hitbox, visible: Bounds<Pixels>, window: &Window) -> bool {
    let (x, y) = (f32::from(visible.origin.x), f32::from(visible.origin.y));
    let (w, h) = (f32::from(visible.size.width), f32::from(visible.size.height));
    if w < 2.0 || h < 2.0 {
        return false;
    }
    let cols = ((w / 14.0).ceil() as usize).clamp(2, 64);
    let rows = ((h / 14.0).ceil() as usize).clamp(2, 64);
    let inset = 0.75;
    for r in 0..=rows {
        let py = y + inset + (h - 2.0 * inset) * r as f32 / rows as f32;
        for c in 0..=cols {
            let px_ = x + inset + (w - 2.0 * inset) * c as f32 / cols as f32;
            if !probe.is_hovered_at(point(px(px_), px(py)), window) {
                return true;
            }
        }
    }
    false
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
