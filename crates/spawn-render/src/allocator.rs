//! In-house GPU memory block suballocator and timeline-semaphore retirement
//! tracker (`ADR-0004` decision 3, `F3D-FR-06`, `F3D-INV-13`, `F3D-T08`).
//!
//! Always compiled (`default-features = false`), GPU-free, `#![forbid(unsafe_code)]`,
//! and `#![deny(clippy::indexing_slicing)]`. The Vulkan RHI pairs each [`BlockId`]
//! with a `VkDeviceMemory` allocation (and persistent host mapping for
//! `HOST_VISIBLE | HOST_COHERENT` blocks) inside `src/vulkan/memory.rs`.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use crate::error::{RenderError, RenderErrorKind};

/// Default `DEVICE_LOCAL` budget ceiling (256 MiB; proposal target well within
/// RX 460's ~2.5 GB full-game ceiling in *Proposed* ADR-0020).
pub const DEFAULT_DEVICE_LOCAL_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// Default `HOST_VISIBLE | HOST_COHERENT` staging/readback budget ceiling (64 MiB).
pub const DEFAULT_HOST_VISIBLE_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// Default `DEVICE_LOCAL` suballocation block size (16 MiB).
pub const DEFAULT_DEVICE_LOCAL_BLOCK_BYTES: u64 = 16 * 1024 * 1024;

/// Default `HOST_VISIBLE | HOST_COHERENT` suballocation block size (4 MiB).
pub const DEFAULT_HOST_VISIBLE_BLOCK_BYTES: u64 = 4 * 1024 * 1024;

/// Configurable GPU memory budget and default block sizes for a `Renderer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuMemoryBudget {
    /// Maximum total bytes across active `DEVICE_LOCAL` blocks.
    pub device_local_bytes: u64,
    /// Maximum total bytes across active `HOST_VISIBLE | HOST_COHERENT` blocks.
    pub host_visible_bytes: u64,
    /// Target block size for pooled `DEVICE_LOCAL` suballocations.
    pub device_local_block_bytes: u64,
    /// Target block size for pooled `HOST_VISIBLE | HOST_COHERENT` suballocations.
    pub host_visible_block_bytes: u64,
}

impl Default for GpuMemoryBudget {
    fn default() -> Self {
        Self {
            device_local_bytes: DEFAULT_DEVICE_LOCAL_BUDGET_BYTES,
            host_visible_bytes: DEFAULT_HOST_VISIBLE_BUDGET_BYTES,
            device_local_block_bytes: DEFAULT_DEVICE_LOCAL_BLOCK_BYTES,
            host_visible_block_bytes: DEFAULT_HOST_VISIBLE_BLOCK_BYTES,
        }
    }
}

impl GpuMemoryBudget {
    /// Validates that all budget caps and block sizes are non-zero.
    #[track_caller]
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.device_local_bytes == 0 {
            return Err(RenderError::new(RenderErrorKind::GpuMemoryExhausted)
                .with_detail("field `device_local_bytes`: budget must be greater than 0"));
        }
        if self.host_visible_bytes == 0 {
            return Err(RenderError::new(RenderErrorKind::GpuMemoryExhausted)
                .with_detail("field `host_visible_bytes`: budget must be greater than 0"));
        }
        if self.device_local_block_bytes == 0 {
            return Err(
                RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(
                    "field `device_local_block_bytes`: block size must be greater than 0",
                ),
            );
        }
        if self.host_visible_block_bytes == 0 {
            return Err(
                RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(
                    "field `host_visible_block_bytes`: block size must be greater than 0",
                ),
            );
        }
        Ok(())
    }

    pub(crate) const fn domain_limit(self, domain: MemoryDomain) -> u64 {
        match domain {
            MemoryDomain::DeviceLocal => self.device_local_bytes,
            MemoryDomain::HostVisible => self.host_visible_bytes,
        }
    }

    pub(crate) const fn domain_block_size(self, domain: MemoryDomain) -> u64 {
        match domain {
            MemoryDomain::DeviceLocal => self.device_local_block_bytes,
            MemoryDomain::HostVisible => self.host_visible_block_bytes,
        }
    }
}

/// Snapshot of live, retired, and peak GPU block allocator usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GpuAllocatorStats {
    /// Active `VkDeviceMemory` blocks (pooled + dedicated).
    pub block_count: u32,
    /// Active dedicated oversized blocks.
    pub dedicated_block_count: u32,
    /// Total bytes across active `DEVICE_LOCAL` blocks.
    pub device_local_block_bytes: u64,
    /// Total bytes across active `HOST_VISIBLE | HOST_COHERENT` blocks.
    pub host_visible_block_bytes: u64,
    /// Live (non-retired) suballocations currently checked out.
    pub live_allocations: u32,
    /// Bytes occupied by live (non-retired) suballocations.
    pub live_allocated_bytes: u64,
    /// Retired suballocations awaiting GPU timeline semaphore completion.
    pub retired_allocations: u32,
    /// Bytes occupied by retired suballocations awaiting GPU completion.
    pub retired_bytes: u64,
    /// Peak total block bytes (`device_local + host_visible`) allocated at once.
    pub peak_block_bytes: u64,
    /// Peak suballocated bytes (`live + retired`) occupied at once.
    pub peak_allocated_bytes: u64,
}

/// Memory domain of a GPU block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) enum MemoryDomain {
    /// GPU-local memory (`VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT`).
    DeviceLocal,
    /// Persistently mapped host-coherent memory (`HOST_VISIBLE | HOST_COHERENT`).
    HostVisible,
}

impl MemoryDomain {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DeviceLocal => "DEVICE_LOCAL",
            Self::HostVisible => "HOST_VISIBLE",
        }
    }
}

/// Vulkan resource linearity class used for `bufferImageGranularity` isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) enum SuballocationKind {
    /// Linear memory (`VkBuffer` or linear-tiling `VkImage`).
    Linear,
    /// Optimal-tiling `VkImage` (`VK_IMAGE_TILING_OPTIMAL`).
    Optimal,
}

