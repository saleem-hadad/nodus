//! AppKit glue through objc2: reach the NSView/NSWindow behind a GPUI window.
use gpui::Window;
use objc2::{DefinedClass as _, msg_send};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_app_kit::{NSView, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// GPUI's content NSView (what raw-window-handle hands out on macOS).
pub fn ns_view(window: &Window) -> Retained<NSView> {
    let handle = HasWindowHandle::window_handle(window).expect("window handle");
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { panic!("not AppKit") };
    let ptr = h.ns_view.as_ptr() as *mut NSView;
    unsafe { Retained::retain(ptr).expect("ns_view") }
}

pub fn ns_window(window: &Window) -> Retained<NSWindow> {
    ns_view(window).window().expect("view has a window")
}

pub fn window_number(window: &Window) -> isize {
    ns_window(window).windowNumber()
}

/// Force this window's appearance (simulates the system light/dark switch for one window,
/// without touching the user's system setting). GPUI sees it via `viewDidChangeEffectiveAppearance`.
pub fn force_appearance(window: &Window, dark: bool) {
    let w = ns_window(window);
    unsafe {
        let name = NSString::from_str(if dark { "NSAppearanceNameDarkAqua" } else { "NSAppearanceNameAqua" });
        let appearance: *mut AnyObject = msg_send![objc2::class!(NSAppearance), appearanceNamed: &*name];
        let _: () = msg_send![&*w, setAppearance: appearance];
    }
}

/// Floating capture panel: what Electron's `type: 'panel'` + `setVisibleOnAllWorkspaces(true,
/// {visibleOnFullScreen})` + `setAlwaysOnTop(true, 'pop-up-menu')` do. GPUI's `WindowKind::PopUp`
/// already creates an NSPanel subclass with NonactivatingPanel, level 101 and
/// CanJoinAllSpaces|FullScreenAuxiliary; we re-assert and add the rest.
pub fn configure_floating_panel(window: &Window) {
    let w = ns_window(window);
    unsafe {
        // setFloatingPanel:YES resets the level to NSFloatingWindowLevel (3), so it goes first.
        let _: () = msg_send![&*w, setFloatingPanel: Bool::YES];
        // NSPopUpMenuWindowLevel = 101 (Electron 'pop-up-menu').
        w.setLevel(101);
        w.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        // We draw our own rounded card + shadow inside a transparent margin (like .cap-stage).
        w.setHasShadow(false);
        w.setOpaque(false);
        let clear: *mut AnyObject = msg_send![objc2::class!(NSColor), clearColor];
        let _: () = msg_send![&*w, setBackgroundColor: clear];
        let _: () = msg_send![&*w, setHidesOnDeactivate: Bool::NO];
        // Key without activating the app (NSNonactivatingPanelMask is set by GPUI for PopUp).
        let _: () = msg_send![&*w, setBecomesKeyOnlyIfNeeded: Bool::NO];
        let _: () = msg_send![&*w, setMovable: Bool::NO];
    }
}

pub fn panel_hide(window: &Window) {
    let w = ns_window(window);
    w.orderOut(None);
}

pub fn panel_show(window: &Window) {
    let w = ns_window(window);
    panel_show_ns(&w);
}

/// Show the non-activating panel and give it the keyboard. The frontmost app stays frontmost
/// (keeps the menu bar); NSApp.isActive reads YES while the panel is key.
pub fn panel_show_ns(w: &NSWindow) {
    w.orderFrontRegardless();
    w.makeKeyWindow();
}

pub fn describe_window(window: &Window) -> String {
    let w = ns_window(window);
    let level = w.level();
    let behavior = w.collectionBehavior();
    let is_panel: bool = unsafe { msg_send![&*w, isKindOfClass: objc2::class!(NSPanel)] };
    let style = w.styleMask();
    let key = w.isKeyWindow();
    let visible = w.isVisible();
    let app_active: bool = unsafe {
        let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
        msg_send![app, isActive]
    };
    let front_is_us: bool = unsafe {
        let ws: *mut AnyObject = msg_send![objc2::class!(NSWorkspace), sharedWorkspace];
        let app: *mut AnyObject = msg_send![ws, frontmostApplication];
        let pid: i32 = if app.is_null() { -1 } else { msg_send![app, processIdentifier] };
        pid == std::process::id() as i32
    };
    format!(
        "frontmostIsUs={front_is_us} level={level} collectionBehavior={:#x} isPanel={is_panel} styleMask={:#x} key={key} visible={visible} appActive={app_active} windowNumber={}",
        behavior.0,
        style.0,
        w.windowNumber()
    )
}

// ---------------------------------------------------------------------------------------------
// Webview host view: clip + rounded corners + pointer pass-through
// ---------------------------------------------------------------------------------------------

pub struct ClipIvars {
    passthrough: std::cell::Cell<bool>,
}

objc2::define_class!(
    // SAFETY: NSView has no subclassing requirements we violate; no Drop impl.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "S1WebHostView"]
    #[ivars = ClipIvars]
    pub struct WebHostView;

    impl WebHostView {
        /// While the board drags/pans/resizes, return nil so AppKit routes the pointer (and the
        /// wheel) to GPUI's view underneath instead of the WKWebView.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            if self.ivars().passthrough.get() {
                return std::ptr::null_mut();
            }
            unsafe { msg_send![super(self), hitTest: point] }
        }
    }
);

