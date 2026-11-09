//! Nodus patch (custom cursors): the native cursors behind `CursorStyle::Move`,
//! `CursorStyle::Cell` and `CursorStyle::Custom`.
//!
//! `move` and `cell` have no public `NSCursor`. Chromium (and WebKit before
//! it) gets them from CoreCursor: an `NSCursor` subclass whose
//! `_coreCursorType` names a system cursor, which AppKit then draws (type 39
//! is the four-way move arrow, 41 the cell plus). Using the same mechanism
//! keeps them the same pictures as 0.9.4's.
//!
//! An image cursor becomes an `NSCursor` over an `NSImage` holding every
//! bitmap the image was registered with, so AppKit picks the one for the
//! display, sized in points with the hotspot from the top left.

use gpui::{CursorBitmap, CustomCursor};
use objc2::{
    class, msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};
use objc2_foundation::{NSInteger, NSPoint, NSSize};
use std::{cell::RefCell, collections::HashMap, ffi::CStr, ptr};

const CORE_CURSOR_TYPE_IVAR: &CStr = c"nodusCoreCursorType";

/// CoreCursor's number for CSS `move`.
pub(crate) const CORE_CURSOR_MOVE: i64 = 39;
/// CoreCursor's number for CSS `cell`.
pub(crate) const CORE_CURSOR_CELL: i64 = 41;

thread_local! {
    static CORE_CURSOR_CLASS: RefCell<Option<&'static AnyClass>> = const { RefCell::new(None) };
    /// Cursors made so far, retained for the life of the process.
    static CORE_CURSORS: RefCell<HashMap<i64, usize>> = RefCell::new(HashMap::new());
    static IMAGE_CURSORS: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
}

extern "C" fn core_cursor_type(this: &AnyObject, _: Sel) -> i64 {
    // SAFETY: the method is only registered on our class, which has the ivar.
    unsafe {
        *this
            .class()
            .instance_variable(CORE_CURSOR_TYPE_IVAR)
            .expect("core cursor ivar")
            .load::<i64>(this)
    }
}

fn core_cursor_class() -> &'static AnyClass {
    CORE_CURSOR_CLASS.with(|class| {
        *class.borrow_mut().get_or_insert_with(|| {
            let mut decl = ClassBuilder::new(c"GPUINodusCoreCursor", class!(NSCursor))
                .expect("GPUINodusCoreCursor is declared once");
            decl.add_ivar::<i64>(CORE_CURSOR_TYPE_IVAR);
            // SAFETY: the function's signature matches the selector's
            // (`- (long long)_coreCursorType`).
            unsafe {
                decl.add_method(
                    sel!(_coreCursorType),
                    core_cursor_type as extern "C" fn(_, _) -> _,
                );
            }
            decl.register()
        })
    })
}

/// A system cursor by its CoreCursor number, or `None` if this macOS has no
/// picture for it.
///
/// # Safety
///
/// Call on the main thread.
pub(crate) unsafe fn core_cursor(kind: i64) -> Option<*mut AnyObject> {
    if let Some(cursor) = CORE_CURSORS.with(|c| c.borrow().get(&kind).copied()) {
        return (cursor != 0).then_some(cursor as *mut AnyObject);
    }
    // SAFETY: main thread (caller); the ivar exists on the class.
    let cursor = unsafe {
        let cursor: *mut AnyObject = msg_send![core_cursor_class(), alloc];
        let cursor: *mut AnyObject = msg_send![cursor, init];
        if !cursor.is_null() {
            *(*cursor)
                .class()
                .instance_variable(CORE_CURSOR_TYPE_IVAR)
                .expect("core cursor ivar")
                .load_mut::<i64>(&mut *cursor) = kind;
            let image: *mut AnyObject = msg_send![cursor, image];
            if image.is_null() {
                let _: () = msg_send![cursor, release];
                ptr::null_mut()
            } else {
                cursor
            }
        } else {
            cursor
        }
    };
    CORE_CURSORS.with(|c| c.borrow_mut().insert(kind, cursor as usize));
    (!cursor.is_null()).then_some(cursor)
}

/// The `NSCursor` for a registered image, or `None` if it can't be made.
///
/// # Safety
///
/// Call on the main thread.
pub(crate) unsafe fn image_cursor(cursor: CustomCursor) -> Option<*mut AnyObject> {
    if let Some(made) = IMAGE_CURSORS.with(|c| c.borrow().get(&cursor.id()).copied()) {
        return (made != 0).then_some(made as *mut AnyObject);
    }
    let made = cursor
        .image()
        // SAFETY: main thread (caller).
        .and_then(|image| unsafe {
            let size = NSSize::new(image.size.width as f64, image.size.height as f64);
            let ns_image: *mut AnyObject = msg_send![class!(NSImage), alloc];
            let ns_image: *mut AnyObject = msg_send![ns_image, initWithSize: size];
            let mut reps = 0;
            for bitmap in &image.bitmaps {
                if let Some(rep) = bitmap_rep(bitmap, size) {
                    let _: () = msg_send![ns_image, addRepresentation: rep];
                    let _: () = msg_send![rep, release];
                    reps += 1;
                }
            }
            if reps == 0 {
                let _: () = msg_send![ns_image, release];
                return None;
            }
            let hotspot = NSPoint::new(image.hotspot.x as f64, image.hotspot.y as f64);
            let ns_cursor: *mut AnyObject = msg_send![class!(NSCursor), alloc];
            let ns_cursor: *mut AnyObject =
                msg_send![ns_cursor, initWithImage: ns_image, hotSpot: hotspot];
            let _: () = msg_send![ns_image, release];
            (!ns_cursor.is_null()).then_some(ns_cursor)
        });
    IMAGE_CURSORS.with(|c| {
        c.borrow_mut()
            .insert(cursor.id(), made.map_or(0, |p| p as usize))
    });
    made
}

