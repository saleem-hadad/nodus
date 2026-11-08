//! The clipboard on a Mac: NSPasteboard's general pasteboard.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF};

/// `clipboard.readText()`: the pasteboard's plain text.
pub(super) fn clipboard_text() -> Option<String> {
    let board = NSPasteboard::generalPasteboard();
    // SAFETY: a framework constant, set once AppKit is loaded.
    let kind = unsafe { NSPasteboardTypeString };
    board.stringForType(kind).map(|text| text.to_string())
}

/// The pasteboard's image as Chromium reads it for `image/png`: a PNG, or
/// else the TIFF every copied image also comes as.
pub(super) fn clipboard_image() -> Option<Vec<u8>> {
    let board = NSPasteboard::generalPasteboard();
    // SAFETY: framework constants, set once AppKit is loaded.
    let kinds = unsafe { [NSPasteboardTypePNG, NSPasteboardTypeTIFF] };
    kinds
        .into_iter()
        .find_map(|kind| board.dataForType(kind))
        .map(|data| data.to_vec())
}
