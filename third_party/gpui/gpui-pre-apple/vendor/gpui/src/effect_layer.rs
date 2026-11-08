// Effect layers: zed#63773 (core), with the Nodus patches on top: element
// transforms, group opacity, the painted extent of a layer and the CSS filter
// matrices. See third_party/gpui/PATCHES.md.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Background, Bounds, ContentMask, Corners, DevicePixels, Pixels, Point, Radians, ScaledPixels,
    Size, TransformationMatrix, point, px, size,
};

/// How a layer mixes with what is under it. The CSS `mix-blend-mode` values
/// in spec order, plus `plus-lighter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(u32)]
#[allow(missing_docs)]
pub enum BlendMode {
    #[default]
    Normal = 0,
    Multiply = 1,
    Screen = 2,
    Overlay = 3,
    Darken = 4,
    Lighten = 5,
    ColorDodge = 6,
    ColorBurn = 7,
    HardLight = 8,
    SoftLight = 9,
    Difference = 10,
    Exclusion = 11,
    Hue = 12,
    Saturation = 13,
    Color = 14,
    Luminosity = 15,
    PlusLighter = 16,
}

/// A 4 by 5 colour matrix in row-major order. Each output channel is a
/// weighted sum of the input r, g, b, a and a constant. Every CSS `filter`
/// function except `blur()` and `drop-shadow()` is one of these, and a list
/// of them multiplies into one.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ColorMatrix(pub [f32; 20]);

impl ColorMatrix {
    /// The matrix that changes nothing.
    pub const IDENTITY: Self = Self([
        1., 0., 0., 0., 0., //
        0., 1., 0., 0., 0., //
        0., 0., 1., 0., 0., //
        0., 0., 0., 1., 0.,
    ]);

    /// Whether the matrix changes nothing.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// The matrix that applies `self` first and then `next`.
    pub fn then(&self, next: &Self) -> Self {
        let a = &next.0;
        let b = &self.0;
        let mut out = [0.0; 20];
        for row in 0..4 {
            for col in 0..5 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += a[row * 5 + k] * b[k * 5 + col];
                }
                if col == 4 {
                    sum += a[row * 5 + 4];
                }
                out[row * 5 + col] = sum;
            }
        }
        Self(out)
    }

    /// CSS `saturate(amount)`: 0 is grey, 1 changes nothing, more than 1
    /// oversaturates. The matrix of Filter Effects 1, section 13.
    pub fn saturate(amount: f32) -> Self {
        let s = amount;
        Self([
            0.213 + 0.787 * s,
            0.715 - 0.715 * s,
            0.072 - 0.072 * s,
            0.,
            0., //
            0.213 - 0.213 * s,
            0.715 + 0.285 * s,
            0.072 - 0.072 * s,
            0.,
            0., //
            0.213 - 0.213 * s,
            0.715 - 0.715 * s,
            0.072 + 0.928 * s,
            0.,
            0., //
            0.,
            0.,
            0.,
            1.,
            0.,
        ])
    }

    /// CSS `grayscale(amount)`, with `amount` from 0 to 1.
    pub fn grayscale(amount: f32) -> Self {
        let g = 1.0 - amount.clamp(0.0, 1.0);
        Self([
            0.2126 + 0.7874 * g,
            0.7152 - 0.7152 * g,
            0.0722 - 0.0722 * g,
            0.,
            0., //
            0.2126 - 0.2126 * g,
            0.7152 + 0.2848 * g,
            0.0722 - 0.0722 * g,
            0.,
            0., //
            0.2126 - 0.2126 * g,
            0.7152 - 0.7152 * g,
            0.0722 + 0.9278 * g,
            0.,
            0., //
            0.,
            0.,
            0.,
            1.,
            0.,
        ])
    }

    /// CSS `sepia(amount)`, with `amount` from 0 to 1.
    pub fn sepia(amount: f32) -> Self {
        let s = 1.0 - amount.clamp(0.0, 1.0);
        Self([
            0.393 + 0.607 * s,
            0.769 - 0.769 * s,
            0.189 - 0.189 * s,
            0.,
            0., //
            0.349 - 0.349 * s,
            0.686 + 0.314 * s,
            0.168 - 0.168 * s,
            0.,
            0., //
            0.272 - 0.272 * s,
            0.534 - 0.534 * s,
            0.131 + 0.869 * s,
            0.,
            0., //
            0.,
            0.,
            0.,
            1.,
            0.,
        ])
    }

    /// CSS `brightness(amount)`: multiplies the colour channels.
    pub fn brightness(amount: f32) -> Self {
        let b = amount.max(0.0);
        Self([
            b, 0., 0., 0., 0., //
            0., b, 0., 0., 0., //
            0., 0., b, 0., 0., //
            0., 0., 0., 1., 0.,
        ])
    }

    /// CSS `contrast(amount)`: scales the colour channels around grey.
    pub fn contrast(amount: f32) -> Self {
        let c = amount.max(0.0);
        let o = 0.5 - 0.5 * c;
        Self([
            c, 0., 0., 0., o, //
            0., c, 0., 0., o, //
            0., 0., c, 0., o, //
            0., 0., 0., 1., 0.,
        ])
    }

    /// CSS `invert(amount)`, with `amount` from 0 to 1.
    pub fn invert(amount: f32) -> Self {
        let i = amount.clamp(0.0, 1.0);
        let k = 1.0 - 2.0 * i;
        Self([
            k, 0., 0., 0., i, //
            0., k, 0., 0., i, //
            0., 0., k, 0., i, //
            0., 0., 0., 1., 0.,
        ])
    }

    /// CSS `opacity(amount)` as a filter function, with `amount` from 0 to 1.
    pub fn opacity(amount: f32) -> Self {
        let a = amount.clamp(0.0, 1.0);
        Self([
            1., 0., 0., 0., 0., //
            0., 1., 0., 0., 0., //
            0., 0., 1., 0., 0., //
            0., 0., 0., a, 0.,
        ])
    }

    /// CSS `hue-rotate(angle)`.
    pub fn hue_rotate(angle: Radians) -> Self {
        let (sin, cos) = angle.0.sin_cos();
        Self([
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
            0.,
            0., //
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
            0.,
            0., //
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
            0.,
            0., //
            0.,
            0.,
            0.,
            1.,
            0.,
        ])
    }
}

