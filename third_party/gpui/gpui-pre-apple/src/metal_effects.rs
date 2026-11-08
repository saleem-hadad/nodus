//! Effect layers for the Metal renderer.
//!
//! An effect layer draws part of the scene into a texture of its own, then
//! paints that texture over the frame with a blur, a colour matrix, a mask,
//! a blend mode, a transform and an opacity. `Effects` owns the pipelines, a
//! pool of textures and the stack of layers that are open while a frame
//! draws.
//!
//! The content of a layer draws into a texture the size of the frame. The
//! fragment shaders read their pixel position from the render target, so a
//! smaller texture with a shifted viewport would put every rounded box and
//! gradient in the wrong place. The blur and composite steps then work on
//! copies of just the regions they need.
//!
//! zed#63777 and #63780 (metal-rs), ported to objc2-metal, with the Nodus
//! patches: transforms, group opacity, reading only what the content
//! painted, a backdrop clipped to the box and mirrored at its edges, and a
//! hardware-blended composite for layers that don't read what is under them.
//! See third_party/gpui/PATCHES.md.

use crate::metal_renderer::new_command_encoder_for_texture;
use anyhow::{Context as _, Result};
use gpui::{
    BlurPlan, Bounds, DevicePixels, EffectLayer, LayerRegion, ScaledPixels, Size,
    TransformationMatrix,
};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor,
    MTLCommandBuffer, MTLCommandEncoder, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin,
    MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLSize, MTLStorageMode, MTLStoreAction,
    MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};
use std::{mem, ptr};

type Texture = Retained<ProtocolObject<dyn MTLTexture>>;
type Encoder = Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>;
type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;

/// The biggest texture pool. Beyond this, a returned texture is dropped.
const POOL_LIMIT: usize = 24;

/// The parts of a frame every layer step needs.
pub(crate) struct Frame<'a> {
    pub command_buffer: &'a ProtocolObject<dyn MTLCommandBuffer>,
    /// The texture of the frame itself.
    pub texture: &'a ProtocolObject<dyn MTLTexture>,
    pub viewport_size: Size<DevicePixels>,
}

pub(crate) struct Effects {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    unit_vertices: Retained<ProtocolObject<dyn MTLBuffer>>,
    blur_pipeline_state: Pipeline,
    /// The blur whose width follows the mask of the layer, pixel by pixel.
    variable_blur_pipeline_state: Pipeline,
    /// Reads what is under the layer and writes the result as is: for
    /// backdrops, masks and blend modes.
    composite_pipeline_state: Pipeline,
    /// Writes the layer's premultiplied picture for the hardware to blend
    /// over what is under it: for every other layer.
    blend_pipeline_state: Pipeline,
    /// Textures free for the next layer, any size.
    pool: Vec<Texture>,
    /// The frame-sized textures of the layers whose content is drawing now.
    open: Vec<Texture>,
}

impl Effects {
    pub fn new(
        device: &ProtocolObject<dyn MTLDevice>,
        library: &ProtocolObject<dyn MTLLibrary>,
        unit_vertices: Retained<ProtocolObject<dyn MTLBuffer>>,
    ) -> Result<Self> {
        Ok(Self {
            device: device.into(),
            unit_vertices,
            blur_pipeline_state: build_effect_pipeline_state(
                device,
                library,
                "blur",
                "blur_vertex",
                "blur_fragment",
                false,
            )?,
            variable_blur_pipeline_state: build_effect_pipeline_state(
                device,
                library,
                "variable_blur",
                "variable_blur_vertex",
                "variable_blur_fragment",
                false,
            )?,
            composite_pipeline_state: build_effect_pipeline_state(
                device,
                library,
                "layer_composite",
                "layer_composite_vertex",
                "layer_composite_fragment",
                false,
            )?,
            blend_pipeline_state: build_effect_pipeline_state(
                device,
                library,
                "layer_blend",
                "layer_composite_vertex",
                "layer_blend_fragment",
                true,
            )?,
            pool: Vec::new(),
            open: Vec::new(),
        })
    }

    /// Drops the pooled textures, for when the window changes size.
    pub fn forget_textures(&mut self) {
        self.pool.clear();
    }

    /// Forgets the layers of a frame that never closed them. A frame that
    /// stops on an error leaves them open.
    pub fn begin_frame(&mut self) {
        self.open.clear();
    }

