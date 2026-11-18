//! Pure swapchain-matched `D32_SFLOAT` reversed-Z depth-attachment lifecycle
//! policy (`F3D-FR-07`, `F3D-INV-07`, `F3D-INV-09`, `F3D-INV-14`, `F3D-T09`).
//!
//! Always compiled (`default-features = false`) and `#![forbid(unsafe_code)]`.
//! Tracks when the Vulkan backend allocates, keeps, or retires its `DEVICE_LOCAL`
//! `VK_FORMAT_D32_SFLOAT` depth attachment alongside `PresentationController`,
//! guaranteeing:
//! - a depth attachment of matching physical pixel extent exists whenever the
//!   swapchain is `Ready`;
//! - **no `0x0` depth attachment** is ever requested or allocated while
//!   `Suspended`;
//! - extent synchronization is idempotent (unchanged swapchain extent reuses the
//!   live depth attachment; changed extent retires the old attachment and creates
//!   a new one);
//! - surface loss, failed swapchain recreation, and renderer shutdown retire the
//!   depth attachment cleanly without touching simulation time or `Tick`.

#![forbid(unsafe_code)]

use crate::types::SurfaceSize;

/// Reversed-Z far-plane depth clear value (`far -> 0.0`, `near -> 1.0`; `SPN-INV-053`).
pub const REVERSED_Z_CLEAR_DEPTH: f32 = 0.0;

/// Action the Vulkan backend must take to synchronize its swapchain depth
/// attachment with the current swapchain state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) enum DepthPlan {
    /// Existing state already matches (`None` stays `None`, or `Some(extent)` stays `Some(extent)`).
    Keep,
    /// Allocate a fresh `D32_SFLOAT` attachment at `extent` (guaranteed non-empty).
    Allocate { extent: SurfaceSize },
    /// Retire the existing attachment at `old_extent` and allocate a new one at `new_extent` (non-empty).
    Recreate {
        old_extent: SurfaceSize,
        new_extent: SurfaceSize,
    },
    /// Retire the existing attachment at `old_extent` without allocating a replacement (`0x0` or stopped).
    Retire { old_extent: SurfaceSize },
}

/// Pure controller tracking the live swapchain-matched `D32_SFLOAT` depth extent.
#[derive(Debug, Default)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct DepthAttachmentController {
    live_extent: Option<SurfaceSize>,
    generation: u64,
    allocations: u64,
    retirements: u64,
}

#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
impl DepthAttachmentController {
    /// Starts with no depth attachment allocated (`live_extent == None`).
    pub(crate) const fn new() -> Self {
        Self {
            live_extent: None,
            generation: 0,
            allocations: 0,
            retirements: 0,
        }
    }

    /// Physical pixel extent of the currently live depth attachment, if any.
    /// Guaranteed never to be empty (`width > 0 && height > 0`).
    #[cfg(test)]
    pub(crate) const fn live_extent(&self) -> Option<SurfaceSize> {
        self.live_extent
    }

    /// Monotonic generation incremented each time a new depth attachment is allocated.
    #[cfg(test)]
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    /// Total depth attachment allocations committed so far.
    #[cfg(test)]
    pub(crate) const fn allocation_count(&self) -> u64 {
        self.allocations
    }

    /// Total depth attachment retirements committed so far.
    #[cfg(test)]
    pub(crate) const fn retirement_count(&self) -> u64 {
        self.retirements
    }

    /// Computes what the backend must do for a swapchain of `swapchain_extent`
    /// (`None` or empty `0x0` means no drawable swapchain exists).
    ///
    /// Does **not** mutate `self` until [`Self::commit_allocated`] or
    /// [`Self::commit_retired`] is called, so a failed GPU allocation leaves
    /// controller state consistent.
    pub(crate) const fn plan_for_swapchain(
        &self,
        swapchain_extent: Option<SurfaceSize>,
    ) -> DepthPlan {
        let valid_target = match swapchain_extent {
            Some(size) if !size.is_empty() => Some(size),
            _ => None,
        };
        match (self.live_extent, valid_target) {
            (None, None) => DepthPlan::Keep,
            (None, Some(extent)) => DepthPlan::Allocate { extent },
            (Some(old), Some(new)) if old.width == new.width && old.height == new.height => {
                DepthPlan::Keep
            }
            (Some(old_extent), Some(new_extent)) => DepthPlan::Recreate {
                old_extent,
                new_extent,
            },
            (Some(old_extent), None) => DepthPlan::Retire { old_extent },
        }
    }

