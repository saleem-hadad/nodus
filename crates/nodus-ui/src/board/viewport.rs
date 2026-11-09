//! The board's viewport: where it's panned to and how far it's zoomed, and the
//! React Flow (`@xyflow/system` 0.0.83) and d3-zoom maths that move it:
//! wheel and pinch zoom, fitting, `setCenter`, animated moves (van Wijk and
//! Nuij's smooth zoom through `d3.interpolateZoom`, eased cubic-in-out) and
//! the auto-pan near the canvas's edges. Pure: the canvas applies it.

use nodus_domain::{Rect, Viewport};

/// `minZoom` / `maxZoom` (BoardView.tsx).
pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 2.0;

/// `screen = flow × zoom + (x, y)`, relative to the canvas's top-left.
pub fn to_screen(vp: Viewport, x: f64, y: f64) -> (f64, f64) {
    (x * vp.zoom + vp.x, y * vp.zoom + vp.y)
}

/// `screenToFlowPosition`, from canvas-relative pixels.
pub fn to_flow(vp: Viewport, sx: f64, sy: f64) -> (f64, f64) {
    ((sx - vp.x) / vp.zoom, (sy - vp.y) / vp.zoom)
}

pub fn clamp_zoom(zoom: f64) -> f64 {
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// The board's saved viewport, its zoom clamped as d3's scale extent does
/// (BOARD-…: initial viewport), or React Flow's default.
pub fn initial(saved: Option<Viewport>) -> Viewport {
    match saved {
        Some(v) => Viewport {
            zoom: clamp_zoom(v.zoom),
            ..v
        },
        None => Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        },
    }
}

/// Scales to `zoom` (clamped) keeping the canvas point `(px, py)` fixed:
/// d3-zoom's `scaleTo(…, k, p)`.
pub fn zoom_about(vp: Viewport, zoom: f64, px: f64, py: f64) -> Viewport {
    let k = clamp_zoom(zoom);
    // The flow point under p stays under p.
    let (fx, fy) = to_flow(vp, px, py);
    Viewport {
        x: px - fx * k,
        y: py - fy * k,
        zoom: k,
    }
}

/// `wheelDelta` and the pinch handler: the zoom factor's exponent of 2 for a
/// wheel `delta_y` (DOM sign: positive scrolls down / pinches in). `mode` is
/// the DOM `deltaMode` (0 pixels, 1 lines, 2 pages); `mac_pinch` is a ctrl
/// wheel on a Mac, whose trackpad pinches arrive that way (×10).
pub fn wheel_zoom_exponent(delta_y: f64, mode: u8, mac_pinch: bool) -> f64 {
    let m = match mode {
        1 => 0.05,
        0 => 0.002,
        _ => 1.0,
    };
    let factor = if mac_pinch { 10.0 } else { 1.0 };
    -delta_y * m * factor
}

/// The pan of a plain wheel (`createPanOnScrollHandler`, speed 0.5): the
/// viewport moves by half the DOM delta, the other way. Lines count 20 px.
/// Off a Mac, Shift turns vertical scrolling horizontal.
pub fn wheel_pan(vp: Viewport, delta_x: f64, delta_y: f64, mode: u8, shift: bool, mac: bool) -> Viewport {
    let normalize = if mode == 1 { 20.0 } else { 1.0 };
    let (mut dx, mut dy) = (delta_x * normalize, delta_y * normalize);
    if !mac && shift {
        dx = delta_y * normalize;
        dy = 0.0;
    }
    // translateBy(-(d / k) × 0.5) in flow units is -(d × 0.5) on screen.
    Viewport {
        x: vp.x - dx * 0.5,
        y: vp.y - dy * 0.5,
        zoom: vp.zoom,
    }
}

/// `parsePadding` for a number: whole pixels of padding on each side.
fn padding_px(padding: f64, len: f64) -> f64 {
    ((len - len / (1.0 + padding)) * 0.5).floor()
}