/// Unique identifier of an allocated memory block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct BlockId(pub(crate) u64);

/// Unique identifier of a suballocation within a `PureGpuAllocator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct AllocationId(pub(crate) u64);

/// Request parameters for suballocating GPU memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct AllocationRequest {
    pub domain: MemoryDomain,
    pub memory_type_index: u32,
    pub size: u64,
    pub alignment: u64,
    pub kind: SuballocationKind,
}

/// Checked-out suballocation slice within a [`BlockId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct Suballocation {
    pub id: AllocationId,
    pub block_id: BlockId,
    pub domain: MemoryDomain,
    pub memory_type_index: u32,
    pub offset: u64,
    pub size: u64,
    pub alignment: u64,
    pub kind: SuballocationKind,
    pub dedicated: bool,
}

/// Plan for allocating a new backing `VkDeviceMemory` block when existing blocks
/// cannot satisfy an [`AllocationRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct PlannedBlock {
    pub id: BlockId,
    pub domain: MemoryDomain,
    pub memory_type_index: u32,
    pub size: u64,
    pub dedicated: bool,
    /// Idle completely-free pooled blocks in `domain` that must be evicted first
    /// to stay within budget.
    pub evict_blocks: Vec<BlockId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionState {
    Free,
    Occupied {
        id: AllocationId,
        kind: SuballocationKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    offset: u64,
    size: u64,
    state: RegionState,
}

#[derive(Debug)]
struct MemoryBlock {
    id: BlockId,
    domain: MemoryDomain,
    memory_type_index: u32,
    size: u64,
    dedicated: bool,
    regions: Vec<Region>,
}

impl MemoryBlock {
    fn new_free(
        id: BlockId,
        domain: MemoryDomain,
        memory_type_index: u32,
        size: u64,
        dedicated: bool,
    ) -> Self {
        Self {
            id,
            domain,
            memory_type_index,
            size,
            dedicated,
            regions: vec![Region {
                offset: 0,
                size,
                state: RegionState::Free,
            }],
        }
    }

    fn is_completely_free(&self) -> bool {
        self.regions.len() == 1
            && self.regions.first().is_some_and(|r| {
                r.state == RegionState::Free && r.offset == 0 && r.size == self.size
            })
    }

    /// Searches `self.regions` for the first `Free` span that satisfies `req.alignment`,
    /// `req.size`, and `buffer_image_granularity` page separation against both adjacent
    /// occupied neighbors.
    fn try_suballocate(
        &mut self,
        alloc_id: AllocationId,
        req: &AllocationRequest,
        granularity: u64,
    ) -> Option<Suballocation> {
        let mut chosen: Option<(usize, u64, u64)> = None;

        for (idx, region) in self.regions.iter().enumerate() {
            if region.state != RegionState::Free {
                continue;
            }
            let free_start = region.offset;
            let free_end = free_start.checked_add(region.size)?;

            // Check immediately preceding occupied neighbor (`idx - 1`):
            // When `left.kind != req.kind` and `granularity > 1`, `(left_end - 1)` and
            // `alloc_start` must reside on strictly distinct `granularity` pages:
            // `alloc_start / granularity > (free_start - 1) / granularity`.
            let min_start = if let Some(prev_idx) = idx.checked_sub(1)
                && let Some(Region {
                    state:
                        RegionState::Occupied {
                            kind: left_kind, ..
                        },
                    ..
                }) = self.regions.get(prev_idx)
                && *left_kind != req.kind
                && granularity > 1
                && free_start > 0
            {
                let left_last_page = (free_start - 1) / granularity;
                left_last_page.checked_add(1)?.checked_mul(granularity)?
            } else {
                free_start
            };

            let aligned_start = align_up(min_start, req.alignment)?;
            let alloc_end = aligned_start.checked_add(req.size)?;
            if aligned_start < free_start || alloc_end > free_end {
                continue;
            }

            // Check immediately following occupied neighbor (`idx + 1`):
            // When `right.kind != req.kind` and `granularity > 1`, `(alloc_end - 1)` and
            // `right_start` (`== free_end`) must reside on strictly distinct `granularity`
            // pages: `(alloc_end - 1) / granularity < free_end / granularity`, which holds
            // iff `alloc_end <= (free_end / granularity) * granularity`.
            if let Some(next_idx) = idx.checked_add(1)
                && let Some(Region {
                    state:
                        RegionState::Occupied {
                            kind: right_kind, ..
                        },
                    ..
                }) = self.regions.get(next_idx)
                && *right_kind != req.kind
                && granularity > 1
            {
                let max_end_exclusive = (free_end / granularity).checked_mul(granularity)?;
                if alloc_end > max_end_exclusive {
                    continue;
                }
            }

            chosen = Some((idx, aligned_start, alloc_end));
            break;
        }

        let (idx, aligned_start, alloc_end) = chosen?;
        let old = *self.regions.get(idx)?;
        let free_start = old.offset;
        let free_end = free_start.checked_add(old.size)?;

        let mut replacement = Vec::with_capacity(3);
        if aligned_start > free_start {
            replacement.push(Region {
                offset: free_start,
                size: aligned_start - free_start,
                state: RegionState::Free,
            });
        }
        replacement.push(Region {
            offset: aligned_start,
            size: req.size,
            state: RegionState::Occupied {
                id: alloc_id,
                kind: req.kind,
            },
        });
        if alloc_end < free_end {
            replacement.push(Region {
                offset: alloc_end,
                size: free_end - alloc_end,
                state: RegionState::Free,
            });
        }

        self.regions.splice(idx..=idx, replacement);

        Some(Suballocation {
            id: alloc_id,
            block_id: self.id,
            domain: self.domain,
            memory_type_index: self.memory_type_index,
            offset: aligned_start,
            size: req.size,
            alignment: req.alignment,
            kind: req.kind,
            dedicated: self.dedicated,
        })
    }

    /// Marks the region occupied by `alloc_id` as `Free` and coalesces with any
    /// adjacent `Free` neighbors so `[0, self.size)` maintains maximal free spans.
    fn free_and_coalesce(&mut self, alloc_id: AllocationId) -> bool {
        let Some(idx) = self
            .regions
            .iter()
            .position(|r| matches!(r.state, RegionState::Occupied { id, .. } if id == alloc_id))
        else {
            return false;
        };

        if let Some(region) = self.regions.get_mut(idx) {
            region.state = RegionState::Free;
        }

        // Coalesce with right neighbor first if also `Free`.
        if let Some(right_idx) = idx.checked_add(1)
            && let Some(&right) = self.regions.get(right_idx)
            && right.state == RegionState::Free
        {
            if let Some(cur) = self.regions.get_mut(idx) {
                cur.size = cur.size.saturating_add(right.size);
            }
            self.regions.remove(right_idx);
        }

        // Coalesce with left neighbor if also `Free`.
        if let Some(left_idx) = idx.checked_sub(1)
            && let Some(&cur) = self.regions.get(idx)
            && self
                .regions
                .get(left_idx)
                .is_some_and(|left| left.state == RegionState::Free)
        {
            if let Some(left) = self.regions.get_mut(left_idx) {
                left.size = left.size.saturating_add(cur.size);
            }
            self.regions.remove(idx);
        }

        true
    }
}

#[derive(Debug, Clone, Copy)]
struct RetiredAllocation {
    alloc: Suballocation,
    last_used_timeline: u64,
}

/// Pure, GPU-free block suballocator and timeline-semaphore retirement tracker.
#[derive(Debug)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct PureGpuAllocator {
    budget: GpuMemoryBudget,
    buffer_image_granularity: u64,
    next_block_id: u64,
    next_alloc_id: u64,
    blocks: Vec<MemoryBlock>,
    live: Vec<Suballocation>,
    retired: Vec<RetiredAllocation>,
    device_local_block_bytes: u64,
    host_visible_block_bytes: u64,
    live_bytes: u64,
    retired_bytes: u64,
    peak_block_bytes: u64,
    peak_allocated_bytes: u64,
}

