
// --- effect layers --- //
//
// zed#63778 and #63780, adapted to this snapshot's fills, with the Nodus
// patches (third_party/gpui/PATCHES.md): transforms, group opacity, a
// backdrop mirrored at the edges of its box, and a hardware-blended
// composite. The renderer draws the content of an effect layer into its own
// texture. These shaders then blur that texture and paint it over the frame
// with a colour matrix, a mask, a blend mode and a transform. Every texture
// here holds premultiplied colour.

struct BlurParams {
    step: vec2<f32>,
    sigma: f32,
    radius: i32,
    edge: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

struct EffectLayer {
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
    painted: Bounds,
    transform: TransformationMatrix,
    blur: f32,
    backdrop_blur: f32,
    opacity: f32,
    blend_mode: u32,
    has_mask: u32,
    has_backdrop: u32,
    clips_content: u32,
    has_transform: u32,
    color_matrix: array<f32, 20>,
    backdrop_matrix: array<f32, 20>,
    mask: Background,
}

struct LayerComposite {
    layer: EffectLayer,
    target: Bounds,
    source: Bounds,
    backdrop: Bounds,
    inverse: TransformationMatrix,
    aa_scale: f32,
    smooth_content: u32,
    // Which axis a variable blur pass runs along: 0 is x, 1 is y. The
    // composite draw does not read it.
    blur_axis: u32,
    pad: u32,
}

@group(1) @binding(0) var<storage, read> b_blur: BlurParams;
@group(1) @binding(0) var<storage, read> b_layer: LayerComposite;
@group(2) @binding(0) var t_layer_content: texture_2d<f32>;
@group(2) @binding(1) var t_layer_under: texture_2d<f32>;
@group(2) @binding(2) var t_layer_backdrop: texture_2d<f32>;
@group(2) @binding(3) var s_layer_smooth: sampler;
@group(2) @binding(4) var s_layer_exact: sampler;
@group(2) @binding(5) var s_layer_mirror: sampler;

fn full_screen_unit_vertex(vertex_id: u32) -> vec2<f32> {
    return vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
}

struct BlurVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_blur(@builtin(vertex_index) vertex_id: u32) -> BlurVarying {
    let unit_vertex = full_screen_unit_vertex(vertex_id);
    var out: BlurVarying;
    out.position = vec4<f32>(unit_vertex.x * 2.0 - 1.0, 1.0 - unit_vertex.y * 2.0, 0.0, 1.0);
    out.uv = unit_vertex;
    return out;
}

// One separable gaussian pass, or a 2 by 2 box average when the radius is
// 0. The step is one source texel along the axis of this pass, and the
// source texture is bound at every texture slot. With `edge` 1 the source
// mirrors at its edges, the edge mode CSS gives a backdrop; with 0 a tap
// past the edge drops out and the weights renormalize.
@fragment
fn fs_blur(input: BlurVarying) -> @location(0) vec4<f32> {
    let sigma = max(b_blur.sigma, 0.001);
    let mirror = b_blur.edge != 0u;
    var sum = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var i = -b_blur.radius; i <= b_blur.radius; i++) {
        let uv = input.uv + b_blur.step * f32(i);
        let weight = exp(-0.5 * f32(i * i) / (sigma * sigma));
        if (mirror) {
            sum += weight * textureSampleLevel(t_layer_content, s_layer_mirror, uv, 0.0);
        } else {
            if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
                continue;
            }
            sum += weight * textureSampleLevel(t_layer_content, s_layer_smooth, uv, 0.0);
        }
        weight_sum += weight;
    }
    return sum / weight_sum;
}

fn apply_color_matrix(m: array<f32, 20>, c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        m[0] * c.r + m[1] * c.g + m[2] * c.b + m[3] * c.a + m[4],
        m[5] * c.r + m[6] * c.g + m[7] * c.b + m[8] * c.a + m[9],
        m[10] * c.r + m[11] * c.g + m[12] * c.b + m[13] * c.a + m[14],
        m[15] * c.r + m[16] * c.g + m[17] * c.b + m[18] * c.a + m[19],
    );
}

fn unpremultiply(c: vec4<f32>) -> vec4<f32> {
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(c.rgb / c.a, c.a);
}

