//! In-house RHI over Vulkan 1.3 (ADR-0004). Private to `spawn-render`.
//!
//! Ownership and destruction order (fields of [`VulkanRenderer`]):
//!
//! 1. user-created resources (offscreen targets, uploaded 3D meshes, graphics
//!    pipelines, shader modules), swapchain-matched `D32_SFLOAT` depth image,
//!    frames (command pools, acquire semaphores, timeline), swapchain (views,
//!    present semaphores), and the in-house GPU block allocator (`VkDeviceMemory`
//!    blocks): destroyed explicitly in [`VulkanRenderer::teardown`] / `Drop`
//!    after `vkDeviceWaitIdle`;
//! 2. the logical device (`DeviceOwner`);
//! 3. the presentation surface (`SurfaceOwner`);
//! 4. the instance and validation messenger (`InstanceOwner`).
//!
//! During construction the same order holds through local variables: a
//! failure at any step drops only what was already created, in reverse order.

mod device;
mod frame;
mod instance;
mod memory;
mod mesh;
mod pipeline;
mod surface;
mod swapchain;

use std::time::Duration;

use ash::vk;

use crate::allocator::{GpuAllocatorStats, GpuMemoryBudget, MemoryDomain};
use crate::depth::{DepthAttachmentController, DepthPlan};
use crate::error::{RenderError, RenderErrorKind};
use crate::handle::{
    GenerationalPool, MeshHandle, OffscreenTargetHandle, PipelineHandle, ShaderHandle,
};
use crate::mesh::{
    CameraRelativeView, MESH_PUSH_CONSTANT_BYTES, MESH_VERTEX_STRIDE, MeshDescriptor, MeshDraw,
};
use crate::presentation::{
    AcquireStatus, FramePlan, PresentStatus, PresentationController, StopReason,
};
use crate::shader::CookedShader;
use crate::types::{
    ClearColor, FrameOutcome, GraphicsPipelineDescriptor, OffscreenReadback, PipelineTarget,
    PresentModePreference, PresentationState, RendererDescriptor, RendererInfo, SurfaceSize,
    ValidationMessage,
};
use device::DeviceOwner;
use frame::{Frames, OffscreenPassTarget, PassDraw, SwapchainPassTarget};
use instance::InstanceOwner;
use memory::{VulkanBuffer, VulkanImage, VulkanMemoryAllocator};
use mesh::{PreparedMeshDraw, UploadedMesh, prepare_mesh_draws_into, prepare_mesh_staging_upload};
use pipeline::{CompiledPipeline, LoadedShader, OffscreenTarget};
use surface::SurfaceOwner;
pub(crate) use surface::SurfaceSource;
use swapchain::{Swapchain, SwapchainBuild, query_chosen_surface_format};

/// Longest a frame waits for a swapchain image. Short, so a hidden window
/// whose compositor holds every image keeps the caller's loop responsive; a
/// timeout skips the frame.
const ACQUIRE_TIMEOUT: Duration = Duration::from_millis(100);

/// Builds the error for a failed Vulkan call. `VK_ERROR_DEVICE_LOST` maps to
/// [`RenderErrorKind::DeviceLost`], everything else to
/// [`RenderErrorKind::VulkanCallFailed`].
#[track_caller]
pub(crate) fn vk_error(call: &'static str, result: vk::Result) -> RenderError {
    let kind = if result == vk::Result::ERROR_DEVICE_LOST {
        RenderErrorKind::DeviceLost
    } else {
        RenderErrorKind::VulkanCallFailed
    };
    RenderError::new(kind).with_detail(call).with_cause(&result)
}

/// A swapchain-creation failure: recoverable surface conditions or fatal.
#[derive(Debug)]
pub(crate) enum VkFailure {
    /// Out of date or surface lost: retry within bounds.
    Recoverable {
        surface_lost: bool,
    },
    Fatal(RenderError),
}

impl VkFailure {
    #[track_caller]
    pub(crate) fn from_call(call: &'static str, result: vk::Result) -> Self {
        match result {
            vk::Result::ERROR_OUT_OF_DATE_KHR => Self::Recoverable {
                surface_lost: false,
            },
            vk::Result::ERROR_SURFACE_LOST_KHR => Self::Recoverable { surface_lost: true },
            _ => Self::Fatal(vk_error(call, result)),
        }
    }
}

/// Draw workload passed internally before resolving `&self.prepared_draws`.
#[derive(Clone, Copy)]
enum PendingPassDraw {
    None,
    Unindexed {
        pipeline: vk::Pipeline,
        vertex_count: u32,
    },
    PreparedIndexedMeshes {
        pipeline: vk::Pipeline,
        layout: vk::PipelineLayout,
        push_stage_flags: vk::ShaderStageFlags,
    },
}

impl PendingPassDraw {
    fn into_pass_draw(self, prepared_draws: &[PreparedMeshDraw]) -> PassDraw<'_> {
        match self {
            Self::None => PassDraw::None,
            Self::Unindexed {
                pipeline,
                vertex_count,
            } => PassDraw::Unindexed {
                pipeline,
                vertex_count,
            },
            Self::PreparedIndexedMeshes {
                pipeline,
                layout,
                push_stage_flags,
            } => PassDraw::IndexedMeshes {
                pipeline,
                layout,
                push_stage_flags,
                draws: prepared_draws,
            },
        }
    }
}

/// The Vulkan renderer: one surface, one device, one queue, one swapchain,
/// one swapchain-matched `D32_SFLOAT` reversed-Z depth attachment, and the
/// in-house GPU block suballocator.
pub(crate) struct VulkanRenderer {
    controller: PresentationController,
    depth_controller: DepthAttachmentController,
    present_mode: PresentModePreference,
    source: SurfaceSource,
    surface_format: vk::Format,
    info: RendererInfo,
    /// Code of the error that stopped the renderer, for later calls.
    fatal_code: Option<&'static str>,
    torn_down: bool,
    /// Reusable scratch buffer for prepared indexed mesh draws (avoids per-frame allocation).
    prepared_draws: Vec<PreparedMeshDraw>,
    offscreen_targets: GenerationalPool<OffscreenTargetHandle, OffscreenTarget>,
    meshes: GenerationalPool<MeshHandle, UploadedMesh>,
    pipelines: GenerationalPool<PipelineHandle, CompiledPipeline>,
    shaders: GenerationalPool<ShaderHandle, LoadedShader>,
    swapchain_depth: Option<VulkanImage>,
    frames: Frames,
    swapchain: Option<Swapchain>,
    allocator: VulkanMemoryAllocator,
    device: DeviceOwner,
    surface: SurfaceOwner,
    instance: InstanceOwner,
}