#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
impl PureGpuAllocator {
    /// Creates a pure GPU block suballocator for `budget` and Vulkan
    /// `buffer_image_granularity` (`>= 1` byte; `0` is normalized to `1`).
    #[track_caller]
    pub(crate) fn new(
        budget: GpuMemoryBudget,
        buffer_image_granularity: u64,
    ) -> Result<Self, RenderError> {
        budget.validate()?;
        Ok(Self {
            budget,
            buffer_image_granularity: buffer_image_granularity.max(1),
            next_block_id: 1,
            next_alloc_id: 1,
            blocks: Vec::new(),
            live: Vec::new(),
            retired: Vec::new(),
            device_local_block_bytes: 0,
            host_visible_block_bytes: 0,
            live_bytes: 0,
            retired_bytes: 0,
            peak_block_bytes: 0,
            peak_allocated_bytes: 0,
        })
    }

    /// Configured `bufferImageGranularity` in bytes (`>= 1`).
    #[cfg(test)]
    pub(crate) const fn buffer_image_granularity(&self) -> u64 {
        self.buffer_image_granularity
    }

    /// Current allocator and retirement usage statistics.
    pub(crate) fn stats(&self) -> GpuAllocatorStats {
        let dedicated_block_count = self.blocks.iter().filter(|b| b.dedicated).count();
        GpuAllocatorStats {
            block_count: u32::try_from(self.blocks.len()).unwrap_or(u32::MAX),
            dedicated_block_count: u32::try_from(dedicated_block_count).unwrap_or(u32::MAX),
            device_local_block_bytes: self.device_local_block_bytes,
            host_visible_block_bytes: self.host_visible_block_bytes,
            live_allocations: u32::try_from(self.live.len()).unwrap_or(u32::MAX),
            live_allocated_bytes: self.live_bytes,
            retired_allocations: u32::try_from(self.retired.len()).unwrap_or(u32::MAX),
            retired_bytes: self.retired_bytes,
            peak_block_bytes: self.peak_block_bytes,
            peak_allocated_bytes: self.peak_allocated_bytes,
        }
    }

    /// Total unfreed suballocations (`live + retired`). Must reach `0` on clean shutdown.
    pub(crate) fn leak_count(&self) -> usize {
        self.live.len().saturating_add(self.retired.len())
    }

