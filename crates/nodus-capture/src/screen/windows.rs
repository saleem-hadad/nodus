//! The clipboard and the screenshot on Windows.

use std::path::Path;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::{dib_to_bmp, png_of};

/// The clipboard, open while this lives.
struct Open;

impl Open {
    fn new() -> Option<Self> {
        // SAFETY: no owner window; the clipboard is closed again on drop.
        (unsafe { OpenClipboard(std::ptr::null_mut()) } != 0).then_some(Self)
    }

    /// The bytes of a format on the clipboard.
    fn bytes(&self, format: u32) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open; the handle is the clipboard's, read
        // only between GlobalLock and GlobalUnlock, for GlobalSize bytes.
        unsafe {
            if IsClipboardFormatAvailable(format) == 0 {
                return None;
            }
            let handle = GetClipboardData(format);
            if handle.is_null() {
                return None;
            }
            let size = GlobalSize(handle);
            let data = GlobalLock(handle);
            if data.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
            GlobalUnlock(handle);
            Some(bytes)
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: opened by `Open::new`.
        unsafe { CloseClipboard() };
    }
}

/// `clipboard.readText()`: CF_UNICODETEXT, up to its NUL.
pub(super) fn clipboard_text() -> Option<String> {
    let bytes = Open::new()?.bytes(u32::from(CF_UNICODETEXT))?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// The clipboard's image as Chromium reads it: the registered `PNG` format
/// when an app put one there, else the bitmap (CF_DIB) as a BMP file.
pub(super) fn clipboard_image() -> Option<Vec<u8>> {
    let open = Open::new()?;
    let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
    // SAFETY: `name` is NUL-terminated and outlives the call.
    let png = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if png != 0
        && let Some(bytes) = open.bytes(png)
    {
        return Some(bytes);
    }
    dib_to_bmp(&open.bytes(u32::from(CF_DIB))?)
}

/// CAP-056: the whole display nearest the pointer, at its physical pixel
/// size, saved as a PNG in `file`. Nothing is written when it can't be shot.
pub(super) fn shoot_screen_under_pointer(file: &Path) {
    let Some(rect) = monitor_under_pointer() else {
        return;
    };
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 {
        return;
    }
    let Some(pixels) = grab(rect.left, rect.top, width, height) else {
        return;
    };
    let Some(image) = image::RgbaImage::from_raw(width as u32, height as u32, pixels) else {
        return;
    };
    let _ = std::fs::write(file, png_of(&image::DynamicImage::ImageRgba8(image)));
}

fn monitor_under_pointer() -> Option<RECT> {
    let mut point = POINT { x: 0, y: 0 };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    // SAFETY: both out-pointers are valid for writes, and `cbSize` is set.
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return None;
        }
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
    }
    Some(info.rcMonitor)
}

/// The screen's pixels in a rectangle, as RGBA rows from the top.
fn grab(x: i32, y: i32, width: i32, height: i32) -> Option<Vec<u8>> {
    // SAFETY: every GDI object made here is released before returning; the
    // buffer holds width × height 32-bit pixels, which GetDIBits fills.
    unsafe {
        let null: HWND = std::ptr::null_mut();
        let screen = GetDC(null);
        if screen.is_null() {
            return None;
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let old = SelectObject(memory, bitmap);
        let copied = BitBlt(memory, 0, 0, width, height, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
        SelectObject(memory, old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows from the top.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let rows = if copied {
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(null, screen);
        if rows == 0 {
            return None;
        }
        // BGRX → RGBA, opaque.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Some(pixels)
    }
}
