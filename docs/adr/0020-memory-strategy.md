# ADR-0020 — Memory Strategy and Budgets

**Status:** Proposed  
**Scope:** CPU memory allocation patterns and memory budgets across crates; GPU memory allocation itself is in-house per [ADR-0004](0004-in-house-graphics-api.md).  
**Related:** [ADR-0012](0012-job-system.md) (per-thread allocation), [ADR-0015](0015-asset-pipeline.md) (asset budgets).

## Context

Root rules forbid per-entity heap allocation in hot paths and unbounded allocation from untrusted input. The reference hardware has 16 GB RAM and a 4 GB GPU (about 2.8 GB usable budget reported by the driver). Games need predictable frame times, which general-purpose allocation under contention can disturb.

## Decision

1. **Global allocator: the system allocator** initially. A different global allocator (e.g. `mimalloc`, MIT) is adopted only after measurements on the reference hardware show a benefit.
2. **Allocation patterns, used where they apply:**
   - **Frame arenas:** bump allocators reset every frame (or tick) for transient data; one per thread to avoid contention.
   - **Pools** for fixed-size, frequently created objects.
   - **Pre-sized containers** (`Vec::with_capacity`) in systems with known upper bounds.
   - **No allocation on the real-time audio thread** ([ADR-0006](0006-in-house-audio.md)).
3. **Budgets:** each subsystem declares memory budgets (CPU and, where relevant, GPU) for the reference hardware. Initial GPU target: the full game stays within **~2.5 GB of VRAM** to leave headroom under the driver budget. Budgets are refined by measurement.
4. **Tracking:** development builds count allocations and bytes per subsystem (through the profiler); exceeding a budget is reported. Leak checks run at world unload and shutdown.
5. **Untrusted sizes** (packets, files, scripts) are bounded before allocation.

## Alternatives

- **Custom global allocator from the start:** possible gains, unproven; rejected until measured.
- **Arena-only design:** fast, but complicates ownership in long-lived systems; used selectively.

## References

`bumpalo` (MIT/Apache-2.0) and `mimalloc` (MIT) as designs; "Memory management in game engines" talks and blog posts (e.g. Our Machinery, Molecular Musings).

## Consequences

- Systems are written with explicit capacity and lifetime decisions.
- Budget numbers become part of each subsystem's documentation once measured.

## Risks

- **Arena misuse** (data outliving its frame). Mitigation: arena lifetimes encoded in Rust lifetimes where possible.
- **VRAM budget too tight** for target visual quality on the RX 460. Mitigation: scalable quality settings; texture streaming.

## Revisit triggers

Allocation measured as a frame-time cost; memory budgets exceeded.

## Acceptance conditions

Owner review.