    /// Whether a layer is still open. True at the end of a frame means the
    /// scene pushed more layers than it popped.
    pub fn has_open_layers(&self) -> bool {
        !self.open.is_empty()
    }

    /// The texture the scene draws into now.
    fn target<'a>(&'a self, frame: &Frame<'a>) -> &'a ProtocolObject<dyn MTLTexture> {
        self.open
            .last()
            .map(|texture| &**texture)
            .unwrap_or(frame.texture)
    }

    /// A new encoder on the current target that keeps what is there.
    pub fn resume(&self, frame: &Frame) -> Result<Encoder> {
        new_command_encoder_for_texture(
            frame.command_buffer,
            self.target(frame),
            frame.viewport_size,
            None,
        )
    }

    /// Opens a layer. The returned encoder draws its content.
    pub fn begin_layer(&mut self, frame: &Frame) -> Result<Encoder> {
        let texture = self.take_texture(LayerRegion::of_viewport(frame.viewport_size))?;
        let encoder = new_command_encoder_for_texture(
            frame.command_buffer,
            &texture,
            frame.viewport_size,
            Some(MTLClearColor {
                red: 0.,
                green: 0.,
                blue: 0.,
                alpha: 0.,
            }),
        )?;
        self.open.push(texture);
        Ok(encoder)
    }

    /// Closes the innermost layer and paints it over its parent. The
    /// returned encoder continues the parent.
    pub fn end_layer(&mut self, frame: &Frame, layer: &EffectLayer) -> Result<Encoder> {
        let Some(content_frame) = self.open.pop() else {
            return self.resume(frame);
        };
        let viewport = LayerRegion::of_viewport(frame.viewport_size);
        let source = layer.source_region(viewport);
        let target = layer.target_region(source, viewport);
        if source.is_empty() || target.is_empty() {
            self.give_back(content_frame);
            return self.resume(frame);
        }
        let parent: Texture = self.target(frame).into();

        // A blurred content reads a blurred copy of what it painted; any
        // other reads its frame-sized texture as is.
        let (content, content_region) = if layer.blur > 0.0 {
            let copy = self.take_texture(source)?;
            copy_region(frame.command_buffer, &content_frame, &copy, source)?;
            self.give_back(content_frame);
            let blurred = self.blur(frame, &copy, source, layer.blur, false)?;
            self.give_back(copy);
            (blurred, source)
        } else {
            (content_frame, viewport)
        };

        // What is under the layer, so the shader can read it while it
        // writes the same pixels.
        let under = if layer.reads_under() {
            let under = self.take_texture(target)?;
            copy_region(frame.command_buffer, &parent, &under, target)?;
            Some(under)
        } else {
            None
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
        // mask, pixel by pixel, which reads the mask, so it runs at full
        // size over the whole target.
        let mut backdrop = None;
        if layer.has_backdrop != 0 && layer.backdrop_blur > 0.0 {
            if let Some(under) = &under {
                if layer.has_mask != 0 {
                    backdrop = Some(self.variable_blur(frame, under, target, &composite)?);
                } else {
                    let region = layer.backdrop_region(target);
                    if !region.is_empty() {
                        let copy = self.take_texture(region)?;
                        copy_region(frame.command_buffer, &parent, &copy, region)?;
                        let blurred = self.blur(frame, &copy, region, layer.backdrop_blur, true)?;
                        self.give_back(copy);
                        composite.backdrop = region.bounds();
                        backdrop = Some(blurred);
                    }
                }
            }
        }

        let encoder = new_command_encoder_for_texture(
            frame.command_buffer,
            &parent,
            frame.viewport_size,
            None,
        )?;
        encoder.setRenderPipelineState(if under.is_some() {
            &self.composite_pipeline_state
        } else {
            &self.blend_pipeline_state
        });
        let viewport_size = frame.viewport_size;
        let under_texture = under.as_deref().unwrap_or(&*content);
        let backdrop_texture = backdrop.as_deref().unwrap_or(under_texture);
        // SAFETY: The slots match the shader's `LayerInputIndex`, the bytes
        // are copied from live values of the types the shader reads, and
        // Metal retains the buffer and textures until the pass completes.
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(
                Some(&self.unit_vertices),
                0,
                LayerInputIndex::Vertices as usize,
            );
            encoder.setVertexBytes_length_atIndex(
                ptr::NonNull::from(&composite).cast(),
                mem::size_of::<LayerComposite>(),
                LayerInputIndex::Layer as usize,
            );
            encoder.setFragmentBytes_length_atIndex(
                ptr::NonNull::from(&composite).cast(),
                mem::size_of::<LayerComposite>(),
                LayerInputIndex::Layer as usize,
            );
            encoder.setVertexBytes_length_atIndex(
                ptr::NonNull::from(&viewport_size).cast(),
                mem::size_of_val(&viewport_size),
                LayerInputIndex::ViewportSize as usize,
            );
            encoder.setFragmentTexture_atIndex(
                Some(&content),
                LayerInputIndex::ContentTexture as usize,
            );
            encoder.setFragmentTexture_atIndex(
                Some(under_texture),
                LayerInputIndex::UnderTexture as usize,
            );
            encoder.setFragmentTexture_atIndex(
                Some(backdrop_texture),
                LayerInputIndex::BackdropTexture as usize,
            );
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
        }

        self.give_back(content);
        if let Some(texture) = under {
            self.give_back(texture);
        }
        if let Some(texture) = backdrop {
            self.give_back(texture);
        }
        Ok(encoder)
    }