/// An sRGB `NSBitmapImageRep` of `bitmap`, `size` points large (+1 retained).
unsafe fn bitmap_rep(bitmap: &CursorBitmap, size: NSSize) -> Option<*mut AnyObject> {
    let (width, height) = (bitmap.width as usize, bitmap.height as usize);
    if width == 0 || height == 0 || bitmap.rgba.len() < width * height * 4 {
        return None;
    }
    // NSBitmapFormatAlphaNonpremultiplied
    const NON_PREMULTIPLIED: usize = 1 << 1;
    // SAFETY: the planes argument is null, so AppKit allocates the buffer
    // `bitmapData` returns, `bytesPerRow × height` bytes long.
    unsafe {
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: *mut AnyObject = msg_send![
            rep,
            initWithBitmapDataPlanes: ptr::null_mut::<*mut u8>(),
            pixelsWide: width as NSInteger,
            pixelsHigh: height as NSInteger,
            bitsPerSample: 8 as NSInteger,
            samplesPerPixel: 4 as NSInteger,
            hasAlpha: Bool::YES,
            isPlanar: Bool::NO,
            colorSpaceName: crate::ns_string("NSCalibratedRGBColorSpace"),
            bitmapFormat: NON_PREMULTIPLIED,
            bytesPerRow: (width * 4) as NSInteger,
            bitsPerPixel: 32 as NSInteger
        ];
        if rep.is_null() {
            return None;
        }
        let data: *mut u8 = msg_send![rep, bitmapData];
        if data.is_null() {
            let _: () = msg_send![rep, release];
            return None;
        }
        ptr::copy_nonoverlapping(bitmap.rgba.as_ptr(), data, width * height * 4);
        // The pixels are sRGB, as CSS colours are.
        let srgb: *mut AnyObject = msg_send![class!(NSColorSpace), sRGBColorSpace];
        let tagged: *mut AnyObject = msg_send![rep, bitmapImageRepByRetaggingWithColorSpace: srgb];
        let rep = if tagged.is_null() || tagged == rep {
            rep
        } else {
            let _: *mut AnyObject = msg_send![tagged, retain];
            let _: () = msg_send![rep, release];
            tagged
        };
        let _: () = msg_send![rep, setSize: size];
        Some(rep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{CursorImage, CursorStyle, point, size};

    unsafe fn image_size(cursor: *mut AnyObject) -> (NSSize, NSPoint) {
        unsafe {
            let image: *mut AnyObject = msg_send![cursor, image];
            let size: NSSize = msg_send![image, size];
            let hotspot: NSPoint = msg_send![cursor, hotSpot];
            (size, hotspot)
        }
    }

    #[test]
    fn move_and_cell_come_from_core_cursor() {
        unsafe {
            let moving = core_cursor(CORE_CURSOR_MOVE).expect("move cursor");
            let (size, hotspot) = image_size(moving);
            assert_eq!((size.width, size.height), (24., 24.));
            assert_eq!((hotspot.x, hotspot.y), (12., 12.));
            assert_eq!(core_cursor(CORE_CURSOR_MOVE), Some(moving));
            assert!(core_cursor(CORE_CURSOR_CELL).is_some());
        }
    }

    #[test]
    fn an_image_cursor_keeps_its_size_and_hotspot() {
        let bitmap = |scale: f32| {
            let side = (20. * scale) as u32;
            CursorBitmap {
                width: side,
                height: side,
                scale,
                rgba: vec![255; (side * side * 4) as usize],
            }
        };
        let cursor = CustomCursor::register(CursorImage {
            size: size(20., 20.),
            hotspot: point(10., 4.),
            bitmaps: vec![bitmap(1.), bitmap(2.)],
            fallback: CursorStyle::Cell,
        });
        unsafe {
            let made = image_cursor(cursor).expect("image cursor");
            let (size, hotspot) = image_size(made);
            assert_eq!((size.width, size.height), (20., 20.));
            assert_eq!((hotspot.x, hotspot.y), (10., 4.));
            let image: *mut AnyObject = msg_send![made, image];
            let reps: *mut AnyObject = msg_send![image, representations];
            let count: usize = msg_send![reps, count];
            assert_eq!(count, 2);
        }
    }
}
