//! Presentation policy: when to acquire, when to recreate, when to stop.
//!
//! The Vulkan backend asks [`PresentationController::plan`] what to do at the
//! start of each frame and reports every swapchain result back. All decisions
//! (suspension at zero size, recreation on resize/out-of-date/suboptimal,
//! surface-loss recovery, bounded retries, fatal stop) live here, use no
//! Vulkan types and are tested without a GPU (FND-T06, FND-T07).
//!
//! The controller has no clock and no access to simulation state: resize and
//! recovery can never reset or advance simulation time (FND-INV-06).

use crate::types::{PresentationState, SurfaceSize};

/// Consecutive failed swapchain/surface recreations before giving up.
pub(crate) const MAX_RECOVERY_FAILURES: u32 = 3;

/// Consecutive recoveries without a successfully presented frame before
/// giving up. A normal resize needs one; an interactive resize that keeps
/// outdating the swapchain needs a few, separated by presented frames.
pub(crate) const MAX_UNPRESENTED_RECOVERIES: u32 = 16;

/// What the backend must do before rendering the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramePlan {
    /// Do nothing: the surface is not drawable.
    Suspend,
    /// Recreate the Vulkan surface, then the swapchain.
    RecreateSurface,
    /// Recreate the swapchain for [`PresentationController::requested`].
    RecreateSwapchain,
    /// Acquire an image from the current swapchain and render.
    Acquire,
}

/// Result of `vkAcquireNextImageKHR`, reduced to what the policy needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcquireStatus {
    Success,
    Suboptimal,
    OutOfDate,
    SurfaceLost,
    /// No image within the timeout (`VK_TIMEOUT` / `VK_NOT_READY`).
    NotReady,
}

/// Result of `vkQueuePresentKHR`, reduced to what the policy needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentStatus {
    Success,
    Suboptimal,
    OutOfDate,
    SurfaceLost,
}

/// Why the controller stopped the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// Recreation failed [`MAX_RECOVERY_FAILURES`] times in a row, or
    /// [`MAX_UNPRESENTED_RECOVERIES`] recoveries brought no presented frame.
    RecoveryExhausted,
    /// The backend reported a fatal error (device lost, out of memory).
    Backend,
}

/// Presentation state machine.
#[derive(Debug)]
pub(crate) struct PresentationController {
    state: PresentationState,
    /// Size the owner asked for (physical pixels).
    requested: SurfaceSize,
    /// Size of the live swapchain, if one exists.
    swapchain: Option<SurfaceSize>,
    /// The swapchain must be recreated before the next acquire.
    stale: bool,
    /// The surface itself must be recreated first.
    surface_lost: bool,
    recovery_failures: u32,
    unpresented_recoveries: u32,
    stop_reason: Option<StopReason>,
}

impl PresentationController {
    /// A controller for a surface of `requested` size with no swapchain yet.
    pub(crate) fn new(requested: SurfaceSize) -> Self {
        Self {
            state: if requested.is_empty() {
                PresentationState::Suspended
            } else {
                PresentationState::Recovering
            },
            requested,
            swapchain: None,
            stale: true,
            surface_lost: false,
            recovery_failures: 0,
            unpresented_recoveries: 0,
            stop_reason: None,
        }
    }

    pub(crate) fn state(&self) -> PresentationState {
        self.state
    }

    pub(crate) fn requested(&self) -> SurfaceSize {
        self.requested
    }

    pub(crate) fn stop_reason(&self) -> Option<StopReason> {
        self.stop_reason
    }

    /// The owner reports a new drawable size. Zero suspends; a size different
    /// from the swapchain's schedules recreation. Repeated calls are harmless
    /// and nothing is done on the GPU until the next frame.
    pub(crate) fn resize(&mut self, size: SurfaceSize) {
        if self.state == PresentationState::Fatal {
            return;
        }
        self.requested = size;
        if size.is_empty() {
            self.state = PresentationState::Suspended;
            return;
        }
        if self.swapchain != Some(size) {
            self.stale = true;
        }
        self.state = self.drawable_state();
    }

    /// What to do before rendering the next frame. `None` once stopped.
    pub(crate) fn plan(&self) -> Option<FramePlan> {
        match self.state {
            PresentationState::Fatal => None,
            PresentationState::Suspended => Some(FramePlan::Suspend),
            PresentationState::Recovering if self.surface_lost => Some(FramePlan::RecreateSurface),
            PresentationState::Recovering => Some(FramePlan::RecreateSwapchain),
            PresentationState::Ready => Some(FramePlan::Acquire),
        }
    }

    /// The surface was recreated; the swapchain still has to be.
    pub(crate) fn surface_recreated(&mut self) {
        self.surface_lost = false;
        self.stale = true;
    }

