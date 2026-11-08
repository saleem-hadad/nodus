//! Effect layers for the wgpu renderer.
//!
//! An effect layer is a subtree of the scene that the renderer must paint
//! with a blur, a colour matrix, a mask, a blend mode, a transform or an
//! opacity. The scene marks the layer with `LayerBegin` and `LayerEnd`.
//! Between the two marks the renderer draws into a texture the size of the
//! whole frame, because the fragment shaders read their pixel position in
//! render-target pixels and a smaller texture would shift every rounded
//! corner and gradient. At `LayerEnd` this module blurs what it must and
//! paints the result back into the parent.
//!
//! Every texture holds premultiplied colour, in the format of the frame. A
//! layer with a backdrop, a mask or a blend mode composites with blending
//! off and mixes with the pixel under it in the shader, so it needs a copy
//! of the parent region, which needs `COPY_SRC` on the frame texture; any
//! other layer blends in hardware.
//!
//! zed#63778 and #63780, with the Nodus patches: transforms, group opacity,
//! reading only what the content painted, a backdrop blurred inside its box
//! with mirrored edges, and the hardware-blended composite. See
//! third_party/gpui/PATCHES.md.

use gpui::{
    BlurPlan, Bounds, DevicePixels, EffectLayer, LayerRegion, ScaledPixels, Size,
    TransformationMatrix,
};

/// Textures kept for reuse between frames. Each open layer holds one
/// frame-sized texture plus a few region-sized ones, so a handful is
/// enough for deep nesting.
const POOL_LIMIT: usize = 16;

/// First size of the parameter buffer. It grows when a frame needs more.
const PARAMS_CAPACITY: u64 = 64 * 1024;

/// One pass of a separable Gaussian blur, laid out like the WGSL
/// `BlurParams`.
#[repr(C)]
#[derive(Clone, Copy)]
struct BlurParams {
    /// One tap in texture coordinates of the source.
    step: [f32; 2],
    /// Standard deviation in taps.
    sigma: f32,
    /// Taps on each side of the centre.
    radius: i32,
    /// 1 mirrors the source at its edges; 0 drops the taps past them.
    edge: u32,
    pad: [u32; 3],
}

/// One composite draw, laid out like the WGSL `LayerComposite`, field for
/// field like the Metal one.
#[repr(C)]
#[derive(Clone, Copy)]
struct LayerComposite {
    layer: EffectLayer,
    /// The pixels of the target the composite writes. The under texture
    /// covers them.
    target: Bounds<ScaledPixels>,
    /// Where the content texture sits in the layer's space.
    source: Bounds<ScaledPixels>,
    /// Where the backdrop texture sits in the target.
    backdrop: Bounds<ScaledPixels>,
    /// Takes a pixel of the target to the layer's space.
    inverse: TransformationMatrix,
    /// Target pixels per layer pixel, which sets the width of anti-aliased
    /// edges.
    aa_scale: f32,
    /// 1 when the content is read between its pixels: blurred or moved.
    smooth_content: u32,
    /// Which axis a variable blur pass runs along: 0 is x, 1 is y. The
    /// composite draw does not read it.
    blur_axis: u32,
    pad: u32,
}

// `ParamsBuffer::write` reads these structs as raw bytes, which is only
// sound when every byte is initialized. Both hold 4-byte fields alone, down
// through `EffectLayer`, so the compiler inserts no padding. The size and
// offset checks keep the Rust structs in step with the WGSL ones, whose
// `Bounds` and matrix members align to 8 bytes.
const _: () = assert!(std::mem::size_of::<BlurParams>() == 32);
const _: () = assert!(std::mem::size_of::<EffectLayer>().is_multiple_of(8));
const _: () = assert!(std::mem::size_of::<LayerComposite>().is_multiple_of(8));
const _: () = assert!(std::mem::offset_of!(LayerComposite, target).is_multiple_of(8));
const _: () = assert!(std::mem::offset_of!(LayerComposite, inverse).is_multiple_of(8));
const _: () = assert!(std::mem::offset_of!(EffectLayer, painted).is_multiple_of(8));
const _: () = assert!(std::mem::offset_of!(EffectLayer, transform).is_multiple_of(8));

