//! Vulkan 1.3 `dynamicRendering` shader modules, graphics pipelines, and
//! offscreen render targets (`ADR-0004`, `ADR-0014`, `SPN-INV-034`, `F3D-FR-08`).

use std::ffi::CString;

use ash::vk;

use crate::error::{RenderError, RenderErrorKind};
use crate::shader::{
    CookedShader, PipelineLayoutDesc, ShaderStageFlags, VertexFormat,
    validate_graphics_pipeline_shaders,
};
use crate::types::{CullMode, FrontFace, GraphicsPipelineDescriptor, PipelineTarget, SurfaceSize};
use crate::vulkan::device::DeviceOwner;
use crate::vulkan::memory::{VulkanImage, VulkanMemoryAllocator};
use crate::vulkan::vk_error;

/// Validated cooked shader stored in a `Renderer`'s generational shader pool.
///
/// The underlying `VkShaderModule` is created lazily only AFTER both
/// [`CookedShader::from_bytes`] and [`validate_graphics_pipeline_shaders`]
/// succeed (`SPN-INV-034`).
pub(crate) struct LoadedShader {
    pub cooked: CookedShader,
    pub label: String,
    pub module: Option<vk::ShaderModule>,
}

impl LoadedShader {
    pub(crate) fn new(label: String, cooked: CookedShader) -> Self {
        Self {
            cooked,
            label,
            module: None,
        }
    }

    pub(crate) fn ensure_module(
        &mut self,
        device: &DeviceOwner,
    ) -> Result<vk::ShaderModule, RenderError> {
        if let Some(module) = self.module {
            return Ok(module);
        }
        let create_info = vk::ShaderModuleCreateInfo::default().code(&self.cooked.spirv_words);
        // SAFETY: `self.cooked.spirv_words` was validated as a 4-byte-aligned SPIR-V 1.6
        // stream with matching `OpEntryPoint` and layout BEFORE calling `vkCreateShaderModule`.
        let module = unsafe { device.device.create_shader_module(&create_info, None) }
            .map_err(|result| vk_error("vkCreateShaderModule", result))?;
        device.name_str(module, &format!("spawn.shader.{}", self.label));
        self.module = Some(module);
        Ok(module)
    }
}

/// Compiled Vulkan 1.3 `dynamicRendering` graphics pipeline (`VkPipelineRenderingCreateInfo`;
/// never uses `VkRenderPass` or `VkFramebuffer`).
pub(crate) struct CompiledPipeline {
    pub pipeline: vk::Pipeline,
    pub layout: vk::PipelineLayout,
    pub target: PipelineTarget,
    pub color_format: vk::Format,
}