    /// A swapchain of `actual` size now exists. An empty `actual` means the
    /// surface itself reports a zero extent (e.g. minimized on some window
    /// systems): presentation suspends until the next non-zero resize.
    pub(crate) fn swapchain_recreated(&mut self, actual: SurfaceSize) {
        if self.state == PresentationState::Fatal {
            return;
        }
        self.recovery_failures = 0;
        if actual.is_empty() {
            self.swapchain = None;
            self.stale = true;
            self.state = PresentationState::Suspended;
            return;
        }
        self.swapchain = Some(actual);
        // The surface may dictate a size other than the requested one; the
        // swapchain follows the surface, so it is not stale.
        self.stale = false;
        self.state = self.drawable_state();
    }

    /// Recreating the swapchain or surface failed with a recoverable error.
    /// Returns `false` once the bounded number of attempts is exhausted.
    pub(crate) fn recovery_failed(&mut self) -> bool {
        self.recovery_failures = self.recovery_failures.saturating_add(1);
        if self.recovery_failures >= MAX_RECOVERY_FAILURES {
            self.stop(StopReason::RecoveryExhausted);
            return false;
        }
        true
    }

    /// Reports the acquire result. Returns whether to record, submit and
    /// present this frame, and `false` once recovery is exhausted too.
    pub(crate) fn acquired(&mut self, status: AcquireStatus) -> bool {
        match status {
            AcquireStatus::Success => true,
            // Render and present this image, recreate afterwards.
            AcquireStatus::Suboptimal => {
                self.stale = true;
                true
            }
            AcquireStatus::OutOfDate => {
                self.schedule_recovery(false);
                false
            }
            AcquireStatus::SurfaceLost => {
                self.schedule_recovery(true);
                false
            }
            AcquireStatus::NotReady => false,
        }
    }

    /// Reports the present result.
    pub(crate) fn presented(&mut self, status: PresentStatus) {
        if self.state == PresentationState::Fatal {
            return;
        }
        match status {
            PresentStatus::Success => {
                self.unpresented_recoveries = 0;
                self.state = self.drawable_state();
            }
            PresentStatus::Suboptimal => {
                self.unpresented_recoveries = 0;
                self.stale = true;
                self.state = self.drawable_state();
            }
            PresentStatus::OutOfDate => self.schedule_recovery(false),
            PresentStatus::SurfaceLost => self.schedule_recovery(true),
        }
    }

    /// The backend hit a fatal error.
    pub(crate) fn fail(&mut self) {
        self.stop(StopReason::Backend);
    }

    fn schedule_recovery(&mut self, surface_lost: bool) {
        if self.state == PresentationState::Fatal {
            return;
        }
        self.stale = true;
        self.surface_lost |= surface_lost;
        self.unpresented_recoveries = self.unpresented_recoveries.saturating_add(1);
        if self.unpresented_recoveries > MAX_UNPRESENTED_RECOVERIES {
            self.stop(StopReason::RecoveryExhausted);
            return;
        }
        self.state = self.drawable_state();
    }

    fn stop(&mut self, reason: StopReason) {
        if self.stop_reason.is_none() {
            self.stop_reason = Some(reason);
        }
        self.state = PresentationState::Fatal;
    }

