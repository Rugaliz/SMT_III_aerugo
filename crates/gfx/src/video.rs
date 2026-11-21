//! Movie frames: three 8-bit planes (4:2:0) uploaded per frame and converted on the GPU into a 10-bit RGB
//! texture, which is then drawn like any other texture.

use anyhow::{Context, Result};
use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::{Gfx, GpuTexture, SpritePush, TextureId, full_range, spirv};

/// Format of the converted frame: 10 bits per color channel, like the original's movie render texture.
const RGB_FORMAT: vk::Format = vk::Format::A2B10G10R10_UNORM_PACK32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoId(pub usize);

struct Plane {
    image: vk::Image,
    view: vk::ImageView,
    alloc: Option<Allocation>,
    width: u32,
    height: u32,
}

pub(crate) struct VideoTarget {
    planes: [Plane; 3],
    set: vk::DescriptorSet,
    staging: vk::Buffer,
    staging_alloc: Option<Allocation>,
    /// The converted frame (an entry of the texture table).
    texture: TextureId,
    display: (u32, u32),
    /// A frame has been converted (the images hold defined contents).
    filled: bool,
}

/// Pipeline objects of the conversion pass, created on first use.
pub(crate) struct VideoPipeline {
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

impl Gfx {
    fn video_pipeline(&mut self) -> Result<&VideoPipeline> {
        if self.video_pipeline.is_none() {
            let d = &self.device;
            // SAFETY: `self.device` is live; created objects are destroyed in `Drop`.
            let p = unsafe {
                let bindings: Vec<_> = (0..3)
                    .map(|i| {
                        vk::DescriptorSetLayoutBinding::default()
                            .binding(i)
                            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                            .descriptor_count(1)
                            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                    })
                    .collect();
                let set_layout = d.create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )?;
                let push = [vk::PushConstantRange::default()
                    .stage_flags(vk::ShaderStageFlags::VERTEX)
                    .size(size_of::<SpritePush>() as u32)];
                let layouts = [set_layout];
                let layout = d.create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&layouts)
                        .push_constant_ranges(&push),
                    None,
                )?;
                let vs_code = spirv(include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv")))?;
                let fs_code = spirv(include_bytes!(concat!(env!("OUT_DIR"), "/video.frag.spv")))?;
                let vs =
                    d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vs_code), None)?;
                let fs =
                    d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fs_code), None)?;
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
                let vin = vk::PipelineVertexInputStateCreateInfo::default();
                let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
                    .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
                let vp = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
                let rs = vk::PipelineRasterizationStateCreateInfo::default()
                    .polygon_mode(vk::PolygonMode::FILL)
                    .cull_mode(vk::CullModeFlags::NONE)
                    .line_width(1.0);
                let ms = vk::PipelineMultisampleStateCreateInfo::default()
                    .rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let blend = [vk::PipelineColorBlendAttachmentState::default()
                    .blend_enable(false)
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let cb = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
                let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyn_states);
                let color_formats = [RGB_FORMAT];
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
                let pipeline = d
                    .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
                    .map_err(|(_, e)| e)?[0];
                d.destroy_shader_module(vs, None);
                d.destroy_shader_module(fs, None);
                VideoPipeline { set_layout, layout, pipeline }
            };
            self.video_pipeline = Some(p);
        }
        Ok(self.video_pipeline.as_ref().expect("created above"))
    }

    fn create_image(
        &mut self,
        format: vk::Format,
        w: u32,
        h: u32,
        usage: vk::ImageUsageFlags,
        name: &str,
    ) -> Result<Plane> {
        let d = self.device.clone();
        // SAFETY: the image and its memory belong to this device and are destroyed in `Drop`.
        unsafe {
            let image = d.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage),
                None,
            )?;
            let alloc = self.allocator.allocate(&AllocationCreateDesc {
                name,
                requirements: d.get_image_memory_requirements(image),
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })?;
            d.bind_image_memory(image, alloc.memory(), alloc.offset())?;
            let view = d.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(full_range(vk::ImageAspectFlags::COLOR)),
                None,
            )?;
            Ok(Plane { image, view, alloc: Some(alloc), width: w, height: h })
        }
    }

    /// A movie frame target for pictures of `coded` size (planes cover whole macroblocks) showing `display`.
    pub fn create_video(&mut self, coded: (u32, u32), display: (u32, u32)) -> Result<VideoId> {
        let set_layout = self.video_pipeline()?.set_layout;
        let (cw, ch) = coded;
        let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
        let planes = [
            self.create_image(vk::Format::R8_UNORM, cw, ch, usage, "video luma")?,
            self.create_image(vk::Format::R8_UNORM, cw / 2, ch / 2, usage, "video cb")?,
            self.create_image(vk::Format::R8_UNORM, cw / 2, ch / 2, usage, "video cr")?,
        ];
        let rgb = self.create_image(
            RGB_FORMAT,
            display.0,
            display.1,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            "video rgb",
        )?;
        let d = self.device.clone();
        let size: u64 = planes.iter().map(|p| u64::from(p.width) * u64::from(p.height)).sum();
        // SAFETY: descriptor sets come from this device's pool; the buffer is destroyed in `Drop`.
        unsafe {
            let layouts = [set_layout];
            let set = d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(&layouts),
            )?[0];
            let infos: Vec<[vk::DescriptorImageInfo; 1]> = planes
                .iter()
                .map(|p| {
                    [vk::DescriptorImageInfo::default()
                        .sampler(self.sampler)
                        .image_view(p.view)
                        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)]
                })
                .collect();
            let writes: Vec<_> = infos
                .iter()
                .enumerate()
                .map(|(i, info)| {
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(i as u32)
                        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                        .image_info(info)
                })
                .collect();
            d.update_descriptor_sets(&writes, &[]);

            let sprite_layouts = [self.set_layout];
            let tex_set = d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(&sprite_layouts),
            )?[0];
            let info = [vk::DescriptorImageInfo::default()
                .sampler(self.sampler)
                .image_view(rgb.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write = [vk::WriteDescriptorSet::default()
                .dst_set(tex_set)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&info)];
            d.update_descriptor_sets(&write, &[]);
            self.textures.push(GpuTexture {
                image: rgb.image,
                view: rgb.view,
                alloc: rgb.alloc,
                set: tex_set,
                width: rgb.width,
                height: rgb.height,
            });
            let texture = TextureId(self.textures.len() - 1);

            let staging = d.create_buffer(
                &vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_SRC),
                None,
            )?;
            let staging_alloc = self.allocator.allocate(&AllocationCreateDesc {
                name: "video staging",
                requirements: d.get_buffer_memory_requirements(staging),
                location: MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })?;
            d.bind_buffer_memory(staging, staging_alloc.memory(), staging_alloc.offset())?;
            self.videos.push(VideoTarget {
                planes,
                set,
                staging,
                staging_alloc: Some(staging_alloc),
                texture,
                display,
                filled: false,
            });
        }
        Ok(VideoId(self.videos.len() - 1))
    }

    /// The texture holding the last converted frame of `id` (draw it only after a first [`Self::upload_video`]).
    pub fn video_texture(&self, id: VideoId) -> TextureId {
        self.videos[id.0].texture
    }

    /// Upload one picture (planes with rows of the plane width, top to bottom) and convert it.
    pub fn upload_video(&mut self, id: VideoId, planes: [&[u8]; 3]) -> Result<()> {
        let pipe = self.video_pipeline()?;
        let (pipeline, layout) = (pipe.pipeline, pipe.layout);
        let d = self.device.clone();
        let v = &mut self.videos[id.0];
        let mapped =
            v.staging_alloc.as_mut().and_then(|a| a.mapped_slice_mut()).context("staging not mapped")?;
        let mut offsets = [0u64; 3];
        let mut at = 0usize;
        for (i, (p, src)) in v.planes.iter().zip(planes).enumerate() {
            let n = (p.width * p.height) as usize;
            anyhow::ensure!(
                src.len() >= n,
                "video plane {i}: {} bytes for {}x{}",
                src.len(),
                p.width,
                p.height
            );
            mapped[at..at + n].copy_from_slice(&src[..n]);
            offsets[i] = at as u64;
            at += n;
        }
        let first = !v.filled;
        v.filled = true;
        let images: Vec<(vk::Image, u32, u32)> =
            v.planes.iter().map(|p| (p.image, p.width, p.height)).collect();
        let (staging, set, display) = (v.staging, v.set, v.display);
        let t = &self.textures[v.texture.0];
        let (rgb_image, rgb_view) = (t.image, t.view);
        let range = full_range(vk::ImageAspectFlags::COLOR);
        let old = if first { vk::ImageLayout::UNDEFINED } else { vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL };
        self.one_shot(|cmd| {
            // SAFETY: every handle belongs to this device; earlier frames' reads are ordered by the barriers.
            unsafe {
                let to_dst: Vec<_> = images
                    .iter()
                    .map(|&(image, _, _)| {
                        vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                            .src_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                            .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                            .old_layout(old)
                            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                            .image(image)
                            .subresource_range(range)
                    })
                    .collect();
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_dst));
                for (i, &(image, w, h)) in images.iter().enumerate() {
                    let region = [vk::BufferImageCopy::default()
                        .buffer_offset(offsets[i])
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(vk::Extent3D { width: w, height: h, depth: 1 })];
                    d.cmd_copy_buffer_to_image(
                        cmd,
                        staging,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &region,
                    );
                }
                let mut barriers: Vec<_> = images
                    .iter()
                    .map(|&(image, _, _)| {
                        vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(vk::PipelineStageFlags2::COPY)
                            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                            .image(image)
                            .subresource_range(range)
                    })
                    .collect();
                barriers.push(
                    vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                        .src_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                        .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                        .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                        .old_layout(old)
                        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .image(rgb_image)
                        .subresource_range(range),
                );
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&barriers));
                let attach = [vk::RenderingAttachmentInfo::default()
                    .image_view(rgb_view)
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let extent = vk::Extent2D { width: display.0, height: display.1 };
                let area = vk::Rect2D { offset: vk::Offset2D::default(), extent };
                d.cmd_begin_rendering(
                    cmd,
                    &vk::RenderingInfo::default().render_area(area).layer_count(1).color_attachments(&attach),
                );
                let vp = vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: display.0 as f32,
                    height: display.1 as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                };
                d.cmd_set_viewport(cmd, 0, &[vp]);
                d.cmd_set_scissor(cmd, 0, &[area]);
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[set], &[]);
                // The visible part of the coded planes.
                let (u1, v1) = (display.0 as f32 / images[0].1 as f32, display.1 as f32 / images[0].2 as f32);
                let push = SpritePush {
                    pos01: [-1.0, -1.0, 1.0, -1.0],
                    pos23: [-1.0, 1.0, 1.0, 1.0],
                    uv01: [0.0, 0.0, u1, 0.0],
                    uv23: [0.0, v1, u1, v1],
                    color: [1.0; 4],
                };
                d.cmd_push_constants(cmd, layout, vk::ShaderStageFlags::VERTEX, 0, bytemuck::bytes_of(&push));
                d.cmd_draw(cmd, 4, 1, 0, 0);
                d.cmd_end_rendering(cmd);
                let to_read = [vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(rgb_image)
                    .subresource_range(range)];
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_read));
            }
        })
    }

    /// Destroy every movie target and the conversion pipeline (called on drop).
    pub(crate) fn destroy_videos(&mut self) {
        // SAFETY: called after `device_wait_idle`; each object is destroyed once.
        unsafe {
            for mut v in self.videos.drain(..) {
                for p in &mut v.planes {
                    self.device.destroy_image_view(p.view, None);
                    self.device.destroy_image(p.image, None);
                    if let Some(a) = p.alloc.take() {
                        let _ = self.allocator.free(a);
                    }
                }
                self.device.destroy_buffer(v.staging, None);
                if let Some(a) = v.staging_alloc.take() {
                    let _ = self.allocator.free(a);
                }
            }
            if let Some(p) = self.video_pipeline.take() {
                self.device.destroy_pipeline(p.pipeline, None);
                self.device.destroy_pipeline_layout(p.layout, None);
                self.device.destroy_descriptor_set_layout(p.set_layout, None);
            }
        }
    }
}