fn premultiply(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Runs a premultiplied colour through a 4x5 colour matrix in straight alpha.
fn filter_color(m: array<f32, 20>, c: vec4<f32>) -> vec4<f32> {
    let straight = unpremultiply(c);
    let filtered = clamp(apply_color_matrix(m, straight), vec4<f32>(0.0), vec4<f32>(1.0));
    return premultiply(filtered);
}

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.3, 0.59, 0.11));
}

fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    var out = c;
    if (n < 0.0) {
        out = l + (out - l) * l / max(l - n, 0.00001);
    }
    if (x > 1.0) {
        out = l + (out - l) * (1.0 - l) / max(x - l, 0.00001);
    }
    return out;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = l - lum(c);
    return clip_color(c + d);
}

fn sat(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let mx = max(c.r, max(c.g, c.b));
    let mn = min(c.r, min(c.g, c.b));
    if (mx <= mn) {
        return vec3<f32>(0.0);
    }
    return (c - mn) * s / (mx - mn);
}

// Separable blend modes, one channel at a time. `b` is the backdrop and `s`
// is the source, both in straight alpha.
fn blend_channel(mode: u32, b: f32, s: f32) -> f32 {
    switch mode {
        case 1u: {
            return b * s;
        }
        case 2u: {
            return b + s - b * s;
        }
        case 3u: {
            if (b <= 0.5) {
                return s * 2.0 * b;
            }
            return 1.0 - (1.0 - s) * (1.0 - (2.0 * b - 1.0));
        }
        case 4u: {
            return min(b, s);
        }
        case 5u: {
            return max(b, s);
        }
        case 6u: {
            if (b == 0.0) {
                return 0.0;
            }
            if (s >= 1.0) {
                return 1.0;
            }
            return min(1.0, b / (1.0 - s));
        }
        case 7u: {
            if (b >= 1.0) {
                return 1.0;
            }
            if (s <= 0.0) {
                return 0.0;
            }
            return 1.0 - min(1.0, (1.0 - b) / s);
        }
        case 8u: {
            if (s <= 0.5) {
                return b * 2.0 * s;
            }
            return 1.0 - (1.0 - b) * (1.0 - (2.0 * s - 1.0));
        }
        case 9u: {
            if (s <= 0.5) {
                return b - (1.0 - 2.0 * s) * b * (1.0 - b);
            }
            var d: f32;
            if (b <= 0.25) {
                d = ((16.0 * b - 12.0) * b + 4.0) * b;
            } else {
                d = sqrt(b);
            }
            return b + (2.0 * s - 1.0) * (d - b);
        }
        case 10u: {
            return abs(b - s);
        }
        case 11u: {
            return b + s - 2.0 * b * s;
        }
        default: {
            return s;
        }
    }
}

fn blend_colors(mode: u32, b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 0u: {
            return s;
        }
        case 12u: {
            return set_lum(set_sat(s, sat(b)), lum(b));
        }
        case 13u: {
            return set_lum(set_sat(b, sat(s)), lum(b));
        }
        case 14u: {
            return set_lum(s, lum(b));
        }
        case 15u: {
            return set_lum(b, lum(s));
        }
        case 16u: {
            return s;
        }
        default: {
            return vec3<f32>(
                blend_channel(mode, b.r, s.r),
                blend_channel(mode, b.g, s.g),
                blend_channel(mode, b.b, s.b),
            );
        }
    }
}

// Composites premultiplied `source` over premultiplied `backdrop` with the
// given blend mode, following the CSS compositing formula.
fn blend_over(mode: u32, backdrop: vec4<f32>, source: vec4<f32>) -> vec4<f32> {
    if (mode == 16u) {
        return min(backdrop + source, vec4<f32>(1.0));
    }
    var src = source;
    if (mode != 0u && source.a > 0.0 && backdrop.a > 0.0) {
        let cs = source.rgb / source.a;
        let cb = backdrop.rgb / backdrop.a;
        let mixed = (1.0 - backdrop.a) * cs + backdrop.a * blend_colors(mode, cb, cs);
        src = vec4<f32>(mixed * source.a, source.a);
    }
    return src + backdrop * (1.0 - src.a);
}

// Takes a pixel of the target to the layer's own space. The Rust side
// stores the matrix row-major, so it reads back transposed here.
fn layer_point(position: vec2<f32>) -> vec2<f32> {
    return transpose(b_layer.inverse.rotation_scale) * position + b_layer.inverse.translation;
}