    /// Blurs `source` with a Gaussian of `sigma` device pixels into a
    /// smaller texture. Two passes, one per axis, on a texture shrunk by
    /// `scale`, so a wide blur costs the same as a narrow one. `mirror`
    /// reflects the source at its edges, as CSS does for a backdrop;
    /// otherwise taps past the edge drop out.
    fn blur(
        &mut self,
        frame: &Frame,
        source: &ProtocolObject<dyn MTLTexture>,
        region: LayerRegion,
        sigma: f32,
        mirror: bool,
    ) -> Result<Texture> {
        let plan = BlurPlan::new(sigma);
        let BlurPlan { sigma, radius, .. } = plan;
        let edge = mirror as u32;

        // Halve the source until it is `plan.scale` times smaller. One tap
        // at the centre of a 2 by 2 block is an exact box average.
        let mut size = LayerRegion::new(0, 0, region.width, region.height);
        let mut shrunk: Option<Texture> = None;
        for _ in 0..plan.shrink_steps() {
            let next_size = size.halved();
            let next = self.take_texture(next_size)?;
            self.blur_pass(
                frame,
                shrunk.as_deref().unwrap_or(source),
                &next,
                next_size,
                BlurParams {
                    step: [0.0, 0.0],
                    sigma: 1.0,
                    radius: 0,
                    edge,
                    pad: [0; 3],
                },
            )?;
            if let Some(texture) = shrunk {
                self.give_back(texture);
            }
            shrunk = Some(next);
            size = next_size;
        }

        let first = self.take_texture(size)?;
        self.blur_pass(
            frame,
            shrunk.as_deref().unwrap_or(source),
            &first,
            size,
            BlurParams {
                step: [1.0 / size.width as f32, 0.0],
                sigma,
                radius,
                edge,
                pad: [0; 3],
            },
        )?;
        if let Some(texture) = shrunk {
            self.give_back(texture);
        }
        let second = self.take_texture(size)?;
        self.blur_pass(
            frame,
            &first,
            &second,
            size,
            BlurParams {
                step: [0.0, 1.0 / size.height as f32],
                sigma,
                radius,
                edge,
                pad: [0; 3],
            },
        )?;
        self.give_back(first);
        Ok(second)
    }

    /// Blurs `source` with the Gaussian the mask of the layer asks for at
    /// each pixel: the mask value there times the `backdrop_blur` sigma.
    /// Two passes, one per axis, at full size, because every pixel has its
    /// own width and a shrunk texture would lose the sharp end.
    fn variable_blur(
        &mut self,
        frame: &Frame,
        source: &ProtocolObject<dyn MTLTexture>,
        region: LayerRegion,
        composite: &LayerComposite,
    ) -> Result<Texture> {
        let size = LayerRegion::new(0, 0, region.width, region.height);
        let first = self.take_texture(size)?;
        self.variable_blur_pass(frame, source, &first, size, composite, 0)?;
        let second = self.take_texture(size)?;
        self.variable_blur_pass(frame, &first, &second, size, composite, 1)?;
        self.give_back(first);
        Ok(second)
    }