/// A container NSView inside GPUI's content view. The WKWebView is re-parented into it, so it can
/// be clipped to the visible board rect (layer masksToBounds) and to the card's rounded corners
/// (cornerRadius + maskedCorners) while keeping its full document frame (no reflow when a card is
/// partly off-screen), and so pointer events can be passed through to GPUI on demand.
pub struct ClipContainer {
    pub view: Retained<WebHostView>,
}

impl ClipContainer {
    pub fn new(parent: &NSView) -> Self {
        let mtm = objc2::MainThreadMarker::new().expect("main thread");
        let this = mtm.alloc::<WebHostView>().set_ivars(ClipIvars { passthrough: std::cell::Cell::new(false) });
        let view: Retained<WebHostView> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.))] };
        view.setWantsLayer(true);
        unsafe {
            let layer: *mut AnyObject = msg_send![&*view, layer];
            let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
        }
        parent.addSubview(&view);
        Self { view }
    }

    pub fn set_passthrough(&self, on: bool) {
        self.view.ivars().passthrough.set(on);
    }

    /// `visible` and `document` are in GPUI window coordinates (logical px, top-left origin).
    /// `radius` rounds the document's bottom corners where they are not cut by the clip.
    pub fn set_geometry(&self, child: &NSView, visible: gpui::Bounds<gpui::Pixels>, document: gpui::Bounds<gpui::Pixels>, radius: f32) {
        let parent = unsafe { self.view.superview() }.expect("container parent");
        let ph = parent.frame().size.height;
        let flipped = parent.isFlipped();
        let (vx, vy, vw, vh) = (
            f64::from(visible.origin.x),
            f64::from(visible.origin.y),
            f64::from(visible.size.width),
            f64::from(visible.size.height),
        );
        let cy = if flipped { vy } else { ph - vy - vh };
        self.view.setFrame(NSRect::new(NSPoint::new(vx, cy), NSSize::new(vw, vh)));
        let dx = f64::from(document.origin.x) - vx;
        let dy_top = f64::from(document.origin.y) - vy;
        let dw = f64::from(document.size.width);
        let dh = f64::from(document.size.height);
        let child_y = if self.view.isFlipped() { dy_top } else { vh - dy_top - dh };
        child.setFrame(NSRect::new(NSPoint::new(dx, child_y), NSSize::new(dw, dh)));
        // Round only the document's bottom corners that are actually visible.
        let eps = gpui::px(0.5);
        let bottom_visible = (visible.bottom() - document.bottom()).abs() < eps;
        let left_visible = (visible.left() - document.left()).abs() < eps;
        let right_visible = (visible.right() - document.right()).abs() < eps;
        // CALayer corners (unflipped): MinXMinY=1 (bottom-left), MaxXMinY=2 (bottom-right).
        let mut mask: usize = 0;
        if bottom_visible && left_visible {
            mask |= 1;
        }
        if bottom_visible && right_visible {
            mask |= 2;
        }
        unsafe {
            let layer: *mut AnyObject = msg_send![&*self.view, layer];
            let _: () = msg_send![layer, setCornerRadius: radius as f64];
            let _: () = msg_send![layer, setMaskedCorners: mask];
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.view.setHidden(hidden);
    }
}

/// Class name of the NSView AppKit would deliver a click at `p` (GPUI window coords) to.
pub fn hit_test_class(window: &Window, p: gpui::Point<gpui::Pixels>) -> String {
    let w = ns_window(window);
    let Some(content) = w.contentView() else { return "-".into() };
    let h = content.frame().size.height;
    let win_pt = NSPoint::new(f64::from(p.x), h - f64::from(p.y));
    unsafe {
        let sup = content.superview();
        let pt_in_super = match &sup { Some(s) => s.convertPoint_fromView(win_pt, None), None => win_pt };
        let hit: *mut AnyObject = msg_send![&*content, hitTest: pt_in_super];
        if hit.is_null() {
            return "nil".into();
        }
        let cls = (*hit).class();
        cls.name().to_string_lossy().into_owned()
    }
}

