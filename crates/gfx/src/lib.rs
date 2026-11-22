//! Vulkan 1.3 renderer: device, swapchain (recreated on resize), frames in flight, sprite pipeline
//! and texture upload. Present mode is uncapped by default (MAILBOX > IMMEDIATE > FIFO).

use std::ffi::{CString, c_char};
use std::io::Cursor;
use std::mem::ManuallyDrop;

use aerugo_unity::{Texture2D, TextureFormat};
use anyhow::{Context, Result, bail};
use ash::{khr, vk};
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{
    Allocation, AllocationCreateDesc, AllocationScheme, Allocator, AllocatorCreateDesc,
};

mod text;
mod video;

pub use text::{DrawItem, TextDraw, TextGlobals, TextVertex};
pub use video::VideoId;

pub(crate) const FRAMES_IN_FLIGHT: usize = 2;

#[derive(Clone, Copy, Debug)]
pub struct TextureId(pub usize);

/// One textured quad. Corners are in Vulkan NDC (x right, y down, -1..1), in triangle-strip order:
/// `pos[0]`, `pos[1]`, `pos[2]` form the first triangle and `pos[1]`, `pos[2]`, `pos[3]` the second.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub tex: TextureId,
    pub pos: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    /// Multiplies the texture sample.
    pub color: [f32; 4],
}

impl Sprite {
    /// An axis-aligned quad: `dst` and `uv` are `x0, y0, x1, y1`.
    pub fn rect(tex: TextureId, dst: [f32; 4], uv: [f32; 4], color: [f32; 4]) -> Self {
        let [x0, y0, x1, y1] = dst;
        let [u0, v0, u1, v1] = uv;
        Self {
            tex,
            pos: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
            uv: [[u0, v0], [u1, v0], [u0, v1], [u1, v1]],
            color,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpritePush {
    pos01: [f32; 4],
    pos23: [f32; 4],
    uv01: [f32; 4],
    uv23: [f32; 4],
    color: [f32; 4],
}

struct GpuTexture {
    image: vk::Image,
    view: vk::ImageView,
    alloc: Option<Allocation>,
    set: vk::DescriptorSet,
    width: u32,
    height: u32,
}

struct Frame {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    acquired: vk::Semaphore,
    fence: vk::Fence,
}

struct Swapchain {
    handle: vk::SwapchainKHR,
    format: vk::Format,
    extent: vk::Extent2D,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    /// One per swapchain image: signalled when rendering to that image is done.
    rendered: Vec<vk::Semaphore>,
}

pub struct Gfx {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_fn: khr::surface::Instance,
    surface: vk::SurfaceKHR,
    pdev: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    swapchain_fn: khr::swapchain::Device,
    sc: Swapchain,
    frames: Vec<Frame>,
    frame: usize,
    allocator: ManuallyDrop<Allocator>,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    sampler: vk::Sampler,
    desc_pool: vk::DescriptorPool,
    textures: Vec<GpuTexture>,
    videos: Vec<video::VideoTarget>,
    video_pipeline: Option<video::VideoPipeline>,
    text: Option<text::TextPipeline>,
    min_ubo_align: u64,
    pub vsync: bool,
    pub present_mode: vk::PresentModeKHR,
    want_extent: (u32, u32),
    dirty: bool,
    pub device_name: String,
    capture_requested: bool,
    captured: Option<Capture>,
}

/// A rendered frame read back from the GPU.
#[derive(Debug, Clone)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    /// RGBA, rows top to bottom.
    pub rgba: Vec<u8>,
}

pub(crate) fn spirv(bytes: &[u8]) -> Result<Vec<u32>> {
    Ok(ash::util::read_spv(&mut Cursor::new(bytes))?)
}

fn full_range(aspect: vk::ImageAspectFlags) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default().aspect_mask(aspect).level_count(1).layer_count(1)
}

impl std::fmt::Debug for Gfx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gfx")
            .field("device", &self.device_name)
            .field("extent", &self.sc.extent)
            .field("present_mode", &self.present_mode)
            .field("textures", &self.textures.len())
            .finish()
    }
}