impl VulkanRenderer {
    pub(crate) fn new(
        source: SurfaceSource,
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
        budget: GpuMemoryBudget,
    ) -> Result<Self, RenderError> {
        budget.validate()?;
        let extension = source.extension()?;
        let instance = InstanceOwner::new(extension, descriptor.validation)?;
        let surface = SurfaceOwner::new(&instance, &source)?;
        let (device, info) = DeviceOwner::new(&instance, &surface)?;
        let surface_format = query_chosen_surface_format(&device, &surface)?;
        let allocator = VulkanMemoryAllocator::new(budget, device.buffer_image_granularity)?;
        let frames = Frames::new(&device, device.queue_family)?;
        let mut renderer = Self {
            controller: PresentationController::new(size),
            depth_controller: DepthAttachmentController::new(),
            present_mode: descriptor.present_mode,
            source,
            surface_format,
            info,
            fatal_code: None,
            torn_down: false,
            prepared_draws: Vec::new(),
            offscreen_targets: GenerationalPool::new(),
            meshes: GenerationalPool::new(),
            pipelines: GenerationalPool::new(),
            shaders: GenerationalPool::new(),
            swapchain_depth: None,
            frames,
            swapchain: None,
            allocator,
            device,
            surface,
            instance,
        };
        // Create the first swapchain + D32_SFLOAT depth attachment now so
        // configuration or budget errors surface at startup; a zero size or zero
        // surface extent just suspends without allocating a 0x0 depth image.
        if renderer.controller.plan() == Some(FramePlan::RecreateSwapchain) {
            renderer.recreate_swapchain()?;
        }
        Ok(renderer)
    }

    pub(crate) fn info(&self) -> &RendererInfo {
        &self.info
    }

    pub(crate) fn state(&self) -> PresentationState {
        self.controller.state()
    }

    pub(crate) fn allocator_stats(&self) -> GpuAllocatorStats {
        self.allocator.stats()
    }

    pub(crate) fn resize(&mut self, size: SurfaceSize) {
        self.controller.resize(size);
    }

    pub(crate) fn drain_validation_messages(&self, out: &mut Vec<ValidationMessage>) -> u64 {
        self.instance.sink().drain_into(out)
    }

    pub(crate) fn validation_error_count(&self) -> u64 {
        self.instance.sink().error_count()
    }

    /// Waits until the GPU is idle and reclaims all timeline-retired GPU resources.
    pub(crate) fn wait_idle_and_reclaim(&mut self) -> Result<GpuAllocatorStats, RenderError> {
        self.ensure_not_stopped()?;
        self.device.wait_idle().map_err(|err| self.fatal(err))?;
        let completed = self
            .frames
            .completed_timeline(&self.device)
            .map_err(|err| self.fatal(err))?;
        self.allocator.reclaim_completed(&self.device, completed);
        Ok(self.allocator.stats())
    }

    /// Explicitly tears down all GPU resources and returns the post-shutdown
    /// allocator snapshot (`block_count == 0`, `live_allocations == 0`,
    /// `retired_allocations == 0`).
    pub(crate) fn close(mut self) -> Result<GpuAllocatorStats, RenderError> {
        self.device.wait_idle()?;
        self.teardown();
        Ok(self.allocator.stats())
    }