impl CompiledPipeline {
    #[track_caller]
    pub(crate) fn create(
        device: &DeviceOwner,
        vs: &mut LoadedShader,
        fs: &mut LoadedShader,
        descriptor: &GraphicsPipelineDescriptor<'_>,
        color_format: vk::Format,
    ) -> Result<Self, RenderError> {
        // 1. Validate pipeline descriptor limits and invariants before touching Vulkan.
        if descriptor.vertex_inputs.is_empty() && descriptor.vertex_stride != 0 {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "pipeline `{}` declares 0 vertex inputs but non-zero vertex_stride {}",
                    descriptor.label, descriptor.vertex_stride
                )),
            );
        }
        if !descriptor.vertex_inputs.is_empty() {
            if descriptor.vertex_inputs.len()
                > usize::try_from(device.max_vertex_input_attributes).unwrap_or(0)
            {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(format!(
                        "pipeline `{}` declares {} vertex attributes (device max {})",
                        descriptor.label,
                        descriptor.vertex_inputs.len(),
                        device.max_vertex_input_attributes
                    )));
            }
            if descriptor.vertex_stride == 0
                || descriptor.vertex_stride > device.max_vertex_input_binding_stride
            {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(format!(
                        "pipeline `{}` vertex_stride {} outside 1..={}",
                        descriptor.label,
                        descriptor.vertex_stride,
                        device.max_vertex_input_binding_stride
                    )));
            }
        }
        if descriptor.push_constants.size > device.max_push_constants_size {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "pipeline `{}` push-constant size {} exceeds device maxPushConstantsSize ({})",
                    descriptor.label,
                    descriptor.push_constants.size,
                    device.max_push_constants_size
                )),
            );
        }
        if !descriptor.descriptor_bindings.is_empty() {
            return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail(format!(
                    "pipeline `{}` declares {} descriptor binding(s); descriptor-set layouts are not yet enabled in Phase 3",
                    descriptor.label,
                    descriptor.descriptor_bindings.len()
                )));
        }

        // 2. Validate cooked VS + FS reflection against pipeline layout BEFORE `vkCreateShaderModule` (`SPN-INV-034`).
        validate_graphics_pipeline_shaders(
            &vs.cooked,
            &fs.cooked,
            &PipelineLayoutDesc {
                vertex_inputs: descriptor.vertex_inputs,
                push_constants: descriptor.push_constants,
                descriptor_bindings: descriptor.descriptor_bindings,
            },
        )?;

        let vs_entry = CString::new(vs.cooked.entry_point.as_bytes()).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidShaderArtifact)
                .with_detail("vertex entry point contains interior NUL")
        })?;
        let fs_entry = CString::new(fs.cooked.entry_point.as_bytes()).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidShaderArtifact)
                .with_detail("fragment entry point contains interior NUL")
        })?;

        // 3. Build vertex input binding & attribute descriptions (if any).
        let mut sorted_attrs = descriptor.vertex_inputs.to_vec();
        sorted_attrs.sort_by_key(|a| a.location);
        let mut vk_attrs = Vec::with_capacity(sorted_attrs.len());
        let mut running_offset = 0_u32;
        for attr in &sorted_attrs {
            vk_attrs.push(
                vk::VertexInputAttributeDescription::default()
                    .binding(0)
                    .location(attr.location)
                    .format(vertex_format_to_vk(attr.format))
                    .offset(running_offset),
            );
            running_offset = running_offset
                .checked_add(attr.format.byte_size())
                .ok_or_else(|| {
                    RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                        .with_detail("vertex attribute byte offset overflow")
                })?;
        }
        if !sorted_attrs.is_empty() && running_offset > descriptor.vertex_stride {
            return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail(format!(
                    "pipeline `{}` vertex_stride {} is smaller than packed attribute size {running_offset}",
                    descriptor.label, descriptor.vertex_stride
                )));
        }
        let vk_bindings = if sorted_attrs.is_empty() {
            Vec::new()
        } else {
            vec![
                vk::VertexInputBindingDescription::default()
                    .binding(0)
                    .stride(descriptor.vertex_stride)
                    .input_rate(vk::VertexInputRate::VERTEX),
            ]
        };

        // 4. Create `VkShaderModule`s now that all binary + layout checks have succeeded.
        let vs_module = vs.ensure_module(device)?;
        let fs_module = fs.ensure_module(device)?;

        // 5. Create `VkPipelineLayout`.
        let push_ranges = if descriptor.push_constants.size == 0 {
            Vec::new()
        } else {
            vec![
                vk::PushConstantRange::default()
                    .stage_flags(stage_flags_to_vk(descriptor.push_constants.stages))
                    .offset(0)
                    .size(descriptor.push_constants.size),
            ]
        };
        let layout_info =
            vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_ranges);
        // SAFETY: `device.device` is valid and `push_ranges` is validated `<= max_push_constants_size`.
        let layout = unsafe { device.device.create_pipeline_layout(&layout_info, None) }
            .map_err(|result| vk_error("vkCreatePipelineLayout", result))?;
        device.name_str(
            layout,
            &format!("spawn.pipeline_layout.{}", descriptor.label),
        );

        // 6. Build Vulkan 1.3 `dynamicRendering` graphics pipeline (`VkPipelineRenderingCreateInfo`).
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs_module)
                .name(&vs_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs_module)
                .name(&fs_entry),
        ];
        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vk_bindings)
            .vertex_attribute_descriptions(&vk_attrs);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(cull_mode_to_vk(descriptor.cull_mode))
            .front_face(front_face_to_vk(descriptor.front_face))
            .depth_bias_enable(false)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1)
            .sample_shading_enable(false);
        let depth_stencil = if descriptor.depth_test_enable {
            vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(true)
                .depth_write_enable(true)
                .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
                .depth_bounds_test_enable(false)
                .stencil_test_enable(false)
        } else {
            vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(false)
                .depth_write_enable(false)
                .depth_compare_op(vk::CompareOp::ALWAYS)
                .depth_bounds_test_enable(false)
                .stencil_test_enable(false)
        };
        let blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let color_formats = [color_format];
        let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);

        let pipeline_info = [vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_info)];

        // SAFETY: all sub-structures live on this stack frame, `dynamicRendering` is enabled on
        // `device`, and shader modules + pipeline layout belong to `device`.
        let created = unsafe {
            device
                .device
                .create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_info, None)
        };
        let pipeline = match created {
            Ok(pipelines) => match pipelines.first().copied() {
                Some(p) if p != vk::Pipeline::null() => p,
                _ => {
                    // SAFETY: `layout` is unused on this error path.
                    unsafe { device.device.destroy_pipeline_layout(layout, None) };
                    return Err(RenderError::new(RenderErrorKind::VulkanCallFailed)
                        .with_detail("vkCreateGraphicsPipelines returned no pipeline"));
                }
            },
            Err((partial, result)) => {
                // SAFETY: clean up any partially created pipeline handles and `layout` on failure.
                unsafe {
                    for p in partial {
                        if p != vk::Pipeline::null() {
                            device.device.destroy_pipeline(p, None);
                        }
                    }
                    device.device.destroy_pipeline_layout(layout, None);
                }
                return Err(vk_error("vkCreateGraphicsPipelines", result));
            }
        };
        device.name_str(pipeline, &format!("spawn.pipeline.{}", descriptor.label));

        Ok(Self {
            pipeline,
            layout,
            target: descriptor.target,
            color_format,
        })
    }
}