    /// Records that any previously live depth attachment has been retired/destroyed.
    pub(crate) fn commit_retired(&mut self) -> Option<SurfaceSize> {
        let old = self.live_extent.take()?;
        self.retirements = self.retirements.saturating_add(1);
        Some(old)
    }

    /// Records that a non-empty `D32_SFLOAT` depth attachment of `extent` is now live.
    /// Returns `false` and refuses to record if `extent.is_empty()`.
    pub(crate) fn commit_allocated(&mut self, extent: SurfaceSize) -> bool {
        if extent.is_empty() {
            return false;
        }
        if self.live_extent.take().is_some() {
            self.retirements = self.retirements.saturating_add(1);
        }
        self.live_extent = Some(extent);
        self.generation = self.generation.saturating_add(1);
        self.allocations = self.allocations.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::{
        AllocationRequest, GpuMemoryBudget, MemoryDomain, PureGpuAllocator, Suballocation,
        SuballocationKind,
    };
    use crate::presentation::{
        AcquireStatus, FramePlan, MAX_RECOVERY_FAILURES, PresentStatus, PresentationController,
        StopReason,
    };
    use crate::types::PresentationState;

    /// Simulates the headless pairing of `PresentationController`,
    /// `DepthAttachmentController`, and `PureGpuAllocator` across frames while
    /// tracking an independent monotonic simulation `tick: u64` (`F3D-INV-09`).
    struct HeadlessPresentationDepthHarness {
        presentation: PresentationController,
        depth: DepthAttachmentController,
        allocator: PureGpuAllocator,
        live_depth_alloc: Option<Suballocation>,
        timeline: u64,
        sim_tick: u64,
        zero_size_alloc_attempts: u64,
    }

    impl HeadlessPresentationDepthHarness {
        fn new(initial: SurfaceSize) -> Self {
            let mut harness = Self {
                presentation: PresentationController::new(initial),
                depth: DepthAttachmentController::new(),
                allocator: PureGpuAllocator::new(GpuMemoryBudget::default(), 4096).unwrap(),
                live_depth_alloc: None,
                timeline: 0,
                sim_tick: 0,
                zero_size_alloc_attempts: 0,
            };
            if harness.presentation.plan() == Some(FramePlan::RecreateSwapchain) {
                harness.recreate_swapchain_ok(initial);
            }
            harness
        }

        fn step_simulation_tick(&mut self) {
            self.sim_tick += 1;
        }

        fn apply_depth_plan(&mut self, plan: DepthPlan) {
            match plan {
                DepthPlan::Keep => {}
                DepthPlan::Allocate { extent } => {
                    if extent.is_empty() {
                        self.zero_size_alloc_attempts += 1;
                        return;
                    }
                    let bytes = u64::from(extent.width) * u64::from(extent.height) * 4;
                    let sub = self
                        .allocator
                        .allocate(AllocationRequest {
                            domain: MemoryDomain::DeviceLocal,
                            memory_type_index: 0,
                            size: bytes,
                            alignment: 4096,
                            kind: SuballocationKind::Optimal,
                        })
                        .unwrap();
                    self.live_depth_alloc = Some(sub);
                    assert!(self.depth.commit_allocated(extent));
                }
                DepthPlan::Recreate { new_extent, .. } => {
                    if let Some(old) = self.live_depth_alloc.take() {
                        self.allocator.retire(old, self.timeline).unwrap();
                        let _ = self.allocator.reclaim_completed(self.timeline);
                        self.depth.commit_retired();
                    }
                    self.apply_depth_plan(DepthPlan::Allocate { extent: new_extent });
                }
                DepthPlan::Retire { .. } => {
                    if let Some(old) = self.live_depth_alloc.take() {
                        self.allocator.retire(old, self.timeline).unwrap();
                        let _ = self.allocator.reclaim_completed(self.timeline);
                        self.depth.commit_retired();
                    }
                }
            }
        }

        fn recreate_swapchain_ok(&mut self, actual: SurfaceSize) {
            self.presentation.swapchain_recreated(actual);
            let plan = self.depth.plan_for_swapchain(Some(actual));
            self.apply_depth_plan(plan);
        }

        fn shutdown(&mut self) {
            let plan = self.depth.plan_for_swapchain(None);
            self.apply_depth_plan(plan);
            let _ = self.allocator.reclaim_completed(u64::MAX);
        }
    }

    /// F3D-T09: full lifecycle verification across
    /// `Ready(w1, h1) -> 0x0 Suspended -> Ready(w1, h1) -> Ready(w2, h2) -> Recovering -> Stopped`.
    #[test]
    fn f3d_t09_depth_lifecycle_ready_suspend_idempotent_resize_recovering_and_stopped() {
        let w1_h1 = SurfaceSize::new(1280, 720);
        let w2_h2 = SurfaceSize::new(1920, 1080);
        let zero = SurfaceSize::new(0, 0);

        // 0. Starting at `0x0` never allocates a depth attachment.
        {
            let start_zero = HeadlessPresentationDepthHarness::new(zero);
            assert_eq!(
                start_zero.presentation.state(),
                PresentationState::Suspended
            );
            assert_eq!(start_zero.depth.live_extent(), None);
            assert_eq!(start_zero.depth.allocation_count(), 0);
            assert_eq!(start_zero.allocator.stats().live_allocations, 0);
        }

        // 1. Start at `Ready(w1, h1)`: allocates `1280x720` D32_SFLOAT once.
        let mut h = HeadlessPresentationDepthHarness::new(w1_h1);
        assert_eq!(h.presentation.state(), PresentationState::Ready);
        assert_eq!(h.depth.live_extent(), Some(w1_h1));
        assert_eq!(h.depth.allocation_count(), 1);
        assert_eq!(h.depth.generation(), 1);
        h.timeline += 1;
        h.step_simulation_tick();
        assert_eq!(h.sim_tick, 1);

        // Repeatedly syncing at the same `(w1, h1)` extent is idempotent (`DepthPlan::Keep`).
        assert_eq!(h.depth.plan_for_swapchain(Some(w1_h1)), DepthPlan::Keep);

        // 2. `Ready(w1, h1) -> 0x0 Suspended`: suspends presentation, performs ZERO `0x0`
        //    depth allocations, and advances simulation `Tick` monotonically!
        for empty in [SurfaceSize::new(0, 720), SurfaceSize::new(1280, 0), zero] {
            h.presentation.resize(empty);
            assert_eq!(h.presentation.state(), PresentationState::Suspended);
            assert_eq!(h.presentation.plan(), Some(FramePlan::Suspend));
            h.step_simulation_tick();
        }
        assert_eq!(h.sim_tick, 4);
        assert_eq!(h.zero_size_alloc_attempts, 0);
        assert!(!h.depth.commit_allocated(zero));
        assert_eq!(h.depth.allocation_count(), 1);

        // 3. `0x0 Suspended -> Ready(w1, h1)`: resumes to the same extent idempotently
        //    (`FramePlan::Acquire` + `DepthPlan::Keep`, zero extra GPU allocations!).
        h.presentation.resize(w1_h1);
        assert_eq!(h.presentation.state(), PresentationState::Ready);
        assert_eq!(h.presentation.plan(), Some(FramePlan::Acquire));
        assert_eq!(h.depth.plan_for_swapchain(Some(w1_h1)), DepthPlan::Keep);
        assert_eq!(h.depth.live_extent(), Some(w1_h1));
        assert_eq!(h.depth.allocation_count(), 1);
        h.timeline += 1;
        h.step_simulation_tick();
        assert_eq!(h.sim_tick, 5);

        // Also verify when the surface itself dictates `0x0` during swapchain creation
        // (`SwapchainBuild::ZeroExtent`): depth retires cleanly and never allocates `0x0`,
        // then allocates once when `(w1, h1)` returns.
        h.recreate_swapchain_ok(zero);
        assert_eq!(h.presentation.state(), PresentationState::Suspended);
        assert_eq!(h.depth.live_extent(), None);
        assert_eq!(h.depth.retirement_count(), 1);
        assert_eq!(h.allocator.stats().live_allocations, 0);

        h.presentation.resize(w1_h1);
        assert_eq!(h.presentation.plan(), Some(FramePlan::RecreateSwapchain));
        h.recreate_swapchain_ok(w1_h1);
        assert_eq!(h.presentation.state(), PresentationState::Ready);
        assert_eq!(h.depth.live_extent(), Some(w1_h1));
        assert_eq!(h.depth.allocation_count(), 2);

        // 4. `Ready(w1, h1) -> Ready(w2, h2)`: coalesces burst resizes into a single
        //    `DepthPlan::Recreate` retiring `(w1, h1)` and allocating `(w2, h2)` in the
        //    same reusable `16 MiB` `DEVICE_LOCAL` block!
        h.presentation.resize(SurfaceSize::new(1600, 900));
        h.presentation.resize(w2_h2);
        h.presentation.resize(w2_h2); // duplicate resize is idempotent
        assert_eq!(h.presentation.state(), PresentationState::Recovering);
        assert_eq!(h.presentation.plan(), Some(FramePlan::RecreateSwapchain));
        let expected_plan = DepthPlan::Recreate {
            old_extent: w1_h1,
            new_extent: w2_h2,
        };
        assert_eq!(h.depth.plan_for_swapchain(Some(w2_h2)), expected_plan);
        h.recreate_swapchain_ok(w2_h2);
        assert_eq!(h.presentation.state(), PresentationState::Ready);
        assert_eq!(h.depth.live_extent(), Some(w2_h2));
        assert_eq!(h.depth.allocation_count(), 3);
        assert_eq!(h.depth.retirement_count(), 2);
        assert_eq!(h.allocator.stats().block_count, 1);
        assert_eq!(h.allocator.stats().live_allocations, 1);
        // Re-running `plan_for_swapchain(Some(w2_h2))` at steady state is `Keep`.
        assert_eq!(h.depth.plan_for_swapchain(Some(w2_h2)), DepthPlan::Keep);
        h.step_simulation_tick();
        assert_eq!(h.sim_tick, 6);

        // 5. `Ready(w2, h2) -> Recovering` (`SurfaceLost` / `OutOfDate`):
        //    First verify successful surface-loss recovery recreates cleanly,
        //    then verify bounded recovery exhaustion transitions to `Fatal` (`Stopped`)
        //    and retires the depth attachment with zero leaked suballocations!
        assert!(!h.presentation.acquired(AcquireStatus::SurfaceLost));
        assert_eq!(h.presentation.state(), PresentationState::Recovering);
        assert_eq!(h.presentation.plan(), Some(FramePlan::RecreateSurface));
        // Tear down depth with old swapchain during surface recreate.
        let retire_plan = h.depth.plan_for_swapchain(None);
        assert_eq!(retire_plan, DepthPlan::Retire { old_extent: w2_h2 });
        h.apply_depth_plan(retire_plan);
        assert_eq!(h.depth.live_extent(), None);
        h.presentation.surface_recreated();
        h.recreate_swapchain_ok(w2_h2);
        assert!(h.presentation.acquired(AcquireStatus::Success));
        h.presentation.presented(PresentStatus::Success);
        assert_eq!(h.presentation.state(), PresentationState::Ready);
        assert_eq!(h.depth.live_extent(), Some(w2_h2));
        h.step_simulation_tick();
        assert_eq!(h.sim_tick, 7);

        // Now trigger `OutOfDate` and exhaust `MAX_RECOVERY_FAILURES` -> `Fatal` / `Stopped`.
        assert!(!h.presentation.acquired(AcquireStatus::OutOfDate));
        assert_eq!(h.presentation.state(), PresentationState::Recovering);
        // Old swapchain & depth retired when swapchain creation fails.
        let retire_on_fail = h.depth.plan_for_swapchain(None);
        h.apply_depth_plan(retire_on_fail);
        for _ in 1..MAX_RECOVERY_FAILURES {
            assert!(h.presentation.recovery_failed());
            h.step_simulation_tick();
        }
        assert!(!h.presentation.recovery_failed());
        assert_eq!(h.presentation.state(), PresentationState::Fatal);
        assert_eq!(
            h.presentation.stop_reason(),
            Some(StopReason::RecoveryExhausted)
        );
        assert_eq!(h.presentation.plan(), None);

        // 6. `Stopped` / Shutdown: depth is retired, zero `0x0` allocations occurred,
        //    zero suballocations leaked, and simulation `Tick` stayed strictly monotonic!
        h.shutdown();
        assert_eq!(h.depth.live_extent(), None);
        assert_eq!(h.zero_size_alloc_attempts, 0);
        assert_eq!(h.allocator.leak_count(), 0);
        h.allocator.check_no_leaks().unwrap();
        assert_eq!(h.sim_tick, 7 + u64::from(MAX_RECOVERY_FAILURES - 1));
    }
}
