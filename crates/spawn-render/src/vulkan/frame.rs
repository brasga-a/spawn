//! Frames in flight: command recording, staging uploads, and CPU/GPU synchronization.
//!
//! Synchronization strategy:
//!
//! - One **timeline semaphore** counts submitted passes/uploads. Submission `n`
//!   signals value `n`; before a frame slot is reused, the CPU waits until the
//!   timeline reaches the value that slot last signaled, so its command buffer
//!   and acquire semaphore are no longer in use.
//! - Each slot has a binary **acquire semaphore** (signaled by the
//!   presentation engine, waited by swapchain submissions at color-attachment
//!   output).
//! - Each swapchain image has a binary **present semaphore** (signaled by the
//!   submission at `ALL_COMMANDS` after the present layout transition, waited
//!   by presentation); see `Swapchain`.
//! - Layout transitions and buffer transfers use `synchronization2` barriers:
//!   - Mesh staging upload: `vkCmdCopyBuffer2` followed by `COPY (TRANSFER_WRITE)`
//!     → `VERTEX_ATTRIBUTE_INPUT (VERTEX_ATTRIBUTE_READ)` and
//!     `INDEX_INPUT (INDEX_READ)` `VkBufferMemoryBarrier2`s.
//!   - Swapchain color: `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL`, then
//!     `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR`.
//!   - Reversed-Z `D32_SFLOAT` depth: `UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL`
//!     ordered across `EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS` (`READ | WRITE`)
//!     and cleared to `0.0` (`REVERSED_Z_CLEAR_DEPTH`).
//!   - Offscreen readback: `COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL`,
//!     `vkCmdCopyImageToBuffer2`, and `COPY (TRANSFER_WRITE) → HOST (HOST_READ)`.

use std::time::Duration;

use ash::vk;

use crate::depth::REVERSED_Z_CLEAR_DEPTH;
use crate::error::{RenderError, RenderErrorKind};
use crate::types::SurfaceSize;
use crate::vulkan::device::DeviceOwner;
use crate::vulkan::mesh::PreparedMeshDraw;
use crate::vulkan::swapchain::{color_range, depth_range};
use crate::vulkan::vk_error;

/// Frames the CPU may record ahead of the GPU.
pub(crate) const FRAMES_IN_FLIGHT: usize = 2;

/// Longest the CPU waits for a frame slot before declaring the GPU hung.
pub(crate) const GPU_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

struct FrameSlot {
    pool: vk::CommandPool,
    commands: vk::CommandBuffer,
    acquire: vk::Semaphore,
    /// Timeline value signaled by this slot's last submission (0: none).
    submitted: u64,
}

/// Graphics draw workload inside a dynamic-rendering pass.
#[derive(Clone, Copy)]
pub(crate) enum PassDraw<'a> {
    None,
    Unindexed {
        pipeline: vk::Pipeline,
        vertex_count: u32,
    },
    IndexedMeshes {
        pipeline: vk::Pipeline,
        layout: vk::PipelineLayout,
        push_stage_flags: vk::ShaderStageFlags,
        draws: &'a [PreparedMeshDraw],
    },
}

/// Swapchain render target bindings for one frame.
#[derive(Clone, Copy)]
pub(crate) struct SwapchainPassTarget {
    pub color_image: vk::Image,
    pub color_view: vk::ImageView,
    pub depth_image: vk::Image,
    pub depth_view: vk::ImageView,
    pub extent: SurfaceSize,
    pub clear_color: [f32; 4],
}

/// Offscreen render + readback bindings for one deterministic capture pass.
#[derive(Clone, Copy)]
pub(crate) struct OffscreenPassTarget {
    pub color_image: vk::Image,
    pub color_view: vk::ImageView,
    pub depth_image: vk::Image,
    pub depth_view: vk::ImageView,
    pub extent: SurfaceSize,
    pub clear_color: [f32; 4],
    pub readback_buffer: vk::Buffer,
    pub readback_bytes: u64,
}

