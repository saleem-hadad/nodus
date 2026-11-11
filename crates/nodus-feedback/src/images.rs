//! Screenshots as GitHub takes them: PNG, JPEG, GIF or WebP, at most
//! [`MAX_IMAGE_BYTES`] each, at most [`MAX_SHOTS`] to a report.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};

/// RPT-010: screenshots on one report.
pub const MAX_SHOTS: usize = 5;

/// RPT-011: GitHub's limit for an image on an issue.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// What a picture couldn't become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareError {
    /// Not an image this can read.
    Unsupported,
    /// Too large, even made smaller.
    TooLarge,
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "That isn't an image Nodus can attach.",
            Self::TooLarge => "That image is too large to attach.",
        })
    }
}

/// The type GitHub sees, from the file's first bytes.
pub fn kind_of(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// RPT-012: a picture as GitHub takes it. PNG, JPEG, GIF and WebP under the
/// limit go as they are; anything else readable (a TIFF or BMP off the
/// clipboard) becomes a PNG; an image over the limit becomes a JPEG, made
/// smaller until it fits.
pub fn prepare(bytes: Vec<u8>) -> Result<Vec<u8>, PrepareError> {
    let known = kind_of(&bytes).is_some();
    if known && bytes.len() <= MAX_IMAGE_BYTES {
        return Ok(bytes);
    }
    let image = image::load_from_memory(&bytes)
        .ok()
        .filter(|i| i.width() > 0 && i.height() > 0)
        .ok_or(if known {
            PrepareError::TooLarge
        } else {
            PrepareError::Unsupported
        })?;
    if !known {
        let png = encode_png(&image);
        if png.len() <= MAX_IMAGE_BYTES {
            return Ok(png);
        }
    }
    fit_as_jpeg(&image)
}

/// RPT-013: the window's pixels as a PNG, made smaller as a JPEG in the rare
/// case it's over the limit.
pub fn png_of_rgba(image: RgbaImage) -> Result<Vec<u8>, PrepareError> {
    let image = DynamicImage::ImageRgba8(image);
    let png = encode_png(&image);
    if png.len() <= MAX_IMAGE_BYTES {
        return Ok(png);
    }
    fit_as_jpeg(&image)
}

fn encode_png(image: &DynamicImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let _ = image.write_to(&mut out, ImageFormat::Png);
    out.into_inner()
}

/// JPEG at quality 85, halving the size until it fits (at most 6 times).
fn fit_as_jpeg(image: &DynamicImage) -> Result<Vec<u8>, PrepareError> {
    let mut image = DynamicImage::ImageRgb8(image.to_rgb8());
    for _ in 0..6 {
        let mut out = Vec::new();
        if JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&image)
            .is_err()
        {
            return Err(PrepareError::Unsupported);
        }
        if out.len() <= MAX_IMAGE_BYTES {
            return Ok(out);
        }
        let (w, h) = ((image.width() / 2).max(1), (image.height() / 2).max(1));
        image = image.resize_exact(w, h, FilterType::Triangle);
    }
    Err(PrepareError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        encode_png(&DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            w,
            h,
            image::Rgba([10, 20, 30, 255]),
        )))
    }

    #[test]
    fn knows_types_by_their_bytes() {
        assert_eq!(kind_of(&png(2, 2)), Some("image/png"));
        assert_eq!(kind_of(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(kind_of(b"GIF89a...."), Some("image/gif"));
        assert_eq!(kind_of(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(kind_of(b"hello"), None);
        assert_eq!(kind_of(b""), None);
    }

    #[test]
    fn keeps_a_png_that_fits() {
        let bytes = png(4, 4);
        assert_eq!(prepare(bytes.clone()), Ok(bytes));
    }

    #[test]
    fn turns_other_images_into_pngs() {
        let mut bmp = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::new(3, 3))
            .write_to(&mut bmp, ImageFormat::Bmp)
            .unwrap();
        let out = prepare(bmp.into_inner()).unwrap();
        assert_eq!(kind_of(&out), Some("image/png"));
    }

    #[test]
    fn refuses_what_isnt_an_image() {
        assert_eq!(prepare(b"not an image".to_vec()), Err(PrepareError::Unsupported));
    }

    #[test]
    fn makes_a_window_shot_a_png() {
        let out = png_of_rgba(RgbaImage::new(8, 5)).unwrap();
        assert_eq!(kind_of(&out), Some("image/png"));
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (8, 5));
    }

    #[test]
    fn shrinks_a_huge_image_into_a_jpeg() {
        // Noise doesn't compress: a 2400×2400 RGBA PNG of it is ~23 MB.
        let mut seed = 1u32;
        let noisy = RgbaImage::from_fn(2400, 2400, |_, _| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let [a, b, c, _] = seed.to_le_bytes();
            image::Rgba([a, b, c, 255])
        });
        let out = png_of_rgba(noisy).unwrap();
        assert_eq!(kind_of(&out), Some("image/jpeg"));
        assert!(out.len() <= MAX_IMAGE_BYTES);
    }
}