/// Bind group layouts shared by the blur and composite pipelines.
#[derive(Clone)]
pub(crate) struct EffectLayouts {
    /// Group 1: one storage buffer with `BlurParams` or `LayerComposite`.
    pub params: wgpu::BindGroupLayout,
    /// Group 2: content, under and backdrop textures plus three samplers.
    pub textures: wgpu::BindGroupLayout,
}

impl EffectLayouts {
    pub fn new(device: &wgpu::Device) -> Self {
        let params = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect_params_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(ty),
            count: None,
        };
        let textures = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect_textures_layout"),
            entries: &[
                texture(0),
                texture(1),
                texture(2),
                sampler(3, wgpu::SamplerBindingType::Filtering),
                sampler(4, wgpu::SamplerBindingType::NonFiltering),
                sampler(5, wgpu::SamplerBindingType::Filtering),
            ],
        });
        Self { params, textures }
    }
}

pub(crate) struct EffectPipelines {
    blur: wgpu::RenderPipeline,
    /// The blur whose width follows the mask of the layer, pixel by pixel.
    variable_blur: wgpu::RenderPipeline,
    /// Reads what is under the layer and writes the result as is.
    composite: wgpu::RenderPipeline,
    /// Writes the layer's premultiplied picture for the hardware to blend.
    blend: wgpu::RenderPipeline,
}

impl EffectPipelines {
    pub fn new(
        device: &wgpu::Device,
        module: &wgpu::ShaderModule,
        globals: &wgpu::BindGroupLayout,
        layouts: &EffectLayouts,
        format: wgpu::TextureFormat,
    ) -> Self {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effect_pipeline_layout"),
            bind_group_layouts: &[
                Some(globals),
                Some(&layouts.params),
                Some(&layouts.textures),
            ],
            immediate_size: 0,
        });
        let create = |name: &str, vs_entry: &str, fs_entry: &str, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs_entry),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs_entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            blur: create("effect_blur", "vs_blur", "fs_blur", None),
            variable_blur: create(
                "effect_variable_blur",
                "vs_variable_blur",
                "fs_variable_blur",
                None,
            ),
            composite: create(
                "effect_composite",
                "vs_layer_composite",
                "fs_layer_composite",
                None,
            ),
            blend: create(
                "effect_blend",
                "vs_layer_composite",
                "fs_layer_blend",
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
        }
    }
}

/// What one frame of layer work reads from the renderer.
pub(crate) struct EffectFrame<'a> {
    pub pipelines: &'a EffectPipelines,
    pub globals: &'a wgpu::BindGroup,
    /// The frame's texture, when it can be copied from. Without it, layers
    /// that read what is under them composite as if they didn't.
    pub texture: Option<&'a wgpu::Texture>,
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub viewport_size: Size<DevicePixels>,
}

struct LayerTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct Samplers {
    smooth: wgpu::Sampler,
    exact: wgpu::Sampler,
    mirror: wgpu::Sampler,
}

/// A bump allocator over one storage buffer for the per-draw parameters.
struct ParamsBuffer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    used: u64,
    alignment: u64,
}

impl ParamsBuffer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            layout: layout.clone(),
            buffer: Self::create(device, PARAMS_CAPACITY),
            used: 0,
            alignment: u64::from(device.limits().min_storage_buffer_offset_alignment),
        }
    }

    fn create(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect_params"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Appends `data` and returns a bind group for it. Old bind groups keep
    /// an old buffer alive when the buffer grows.
    fn write<T: Copy>(&mut self, data: &T) -> wgpu::BindGroup {
        let size = std::mem::size_of::<T>() as u64;
        let offset = self.used.next_multiple_of(self.alignment);
        if offset + size > self.buffer.size() {
            let capacity = (offset + size).max(self.buffer.size() * 2);
            self.buffer = Self::create(&self.device, capacity);
        }
        // SAFETY: `T` is `BlurParams` or `LayerComposite`. The assertions at
        // their definitions keep them free of padding, so every byte of the
        // live `data` is initialized, and the slice spans exactly its size.
        let bytes = unsafe {
            std::slice::from_raw_parts((data as *const T).cast::<u8>(), std::mem::size_of::<T>())
        };
        self.queue.write_buffer(&self.buffer, offset, bytes);
        self.used = offset + size;
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect_params_bind_group"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.buffer,
                    offset,
                    size: wgpu::BufferSize::new(size),
                }),
            }],
        })
    }
}