impl Gfx {
    pub fn new(window: &sdl3::video::Window, vsync: bool) -> Result<Self> {
        // SAFETY: loading the system Vulkan loader runs its initialization; nothing else uses it concurrently.
        let entry = unsafe { ash::Entry::load()? };
        let app = CString::new("aerugo")?;
        let app_info = vk::ApplicationInfo::default().application_name(&app).api_version(vk::API_VERSION_1_3);
        let ext_names: Vec<CString> = window
            .vulkan_instance_extensions()
            .map_err(|e| anyhow::anyhow!("SDL Vulkan extensions: {e}"))?
            .into_iter()
            .map(CString::new)
            .collect::<Result<_, _>>()?;
        let ext_ptrs: Vec<*const c_char> = ext_names.iter().map(|s| s.as_ptr()).collect();
        let validation = CString::new("VK_LAYER_KHRONOS_validation")?;
        let want_validation = std::env::var_os("AERUGO_VK_VALIDATION").is_some();
        // SAFETY: the entry is loaded and valid.
        let has_validation = unsafe { entry.enumerate_instance_layer_properties()? }
            .iter()
            .any(|l| l.layer_name_as_c_str().is_ok_and(|n| n == validation.as_c_str()));
        let layers: Vec<*const c_char> =
            if want_validation && has_validation { vec![validation.as_ptr()] } else { vec![] };
        // SAFETY: the create info and every string it points to outlive the call.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app_info)
                    .enabled_extension_names(&ext_ptrs)
                    .enabled_layer_names(&layers),
                None,
            )?
        };
        let surface_fn = khr::surface::Instance::new(&entry, &instance);
        // SAFETY: `instance` is a live instance created with the extensions SDL asked for.
        let surface = unsafe { window.vulkan_create_surface(instance.handle()) }
            .map_err(|e| anyhow::anyhow!("SDL_Vulkan_CreateSurface: {e}"))?;

        // Pick a Vulkan 1.3 device with a graphics+present queue; prefer discrete GPUs.
        let mut best: Option<(i32, vk::PhysicalDevice, u32, String)> = None;
        // SAFETY: `instance` is live.
        for pdev in unsafe { instance.enumerate_physical_devices()? } {
            // SAFETY: `pdev` was returned by this instance.
            let props = unsafe { instance.get_physical_device_properties(pdev) };
            if props.api_version < vk::API_VERSION_1_3 {
                continue;
            }
            // SAFETY: `pdev` was returned by this instance.
            let qfs = unsafe { instance.get_physical_device_queue_family_properties(pdev) };
            let Some(qf) = (0..qfs.len() as u32).find(|&i| {
                qfs[i as usize].queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    // SAFETY: `pdev`, the queue family index and `surface` all belong to this instance.
                    && unsafe { surface_fn.get_physical_device_surface_support(pdev, i, surface) }.unwrap_or(false)
            }) else {
                continue;
            };
            let score = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 3,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 1,
                _ => 0,
            };
            let name =
                props.device_name_as_c_str().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
            if best.as_ref().is_none_or(|b| score > b.0) {
                best = Some((score, pdev, qf, name));
            }
        }
        let (_, pdev, queue_family, device_name) =
            best.context("no Vulkan 1.3 device with presentation support")?;

        let prio = [1.0f32];
        let qci =
            [vk::DeviceQueueCreateInfo::default().queue_family_index(queue_family).queue_priorities(&prio)];
        let dev_exts = [khr::swapchain::NAME.as_ptr()];
        let mut f13 =
            vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true).synchronization2(true);
        let features = vk::PhysicalDeviceFeatures::default().texture_compression_bc(true);
        // SAFETY: `pdev` belongs to this instance; the create info and its chained structs outlive the call.
        let device = unsafe {
            instance.create_device(
                pdev,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&qci)
                    .enabled_extension_names(&dev_exts)
                    .enabled_features(&features)
                    .push_next(&mut f13),
                None,
            )?
        };
        // SAFETY: the device was created with one queue in this family.
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        let swapchain_fn = khr::swapchain::Device::new(&instance, &device);
        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.clone(),
            physical_device: pdev,
            debug_settings: Default::default(),
            buffer_device_address: false,
            allocation_sizes: Default::default(),
        })?;

        let mut frames = Vec::new();
        for _ in 0..FRAMES_IN_FLIGHT {
            // SAFETY: `device` is live; the created objects are destroyed in `Drop`.
            unsafe {
                let pool = device.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(queue_family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )?;
                let cmd = device.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default().command_pool(pool).command_buffer_count(1),
                )?[0];
                frames.push(Frame {
                    pool,
                    cmd,
                    acquired: device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?,
                    fence: device.create_fence(
                        &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                        None,
                    )?,
                });
            }
        }

        let (w, h) = window.size_in_pixels();
        // SAFETY: `pdev` was enumerated from this instance.
        let min_ubo_align = unsafe { instance.get_physical_device_properties(pdev) }
            .limits
            .min_uniform_buffer_offset_alignment;
        let mut gfx = Self {
            _entry: entry,
            instance,
            surface_fn,
            surface,
            pdev,
            device,
            queue,
            queue_family,
            swapchain_fn,
            sc: Swapchain {
                handle: vk::SwapchainKHR::null(),
                format: vk::Format::UNDEFINED,
                extent: vk::Extent2D::default(),
                images: vec![],
                views: vec![],
                rendered: vec![],
            },
            frames,
            frame: 0,
            allocator: ManuallyDrop::new(allocator),
            set_layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            sampler: vk::Sampler::null(),
            desc_pool: vk::DescriptorPool::null(),
            textures: vec![],
            videos: vec![],
            video_pipeline: None,
            text: None,
            min_ubo_align,
            vsync,
            present_mode: vk::PresentModeKHR::FIFO,
            want_extent: (w, h),
            dirty: false,
            device_name,
            capture_requested: false,
            captured: None,
        };
        gfx.create_swapchain()?;
        gfx.create_sprite_pipeline()?;
        gfx.create_text_pipeline()?;
        Ok(gfx)
    }

    fn create_swapchain(&mut self) -> Result<()> {
        // SAFETY: `pdev` and `surface` belong to the live instance.
        let caps =
            unsafe { self.surface_fn.get_physical_device_surface_capabilities(self.pdev, self.surface)? };
        // SAFETY: `pdev` and `surface` belong to the live instance.
        let formats =
            unsafe { self.surface_fn.get_physical_device_surface_formats(self.pdev, self.surface)? };
        // SAFETY: `pdev` and `surface` belong to the live instance.
        let modes =
            unsafe { self.surface_fn.get_physical_device_surface_present_modes(self.pdev, self.surface)? };
        // UNORM swapchain: texel values reach the screen unchanged (no implicit sRGB encode).
        let fmt = formats
            .iter()
            .find(|f| {
                matches!(f.format, vk::Format::B8G8R8A8_UNORM | vk::Format::R8G8B8A8_UNORM)
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .or(formats.first())
            .copied()
            .context("surface has no formats")?;
        let prefs: &[vk::PresentModeKHR] = if self.vsync {
            &[vk::PresentModeKHR::FIFO]
        } else {
            &[vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::FIFO]
        };
        self.present_mode = *prefs.iter().find(|m| modes.contains(m)).unwrap_or(&vk::PresentModeKHR::FIFO);
        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: self.want_extent.0.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: self.want_extent.1.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };
        let mut count = caps.min_image_count + 1;
        if caps.max_image_count > 0 {
            count = count.min(caps.max_image_count);
        }
        let old = self.sc.handle;
        // SAFETY: the surface is live and the old swapchain (if any) is retired by this call.
        let handle = unsafe {
            self.swapchain_fn.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(self.surface)
                    .min_image_count(count)
                    .image_format(fmt.format)
                    .image_color_space(fmt.color_space)
                    .image_extent(extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(self.present_mode)
                    .clipped(true)
                    .old_swapchain(old),
                None,
            )?
        };
        self.destroy_swapchain_resources();
        if old != vk::SwapchainKHR::null() {
            // SAFETY: the old swapchain was retired by the create call above and the device is idle.
            unsafe { self.swapchain_fn.destroy_swapchain(old, None) };
        }
        // SAFETY: `handle` was just created on this device.
        let images = unsafe { self.swapchain_fn.get_swapchain_images(handle)? };
        let mut views = Vec::new();
        let mut rendered = Vec::new();
        for &img in &images {
            // SAFETY: each image belongs to the new swapchain; views and semaphores are destroyed with it.
            unsafe {
                views.push(
                    self.device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(img)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(fmt.format)
                            .subresource_range(full_range(vk::ImageAspectFlags::COLOR)),
                        None,
                    )?,
                );
                rendered.push(self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?);
            }
        }
        self.sc = Swapchain { handle, format: fmt.format, extent, images, views, rendered };
        log::info!(
            "swapchain {}x{} {:?} {:?} ({} images)",
            extent.width,
            extent.height,
            fmt.format,
            self.present_mode,
            self.sc.images.len()
        );
        Ok(())
    }

    fn destroy_swapchain_resources(&mut self) {
        // SAFETY: callers wait for the device to be idle before the swapchain objects are destroyed.
        unsafe {
            for v in self.sc.views.drain(..) {
                self.device.destroy_image_view(v, None);
            }
            for s in self.sc.rendered.drain(..) {
                self.device.destroy_semaphore(s, None);
            }
        }
        self.sc.images.clear();
    }

    fn create_sprite_pipeline(&mut self) -> Result<()> {
        let d = &self.device;
        // SAFETY: `self.device` is live; created objects are destroyed in `Drop`.
        unsafe {
            let binding = [vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
            self.set_layout = d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&binding),
                None,
            )?;
            let push = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX)
                .size(size_of::<SpritePush>() as u32)];
            let layouts = [self.set_layout];
            self.pipeline_layout = d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts).push_constant_ranges(&push),
                None,
            )?;
            let vs_code = spirv(include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv")))?;
            let fs_code = spirv(include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv")))?;
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
                .blend_enable(true)
                .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::SRC_ALPHA)
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
                .layout(self.pipeline_layout)
                .push_next(&mut rendering);
            self.pipeline =
                d.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None).map_err(|(_, e)| e)?[0];
            d.destroy_shader_module(vs, None);
            d.destroy_shader_module(fs, None);
            self.sampler = d.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .max_lod(vk::LOD_CLAMP_NONE),
                None,
            )?;
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(4096)];
            self.desc_pool = d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(4096).pool_sizes(&sizes),
                None,
            )?;
        }
        Ok(())
    }

    /// Upload mip level 0 of a Unity texture. Block-compressed formats are uploaded as-is.
    pub fn upload_texture(&mut self, tex: &Texture2D) -> Result<TextureId> {
        let (format, pixels, swizzle) = match tex.format {
            TextureFormat::Dxt1 => (vk::Format::BC1_RGB_UNORM_BLOCK, tex.data.clone(), None),
            TextureFormat::Dxt5 => (vk::Format::BC3_UNORM_BLOCK, tex.data.clone(), None),
            TextureFormat::Bc7 => (vk::Format::BC7_UNORM_BLOCK, tex.data.clone(), None),
            TextureFormat::Rgba32 => (vk::Format::R8G8B8A8_UNORM, tex.data.clone(), None),
            TextureFormat::Rgb24 => {
                let n = (tex.width * tex.height) as usize;
                let mut out = Vec::with_capacity(n * 4);
                for px in tex.data[..n * 3].chunks_exact(3) {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
                (vk::Format::R8G8B8A8_UNORM, out, None)
            }
            TextureFormat::Alpha8 => (
                vk::Format::R8_UNORM,
                tex.data.clone(),
                Some(vk::ComponentMapping {
                    r: vk::ComponentSwizzle::ONE,
                    g: vk::ComponentSwizzle::ONE,
                    b: vk::ComponentSwizzle::ONE,
                    a: vk::ComponentSwizzle::R,
                }),
            ),
        };
        let level0 = match tex.format {
            TextureFormat::Rgb24 => (tex.width * tex.height * 4) as usize,
            f => f.level_size(tex.width, tex.height),
        };
        let bytes = &pixels[..level0];
        let d = self.device.clone();
        // SAFETY: every handle used belongs to this device; the upload waits for its own fence before the staging buffer is freed.
        unsafe {
            // Staging buffer.
            let buf = d.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes.len() as u64)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC),
                None,
            )?;
            let mut staging = self.allocator.allocate(&AllocationCreateDesc {
                name: "staging",
                requirements: d.get_buffer_memory_requirements(buf),
                location: MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })?;
            d.bind_buffer_memory(buf, staging.memory(), staging.offset())?;
            staging.mapped_slice_mut().context("staging not mapped")?[..bytes.len()].copy_from_slice(bytes);

            let extent = vk::Extent3D { width: tex.width, height: tex.height, depth: 1 };
            let image = d.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(extent)
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED),
                None,
            )?;
            let alloc = self.allocator.allocate(&AllocationCreateDesc {
                name: "texture",
                requirements: d.get_image_memory_requirements(image),
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })?;
            d.bind_image_memory(image, alloc.memory(), alloc.offset())?;

            self.one_shot(|cmd| {
                let range = full_range(vk::ImageAspectFlags::COLOR);
                let to_dst = [vk::ImageMemoryBarrier2::default()
                    .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .image(image)
                    .subresource_range(range)];
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_dst));
                let region = [vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(extent)];
                d.cmd_copy_buffer_to_image(cmd, buf, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &region);
                let to_read = [vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(image)
                    .subresource_range(range)];
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_read));
            })?;
            d.destroy_buffer(buf, None);
            self.allocator.free(staging)?;

            let mut view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(full_range(vk::ImageAspectFlags::COLOR));
            if let Some(s) = swizzle {
                view_info = view_info.components(s);
            }
            let view = d.create_image_view(&view_info, None)?;
            let layouts = [self.set_layout];
            let set = d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(&layouts),
            )?[0];
            let img_info = [vk::DescriptorImageInfo::default()
                .sampler(self.sampler)
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&img_info)];
            d.update_descriptor_sets(&write, &[]);
            self.textures.push(GpuTexture {
                image,
                view,
                alloc: Some(alloc),
                set,
                width: tex.width,
                height: tex.height,
            });
        }
        Ok(TextureId(self.textures.len() - 1))
    }

    pub fn texture_size(&self, id: TextureId) -> (u32, u32) {
        let t = &self.textures[id.0];
        (t.width, t.height)
    }

    fn one_shot(&self, record: impl FnOnce(vk::CommandBuffer)) -> Result<()> {
        let d = &self.device;
        // SAFETY: the command pool, buffer and fence are created and destroyed here, and the fence is waited on first.
        unsafe {
            let pool = d.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(self.queue_family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )?;
            let cmd = d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default().command_pool(pool).command_buffer_count(1),
            )?[0];
            d.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            record(cmd);
            d.end_command_buffer(cmd)?;
            let fence = d.create_fence(&vk::FenceCreateInfo::default(), None)?;
            let cbi = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            d.queue_submit2(self.queue, &[vk::SubmitInfo2::default().command_buffer_infos(&cbi)], fence)?;
            d.wait_for_fences(&[fence], true, u64::MAX)?;
            d.destroy_fence(fence, None);
            d.destroy_command_pool(pool, None);
        }
        Ok(())
    }

    /// Mark the swapchain for recreation (window resized / vsync toggled).
    pub fn resize(&mut self, w: u32, h: u32) {
        self.want_extent = (w, h);
        self.dirty = true;
    }

    pub fn extent(&self) -> (u32, u32) {
        (self.sc.extent.width, self.sc.extent.height)
    }

    /// Record and present one frame: `items` in order. Returns false if the frame was skipped (minimized /
    /// out of date).
    pub fn render(&mut self, clear: [f32; 4], items: &[DrawItem]) -> Result<bool> {
        if self.want_extent.0 == 0 || self.want_extent.1 == 0 {
            return Ok(false);
        }
        if self.dirty {
            // SAFETY: the device is live.
            unsafe { self.device.device_wait_idle()? };
            self.create_swapchain()?;
            self.dirty = false;
        }
        // SAFETY: the fence belongs to this device; once it signals, the frame's buffers are free to rewrite.
        unsafe { self.device.wait_for_fences(&[self.frames[self.frame].fence], true, u64::MAX)? };
        let text_ranges = self.prepare_text(items)?;
        let f = &self.frames[self.frame];
        let d = &self.device;
        // SAFETY: the frame's fence is waited on before its command buffer and semaphore are reused; every handle belongs to this device.
        let readback = unsafe {
            d.wait_for_fences(&[f.fence], true, u64::MAX)?;
            let image_index = match self.swapchain_fn.acquire_next_image(
                self.sc.handle,
                u64::MAX,
                f.acquired,
                vk::Fence::null(),
            ) {
                Ok((i, suboptimal)) => {
                    if suboptimal {
                        self.dirty = true;
                    }
                    i
                }
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.dirty = true;
                    return Ok(false);
                }
                Err(e) => bail!("acquire_next_image: {e}"),
            };
            d.reset_fences(&[f.fence])?;
            let readback = if self.capture_requested {
                self.capture_requested = false;
                Some(readback_buffer(d, &mut self.allocator, self.sc.extent)?)
            } else {
                None
            };
            let cmd = f.cmd;
            d.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            d.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            let img = self.sc.images[image_index as usize];
            let range = full_range(vk::ImageAspectFlags::COLOR);
            let to_color = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(img)
                .subresource_range(range)];
            d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_color));
            let attach = [vk::RenderingAttachmentInfo::default()
                .image_view(self.sc.views[image_index as usize])
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue { color: vk::ClearColorValue { float32: clear } })];
            let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: self.sc.extent };
            d.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default().render_area(area).layer_count(1).color_attachments(&attach),
            );
            let vp = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.sc.extent.width as f32,
                height: self.sc.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            d.cmd_set_viewport(cmd, 0, &[vp]);
            d.cmd_set_scissor(cmd, 0, &[area]);
            // Which pipeline is bound: false sprites, true text.
            let mut bound: Option<bool> = None;
            let mut text_ranges = text_ranges.into_iter();
            for item in items {
                let s = match item {
                    DrawItem::Sprite(s) => s,
                    DrawItem::Text(t) => {
                        let Some(r) = text_ranges.next() else { continue };
                        if r.count == 0 {
                            continue;
                        }
                        if bound != Some(true) {
                            self.bind_text(cmd);
                            bound = Some(true);
                        }
                        self.record_text(cmd, t, r);
                        continue;
                    }
                };
                if bound != Some(false) {
                    d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
                    bound = Some(false);
                }
                let t = &self.textures[s.tex.0];
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &[t.set],
                    &[],
                );
                let (p, u) = (s.pos, s.uv);
                let push = SpritePush {
                    pos01: [p[0][0], p[0][1], p[1][0], p[1][1]],
                    pos23: [p[2][0], p[2][1], p[3][0], p[3][1]],
                    uv01: [u[0][0], u[0][1], u[1][0], u[1][1]],
                    uv23: [u[2][0], u[2][1], u[3][0], u[3][1]],
                    color: s.color,
                };
                d.cmd_push_constants(
                    cmd,
                    self.pipeline_layout,
                    vk::ShaderStageFlags::VERTEX,
                    0,
                    bytemuck::bytes_of(&push),
                );
                d.cmd_draw(cmd, 4, 1, 0, 0);
            }
            d.cmd_end_rendering(cmd);
            let mut last_layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            if let Some((buf, _)) = &readback {
                let to_src = [vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                    .old_layout(last_layout)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .image(img)
                    .subresource_range(range)];
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_src));
                let region = vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: self.sc.extent.width,
                        height: self.sc.extent.height,
                        depth: 1,
                    });
                d.cmd_copy_image_to_buffer(cmd, img, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, *buf, &[region]);
                last_layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
            }
            let to_present = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(
                    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags2::COPY,
                )
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(last_layout)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(img)
                .subresource_range(range)];
            d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_present));
            d.end_command_buffer(cmd)?;

            let rendered = self.sc.rendered[image_index as usize];
            let wait = [vk::SemaphoreSubmitInfo::default()
                .semaphore(f.acquired)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
            let signal = [vk::SemaphoreSubmitInfo::default()
                .semaphore(rendered)
                .stage_mask(vk::PipelineStageFlags2::ALL_GRAPHICS)];
            let cbi = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            d.queue_submit2(
                self.queue,
                &[vk::SubmitInfo2::default()
                    .wait_semaphore_infos(&wait)
                    .command_buffer_infos(&cbi)
                    .signal_semaphore_infos(&signal)],
                f.fence,
            )?;
            let waits = [rendered];
            let scs = [self.sc.handle];
            let idx = [image_index];
            match self.swapchain_fn.queue_present(
                self.queue,
                &vk::PresentInfoKHR::default().wait_semaphores(&waits).swapchains(&scs).image_indices(&idx),
            ) {
                Ok(suboptimal) if suboptimal => self.dirty = true,
                Ok(_) => {}
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.dirty = true,
                Err(e) => bail!("queue_present: {e}"),
            }
            readback
        };
        if let Some((buf, alloc)) = readback {
            let fence = self.frames[self.frame].fence;
            // SAFETY: the copy is complete once the frame's fence signals; the buffer is destroyed after reading.
            unsafe {
                self.device.wait_for_fences(&[fence], true, u64::MAX)?;
                let (w, h) = (self.sc.extent.width, self.sc.extent.height);
                let src = alloc.mapped_slice().context("readback not mapped")?;
                let mut rgba = src[..(w * h * 4) as usize].to_vec();
                if matches!(self.sc.format, vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB) {
                    for px in rgba.chunks_exact_mut(4) {
                        px.swap(0, 2);
                    }
                }
                self.captured = Some(Capture { width: w, height: h, rgba });
                self.device.destroy_buffer(buf, None);
            }
            self.allocator.free(alloc)?;
        }
        self.frame = (self.frame + 1) % FRAMES_IN_FLIGHT;
        Ok(true)
    }

    /// Read back the next rendered frame; fetch it with [`take_capture`](Self::take_capture).
    pub fn capture_next_frame(&mut self) {
        self.capture_requested = true;
    }

    pub fn take_capture(&mut self) -> Option<Capture> {
        self.captured.take()
    }
}