    /// Validates cooked `.spnshdr` bytes (`SPN-INV-034`) and registers a shader handle.
    #[track_caller]
    pub(crate) fn load_shader(
        &mut self,
        label: Option<&str>,
        cooked_bytes: &[u8],
    ) -> Result<ShaderHandle, RenderError> {
        self.ensure_not_stopped()?;
        let cooked = match label {
            Some(name) => CookedShader::from_bytes_named(name, cooked_bytes)?,
            None => CookedShader::from_bytes(cooked_bytes)?,
        };
        let debug_label = label
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("{}.{}", cooked.stage, cooked.entry_point));
        self.shaders.insert(LoadedShader::new(debug_label, cooked))
    }

    /// Destroys a loaded shader handle (`REND-0013` if stale or forged).
    #[track_caller]
    pub(crate) fn destroy_shader(&mut self, handle: ShaderHandle) -> Result<(), RenderError> {
        self.ensure_not_stopped()?;
        let loaded = self.shaders.remove(handle)?;
        if let Some(module) = loaded.module {
            let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
            self.allocator.retire_shader_module(
                &self.device,
                module,
                completed,
                self.frames.last_submitted(),
            );
        }
        Ok(())
    }

    /// Validates shader compatibility against `descriptor` (`SPN-INV-034`) and
    /// compiles a Vulkan 1.3 `dynamicRendering` graphics pipeline (`VkPipelineRenderingCreateInfo`).
    #[track_caller]
    pub(crate) fn create_graphics_pipeline(
        &mut self,
        descriptor: &GraphicsPipelineDescriptor<'_>,
    ) -> Result<PipelineHandle, RenderError> {
        self.ensure_not_stopped()?;
        if descriptor.vertex_shader == descriptor.fragment_shader {
            return Err(RenderError::new(RenderErrorKind::ShaderLayoutMismatch)
                .with_detail("vertex_shader and fragment_shader cannot reference the same single-stage ShaderHandle"));
        }

        // Validate both handles exist before mutably borrowing each slot.
        let _ = self.shaders.get(descriptor.vertex_shader)?;
        let _ = self.shaders.get(descriptor.fragment_shader)?;

        let color_format = match descriptor.target {
            PipelineTarget::Swapchain => self
                .swapchain
                .as_ref()
                .map_or(self.surface_format, |sc| sc.format),
            PipelineTarget::OffscreenRgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        };

        // Temporarily remove `vs` so we can borrow `fs` mutably too, then restore `vs`
        // into the exact same slot/generation! Instead of altering slot generation,
        // clone/split or ensure modules sequentially after pure validation!
        {
            let vs_ref = self.shaders.get(descriptor.vertex_shader)?;
            let fs_ref = self.shaders.get(descriptor.fragment_shader)?;
            crate::shader::validate_graphics_pipeline_shaders(
                &vs_ref.cooked,
                &fs_ref.cooked,
                &crate::shader::PipelineLayoutDesc {
                    vertex_inputs: descriptor.vertex_inputs,
                    push_constants: descriptor.push_constants,
                    descriptor_bindings: descriptor.descriptor_bindings,
                },
            )?;
        }

        // Pull out a temporary copy of `vs` while keeping its handle valid: even simpler,
        // `ensure_module` on `vs`, then `ensure_module` on `fs`, or pass two temp `LoadedShader`s!
        let mut vs_temp = {
            let slot = self.shaders.get_mut(descriptor.vertex_shader)?;
            LoadedShader {
                cooked: slot.cooked.clone(),
                label: slot.label.clone(),
                module: slot.module,
            }
        };
        let compiled_res = {
            let fs_slot = self.shaders.get_mut(descriptor.fragment_shader)?;
            CompiledPipeline::create(
                &self.device,
                &mut vs_temp,
                fs_slot,
                descriptor,
                color_format,
            )
        };
        if let Ok(vs_slot) = self.shaders.get_mut(descriptor.vertex_shader) {
            vs_slot.module = vs_temp.module;
        }
        let compiled = compiled_res.map_err(|err| {
            if err.kind() == RenderErrorKind::DeviceLost {
                self.fatal(err)
            } else {
                err
            }
        })?;
        match self.pipelines.try_insert(compiled) {
            Ok(handle) => Ok(handle),
            Err((compiled, error)) => {
                let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
                self.allocator.retire_pipeline(
                    &self.device,
                    compiled.pipeline,
                    compiled.layout,
                    completed,
                    self.frames.last_submitted(),
                );
                Err(error)
            }
        }
    }

    /// Destroys a graphics pipeline handle (`REND-0013` if stale or forged),
    /// retiring the underlying `VkPipeline` and `VkPipelineLayout` on the timeline semaphore.
    #[track_caller]
    pub(crate) fn destroy_pipeline(&mut self, handle: PipelineHandle) -> Result<(), RenderError> {
        self.ensure_not_stopped()?;
        let compiled = self.pipelines.remove(handle)?;
        let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
        self.allocator.retire_pipeline(
            &self.device,
            compiled.pipeline,
            compiled.layout,
            completed,
            self.frames.last_submitted(),
        );
        Ok(())
    }

    /// Allocates a dedicated offscreen `R8G8B8A8_UNORM` (`COLOR_ATTACHMENT | TRANSFER_SRC`)
    /// + `D32_SFLOAT` reversed-Z depth target for deterministic GPU rendering & readback.
    #[track_caller]
    pub(crate) fn create_offscreen_target(
        &mut self,
        size: SurfaceSize,
    ) -> Result<OffscreenTargetHandle, RenderError> {
        self.ensure_not_stopped()?;
        if let Ok(completed) = self.frames.completed_timeline(&self.device) {
            self.allocator.reclaim_completed(&self.device, completed);
        }
        let target =
            OffscreenTarget::create(&self.device, &mut self.allocator, size).map_err(|err| {
                if err.kind() == RenderErrorKind::DeviceLost {
                    self.fatal(err)
                } else {
                    err
                }
            })?;
        match self.offscreen_targets.try_insert(target) {
            Ok(handle) => Ok(handle),
            Err((target, error)) => {
                let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
                let last_sub = self.frames.last_submitted();
                let _ =
                    self.allocator
                        .retire_image(&self.device, target.depth, completed, last_sub);
                let _ =
                    self.allocator
                        .retire_image(&self.device, target.color, completed, last_sub);
                Err(error)
            }
        }
    }

    /// Destroys an offscreen render target handle (`REND-0013` if stale or forged),
    /// retiring its `DEVICE_LOCAL` color and depth suballocations on the timeline semaphore.
    #[track_caller]
    pub(crate) fn destroy_offscreen_target(
        &mut self,
        handle: OffscreenTargetHandle,
    ) -> Result<(), RenderError> {
        self.ensure_not_stopped()?;
        let target = self.offscreen_targets.remove(handle)?;
        let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
        let last_sub = self.frames.last_submitted();
        self.allocator
            .retire_image(&self.device, target.depth, completed, last_sub)?;
        self.allocator
            .retire_image(&self.device, target.color, completed, last_sub)?;
        Ok(())
    }

    /// Validates `descriptor` (`SPN-INV-034`), copies vertices and `u32` indices
    /// through a suballocated `HOST_VISIBLE | HOST_COHERENT` staging buffer into
    /// `DEVICE_LOCAL` vertex and index buffers via `vkCmdCopyBuffer2` +
    /// `synchronization2`, queues the staging buffer on the timeline retirement
    /// list, and returns a generational [`MeshHandle`].
    #[track_caller]
    pub(crate) fn upload_mesh(
        &mut self,
        descriptor: &MeshDescriptor<'_>,
    ) -> Result<MeshHandle, RenderError> {
        self.ensure_not_stopped()?;
        if let Ok(completed) = self.frames.completed_timeline(&self.device) {
            self.allocator.reclaim_completed(&self.device, completed);
        }
        let staged = prepare_mesh_staging_upload(&self.device, &mut self.allocator, descriptor)
            .map_err(|err| {
                if err.kind() == RenderErrorKind::DeviceLost {
                    self.fatal(err)
                } else {
                    err
                }
            })?;

        let submit_res = self.frames.execute_mesh_staging_upload(
            &self.device,
            staged.staging_buffer.handle,
            staged.mesh.vertex_buffer.handle,
            staged.vertex_bytes,
            staged.mesh.index_buffer.handle,
            staged.index_bytes,
        );
        let upload_timeline = match submit_res {
            Ok(val) => val,
            Err(error) => {
                let _ = self.device.wait_idle();
                let _ = self.allocator.retire_buffer(
                    &self.device,
                    staged.mesh.index_buffer,
                    u64::MAX,
                    0,
                );
                let _ = self.allocator.retire_buffer(
                    &self.device,
                    staged.mesh.vertex_buffer,
                    u64::MAX,
                    0,
                );
                let _ =
                    self.allocator
                        .retire_buffer(&self.device, staged.staging_buffer, u64::MAX, 0);
                return Err(self.fatal(error));
            }
        };

        let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
        let _ = self.allocator.retire_buffer(
            &self.device,
            staged.staging_buffer,
            completed,
            upload_timeline,
        );

        match self.meshes.try_insert(staged.mesh) {
            Ok(handle) => Ok(handle),
            Err((mesh, error)) => {
                // Roll back both DEVICE_LOCAL buffers on the same upload timeline
                // so no GPU memory or VkBuffer leaks even if pool insertion fails.
                let _ = self.allocator.retire_buffer(
                    &self.device,
                    mesh.index_buffer,
                    completed,
                    upload_timeline,
                );
                let _ = self.allocator.retire_buffer(
                    &self.device,
                    mesh.vertex_buffer,
                    completed,
                    upload_timeline,
                );
                Err(error)
            }
        }
    }

    /// Destroys a resident 3D mesh handle (`REND-0013` if stale or forged),
    /// retiring its `DEVICE_LOCAL` vertex and index suballocations on the timeline
    /// semaphore.
    #[track_caller]
    pub(crate) fn destroy_mesh(&mut self, handle: MeshHandle) -> Result<(), RenderError> {
        self.ensure_not_stopped()?;
        let mesh = self.meshes.remove(handle)?;
        let completed = self.frames.completed_timeline(&self.device).unwrap_or(0);
        let last_sub = self.frames.last_submitted();
        self.allocator
            .retire_buffer(&self.device, mesh.index_buffer, completed, last_sub)?;
        self.allocator
            .retire_buffer(&self.device, mesh.vertex_buffer, completed, last_sub)?;
        Ok(())
    }

    /// Clears the next swapchain image to `color` (and its `D32_SFLOAT` depth to `0.0`)
    /// and presents it.
    pub(crate) fn render_clear(&mut self, color: ClearColor) -> Result<FrameOutcome, RenderError> {
        self.render_swapchain_internal(color, PendingPassDraw::None, None)
    }

    /// Clears the next swapchain image to `clear_color` and `D32_SFLOAT` depth to `0.0`,
    /// draws `vertex_count` unindexed vertices with `pipeline`, and presents the frame.
    #[track_caller]
    pub(crate) fn render_triangle(
        &mut self,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        vertex_count: u32,
    ) -> Result<FrameOutcome, RenderError> {
        self.ensure_not_stopped()?;
        let (vk_pipeline, expected_format) = {
            let compiled = self.pipelines.get(pipeline)?;
            if compiled.target != PipelineTarget::Swapchain {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(
                        "render_triangle requires a pipeline created with `PipelineTarget::Swapchain`",
                    ));
            }
            if compiled.vertex_stride != 0 || compiled.push_constants.size != 0 {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(
                        "render_triangle requires an unindexed pipeline with 0 vertex inputs and 0 push constants",
                    ));
            }
            (compiled.pipeline, compiled.color_format)
        };
        self.render_swapchain_internal(
            clear_color,
            PendingPassDraw::Unindexed {
                pipeline: vk_pipeline,
                vertex_count,
            },
            Some(expected_format),
        )
    }

    /// Clears the next swapchain image to `clear_color` and `D32_SFLOAT` depth to `0.0`,
    /// draws `draws` as camera-relative indexed 3D meshes (`SPN-INV-052`) with `pipeline`,
    /// and presents the frame.
    #[track_caller]
    pub(crate) fn render_meshes(
        &mut self,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        view: &CameraRelativeView,
        draws: &[MeshDraw],
    ) -> Result<FrameOutcome, RenderError> {
        self.ensure_not_stopped()?;
        let (vk_pipeline, vk_layout, push_stage_flags, expected_format) =
            self.validate_mesh_pipeline(pipeline, PipelineTarget::Swapchain, "render_meshes")?;
        prepare_mesh_draws_into(&self.meshes, &mut self.prepared_draws, view, draws)?;
        self.render_swapchain_internal(
            clear_color,
            PendingPassDraw::PreparedIndexedMeshes {
                pipeline: vk_pipeline,
                layout: vk_layout,
                push_stage_flags,
            },
            Some(expected_format),
        )
    }

    /// Renders `vertex_count` vertices with `pipeline` into `target` (`R8G8B8A8_UNORM` +
    /// `D32_SFLOAT` reversed-Z clear `0.0`) and reads back the resulting linear RGBA8 pixels
    /// through a suballocated `HOST_VISIBLE | HOST_COHERENT` buffer (`vkCmdCopyImageToBuffer2`).
    #[track_caller]
    pub(crate) fn render_offscreen_triangle(
        &mut self,
        target: OffscreenTargetHandle,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        vertex_count: u32,
    ) -> Result<OffscreenReadback, RenderError> {
        self.ensure_not_stopped()?;
        let vk_pipeline = {
            let compiled = self.pipelines.get(pipeline)?;
            if compiled.target != PipelineTarget::OffscreenRgba8Unorm {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail("render_offscreen_triangle requires a pipeline created with `PipelineTarget::OffscreenRgba8Unorm`"));
            }
            if compiled.vertex_stride != 0 || compiled.push_constants.size != 0 {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail("render_offscreen_triangle requires an unindexed pipeline with 0 vertex inputs and 0 push constants"));
            }
            compiled.pipeline
        };
        self.execute_offscreen_pass(
            target,
            clear_color,
            PendingPassDraw::Unindexed {
                pipeline: vk_pipeline,
                vertex_count,
            },
        )
    }

    /// Clears `target` (`R8G8B8A8_UNORM` + `D32_SFLOAT` reversed-Z clear `0.0`),
    /// draws `draws` as camera-relative indexed 3D meshes (`SPN-INV-052`) with `pipeline`,
    /// and reads back the linear RGBA8 image through a suballocated
    /// `HOST_VISIBLE | HOST_COHERENT` buffer (`vkCmdCopyImageToBuffer2`).
    #[track_caller]
    pub(crate) fn render_offscreen_meshes(
        &mut self,
        target: OffscreenTargetHandle,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        view: &CameraRelativeView,
        draws: &[MeshDraw],
    ) -> Result<OffscreenReadback, RenderError> {
        self.ensure_not_stopped()?;
        let (vk_pipeline, vk_layout, push_stage_flags, _) = self.validate_mesh_pipeline(
            pipeline,
            PipelineTarget::OffscreenRgba8Unorm,
            "render_offscreen_meshes",
        )?;
        prepare_mesh_draws_into(&self.meshes, &mut self.prepared_draws, view, draws)?;
        self.execute_offscreen_pass(
            target,
            clear_color,
            PendingPassDraw::PreparedIndexedMeshes {
                pipeline: vk_pipeline,
                layout: vk_layout,
                push_stage_flags,
            },
        )
    }

    #[track_caller]
    fn validate_mesh_pipeline(
        &self,
        pipeline: PipelineHandle,
        expected_target: PipelineTarget,
        caller: &'static str,
    ) -> Result<
        (
            vk::Pipeline,
            vk::PipelineLayout,
            vk::ShaderStageFlags,
            vk::Format,
        ),
        RenderError,
    > {
        let compiled = self.pipelines.get(pipeline)?;
        if compiled.target != expected_target {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "{caller} requires a pipeline created with `PipelineTarget::{expected_target:?}`"
                )),
            );
        }
        if compiled.vertex_stride != MESH_VERTEX_STRIDE
            || compiled.push_constants.size != MESH_PUSH_CONSTANT_BYTES
        {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "{caller} requires a 3D mesh pipeline (`vertex_stride == {MESH_VERTEX_STRIDE}`, \
                     `push_constants.size == {MESH_PUSH_CONSTANT_BYTES}`), got stride {} and push_constants {}",
                    compiled.vertex_stride, compiled.push_constants.size
                )),
            );
        }
        Ok((
            compiled.pipeline,
            compiled.layout,
            compiled.push_stage_flags,
            compiled.color_format,
        ))
    }

    #[track_caller]
    fn execute_offscreen_pass(
        &mut self,
        target: OffscreenTargetHandle,
        clear_color: ClearColor,
        draw: PendingPassDraw,
    ) -> Result<OffscreenReadback, RenderError> {
        let (extent, color_image, color_view, depth_image, depth_view) = {
            let offscreen = self.offscreen_targets.get(target)?;
            (
                offscreen.size,
                offscreen.color.image,
                offscreen.color.view,
                offscreen.depth.image,
                offscreen.depth.view,
            )
        };

        if let Ok(completed) = self.frames.completed_timeline(&self.device) {
            self.allocator.reclaim_completed(&self.device, completed);
        }

        let byte_len_u64 = u64::from(extent.width)
            .checked_mul(u64::from(extent.height))
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| {
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail("offscreen readback byte length overflow")
            })?;
        let byte_len_usize = usize::try_from(byte_len_u64).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("offscreen readback byte length overflows usize")
        })?;

        let readback_buffer = VulkanBuffer::create(
            &self.device,
            &mut self.allocator,
            byte_len_u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryDomain::HostVisible,
            "spawn.readback.buffer",
        )?;

        let pass_target = OffscreenPassTarget {
            color_image,
            color_view,
            depth_image,
            depth_view,
            extent,
            clear_color: clear_color.sanitized(),
            readback_buffer: readback_buffer.handle,
            readback_bytes: byte_len_u64,
        };

        let pass_draw = draw.into_pass_draw(&self.prepared_draws);
        let submit_res =
            self.frames
                .execute_offscreen_draw_and_copy(&self.device, &pass_target, pass_draw);

        let signaled_value = match submit_res {
            Ok(val) => val,
            Err(error) => {
                // Ensure the GPU is idle before freeing `readback_buffer` on error unwind.
                let _ = self.device.wait_idle();
                let _ = self
                    .allocator
                    .retire_buffer(&self.device, readback_buffer, u64::MAX, 0);
                return Err(self.fatal(error));
            }
        };

        let rgba_res = self
            .allocator
            .read_host_bytes(&readback_buffer.suballocation, byte_len_usize);

        // Retire `readback_buffer` at `signaled_value` and immediately reclaim through
        // `signaled_value` (already waited by `execute_offscreen_draw_and_copy`)!
        let _ = self.allocator.retire_buffer(
            &self.device,
            readback_buffer,
            0, // queue through retirement list first so retirement + reclaim path runs!
            signaled_value,
        );
        self.allocator
            .reclaim_completed(&self.device, signaled_value);

        let rgba8 = rgba_res?;
        Ok(OffscreenReadback::new(extent, rgba8))
    }

    fn render_swapchain_internal(
        &mut self,
        color: ClearColor,
        draw: PendingPassDraw,
        expected_format: Option<vk::Format>,
    ) -> Result<FrameOutcome, RenderError> {
        let Some(plan) = self.controller.plan() else {
            return Err(self.stopped_error());
        };
        match plan {
            FramePlan::Suspend => return Ok(FrameOutcome::Suspended),
            FramePlan::RecreateSurface => {
                if !self.recreate_surface()? || !self.recreate_swapchain()? {
                    return Ok(self.idle_outcome());
                }
            }
            FramePlan::RecreateSwapchain => {
                if !self.recreate_swapchain()? {
                    return Ok(self.idle_outcome());
                }
            }
            FramePlan::Acquire => {}
        }
        self.draw_swapchain(color, draw, expected_format)
    }

    fn draw_swapchain(
        &mut self,
        color: ClearColor,
        draw: PendingPassDraw,
        expected_format: Option<vk::Format>,
    ) -> Result<FrameOutcome, RenderError> {
        let ticket = self
            .frames
            .begin(&self.device)
            .map_err(|error| self.fatal(error))?;
        if let Ok(completed) = self.frames.completed_timeline(&self.device) {
            self.allocator.reclaim_completed(&self.device, completed);
        }

        let Some((handle, extent, sc_format, encode_srgb)) = self
            .swapchain
            .as_ref()
            .map(|sc| (sc.handle, sc.extent, sc.format, sc.encode_srgb))
        else {
            return Ok(FrameOutcome::Skipped);
        };
        let Some((depth_image, depth_view)) =
            self.swapchain_depth.as_ref().map(|d| (d.image, d.view))
        else {
            return Ok(FrameOutcome::Skipped);
        };

        if let Some(expected_format) = expected_format
            && expected_format != sc_format
        {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "pipeline color format ({}) does not match active swapchain format ({})",
                    expected_format.as_raw(),
                    sc_format.as_raw()
                )),
            );
        }

        let timeout = u64::try_from(ACQUIRE_TIMEOUT.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the swapchain and semaphore belong to this device; the
        // acquire semaphore is unsignaled because its slot's last submission
        // (which waited on it) completed in `begin`, and failed or timed-out
        // acquires do not signal it.
        let acquired = unsafe {
            self.device.swapchain.acquire_next_image(
                handle,
                timeout,
                ticket.acquire,
                vk::Fence::null(),
            )
        };
        let (image_index, status) = match acquired {
            Ok((index, false)) => (index, AcquireStatus::Success),
            Ok((index, true)) => (index, AcquireStatus::Suboptimal),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => (0, AcquireStatus::OutOfDate),
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => (0, AcquireStatus::SurfaceLost),
            Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => (0, AcquireStatus::NotReady),
            Err(result) => return Err(self.fatal(vk_error("vkAcquireNextImageKHR", result))),
        };
        if !self.controller.acquired(status) {
            return self.after_skip();
        }

        let index = usize::try_from(image_index).unwrap_or(usize::MAX);
        let targets = self.swapchain.as_ref().and_then(|swapchain| {
            Some((
                *swapchain.images.get(index)?,
                *swapchain.views.get(index)?,
                *swapchain.present_semaphores.get(index)?,
            ))
        });
        let Some((color_image, color_view, present)) = targets else {
            return Err(self.fatal(
                RenderError::new(RenderErrorKind::VulkanCallFailed).with_detail(format!(
                    "vkAcquireNextImageKHR returned image index {image_index} out of range"
                )),
            ));
        };
        let clear_color = if encode_srgb {
            color.srgb_encoded()
        } else {
            color.sanitized()
        };

        let pass_target = SwapchainPassTarget {
            color_image,
            color_view,
            depth_image,
            depth_view,
            extent,
            clear_color,
        };
        let pass_draw = draw.into_pass_draw(&self.prepared_draws);
        let recorded = self
            .frames
            .record_swapchain_pass(&self.device, &ticket, &pass_target, pass_draw)
            .and_then(|()| self.frames.submit(&self.device, ticket, present));
        recorded.map_err(|error| self.fatal(error))?;

        let waits = [present];
        let swapchains = [handle];
        let indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&waits)
            .swapchains(&swapchains)
            .image_indices(&indices);
        // SAFETY: the image was acquired from this swapchain and the submission
        // that signals `present` was queued on the same queue.
        let presented = unsafe {
            self.device
                .swapchain
                .queue_present(self.device.queue, &present_info)
        };
        let status = match presented {
            Ok(false) => PresentStatus::Success,
            Ok(true) => PresentStatus::Suboptimal,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => PresentStatus::OutOfDate,
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => PresentStatus::SurfaceLost,
            Err(result) => return Err(self.fatal(vk_error("vkQueuePresentKHR", result))),
        };
        self.controller.presented(status);
        match status {
            PresentStatus::Success | PresentStatus::Suboptimal => Ok(FrameOutcome::Presented),
            PresentStatus::OutOfDate | PresentStatus::SurfaceLost => self.after_skip(),
        }
    }

    /// Outcome for a frame that showed nothing; an error if that exhausted
    /// recovery.
    fn after_skip(&mut self) -> Result<FrameOutcome, RenderError> {
        if self.controller.plan().is_none() {
            return Err(self.stopped_error());
        }
        Ok(FrameOutcome::Skipped)
    }

    /// Outcome when recreation left nothing to draw to.
    fn idle_outcome(&self) -> FrameOutcome {
        if self.controller.state() == PresentationState::Suspended {
            FrameOutcome::Suspended
        } else {
            FrameOutcome::Skipped
        }
    }

    /// Retires any live swapchain depth attachment immediately (valid only after
    /// `device.wait_idle()`).
    fn retire_swapchain_depth_idle(&mut self) {
        if let Some(depth) = self.swapchain_depth.take() {
            let _ = self
                .allocator
                .retire_image(&self.device, depth, u64::MAX, 0);
            self.depth_controller.commit_retired();
        }
    }

    /// Synchronizes `self.swapchain_depth` to `actual_extent` after `device.wait_idle()`.
    fn sync_swapchain_depth_idle(
        &mut self,
        actual_extent: Option<SurfaceSize>,
    ) -> Result<(), RenderError> {
        match self.depth_controller.plan_for_swapchain(actual_extent) {
            DepthPlan::Keep => Ok(()),
            DepthPlan::Retire { .. } => {
                self.retire_swapchain_depth_idle();
                Ok(())
            }
            DepthPlan::Allocate { extent }
            | DepthPlan::Recreate {
                new_extent: extent, ..
            } => {
                // Free the previous depth image BEFORE allocating the replacement so both
                // resizes share the same pooled 16 MiB `DEVICE_LOCAL` block!
                self.retire_swapchain_depth_idle();
                let depth = VulkanImage::create_2d(
                    &self.device,
                    &mut self.allocator,
                    extent,
                    vk::Format::D32_SFLOAT,
                    vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
                    vk::ImageAspectFlags::DEPTH,
                    "spawn.swapchain.depth",
                )?;
                self.depth_controller.commit_allocated(extent);
                self.swapchain_depth = Some(depth);
                Ok(())
            }
        }
    }

    /// Recreates the swapchain and its matching `D32_SFLOAT` depth attachment for
    /// the requested size. Returns whether a drawable swapchain now exists.
    fn recreate_swapchain(&mut self) -> Result<bool, RenderError> {
        // The old swapchain's images and depth attachment may still be used by submitted frames.
        self.device.wait_idle().map_err(|error| self.fatal(error))?;
        let completed = self
            .frames
            .completed_timeline(&self.device)
            .unwrap_or(self.frames.last_submitted());
        self.allocator.reclaim_completed(&self.device, completed);

        let old = self.swapchain.take();
        let built = Swapchain::create(
            &self.device,
            &self.surface,
            self.controller.requested(),
            self.present_mode,
            old.as_ref()
                .map_or(vk::SwapchainKHR::null(), |old| old.handle),
        );
        // The old swapchain is retired by the create call even when it fails.
        if let Some(old) = old {
            old.destroy(&self.device);
        }
        match built {
            Ok(SwapchainBuild::Created(swapchain)) => {
                self.surface_format = swapchain.format;
                if let Err(error) = self.sync_swapchain_depth_idle(Some(swapchain.extent)) {
                    swapchain.destroy(&self.device);
                    return Err(self.fatal(error));
                }
                self.controller.swapchain_recreated(swapchain.extent);
                self.swapchain = Some(swapchain);
                Ok(true)
            }
            Ok(SwapchainBuild::ZeroExtent) => {
                let _ = self.sync_swapchain_depth_idle(None);
                self.controller.swapchain_recreated(SurfaceSize::default());
                Ok(false)
            }
            Err(VkFailure::Recoverable { surface_lost }) => {
                let _ = self.sync_swapchain_depth_idle(None);
                if surface_lost {
                    // Schedules surface recreation before the next attempt.
                    self.controller.acquired(AcquireStatus::SurfaceLost);
                }
                if self.controller.recovery_failed() {
                    Ok(false)
                } else {
                    Err(self.stopped_error())
                }
            }
            Err(VkFailure::Fatal(error)) => {
                let _ = self.sync_swapchain_depth_idle(None);
                Err(self.fatal(error))
            }
        }
    }

    /// Replaces a lost surface with a new one for the same window. Returns
    /// whether it succeeded; a failure is retried at the next frame within
    /// the recovery bound.
    fn recreate_surface(&mut self) -> Result<bool, RenderError> {
        self.device.wait_idle().map_err(|error| self.fatal(error))?;
        self.retire_swapchain_depth_idle();
        if let Some(old) = self.swapchain.take() {
            old.destroy(&self.device);
        }
        self.surface.destroy();
        let Ok(surface) = SurfaceOwner::new(&self.instance, &self.source) else {
            // The surface stays null; the next frame retries.
            return if self.controller.recovery_failed() {
                Ok(false)
            } else {
                Err(self.stopped_error())
            };
        };
        // SAFETY: the physical device and new surface belong to this instance.
        let supported = unsafe {
            surface.loader.get_physical_device_surface_support(
                self.device.physical,
                self.device.queue_family,
                surface.handle,
            )
        }
        .unwrap_or(false);
        self.surface = surface;
        if !supported {
            return Err(self.fatal(
                RenderError::new(RenderErrorKind::SurfaceRecoveryFailed).with_detail(
                    "the recreated surface is not supported by the selected queue family",
                ),
            ));
        }
        self.controller.surface_recreated();
        Ok(true)
    }

    #[track_caller]
    fn ensure_not_stopped(&mut self) -> Result<(), RenderError> {
        if self.controller.state() == PresentationState::Fatal || self.torn_down {
            return Err(self.stopped_error());
        }
        Ok(())
    }

    /// Stops the renderer because of `error` and returns it.
    fn fatal(&mut self, error: RenderError) -> RenderError {
        self.controller.fail();
        if self.fatal_code.is_none() {
            self.fatal_code = Some(error.code());
        }
        error
    }

    /// The error for a call on a stopped renderer, or for the call that
    /// exhausted recovery.
    #[track_caller]
    fn stopped_error(&mut self) -> RenderError {
        match (self.fatal_code, self.controller.stop_reason()) {
            (None, Some(StopReason::RecoveryExhausted)) => {
                let error =
                    RenderError::new(RenderErrorKind::SurfaceRecoveryFailed).with_detail(format!(
                        "recovery bound exhausted: {} consecutive failed recreations or {} recoveries \
                         without a presented frame",
                        crate::presentation::MAX_RECOVERY_FAILURES,
                        crate::presentation::MAX_UNPRESENTED_RECOVERIES
                    ));
                self.fatal_code = Some(error.code());
                error
            }
            (code, _) => RenderError::new(RenderErrorKind::RendererFailed)
                .with_detail(format!("stopped by error[{}]", code.unwrap_or("closed"))),
        }
    }

    /// Idempotent reverse-order GPU resource teardown.
    fn teardown(&mut self) {
        if self.torn_down {
            return;
        }
        self.torn_down = true;
        // Nothing may be destroyed while the GPU still uses it. If the device
        // is lost, waiting fails but destruction is still valid.
        let _ = self.device.wait_idle();

        for target in self.offscreen_targets.drain_all() {
            let _ = self
                .allocator
                .retire_image(&self.device, target.depth, u64::MAX, 0);
            let _ = self
                .allocator
                .retire_image(&self.device, target.color, u64::MAX, 0);
        }
        for mesh in self.meshes.drain_all() {
            let _ = self
                .allocator
                .retire_buffer(&self.device, mesh.index_buffer, u64::MAX, 0);
            let _ = self
                .allocator
                .retire_buffer(&self.device, mesh.vertex_buffer, u64::MAX, 0);
        }
        for compiled in self.pipelines.drain_all() {
            self.allocator.retire_pipeline(
                &self.device,
                compiled.pipeline,
                compiled.layout,
                u64::MAX,
                0,
            );
        }
        for shader in self.shaders.drain_all() {
            if let Some(module) = shader.module {
                self.allocator
                    .retire_shader_module(&self.device, module, u64::MAX, 0);
            }
        }
        self.retire_swapchain_depth_idle();
        self.frames.destroy(&self.device);
        if let Some(swapchain) = self.swapchain.take() {
            swapchain.destroy(&self.device);
        }
        let _leaked = self.allocator.shutdown(&self.device);
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        self.teardown();
        // `device`, `surface` and `instance` drop next, in field order.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vk_failure_classifies_recoverable_and_fatal_results() {
        assert!(matches!(
            VkFailure::from_call("vkCreateSwapchainKHR", vk::Result::ERROR_OUT_OF_DATE_KHR),
            VkFailure::Recoverable {
                surface_lost: false
            }
        ));
        assert!(matches!(
            VkFailure::from_call("vkCreateSwapchainKHR", vk::Result::ERROR_SURFACE_LOST_KHR),
            VkFailure::Recoverable { surface_lost: true }
        ));
        match VkFailure::from_call(
            "vkCreateSwapchainKHR",
            vk::Result::ERROR_NATIVE_WINDOW_IN_USE_KHR,
        ) {
            VkFailure::Fatal(error) => {
                assert_eq!(error.kind(), RenderErrorKind::VulkanCallFailed);
                assert_eq!(error.detail(), Some("vkCreateSwapchainKHR"));
            }
            other @ VkFailure::Recoverable { .. } => {
                panic!("expected fatal error for window in use, got {other:?}")
            }
        }
        match VkFailure::from_call("vkCreateSwapchainKHR", vk::Result::ERROR_DEVICE_LOST) {
            VkFailure::Fatal(error) => assert_eq!(error.kind(), RenderErrorKind::DeviceLost),
            other @ VkFailure::Recoverable { .. } => {
                panic!("expected fatal error for device lost, got {other:?}")
            }
        }
    }
}