/// `getViewportForBounds`: the viewport that shows `bounds` in a `w × h`
/// canvas with `padding`, the zoom clamped to [min, max], centred, then
/// nudged so no side gets less than its padding.
pub fn for_bounds(bounds: Rect, w: f64, h: f64, min: f64, max: f64, padding: f64) -> Viewport {
    let px = padding_px(padding, w);
    let py = padding_px(padding, h);
    let zoom = ((w - 2.0 * px) / bounds.w).min((h - 2.0 * py) / bounds.h);
    let k = zoom.clamp(min, max);
    let cx = bounds.x + bounds.w / 2.0;
    let cy = bounds.y + bounds.h / 2.0;
    let x = w / 2.0 - cx * k;
    let y = h / 2.0 - cy * k;
    // calculateAppliedPaddings.
    let left = (bounds.x * k + x).floor();
    let top = (bounds.y * k + y).floor();
    let right = (w - ((bounds.x + bounds.w) * k + x)).floor();
    let bottom = (h - ((bounds.y + bounds.h) * k + y)).floor();
    let off_left = (left - px).min(0.0);
    let off_top = (top - py).min(0.0);
    let off_right = (right - px).min(0.0);
    let off_bottom = (bottom - py).min(0.0);
    Viewport {
        x: x - off_left + off_right,
        y: y - off_top + off_bottom,
        zoom: k,
    }
}

/// `setCenter(cx, cy, { zoom })`: not clamped.
pub fn centred_on(cx: f64, cy: f64, zoom: f64, w: f64, h: f64) -> Viewport {
    Viewport {
        x: w / 2.0 - cx * zoom,
        y: h / 2.0 - cy * zoom,
        zoom,
    }
}

/// d3's `easeCubicInOut`.
pub fn ease_cubic_in_out(t: f64) -> f64 {
    let t = t * 2.0;
    if t <= 1.0 {
        t * t * t / 2.0
    } else {
        let t = t - 2.0;
        (t * t * t + 2.0) / 2.0
    }
}

/// An animated move between two viewports, as d3-zoom's transition runs it:
/// `interpolateZoom` (ρ = √2) between the view centres and widths about the
/// canvas's centre, eased cubic-in-out over `duration` ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flight {
    from: Viewport,
    to: Viewport,
    /// The canvas's centre and its larger side (d3's `p` and `w`).
    p: (f64, f64),
    w: f64,
}

impl Flight {
    pub fn new(from: Viewport, to: Viewport, canvas_w: f64, canvas_h: f64) -> Self {
        Self {
            from,
            to,
            p: (canvas_w / 2.0, canvas_h / 2.0),
            w: canvas_w.max(canvas_h),
        }
    }

    pub fn target(&self) -> Viewport {
        self.to
    }

    /// The viewport at linear progress `t` in [0, 1] (before easing).
    pub fn at(&self, t: f64) -> Viewport {
        if t >= 1.0 {
            return self.to;
        }
        let t = ease_cubic_in_out(t.max(0.0));
        let (px, py) = self.p;
        // a.invert(p).concat(w / a.k)
        let a = [
            (px - self.from.x) / self.from.zoom,
            (py - self.from.y) / self.from.zoom,
            self.w / self.from.zoom,
        ];
        let b = [
            (px - self.to.x) / self.to.zoom,
            (py - self.to.y) / self.to.zoom,
            self.w / self.to.zoom,
        ];
        let l = interpolate_zoom(a, b, t);
        let k = self.w / l[2];
        Viewport {
            x: px - l[0] * k,
            y: py - l[1] * k,
            zoom: k,
        }
    }
}

