//! Pictures on the board (SHAPE-006…SHAPE-008, TOOL-112): what kind of file
//! one is and how big, read from its header, and the size it goes up at.

/// The widest or tallest a picture goes up, in board units.
pub const FIT: f64 = 480.0;

/// What a picture is, from its first bytes: its usual extension, for PNG,
/// JPEG, GIF and WebP, the kinds the board shows.
pub fn picture_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sizes_from_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(picture_size(&png), Some((640, 480)));
        assert_eq!(picture_kind(&png), Some("png"));
        let gif = b"GIF89a\x20\x03\x58\x02rest";
        assert_eq!(picture_size(gif), Some((800, 600)));
        assert_eq!(picture_kind(gif), Some("gif"));
        // A JPEG: SOI, an APP0 segment, then SOF0 with height 300, width 400.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0x2c, 0x01, 0x90, 0x03,
            0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(picture_size(&jpeg), Some((400, 300)));
        assert_eq!(picture_kind(&jpeg), Some("jpg"));
        assert_eq!(picture_size(b"nothing"), None);
        assert_eq!(picture_kind(b"nothing"), None);
        assert_eq!(picture_kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
    }

    #[test]
    fn fits_pictures_within_bounds() {
        assert_eq!(fitted(Some((1920, 1080)), 1.0), (480.0, 270.0));
        assert_eq!(fitted(Some((100, 50)), 1.0), (100.0, 50.0));
        assert_eq!(fitted(None, 2.0), (640.0, 480.0));
    }
}