// How much of the pixel at `point`, in the layer's space, lies inside the
// box. `aa_scale` is how many target pixels one layer pixel spans.
fn box_coverage(point: vec2<f32>, bounds: Bounds, radii: Corners, aa_scale: f32) -> f32 {
    return saturate(0.5 - quad_sdf(point, bounds, radii) * aa_scale);
}

// The layer's own picture at `point`, in its space: premultiplied, run
// through its colour matrix and faded by its opacity, clipped to its
// rounded box when it clips. Nothing outside its texture.
fn layer_content(point: vec2<f32>) -> vec4<f32> {
    let layer = b_layer.layer;
    let uv = (point - b_layer.source.origin) / b_layer.source.size;
    var content = vec4<f32>(0.0);
    if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
        if (b_layer.smooth_content != 0u) {
            content = textureSampleLevel(t_layer_content, s_layer_smooth, uv, 0.0);
        } else {
            content = textureSampleLevel(t_layer_content, s_layer_exact, uv, 0.0);
        }
    }
    content = filter_color(layer.color_matrix, content) * layer.opacity;
    if (layer.clips_content != 0u) {
        content *= box_coverage(point, layer.bounds, layer.corner_radii, b_layer.aa_scale);
    }
    return content;
}

struct LayerCompositeVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) mask_solid: vec4<f32>,
    @location(1) @interpolate(flat) mask_color0: vec4<f32>,
    @location(2) @interpolate(flat) mask_color1: vec4<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_layer_composite(@builtin(vertex_index) vertex_id: u32) -> LayerCompositeVarying {
    let unit_vertex = full_screen_unit_vertex(vertex_id);
    let mask = b_layer.layer.mask;
    let gradient = prepare_gradient_color(mask.tag, mask.color_space, mask.solid, mask.colors);
    var out: LayerCompositeVarying;
    out.position = to_device_position(unit_vertex, b_layer.target);
    out.mask_solid = gradient.solid;
    out.mask_color0 = gradient.color0;
    out.mask_color1 = gradient.color1;
    out.clip_distances = distance_from_clip_rect(unit_vertex, b_layer.target, b_layer.layer.content_mask);
    return out;
}

// The composite of a layer that leaves what is under it alone: its picture,
// premultiplied, for the pipeline to blend over the target.
@fragment
fn fs_layer_blend(input: LayerCompositeVarying) -> @location(0) vec4<f32> {
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    return layer_content(layer_point(input.position.xy));
}

// The composite of a layer with a backdrop, a mask or a blend mode. It reads
// what is under the layer and writes the result as is.
@fragment
fn fs_layer_composite(input: LayerCompositeVarying) -> @location(0) vec4<f32> {
    let layer = b_layer.layer;
    let position = input.position.xy;
    let uv = (position - b_layer.target.origin) / b_layer.target.size;
    let under = textureSampleLevel(t_layer_under, s_layer_exact, uv, 0.0);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return under;
    }
    let point = layer_point(position);
    let content = layer_content(point);
    let shape = box_coverage(point, layer.bounds, layer.corner_radii, b_layer.aa_scale);

    var keep = 1.0;
    if (layer.has_mask != 0u) {
        let box_min = layer.bounds.origin;
        let box_max = box_min + layer.bounds.size;
        let inside = all(point >= box_min) && all(point < box_max);
        if (inside) {
            keep = gradient_color(layer.mask, point, layer.bounds, input.mask_solid,
                                  input.mask_color0, input.mask_color1).a;
        } else {
            keep = 0.0;
        }
    }

    var base = under;
    if (layer.has_backdrop != 0u) {
        var backdrop = under;
        if (layer.backdrop_blur > 0.0) {
            if (layer.has_mask != 0u) {
                // The variable blur covers the target, already at the width
                // the mask asks for at every pixel.
                backdrop = textureSampleLevel(t_layer_backdrop, s_layer_smooth, uv, 0.0);
            } else {
                let backdrop_uv =
                    saturate((position - b_layer.backdrop.origin) / b_layer.backdrop.size);
                backdrop = textureSampleLevel(t_layer_backdrop, s_layer_smooth, backdrop_uv, 0.0);
            }
        }
        backdrop = mix(backdrop, filter_color(layer.backdrop_matrix, backdrop), keep);
        base = mix(under, backdrop, shape);
    }

    let result = blend_over(layer.blend_mode, base, content);
    return mix(base, result, keep);
}

