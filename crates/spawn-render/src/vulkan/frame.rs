//! Frames in flight: command recording and CPU/GPU synchronization.
//!
//! Synchronization strategy:
//!
//! - One **timeline semaphore** counts submitted frames. Frame `n` signals
//!   value `n`; before a frame slot is reused, the CPU waits until the
//!   timeline reaches the value that slot last signaled, so its command buffer
//!   and acquire semaphore are no longer in use.
//! - Each slot has a binary **acquire semaphore** (signaled by the
//!   presentation engine, waited by the submission at color-attachment
//!   output).
//! - Each swapchain image has a binary **present semaphore** (signaled by the
//!   submission, waited by presentation); see `Swapchain`.
//! - Layout transitions use synchronization2 barriers:
//!   `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` before dynamic rendering clears
//!   the image, `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` after it.

use std::time::Duration;

use ash::vk;

use crate::error::{RenderError, RenderErrorKind};
use crate::types::SurfaceSize;
use crate::vulkan::device::DeviceOwner;
use crate::vulkan::swapchain::color_range;
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

    /// Waits until the next slot's previous submission finished.
    pub(crate) fn begin(&mut self, device: &DeviceOwner) -> Result<FrameTicket, RenderError> {
        let index = self.next;
        let slot = self
            .slots
            .get(index)
            .ok_or_else(|| RenderError::new(RenderErrorKind::VulkanCallFailed))?;
        if slot.submitted > 0 {
            let semaphores = [self.timeline];
            let values = [slot.submitted];
            let info = vk::SemaphoreWaitInfo::default()
                .semaphores(&semaphores)
                .values(&values);
            let timeout = u64::try_from(GPU_WAIT_TIMEOUT.as_nanos()).unwrap_or(u64::MAX);
            // SAFETY: the timeline belongs to this device.
            match unsafe { device.device.wait_semaphores(&info, timeout) } {
                Ok(()) => {}
                Err(vk::Result::TIMEOUT) => {
                    return Err(RenderError::new(RenderErrorKind::GpuTimeout).with_detail(
                        format!("frame slot {index} not finished after {GPU_WAIT_TIMEOUT:?}"),
                    ));
                }
                Err(result) => return Err(vk_error("vkWaitSemaphores", result)),
            }
        }
        Ok(FrameTicket {
            index,
            acquire: slot.acquire,
        })
    }

    /// Records the clear of `image` into the ticket's command buffer.
    pub(crate) fn record_clear(
        &self,
        device: &DeviceOwner,
        ticket: &FrameTicket,
        image: vk::Image,
        view: vk::ImageView,
        extent: SurfaceSize,
        color: [f32; 4],
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

        let to_attachment = [vk::ImageMemoryBarrier2::default()
            // Chains with the acquire semaphore wait at this stage.
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .image(image)
            .subresource_range(color_range())];
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue { float32: color },
            })];
        let rendering = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: vk::Extent2D {
                    width: extent.width,
                    height: extent.height,
                },
            })
            .layer_count(1)
            .color_attachments(&attachments);
        let to_present = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            // Presentation is ordered by the present semaphore, not by a stage.
            .dst_stage_mask(vk::PipelineStageFlags2::NONE)
            .dst_access_mask(vk::AccessFlags2::NONE)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(image)
            .subresource_range(color_range())];
        // SAFETY: the command buffer is recording; `image` and `view` belong
        // to the current swapchain, which stays alive until this submission
        // completes (recreation waits for the device to be idle).
        unsafe {
            vk_device.cmd_pipeline_barrier2(
                slot.commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
            );
            vk_device.cmd_begin_rendering(slot.commands, &rendering);
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

    /// Submits the ticket's commands: waits for the acquire semaphore,
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
            vk::SemaphoreSubmitInfo::default()
                .semaphore(present)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT),
            vk::SemaphoreSubmitInfo::default()
                .semaphore(self.timeline)
                .value(value)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
        ];
        let submit = [vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .command_buffer_infos(&commands)
            .signal_semaphore_infos(&signals)];
        // SAFETY: the command buffer was recorded by `record_clear`; the
        // semaphores belong to this device; the queue is used from this thread
        // only.
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