/// Send a synthetic left click to the window at `p` (GPUI window coords) through
/// -[NSWindow sendEvent:], i.e. AppKit hit-testing and WebKit's real event path, without moving the
/// user's cursor.
/// Must be called OUTSIDE any GPUI update (AppKit dispatches synchronously, and if the hit view is
/// GPUI's it re-enters the App).
pub fn synthetic_click(w: &NSWindow, p: gpui::Point<gpui::Pixels>) {
    let Some(content) = w.contentView() else { return };
    let h = content.frame().size.height;
    let loc = NSPoint::new(f64::from(p.x), h - f64::from(p.y));
    let wn = w.windowNumber();
    unsafe {
        for (ty, pressure) in [(1usize, 1.0f32), (2usize, 0.0f32)] {
            let ev: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                mouseEventWithType: ty,
                location: loc,
                modifierFlags: 0usize,
                timestamp: 0.0f64,
                windowNumber: wn,
                context: std::ptr::null_mut::<AnyObject>(),
                eventNumber: 0isize,
                clickCount: 1isize,
                pressure: pressure];
            let _: () = msg_send![w, sendEvent: ev];
        }
    }
}

/// WKWebView -takeSnapshotWithConfiguration:completionHandler: -> BGRA pixels for a GPUI image.
pub fn snapshot_wkwebview(wk: &NSView, done: impl FnOnce(Option<(u32, u32, Vec<u8>)>) + 'static) {
    let done = std::cell::RefCell::new(Some(done));
    let block = block2::RcBlock::new(move |image: *mut AnyObject, _err: *mut AnyObject| {
        let Some(done) = done.borrow_mut().take() else { return };
        if image.is_null() {
            return done(None);
        }
        unsafe {
            let cg: *mut std::ffi::c_void = msg_send![image,
                CGImageForProposedRect: std::ptr::null_mut::<NSRect>(),
                context: std::ptr::null_mut::<AnyObject>(),
                hints: std::ptr::null_mut::<AnyObject>()];
            if cg.is_null() {
                return done(None);
            }
            let rep: *mut AnyObject = msg_send![objc2::class!(NSBitmapImageRep), alloc];
            let rep: *mut AnyObject = msg_send![rep, initWithCGImage: cg];
            let w: isize = msg_send![rep, pixelsWide];
            let h: isize = msg_send![rep, pixelsHigh];
            let bpr: isize = msg_send![rep, bytesPerRow];
            let spp: isize = msg_send![rep, samplesPerPixel];
            let bps: isize = msg_send![rep, bitsPerSample];
            let fmt: usize = msg_send![rep, bitmapFormat];
            let data: *const u8 = msg_send![rep, bitmapData];
            if data.is_null() || bps != 8 || spp < 3 {
                let _: () = msg_send![rep, release];
                return done(None);
            }
            let alpha_first = fmt & 1 != 0; // NSBitmapFormatAlphaFirst
            println!("S1 SNAPSHOT_REP w={w} h={h} bpr={bpr} spp={spp} bps={bps} fmt={fmt:#x}");
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                let row = data.add((y * bpr) as usize);
                for x in 0..w {
                    let px = row.add((x * spp) as usize);
                    let (r, g, b, a) = if spp == 4 {
                        if alpha_first { (*px.add(1), *px.add(2), *px.add(3), *px) } else { (*px, *px.add(1), *px.add(2), *px.add(3)) }
                    } else {
                        (*px, *px.add(1), *px.add(2), 255)
                    };
                    out.extend_from_slice(&[b, g, r, a]); // GPUI images are BGRA
                }
            }
            let _: () = msg_send![rep, release];
            done(Some((w as u32, h as u32, out)));
        }
    });
    unsafe {
        let _: () = msg_send![wk, takeSnapshotWithConfiguration: std::ptr::null_mut::<AnyObject>(), completionHandler: &*block];
    }
}

/// Spike-only: keep the main window out of Stage Manager (so it can be captured without
/// activating the app and pushing the user's own windows into the strip) and optionally park it
/// almost entirely off-screen so it doesn't cover anything the user is doing.
pub fn stage_exempt(window: &Window, park_offscreen: bool) {
    let w = ns_window(window);
    w.setCollectionBehavior(NSWindowCollectionBehavior::Auxiliary | NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle);
    if std::env::var("S1_LEVEL").is_ok() {
        w.setLevel(std::env::var("S1_LEVEL").unwrap().parse().unwrap_or(3));
    }
    w.orderFrontRegardless();
    if park_offscreen {
        if let Some(screen) = w.screen() {
            let sf = screen.frame();
            let fr = w.frame();
            w.setFrameOrigin(NSPoint::new(sf.origin.x + sf.size.width - 12., fr.origin.y));
        }
    }
}