/// Textures kept between frames, matched by size.
struct TexturePool {
    device: wgpu::Device,
    textures: Vec<LayerTexture>,
}

impl TexturePool {
    fn take(&mut self, region: LayerRegion, format: wgpu::TextureFormat) -> LayerTexture {
        let width = region.width.max(1) as u32;
        let height = region.height.max(1) as u32;
        if let Some(index) = self.textures.iter().position(|t| {
            t.texture.width() == width
                && t.texture.height() == height
                && t.texture.format() == format
        }) {
            return self.textures.swap_remove(index);
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effect_layer_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        LayerTexture { texture, view }
    }

    fn give_back(&mut self, texture: LayerTexture) {
        if self.textures.len() >= POOL_LIMIT {
            self.textures.remove(0);
        }
        self.textures.push(texture);
    }
}

pub(crate) struct Effects {
    device: wgpu::Device,
    layouts: EffectLayouts,
    samplers: Samplers,
    params: ParamsBuffer,
    pool: TexturePool,
    /// The frame-sized textures of the layers whose content is drawing now.
    open: Vec<LayerTexture>,
}

impl Effects {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, layouts: &EffectLayouts) -> Self {
        let sampler = |label, filter, address| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            device: device.clone(),
            layouts: layouts.clone(),
            samplers: Samplers {
                smooth: sampler(
                    "effect_smooth_sampler",
                    wgpu::FilterMode::Linear,
                    wgpu::AddressMode::ClampToEdge,
                ),
                exact: sampler(
                    "effect_exact_sampler",
                    wgpu::FilterMode::Nearest,
                    wgpu::AddressMode::ClampToEdge,
                ),
                mirror: sampler(
                    "effect_mirror_sampler",
                    wgpu::FilterMode::Linear,
                    wgpu::AddressMode::MirrorRepeat,
                ),
            },
            params: ParamsBuffer::new(device, queue, &layouts.params),
            pool: TexturePool {
                device: device.clone(),
                textures: Vec::new(),
            },
            open: Vec::new(),
        }
    }

    /// Call once per frame before the first layer.
    pub fn begin_frame(&mut self) {
        self.params.used = 0;
        self.open.clear();
    }

    /// Drop every pooled texture, for example after a resize.
    pub fn forget_textures(&mut self) {
        self.pool.textures.clear();
        self.open.clear();
    }

    pub fn has_open_layers(&self) -> bool {
        !self.open.is_empty()
    }

    /// The view the scene draws into right now: the innermost open layer,
    /// or the frame.
    pub fn target_view<'a>(&'a self, frame: &EffectFrame<'a>) -> &'a wgpu::TextureView {
        self.open.last().map_or(frame.view, |texture| &texture.view)
    }

    /// Opens a layer. The caller must have ended the current render pass
    /// and must begin a new one on `target_view` afterwards.
    pub fn begin_layer(&mut self, frame: &EffectFrame, encoder: &mut wgpu::CommandEncoder) {
        let texture = self
            .pool
            .take(LayerRegion::of_viewport(frame.viewport_size), frame.format);
        drop(begin_pass(encoder, &texture.view, true, "layer_clear"));
        self.open.push(texture);
    }

    /// Closes the innermost layer and paints it into its parent. The caller
    /// must have ended the current render pass and must begin a new one on
    /// `target_view` afterwards.
    pub fn end_layer(
        &mut self,
        frame: &EffectFrame,
        encoder: &mut wgpu::CommandEncoder,
        layer: &EffectLayer,
    ) {
        let Some(content_frame) = self.open.pop() else {
            return;
        };
        let format = frame.format;
        let viewport = LayerRegion::of_viewport(frame.viewport_size);
        let source = layer.source_region(viewport);
        let target = layer.target_region(source, viewport);
        if source.is_empty() || target.is_empty() {
            self.pool.give_back(content_frame);
            return;
        }
        let parent_view = self.target_view(frame).clone();
        // The parent can be copied from when it is a layer, or a frame that
        // allows it.
        let parent_texture = match self.open.last() {
            Some(texture) => Some(texture.texture.clone()),
            None => frame.texture.cloned(),
        };

        // A blurred content reads a blurred copy of what it painted; any
        // other reads its frame-sized texture as is.
        let (content, content_region) = if layer.blur > 0.0 {
            let copy = self.pool.take(source, format);
            copy_region(encoder, &content_frame.texture, &copy.texture, source);
            self.pool.give_back(content_frame);
            let blurred = self.blur(frame, encoder, &copy, source, layer.blur, false);
            self.pool.give_back(copy);
            (blurred, source)
        } else {
            (content_frame, viewport)
        };

        let under = match &parent_texture {
            Some(parent) if layer.reads_under() => {
                let under = self.pool.take(target, format);
                copy_region(encoder, parent, &under.texture, target);
                Some(under)
            }
            _ => None,
        };

        let (inverse, aa_scale) = layer.inverse_transform();
        let mut composite = LayerComposite {
            layer: *layer,
            target: target.bounds(),
            source: content_region.bounds(),
            backdrop: target.bounds(),
            inverse,
            aa_scale,
            smooth_content: (layer.blur > 0.0 || layer.has_transform != 0) as u32,
            blur_axis: 0,
            pad: 0,
        };

        // CSS blurs the backdrop inside the border box, mirrored at its
        // edges. A mask asks instead for a blur whose width follows the
        // mask, pixel by pixel, which runs at full size over the target.
        let mut backdrop = None;
        if layer.has_backdrop != 0 && layer.backdrop_blur > 0.0 {
            if let (Some(under), Some(parent)) = (&under, &parent_texture) {
                if layer.has_mask != 0 {
                    backdrop = Some(self.variable_blur(frame, encoder, under, target, &composite));
                } else {
                    let region = layer.backdrop_region(target);
                    if !region.is_empty() {
                        let copy = self.pool.take(region, format);
                        copy_region(encoder, parent, &copy.texture, region);
                        let blurred =
                            self.blur(frame, encoder, &copy, region, layer.backdrop_blur, true);
                        self.pool.give_back(copy);
                        composite.backdrop = region.bounds();
                        backdrop = Some(blurred);
                    }
                }
            }
        }

        let params = self.params.write(&composite);
        let under_texture = under.as_ref().unwrap_or(&content);
        let backdrop_texture = backdrop.as_ref().unwrap_or(under_texture);
        let textures = self.texture_bind_group(&content, under_texture, backdrop_texture);
        {
            let mut pass = begin_pass(encoder, &parent_view, false, "layer_composite");
            pass.set_pipeline(if under.is_some() {
                &frame.pipelines.composite
            } else {
                &frame.pipelines.blend
            });
            pass.set_bind_group(0, frame.globals, &[]);
            pass.set_bind_group(1, &params, &[]);
            pass.set_bind_group(2, &textures, &[]);
            pass.draw(0..4, 0..1);
        }

        self.pool.give_back(content);
        if let Some(texture) = under {
            self.pool.give_back(texture);
        }
        if let Some(texture) = backdrop {
            self.pool.give_back(texture);
        }
    }

    /// Two separable gaussian passes over a shrunk copy of `source`. The
    /// result is a texture the size of the small region, which the
    /// composite samples with the linear sampler to scale it back up.
    /// `mirror` reflects the source at its edges, as CSS does for a
    /// backdrop; otherwise taps past the edge drop out.
    fn blur(
        &mut self,
        frame: &EffectFrame,
        encoder: &mut wgpu::CommandEncoder,
        source: &LayerTexture,
        region: LayerRegion,
        sigma: f32,
        mirror: bool,
    ) -> LayerTexture {
        let plan = BlurPlan::new(sigma);
        let BlurPlan { sigma, radius, .. } = plan;
        let edge = mirror as u32;
        let format = frame.format;

        // Halve the source until it is `plan.scale` times smaller. One tap
        // at the centre of a 2 by 2 block is an exact box average.
        let mut size = LayerRegion::new(0, 0, region.width, region.height);
        let mut shrunk: Option<LayerTexture> = None;
        for _ in 0..plan.shrink_steps() {
            let next_size = size.halved();
            let next = self.pool.take(next_size, format);
            self.blur_pass(
                frame,
                encoder,
                shrunk.as_ref().unwrap_or(source),
                &next,
                BlurParams {
                    step: [0.0, 0.0],
                    sigma: 1.0,
                    radius: 0,
                    edge,
                    pad: [0; 3],
                },
            );
            if let Some(texture) = shrunk {
                self.pool.give_back(texture);
            }
            shrunk = Some(next);
            size = next_size;
        }

        let first = self.pool.take(size, format);
        self.blur_pass(
            frame,
            encoder,
            shrunk.as_ref().unwrap_or(source),
            &first,
            BlurParams {
                step: [1.0 / size.width as f32, 0.0],
                sigma,
                radius,
                edge,
                pad: [0; 3],
            },
        );
        if let Some(texture) = shrunk {
            self.pool.give_back(texture);
        }
        let second = self.pool.take(size, format);
        self.blur_pass(
            frame,
            encoder,
            &first,
            &second,
            BlurParams {
                step: [0.0, 1.0 / size.height as f32],
                sigma,
                radius,
                edge,
                pad: [0; 3],
            },
        );
        self.pool.give_back(first);
        second
    }

    fn blur_pass(
        &mut self,
        frame: &EffectFrame,
        encoder: &mut wgpu::CommandEncoder,
        source: &LayerTexture,
        target: &LayerTexture,
        params: BlurParams,
    ) {
        let params = self.params.write(&params);
        let textures = self.texture_bind_group(source, source, source);
        let mut pass = begin_pass(encoder, &target.view, true, "layer_blur");
        pass.set_pipeline(&frame.pipelines.blur);
        pass.set_bind_group(0, frame.globals, &[]);
        pass.set_bind_group(1, &params, &[]);
        pass.set_bind_group(2, &textures, &[]);
        pass.draw(0..4, 0..1);
    }

    /// Blurs `source` with the Gaussian the mask of the layer asks for at
    /// each pixel: the mask value there times the `backdrop_blur` sigma.
    /// Two passes, one per axis, at full size, because every pixel has its
    /// own width and a shrunk texture would lose the sharp end.
    fn variable_blur(
        &mut self,
        frame: &EffectFrame,
        encoder: &mut wgpu::CommandEncoder,
        source: &LayerTexture,
        region: LayerRegion,
        composite: &LayerComposite,
    ) -> LayerTexture {
        let size = LayerRegion::new(0, 0, region.width, region.height);
        let first = self.pool.take(size, frame.format);
        self.variable_blur_pass(frame, encoder, source, &first, composite, 0);
        let second = self.pool.take(size, frame.format);
        self.variable_blur_pass(frame, encoder, &first, &second, composite, 1);
        self.pool.give_back(first);
        second
    }

    fn variable_blur_pass(
        &mut self,
        frame: &EffectFrame,
        encoder: &mut wgpu::CommandEncoder,
        source: &LayerTexture,
        target: &LayerTexture,
        composite: &LayerComposite,
        blur_axis: u32,
    ) {
        let composite = LayerComposite {
            blur_axis,
            ..*composite
        };
        let params = self.params.write(&composite);
        let textures = self.texture_bind_group(source, source, source);
        let mut pass = begin_pass(encoder, &target.view, true, "layer_variable_blur");
        pass.set_pipeline(&frame.pipelines.variable_blur);
        pass.set_bind_group(0, frame.globals, &[]);
        pass.set_bind_group(1, &params, &[]);
        pass.set_bind_group(2, &textures, &[]);
        pass.draw(0..4, 0..1);
    }

    fn texture_bind_group(
        &self,
        content: &LayerTexture,
        under: &LayerTexture,
        backdrop: &LayerTexture,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect_textures_bind_group"),
            layout: &self.layouts.textures,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&content.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&under.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&backdrop.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.samplers.smooth),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.samplers.exact),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.samplers.mirror),
                },
            ],
        })
    }
}

/// Copies `region` of `source` into the top left of `target`.
fn copy_region(
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    target: &wgpu::Texture,
    region: LayerRegion,
) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: region.x.max(0) as u32,
                y: region.y.max(0) as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: region.width as u32,
            height: region.height as u32,
            depth_or_array_layers: 1,
        },
    );
}

/// Begins a colour-only render pass on `view`, cleared to transparent
/// when `clear` is set and loaded otherwise.
pub(crate) fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    clear: bool,
    label: &str,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    })
}
