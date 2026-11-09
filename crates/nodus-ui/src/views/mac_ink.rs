//! The native overlay that draws the ink over a live page (`views::ink`):
//! a view above the WKWebView in its clip container that never takes the
//! pointer (its hit test is nil), showing a picture of the marks drawn with
//! Core Graphics and Core Text as GPUI draws them around the page: round
//! caps and joins, the zoom's widths, the theme's colours in sRGB, the
//! selection's dashed outline, and labels in the system font at 600 with no
//! font smoothing (GPUI's patches 7 and 8). A page's snapshot is taken of
//! the webview alone, so it never holds the overlay.

use std::ptr;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSView};
use objc2_core_foundation::{
    CFAttributedString, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGAffineTransform, CGFloat,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColor, CGColorSpace, CGContext, CGImage, CGLineCap,
    CGLineJoin, kCGColorSpaceSRGB,
};
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontSymbolicTraits, CTFontUIFontType, CTLine, kCTFontAttributeName,
    kCTFontVariationAttribute, kCTForegroundColorFromContextAttributeName,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};

use super::ink::{PageInk, PageMark};
use nodus_board::ink_draw::Seg;

define_class!(
    // SAFETY: NSView has no subclassing requirements this breaks; no Drop.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NodusViewInkView"]
    pub struct InkView;

    impl InkView {
        /// The pointer goes through to the page, as it went through 0.9.4's
        /// ink layer (`pointer-events: none`).
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            ptr::null_mut()
        }
    }
);

/// A page's ink overlay.
pub struct InkOverlay {
    view: Retained<InkView>,
}

impl InkOverlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let zero = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        // SAFETY: NSView's designated initialiser.
        let view: Retained<InkView> = unsafe { msg_send![InkView::alloc(mtm), initWithFrame: zero] };
        view.setWantsLayer(true);
        view.setHidden(true);
        // SAFETY: NSViewLayerContentsRedrawNever (0): AppKit never redraws
        // the layer, whose contents are set here.
        unsafe {
            let _: () = msg_send![&*view, setLayerContentsRedrawPolicy: 0isize];
        }
        Self { view }
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Shows `image` (or nothing) over `frame`, in the container's coordinates.
    pub fn show(&self, image: Option<&CGImage>, frame: NSRect, scale: f64) {
        if self.view.frame() != frame {
            self.view.setFrame(frame);
        }
        // SAFETY: a layer-backed view has a layer; `contents` takes a
        // CGImage (or nil), `contentsScale` a CGFloat.
        unsafe {
            let layer: *mut AnyObject = msg_send![&*self.view, layer];
            if !layer.is_null() {
                let contents: *const AnyObject = image.map_or(ptr::null(), |i| ptr::from_ref(i).cast());
                let _: () = msg_send![layer, setContents: contents];
                let _: () = msg_send![layer, setContentsScale: scale];
            }
        }
        self.view.setHidden(image.is_none());
    }

    /// Moves the overlay (the container moved or was clipped).
    pub fn place(&self, frame: NSRect) {
        if self.view.frame() != frame {
            self.view.setFrame(frame);
        }
    }
}

/// `wght`.
const WEIGHT_AXIS: i64 = 0x7767_6874;

/// The system UI font at 600 and `size`, made as GPUI's patch 7 makes it
/// (Blink's `MatchSystemUIFont`): the UI font, the bold trait, then 600 set
/// on the `wght` axis.
fn label_font(size: f64) -> Option<CFRetained<CTFont>> {
    // SAFETY: CoreText calls with valid arguments; a null matrix is identity.
    unsafe {
        let base = CTFont::new_ui_font_for_language(CTFontUIFontType::System, size, None)?;
        let bold = base
            .copy_with_symbolic_traits(
                size,
                ptr::null(),
                CTFontSymbolicTraits::BoldTrait,
                CTFontSymbolicTraits::BoldTrait,
            )
            .unwrap_or(base);
        let axis = CFNumber::new_i64(WEIGHT_AXIS);
        let weight = CFNumber::new_f64(600.0);
        let variation = CFDictionary::<CFNumber, CFNumber>::from_slices(&[&*axis], &[&*weight]);
        let attributes =
            CFDictionary::<CFString, CFType>::from_slices(&[kCTFontVariationAttribute], &[variation.as_ref()]);
        let descriptor = CTFontDescriptor::with_attributes(attributes.as_opaque());
        Some(bold.copy_with_attributes(size, ptr::null(), Some(&descriptor)))
    }
}

fn cg_color(c: gpui::Rgba) -> CFRetained<CGColor> {
    CGColor::new_srgb(
        CGFloat::from(c.r),
        CGFloat::from(c.g),
        CGFloat::from(c.b),
        CGFloat::from(c.a),
    )
}

