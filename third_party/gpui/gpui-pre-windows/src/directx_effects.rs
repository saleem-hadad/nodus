//! Effect layers for the DirectX renderer.
//!
//! An effect layer is a subtree of the scene that the renderer must paint
//! with a blur, a colour matrix, a mask, a blend mode, a transform or an
//! opacity. The scene marks the layer with `LayerBegin` and `LayerEnd`.
//! Between the two marks the renderer draws into a texture the size of the
//! whole frame, because the pixel shaders read `SV_Position` in
//! render-target pixels and a smaller texture would shift every rounded
//! corner and gradient. At `LayerEnd` this module blurs what it must and
//! paints the result back into the parent.
//!
//! Every texture holds premultiplied colour in the frame format. A layer
//! with a backdrop, a mask or a blend mode composites with blending off and
//! mixes with the pixel under it in the shader, so it needs a copy of the
//! parent region; any other layer blends in hardware.
//!
//! zed#63779 and #63780, with the Nodus patches: transforms, group opacity,
//! reading only what the content painted, a backdrop blurred inside its box
//! with mirrored edges, and the hardware-blended composite. See
//! third_party/gpui/PATCHES.md. The HLSL is compiled by fxc in release
//! builds on Windows only; nothing here has run on a GPU yet.

use std::slice;

use anyhow::{Context, Result};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

use crate::directx_renderer::{
    PipelineState, RENDER_TARGET_FORMAT, shader_resources::ShaderModule,
};
use gpui::{
    BlurPlan, Bounds, DevicePixels, EffectLayer, LayerRegion, ScaledPixels, Size,
    TransformationMatrix,
};

/// Textures kept for reuse between frames. Each open layer holds one
/// frame-sized texture plus a few region-sized ones, so a handful is
/// enough for deep nesting.
const POOL_LIMIT: usize = 16;

/// One pass of a separable Gaussian blur, laid out like the HLSL
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

/// One composite draw, laid out like the HLSL `LayerComposite`. Structured
/// buffers pack like C, so the field order matches the Metal one.
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

// A structured buffer packs its members at 4 bytes. The Rust structs match
// the HLSL ones field by field only while no member asks for more.
const _: () = assert!(std::mem::align_of::<BlurParams>() == 4);
const _: () = assert!(std::mem::size_of::<BlurParams>() == 32);
const _: () = assert!(std::mem::align_of::<LayerComposite>() == 4);

/// What one frame of layer work reads from the renderer.
pub(crate) struct EffectFrame<'a> {
    pub texture: &'a ID3D11Texture2D,
    pub view: &'a Option<ID3D11RenderTargetView>,
    pub viewport: D3D11_VIEWPORT,
    pub viewport_size: Size<DevicePixels>,
}

struct LayerTexture {
    texture: ID3D11Texture2D,
    view: Option<ID3D11RenderTargetView>,
    resource: Option<ID3D11ShaderResourceView>,
    width: u32,
    height: u32,
}

impl LayerTexture {
    fn viewport(&self) -> D3D11_VIEWPORT {
        D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: self.width as f32,
            Height: self.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }
    }
}

pub(crate) struct DirectXEffects {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    blur: PipelineState<BlurParams>,
    /// The blur whose width follows the mask of the layer, pixel by pixel.
    variable_blur: PipelineState<LayerComposite>,
    /// Reads what is under the layer and writes the result as is.
    composite: PipelineState<LayerComposite>,
    /// Writes the layer's premultiplied picture for the hardware to blend.
    blend: PipelineState<LayerComposite>,
    smooth_sampler: Option<ID3D11SamplerState>,
    exact_sampler: Option<ID3D11SamplerState>,
    mirror_sampler: Option<ID3D11SamplerState>,
    pool: Vec<LayerTexture>,
    /// The frame-sized textures of the layers whose content is drawing now.
    open: Vec<LayerTexture>,
}

