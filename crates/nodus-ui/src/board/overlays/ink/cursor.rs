//! INKUI-017: the eraser's cursor, `ink.css`'s
//! `cursor: url("data:image/svg+xml,…") 10 10, cell`: a 20×20 circle of
//! radius 7, white at 65% outlined in black at 70%, 1.5px, the hotspot at its
//! centre. Chromium draws an SVG cursor at the display's scale, so the same
//! SVG is rasterized here at 1× and 2× and handed to the patched GPUI as an
//! image cursor (`CursorStyle::Custom`), with `cell` where an image cursor
//! can't be made.

use std::sync::OnceLock;

use gpui::{CursorBitmap, CursorImage, CursorStyle, CustomCursor, point, size};

/// The SVG of `ink.css`, decoded from its data URL.
pub(crate) const ERASER_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'><circle cx='10' cy='10' r='7' fill='white' fill-opacity='0.65' stroke='black' stroke-opacity='0.7' stroke-width='1.5'/></svg>";

/// The eraser's cursor, registered the first time it's asked for.
pub(crate) fn eraser() -> CursorStyle {
    static CURSOR: OnceLock<CursorStyle> = OnceLock::new();
    *CURSOR.get_or_init(|| {
        let bitmaps: Vec<CursorBitmap> = [1.0, 2.0]
            .into_iter()
            .filter_map(|s| rasterize(ERASER_SVG, 20, s))
            .collect();
        if bitmaps.is_empty() {
            return CursorStyle::Cell;
        }
        CursorStyle::Custom(CustomCursor::register(CursorImage {
            size: size(20.0, 20.0),
            hotspot: point(10.0, 10.0),
            bitmaps,
            fallback: CursorStyle::Cell,
        }))
    })
}

/// `svg` (`side` CSS px square) drawn at `scale`, as straight RGBA.
pub(crate) fn rasterize(svg: &str, side: u32, scale: f32) -> Option<CursorBitmap> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let px = (side as f32 * scale).round() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some(CursorBitmap {
        width: px,
        height: px,
        scale,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eraser_circle_is_drawn_at_each_scale() {
        let one = rasterize(ERASER_SVG, 20, 1.0).unwrap();
        assert_eq!((one.width, one.height), (20, 20));
        let two = rasterize(ERASER_SVG, 20, 2.0).unwrap();
        assert_eq!((two.width, two.height), (40, 40));
        let at = |b: &CursorBitmap, x: u32, y: u32| {
            let i = ((y * b.width + x) * 4) as usize;
            [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2], b.rgba[i + 3]]
        };
        // The centre: white at 65%.
        assert_eq!(at(&two, 20, 20), [255, 255, 255, 166]);
        // The corner is outside the circle.
        assert_eq!(at(&two, 1, 1)[3], 0);
        // On the outline (radius 7 at 2×: 14 px from the centre), dark.
        let edge = at(&two, 20, 6);
        assert!(edge[0] < 100 && edge[3] > 120, "{edge:?}");
    }

    #[test]
    fn the_eraser_is_an_image_cursor_falling_back_to_cell() {
        let CursorStyle::Custom(cursor) = eraser() else {
            panic!("not an image cursor")
        };
        let image = cursor.image().unwrap();
        assert_eq!(image.hotspot, point(10.0, 10.0));
        assert_eq!(image.fallback, CursorStyle::Cell);
        assert_eq!(eraser(), CursorStyle::Custom(cursor));
    }
}