/// GPU tests through `VK_EXT_headless_surface`: the real backend without a
/// window. Opt-in because CI has no GPU:
/// `cargo test -p spawn-render --features vulkan -- --ignored`.
#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::types::Validation;

    const IGNORE: &str = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface";

    fn renderer(size: SurfaceSize) -> VulkanRenderer {
        renderer_with_mode(size, PresentModePreference::Fifo)
    }

    fn renderer_with_mode(
        size: SurfaceSize,
        present_mode: PresentModePreference,
    ) -> VulkanRenderer {
        let descriptor = RendererDescriptor {
            present_mode,
            validation: Validation::IfAvailable,
        };
        match VulkanRenderer::new(
            SurfaceSource::Headless,
            size,
            &descriptor,
            GpuMemoryBudget::default(),
        ) {
            Ok(renderer) => renderer,
            Err(error) => panic!("{IGNORE}:\n{error}"),
        }
    }

    fn assert_no_validation_errors(renderer: &VulkanRenderer) {
        let mut messages = Vec::new();
        let dropped = renderer.drain_validation_messages(&mut messages);
        for message in &messages {
            eprintln!("validation {:?}: {}", message.severity, message.text);
        }
        assert_eq!(
            renderer.validation_error_count(),
            0,
            "validation errors (dropped: {dropped})"
        );
    }

    fn color() -> ClearColor {
        ClearColor::new(0.1, 0.3, 0.6, 1.0)
    }

    #[test]
    #[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
    fn presents_repeated_clear_frames() {
        let mut renderer = renderer(SurfaceSize::new(640, 480));
        eprintln!("{:?}", renderer.info());
        assert_eq!(renderer.state(), PresentationState::Ready);
        for frame in 0..120 {
            let outcome = renderer.render_clear(color());
            assert!(
                matches!(outcome, Ok(FrameOutcome::Presented)),
                "frame {frame}: {outcome:?}"
            );
        }
        assert_no_validation_errors(&renderer);
    }

    /// FND-T06 on a real device: zero size suspends without acquiring,
    /// valid sizes resume with a recreated swapchain and matching `D32_SFLOAT` depth.
    #[test]
    #[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
    fn resize_suspend_and_resume() {
        let mut renderer = renderer(SurfaceSize::new(640, 480));
        assert!(matches!(
            renderer.render_clear(color()),
            Ok(FrameOutcome::Presented)
        ));
        renderer.resize(SurfaceSize::new(0, 0));
        for _ in 0..3 {
            assert!(matches!(
                renderer.render_clear(color()),
                Ok(FrameOutcome::Suspended)
            ));
        }
        assert_eq!(renderer.state(), PresentationState::Suspended);
        for (width, height) in [(800, 600), (801, 599), (1, 1), (1920, 1080)] {
            renderer.resize(SurfaceSize::new(width, height));
            assert!(matches!(
                renderer.render_clear(color()),
                Ok(FrameOutcome::Presented)
            ));
            let extent = renderer
                .swapchain
                .as_ref()
                .map(|swapchain| swapchain.extent);
            let depth_extent = renderer.swapchain_depth.as_ref().map(|d| d.extent);
            assert_eq!(extent, Some(SurfaceSize::new(width, height)));
            assert_eq!(depth_extent, Some(SurfaceSize::new(width, height)));
        }
        assert_no_validation_errors(&renderer);
    }

    #[test]
    #[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
    fn starts_suspended_at_zero_size_and_repeated_create_drop_is_clean() {
        for _ in 0..3 {
            let mut renderer = renderer(SurfaceSize::new(0, 0));
            assert_eq!(renderer.state(), PresentationState::Suspended);
            assert!(renderer.swapchain.is_none());
            assert!(renderer.swapchain_depth.is_none());
            assert_eq!(renderer.allocator_stats().live_allocations, 0);
            assert!(matches!(
                renderer.render_clear(color()),
                Ok(FrameOutcome::Suspended)
            ));
            renderer.resize(SurfaceSize::new(320, 200));
            assert!(matches!(
                renderer.render_clear(color()),
                Ok(FrameOutcome::Presented)
            ));
            assert_eq!(
                renderer.swapchain_depth.as_ref().map(|d| d.extent),
                Some(SurfaceSize::new(320, 200))
            );
            assert_no_validation_errors(&renderer);
        }
    }

    #[test]
    #[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
    fn presents_across_present_mode_preferences() {
        for mode in [
            PresentModePreference::Fifo,
            PresentModePreference::Mailbox,
            PresentModePreference::Immediate,
        ] {
            let mut renderer = renderer_with_mode(SurfaceSize::new(320, 240), mode);
            for frame in 0..32 {
                let outcome = renderer.render_clear(color());
                assert!(
                    matches!(outcome, Ok(FrameOutcome::Presented)),
                    "{mode:?} frame {frame}: {outcome:?}"
                );
            }
            assert_no_validation_errors(&renderer);
        }
    }

    #[test]
    #[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
    fn upload_mesh_insert_failure_retires_gpu_buffers_with_zero_leaks() {
        let descriptor = RendererDescriptor {
            present_mode: PresentModePreference::Fifo,
            validation: Validation::Required,
        };
        let mut renderer = VulkanRenderer::new(
            SurfaceSource::Headless,
            SurfaceSize::new(0, 0),
            &descriptor,
            GpuMemoryBudget::default(),
        )
        .expect("headless Vulkan 1.3 renderer with required validation");
        assert!(renderer.info().validation_enabled);

        let verts = [
            crate::mesh::MeshVertex::new(
                spawn_math::Vec3::new(-1.0, -1.0, 0.0),
                spawn_math::Vec3::Z,
                spawn_math::Vec4::ONE,
            ),
            crate::mesh::MeshVertex::new(
                spawn_math::Vec3::new(1.0, -1.0, 0.0),
                spawn_math::Vec3::Z,
                spawn_math::Vec4::ONE,
            ),
            crate::mesh::MeshVertex::new(
                spawn_math::Vec3::new(0.0, 1.0, 0.0),
                spawn_math::Vec3::Z,
                spawn_math::Vec4::ONE,
            ),
        ];
        let indices = [0_u32, 1, 2];

        renderer.meshes.inject_insert_failure();
        let err = renderer
            .upload_mesh(&MeshDescriptor::new("rollback_tri", &verts, &indices))
            .unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::GpuMemoryExhausted);
        assert_eq!(err.code(), "REND-0012");

        // Buffers were immediately retired on error, not leaked as live allocations.
        let immediate_stats = renderer.allocator_stats();
        assert_eq!(immediate_stats.live_allocations, 0);
        assert_eq!(immediate_stats.live_allocated_bytes, 0);

        // Waiting and reclaiming frees both the rolled-back vertex/index buffers and staging buffer.
        let reclaimed = renderer.wait_idle_and_reclaim().unwrap();
        assert_eq!(reclaimed.live_allocations, 0);
        assert_eq!(reclaimed.live_allocated_bytes, 0);
        assert_eq!(reclaimed.retired_allocations, 0);
        assert_eq!(reclaimed.retired_bytes, 0);

        assert_no_validation_errors(&renderer);
        let closed = renderer.close().unwrap();
        assert_eq!(closed.block_count, 0);
        assert_eq!(closed.live_allocations, 0);
        assert_eq!(closed.retired_allocations, 0);
    }

    #[test]
    #[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
    fn render_meshes_reuses_prepared_draw_scratch_across_frames_without_reallocating() {
        let descriptor = RendererDescriptor {
            present_mode: PresentModePreference::Fifo,
            validation: Validation::Required,
        };
        let mut renderer = VulkanRenderer::new(
            SurfaceSource::Headless,
            SurfaceSize::new(0, 0),
            &descriptor,
            GpuMemoryBudget::default(),
        )
        .expect("headless Vulkan 1.3 renderer with required validation");
        assert!(renderer.info().validation_enabled);

        let proj = spawn_math::Mat4::perspective_reversed_z_rh(1.2, 1.0, 0.1, 100.0).unwrap();
        let view = CameraRelativeView::new(proj, spawn_math::Quat::IDENTITY);
        let verts = [
            crate::mesh::MeshVertex::new(
                spawn_math::Vec3::new(-1.0, -1.0, 0.0),
                spawn_math::Vec3::Z,
                spawn_math::Vec4::ONE,
            ),
            crate::mesh::MeshVertex::new(
                spawn_math::Vec3::new(1.0, -1.0, 0.0),
                spawn_math::Vec3::Z,
                spawn_math::Vec4::ONE,
            ),
            crate::mesh::MeshVertex::new(
                spawn_math::Vec3::new(0.0, 1.0, 0.0),
                spawn_math::Vec3::Z,
                spawn_math::Vec4::ONE,
            ),
        ];
        let indices = [0_u32, 1, 2];
        let mesh = renderer
            .upload_mesh(&MeshDescriptor::new("scratch_tri", &verts, &indices))
            .unwrap();
        let draws: Vec<MeshDraw> = (0..4)
            .map(|i| MeshDraw::new(mesh, spawn_math::Vec3::new(i as f32, 0.0, -3.0)))
            .collect();

        prepare_mesh_draws_into(
            &renderer.meshes,
            &mut renderer.prepared_draws,
            &view,
            &draws,
        )
        .unwrap();
        let cap = renderer.prepared_draws.capacity();
        let ptr = renderer.prepared_draws.as_ptr();
        assert!(cap >= 4);

        for count in [1_usize, 4, 2, 4] {
            prepare_mesh_draws_into(
                &renderer.meshes,
                &mut renderer.prepared_draws,
                &view,
                &draws[..count],
            )
            .unwrap();
            assert_eq!(renderer.prepared_draws.len(), count);
            assert_eq!(renderer.prepared_draws.capacity(), cap);
            assert_eq!(renderer.prepared_draws.as_ptr(), ptr);
        }

        renderer.destroy_mesh(mesh).unwrap();
        assert_no_validation_errors(&renderer);
        let closed = renderer.close().unwrap();
        assert_eq!(closed.block_count, 0);
        assert_eq!(closed.live_allocations, 0);
        assert_eq!(closed.retired_allocations, 0);
    }
}