/// Offscreen `R8G8B8A8_UNORM` (`COLOR_ATTACHMENT | TRANSFER_SRC`) + `D32_SFLOAT`
/// reversed-Z depth render target (`SPN-INV-092`, `F3D-T11`).
pub(crate) struct OffscreenTarget {
    pub size: SurfaceSize,
    pub color: VulkanImage,
    pub depth: VulkanImage,
}

impl OffscreenTarget {
    #[track_caller]
    pub(crate) fn create(
        device: &DeviceOwner,
        allocator: &mut VulkanMemoryAllocator,
        size: SurfaceSize,
    ) -> Result<Self, RenderError> {
        if size.is_empty() {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "offscreen target dimensions {}x{} must both be greater than 0",
                    size.width, size.height
                )),
            );
        }
        let _readback_bytes = u64::from(size.width)
            .checked_mul(u64::from(size.height))
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| {
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail("offscreen target RGBA8 byte count overflows u64")
            })?;

        let color = VulkanImage::create_2d(
            device,
            allocator,
            size,
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageAspectFlags::COLOR,
            "spawn.offscreen.color",
        )?;

        let depth = match VulkanImage::create_2d(
            device,
            allocator,
            size,
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::ImageAspectFlags::DEPTH,
            "spawn.offscreen.depth",
        ) {
            Ok(depth) => depth,
            Err(error) => {
                // Reverse-order unwind: release `color` before propagating `depth` failure.
                let _ = allocator.retire_image(device, color, 0, 0);
                return Err(error);
            }
        };

        Ok(Self { size, color, depth })
    }
}

fn front_face_to_vk(front_face: FrontFace) -> vk::FrontFace {
    match front_face {
        FrontFace::CounterClockwise => vk::FrontFace::COUNTER_CLOCKWISE,
        FrontFace::Clockwise => vk::FrontFace::CLOCKWISE,
    }
}

fn cull_mode_to_vk(cull_mode: CullMode) -> vk::CullModeFlags {
    match cull_mode {
        CullMode::Back => vk::CullModeFlags::BACK,
        CullMode::Front => vk::CullModeFlags::FRONT,
        CullMode::None => vk::CullModeFlags::NONE,
    }
}

fn stage_flags_to_vk(flags: ShaderStageFlags) -> vk::ShaderStageFlags {
    let mut out = vk::ShaderStageFlags::empty();
    if flags.contains(ShaderStageFlags::VERTEX) {
        out |= vk::ShaderStageFlags::VERTEX;
    }
    if flags.contains(ShaderStageFlags::FRAGMENT) {
        out |= vk::ShaderStageFlags::FRAGMENT;
    }
    if flags.contains(ShaderStageFlags::COMPUTE) {
        out |= vk::ShaderStageFlags::COMPUTE;
    }
    out
}

fn vertex_format_to_vk(format: VertexFormat) -> vk::Format {
    match format {
        VertexFormat::Float32 => vk::Format::R32_SFLOAT,
        VertexFormat::Float32x2 => vk::Format::R32G32_SFLOAT,
        VertexFormat::Float32x3 => vk::Format::R32G32B32_SFLOAT,
        VertexFormat::Float32x4 => vk::Format::R32G32B32A32_SFLOAT,
        VertexFormat::Sint32 => vk::Format::R32_SINT,
        VertexFormat::Sint32x2 => vk::Format::R32G32_SINT,
        VertexFormat::Sint32x3 => vk::Format::R32G32B32_SINT,
        VertexFormat::Sint32x4 => vk::Format::R32G32B32A32_SINT,
        VertexFormat::Uint32 => vk::Format::R32_UINT,
        VertexFormat::Uint32x2 => vk::Format::R32G32_UINT,
        VertexFormat::Uint32x3 => vk::Format::R32G32B32_UINT,
        VertexFormat::Uint32x4 => vk::Format::R32G32B32A32_UINT,
    }
}
