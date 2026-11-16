# ADR-0012 — Job System

**Status:** Accepted  
**Scope:** Shared parallel-execution infrastructure; step 2 of the threading model in [ADR-0003](0003-fixed-step-loop-and-threading.md).  
**Related:** [ADR-0011](0011-ecs-architecture.md) (ECS scheduler), [ADR-0020](0020-memory-strategy.md) (per-thread allocation).

## Context

Large battles require using every core for physics, animation, AI and ECS systems. The reference CPU has 6 cores / 12 threads. ADR-0003 records the risks of multi-threading (determinism, deadlocks, debugging). A single shared pool avoids subsystems each creating their own threads and oversubscribing the CPU.

## Decision

1. **In-house work-stealing job system.** The worker count is **configurable**; the default is chosen by benchmark on the reference hardware (6 physical cores / 12 logical threads), comparing one worker per physical core against one per logical thread, minus the dedicated threads (main/game, window/input, audio, render when introduced).
2. **Tasks, not fibers.** Jobs are closures or task objects scheduled on per-worker deques with stealing (Chase-Lev), using `crossbeam-deque` (approved in [DEPENDENCIES](../DEPENDENCIES.md)); scheduling logic, priorities and waiting are Spawn's own. Fibers (stack switching) are not used: they need platform-specific `unsafe` code and interact badly with Rust's borrow and thread-local model.
3. **Waiting without blocking:** a thread waiting for a job group runs other jobs instead of sleeping (helping), avoiding deadlock from blocked workers.
4. **Priorities:** at least two queues: frame-critical (simulation, rendering preparation) and background (asset loading, decompression, streaming). Background work never delays frame-critical work.
5. **Placement: a dedicated crate, `spawn-jobs`** (approved by the owner; `PLANNED`, created with the first implementation). It is foundation-level: it depends on no other `spawn-*` crate (not ECS, not profiler, not core), so games that do not use parallel jobs do not compile it. The ECS scheduler and other systems depend on it.
6. **Determinism:** the job system guarantees nothing about completion order; callers that need deterministic results combine outputs in a fixed order (ADR-0003 risks).
7. **Single-threaded mode:** a configuration that runs all jobs inline on the calling thread, for debugging and reproducing bugs.
8. **Profiling:** every job carries a static label. Timing is reported through an optional hook installed by the root binary (for example, forwarding to `spawn-profiler`), so `spawn-jobs` never depends on `spawn-profiler`.

## Alternatives

- **Job system inside `spawn-core`:** no extra crate, but enlarges the core and forces every user of core contracts to compile the job system.
- **`rayon`:** mature, excellent data parallelism; global pool design, no priorities, less control over thread layout alongside dedicated threads. Primary reference.
- **Async runtimes (`tokio`):** built for I/O concurrency, not CPU-bound frame work.
- **Fiber-based (Naughty Dog style):** elegant job dependencies; rejected for the reasons above.
- **Per-subsystem threads:** simple, oversubscribes the CPU and duplicates work.

## References

`rayon` and `crossbeam-deque` (MIT/Apache-2.0), enkiTS (zlib), "Parallelizing the Naughty Dog Engine Using Fibers" (GDC 2015), Bevy tasks (MIT/Apache-2.0), Intel TBB documentation (Apache-2.0).

## Consequences

- Physics, animation and ECS systems are written as parallelizable jobs from their first implementation, even if they run single-threaded initially.
- `spawn-core` stays a small contract crate; parallel execution lives in `spawn-jobs`.

## Risks

- **Lock-free deque bugs** (memory ordering). Mitigation: the deques come from `crossbeam-deque`; Spawn's own synchronization is tested with `loom` and Miri.
- **Oversubscription** with OS and driver threads. Mitigation: measured worker count, configurable.
- **Jobs too small** cost more than they save. Mitigation: profiler data; minimum batch sizes.

## Revisit triggers

Measured contention or idle workers; a need for job dependencies beyond simple groups.

## Acceptance conditions

Accepted by the owner. Applying it is an implementation task.