impl Default for ColorMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A 2D transform of an element and everything it paints, like CSS
/// `transform` with `transform-origin`. The builder methods compose in CSS
/// order, so `ElementTransform::default().translate(x, y).rotate(a).scale(s)`
/// is `transform: translate(x, y) rotate(a) scale(s)`.
///
/// Layout ignores the transform, as in CSS: the element keeps its box, and
/// only its picture (box, shadows, text, images and children) moves. It
/// draws through an effect layer, and hit testing maps the pointer back
/// through the transform, so a rotated button is clicked where it shows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElementTransform {
    /// CSS `matrix(a, b, c, d, e, f)` about the origin: `x' = a x + c y + e`
    /// and `y' = b x + d y + f`, with `e` and `f` in pixels.
    pub matrix: [f32; 6],
    /// `transform-origin` as a fraction of the border box. CSS's default,
    /// and this one, is the centre: `(0.5, 0.5)`.
    pub origin: Point<f32>,
    /// `transform-origin` in pixels, added to `origin`.
    pub origin_offset: Point<Pixels>,
}

impl Default for ElementTransform {
    fn default() -> Self {
        Self {
            matrix: [1., 0., 0., 1., 0., 0.],
            origin: point(0.5, 0.5),
            origin_offset: Point::default(),
        }
    }
}

impl ElementTransform {
    /// Whether the transform moves nothing.
    pub fn is_identity(&self) -> bool {
        self.matrix == [1., 0., 0., 1., 0., 0.]
    }

    /// Appends a CSS `matrix(a, b, c, d, e, f)`.
    pub fn matrix(self, [a, b, c, d, e, f]: [f32; 6]) -> Self {
        let [a0, b0, c0, d0, e0, f0] = self.matrix;
        Self {
            matrix: [
                a0 * a + c0 * b,
                b0 * a + d0 * b,
                a0 * c + c0 * d,
                b0 * c + d0 * d,
                a0 * e + c0 * f + e0,
                b0 * e + d0 * f + f0,
            ],
            ..self
        }
    }

    /// Appends CSS `translate(x, y)`.
    pub fn translate(self, x: Pixels, y: Pixels) -> Self {
        self.matrix([1., 0., 0., 1., x.0, y.0])
    }