impl DirectXEffects {
    pub fn new(device: &ID3D11Device, context: &ID3D11DeviceContext) -> Result<Self> {
        let blur = PipelineState::new(
            device,
            "layer_blur_pipeline",
            ShaderModule::Blur,
            1,
            create_blend_state_without_blending(device)?,
        )?;
        let variable_blur = PipelineState::new(
            device,
            "layer_variable_blur_pipeline",
            ShaderModule::VariableBlur,
            1,
            create_blend_state_without_blending(device)?,
        )?;
        let composite = PipelineState::new(
            device,
            "layer_composite_pipeline",
            ShaderModule::LayerComposite,
            1,
            create_blend_state_without_blending(device)?,
        )?;
        let blend = PipelineState::new(
            device,
            "layer_blend_pipeline",
            ShaderModule::LayerBlend,
            1,
            create_premultiplied_blend_state(device)?,
        )?;
        Ok(Self {
            device: device.clone(),
            context: context.clone(),
            blur,
            variable_blur,
            composite,
            blend,
            smooth_sampler: create_sampler(
                device,
                D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                D3D11_TEXTURE_ADDRESS_CLAMP,
            )?,
            exact_sampler: create_sampler(
                device,
                D3D11_FILTER_MIN_MAG_MIP_POINT,
                D3D11_TEXTURE_ADDRESS_CLAMP,
            )?,
            mirror_sampler: create_sampler(
                device,
                D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                D3D11_TEXTURE_ADDRESS_MIRROR,
            )?,
            pool: Vec::new(),
            open: Vec::new(),
        })
    }

    /// Call once per frame before the first layer. A frame that stopped on
    /// an error leaves its layers open.
    pub fn begin_frame(&mut self) {
        self.open.clear();
    }

    /// Drop every pooled texture, for example after a resize.
    pub fn forget_textures(&mut self) {
        self.pool.clear();
        self.open.clear();
    }

    pub fn has_open_layers(&self) -> bool {
        !self.open.is_empty()
    }