    /// State for a non-fatal controller given the current flags.
    fn drawable_state(&self) -> PresentationState {
        if self.requested.is_empty() {
            PresentationState::Suspended
        } else if self.stale || self.surface_lost || self.swapchain.is_none() {
            PresentationState::Recovering
        } else {
            PresentationState::Ready
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: SurfaceSize = SurfaceSize::new(1280, 720);
    const ZERO: SurfaceSize = SurfaceSize::new(0, 0);

    /// Drives one frame the way the backend does; returns the plan executed
    /// and whether an acquire happened.
    fn frame(
        controller: &mut PresentationController,
        acquire: AcquireStatus,
        present: PresentStatus,
    ) -> (FramePlan, bool) {
        let Some(plan) = controller.plan() else {
            return (FramePlan::Suspend, false);
        };
        match plan {
            FramePlan::Suspend => return (plan, false),
            FramePlan::RecreateSurface => {
                controller.surface_recreated();
                controller.swapchain_recreated(controller.requested());
            }
            FramePlan::RecreateSwapchain => controller.swapchain_recreated(controller.requested()),
            FramePlan::Acquire => {}
        }
        if controller.plan() != Some(FramePlan::Acquire) {
            return (plan, false);
        }
        if controller.acquired(acquire) {
            controller.presented(present);
        }
        (plan, true)
    }

    fn ready(size: SurfaceSize) -> PresentationController {
        let mut controller = PresentationController::new(size);
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
        controller.swapchain_recreated(size);
        assert_eq!(controller.state(), PresentationState::Ready);
        controller
    }

    #[test]
    fn new_controller_creates_swapchain_or_suspends() {
        assert_eq!(
            PresentationController::new(HD).state(),
            PresentationState::Recovering
        );
        let suspended = PresentationController::new(ZERO);
        assert_eq!(suspended.state(), PresentationState::Suspended);
        assert_eq!(suspended.plan(), Some(FramePlan::Suspend));
    }

    /// FND-T06: zero size suspends, no acquire happens while suspended, and a
    /// valid size resumes through one recreation.
    #[test]
    fn zero_size_suspends_without_acquire_and_valid_size_resumes() {
        let mut controller = ready(HD);
        for size in [SurfaceSize::new(0, 720), SurfaceSize::new(1280, 0), ZERO] {
            controller.resize(size);
            assert_eq!(controller.state(), PresentationState::Suspended);
            for _ in 0..5 {
                let (plan, acquired) = frame(
                    &mut controller,
                    AcquireStatus::Success,
                    PresentStatus::Success,
                );
                assert_eq!(plan, FramePlan::Suspend);
                assert!(!acquired, "acquired while suspended");
            }
        }
        controller.resize(SurfaceSize::new(800, 600));
        assert_eq!(controller.state(), PresentationState::Recovering);
        let (plan, acquired) = frame(
            &mut controller,
            AcquireStatus::Success,
            PresentStatus::Success,
        );
        assert_eq!(plan, FramePlan::RecreateSwapchain);
        assert!(acquired);
        assert_eq!(controller.state(), PresentationState::Ready);
    }

    /// FND-T06: restoring the size the swapchain already has needs no
    /// recreation.
    #[test]
    fn restoring_the_same_size_after_suspension_reuses_the_swapchain() {
        let mut controller = ready(HD);
        controller.resize(ZERO);
        controller.resize(HD);
        assert_eq!(controller.plan(), Some(FramePlan::Acquire));
    }

    /// FND-T06: a burst of resize events costs one recreation at the next
    /// frame, for the last size.
    #[test]
    fn repeated_resizes_coalesce_into_one_recreation() {
        let mut controller = ready(HD);
        for width in 100..200 {
            controller.resize(SurfaceSize::new(width, 300));
        }
        controller.resize(SurfaceSize::new(640, 480));
        assert_eq!(controller.requested(), SurfaceSize::new(640, 480));
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
        controller.swapchain_recreated(SurfaceSize::new(640, 480));
        assert_eq!(controller.plan(), Some(FramePlan::Acquire));
        controller.resize(SurfaceSize::new(640, 480));
        assert_eq!(controller.plan(), Some(FramePlan::Acquire));
    }

    /// FND-T06: a surface reporting a zero extent (minimized on some window
    /// systems) suspends until the owner reports a size again.
    #[test]
    fn zero_surface_extent_suspends_until_next_resize() {
        let mut controller = ready(HD);
        controller.resize(SurfaceSize::new(1000, 700));
        controller.swapchain_recreated(ZERO);
        assert_eq!(controller.state(), PresentationState::Suspended);
        assert_eq!(controller.plan(), Some(FramePlan::Suspend));
        controller.resize(SurfaceSize::new(1000, 700));
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
    }

    #[test]
    fn surface_dictated_size_is_not_stale() {
        let mut controller = PresentationController::new(HD);
        controller.swapchain_recreated(SurfaceSize::new(1270, 710));
        assert_eq!(controller.plan(), Some(FramePlan::Acquire));
    }

    /// FND-T07: out-of-date acquire skips the frame and recreates next time.
    #[test]
    fn out_of_date_acquire_recreates_on_next_frame() {
        let mut controller = ready(HD);
        assert!(!controller.acquired(AcquireStatus::OutOfDate));
        assert_eq!(controller.state(), PresentationState::Recovering);
        let (plan, acquired) = frame(
            &mut controller,
            AcquireStatus::Success,
            PresentStatus::Success,
        );
        assert_eq!(plan, FramePlan::RecreateSwapchain);
        assert!(acquired);
        assert_eq!(controller.state(), PresentationState::Ready);
    }

    /// FND-T07: suboptimal images are still presented, then recreated.
    #[test]
    fn suboptimal_is_presented_then_recreated() {
        let mut controller = ready(HD);
        assert!(controller.acquired(AcquireStatus::Suboptimal));
        controller.presented(PresentStatus::Success);
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));

        let mut controller = ready(HD);
        assert!(controller.acquired(AcquireStatus::Success));
        controller.presented(PresentStatus::Suboptimal);
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
    }

