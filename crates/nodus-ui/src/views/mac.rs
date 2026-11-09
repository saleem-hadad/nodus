//! AppKit glue for views' webviews (spike S1 step 4): the container that
//! clips a WKWebView to the visible part of its card with the card's rounded
//! corners and lets the pointer through on demand, and page snapshots for
//! GPUI to draw while something covers the page (decision D3).
//!
//! AppKit calls that dispatch events synchronously must run outside GPUI
//! update closures; nothing here does (frames and alpha only).

use std::cell::Cell;

use gpui::{Bounds, Pixels, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSView};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// GPUI's content view, the webviews' parent.
pub fn content_view(window: &Window) -> Option<Retained<NSView>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the pointer is GPUI's live content view, retained for the caller.
    unsafe { Retained::retain(h.ns_view.as_ptr().cast::<NSView>()) }
}

pub struct HostIvars {
    /// The pointer goes to GPUI underneath, not the page.
    passthrough: Cell<bool>,
    /// The pointer goes to GPUI this close to the page's left, right and
    /// bottom edges (the card's resize handles, which straddle them).
    edge: Cell<f64>,
    /// The page on screen, in the container's coordinates (x, y from the
    /// top, w, h): the webview itself is larger when scaled down.
    page: Cell<(f64, f64, f64, f64)>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements this breaks; no Drop.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NodusViewHostView"]
    #[ivars = HostIvars]
    pub struct HostView;

    impl HostView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            if self.ivars().passthrough.get() {
                return std::ptr::null_mut();
            }
            let edge = self.ivars().edge.get();
            if edge > 0.0 {
                // `point` is in the superview's coordinates.
                // SAFETY: the container's superview is GPUI's view, alive.
                let local = self.convertPoint_fromView(point, unsafe { self.superview() }.as_deref());
                let (x, y, w, h) = self.ivars().page.get();
                let from_top = if self.isFlipped() {
                    local.y
                } else {
                    self.frame().size.height - local.y
                };
                if local.x - x < edge || x + w - local.x < edge || y + h - from_top < edge {
                    return std::ptr::null_mut();
                }
            }
            // SAFETY: NSView's own hit test, on this view.
            unsafe { msg_send![super(self), hitTest: point] }
        }
    }
);

/// A container view inside GPUI's content view holding one WKWebView: its
/// frame is the visible part of the card's body, the webview's the whole
/// body, so a card half off screen doesn't reflow its page.
pub struct Container {
    view: Retained<HostView>,
}

impl Container {
    pub fn new(parent: &NSView) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let this = HostView::alloc(mtm).set_ivars(HostIvars {
            passthrough: Cell::new(false),
            edge: Cell::new(0.0),
            page: Cell::new((0.0, 0.0, 0.0, 0.0)),
        });
        let zero = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        // SAFETY: NSView's designated initialiser.
        let view: Retained<HostView> = unsafe { msg_send![super(this), initWithFrame: zero] };
        view.setWantsLayer(true);
        // SAFETY: a layer-backed view has a layer; masksToBounds takes a BOOL.
        unsafe {
            let layer: *mut AnyObject = msg_send![&*view, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
            }
        }
        view.setHidden(true);
        parent.addSubview(&view);
        Some(Self { view })
    }

    /// Moves a view (the WKWebView) into the container.
    pub fn adopt(&self, child: &NSView) {
        child.removeFromSuperview();
        self.view.addSubview(child);
    }

    pub fn set_passthrough(&self, on: bool) {
        self.view.ivars().passthrough.set(on);
    }

    pub fn set_edge(&self, edge: f64) {
        self.view.ivars().edge.set(edge);
    }

    pub fn set_hidden(&self, hidden: bool) {
        if self.view.isHidden() != hidden {
            self.view.setHidden(hidden);
        }
    }

    /// Fully transparent while GPUI draws a snapshot in its place; the page
    /// keeps rendering, so the next snapshot is current.
    pub fn set_alpha(&self, alpha: f64) {
        if (self.view.alphaValue() - alpha).abs() > f64::EPSILON {
            self.view.setAlphaValue(alpha);
        }
    }

    /// `visible` and `document` in GPUI window coordinates (logical px, from
    /// the top left). `radius` rounds the document's bottom corners where
    /// they're on screen. `scale` is the view scale the webview draws at: its
    /// frame is the document unscaled, from the same top left.
    pub fn set_geometry(
        &self,
        child: &NSView,
        visible: Bounds<Pixels>,
        document: Bounds<Pixels>,
        radius: f32,
        scale: f64,
    ) {
        // SAFETY: the container was added to GPUI's view and stays there.
        let Some(parent) = (unsafe { self.view.superview() }) else {
            return;
        };
        let ph = parent.frame().size.height;
        let (vx, vy, vw, vh) = (
            f64::from(f32::from(visible.origin.x)),
            f64::from(f32::from(visible.origin.y)),
            f64::from(f32::from(visible.size.width)),
            f64::from(f32::from(visible.size.height)),
        );
        let y = if parent.isFlipped() { vy } else { ph - vy - vh };
        let frame = NSRect::new(NSPoint::new(vx, y), NSSize::new(vw, vh));
        if self.view.frame() != frame {
            self.view.setFrame(frame);
        }
        let dx = f64::from(f32::from(document.origin.x)) - vx;
        let dy = f64::from(f32::from(document.origin.y)) - vy;
        let (dw, dh) = (
            f64::from(f32::from(document.size.width)),
            f64::from(f32::from(document.size.height)),
        );
        self.view.ivars().page.set((dx, dy, dw, dh));
        let (cw, ch) = (dw / scale, dh / scale);
        let child_y = if self.view.isFlipped() { dy } else { vh - dy - ch };
        let child_frame = NSRect::new(NSPoint::new(dx, child_y), NSSize::new(cw, ch));
        if child.frame() != child_frame {
            child.setFrame(child_frame);
        }
        // Round only the bottom corners that are on screen.
        let eps = gpui::px(0.5);
        let bottom = (visible.bottom() - document.bottom()).abs() < eps;
        let left = (visible.left() - document.left()).abs() < eps;
        let right = (visible.right() - document.right()).abs() < eps;
        // CALayer corners in an unflipped layer: MinXMinY = 1 (bottom left),
        // MaxXMinY = 2 (bottom right).
        let mut mask: usize = 0;
        if radius > 0.0 && bottom && left {
            mask |= 1;
        }
        if radius > 0.0 && bottom && right {
            mask |= 2;
        }
        // SAFETY: the container is layer-backed; both setters take scalars.
        unsafe {
            let layer: *mut AnyObject = msg_send![&*self.view, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: f64::from(radius)];
                let _: () = msg_send![layer, setMaskedCorners: mask];
            }
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
    }
}