    /// Appends CSS `rotate(angle)`. Positive angles turn clockwise.
    pub fn rotate(self, angle: Radians) -> Self {
        let (sin, cos) = angle.0.sin_cos();
        self.matrix([cos, sin, -sin, cos, 0., 0.])
    }

    /// Appends CSS `scale(factor)`.
    pub fn scale(self, factor: f32) -> Self {
        self.scale_xy(factor, factor)
    }

    /// Appends CSS `scale(x, y)`.
    pub fn scale_xy(self, x: f32, y: f32) -> Self {
        self.matrix([x, 0., 0., y, 0., 0.])
    }

    /// Sets `transform-origin` as fractions of the border box.
    pub fn origin(self, x: f32, y: f32) -> Self {
        Self {
            origin: point(x, y),
            origin_offset: Point::default(),
            ..self
        }
    }

    /// Sets `transform-origin` in pixels from the top left of the border box.
    pub fn origin_px(self, x: Pixels, y: Pixels) -> Self {
        Self {
            origin: point(0., 0.),
            origin_offset: point(x, y),
            ..self
        }
    }

    /// The matrix that takes a point of the element's untransformed picture,
    /// in window pixels, to where it shows when the element sits in `bounds`.
    pub fn to_matrix(&self, bounds: Bounds<Pixels>) -> TransformationMatrix {
        let origin = point(
            bounds.origin.x.0 + bounds.size.width.0 * self.origin.x + self.origin_offset.x.0,
            bounds.origin.y.0 + bounds.size.height.0 * self.origin.y + self.origin_offset.y.0,
        );
        let [a, b, c, d, e, f] = self.matrix;
        TransformationMatrix {
            rotation_scale: [[a, c], [b, d]],
            translation: [
                origin.x + e - (a * origin.x + c * origin.y),
                origin.y + f - (b * origin.x + d * origin.y),
            ],
        }
    }
}

/// An effect layer as the renderer sees it. The renderer draws the content
/// between the begin and end marks into a texture of its own, then paints
/// that texture over the frame with these effects, in device pixels.
///
/// The content draws untransformed, so `bounds`, `corner_radii`, `painted`
/// and the mask live in the layer's own space; `transform` takes that space
/// to the space of whatever the layer paints into, where `content_mask`
/// lives.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct EffectLayer {
    /// The border box of the element.
    pub bounds: Bounds<ScaledPixels>,
    /// Clips the result in the space the layer paints into.
    pub content_mask: ContentMask<ScaledPixels>,
    /// The corners of the box, which clip the backdrop effect.
    pub corner_radii: Corners<ScaledPixels>,
    /// Everything the content painted, clipped to the frame. The scene fills
    /// it in when the layer closes.
    pub painted: Bounds<ScaledPixels>,
    /// Takes the layer's space to the space it paints into.
    pub transform: TransformationMatrix,
    /// Gaussian sigma of the blur on the content. 0 is none.
    pub blur: f32,
    /// Gaussian sigma of the blur on what is under the box. 0 is none.
    pub backdrop_blur: f32,
    /// Multiplies the alpha of the content.
    pub opacity: f32,
    /// A `BlendMode` as a number.
    pub blend_mode: u32,
    /// 1 when `mask` is live.
    pub has_mask: u32,
    /// 1 when the layer changes what is under it.
    pub has_backdrop: u32,
    /// 1 when the box clips its content to its rounded corners.
    pub clips_content: u32,
    /// 1 when `transform` is not the identity.
    pub has_transform: u32,
    /// Applied to the content after the blur.
    pub color_matrix: [f32; 20],
    /// Applied to the backdrop after its blur.
    pub backdrop_matrix: [f32; 20],
    /// A fill over the box whose alpha keeps or drops each pixel.
    pub mask: Background,
}

// WGSL rounds a struct that holds a `vec2<f32>` up to 8 bytes, and HLSL
// structured buffers read it field by field: keep the size a multiple of 16,
// so the field after a layer lands at the same offset on every side.
const _: () = assert!(std::mem::size_of::<EffectLayer>() % 16 == 0);

impl EffectLayer {
    /// Whether the composite has to read what is under the layer: a
    /// backdrop, a mask or a blend mode. The others blend in hardware.
    pub fn reads_under(&self) -> bool {
        self.has_backdrop != 0 || self.has_mask != 0 || self.blend_mode != BlendMode::Normal as u32
    }