    fn variable_blur_pass(
        &self,
        frame: &Frame,
        source: &ProtocolObject<dyn MTLTexture>,
        target: &ProtocolObject<dyn MTLTexture>,
        target_region: LayerRegion,
        composite: &LayerComposite,
        blur_axis: u32,
    ) -> Result<()> {
        let composite = LayerComposite {
            blur_axis,
            ..*composite
        };
        let encoder = self.full_pass(
            frame,
            target,
            target_region,
            &self.variable_blur_pipeline_state,
        )?;
        // SAFETY: The slots match the shader's `BlurInputIndex`, the bytes
        // are copied from a live `LayerComposite`, and Metal retains the
        // texture until the pass completes.
        unsafe {
            encoder.setVertexBytes_length_atIndex(
                ptr::NonNull::from(&composite).cast(),
                mem::size_of::<LayerComposite>(),
                BlurInputIndex::Layer as usize,
            );
            encoder.setFragmentBytes_length_atIndex(
                ptr::NonNull::from(&composite).cast(),
                mem::size_of::<LayerComposite>(),
                BlurInputIndex::Layer as usize,
            );
            encoder.setFragmentTexture_atIndex(Some(source), BlurInputIndex::Source as usize);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
        }
        encoder.endEncoding();
        Ok(())
    }

    fn blur_pass(
        &self,
        frame: &Frame,
        source: &ProtocolObject<dyn MTLTexture>,
        target: &ProtocolObject<dyn MTLTexture>,
        target_region: LayerRegion,
        params: BlurParams,
    ) -> Result<()> {
        let encoder = self.full_pass(frame, target, target_region, &self.blur_pipeline_state)?;
        // SAFETY: The slots match the shader's `BlurInputIndex`, the bytes
        // are copied from a live `BlurParams`, and Metal retains the texture
        // until the pass completes.
        unsafe {
            encoder.setFragmentBytes_length_atIndex(
                ptr::NonNull::from(&params).cast(),
                mem::size_of::<BlurParams>(),
                BlurInputIndex::Params as usize,
            );
            encoder.setFragmentTexture_atIndex(Some(source), BlurInputIndex::Source as usize);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
        }
        encoder.endEncoding();
        Ok(())
    }

    /// Starts a pass that draws the unit quad over the whole of `target`.
    fn full_pass(
        &self,
        frame: &Frame,
        target: &ProtocolObject<dyn MTLTexture>,
        target_region: LayerRegion,
        pipeline: &ProtocolObject<dyn MTLRenderPipelineState>,
    ) -> Result<Encoder> {
        let descriptor = MTLRenderPassDescriptor::new();
        // SAFETY: Index zero is within the render pass's color attachment array.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(target));
        attachment.setLoadAction(MTLLoadAction::DontCare);
        attachment.setStoreAction(MTLStoreAction::Store);
        let encoder = frame
            .command_buffer
            .renderCommandEncoderWithDescriptor(&descriptor)
            .context("creating effect pass encoder")?;
        encoder.setViewport(MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: target_region.width as f64,
            height: target_region.height as f64,
            znear: 0.0,
            zfar: 1.0,
        });
        encoder.setRenderPipelineState(pipeline);
        // SAFETY: Six unit vertices match the shader's vertex slot.
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(
                Some(&self.unit_vertices),
                0,
                BlurInputIndex::Vertices as usize,
            );
        }
        Ok(encoder)
    }

    /// A texture of exactly the size of `region`, from the pool when one fits.
    fn take_texture(&mut self, region: LayerRegion) -> Result<Texture> {
        let width = region.width.max(1) as usize;
        let height = region.height.max(1) as usize;
        if let Some(index) = self
            .pool
            .iter()
            .position(|texture| texture.width() == width && texture.height() == height)
        {
            return Ok(self.pool.swap_remove(index));
        }
        let descriptor = MTLTextureDescriptor::new();
        // SAFETY: The dimensions are positive and this descriptor creates a 2D texture.
        unsafe {
            descriptor.setWidth(width);
            descriptor.setHeight(height);
        }
        descriptor.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        descriptor.setStorageMode(MTLStorageMode::Private);
        descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        self.device
            .newTextureWithDescriptor(&descriptor)
            .context("allocating effect layer texture")
    }

    fn give_back(&mut self, texture: Texture) {
        if self.pool.len() >= POOL_LIMIT {
            self.pool.remove(0);
        }
        self.pool.push(texture);
    }
}