/// d3's `interpolateZoom` with ρ = √2, at `t`.
pub fn interpolate_zoom(p0: [f64; 3], p1: [f64; 3], t: f64) -> [f64; 3] {
    const RHO: f64 = std::f64::consts::SQRT_2;
    const RHO2: f64 = 2.0;
    const RHO4: f64 = 4.0;
    let [ux0, uy0, w0] = p0;
    let [ux1, uy1, w1] = p1;
    let dx = ux1 - ux0;
    let dy = uy1 - uy0;
    let d2 = dx * dx + dy * dy;
    if d2 < 1e-12 {
        let s = (w1 / w0).ln() / RHO;
        return [ux0 + t * dx, uy0 + t * dy, w0 * (RHO * t * s).exp()];
    }
    let d1 = d2.sqrt();
    let b0 = (w1 * w1 - w0 * w0 + RHO4 * d2) / (2.0 * w0 * RHO2 * d1);
    let b1 = (w1 * w1 - w0 * w0 - RHO4 * d2) / (2.0 * w1 * RHO2 * d1);
    let r0 = ((b0 * b0 + 1.0).sqrt() - b0).ln();
    let r1 = ((b1 * b1 + 1.0).sqrt() - b1).ln();
    let s_total = (r1 - r0) / RHO;
    let s = t * s_total;
    let coshr0 = r0.cosh();
    let u = w0 / (RHO2 * d1) * (coshr0 * (RHO * s + r0).tanh() - r0.sinh());
    [ux0 + u * dx, uy0 + u * dy, w0 * coshr0 / (RHO * s + r0).cosh()]
}