/// All per-frame resources. Destroyed explicitly with the device.
pub(crate) struct Frames {
    slots: Vec<FrameSlot>,
    timeline: vk::Semaphore,
    last_submitted: u64,
    next: usize,
}

/// A slot ready for recording.
pub(crate) struct FrameTicket {
    index: usize,
    pub acquire: vk::Semaphore,
}

impl Frames {
    pub(crate) fn new(device: &DeviceOwner, queue_family: u32) -> Result<Self, RenderError> {
        let mut timeline_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline_info);
        // SAFETY: timeline semaphores are an enabled device feature.
        let timeline = unsafe { device.device.create_semaphore(&info, None) }
            .map_err(|result| vk_error("vkCreateSemaphore (timeline)", result))?;
        device.name(timeline, c"spawn.frames.timeline");
        let mut frames = Self {
            slots: Vec::with_capacity(FRAMES_IN_FLIGHT),
            timeline,
            last_submitted: 0,
            next: 0,
        };
        for _ in 0..FRAMES_IN_FLIGHT {
            match create_slot(device, queue_family) {
                Ok(slot) => frames.slots.push(slot),
                Err(error) => {
                    frames.destroy(device);
                    return Err(error);
                }
            }
        }
        Ok(frames)
    }

    /// Most recent timeline value submitted to the queue (`0` before first submission).
    pub(crate) const fn last_submitted(&self) -> u64 {
        self.last_submitted
    }

    /// Queries the current GPU-completed counter value of the timeline semaphore.
    pub(crate) fn completed_timeline(&self, device: &DeviceOwner) -> Result<u64, RenderError> {
        if self.timeline == vk::Semaphore::null() {
            return Ok(self.last_submitted);
        }
        // SAFETY: `self.timeline` is a valid timeline semaphore created on `device`.
        unsafe { device.device.get_semaphore_counter_value(self.timeline) }
            .map_err(|result| vk_error("vkGetSemaphoreCounterValue", result))
    }

    /// Waits until `self.timeline` reaches at least `value` on the GPU.
    pub(crate) fn wait_timeline(
        &self,
        device: &DeviceOwner,
        value: u64,
    ) -> Result<(), RenderError> {
        if value == 0 {
            return Ok(());
        }
        let semaphores = [self.timeline];
        let values = [value];
        let info = vk::SemaphoreWaitInfo::default()
            .semaphores(&semaphores)
            .values(&values);
        let timeout = u64::try_from(GPU_WAIT_TIMEOUT.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: `self.timeline` belongs to `device`.
        match unsafe { device.device.wait_semaphores(&info, timeout) } {
            Ok(()) => Ok(()),
            Err(vk::Result::TIMEOUT) => Err(RenderError::new(RenderErrorKind::GpuTimeout)
                .with_detail(format!(
                    "timeline value {value} not reached after {GPU_WAIT_TIMEOUT:?}"
                ))),
            Err(result) => Err(vk_error("vkWaitSemaphores", result)),
        }
    }

    /// Waits until the next slot's previous submission finished.
    pub(crate) fn begin(&mut self, device: &DeviceOwner) -> Result<FrameTicket, RenderError> {
        let index = self.next;
        let slot = self
            .slots
            .get(index)
            .ok_or_else(|| RenderError::new(RenderErrorKind::VulkanCallFailed))?;
        let acquire = slot.acquire;
        let submitted = slot.submitted;
        if submitted > 0 {
            self.wait_timeline(device, submitted).map_err(|err| {
                if err.kind() == RenderErrorKind::GpuTimeout {
                    RenderError::new(RenderErrorKind::GpuTimeout).with_detail(format!(
                        "frame slot {index} not finished after {GPU_WAIT_TIMEOUT:?}"
                    ))
                } else {
                    err
                }
            })?;
        }
        Ok(FrameTicket { index, acquire })
    }

    /// Records `vkCmdCopyBuffer2` staging copies for `vertex_buffer` (`[0..vertex_bytes)`)
    /// and `index_buffer` (`[vertex_bytes..vertex_bytes + index_bytes)`), places
    /// `synchronization2` `VkBufferMemoryBarrier2`s ordering `COPY (TRANSFER_WRITE)`
    /// before `VERTEX_ATTRIBUTE_INPUT` and `INDEX_INPUT`, and submits on `device.queue`
    /// signaling the timeline semaphore. Returns the signaled timeline value so the
    /// caller can queue `staging_buffer` on the timeline retirement list without stalling.
    pub(crate) fn execute_mesh_staging_upload(
        &mut self,
        device: &DeviceOwner,
        staging_buffer: vk::Buffer,
        vertex_buffer: vk::Buffer,
        vertex_bytes: u64,
        index_buffer: vk::Buffer,
        index_bytes: u64,
    ) -> Result<u64, RenderError> {
        let ticket = self.begin(device)?;
        let commands_handle = self.slot(&ticket)?.commands;
        let pool_handle = self.slot(&ticket)?.pool;
        let vk_device = &device.device;

        // SAFETY: `begin` waited for this slot's previous submission to complete.
        unsafe { vk_device.reset_command_pool(pool_handle, vk::CommandPoolResetFlags::empty()) }
            .map_err(|result| vk_error("vkResetCommandPool", result))?;
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: `commands_handle` is in initial state after resetting `pool_handle`.
        unsafe { vk_device.begin_command_buffer(commands_handle, &begin_info) }
            .map_err(|result| vk_error("vkBeginCommandBuffer", result))?;

        let vb_regions = [vk::BufferCopy2::default()
            .src_offset(0)
            .dst_offset(0)
            .size(vertex_bytes)];
        let vb_copy = vk::CopyBufferInfo2::default()
            .src_buffer(staging_buffer)
            .dst_buffer(vertex_buffer)
            .regions(&vb_regions);

        let ib_regions = [vk::BufferCopy2::default()
            .src_offset(vertex_bytes)
            .dst_offset(0)
            .size(index_bytes)];
        let ib_copy = vk::CopyBufferInfo2::default()
            .src_buffer(staging_buffer)
            .dst_buffer(index_buffer)
            .regions(&ib_regions);

        let post_copy_barriers = [
            vk::BufferMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT)
                .dst_access_mask(vk::AccessFlags2::VERTEX_ATTRIBUTE_READ)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(vertex_buffer)
                .offset(0)
                .size(vertex_bytes),
            vk::BufferMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::INDEX_INPUT)
                .dst_access_mask(vk::AccessFlags2::INDEX_READ)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(index_buffer)
                .offset(0)
                .size(index_bytes),
        ];

        // SAFETY: `commands_handle` is recording; `staging_buffer`, `vertex_buffer`, and
        // `index_buffer` belong to `device` and stay alive until this timeline value completes.
        unsafe {
            vk_device.cmd_copy_buffer2(commands_handle, &vb_copy);
            vk_device.cmd_copy_buffer2(commands_handle, &ib_copy);
            vk_device.cmd_pipeline_barrier2(
                commands_handle,
                &vk::DependencyInfo::default().buffer_memory_barriers(&post_copy_barriers),
            );
            vk_device
                .end_command_buffer(commands_handle)
                .map_err(|result| vk_error("vkEndCommandBuffer", result))?;
        }

        self.submit_timeline_only(device, ticket, commands_handle)
    }

    /// Records a swapchain dynamic-rendering pass (color clear + `D32_SFLOAT`
    /// reversed-Z depth clear to `0.0` + optional unindexed or indexed 3D draws) into `ticket`.
    pub(crate) fn record_swapchain_pass(
        &self,
        device: &DeviceOwner,
        ticket: &FrameTicket,
        target: &SwapchainPassTarget,
        draw: PassDraw<'_>,
    ) -> Result<(), RenderError> {
        let slot = self.slot(ticket)?;
        let vk_device = &device.device;
        // SAFETY: the slot's previous submission completed (`begin`), so the
        // pool's command buffer is not in use.
        unsafe { vk_device.reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty()) }
            .map_err(|result| vk_error("vkResetCommandPool", result))?;
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: the command buffer is in the initial state after the reset.
        unsafe { vk_device.begin_command_buffer(slot.commands, &begin) }
            .map_err(|result| vk_error("vkBeginCommandBuffer", result))?;

        let depth_stages = vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
        let depth_access = vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
            | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE;

        let to_attachments = [
            vk::ImageMemoryBarrier2::default()
                // Chains with the acquire semaphore wait at this stage.
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(target.color_image)
                .subresource_range(color_range()),
            vk::ImageMemoryBarrier2::default()
                // Orders after prior frame depth reads/writes on the shared depth image.
                .src_stage_mask(depth_stages)
                .src_access_mask(depth_access)
                .dst_stage_mask(depth_stages)
                .dst_access_mask(depth_access)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .image(target.depth_image)
                .subresource_range(depth_range()),
        ];

        let color_attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(target.color_view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: target.clear_color,
                },
            })];
        let depth_attachment = vk::RenderingAttachmentInfo::default()
            .image_view(target.depth_view)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .clear_value(vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: REVERSED_Z_CLEAR_DEPTH,
                    stencil: 0,
                },
            });
        let render_area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent: vk::Extent2D {
                width: target.extent.width,
                height: target.extent.height,
            },
        };
        let rendering = vk::RenderingInfo::default()
            .render_area(render_area)
            .layer_count(1)
            .color_attachments(&color_attachments)
            .depth_attachment(&depth_attachment);

        let to_present = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            // Ordered before presentation by the present semaphore signal at
            // `ALL_COMMANDS`, which includes prior layout transitions.
            .dst_stage_mask(vk::PipelineStageFlags2::NONE)
            .dst_access_mask(vk::AccessFlags2::NONE)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(target.color_image)
            .subresource_range(color_range())];

        // SAFETY: the command buffer is recording; color and depth images/views
        // and any bound pipeline/mesh buffers belong to `device` and stay alive
        // until this submission completes on the timeline semaphore.
        unsafe {
            vk_device.cmd_pipeline_barrier2(
                slot.commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_attachments),
            );
            vk_device.cmd_begin_rendering(slot.commands, &rendering);
            record_pass_draws(vk_device, slot.commands, target.extent, draw);
            vk_device.cmd_end_rendering(slot.commands);
            vk_device.cmd_pipeline_barrier2(
                slot.commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_present),
            );
        }
        // SAFETY: the command buffer is recording.
        unsafe { vk_device.end_command_buffer(slot.commands) }
            .map_err(|result| vk_error("vkEndCommandBuffer", result))
    }

    /// Records an offscreen `R8G8B8A8_UNORM` + `D32_SFLOAT` reversed-Z dynamic-rendering
    /// pass (`unindexed` or `indexed` 3D meshes), transitions color to `TRANSFER_SRC_OPTIMAL`,
    /// copies into `target.readback_buffer` via `vkCmdCopyImageToBuffer2`, orders
    /// `COPY -> HOST_READ`, submits on `device.queue` signaling the timeline semaphore,
    /// and waits for completion. Returns the signaled timeline value.
    pub(crate) fn execute_offscreen_draw_and_copy(
        &mut self,
        device: &DeviceOwner,
        target: &OffscreenPassTarget,
        draw: PassDraw<'_>,
    ) -> Result<u64, RenderError> {
        let ticket = self.begin(device)?;
        let commands_handle = self.slot(&ticket)?.commands;
        let pool_handle = self.slot(&ticket)?.pool;
        let vk_device = &device.device;

        // SAFETY: `begin` waited for this slot's previous submission to complete.
        unsafe { vk_device.reset_command_pool(pool_handle, vk::CommandPoolResetFlags::empty()) }
            .map_err(|result| vk_error("vkResetCommandPool", result))?;
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: `commands_handle` is in initial state after resetting `pool_handle`.
        unsafe { vk_device.begin_command_buffer(commands_handle, &begin_info) }
            .map_err(|result| vk_error("vkBeginCommandBuffer", result))?;

        let depth_stages = vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
        let depth_access = vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
            | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE;

        let pre_render_barriers = [
            vk::ImageMemoryBarrier2::default()
                // Orders after any prior offscreen render or `TRANSFER_READ` copy on the same target.
                .src_stage_mask(
                    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags2::COPY,
                )
                .src_access_mask(
                    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE | vk::AccessFlags2::TRANSFER_READ,
                )
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(
                    vk::AccessFlags2::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                )
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(target.color_image)
                .subresource_range(color_range()),
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(depth_stages)
                .src_access_mask(depth_access)
                .dst_stage_mask(depth_stages)
                .dst_access_mask(depth_access)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .image(target.depth_image)
                .subresource_range(depth_range()),
        ];

        let color_attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(target.color_view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: target.clear_color,
                },
            })];
        let depth_attachment = vk::RenderingAttachmentInfo::default()
            .image_view(target.depth_view)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: REVERSED_Z_CLEAR_DEPTH,
                    stencil: 0,
                },
            });
        let rendering = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: vk::Extent2D {
                    width: target.extent.width,
                    height: target.extent.height,
                },
            })
            .layer_count(1)
            .color_attachments(&color_attachments)
            .depth_attachment(&depth_attachment);

        let to_transfer_src = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(target.color_image)
            .subresource_range(color_range())];

        let copy_regions = [vk::BufferImageCopy2::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width: target.extent.width,
                height: target.extent.height,
                depth: 1,
            })];
        let copy_info = vk::CopyImageToBufferInfo2::default()
            .src_image(target.color_image)
            .src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .dst_buffer(target.readback_buffer)
            .regions(&copy_regions);

        let to_host_read = [vk::BufferMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(target.readback_buffer)
            .offset(0)
            .size(target.readback_bytes)];

        // SAFETY: `commands_handle` is recording; offscreen images/views, pipeline,
        // mesh buffers, and `readback_buffer` belong to `device` and stay alive
        // through `wait_timeline` below.
        unsafe {
            vk_device.cmd_pipeline_barrier2(
                commands_handle,
                &vk::DependencyInfo::default().image_memory_barriers(&pre_render_barriers),
            );
            vk_device.cmd_begin_rendering(commands_handle, &rendering);
            record_pass_draws(vk_device, commands_handle, target.extent, draw);
            vk_device.cmd_end_rendering(commands_handle);
            vk_device.cmd_pipeline_barrier2(
                commands_handle,
                &vk::DependencyInfo::default().image_memory_barriers(&to_transfer_src),
            );
            vk_device.cmd_copy_image_to_buffer2(commands_handle, &copy_info);
            vk_device.cmd_pipeline_barrier2(
                commands_handle,
                &vk::DependencyInfo::default().buffer_memory_barriers(&to_host_read),
            );
            vk_device
                .end_command_buffer(commands_handle)
                .map_err(|result| vk_error("vkEndCommandBuffer", result))?;
        }

        let value = self.submit_timeline_only(device, ticket, commands_handle)?;
        self.wait_timeline(device, value)?;
        Ok(value)
    }

    /// Submits `commands_handle` with no swapchain semaphores, signaling only the
    /// next timeline value at `ALL_COMMANDS`.
    fn submit_timeline_only(
        &mut self,
        device: &DeviceOwner,
        ticket: FrameTicket,
        commands_handle: vk::CommandBuffer,
    ) -> Result<u64, RenderError> {
        let value = self.last_submitted.saturating_add(1);
        let cmd_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(commands_handle)];
        let signals = [vk::SemaphoreSubmitInfo::default()
            .semaphore(self.timeline)
            .value(value)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
        let submit = [vk::SubmitInfo2::default()
            .command_buffer_infos(&cmd_infos)
            .signal_semaphore_infos(&signals)];

        // SAFETY: `commands_handle` is executable, `self.timeline` belongs to `device`,
        // and `device.queue` is accessed exclusively on this thread.
        unsafe {
            device
                .device
                .queue_submit2(device.queue, &submit, vk::Fence::null())
        }
        .map_err(|result| vk_error("vkQueueSubmit2 (timeline)", result))?;

        self.last_submitted = value;
        if let Some(slot) = self.slots.get_mut(ticket.index) {
            slot.submitted = value;
        }
        self.next = (ticket.index + 1) % FRAMES_IN_FLIGHT;
        Ok(value)
    }

    /// Submits the ticket's swapchain commands: waits for the acquire semaphore,
    /// signals `present` and the next timeline value.
    pub(crate) fn submit(
        &mut self,
        device: &DeviceOwner,
        ticket: FrameTicket,
        present: vk::Semaphore,
    ) -> Result<(), RenderError> {
        let value = self.last_submitted.saturating_add(1);
        let slot = self.slot(&ticket)?;
        let waits = [vk::SemaphoreSubmitInfo::default()
            .semaphore(slot.acquire)
            .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
        let commands = [vk::CommandBufferSubmitInfo::default().command_buffer(slot.commands)];
        let signals = [
            // `ALL_COMMANDS` (not `COLOR_ATTACHMENT_OUTPUT`) so the semaphore
            // signal orders after the `to_present` layout transition barrier
            // whose `dst_stage_mask` is `NONE`.
            vk::SemaphoreSubmitInfo::default()
                .semaphore(present)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
            vk::SemaphoreSubmitInfo::default()
                .semaphore(self.timeline)
                .value(value)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
        ];
        let submit = [vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .command_buffer_infos(&commands)
            .signal_semaphore_infos(&signals)];
        // SAFETY: the command buffer was recorded by `record_swapchain_pass`; the
        // semaphores belong to this device; the queue is used from this thread only.
        unsafe {
            device
                .device
                .queue_submit2(device.queue, &submit, vk::Fence::null())
        }
        .map_err(|result| vk_error("vkQueueSubmit2", result))?;
        self.last_submitted = value;
        if let Some(slot) = self.slots.get_mut(ticket.index) {
            slot.submitted = value;
        }
        self.next = (ticket.index + 1) % FRAMES_IN_FLIGHT;
        Ok(())
    }

    /// Destroys every frame resource. The caller ensures the device is idle.
    pub(crate) fn destroy(&mut self, device: &DeviceOwner) {
        // SAFETY: the device is idle, so no submission references these
        // objects; slots are drained so each is destroyed once.
        unsafe {
            for slot in self.slots.drain(..) {
                device.device.destroy_command_pool(slot.pool, None);
                device.device.destroy_semaphore(slot.acquire, None);
            }
            if self.timeline != vk::Semaphore::null() {
                device.device.destroy_semaphore(self.timeline, None);
                self.timeline = vk::Semaphore::null();
            }
        }
    }

    fn slot(&self, ticket: &FrameTicket) -> Result<&FrameSlot, RenderError> {
        self.slots.get(ticket.index).ok_or_else(|| {
            RenderError::new(RenderErrorKind::VulkanCallFailed).with_detail("frame slot missing")
        })
    }
}