/// Copies the pixels of `region` from `source`, a texture the size of the
/// frame, into the top left of `target`.
fn copy_region(
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    source: &ProtocolObject<dyn MTLTexture>,
    target: &ProtocolObject<dyn MTLTexture>,
    region: LayerRegion,
) -> Result<()> {
    let blit = command_buffer
        .blitCommandEncoder()
        .context("creating effect copy encoder")?;
    // SAFETY: The region lies inside `source` (every region is clipped to
    // the frame) and `target` is at least as big as the region.
    unsafe {
        blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
            source,
            0,
            0,
            MTLOrigin {
                x: region.x.max(0) as usize,
                y: region.y.max(0) as usize,
                z: 0,
            },
            MTLSize {
                width: region.width as usize,
                height: region.height as usize,
                depth: 1,
            },
            target,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
        );
    }
    blit.endEncoding();
    Ok(())
}

/// A pipeline for one of the effect passes. The blur and composite shaders
/// do their own mixing and write their output as is; `blend` paints a
/// premultiplied picture over the target.
fn build_effect_pipeline_state(
    device: &ProtocolObject<dyn MTLDevice>,
    library: &ProtocolObject<dyn MTLLibrary>,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    blend: bool,
) -> Result<Pipeline> {
    let vertex_fn = library
        .newFunctionWithName(&NSString::from_str(vertex_fn_name))
        .with_context(|| format!("locating vertex function {vertex_fn_name}"))?;
    let fragment_fn = library
        .newFunctionWithName(&NSString::from_str(fragment_fn_name))
        .with_context(|| format!("locating fragment function {fragment_fn_name}"))?;
    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setLabel(Some(&NSString::from_str(label)));
    descriptor.setVertexFunction(Some(vertex_fn.as_ref()));
    descriptor.setFragmentFunction(Some(fragment_fn.as_ref()));
    // SAFETY: Index zero is within the pipeline's color attachment array.
    let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    color_attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    color_attachment.setBlendingEnabled(blend);
    if blend {
        color_attachment.setRgbBlendOperation(MTLBlendOperation::Add);
        color_attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
        color_attachment.setSourceRGBBlendFactor(MTLBlendFactor::One);
        color_attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        color_attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        color_attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    }
    device
        .newRenderPipelineStateWithDescriptor_error(&descriptor)
        .with_context(|| format!("creating {label} render pipeline"))
}

#[repr(C)]
pub(crate) enum BlurInputIndex {
    Vertices = 0,
    Params = 1,
    Source = 2,
    /// The `LayerComposite` of the variable blur, which reads the mask.
    Layer = 3,
}

/// One pass of a separable Gaussian blur.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub(crate) struct BlurParams {
    /// One tap in texture coordinates of the source.
    pub step: [f32; 2],
    /// Standard deviation in taps.
    pub sigma: f32,
    /// Taps on each side of the centre.
    pub radius: i32,
    /// 1 mirrors the source at its edges; 0 drops the taps past them.
    pub edge: u32,
    pub pad: [u32; 3],
}

#[repr(C)]
pub(crate) enum LayerInputIndex {
    Vertices = 0,
    Layer = 1,
    ViewportSize = 2,
    ContentTexture = 3,
    UnderTexture = 4,
    BackdropTexture = 5,
}

/// Everything the composite shader needs to paint one layer.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub(crate) struct LayerComposite {
    pub layer: EffectLayer,
    /// The pixels of the target the composite writes. The under texture
    /// covers them.
    pub target: Bounds<ScaledPixels>,
    /// Where the content texture sits in the layer's space.
    pub source: Bounds<ScaledPixels>,
    /// Where the backdrop texture sits in the target.
    pub backdrop: Bounds<ScaledPixels>,
    /// Takes a pixel of the target to the layer's space.
    pub inverse: TransformationMatrix,
    /// Target pixels per layer pixel, which sets the width of anti-aliased
    /// edges.
    pub aa_scale: f32,
    /// 1 when the content is read between its pixels: blurred or moved.
    pub smooth_content: u32,
    /// Which axis a variable blur pass runs along: 0 is x, 1 is y. The
    /// composite draw does not read it.
    pub blur_axis: u32,
    pub pad: u32,
}