/// Scales the page with the board: WebKit's view scale lays it out at the
/// webview's frame and draws it scaled from the top left, as Chromium drew
/// the 0.9.4 iframe under the board's CSS transform. Page zoom would lay it
/// out too, but bumps zoomed text up to WebKit's 9px "smart minimum".
/// False when this WebKit has no view scale.
pub fn set_view_scale(webview: &NSView, scale: f64) -> bool {
    let sel = objc2::sel!(_setViewScale:);
    // SAFETY: respondsToSelector: on a live NSObject.
    let has: bool = unsafe { msg_send![webview, respondsToSelector: sel] };
    if has && scale.is_finite() && scale > 0.0 {
        // SAFETY: WKWebView's `-_setViewScale:(CGFloat)`, checked above.
        unsafe {
            let _: () = msg_send![webview, _setViewScale: scale];
        }
    }
    has
}

/// Keeps the page drawing while the window is covered: WebKit otherwise
/// stops rendering an occluded window's pages, and a snapshot then comes
/// back blank. Chromium kept 0.9.4's frames drawing, on screen or not.
pub fn ignore_window_occlusion(webview: &NSView) {
    let sel = objc2::sel!(_setWindowOcclusionDetectionEnabled:);
    // SAFETY: respondsToSelector: on a live NSObject.
    let has: bool = unsafe { msg_send![webview, respondsToSelector: sel] };
    if has {
        // SAFETY: WKWebView's `-_setWindowOcclusionDetectionEnabled:(BOOL)`, checked above.
        unsafe {
            let _: () = msg_send![webview, _setWindowOcclusionDetectionEnabled: Bool::NO];
        }
    }
}

/// `-[WKWebView takeSnapshotWithConfiguration:completionHandler:]`, as BGRA
/// pixels in sRGB (premultiplied), for a GPUI image. `done` runs on the main
/// thread, with None when there's no picture.
/// `rect`: the part to take, from the top left (the scaled page), else all.
pub fn snapshot(webview: &NSView, rect: Option<(f64, f64)>, done: impl FnOnce(Option<(u32, u32, Vec<u8>)>) + 'static) {
    let done = std::cell::RefCell::new(Some(done));
    let block = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
        let Some(done) = done.borrow_mut().take() else { return };
        done(if image.is_null() { None } else { pixels(image) });
    });
    // SAFETY: a WKWebView; a nil configuration snapshots its whole bounds,
    // else WKSnapshotConfiguration's `rect` is in the view's coordinates.
    unsafe {
        let config: *mut AnyObject = match rect {
            Some((w, h)) => {
                let config: *mut AnyObject = msg_send![objc2::class!(WKSnapshotConfiguration), new];
                let r = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
                let _: () = msg_send![config, setRect: r];
                config
            }
            None => std::ptr::null_mut(),
        };
        let _: () = msg_send![webview, takeSnapshotWithConfiguration: config, completionHandler: &*block];
        if !config.is_null() {
            let _: () = msg_send![config, release];
        }
    }
}

/// An NSImage's best bitmap, drawn into an sRGB BGRA buffer.
fn pixels(image: *mut AnyObject) -> Option<(u32, u32, Vec<u8>)> {
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, kCGColorSpaceSRGB};
    // SAFETY: `image` is the NSImage WebKit handed the completion handler;
    // CGImageForProposedRect returns an image it owns (autoreleased).
    let cg: *mut CGImage = unsafe {
        msg_send![image,
            CGImageForProposedRect: std::ptr::null_mut::<NSRect>(),
            context: std::ptr::null_mut::<AnyObject>(),
            hints: std::ptr::null_mut::<AnyObject>()]
    };
    // SAFETY: null or a valid CGImage for the rest of this call.
    let cg = unsafe { cg.as_ref() }?;
    let (w, h) = (CGImage::width(Some(cg)), CGImage::height(Some(cg)));
    if w == 0 || h == 0 {
        return None;
    }
    // SAFETY: a constant CoreGraphics exports.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    let mut buf = vec![0u8; w * h * 4];
    // kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little: BGRA.
    let info = 2 | (2 << 12);
    // SAFETY: `buf` holds w×h×4 bytes and outlives the context.
    let ctx = unsafe { CGBitmapContextCreate(buf.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), info) }?;
    let rect = objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(w as f64, h as f64),
    );
    CGContext::draw_image(Some(&ctx), rect, Some(cg));
    drop(ctx);
    Some((w as u32, h as u32, buf))
}
