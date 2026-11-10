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
/// body, so a card half off screen doesn't reflow its page. Above the
/// webview, the ink over the page (`mac_ink.rs`).
pub struct Container {
    view: Retained<HostView>,
    ink: super::mac_ink::InkOverlay,
    /// Where the ink's picture goes, in the body's px (x, y, w, h).
    ink_rect: Cell<Option<(f64, f64, f64, f64)>>,
    /// The body's top left in the container (from its top), and the
    /// container's height: what places the ink in it.
    body: Cell<(f64, f64, f64)>,
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
        parent.addSubview(&view);
        let ink = super::mac_ink::InkOverlay::new(mtm);
        view.addSubview(ink.view());
        Some(Self {
            view,
            ink,
            ink_rect: Cell::new(None),
            body: Cell::new((0.0, 0.0, 0.0)),
        })
    }

    /// Moves a view (the WKWebView) into the container, under the ink.
    pub fn adopt(&self, child: &NSView) {
        child.removeFromSuperview();
        // SAFETY: both are views; NSWindowBelow (-1) relative to the ink.
        unsafe {
            let _: () = msg_send![&*self.view, addSubview: child, positioned: -1isize, relativeTo: self.ink.view()];
        }
    }

    /// Shows the ink over the page (None: none crosses it).
    pub fn set_ink(&self, ink: Option<&super::ink::PageInk>) {
        let image = ink.and_then(super::mac_ink::render);
        let rect = ink.filter(|_| image.is_some()).map(|i| i.rect);
        self.ink_rect.set(rect);
        let scale = ink.map_or(2.0, |i| i.scale);
        self.ink.show(image.as_deref(), self.ink_frame(), scale);
    }

    /// The ink's frame in the container's coordinates.
    fn ink_frame(&self) -> NSRect {
        let Some((l, t, w, h)) = self.ink_rect.get() else {
            return NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        };
        let (dx, dy, vh) = self.body.get();
        let top = dy + t;
        let y = if self.view.isFlipped() { top } else { vh - top - h };
        NSRect::new(NSPoint::new(dx + l, y), NSSize::new(w, h))
    }

    pub fn set_passthrough(&self, on: bool) {
        self.view.ivars().passthrough.set(on);
    }

    pub fn set_edge(&self, edge: f64) {
        self.view.ivars().edge.set(edge);
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
    /// they're on screen.
    pub fn set_geometry(&self, child: &NSView, visible: Bounds<Pixels>, document: Bounds<Pixels>, radius: f32) {
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
        let (cw, ch) = (dw, dh);
        let child_y = if self.view.isFlipped() { dy } else { vh - dy - ch };
        let child_frame = NSRect::new(NSPoint::new(dx, child_y), NSSize::new(cw, ch));
        if child.frame() != child_frame {
            child.setFrame(child_frame);
        }
        self.body.set((dx, dy, vh));
        if self.ink_rect.get().is_some() {
            self.ink.place(self.ink_frame());
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
pub fn snapshot(webview: &NSView, done: impl FnOnce(Option<(u32, u32, Vec<u8>)>) + 'static) {
    let done = std::cell::RefCell::new(Some(done));
    let block = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
        let Some(done) = done.borrow_mut().take() else { return };
        done(if image.is_null() { None } else { pixels(image) });
    });
    // SAFETY: a WKWebView; a nil configuration snapshots its whole bounds.
    unsafe {
        let _: () = msg_send![webview, takeSnapshotWithConfiguration: std::ptr::null_mut::<AnyObject>(), completionHandler: &*block];
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

/// Whether the page has the window's keyboard: its webview, or a view inside
/// it, is the window's first responder. A click in the page makes it so; a
/// click anywhere else on the board takes it away (VIEW-051).
pub fn has_keyboard(webview: &NSView) -> bool {
    let Some(window) = webview.window() else {
        return false;
    };
    let Some(responder) = window.firstResponder() else {
        return false;
    };
    // SAFETY: isKindOfClass: on a live responder; a view's superview chain.
    unsafe {
        let is_view: bool = msg_send![&*responder, isKindOfClass: objc2::class!(NSView)];
        if !is_view {
            return false;
        }
        let mut view: Option<Retained<NSView>> = Some(Retained::cast_unchecked(responder));
        while let Some(v) = view {
            if std::ptr::eq(&*v, webview) {
                return true;
            }
            view = v.superview();
        }
    }
    false
}

/// No WebKit context menu in pages: Electron showed none in a view's frame.
/// WebKit builds its menu, then asks the view `willOpenMenu:withEvent:`;
/// emptied there, nothing opens. Done once for wry's webview class.
pub fn no_context_menu(webview: &NSView) {
    static DONE: std::sync::Once = std::sync::Once::new();
    unsafe extern "C-unwind" fn will_open_menu(
        _this: *mut AnyObject,
        _cmd: objc2::runtime::Sel,
        menu: *mut AnyObject,
        _event: *mut AnyObject,
    ) {
        if !menu.is_null() {
            // SAFETY: the NSMenu AppKit is about to show.
            unsafe {
                let _: () = msg_send![menu, removeAllItems];
            }
        }
    }
    let mut class = webview.class();
    // Key-value observing swaps in a subclass; the method goes on wry's own.
    if class.name().to_bytes().starts_with(b"NSKVONotifying_")
        && let Some(sup) = class.superclass()
    {
        class = sup;
    }
    let class = std::ptr::from_ref(class).cast_mut();
    DONE.call_once(|| {
        // SAFETY: an IMP with the selector's signature (void, id, SEL, id,
        // id), on a class the runtime owns; "v@:@@" says so.
        unsafe {
            let imp: objc2::runtime::Imp = std::mem::transmute(
                will_open_menu
                    as unsafe extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel, *mut AnyObject, *mut AnyObject),
            );
            objc2::ffi::class_replaceMethod(class, objc2::sel!(willOpenMenu:withEvent:), imp, c"v@:@@".as_ptr());
        }
    });
}

/// Gives the keyboard back to GPUI's view after a press on the board: AppKit
/// leaves it with the page the user last clicked, where a browser would move
/// the focus off the frame. On the main queue, off any GPUI update (the
/// responder change calls back into GPUI).
pub fn keyboard_to_board(window_number: isize) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: NSApplication and NSWindow on the main thread, checked
        // for nil; makeFirstResponder: takes the window's content view.
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            let w: *mut AnyObject = msg_send![app, windowWithWindowNumber: window_number];
            if w.is_null() {
                return;
            }
            let content: *mut AnyObject = msg_send![w, contentView];
            if !content.is_null() {
                let _: Bool = msg_send![w, makeFirstResponder: content];
            }
        }
    });
}