/// A host-visible buffer for reading back a whole swapchain image.
fn readback_buffer(
    d: &ash::Device,
    allocator: &mut Allocator,
    extent: vk::Extent2D,
) -> Result<(vk::Buffer, Allocation)> {
    let size = u64::from(extent.width) * u64::from(extent.height) * 4;
    // SAFETY: the buffer is created and bound here and destroyed by the caller after use.
    unsafe {
        let buf = d.create_buffer(
            &vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_DST),
            None,
        )?;
        let alloc = allocator.allocate(&AllocationCreateDesc {
            name: "readback",
            requirements: d.get_buffer_memory_requirements(buf),
            location: MemoryLocation::GpuToCpu,
            linear: true,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })?;
        d.bind_buffer_memory(buf, alloc.memory(), alloc.offset())?;
        Ok((buf, alloc))
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        // SAFETY: `Drop` waits for the device to be idle, then destroys objects in reverse creation order.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.destroy_videos();
            self.destroy_text_pipeline();
            for t in self.textures.drain(..) {
                self.device.destroy_image_view(t.view, None);
                self.device.destroy_image(t.image, None);
                if let Some(a) = t.alloc {
                    let _ = self.allocator.free(a);
                }
            }
            self.device.destroy_descriptor_pool(self.desc_pool, None);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_descriptor_set_layout(self.set_layout, None);
            for f in self.frames.drain(..) {
                self.device.destroy_fence(f.fence, None);
                self.device.destroy_semaphore(f.acquired, None);
                self.device.destroy_command_pool(f.pool, None);
            }
            self.destroy_swapchain_resources();
            self.swapchain_fn.destroy_swapchain(self.sc.handle, None);
            ManuallyDrop::drop(&mut self.allocator);
            self.device.destroy_device(None);
            self.surface_fn.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}
