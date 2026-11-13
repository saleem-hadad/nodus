//! Pictures on the board (SHAPE-006…SHAPE-008): picked from a file, pasted,
//! or dropped from Finder. Each is copied into the board's images folder
//! and put up as a picture shape at its own proportions.

use std::path::{Path, PathBuf};

use gpui::{Context, PathPromptOptions};
use nodus_board::geometry::Point;
use nodus_domain::{Rect, ShapeKind};

use super::canvas::BoardCanvas;

/// The widest or tallest a picture goes up, in board units.
const FIT: f64 = 480.0;
/// Pictures dropped together step this far apart.
const CASCADE: f64 = 32.0;

/// The kinds of picture the board takes, by extension.
pub fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp"))
}

/// A picture's width and height from its header: PNG, GIF, JPEG and WebP.
pub fn picture_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u32::from(u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?])));
    let le16 = |i: usize| Some(u32::from(u16::from_le_bytes([*bytes.get(i)?, *bytes.get(i + 1)?])));
    let be32 = |i: usize| Some(u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?));
    let le24 = |i: usize| {
        Some(u32::from_le_bytes([
            *bytes.get(i)?,
            *bytes.get(i + 1)?,
            *bytes.get(i + 2)?,
            0,
        ]))
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if bytes.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if bytes.len() > 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return match &bytes[12..16] {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8L" => {
                let b = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((b & 0x3fff) + 1, ((b >> 14) & 0x3fff) + 1))
            }
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            _ => None,
        };
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        // Walk the segments to the frame header.
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if marker == 0xff {
                i += 1;
                continue;
            }
            let len = be16(i + 2)? as usize;
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + len;
        }
    }
    None
}

/// How big a picture goes up: its own size, brought within [`FIT`] (at
/// the zoom it's put up at, so it looks the same size on screen).
pub fn fitted(size: Option<(u32, u32)>, scale: f64) -> (f64, f64) {
    let (w, h) = size.map_or((320.0, 240.0), |(w, h)| (f64::from(w.max(1)), f64::from(h.max(1))));
    let fit = FIT * scale;
    let k = (fit / w.max(h)).min(scale);
    ((w * k).round().max(24.0), (h * k).round().max(24.0))
}

impl BoardCanvas {
    /// SHAPE-006: Add a picture…, put up in the middle of the view.
    pub(crate) fn pick_images(&mut self, cx: &mut Context<'_, Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let _ = this.update(cx, |c, cx| c.add_images(paths, None, cx));
        })
        .detach();
    }

    /// SHAPE-007, SHAPE-008: pictures from files, the first centred on
    /// `at` (or the view's middle), the rest stepping down from it.
    pub(crate) fn add_images(&mut self, paths: Vec<PathBuf>, at: Option<Point>, cx: &mut Context<'_, Self>) {
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| is_picture(p)).collect();
        if paths.is_empty() {
            return;
        }
        let nodus = self.state.read(cx).nodus.clone();
        let rt = super::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            for (i, path) in paths.into_iter().enumerate() {
                let bytes = std::fs::read(&path).ok();
                let size = bytes.as_deref().and_then(picture_size);
                let (nodus, project_id) = (nodus.clone(), project_id.clone());
                let name =
                    super::on_core(&rt, async move { nodus.board().import_image(&project_id, &path).await }).await;
                let Ok(name) = name else { continue };
                let _ = this.update(cx, |c, cx| c.put_up_image(name, size, at, i, cx));
            }
        })
        .detach();
    }

    /// SHAPE-007: a picture pasted (its bytes and type).
    pub(crate) fn paste_image(&mut self, bytes: Vec<u8>, ext: &'static str, cx: &mut Context<'_, Self>) {
        let size = picture_size(&bytes);
        let nodus = self.state.read(cx).nodus.clone();
        let rt = super::core_rt(cx);
        // At the pointer when it's over the board, else the view's middle.
        let at = self
            .pointer
            .filter(|p| self.bounds.contains(p))
            .map(|p| self.to_flow(p));
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let name = super::on_core(
                &rt,
                async move { nodus.board().save_image(&project_id, bytes, ext).await },
            )
            .await;
            let Ok(name) = name else { return };
            let _ = this.update(cx, |c, cx| c.put_up_image(name, size, at, 0, cx));
        })
        .detach();
    }

    fn put_up_image(
        &mut self,
        name: String,
        size: Option<(u32, u32)>,
        at: Option<Point>,
        nth: usize,
        cx: &mut Context<'_, Self>,
    ) {
        let scale = nodus_board::ink_draw::size_at(self.viewport.zoom);
        let (w, h) = fitted(size, scale);
        let centre = at.unwrap_or_else(|| self.viewport_centre(nodus_board::geometry::Size { w: 0.0, h: 0.0 }));
        let step = CASCADE * scale * nth as f64;
        let rect = Rect {
            x: centre.x - w / 2.0 + step,
            y: centre.y - h / 2.0 + step,
            w,
            h,
        };
        self.add_shape(ShapeKind::Image, rect, String::new(), Some(name), false, cx);
    }

    /// Where a picture shape's file is; a server board's, once it's here
    /// (SRV-163).
    pub(crate) fn image_path(&self, name: &str, cx: &gpui::App) -> Option<PathBuf> {
        self.state.read(cx).nodus.board().image_path(&self.project_id, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sizes_from_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(picture_size(&png), Some((640, 480)));
        let gif = b"GIF89a\x20\x03\x58\x02rest";
        assert_eq!(picture_size(gif), Some((800, 600)));
        // A JPEG: SOI, an APP0 segment, then SOF0 with height 300, width 400.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0x2c, 0x01, 0x90, 0x03,
            0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(picture_size(&jpeg), Some((400, 300)));
        assert_eq!(picture_size(b"nothing"), None);
    }

    #[test]
    fn fits_pictures_within_bounds() {
        assert_eq!(fitted(Some((1920, 1080)), 1.0), (480.0, 270.0));
        assert_eq!(fitted(Some((100, 50)), 1.0), (100.0, 50.0));
        assert_eq!(fitted(None, 2.0), (640.0, 480.0));
        assert!(is_picture(Path::new("/a/b.JPG")) && !is_picture(Path::new("/a/b.txt")));
    }
}