    /// The render target the scene draws into right now: the innermost
    /// open layer, or the frame.
    pub fn target_view<'a>(
        &'a self,
        frame_view: &'a Option<ID3D11RenderTargetView>,
    ) -> &'a Option<ID3D11RenderTargetView> {
        self.open.last().map_or(frame_view, |texture| &texture.view)
    }

    fn target_texture<'a>(&'a self, frame: &EffectFrame<'a>) -> &'a ID3D11Texture2D {
        self.open
            .last()
            .map_or(frame.texture, |texture| &texture.texture)
    }

    /// Opens a layer and points the output merger at its texture.
    pub fn begin_layer(&mut self, frame: &EffectFrame) -> Result<()> {
        let texture = self.take_texture(LayerRegion::of_viewport(frame.viewport_size))?;
        unsafe {
            self.context.ClearRenderTargetView(
                texture.view.as_ref().context("missing layer view")?,
                &[0.0; 4],
            );
            self.context
                .OMSetRenderTargets(Some(slice::from_ref(&texture.view)), None);
        }
        self.open.push(texture);
        Ok(())
    }

    /// Closes the innermost layer, paints it into its parent and points the
    /// output merger back at the parent.
    pub fn end_layer(&mut self, frame: &EffectFrame, layer: &EffectLayer) -> Result<()> {
        let Some(content_frame) = self.open.pop() else {
            return Ok(());
        };
        let context = self.context.clone();
        // Nothing may be a render target and a copy source at once.
        unsafe { context.OMSetRenderTargets(None, None) };
        let parent_view = self.target_view(frame.view).clone();
        let viewport = LayerRegion::of_viewport(frame.viewport_size);
        let source = layer.source_region(viewport);
        let target = layer.target_region(source, viewport);
        if source.is_empty() || target.is_empty() {
            self.give_back(content_frame);
            unsafe { context.OMSetRenderTargets(Some(slice::from_ref(&parent_view)), None) };
            return Ok(());
        }
        let parent_texture = self.target_texture(frame).clone();

        // A blurred content reads a blurred copy of what it painted; any
        // other reads its frame-sized texture as is.
        let (content, content_region) = if layer.blur > 0.0 {
            let copy = self.take_texture(source)?;
            copy_region(&context, &content_frame.texture, &copy.texture, source);
            self.give_back(content_frame);
            let blurred = self.blur(&copy, source, layer.blur, false)?;
            self.give_back(copy);
            (blurred, source)
        } else {
            (content_frame, viewport)
        };

        let under = if layer.reads_under() {
            let under = self.take_texture(target)?;
            copy_region(&context, &parent_texture, &under.texture, target);
            Some(under)
        } else {
            None
        };

        let (inverse, aa_scale) = layer.inverse_transform();
        let mut composite_params = LayerComposite {
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
            if let Some(under) = &under {
                if layer.has_mask != 0 {
                    backdrop = Some(self.variable_blur(under, target, composite_params)?);
                } else {
                    let region = layer.backdrop_region(target);
                    if !region.is_empty() {
                        let copy = self.take_texture(region)?;
                        copy_region(&context, &parent_texture, &copy.texture, region);
                        let blurred = self.blur(&copy, region, layer.backdrop_blur, true)?;
                        self.give_back(copy);
                        composite_params.backdrop = region.bounds();
                        backdrop = Some(blurred);
                    }
                }
            }
        }

        let pipeline = if under.is_some() {
            &mut self.composite
        } else {
            &mut self.blend
        };
        pipeline.update_buffer(&self.device, &context, &[composite_params])?;
        unsafe {
            context.OMSetRenderTargets(Some(slice::from_ref(&parent_view)), None);
            context.RSSetViewports(Some(slice::from_ref(&frame.viewport)));
            let under_resource = under.as_ref().unwrap_or(&content).resource.clone();
            let backdrop_resource = backdrop.as_ref().map_or_else(
                || under_resource.clone(),
                |texture| texture.resource.clone(),
            );
            context.PSSetShaderResources(2, Some(&[under_resource, backdrop_resource]));
            context.PSSetSamplers(1, Some(slice::from_ref(&self.exact_sampler)));
        }
        let pipeline = if under.is_some() {
            &self.composite
        } else {
            &self.blend
        };
        pipeline.draw_with_texture(
            &context,
            slice::from_ref(&content.resource),
            slice::from_ref(&self.smooth_sampler),
            1,
        )?;
        unbind_textures(&context);

        self.give_back(content);
        if let Some(texture) = under {
            self.give_back(texture);
        }
        if let Some(texture) = backdrop {
            self.give_back(texture);
        }
        Ok(())
    }

    /// Two separable gaussian passes over a shrunk copy of `source`. The
    /// result is a texture the size of the small region, which the
    /// composite samples with the linear sampler to scale it back up.
    /// `mirror` reflects the source at its edges, as CSS does for a
    /// backdrop; otherwise taps past the edge drop out.
    fn blur(
        &mut self,
        source: &LayerTexture,
        region: LayerRegion,
        sigma: f32,
        mirror: bool,
    ) -> Result<LayerTexture> {
        let plan = BlurPlan::new(sigma);
        let BlurPlan { sigma, radius, .. } = plan;
        let edge = mirror as u32;

        // Halve the source until it is `plan.scale` times smaller. One tap
        // at the centre of a 2 by 2 block is an exact box average.
        let mut size = LayerRegion::new(0, 0, region.width, region.height);
        let mut shrunk: Option<LayerTexture> = None;
        for _ in 0..plan.shrink_steps() {
            let next_size = size.halved();
            let next = self.take_texture(next_size)?;
            self.blur_pass(
                shrunk.as_ref().unwrap_or(source),
                &next,
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
            shrunk.as_ref().unwrap_or(source),
            &first,
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
            &first,
            &second,
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

    fn blur_pass(
        &mut self,
        source: &LayerTexture,
        target: &LayerTexture,
        params: BlurParams,
    ) -> Result<()> {
        let context = &self.context;
        self.blur.update_buffer(&self.device, context, &[params])?;
        unsafe {
            context.OMSetRenderTargets(Some(slice::from_ref(&target.view)), None);
            context.RSSetViewports(Some(&[target.viewport()]));
            context.PSSetSamplers(2, Some(slice::from_ref(&self.mirror_sampler)));
        }
        self.blur.draw_with_texture(
            context,
            slice::from_ref(&source.resource),
            slice::from_ref(&self.smooth_sampler),
            1,
        )?;
        unbind_textures(context);
        unsafe { context.OMSetRenderTargets(None, None) };
        Ok(())
    }

    /// Two passes of the variable blur, one per axis, at the full region
    /// size. Each pass reads the mask of the layer and blurs every pixel
    /// with the sigma the mask asks for there.
    fn variable_blur(
        &mut self,
        source: &LayerTexture,
        region: LayerRegion,
        composite: LayerComposite,
    ) -> Result<LayerTexture> {
        let size = LayerRegion::new(0, 0, region.width, region.height);
        let first = self.take_texture(size)?;
        self.variable_blur_pass(
            source,
            &first,
            LayerComposite {
                blur_axis: 0,
                ..composite
            },
        )?;
        let second = self.take_texture(size)?;
        self.variable_blur_pass(
            &first,
            &second,
            LayerComposite {
                blur_axis: 1,
                ..composite
            },
        )?;
        self.give_back(first);
        Ok(second)
    }

    fn variable_blur_pass(
        &mut self,
        source: &LayerTexture,
        target: &LayerTexture,
        params: LayerComposite,
    ) -> Result<()> {
        let context = &self.context;
        self.variable_blur
            .update_buffer(&self.device, context, &[params])?;
        unsafe {
            context.OMSetRenderTargets(Some(slice::from_ref(&target.view)), None);
            context.RSSetViewports(Some(&[target.viewport()]));
        }
        self.variable_blur.draw_with_texture(
            context,
            slice::from_ref(&source.resource),
            slice::from_ref(&self.smooth_sampler),
            1,
        )?;
        unbind_textures(context);
        unsafe { context.OMSetRenderTargets(None, None) };
        Ok(())
    }

    fn take_texture(&mut self, region: LayerRegion) -> Result<LayerTexture> {
        let width = region.width.max(1) as u32;
        let height = region.height.max(1) as u32;
        if let Some(index) = self
            .pool
            .iter()
            .position(|t| t.width == width && t.height == height)
        {
            return Ok(self.pool.swap_remove(index));
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe {
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))?
        };
        let texture = texture.context("layer texture was not created")?;
        let mut view = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&texture, None, Some(&mut view))?
        };
        let mut resource = None;
        unsafe {
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut resource))?
        };
        Ok(LayerTexture {
            texture,
            view,
            resource,
            width,
            height,
        })
    }

    fn give_back(&mut self, texture: LayerTexture) {
        if self.pool.len() >= POOL_LIMIT {
            self.pool.remove(0);
        }
        self.pool.push(texture);
    }
}

