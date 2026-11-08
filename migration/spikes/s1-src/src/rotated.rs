//! Step 7 prototype: rotated elements without forking GPUI.
//!
//! Upstream GPUI can only transform monochrome sprites, and only `paint_svg` exposes the
//! transform. Quads, shadows, images and text glyphs are painted axis-aligned. So a tilted note is
//! drawn as a stack of single-colour SVG layers (shadow, paper, ink), rasterised by resvg into the
//! sprite atlas once per (content, size) and rotated on the GPU at sampling time. Changing only the
//! angle (the tilt while dragging, the 3° sticker) re-uses the cached rasters.
//!
//! Also here: CSS-style easing (cubic-bezier and the multi-stop `linear()`) for GPUI's
//! `Animation::with_easing`, which accepts any `Fn(f32) -> f32`.
use gpui::*;

pub struct StickerLayers {
    pub key: u64,
    pub shadow: String,
    pub paper: String,
    pub ink: String,
    pub margin: f32,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Layers in *logical* px of the unrotated sticker; `margin` leaves room for shadow + rotation.
pub fn sticker_layers(w: f32, h: f32, lines: &[&str]) -> StickerLayers {
    let m = 24.0;
    let (sw, sh) = (w + 2. * m, h + 2. * m);
    let head = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{sw}" height="{sh}" viewBox="0 0 {sw} {sh}">"#);
    let shadow = format!(
        r#"{head}<defs><filter id="b" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="7"/></filter></defs><rect x="{m}" y="{}" width="{w}" height="{h}" rx="3" fill="black" filter="url(#b)"/></svg>"#,
        m + 7.
    );
    let paper = format!(r#"{head}<rect x="{m}" y="{m}" width="{w}" height="{h}" rx="3" fill="black"/></svg>"#);
    let mut ink = head.clone();
    ink.push_str(&format!(
        r#"<rect x="{m}" y="{m}" width="{w}" height="22" rx="3" fill="black" fill-opacity="0.10"/>"#
    ));
    for (i, l) in lines.iter().enumerate() {
        ink.push_str(&format!(
            r#"<text x="{}" y="{}" font-family="Helvetica Neue, Helvetica, sans-serif" font-size="{}" font-weight="{}" fill="black">{}</text>"#,
            m + 12.,
            m + 44. + 19. * i as f32,
            if i == 0 { 15 } else { 13 },
            if i == 0 { 600 } else { 400 },
            esc(l)
        ));
    }
    ink.push_str("</svg>");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    (w.to_bits(), h.to_bits(), lines).hash(&mut hasher);
    StickerLayers { key: hasher.finish(), shadow, paper, ink, margin: m }
}

/// Paint the layers rotated by `angle_deg` around the centre of `bounds` (the *unrotated* sticker
/// bounds on screen, already scaled by the board zoom).
pub fn paint_sticker(
    layers: &StickerLayers,
    bounds: Bounds<Pixels>,
    zoom: f32,
    angle_deg: f32,
    colors: (Hsla, Hsla, Hsla),
    window: &mut Window,
    cx: &App,
) {
    // Rasterise at a quantised zoom (half-octave buckets) and make up the rest with the sprite
    // transform, so a zoom gesture re-uses cached rasters instead of re-running resvg every frame.
    let bucket = 2f32.powf((zoom.log2() * 2.).round() / 2.);
    let residual = zoom / bucket;
    let center_l = bounds.center();
    let unscaled = size(bounds.size.width / zoom, bounds.size.height / zoom);
    let m = layers.margin;
    let raster = size((unscaled.width + px(2. * m)) * bucket, (unscaled.height + px(2. * m)) * bucket);
    let outer = Bounds::new(center_l - point(raster.width / 2., raster.height / 2.), raster);
    let sf = window.scale_factor();
    let center = outer.center().scale(sf);
    let matrix = TransformationMatrix::unit()
        .translate(center)
        .rotate(radians(angle_deg.to_radians()))
        .scale(size(residual, residual))
        .translate(point(ScaledPixels(-center.x.0), ScaledPixels(-center.y.0)));
    let (shadow_c, paper_c, ink_c) = colors;
    let k = layers.key;
    // The shadow is blurred anyway: rasterise it at a quarter of the size and scale it up 4x on the
    // GPU (resvg's feGaussianBlur is the expensive part, ~80ms at full size).
    let small = size(raster.width / 4., raster.height / 4.);
    let shadow_bounds = Bounds::new(center_l - point(small.width / 2., small.height / 2.), small);
    let shadow_matrix = TransformationMatrix::unit()
        .translate(center)
        .rotate(radians(angle_deg.to_radians()))
        .scale(size(residual * 4., residual * 4.))
        .translate(point(ScaledPixels(-center.x.0), ScaledPixels(-center.y.0)));
    let _ = window.paint_svg(shadow_bounds, format!("sticker-{k}-shadow").into(), Some(layers.shadow.as_bytes()), shadow_matrix, shadow_c, cx);
    let _ = window.paint_svg(outer, format!("sticker-{k}-paper").into(), Some(layers.paper.as_bytes()), matrix, paper_c, cx);
    let _ = window.paint_svg(outer, format!("sticker-{k}-ink").into(), Some(layers.ink.as_bytes()), matrix, ink_c, cx);
}

// ---------------------------------------------------------------------------------------------
// Easing
// ---------------------------------------------------------------------------------------------

/// CSS `cubic-bezier(x1, y1, x2, y2)`.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 + Clone {
    move |t: f32| {
        let t = t.clamp(0., 1.);
        let bx = |s: f32| 3. * (1. - s) * (1. - s) * s * x1 + 3. * (1. - s) * s * s * x2 + s * s * s;
        let by = |s: f32| 3. * (1. - s) * (1. - s) * s * y1 + 3. * (1. - s) * s * s * y2 + s * s * s;
        // Solve bx(s) = t (monotonic in s for 0<=x1,x2<=1) by bisection.
        let (mut lo, mut hi) = (0f32, 1f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.;
            if bx(mid) < t { lo = mid } else { hi = mid }
        }
        by((lo + hi) / 2.)
    }
}

/// CSS `linear(v0, v1, ..., vn)` with evenly spaced stops (what Nodus' --ease-stack uses).
pub fn css_linear(stops: Vec<f32>) -> impl Fn(f32) -> f32 + Clone {
    move |t: f32| {
        let n = stops.len();
        if n < 2 {
            return t;
        }
        let x = t.clamp(0., 1.) * (n - 1) as f32;
        let i = (x.floor() as usize).min(n - 2);
        let f = x - i as f32;
        stops[i] + (stops[i + 1] - stops[i]) * f
    }
}
