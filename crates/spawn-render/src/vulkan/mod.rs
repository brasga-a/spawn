//! In-house RHI over Vulkan 1.3 (ADR-0004). Private to `spawn-render`.
//!
//! Ownership and destruction order (fields of [`VulkanRenderer`]):
//!
//! 1. frames (command pools, acquire semaphores, timeline) and the swapchain
//!    (views, present semaphores): destroyed explicitly in `Drop` after the
//!    device is idle;
//! 2. the device (`DeviceOwner`);
//! 3. the surface (`SurfaceOwner`);
//! 4. the instance and validation messenger (`InstanceOwner`).
//!
//! During construction the same order holds through local variables: a
//! failure at any step drops only what was already created, in reverse order.

mod device;
mod frame;
mod instance;
mod surface;
mod swapchain;

use std::time::Duration;

use ash::vk;

use crate::error::{RenderError, RenderErrorKind};
use crate::presentation::{
    AcquireStatus, FramePlan, PresentStatus, PresentationController, StopReason,
};
use crate::types::{
    ClearColor, FrameOutcome, PresentModePreference, PresentationState, RendererDescriptor,
    RendererInfo, SurfaceSize, ValidationMessage,
};
use device::DeviceOwner;
use frame::Frames;
use instance::InstanceOwner;
use surface::SurfaceOwner;
pub(crate) use surface::SurfaceSource;
use swapchain::{Swapchain, SwapchainBuild};

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

/// The Vulkan renderer: one surface, one device, one queue, one swapchain.
pub(crate) struct VulkanRenderer {
    controller: PresentationController,
    present_mode: PresentModePreference,
    source: SurfaceSource,
    info: RendererInfo,
    /// Code of the error that stopped the renderer, for later calls.
    fatal_code: Option<&'static str>,
    frames: Frames,
    swapchain: Option<Swapchain>,
    device: DeviceOwner,
    surface: SurfaceOwner,
    instance: InstanceOwner,
}

impl VulkanRenderer {
    pub(crate) fn new(
        source: SurfaceSource,
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
    ) -> Result<Self, RenderError> {
        let extension = source.extension()?;
        let instance = InstanceOwner::new(extension, descriptor.validation)?;
        let surface = SurfaceOwner::new(&instance, &source)?;
        let (device, info) = DeviceOwner::new(&instance, &surface)?;
        let frames = Frames::new(&device, device.queue_family)?;
        let mut renderer = Self {
            controller: PresentationController::new(size),
            present_mode: descriptor.present_mode,
            source,
            info,
            fatal_code: None,
            frames,
            swapchain: None,
            device,
            surface,
            instance,
        };
        // Create the first swapchain now so configuration errors surface at
        // startup; a zero size or zero surface extent just suspends.
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

    pub(crate) fn resize(&mut self, size: SurfaceSize) {
        self.controller.resize(size);
    }

    pub(crate) fn drain_validation_messages(&self, out: &mut Vec<ValidationMessage>) -> u64 {
        self.instance.sink().drain_into(out)
    }

    pub(crate) fn validation_error_count(&self) -> u64 {
        self.instance.sink().error_count()
    }

    /// Clears the next swapchain image to `color` and presents it.
    pub(crate) fn render_clear(&mut self, color: ClearColor) -> Result<FrameOutcome, RenderError> {
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
        self.draw(color)
    }

    fn draw(&mut self, color: ClearColor) -> Result<FrameOutcome, RenderError> {
        let ticket = self
            .frames
            .begin(&self.device)
            .map_err(|error| self.fatal(error))?;
        let Some((handle, extent, encode_srgb)) = self
            .swapchain
            .as_ref()
            .map(|swapchain| (swapchain.handle, swapchain.extent, swapchain.encode_srgb))
        else {
            return Ok(FrameOutcome::Skipped);
        };
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
        let Some((image, view, present)) = targets else {
            return Err(self.fatal(
                RenderError::new(RenderErrorKind::VulkanCallFailed).with_detail(format!(
                    "vkAcquireNextImageKHR returned image index {image_index} out of range"
                )),
            ));
        };
        let clear = if encode_srgb {
            color.srgb_encoded()
        } else {
            color.sanitized()
        };

        let recorded = self
            .frames
            .record_clear(&self.device, &ticket, image, view, extent, clear)
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

    /// Recreates the swapchain for the requested size. Returns whether a
    /// drawable swapchain now exists.
    fn recreate_swapchain(&mut self) -> Result<bool, RenderError> {
        // The old swapchain's images may still be used by submitted frames.
        self.device.wait_idle().map_err(|error| self.fatal(error))?;
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
                self.controller.swapchain_recreated(swapchain.extent);
                self.swapchain = Some(swapchain);
                Ok(true)
            }
            Ok(SwapchainBuild::ZeroExtent) => {
                self.controller.swapchain_recreated(SurfaceSize::default());
                Ok(false)
            }
            Err(VkFailure::Recoverable { surface_lost }) => {
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
            Err(VkFailure::Fatal(error)) => Err(self.fatal(error)),
        }
    }

    /// Replaces a lost surface with a new one for the same window. Returns
    /// whether it succeeded; a failure is retried at the next frame within
    /// the recovery bound.
    fn recreate_surface(&mut self) -> Result<bool, RenderError> {
        self.device.wait_idle().map_err(|error| self.fatal(error))?;
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
                .with_detail(format!("stopped by error[{}]", code.unwrap_or("unknown"))),
        }
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        // Nothing may be destroyed while the GPU still uses it. If the device
        // is lost, waiting fails but destruction is still valid.
        let _ = self.device.wait_idle();
        self.frames.destroy(&self.device);
        if let Some(swapchain) = self.swapchain.take() {
            swapchain.destroy(&self.device);
        }
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
        match VulkanRenderer::new(SurfaceSource::Headless, size, &descriptor) {
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
    /// valid sizes resume with a recreated swapchain of that size.
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
            assert_eq!(extent, Some(SurfaceSize::new(width, height)));
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
            assert!(matches!(
                renderer.render_clear(color()),
                Ok(FrameOutcome::Suspended)
            ));
            renderer.resize(SurfaceSize::new(320, 200));
            assert!(matches!(
                renderer.render_clear(color()),
                Ok(FrameOutcome::Presented)
            ));
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
}