    /// The pixels of the layer's own texture the composite reads: what the
    /// content painted, and the box when the layer shows a backdrop or a
    /// mask, grown by the reach of the content blur and, under a transform,
    /// by two clear pixels that anti-alias the edges. `viewport` is the
    /// texture.
    pub fn source_region(&self, viewport: LayerRegion) -> LayerRegion {
        let mut pad = (3.0 * self.blur).ceil() as i32;
        if self.has_transform != 0 {
            pad += 2;
        }
        let mut source = LayerRegion::around(self.painted, pad);
        if self.reads_under() {
            source = source.union(LayerRegion::around(self.bounds, 0));
        }
        if self.has_transform == 0 {
            source = source.intersect(LayerRegion::around(self.content_mask.bounds, 0));
        }
        source.intersect(viewport)
    }

    /// The pixels of the target the composite writes, for `source`.
    pub fn target_region(&self, source: LayerRegion, viewport: LayerRegion) -> LayerRegion {
        let target = if self.has_transform != 0 {
            source.transformed(&self.transform)
        } else {
            source
        };
        target
            .intersect(LayerRegion::around(self.content_mask.bounds, 0))
            .intersect(viewport)
    }

    /// The pixels under the box a backdrop blur reads, inside `target`. CSS
    /// clips the backdrop to the border box and mirrors it at the edges, so
    /// nothing outside the box bleeds into the blur.
    pub fn backdrop_region(&self, target: LayerRegion) -> LayerRegion {
        let bounds = LayerRegion::around(self.bounds, 0);
        let bounds = if self.has_transform != 0 {
            bounds.transformed(&self.transform)
        } else {
            bounds
        };
        bounds.intersect(target)
    }

    /// Roughly what the layer paints into its parent: its source, moved by
    /// its transform and clipped by its content mask.
    pub(crate) fn extent_in_parent(&self) -> Bounds<ScaledPixels> {
        let mut extent = self.painted;
        if self.reads_under() {
            extent = union_bounds(extent, self.bounds);
        }
        if self.blur > 0.0 {
            extent = extent.dilate(ScaledPixels((3.0 * self.blur).ceil()));
        }
        if self.has_transform != 0 {
            extent = transform_bounds(extent, &self.transform);
        }
        extent.intersect(&self.content_mask.bounds)
    }

    /// The inverse of `transform`, and how many target pixels one layer
    /// pixel spans, which sets the width of anti-aliased edges.
    pub fn inverse_transform(&self) -> (TransformationMatrix, f32) {
        if self.has_transform == 0 {
            return (TransformationMatrix::unit(), 1.0);
        }
        match self.transform.inverse() {
            Some(inverse) => (inverse, self.transform.determinant().abs().sqrt()),
            None => (TransformationMatrix::unit(), 1.0),
        }
    }
}