    /// Verifies that zero live or retired suballocations remain outstanding.
    #[cfg(test)]
    #[track_caller]
    pub(crate) fn check_no_leaks(&self) -> Result<(), RenderError> {
        if !self.live.is_empty() || !self.retired.is_empty() {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceHandle).with_detail(format!(
                    "GPU allocator shutdown leak: {} live ({} B) and {} retired ({} B) suballocations still outstanding",
                    self.live.len(),
                    self.live_bytes,
                    self.retired.len(),
                    self.retired_bytes
                )),
            );
        }
        Ok(())
    }

    /// Pure single-step allocation helper (tries existing pooled blocks, otherwise
    /// creates and registers a new block within budget). Used directly by headless
    /// `F3D-T08` tests.
    #[cfg(test)]
    #[track_caller]
    pub(crate) fn allocate(
        &mut self,
        req: AllocationRequest,
    ) -> Result<Suballocation, RenderError> {
        if let Some(sub) = self.try_suballocate_existing(req)? {
            return Ok(sub);
        }
        let plan = self.plan_new_block(req)?;
        let _evicted = self.commit_evictions(&plan.evict_blocks);
        self.register_block_and_allocate(&plan, req)
    }

    /// Attempts to satisfy `req` from an existing non-dedicated block of matching
    /// `(domain, memory_type_index)` without allocating a new `VkDeviceMemory`.
    #[track_caller]
    pub(crate) fn try_suballocate_existing(
        &mut self,
        req: AllocationRequest,
    ) -> Result<Option<Suballocation>, RenderError> {
        validate_request(&req)?;
        let alloc_id = AllocationId(self.next_alloc_id);
        let granularity = self.buffer_image_granularity;

        for block in &mut self.blocks {
            if block.dedicated
                || block.domain != req.domain
                || block.memory_type_index != req.memory_type_index
            {
                continue;
            }
            if let Some(sub) = block.try_suballocate(alloc_id, &req, granularity) {
                self.next_alloc_id = self.next_alloc_id.saturating_add(1);
                self.record_new_live(sub);
                return Ok(Some(sub));
            }
        }
        Ok(None)
    }

    /// Plans a new backing block for `req` when `try_suballocate_existing` returned
    /// `None`, verifying domain budget limits BEFORE any Vulkan call is made.
    #[track_caller]
    pub(crate) fn plan_new_block(
        &self,
        req: AllocationRequest,
    ) -> Result<PlannedBlock, RenderError> {
        validate_request(&req)?;
        let limit = self.budget.domain_limit(req.domain);
        let pooled_block_size = self.budget.domain_block_size(req.domain).min(limit);
        let dedicated = req.size > pooled_block_size;
        let new_block_size = if dedicated {
            req.size
        } else {
            pooled_block_size
        };

        let current_domain_bytes = self.domain_block_bytes(req.domain);
        let mut evict_blocks = Vec::new();
        let mut reclaimable_bytes = 0_u64;

        if current_domain_bytes
            .checked_add(new_block_size)
            .is_none_or(|after| after > limit)
        {
            // Evict completely idle pooled blocks in this domain (for example,
            // from another memory_type_index or prior peak) until headroom fits.
            for block in &self.blocks {
                if block.domain == req.domain && !block.dedicated && block.is_completely_free() {
                    evict_blocks.push(block.id);
                    reclaimable_bytes = reclaimable_bytes.saturating_add(block.size);
                    let projected = current_domain_bytes
                        .saturating_sub(reclaimable_bytes)
                        .checked_add(new_block_size);
                    if projected.is_some_and(|after| after <= limit) {
                        break;
                    }
                }
            }
        }

        let effective_used = current_domain_bytes.saturating_sub(reclaimable_bytes);
        let projected = effective_used.checked_add(new_block_size).ok_or_else(|| {
            RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(format!(
                "domain `{}` byte accounting overflow while requesting {} bytes",
                req.domain.as_str(),
                req.size
            ))
        })?;

        if projected > limit {
            return Err(RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(
                format!(
                    "domain `{}` budget exhausted: requested {} B (block {} B, dedicated={}), in use {} B, budget {} B",
                    req.domain.as_str(),
                    req.size,
                    new_block_size,
                    dedicated,
                    effective_used,
                    limit
                ),
            ));
        }

        Ok(PlannedBlock {
            id: BlockId(self.next_block_id),
            domain: req.domain,
            memory_type_index: req.memory_type_index,
            size: new_block_size,
            dedicated,
            evict_blocks,
        })
    }

    /// Removes the idle blocks planned for eviction, returning the removed `BlockId`s.
    pub(crate) fn commit_evictions(&mut self, evict_ids: &[BlockId]) -> Vec<BlockId> {
        let mut removed = Vec::with_capacity(evict_ids.len());
        for &id in evict_ids {
            if let Some(pos) = self
                .blocks
                .iter()
                .position(|b| b.id == id && b.is_completely_free())
            {
                let block = self.blocks.remove(pos);
                self.sub_domain_block_bytes(block.domain, block.size);
                removed.push(id);
            }
        }
        removed
    }

    /// Commits a newly allocated block into the suballocator and carves out `req`.
    #[track_caller]
    pub(crate) fn register_block_and_allocate(
        &mut self,
        plan: &PlannedBlock,
        req: AllocationRequest,
    ) -> Result<Suballocation, RenderError> {
        validate_request(&req)?;
        let mut block = MemoryBlock::new_free(
            plan.id,
            plan.domain,
            plan.memory_type_index,
            plan.size,
            plan.dedicated,
        );
        let alloc_id = AllocationId(self.next_alloc_id);
        let sub = block
            .try_suballocate(alloc_id, &req, self.buffer_image_granularity)
            .ok_or_else(|| {
                RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(format!(
                    "fresh block {} ({} B) could not satisfy {} B (alignment {})",
                    plan.id.0, plan.size, req.size, req.alignment
                ))
            })?;

        self.next_block_id = plan.id.0.saturating_add(1);
        self.next_alloc_id = self.next_alloc_id.saturating_add(1);
        self.add_domain_block_bytes(plan.domain, plan.size);
        self.blocks.push(block);
        self.record_new_live(sub);
        Ok(sub)
    }

    /// Queues `alloc` for deferred reclamation once the GPU timeline semaphore
    /// reaches `last_used_timeline`. Its bytes remain occupied until reclaimed.
    #[track_caller]
    pub(crate) fn retire(
        &mut self,
        alloc: Suballocation,
        last_used_timeline: u64,
    ) -> Result<(), RenderError> {
        let Some(pos) = self.live.iter().position(|item| *item == alloc) else {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceHandle).with_detail(format!(
                    "Suballocation(id={}, block={}) is not live (stale, forged, or already retired)",
                    alloc.id.0, alloc.block_id.0
                )),
            );
        };
        self.live.remove(pos);
        self.live_bytes = self.live_bytes.saturating_sub(alloc.size);
        self.retired_bytes = self.retired_bytes.saturating_add(alloc.size);
        self.retired.push(RetiredAllocation {
            alloc,
            last_used_timeline,
        });
        Ok(())
    }

    /// Immediately frees `alloc` without waiting for a timeline semaphore (only
    /// valid for allocations never referenced by an in-flight submission or after
    /// `vkDeviceWaitIdle`). Returns `Some(BlockId)` if a dedicated block was freed.
    #[track_caller]
    pub(crate) fn free_now(
        &mut self,
        alloc: Suballocation,
    ) -> Result<Option<BlockId>, RenderError> {
        let Some(pos) = self.live.iter().position(|item| *item == alloc) else {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceHandle).with_detail(format!(
                    "Suballocation(id={}, block={}) is not live (stale, forged, or already freed)",
                    alloc.id.0, alloc.block_id.0
                )),
            );
        };
        self.live.remove(pos);
        self.live_bytes = self.live_bytes.saturating_sub(alloc.size);
        Ok(self.release_region(alloc))
    }

    /// Reclaims all retired suballocations whose `last_used_timeline <= completed_timeline`,
    /// coalescing freed ranges and returning any dedicated `BlockId`s whose backing
    /// `VkDeviceMemory` must now be freed.
    pub(crate) fn reclaim_completed(&mut self, completed_timeline: u64) -> Vec<BlockId> {
        let mut freed_dedicated_blocks = Vec::new();
        let retired = std::mem::take(&mut self.retired);
        let mut remaining = Vec::with_capacity(retired.len());

        for entry in retired {
            if completed_timeline >= entry.last_used_timeline {
                self.retired_bytes = self.retired_bytes.saturating_sub(entry.alloc.size);
                if let Some(block_id) = self.release_region(entry.alloc) {
                    freed_dedicated_blocks.push(block_id);
                }
            } else {
                remaining.push(entry);
            }
        }
        self.retired = remaining;
        freed_dedicated_blocks
    }

    /// Reclaims any retired allocations (`u64::MAX`), checks leak status, and
    /// removes all remaining pooled blocks so the backend can free every
    /// `VkDeviceMemory` on shutdown.
    pub(crate) fn drain_all_blocks_for_shutdown(&mut self) -> (Vec<BlockId>, usize) {
        let mut blocks_to_free = self.reclaim_completed(u64::MAX);
        let leaked = self.leak_count();
        let blocks = std::mem::take(&mut self.blocks);
        for block in blocks {
            self.sub_domain_block_bytes(block.domain, block.size);
            blocks_to_free.push(block.id);
        }
        (blocks_to_free, leaked)
    }

    fn release_region(&mut self, alloc: Suballocation) -> Option<BlockId> {
        let pos = self.blocks.iter().position(|b| b.id == alloc.block_id)?;
        let is_dedicated = self.blocks.get(pos).is_some_and(|b| b.dedicated);
        if is_dedicated {
            let block = self.blocks.remove(pos);
            self.sub_domain_block_bytes(block.domain, block.size);
            return Some(block.id);
        }
        if let Some(block) = self.blocks.get_mut(pos) {
            let _ = block.free_and_coalesce(alloc.id);
        }
        None
    }

    fn record_new_live(&mut self, sub: Suballocation) {
        self.live_bytes = self.live_bytes.saturating_add(sub.size);
        self.live.push(sub);
        let total_occupied = self.live_bytes.saturating_add(self.retired_bytes);
        if total_occupied > self.peak_allocated_bytes {
            self.peak_allocated_bytes = total_occupied;
        }
    }

    const fn domain_block_bytes(&self, domain: MemoryDomain) -> u64 {
        match domain {
            MemoryDomain::DeviceLocal => self.device_local_block_bytes,
            MemoryDomain::HostVisible => self.host_visible_block_bytes,
        }
    }

    fn add_domain_block_bytes(&mut self, domain: MemoryDomain, bytes: u64) {
        match domain {
            MemoryDomain::DeviceLocal => {
                self.device_local_block_bytes = self.device_local_block_bytes.saturating_add(bytes);
            }
            MemoryDomain::HostVisible => {
                self.host_visible_block_bytes = self.host_visible_block_bytes.saturating_add(bytes);
            }
        }
        let total_blocks = self
            .device_local_block_bytes
            .saturating_add(self.host_visible_block_bytes);
        if total_blocks > self.peak_block_bytes {
            self.peak_block_bytes = total_blocks;
        }
    }

    fn sub_domain_block_bytes(&mut self, domain: MemoryDomain, bytes: u64) {
        match domain {
            MemoryDomain::DeviceLocal => {
                self.device_local_block_bytes = self.device_local_block_bytes.saturating_sub(bytes);
            }
            MemoryDomain::HostVisible => {
                self.host_visible_block_bytes = self.host_visible_block_bytes.saturating_sub(bytes);
            }
        }
    }
}