/// Binds the pass's graphics pipeline, configures a positive-height `[0.0, 1.0]`
/// `VkViewport` (`SPN-INV-053`: never flipping Y in viewport) and full-extent
/// `VkRect2D` scissor, and records either an unindexed draw or indexed 3D mesh draws.
fn record_pass_draws(
    vk_device: &ash::Device,
    commands: vk::CommandBuffer,
    extent: SurfaceSize,
    draw: PassDraw<'_>,
) {
    let viewports = [vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    }];
    let scissors = [vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: extent.width,
            height: extent.height,
        },
    }];

    match draw {
        PassDraw::None => {}
        PassDraw::Unindexed {
            pipeline,
            vertex_count,
        } => {
            // SAFETY: `commands` is inside an active `cmd_begin_rendering` pass matching
            // `pipeline`'s color and `D32_SFLOAT` depth formats, with dynamic `VIEWPORT`
            // and `SCISSOR` states enabled on `pipeline`.
            unsafe {
                vk_device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, pipeline);
                vk_device.cmd_set_viewport(commands, 0, &viewports);
                vk_device.cmd_set_scissor(commands, 0, &scissors);
                if vertex_count > 0 {
                    vk_device.cmd_draw(commands, vertex_count, 1, 0, 0);
                }
            }
        }
        PassDraw::IndexedMeshes {
            pipeline,
            layout,
            push_stage_flags,
            draws,
        } => {
            // SAFETY: `commands` is inside an active `cmd_begin_rendering` pass matching
            // `pipeline`; `push_stage_flags` matches `layout`'s `VkPushConstantRange`
            // (`128` bytes), and each `PreparedMeshDraw`'s `DEVICE_LOCAL` vertex/index
            // buffers belong to `vk_device` and stay alive until timeline completion.
            unsafe {
                vk_device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, pipeline);
                vk_device.cmd_set_viewport(commands, 0, &viewports);
                vk_device.cmd_set_scissor(commands, 0, &scissors);
                for item in draws {
                    vk_device.cmd_push_constants(
                        commands,
                        layout,
                        push_stage_flags,
                        0,
                        bytemuck::bytes_of(&item.push_constants),
                    );
                    let vbs = [item.vertex_buffer];
                    let offsets = [0_u64];
                    vk_device.cmd_bind_vertex_buffers(commands, 0, &vbs, &offsets);
                    vk_device.cmd_bind_index_buffer(
                        commands,
                        item.index_buffer,
                        0,
                        vk::IndexType::UINT32,
                    );
                    vk_device.cmd_draw_indexed(commands, item.index_count, 1, 0, 0, 0);
                }
            }
        }
    }
}