/// Test driver: a left click at a point in the window through
/// `-[NSWindow sendEvent:]`, so AppKit's own hit test picks the view (a
/// page, or GPUI's). Runs off any GPUI update, on the main queue; never
/// moves the user's pointer.
pub fn native_click(window_number: isize, x: f64, y: f64) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: NSApplication, NSWindow and NSEvent on the main thread;
        // every object is checked for nil before use.
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            let w: *mut AnyObject = msg_send![app, windowWithWindowNumber: window_number];
            if w.is_null() {
                return;
            }
            let content: *mut AnyObject = msg_send![w, contentView];
            if content.is_null() {
                return;
            }
            let frame: NSRect = msg_send![content, frame];
            let loc = NSPoint::new(x, frame.size.height - y);
            for (ty, pressure) in [(1usize, 1.0f32), (2usize, 0.0f32)] {
                let ev: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                    mouseEventWithType: ty,
                    location: loc,
                    modifierFlags: 0usize,
                    timestamp: 0.0f64,
                    windowNumber: window_number,
                    context: std::ptr::null_mut::<AnyObject>(),
                    eventNumber: 0isize,
                    clickCount: 1isize,
                    pressure: pressure];
                if !ev.is_null() {
                    let _: () = msg_send![w, sendEvent: ev];
                }
            }
        }
    });
}

/// Test driver: types `text` (lowercase letters and spaces) as AppKit
/// delivers a keyboard's keys, a key down and up each through the window's
/// `sendEvent:`, so they take the platform's path (key handlers first, then
/// the text input client), as real typing does.
pub fn native_keys(window_number: isize, text: String) {
    fn key_code(c: char) -> u16 {
        const CODES: &str = "asdfhgzxcv_bqweryt123465=97-80]ou[ip_lj'k;\\,/nm.";
        if c == ' ' {
            return 49;
        }
        CODES.find(c).map_or(0, |i| i as u16)
    }
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: NSApplication, NSWindow and NSEvent on the main thread;
        // every object is checked for nil before use.
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            let w: *mut AnyObject = msg_send![app, windowWithWindowNumber: window_number];
            if w.is_null() {
                return;
            }
            for c in text.chars() {
                let chars = objc2_foundation::NSString::from_str(&c.to_string());
                for ty in [10usize, 11usize] {
                    let ev: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                        keyEventWithType: ty,
                        location: NSPoint::new(0.0, 0.0),
                        modifierFlags: 0usize,
                        timestamp: 0.0f64,
                        windowNumber: window_number,
                        context: std::ptr::null_mut::<AnyObject>(),
                        characters: &*chars,
                        charactersIgnoringModifiers: &*chars,
                        isARepeat: false,
                        keyCode: key_code(c)];
                    if !ev.is_null() {
                        let _: () = msg_send![w, sendEvent: ev];
                    }
                }
            }
        }
    });
}

/// Test driver: the class of the view AppKit would hand a click at a point
/// in the window (a page's `WryWebView`, or GPUI's view).
pub fn hit_class(window: &Window, x: f64, y: f64) -> String {
    let Some(content) = content_view(window) else {
        return "-".into();
    };
    let py = if content.isFlipped() {
        y
    } else {
        content.frame().size.height - y
    };
    let p = NSPoint::new(x, py);
    // SAFETY: hitTest: takes a point in the superview's coordinates, which
    // convertPoint:toView: gives from the content view's own.
    unsafe {
        let p = match content.superview() {
            Some(sup) => content.convertPoint_toView(p, Some(&sup)),
            None => p,
        };
        let hit: *mut AnyObject = msg_send![&*content, hitTest: p];
        if hit.is_null() {
            return "nil".into();
        }
        (*hit).class().name().to_string_lossy().into_owned()
    }
}