/// Copies `region` of `source` into the top left of `target`.
fn copy_region(
    context: &ID3D11DeviceContext,
    source: &ID3D11Texture2D,
    target: &ID3D11Texture2D,
    region: LayerRegion,
) {
    let source_box = D3D11_BOX {
        left: region.x.max(0) as u32,
        top: region.y.max(0) as u32,
        front: 0,
        right: (region.x + region.width) as u32,
        bottom: (region.y + region.height) as u32,
        back: 1,
    };
    unsafe {
        context.CopySubresourceRegion(target, 0, 0, 0, 0, source, 0, Some(&source_box));
    }
}

/// Clears t0 on both stages and t2 to t3, so a texture can become a render
/// target again.
fn unbind_textures(context: &ID3D11DeviceContext) {
    unsafe {
        context.VSSetShaderResources(0, Some(&[None]));
        context.PSSetShaderResources(0, Some(&[None]));
        context.PSSetShaderResources(2, Some(&[None, None]));
    }
}

fn create_sampler(
    device: &ID3D11Device,
    filter: D3D11_FILTER,
    address: D3D11_TEXTURE_ADDRESS_MODE,
) -> Result<Option<ID3D11SamplerState>> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: filter,
        AddressU: address,
        AddressV: address,
        AddressW: address,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_ALWAYS,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };
    let mut sampler = None;
    unsafe { device.CreateSamplerState(&desc, Some(&mut sampler))? };
    Ok(sampler)
}

fn create_blend_state_without_blending(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = false.into();
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    let mut state = None;
    unsafe { device.CreateBlendState(&desc, Some(&mut state))? };
    state.context("blend state was not created")
}

/// Paints a premultiplied picture over the target.
fn create_premultiplied_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    let mut state = None;
    unsafe { device.CreateBlendState(&desc, Some(&mut state))? };
    state.context("blend state was not created")
}