/// A rectangle of the frame in whole device pixels, for the renderers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct LayerRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl LayerRegion {
    /// Half the size, at the origin. The next step of a shrink chain.
    pub fn halved(self) -> Self {
        Self::new(0, 0, (self.width / 2).max(1), (self.height / 2).max(1))
    }

    /// A region that is never smaller than nothing.
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: width.max(0),
            height: height.max(0),
        }
    }

    /// The whole frame.
    pub fn of_viewport(viewport_size: Size<DevicePixels>) -> Self {
        Self::new(0, 0, viewport_size.width.0, viewport_size.height.0)
    }

    /// The pixels `bounds` touches, grown by `pad` on every side.
    pub fn around(bounds: Bounds<ScaledPixels>, pad: i32) -> Self {
        if bounds.size.width.0 <= 0.0 || bounds.size.height.0 <= 0.0 {
            return Self::new(0, 0, 0, 0);
        }
        let left = bounds.origin.x.0.floor() as i32 - pad;
        let top = bounds.origin.y.0.floor() as i32 - pad;
        let right = (bounds.origin.x.0 + bounds.size.width.0).ceil() as i32 + pad;
        let bottom = (bounds.origin.y.0 + bounds.size.height.0).ceil() as i32 + pad;
        Self::new(left, top, right - left, bottom - top)
    }

    /// The pixels in both regions.
    pub fn intersect(self, other: Self) -> Self {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        Self::new(left, top, right - left, bottom - top)
    }

    /// The smallest region that holds both.
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self::new(left, top, right - left, bottom - top)
    }

    /// The pixels the region covers once `transform` moves it.
    pub fn transformed(self, transform: &TransformationMatrix) -> Self {
        if self.is_empty() {
            return self;
        }
        let corners = [
            (self.x, self.y),
            (self.x + self.width, self.y),
            (self.x, self.y + self.height),
            (self.x + self.width, self.y + self.height),
        ]
        .map(|(x, y)| transform.apply_f32([x as f32, y as f32]));
        let left = corners.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
        let top = corners.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
        let right = corners
            .iter()
            .map(|c| c[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = corners
            .iter()
            .map(|c| c[1])
            .fold(f32::NEG_INFINITY, f32::max);
        let left = left.floor() as i32;
        let top = top.floor() as i32;
        Self::new(
            left,
            top,
            right.ceil() as i32 - left,
            bottom.ceil() as i32 - top,
        )
    }

    /// Whether the region has no pixels.
    pub fn is_empty(self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// The region as bounds in device pixels.
    pub fn bounds(self) -> Bounds<ScaledPixels> {
        Bounds {
            origin: point(ScaledPixels(self.x as f32), ScaledPixels(self.y as f32)),
            size: size(
                ScaledPixels(self.width as f32),
                ScaledPixels(self.height as f32),
            ),
        }
    }
}

/// How a renderer blurs a region: two passes of a Gaussian, one per axis,
/// on a texture shrunk by `scale`, so a wide blur costs the same as a
/// narrow one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurPlan {
    /// How many source pixels one pixel of the small texture stands for.
    pub scale: i32,
    /// The standard deviation in pixels of the small texture.
    pub sigma: f32,
    /// Taps on each side of the centre.
    pub radius: i32,
}

impl BlurPlan {
    /// The plan for a Gaussian of `sigma` device pixels.
    pub fn new(sigma: f32) -> Self {
        // A power of two, so the renderer shrinks the source by exact 2 by 2
        // box steps. A small grid that samples a big source skips pixels,
        // and the result flickers as content scrolls.
        let scale = if sigma >= 32.0 {
            8
        } else if sigma >= 16.0 {
            4
        } else if sigma >= 8.0 {
            2
        } else {
            1
        };
        let sigma = sigma / scale as f32;
        let radius = (3.0 * sigma).ceil().min(24.0) as i32;
        Self {
            scale,
            sigma,
            radius,
        }
    }

    /// How many times the renderer halves the source before the blur.
    pub fn shrink_steps(&self) -> u32 {
        self.scale.trailing_zeros()
    }
}

/// An empty extent, which a union with any bounds replaces.
pub(crate) fn no_bounds() -> Bounds<ScaledPixels> {
    Bounds {
        origin: point(ScaledPixels(0.), ScaledPixels(0.)),
        size: size(ScaledPixels(0.), ScaledPixels(0.)),
    }
}

/// The union of two extents, where an empty one counts as nothing.
pub(crate) fn union_bounds(
    a: Bounds<ScaledPixels>,
    b: Bounds<ScaledPixels>,
) -> Bounds<ScaledPixels> {
    let empty = |b: &Bounds<ScaledPixels>| b.size.width.0 <= 0.0 || b.size.height.0 <= 0.0;
    if empty(&a) {
        return b;
    }
    if empty(&b) {
        return a;
    }
    a.union(&b)
}

/// The bounds a transformed extent covers.
pub(crate) fn transform_bounds(
    bounds: Bounds<ScaledPixels>,
    transform: &TransformationMatrix,
) -> Bounds<ScaledPixels> {
    let corners = [
        bounds.origin,
        bounds.top_right(),
        bounds.bottom_left(),
        bounds.bottom_right(),
    ]
    .map(|corner| transform.apply_f32([corner.x.0, corner.y.0]));
    let left = corners.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
    let top = corners.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
    let right = corners
        .iter()
        .map(|c| c[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = corners
        .iter()
        .map(|c| c[1])
        .fold(f32::NEG_INFINITY, f32::max);
    Bounds::from_corners(
        point(ScaledPixels(left), ScaledPixels(top)),
        point(ScaledPixels(right), ScaledPixels(bottom)),
    )
}

/// A transform in window pixels, scaled to device pixels.
pub(crate) fn to_device_transform(
    transform: TransformationMatrix,
    scale_factor: f32,
) -> TransformationMatrix {
    TransformationMatrix {
        rotation_scale: transform.rotation_scale,
        translation: [
            transform.translation[0] * scale_factor,
            transform.translation[1] * scale_factor,
        ],
    }
}

/// Where hit testing maps the pointer through a transformed element. Each
/// hitbox inserted under a transform holds the innermost one; `parent` leads
/// out to the window.
#[derive(Debug)]
pub(crate) struct HitTransform {
    /// Takes a point in the parent's space to the element's own.
    pub inverse: TransformationMatrix,
    /// The content mask around the element, in the parent's space.
    pub parent_mask: ContentMask<Pixels>,
    /// The transform around this one, if any.
    pub parent: Option<std::sync::Arc<HitTransform>>,
}

impl HitTransform {
    /// The point in the element's own space, or `None` when a content mask
    /// outside the element hides it there.
    pub fn local_point(&self, position: Point<Pixels>) -> Option<Point<Pixels>> {
        let position = match &self.parent {
            Some(parent) => parent.local_point(position)?,
            None => position,
        };
        if !self.parent_mask.bounds.contains(&position) {
            return None;
        }
        let [x, y] = self.inverse.apply_f32([position.x.0, position.y.0]);
        Some(point(px(x), px(y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bounds, point, px, radians, size};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn saturate_one_changes_nothing() {
        let m = ColorMatrix::saturate(1.0);
        for (a, b) in m.0.iter().zip(ColorMatrix::IDENTITY.0.iter()) {
            assert!(close(*a, *b), "{m:?}");
        }
    }

    #[test]
    fn then_applies_the_first_matrix_first() {
        let m = ColorMatrix::brightness(0.5).then(&ColorMatrix::contrast(2.0));
        // red 1 -> 0.5 -> 2 * 0.5 + (0.5 - 1) = 0.5
        let r = m.0[0] * 1.0 + m.0[4];
        assert!(close(r, 0.5), "{m:?}");
    }

    #[test]
    fn a_rotation_turns_around_the_centre() {
        let bounds = Bounds::new(point(px(100.), px(100.)), size(px(40.), px(20.)));
        let m = ElementTransform::default()
            .rotate(radians(std::f32::consts::FRAC_PI_2))
            .to_matrix(bounds);
        // The centre stays put.
        let [cx, cy] = m.apply_f32([120., 110.]);
        assert!(close(cx, 120.) && close(cy, 110.));
        // The right middle turns clockwise to the bottom middle.
        let [x, y] = m.apply_f32([140., 110.]);
        assert!(close(x, 120.) && close(y, 130.), "{x} {y}");
    }

    #[test]
    fn functions_compose_in_css_order() {
        // translate(10px, 0) scale(2): scale first, then move.
        let t = ElementTransform::default()
            .origin(0., 0.)
            .translate(px(10.), px(0.))
            .scale(2.);
        let m = t.to_matrix(Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.))));
        let [x, y] = m.apply_f32([5., 5.]);
        assert!(close(x, 20.) && close(y, 10.), "{x} {y}");
    }

    #[test]
    fn a_region_grows_to_hold_its_rotation() {
        let m = ElementTransform::default()
            .rotate(radians(std::f32::consts::FRAC_PI_4))
            .to_matrix(Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.))));
        let region = LayerRegion::new(0, 0, 10, 10).transformed(&m);
        assert_eq!(region, LayerRegion::new(-3, -3, 16, 16));
    }

    #[test]
    fn hit_testing_maps_through_nested_transforms() {
        let outer = ElementTransform::default()
            .rotate(radians(std::f32::consts::FRAC_PI_2))
            .to_matrix(Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.))));
        let everywhere = ContentMask {
            bounds: Bounds::new(point(px(-1e6), px(-1e6)), size(px(2e6), px(2e6))),
        };
        let outer = std::sync::Arc::new(HitTransform {
            inverse: outer.inverse().unwrap(),
            parent_mask: everywhere.clone(),
            parent: None,
        });
        let inner = HitTransform {
            inverse: ElementTransform::default()
                .origin(0., 0.)
                .scale(2.)
                .to_matrix(Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.))))
                .inverse()
                .unwrap(),
            parent_mask: everywhere,
            parent: Some(outer),
        };
        // (90, 10) in the window is (10, 10) before the quarter turn about
        // (50, 50), and (5, 5) before the scale.
        let local = inner.local_point(point(px(90.), px(10.))).unwrap();
        assert!(close(local.x.0, 5.) && close(local.y.0, 5.), "{local:?}");
    }
}