/// `calcAutoPan`: how far to pan this frame (screen px, x and y) for a
/// pointer at canvas-relative `(x, y)` in a `w × h` canvas: within 40px of an
/// edge, up to 15px a frame towards it.
pub fn auto_pan(x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    const SPEED: f64 = 15.0;
    const DISTANCE: f64 = 40.0;
    let velocity = |value: f64, min: f64, max: f64| {
        if value < min {
            (value - min).abs().clamp(1.0, min) / min
        } else if value > max {
            -(value - max).abs().clamp(1.0, min) / min
        } else {
            0.0
        }
    };
    (
        velocity(x, DISTANCE, w - DISTANCE) * SPEED,
        velocity(y, DISTANCE, h - DISTANCE) * SPEED,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(x: f64, y: f64, zoom: f64) -> Viewport {
        Viewport { x, y, zoom }
    }

    fn close(a: Viewport, b: Viewport) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.zoom - b.zoom).abs() < 1e-9
    }

    #[test]
    fn converts_between_screen_and_flow() {
        let v = vp(60.0, 96.0, 0.8);
        assert_eq!(to_screen(v, 100.0, 50.0), (140.0, 136.0));
        assert_eq!(to_flow(v, 140.0, 136.0), (100.0, 50.0));
    }

    #[test]
    fn starts_from_the_saved_viewport_clamped() {
        assert_eq!(initial(None), vp(0.0, 0.0, 1.0));
        assert_eq!(initial(Some(vp(1.0, 2.0, 5.0))), vp(1.0, 2.0, 2.0));
        assert_eq!(initial(Some(vp(1.0, 2.0, 0.01))), vp(1.0, 2.0, 0.1));
    }

    #[test]
    fn zooms_about_a_point_within_limits() {
        let v = zoom_about(vp(0.0, 0.0, 1.0), 2.0, 100.0, 100.0);
        assert_eq!(v, vp(-100.0, -100.0, 2.0));
        assert_eq!(zoom_about(v, 9.0, 0.0, 0.0).zoom, 2.0);
        assert_eq!(zoom_about(v, 0.0001, 0.0, 0.0).zoom, 0.1);
    }

    #[test]
    fn zooms_with_react_flows_wheel_speeds() {
        // A Mac trackpad pinch (ctrl wheel, pixels): ×10.
        assert!((wheel_zoom_exponent(-10.0, 0, true) - 0.2).abs() < 1e-12);
        // ⌘-wheel, or a Windows pinch: ×1.
        assert!((wheel_zoom_exponent(-10.0, 0, false) - 0.02).abs() < 1e-12);
        assert!((wheel_zoom_exponent(3.0, 1, false) + 0.15).abs() < 1e-12);
        assert!((wheel_zoom_exponent(1.0, 2, false) + 1.0).abs() < 1e-12);
    }

    #[test]
    fn pans_half_the_wheel_delta() {
        assert_eq!(
            wheel_pan(vp(0.0, 0.0, 0.5), 10.0, 20.0, 0, false, true),
            vp(-5.0, -10.0, 0.5)
        );
        // Lines count 20px.
        assert_eq!(
            wheel_pan(vp(0.0, 0.0, 1.0), 0.0, 1.0, 1, false, true),
            vp(0.0, -10.0, 1.0)
        );
        // Shift turns vertical horizontal off a Mac only.
        assert_eq!(
            wheel_pan(vp(0.0, 0.0, 1.0), 0.0, 20.0, 0, true, false),
            vp(-10.0, 0.0, 1.0)
        );
        assert_eq!(
            wheel_pan(vp(0.0, 0.0, 1.0), 0.0, 20.0, 0, true, true),
            vp(0.0, -10.0, 1.0)
        );
    }

    #[test]
    fn fits_bounds_as_react_flow_does() {
        let b = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 500.0,
        };
        // padding 0.2 of 1440×848: floor((1440 − 1200) × 0.5) = 120, floor((848 − 706.67) × 0.5) = 70.
        let v = for_bounds(b, 1440.0, 848.0, 0.1, 1.0, 0.2);
        assert!((v.zoom - 1.0).abs() < 1e-12);
        assert_eq!((v.x, v.y), (220.0, 174.0));
        let v = for_bounds(Rect { w: 4000.0, ..b }, 1440.0, 848.0, 0.1, 2.0, 0.2);
        assert!((v.zoom - 0.3).abs() < 1e-12, "{v:?}");
        assert!((v.x - 120.0).abs() < 1e-9);
    }

    #[test]
    fn centres_without_clamping() {
        assert_eq!(centred_on(100.0, 50.0, 3.0, 800.0, 600.0), vp(100.0, 150.0, 3.0));
    }

    #[test]
    fn eases_cubic_in_out() {
        assert_eq!(ease_cubic_in_out(0.0), 0.0);
        assert_eq!(ease_cubic_in_out(0.5), 0.5);
        assert_eq!(ease_cubic_in_out(1.0), 1.0);
        assert!((ease_cubic_in_out(0.25) - 0.0625).abs() < 1e-12);
    }

    #[test]
    fn flies_from_one_viewport_to_another() {
        let from = vp(0.0, 0.0, 1.0);
        let to = vp(-300.0, 120.0, 0.5);
        let f = Flight::new(from, to, 1440.0, 848.0);
        assert!(close(f.at(0.0), from));
        assert!(close(f.at(1.0), to));
        let mid = f.at(0.5);
        assert!(mid.zoom < 1.0 && mid.zoom > 0.4, "{mid:?}");
        // A pure zoom about the centre keeps the centre's flow point.
        let f = Flight::new(from, zoom_about(from, 1.2, 720.0, 424.0), 1440.0, 848.0);
        let v = f.at(0.3);
        let (cx, cy) = to_flow(v, 720.0, 424.0);
        assert!((cx - 720.0).abs() < 1e-6 && (cy - 424.0).abs() < 1e-6);
    }

    #[test]
    fn auto_pans_near_the_edges() {
        assert_eq!(auto_pan(500.0, 400.0, 1000.0, 800.0), (0.0, 0.0));
        assert_eq!(auto_pan(0.0, 400.0, 1000.0, 800.0), (15.0, 0.0));
        assert_eq!(auto_pan(39.5, 400.0, 1000.0, 800.0), (15.0 / 40.0, 0.0));
        assert_eq!(auto_pan(1000.0, 790.0, 1000.0, 800.0), (-15.0, -15.0 * 30.0 / 40.0));
    }
}