fn create_slot(device: &DeviceOwner, queue_family: u32) -> Result<FrameSlot, RenderError> {
    let vk_device = &device.device;
    let pool_info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::TRANSIENT)
        .queue_family_index(queue_family);
    // SAFETY: valid device and queue family.
    let pool = unsafe { vk_device.create_command_pool(&pool_info, None) }
        .map_err(|result| vk_error("vkCreateCommandPool", result))?;
    device.name(pool, c"spawn.frame.commands");
    let allocate = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    // SAFETY: the pool was just created on this device.
    let commands = match unsafe { vk_device.allocate_command_buffers(&allocate) } {
        Ok(buffers) => buffers.first().copied(),
        Err(result) => {
            // SAFETY: the pool is unused; destroyed once on this error path.
            unsafe { vk_device.destroy_command_pool(pool, None) };
            return Err(vk_error("vkAllocateCommandBuffers", result));
        }
    };
    let Some(commands) = commands else {
        // SAFETY: as above.
        unsafe { vk_device.destroy_command_pool(pool, None) };
        return Err(RenderError::new(RenderErrorKind::VulkanCallFailed)
            .with_detail("vkAllocateCommandBuffers returned no command buffer"));
    };
    // SAFETY: plain binary semaphore on a valid device.
    match unsafe { vk_device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) } {
        Ok(acquire) => {
            device.name(acquire, c"spawn.frame.acquire");
            Ok(FrameSlot {
                pool,
                commands,
                acquire,
                submitted: 0,
            })
        }
        Err(result) => {
            // SAFETY: the pool (and its command buffer) is unused.
            unsafe { vk_device.destroy_command_pool(pool, None) };
            Err(vk_error("vkCreateSemaphore", result))
        }
    }
}