    /// FND-T07: out-of-date on present schedules recreation.
    #[test]
    fn out_of_date_present_recreates() {
        let mut controller = ready(HD);
        assert!(controller.acquired(AcquireStatus::Success));
        controller.presented(PresentStatus::OutOfDate);
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
    }

    /// FND-T07: a lost surface is recreated before the swapchain.
    #[test]
    fn surface_loss_recreates_surface_then_swapchain() {
        let mut controller = ready(HD);
        assert!(!controller.acquired(AcquireStatus::SurfaceLost));
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSurface));
        controller.surface_recreated();
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
        controller.swapchain_recreated(HD);
        assert_eq!(controller.plan(), Some(FramePlan::Acquire));

        let mut controller = ready(HD);
        controller.acquired(AcquireStatus::Success);
        controller.presented(PresentStatus::SurfaceLost);
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSurface));
    }

    #[test]
    fn surface_loss_while_suspended_is_recovered_after_resume() {
        let mut controller = ready(HD);
        controller.acquired(AcquireStatus::SurfaceLost);
        controller.resize(ZERO);
        assert_eq!(controller.plan(), Some(FramePlan::Suspend));
        controller.resize(HD);
        assert_eq!(controller.plan(), Some(FramePlan::RecreateSurface));
    }

    #[test]
    fn not_ready_skips_without_recovery() {
        let mut controller = ready(HD);
        assert!(!controller.acquired(AcquireStatus::NotReady));
        assert_eq!(controller.state(), PresentationState::Ready);
    }

    /// FND-T07: recreation failures are bounded and end in a fatal stop.
    #[test]
    fn recreation_failures_are_bounded() {
        let mut controller = ready(HD);
        controller.acquired(AcquireStatus::OutOfDate);
        for attempt in 1..MAX_RECOVERY_FAILURES {
            assert!(
                controller.recovery_failed(),
                "attempt {attempt} should be retried"
            );
            assert_eq!(controller.plan(), Some(FramePlan::RecreateSwapchain));
        }
        assert!(!controller.recovery_failed());
        assert_eq!(controller.state(), PresentationState::Fatal);
        assert_eq!(controller.plan(), None);
        assert_eq!(
            controller.stop_reason(),
            Some(StopReason::RecoveryExhausted)
        );
    }

    #[test]
    fn a_successful_recreation_resets_the_failure_count() {
        let mut controller = ready(HD);
        controller.acquired(AcquireStatus::OutOfDate);
        for _ in 0..10 {
            assert!(controller.recovery_failed());
            controller.swapchain_recreated(HD);
            controller.acquired(AcquireStatus::Success);
            controller.presented(PresentStatus::Success);
            controller.acquired(AcquireStatus::OutOfDate);
        }
        assert_ne!(controller.state(), PresentationState::Fatal);
    }

    /// FND-T07: a swapchain that outdates after every recreation without ever
    /// presenting stops after a bounded number of frames.
    #[test]
    fn endless_out_of_date_without_presenting_is_bounded() {
        let mut controller = ready(HD);
        let mut frames = 0;
        while controller.plan().is_some() {
            frame(
                &mut controller,
                AcquireStatus::OutOfDate,
                PresentStatus::Success,
            );
            frames += 1;
            assert!(
                frames <= 2 * MAX_UNPRESENTED_RECOVERIES + 2,
                "recovery is unbounded"
            );
        }
        assert_eq!(
            controller.stop_reason(),
            Some(StopReason::RecoveryExhausted)
        );
    }

    #[test]
    fn presenting_resets_the_unpresented_count() {
        let mut controller = ready(HD);
        for _ in 0..(3 * MAX_UNPRESENTED_RECOVERIES) {
            controller.acquired(AcquireStatus::OutOfDate);
            controller.swapchain_recreated(HD);
            controller.acquired(AcquireStatus::Success);
            controller.presented(PresentStatus::Success);
        }
        assert_eq!(controller.state(), PresentationState::Ready);
    }

    /// FND-T07: fatal backend errors stop the controller for good.
    #[test]
    fn fatal_stops_permanently() {
        let mut controller = ready(HD);
        controller.fail();
        assert_eq!(controller.plan(), None);
        controller.resize(SurfaceSize::new(10, 10));
        controller.swapchain_recreated(SurfaceSize::new(10, 10));
        controller.presented(PresentStatus::Success);
        controller.acquired(AcquireStatus::OutOfDate);
        assert_eq!(controller.state(), PresentationState::Fatal);
        assert_eq!(controller.stop_reason(), Some(StopReason::Backend));
    }
}