struct VariableBlurVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) mask_solid: vec4<f32>,
    @location(2) @interpolate(flat) mask_color0: vec4<f32>,
    @location(3) @interpolate(flat) mask_color1: vec4<f32>,
}

@vertex
fn vs_variable_blur(@builtin(vertex_index) vertex_id: u32) -> VariableBlurVarying {
    let unit_vertex = full_screen_unit_vertex(vertex_id);
    let mask = b_layer.layer.mask;
    let gradient = prepare_gradient_color(mask.tag, mask.color_space, mask.solid, mask.colors);
    var out: VariableBlurVarying;
    out.position = vec4<f32>(unit_vertex.x * 2.0 - 1.0, 1.0 - unit_vertex.y * 2.0, 0.0, 1.0);
    out.uv = unit_vertex;
    out.mask_solid = gradient.solid;
    out.mask_color0 = gradient.color0;
    out.mask_color1 = gradient.color1;
    return out;
}

// The most taps a variable blur pass takes on each side of a pixel. The
// cap trims the tails of a sigma past 32 device pixels, and it bounds the
// cost of one huge blur.
const VARIABLE_BLUR_RADIUS_CAP: i32 = 96;

// One pass of the variable backdrop blur, the blur a gradient mask asks
// for. The sigma at each pixel is the mask value there times the full
// sigma, the contract of the variable blur filter of iOS. Two passes,
// one per axis, come close to the true variable Gaussian, because the
// mask changes slowly against the width of the kernel. Pairs of taps
// merge into one linear read at their weighted centre, which halves the
// reads. A tap past the source is dropped and the weights renormalize,
// as in the fixed blur.
@fragment
fn fs_variable_blur(input: VariableBlurVarying) -> @location(0) vec4<f32> {
    let layer = b_layer.layer;
    let position = b_layer.target.origin + input.uv * b_layer.target.size;
    let point = layer_point(position);
    let box_min = layer.bounds.origin;
    let box_max = box_min + layer.bounds.size;
    // The pass also blurs the pad around the layer, the pixels only the
    // other pass reads. Clamp the mask read to the bounds, so a pad row
    // above the layer blurs like the top row.
    let mask_point = clamp(point, box_min, box_max);
    let mask = saturate(gradient_color(layer.mask, mask_point, layer.bounds, input.mask_solid,
                                       input.mask_color0, input.mask_color1).a);
    let sigma = mask * layer.backdrop_blur;
    let centre = textureSampleLevel(t_layer_content, s_layer_smooth, input.uv, 0.0);
    if (sigma < 0.3) {
        return centre;
    }
    let radius = min(i32(ceil(3.0 * sigma)), VARIABLE_BLUR_RADIUS_CAP);
    let size = vec2<f32>(textureDimensions(t_layer_content));
    var step = vec2<f32>(0.0, 1.0 / size.y);
    if (b_layer.blur_axis == 0u) {
        step = vec2<f32>(1.0 / size.x, 0.0);
    }
    var sum = centre;
    var weight_sum = 1.0;
    for (var i = 1; i <= radius; i += 2) {
        let near_weight = exp(-0.5 * f32(i * i) / (sigma * sigma));
        let far_weight = exp(-0.5 * f32((i + 1) * (i + 1)) / (sigma * sigma));
        let pair = near_weight + far_weight;
        let offset = (f32(i) * near_weight + f32(i + 1) * far_weight) / pair;
        let reach = step * offset;
        let left = input.uv - reach;
        if (all(left >= vec2<f32>(0.0)) && all(left <= vec2<f32>(1.0))) {
            sum += pair * textureSampleLevel(t_layer_content, s_layer_smooth, left, 0.0);
            weight_sum += pair;
        }
        let right = input.uv + reach;
        if (all(right >= vec2<f32>(0.0)) && all(right <= vec2<f32>(1.0))) {
            sum += pair * textureSampleLevel(t_layer_content, s_layer_smooth, right, 0.0);
            weight_sum += pair;
        }
    }
    return sum / weight_sum;
}
