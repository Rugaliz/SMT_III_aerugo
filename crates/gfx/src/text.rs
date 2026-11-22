//! Signed-distance-field text: glyph triangles drawn with the text shader and a block of shader
//! parameters per draw (the text material's values and the matrices of the text's canvas).

use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::{FRAMES_IN_FLIGHT, Gfx, TextureId, spirv};
use anyhow::{Context, Result};

/// One glyph vertex. Six per glyph (two triangles).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [u8; 4],
    /// Atlas coordinates.
    pub uv0: [f32; 2],
    /// x: packed face-texture coordinates, y: distance-field scale (negative for bold).
    pub uv1: [f32; 2],
}

/// The text shader's parameters. Matrices are column arrays. Layout matches the shaders' uniform block
/// (std140: every member is a vec4 or mat4).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextGlobals {
    pub object_to_world: [[f32; 4]; 4],
    pub world_to_object: [[f32; 4]; 4],
    pub matrix_vp: [[f32; 4]; 4],
    pub projection: [[f32; 4]; 4],
    pub screen_params: [f32; 4],
    pub camera_pos: [f32; 4],
    pub time: [f32; 4],
    pub face_color: [f32; 4],
    pub outline_color: [f32; 4],
    pub underlay_color: [f32; 4],
    pub face_tex_st: [f32; 4],
    pub outline_tex_st: [f32; 4],
    pub clip_rect: [f32; 4],
    pub env_matrix: [[f32; 4]; 3],
    /// Face dilate, outline width, outline softness, weight normal.
    pub p0: [f32; 4],
    /// Weight bold, scale ratio A, scale ratio C, gradient scale.
    pub p1: [f32; 4],
    /// Underlay offset x, y, dilate, softness.
    pub p2: [f32; 4],
    /// Vertex offset x, y, texture width, height.
    pub p3: [f32; 4],
    /// Scale x, y, perspective filter, unused.
    pub p4: [f32; 4],
    /// Face uv speed x, y, outline uv speed x, y.
    pub p5: [f32; 4],
    /// Brightness, contrast, mask softness x, y.
    pub p6: [f32; 4],
}

/// One text mesh to draw.
#[derive(Clone, Debug)]
pub struct TextDraw {
    pub atlas: TextureId,
    pub globals: TextGlobals,
    pub vertices: Vec<TextVertex>,
}

/// Something to draw, in order.
#[derive(Clone, Debug)]
pub enum DrawItem {
    Sprite(crate::Sprite),
    Text(Box<TextDraw>),
}

struct Buffer {
    buf: vk::Buffer,
    alloc: Option<Allocation>,
    size: u64,
}

struct TextFrame {
    vertices: Option<Buffer>,
    uniforms: Option<Buffer>,
    set: vk::DescriptorSet,
}

pub(crate) struct TextPipeline {
    ubo_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    pub(crate) layout: vk::PipelineLayout,
    pub(crate) pipeline: vk::Pipeline,
    pub(crate) white: TextureId,
    frames: Vec<TextFrame>,
    ubo_stride: u64,
}

/// Where one text draw's data went in the frame's buffers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextRange {
    pub first: u32,
    pub count: u32,
    pub ubo_offset: u32,
}