/// Draws a page's ink into a picture of its rect at its scale.
pub fn render(ink: &PageInk) -> Option<CFRetained<CGImage>> {
    let (l, t, w, h) = ink.rect;
    let s = ink.scale;
    let (pw, ph) = ((w * s).round() as usize, (h * s).round() as usize);
    if pw == 0 || ph == 0 {
        return None;
    }
    // SAFETY: a constant CoreGraphics exports.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    // kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little: BGRA.
    let info = 2 | (2 << 12);
    // SAFETY: CoreGraphics allocates the pixels (null data, rows sized by it).
    let ctx = unsafe { CGBitmapContextCreate(ptr::null_mut(), pw, ph, 8, 0, Some(&space), info) }?;
    let c = Some(&*ctx);
    // y down, in logical px, the rect's top left at the picture's.
    CGContext::translate_ctm(c, 0.0, ph as f64);
    CGContext::scale_ctm(c, s, -s);
    CGContext::translate_ctm(c, -l, -t);
    CGContext::set_should_antialias(c, true);
    // `-webkit-font-smoothing: antialiased` (GPUI's patch 8).
    CGContext::set_should_smooth_fonts(c, false);
    CGContext::set_should_subpixel_position_fonts(c, true);
    for mark in &ink.marks {
        CGContext::save_g_state(c);
        match mark {
            PageMark::Line { segs, width, color } => {
                CGContext::begin_path(c);
                for seg in segs {
                    match *seg {
                        Seg::Move(p) => CGContext::move_to_point(c, p[0], p[1]),
                        Seg::Line(p) => CGContext::add_line_to_point(c, p[0], p[1]),
                        Seg::Quad(k, p) => CGContext::add_quad_curve_to_point(c, k[0], k[1], p[0], p[1]),
                    }
                }
                CGContext::set_line_width(c, *width);
                CGContext::set_line_cap(c, CGLineCap::Round);
                CGContext::set_line_join(c, CGLineJoin::Round);
                CGContext::set_stroke_color_with_color(c, Some(&cg_color(*color)));
                CGContext::stroke_path(c);
            }
            PageMark::Dashed {
                x,
                y,
                w,
                h,
                radius,
                width,
                dash,
                color,
            } => {
                // `paint_dashed_rect`'s path: from the top edge after the
                // corner, quarter circles as cubics, butt caps.
                let (x, y, w, h) = (*x, *y, *w, *h);
                let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
                let k = 0.5523;
                CGContext::begin_path(c);
                CGContext::move_to_point(c, x + r, y);
                CGContext::add_line_to_point(c, x + w - r, y);
                CGContext::add_curve_to_point(c, x + w - r + k * r, y, x + w, y + r - k * r, x + w, y + r);
                CGContext::add_line_to_point(c, x + w, y + h - r);
                CGContext::add_curve_to_point(c, x + w, y + h - r + k * r, x + w - r + k * r, y + h, x + w - r, y + h);
                CGContext::add_line_to_point(c, x + r, y + h);
                CGContext::add_curve_to_point(c, x + r - k * r, y + h, x, y + h - r + k * r, x, y + h - r);
                CGContext::add_line_to_point(c, x, y + r);
                CGContext::add_curve_to_point(c, x, y + r - k * r, x + r - k * r, y, x + r, y);
                CGContext::set_line_width(c, *width);
                let lengths = [dash.0, dash.1];
                // SAFETY: two lengths, read during the call.
                unsafe { CGContext::set_line_dash(c, 0.0, lengths.as_ptr(), lengths.len()) };
                CGContext::set_stroke_color_with_color(c, Some(&cg_color(*color)));
                CGContext::stroke_path(c);
            }
            PageMark::Label {
                x,
                baseline,
                font_size,
                text,
                color,
            } => draw_label(&ctx, *x, *baseline, *font_size, text, *color),
        }
        CGContext::restore_g_state(c);
    }
    CGBitmapContextCreateImage(c)
}

fn draw_label(ctx: &CGContext, x: f64, baseline: f64, size: f64, text: &str, color: gpui::Rgba) {
    if text.is_empty() || size <= 0.0 {
        return;
    }
    let Some(font) = label_font(size) else { return };
    let string = CFString::from_str(text);
    let from_context = CFBoolean::new(true);
    // SAFETY: CoreText's attribute names, constants it exports.
    let keys: [&CFString; 2] = unsafe { [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName] };
    let values: [&CFType; 2] = [font.as_ref(), from_context.as_ref()];
    let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
    // SAFETY: a string and a dictionary of CoreText attributes.
    let Some(attributed) = (unsafe { CFAttributedString::new(None, Some(&string), Some(attributes.as_opaque())) })
    else {
        return;
    };
    // SAFETY: a valid attributed string.
    let line = unsafe { CTLine::with_attributed_string(&attributed) };
    let c = Some(ctx);
    CGContext::set_fill_color_with_color(c, Some(&cg_color(color)));
    // The context's y runs down: flip the text back up.
    CGContext::set_text_matrix(
        c,
        CGAffineTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            tx: 0.0,
            ty: 0.0,
        },
    );
    CGContext::set_text_position(c, x, baseline);
    // SAFETY: a line drawn into a live context.
    unsafe { line.draw(ctx) };
}