#[track_caller]
fn validate_request(req: &AllocationRequest) -> Result<(), RenderError> {
    if req.size == 0 {
        return Err(RenderError::new(RenderErrorKind::GpuMemoryExhausted)
            .with_detail("GPU allocation size must be greater than 0 bytes"));
    }
    if req.alignment == 0 || !req.alignment.is_power_of_two() {
        return Err(
            RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(format!(
                "GPU allocation alignment {} is not a non-zero power of two",
                req.alignment
            )),
        );
    }
    Ok(())
}

const fn align_up(value: u64, alignment: u64) -> Option<u64> {
    let mask = alignment.saturating_sub(1);
    let added = match value.checked_add(mask) {
        Some(v) => v,
        None => return None,
    };
    Some(added & !mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn normalize_location(report: &str) -> String {
        report
            .lines()
            .map(|line| {
                if let Some(rest) = line.strip_prefix("  --> ") {
                    let file_only = rest
                        .rsplit_once(':')
                        .and_then(|(prefix, _col)| prefix.rsplit_once(':'))
                        .map_or(rest, |(file, _line)| file)
                        .replace('\\', "/");
                    format!("  --> {file_only}:<LINE>:<COL>")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Brute-force invariant checker over all currently occupied (`live + retired`)
    /// suballocations in `allocator`.
    fn assert_allocator_invariants(allocator: &PureGpuAllocator) {
        let granularity = allocator.buffer_image_granularity();
        let mut active: Vec<Suballocation> = allocator.live.clone();
        active.extend(allocator.retired.iter().map(|r| r.alloc));

        for (i, a) in active.iter().enumerate() {
            assert!(a.size > 0);
            assert!(a.alignment.is_power_of_two());
            assert_eq!(
                a.offset % a.alignment,
                0,
                "allocation {a:?} violates alignment"
            );
            let block = allocator
                .blocks
                .iter()
                .find(|b| b.id == a.block_id)
                .expect("owning block must exist while allocation is live or retired");
            assert_eq!(block.domain, a.domain);
            assert_eq!(block.memory_type_index, a.memory_type_index);
            assert!(a.offset + a.size <= block.size);

            for b in active.iter().skip(i + 1) {
                if a.block_id != b.block_id {
                    continue;
                }
                let (first, second) = if a.offset <= b.offset { (a, b) } else { (b, a) };
                assert!(
                    first.offset + first.size <= second.offset,
                    "overlapping suballocations in {:?}: {first:?} vs {second:?}",
                    a.block_id
                );
                if first.kind != second.kind && granularity > 1 {
                    let first_last_page = (first.offset + first.size - 1) / granularity;
                    let second_first_page = second.offset / granularity;
                    assert!(
                        first_last_page < second_first_page,
                        "bufferImageGranularity ({granularity}) violation between {first:?} (page {first_last_page}) and {second:?} (page {second_first_page})"
                    );
                }
            }
        }

        // Verify adjacent `Free` regions are always maximally coalesced.
        for block in &allocator.blocks {
            let mut cursor = 0_u64;
            for window in block.regions.windows(2) {
                if let [first, second] = window {
                    assert!(
                        !(first.state == RegionState::Free && second.state == RegionState::Free),
                        "adjacent uncoalesced Free regions in block {:?}: {:?}",
                        block.id,
                        block.regions
                    );
                }
            }
            for r in &block.regions {
                assert_eq!(r.offset, cursor);
                cursor += r.size;
            }
            assert_eq!(cursor, block.size);
        }
    }

    /// F3D-T08: power-of-two alignment and `bufferImageGranularity` isolation
    /// across both left and right neighbors when Linear and Optimal share a block.
    #[test]
    fn f3d_t08_alignment_and_buffer_image_granularity_left_and_right_neighbors() {
        let budget = GpuMemoryBudget {
            device_local_bytes: 64 * 1024,
            host_visible_bytes: 64 * 1024,
            device_local_block_bytes: 32 * 1024,
            host_visible_block_bytes: 16 * 1024,
        };
        let mut alloc = PureGpuAllocator::new(budget, 4096).unwrap();

        // 1. Place two small `Linear` buffers in page 0 (`[0, 100)` and `[128, 328)`).
        let a = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 100,
                alignment: 16,
                kind: SuballocationKind::Linear,
            })
            .unwrap();
        assert_eq!(a.offset, 0);

        let b = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 200,
                alignment: 128,
                kind: SuballocationKind::Linear,
            })
            .unwrap();
        assert_eq!(b.block_id, a.block_id);
        assert_eq!(b.offset, 128);

        // 2. An `Optimal` image after `b` (`Linear` in page 0) must advance to page 1 (`4096`),
        //    and also honor a 512-byte alignment!
        let img1 = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 1024,
                alignment: 512,
                kind: SuballocationKind::Optimal,
            })
            .unwrap();
        assert_eq!(img1.block_id, a.block_id);
        assert_eq!(img1.offset, 4096);

        // 3. Free `a` (`[0, 100)`), leaving `b` (`Linear` at `[128, 328)` in page 0) as RIGHT neighbor
        //    of the leading hole `[0, 128)`. Requesting a 64-byte `Optimal` image MUST NOT reuse `[0, 64)`
        //    because page 0 still contains `b` (`Linear`)!
        assert_eq!(alloc.free_now(a).unwrap(), None);
        let img2 = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 64,
                alignment: 64,
                kind: SuballocationKind::Optimal,
            })
            .unwrap();
        assert_eq!(img2.block_id, b.block_id);
        // `img1` is also `Optimal` at `[4096, 5120)`, so `img2` (`Optimal`) packs right after `img1` at `5120`!
        assert_eq!(img2.offset, 5120);

        // Meanwhile a 64-byte `Linear` buffer CAN reuse `[0, 64)` next to `b` (`Linear`)!
        let lin_reuse = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 64,
                alignment: 64,
                kind: SuballocationKind::Linear,
            })
            .unwrap();
        assert_eq!(lin_reuse.offset, 0);

        assert_allocator_invariants(&alloc);
    }

    /// F3D-T08: deterministic coalescing of freed ranges prevents fragmentation
    /// across repeated out-of-order alloc/free cycles.
    #[test]
    fn f3d_t08_coalesces_adjacent_freed_ranges_without_fragmentation() {
        let budget = GpuMemoryBudget {
            device_local_bytes: 16 * 1024,
            host_visible_bytes: 16 * 1024,
            device_local_block_bytes: 16 * 1024,
            host_visible_block_bytes: 16 * 1024,
        };
        let mut alloc = PureGpuAllocator::new(budget, 1024).unwrap();

        for cycle in 0..8 {
            // Carve block into 8 x 2048 B slices (`16 KiB` total).
            let mut slices = Vec::new();
            for _ in 0..8 {
                let s = alloc
                    .allocate(AllocationRequest {
                        domain: MemoryDomain::DeviceLocal,
                        memory_type_index: 0,
                        size: 2048,
                        alignment: 256,
                        kind: SuballocationKind::Linear,
                    })
                    .unwrap();
                slices.push(s);
            }
            assert_eq!(alloc.stats().block_count, 1, "cycle {cycle}");
            assert_eq!(alloc.stats().live_allocated_bytes, 16 * 1024);

            // Free in interleaved order (even indices, then odd indices in reverse)
            // to exercise left-only, right-only, and two-sided coalescing.
            for idx in [0, 2, 4, 6, 7, 5, 3, 1] {
                let slice = *slices.get(idx).unwrap();
                alloc.free_now(slice).unwrap();
                assert_allocator_invariants(&alloc);
            }

            // Now a single full-block `16 KiB` allocation MUST fit into that coalesced block!
            let full = alloc
                .allocate(AllocationRequest {
                    domain: MemoryDomain::DeviceLocal,
                    memory_type_index: 0,
                    size: 16 * 1024,
                    alignment: 4096,
                    kind: SuballocationKind::Optimal,
                })
                .unwrap();
            assert_eq!(full.offset, 0);
            assert_eq!(alloc.stats().block_count, 1);
            alloc.free_now(full).unwrap();
        }

        assert_eq!(alloc.leak_count(), 0);
        alloc.check_no_leaks().unwrap();
    }

    /// F3D-T08: timeline-semaphore retirement holds allocations when
    /// `completed_timeline < last_used_timeline` and reclaims only when `>=`.
    #[test]
    fn f3d_t08_timeline_retirement_respects_strict_inequality_and_reclaims_at_or_after() {
        let budget = GpuMemoryBudget {
            device_local_bytes: 8 * 1024,
            host_visible_bytes: 8 * 1024,
            device_local_block_bytes: 4 * 1024,
            host_visible_block_bytes: 4 * 1024,
        };
        let mut alloc = PureGpuAllocator::new(budget, 256).unwrap();

        let s1 = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::HostVisible,
                memory_type_index: 1,
                size: 4 * 1024,
                alignment: 64,
                kind: SuballocationKind::Linear,
            })
            .unwrap();
        let s2_dedicated = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 6 * 1024, // > 4 KiB block -> dedicated fallback!
                alignment: 256,
                kind: SuballocationKind::Optimal,
            })
            .unwrap();
        assert!(s2_dedicated.dedicated);
        assert_eq!(alloc.stats().dedicated_block_count, 1);

        alloc.retire(s1, 10).unwrap();
        alloc.retire(s2_dedicated, 12).unwrap();

        // Double-retiring or freeing an already retired allocation returns REND-0013.
        assert_eq!(
            alloc.retire(s1, 11).unwrap_err().kind(),
            RenderErrorKind::InvalidResourceHandle
        );
        assert_eq!(
            alloc.free_now(s1).unwrap_err().kind(),
            RenderErrorKind::InvalidResourceHandle
        );

        // At timeline 9 (`9 < 10`): NOTHING is reclaimed; `s1` still occupies the 4 KiB
        // pooled block, so another 4 KiB HostVisible allocation needs the second 4 KiB block.
        assert!(alloc.reclaim_completed(9).is_empty());
        assert_eq!(alloc.stats().retired_allocations, 2);
        assert_eq!(alloc.stats().retired_bytes, 10 * 1024);

        let s3 = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::HostVisible,
                memory_type_index: 1,
                size: 4 * 1024,
                alignment: 64,
                kind: SuballocationKind::Linear,
            })
            .unwrap();
        assert_ne!(s3.block_id, s1.block_id);

        // A third 4 KiB HostVisible allocation at timeline 9 exceeds the 8 KiB budget!
        assert_eq!(
            alloc
                .allocate(AllocationRequest {
                    domain: MemoryDomain::HostVisible,
                    memory_type_index: 1,
                    size: 4 * 1024,
                    alignment: 64,
                    kind: SuballocationKind::Linear,
                })
                .unwrap_err()
                .kind(),
            RenderErrorKind::GpuMemoryExhausted
        );

        // At timeline 10 (`10 >= 10`, `10 < 12`): `s1` is reclaimed, while `s2_dedicated` stays retired!
        let freed_at_10 = alloc.reclaim_completed(10);
        assert!(freed_at_10.is_empty(), "s1 was pooled, not dedicated");
        assert_eq!(alloc.stats().retired_allocations, 1);
        assert_eq!(alloc.stats().dedicated_block_count, 1);

        // Now `s1`'s block is free again, so a new 4 KiB HostVisible allocation succeeds!
        let s4 = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::HostVisible,
                memory_type_index: 1,
                size: 4 * 1024,
                alignment: 64,
                kind: SuballocationKind::Linear,
            })
            .unwrap();
        assert_eq!(s4.block_id, s1.block_id);

        // At timeline 11 (`11 < 12`): `s2_dedicated` is STILL held.
        assert!(alloc.reclaim_completed(11).is_empty());
        assert_eq!(alloc.stats().dedicated_block_count, 1);

        // At timeline 12 (`12 >= 12`): `s2_dedicated` is reclaimed and its dedicated BlockId returned!
        let freed_at_12 = alloc.reclaim_completed(12);
        assert_eq!(freed_at_12, vec![s2_dedicated.block_id]);
        assert_eq!(alloc.stats().dedicated_block_count, 0);
        assert_eq!(alloc.stats().retired_allocations, 0);

        // Leak check catches `s3` and `s4` while still checked out!
        assert_eq!(alloc.leak_count(), 2);
        let leak_err = alloc.check_no_leaks().unwrap_err();
        assert_eq!(leak_err.kind(), RenderErrorKind::InvalidResourceHandle);
        assert!(leak_err.detail().unwrap_or("").contains("2 live"));

        alloc.retire(s3, 15).unwrap();
        alloc.retire(s4, 15).unwrap();
        let (shutdown_blocks, leaked) = alloc.drain_all_blocks_for_shutdown();
        assert_eq!(leaked, 0);
        assert_eq!(shutdown_blocks.len(), 2);
        alloc.check_no_leaks().unwrap();
    }

    /// F3D-T08: OOM budget exhaustion and invalid alignment/size return `REND-0012`
    /// matching ADR-0009 snapshot format.
    #[test]
    fn f3d_t08_oom_and_invalid_request_snapshots() {
        let budget = GpuMemoryBudget {
            device_local_bytes: 1024 * 1024,
            host_visible_bytes: 256 * 1024,
            device_local_block_bytes: 512 * 1024,
            host_visible_block_bytes: 128 * 1024,
        };
        let mut alloc = PureGpuAllocator::new(budget, 4096).unwrap();

        let _first = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 400 * 1024,
                alignment: 4096,
                kind: SuballocationKind::Optimal,
            })
            .unwrap();

        // Requesting an oversized 768 KiB dedicated image when 512 KiB block is already in use
        // (`512 KiB + 768 KiB = 1280 KiB > 1024 KiB`) fails with REND-0012!
        let oom_err = alloc
            .allocate(AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 768 * 1024,
                alignment: 4096,
                kind: SuballocationKind::Optimal,
            })
            .unwrap_err();
        assert_eq!(oom_err.kind(), RenderErrorKind::GpuMemoryExhausted);
        assert_eq!(oom_err.code(), "REND-0012");
        insta::assert_snapshot!(
            normalize_location(&oom_err.to_string()),
            @r#"
error[REND-0012]: GPU memory allocation failed or exceeded budget
  --> crates/spawn-render/src/allocator.rs:<LINE>:<COL>
   = detail: domain `DEVICE_LOCAL` budget exhausted: requested 786432 B (block 786432 B, dedicated=true), in use 524288 B, budget 1048576 B
"#
        );

        // Zero size and non-power-of-two alignment also return REND-0012 without mutating state.
        for bad_req in [
            AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 0,
                alignment: 16,
                kind: SuballocationKind::Linear,
            },
            AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 64,
                alignment: 0,
                kind: SuballocationKind::Linear,
            },
            AllocationRequest {
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                size: 64,
                alignment: 24,
                kind: SuballocationKind::Linear,
            },
        ] {
            assert_eq!(
                alloc.allocate(bad_req).unwrap_err().kind(),
                RenderErrorKind::GpuMemoryExhausted
            );
        }
    }

    #[derive(Debug, Clone)]
    enum Op {
        Alloc {
            host_visible: bool,
            mem_type: u32,
            size: u64,
            align_shift: u32,
            optimal: bool,
        },
        Retire {
            slot_seed: usize,
            timeline_delta: u64,
        },
        FreeNow {
            slot_seed: usize,
        },
        AdvanceTimeline {
            delta: u64,
        },
    }

    fn arb_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            5 => (
                any::<bool>(),
                0u32..=1u32,
                1u64..=20_000u64,
                0u32..=10u32,
                any::<bool>(),
            )
                .prop_map(|(host_visible, mem_type, size, align_shift, optimal)| Op::Alloc {
                    host_visible,
                    mem_type,
                    size,
                    align_shift,
                    optimal,
                }),
            3 => (0usize..64usize, 0u64..=4u64).prop_map(|(slot_seed, timeline_delta)| {
                Op::Retire {
                    slot_seed,
                    timeline_delta,
                }
            }),
            2 => (0usize..64usize).prop_map(|slot_seed| Op::FreeNow { slot_seed }),
            2 => (1u64..=3u64).prop_map(|delta| Op::AdvanceTimeline { delta }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// F3D-T08 property suite: randomized allocate / retire / free / reclaim sequences
        /// preserve non-overlap, alignment, `bufferImageGranularity` separation, coalescing,
        /// budget limits, and zero leaks once drained.
        #[test]
        fn f3d_t08_proptest_suballocator_invariants_coalescing_and_clean_drain(
            granularity_shift in 0u32..=12u32,
            ops in prop::collection::vec(arb_op(), 1..80),
        ) {
            let granularity = 1_u64 << granularity_shift;
            let budget = GpuMemoryBudget {
                device_local_bytes: 64 * 1024,
                host_visible_bytes: 32 * 1024,
                device_local_block_bytes: 16 * 1024,
                host_visible_block_bytes: 8 * 1024,
            };
            let mut alloc = PureGpuAllocator::new(budget, granularity).unwrap();
            let mut checked_out: Vec<Suballocation> = Vec::new();
            let mut current_timeline = 1_u64;

            for op in ops {
                match op {
                    Op::Alloc {
                        host_visible,
                        mem_type,
                        size,
                        align_shift,
                        optimal,
                    } => {
                        let req = AllocationRequest {
                            domain: if host_visible {
                                MemoryDomain::HostVisible
                            } else {
                                MemoryDomain::DeviceLocal
                            },
                            memory_type_index: mem_type,
                            size,
                            alignment: 1_u64 << align_shift,
                            kind: if optimal {
                                SuballocationKind::Optimal
                            } else {
                                SuballocationKind::Linear
                            },
                        };
                        match alloc.allocate(req) {
                            Ok(sub) => checked_out.push(sub),
                            Err(err) => prop_assert_eq!(err.kind(), RenderErrorKind::GpuMemoryExhausted),
                        }
                    }
                    Op::Retire {
                        slot_seed,
                        timeline_delta,
                    } => {
                        if !checked_out.is_empty() {
                            let idx = slot_seed % checked_out.len();
                            let sub = checked_out.swap_remove(idx);
                            alloc.retire(sub, current_timeline + timeline_delta).unwrap();
                        }
                    }
                    Op::FreeNow { slot_seed } => {
                        if !checked_out.is_empty() {
                            let idx = slot_seed % checked_out.len();
                            let sub = checked_out.swap_remove(idx);
                            let _ = alloc.free_now(sub).unwrap();
                        }
                    }
                    Op::AdvanceTimeline { delta } => {
                        current_timeline = current_timeline.saturating_add(delta);
                        let _ = alloc.reclaim_completed(current_timeline);
                    }
                }

                let stats = alloc.stats();
                prop_assert!(stats.device_local_block_bytes <= budget.device_local_bytes);
                prop_assert!(stats.host_visible_block_bytes <= budget.host_visible_bytes);
                assert_allocator_invariants(&alloc);
            }

            // Retire every remaining live allocation and advance past maximum queued `timeline_delta` (4).
            let drain_timeline = current_timeline.saturating_add(5);
            for sub in checked_out {
                alloc.retire(sub, drain_timeline).unwrap();
            }
            let _ = alloc.reclaim_completed(drain_timeline);
            assert_allocator_invariants(&alloc);
            prop_assert_eq!(alloc.leak_count(), 0);
            alloc.check_no_leaks().unwrap();

            // Every remaining pooled block must have coalesced back to 100% free!
            for block in &alloc.blocks {
                prop_assert!(block.is_completely_free());
            }
        }
    }
}