impl Gfx {
    pub(crate) fn create_text_pipeline(&mut self) -> Result<()> {
        let white = self.upload_texture(&aerugo_unity::Texture2D {
            name: "white".into(),
            width: 1,
            height: 1,
            format: aerugo_unity::TextureFormat::Rgba32,
            mip_count: 1,
            data: vec![255; 4],
        })?;
        let d = &self.device;
        // SAFETY: `self.device` is live; created objects are destroyed in `destroy_text_pipeline`.
        unsafe {
            let binding = [vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
            let ubo_layout = d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&binding),
                None,
            )?;
            let layouts = [ubo_layout, self.set_layout, self.set_layout, self.set_layout];
            let layout = d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
                None,
            )?;
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .descriptor_count(FRAMES_IN_FLIGHT as u32)];
            let pool = d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(FRAMES_IN_FLIGHT as u32).pool_sizes(&sizes),
                None,
            )?;
            let set_layouts = [ubo_layout; FRAMES_IN_FLIGHT];
            let sets = d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&set_layouts),
            )?;

            let vs_code = spirv(include_bytes!(concat!(env!("OUT_DIR"), "/text.vert.spv")))?;
            let fs_code = spirv(include_bytes!(concat!(env!("OUT_DIR"), "/text.frag.spv")))?;
            let vs = d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vs_code), None)?;
            let fs = d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fs_code), None)?;
            let main = c"main";
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vs)
                    .name(main),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fs)
                    .name(main),
            ];
            let bindings = [vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(size_of::<TextVertex>() as u32)
                .input_rate(vk::VertexInputRate::VERTEX)];
            let attr = |location: u32, format: vk::Format, offset: u32| {
                vk::VertexInputAttributeDescription::default()
                    .location(location)
                    .binding(0)
                    .format(format)
                    .offset(offset)
            };
            let attrs = [
                attr(0, vk::Format::R32G32B32_SFLOAT, 0),
                attr(1, vk::Format::R32G32B32_SFLOAT, 12),
                attr(2, vk::Format::R8G8B8A8_UNORM, 24),
                attr(3, vk::Format::R32G32_SFLOAT, 28),
                attr(4, vk::Format::R32G32_SFLOAT, 36),
            ];
            let vin = vk::PipelineVertexInputStateCreateInfo::default()
                .vertex_binding_descriptions(&bindings)
                .vertex_attribute_descriptions(&attrs);
            let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let vp = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
            let rs = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .line_width(1.0);
            let ms = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);
            // The text shader's pass: premultiplied blending, all channels written.
            let blend = [vk::PipelineColorBlendAttachmentState::default()
                .blend_enable(true)
                .src_color_blend_factor(vk::BlendFactor::ONE)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::ONE)
                .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .alpha_blend_op(vk::BlendOp::ADD)
                .color_write_mask(vk::ColorComponentFlags::RGBA)];
            let cb = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
            let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
            let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyn_states);
            let color_formats = [self.sc.format];
            let mut rendering =
                vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);
            let info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vin)
                .input_assembly_state(&ia)
                .viewport_state(&vp)
                .rasterization_state(&rs)
                .multisample_state(&ms)
                .color_blend_state(&cb)
                .dynamic_state(&dynamic)
                .layout(layout)
                .push_next(&mut rendering);
            let pipeline =
                d.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None).map_err(|(_, e)| e)?[0];
            d.destroy_shader_module(vs, None);
            d.destroy_shader_module(fs, None);

            let align = self.min_ubo_align.max(16);
            let ubo_stride = (size_of::<TextGlobals>() as u64).div_ceil(align) * align;
            let frames =
                sets.into_iter().map(|set| TextFrame { vertices: None, uniforms: None, set }).collect();
            self.text = Some(TextPipeline { ubo_layout, pool, layout, pipeline, white, frames, ubo_stride });
        }
        Ok(())
    }

    /// Copy this frame's text vertices and parameters into the frame's buffers (growing them as needed).
    /// Call after the frame's fence has signalled.
    pub(crate) fn prepare_text(&mut self, items: &[DrawItem]) -> Result<Vec<TextRange>> {
        let texts: Vec<&TextDraw> = items
            .iter()
            .filter_map(|i| match i {
                DrawItem::Text(t) => Some(t.as_ref()),
                DrawItem::Sprite(_) => None,
            })
            .collect();
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let frame = self.frame;
        let stride = self.text.as_ref().context("text pipeline")?.ubo_stride;
        let vbytes: u64 = texts.iter().map(|t| (t.vertices.len() * size_of::<TextVertex>()) as u64).sum();
        let ubytes = stride * texts.len() as u64;
        self.ensure_text_buffer(frame, true, vbytes.max(1))?;
        self.ensure_text_buffer(frame, false, ubytes)?;
        let tp = self.text.as_mut().context("text pipeline")?;
        let f = &mut tp.frames[frame];
        let mut ranges = Vec::with_capacity(texts.len());
        let mut first = 0u32;
        {
            let v = f.vertices.as_mut().and_then(|b| b.alloc.as_mut()).context("vertex buffer")?;
            let dst = v.mapped_slice_mut().context("vertex buffer not mapped")?;
            let mut at = 0usize;
            for t in &texts {
                let bytes: &[u8] = bytemuck::cast_slice(&t.vertices);
                dst[at..at + bytes.len()].copy_from_slice(bytes);
                at += bytes.len();
            }
        }
        let u = f.uniforms.as_mut().and_then(|b| b.alloc.as_mut()).context("uniform buffer")?;
        let dst = u.mapped_slice_mut().context("uniform buffer not mapped")?;
        for (k, t) in texts.iter().enumerate() {
            let at = k * stride as usize;
            dst[at..at + size_of::<TextGlobals>()].copy_from_slice(bytemuck::bytes_of(&t.globals));
            ranges.push(TextRange { first, count: t.vertices.len() as u32, ubo_offset: at as u32 });
            first += t.vertices.len() as u32;
        }
        Ok(ranges)
    }

    fn ensure_text_buffer(&mut self, frame: usize, vertices: bool, size: u64) -> Result<()> {
        let tp = self.text.as_ref().context("text pipeline")?;
        let f = &tp.frames[frame];
        let cur = if vertices { &f.vertices } else { &f.uniforms };
        if cur.as_ref().is_some_and(|b| b.size >= size) {
            return Ok(());
        }
        let size = size.next_power_of_two().max(4096);
        let usage =
            if vertices { vk::BufferUsageFlags::VERTEX_BUFFER } else { vk::BufferUsageFlags::UNIFORM_BUFFER };
        let d = self.device.clone();
        // SAFETY: the frame's fence has signalled, so its old buffer is no longer in use.
        unsafe {
            let buf = d.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage), None)?;
            let alloc = self.allocator.allocate(&AllocationCreateDesc {
                name: if vertices { "text vertices" } else { "text uniforms" },
                requirements: d.get_buffer_memory_requirements(buf),
                location: MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })?;
            d.bind_buffer_memory(buf, alloc.memory(), alloc.offset())?;
            let tp = self.text.as_mut().context("text pipeline")?;
            let f = &mut tp.frames[frame];
            let slot = if vertices { &mut f.vertices } else { &mut f.uniforms };
            if let Some(mut old) = slot.replace(Buffer { buf, alloc: Some(alloc), size }) {
                d.destroy_buffer(old.buf, None);
                if let Some(a) = old.alloc.take() {
                    self.allocator.free(a)?;
                }
            }
            if !vertices {
                let info = [vk::DescriptorBufferInfo::default().buffer(buf).offset(0).range(size_of::<
                    TextGlobals,
                >(
                )
                    as u64)];
                let write = [vk::WriteDescriptorSet::default()
                    .dst_set(f.set)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .buffer_info(&info)];
                d.update_descriptor_sets(&write, &[]);
            }
        }
        Ok(())
    }

    /// Record one text draw (the text pipeline and the frame's vertex buffer must be bound).
    pub(crate) fn record_text(&self, cmd: vk::CommandBuffer, t: &TextDraw, r: TextRange) {
        let Some(tp) = &self.text else { return };
        let atlas = &self.textures[t.atlas.0];
        let white = &self.textures[tp.white.0];
        // SAFETY: called while recording `cmd` inside the frame's rendering pass.
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                tp.layout,
                0,
                &[tp.frames[self.frame].set, atlas.set, white.set, white.set],
                &[r.ubo_offset],
            );
            self.device.cmd_draw(cmd, r.count, 1, r.first, 0);
        }
    }

    /// Bind the text pipeline and this frame's vertex buffer.
    pub(crate) fn bind_text(&self, cmd: vk::CommandBuffer) {
        let Some(tp) = &self.text else { return };
        let Some(v) = &tp.frames[self.frame].vertices else { return };
        // SAFETY: called while recording `cmd`.
        unsafe {
            self.device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, tp.pipeline);
            self.device.cmd_bind_vertex_buffers(cmd, 0, &[v.buf], &[0]);
        }
    }

    pub(crate) fn destroy_text_pipeline(&mut self) {
        let Some(mut tp) = self.text.take() else { return };
        // SAFETY: the device is idle (called from `Drop`).
        unsafe {
            for f in &mut tp.frames {
                for b in [f.vertices.take(), f.uniforms.take()].into_iter().flatten() {
                    let mut b = b;
                    self.device.destroy_buffer(b.buf, None);
                    if let Some(a) = b.alloc.take() {
                        let _ = self.allocator.free(a);
                    }
                }
            }
            self.device.destroy_pipeline(tp.pipeline, None);
            self.device.destroy_pipeline_layout(tp.layout, None);
            self.device.destroy_descriptor_pool(tp.pool, None);
            self.device.destroy_descriptor_set_layout(tp.ubo_layout, None);
        }
    }
}
