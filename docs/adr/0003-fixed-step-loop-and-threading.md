# ADR-0003 — Fixed-Step Main Loop and Threading Model

**Status:** Accepted (foundation scope; revisited when a later milestone or a measured limitation requires it).  
**Scope:** Runtime timing contracts, event-loop control and future ECS scheduling.  
**Related:** [ADR-0001](0001-in-house-platform-layer.md) (windowing backend), [ADR-0004](0004-in-house-graphics-api.md) (graphics API, presentation).

## Context

Spawn needs fixed-step simulation for future server-authoritative multiplayer, but presenting a window is driven by a native event loop with platform-specific wakeup rules. A generic engine contract must not own OS objects or decide ECS scheduling policy.

## Decision

`spawn-core` defines a minimal, headless-testable fixed-step timing/lifecycle **contract**. The root `spawn-engine` composition layer drives that contract using monotonic time while responding to native event-loop callbacks/events from `spawn-platform`. `spawn-render` presents independently of the number of simulation ticks executed; `spawn-input` samples at documented tick boundaries.

Future ECS system scheduling remains owned by `spawn-ecs`, not by the generic loop contract. Do not implement ECS scheduling in the foundation milestone.

### Fixed step and backlog

- Fixed step `dt` (proposed test default: 60 Hz, ≈16.67 ms). Each wakeup adds the measured elapsed time to an accumulator and runs ticks while the accumulator holds at least `dt`, up to a per-wakeup limit (proposed test default: 4 ticks).
- **Backlog policy: drop.** When the limit is reached, the remaining accumulated time beyond one `dt` is discarded and a diagnostic is emitted. Simulation then runs slower than real time for that moment instead of entering an unbounded catch-up loop. This follows the common engine practice of clamping frame time (e.g. Unity's maximum allowed timestep, the accumulator clamp in "Fix Your Timestep").
- Defaults are measured later, not mandated permanently.

### Tick index

`spawn-core` owns the simulation tick index: a monotonically increasing `u64`, incremented once per executed tick, never reset by resize, surface recovery, or dropped backlog. Input sampling, and later networking (prediction, reconciliation, snapshots), refer to ticks by this index.

### Event-loop integration

Spawn owns the loop ([ADR-0001](0001-in-house-platform-layer.md)). The composition root runs a poll-driven loop:

1. Pump pending OS events through `spawn-platform` (non-blocking) and route them to `spawn-input`.
2. Advance the accumulator and run fixed ticks.
3. Present one frame through `spawn-render`.
4. While idle or suspended, wait with a timeout instead of spinning.

On Windows, live resize and window dragging block the thread inside the OS modal loop; the backend must handle this explicitly (e.g. timer-driven updates during the modal loop, or a dedicated window thread), not by assuming the loop keeps running.

### Threading

**Foundation: single-threaded.** Events, simulation ticks and presentation run on one thread. Simulation state and presentation state stay separate so the target model below can be introduced without rewriting the timing contract.

**Target model (introduced incrementally, each step measured and with its own review):**

| Step | Change | Owner crate | Motivation |
| --- | --- | --- | --- |
| 1 | Window/input thread: pumps OS events and sends timestamped raw events to the game thread through a bounded queue | `spawn-platform` | Continuous, precisely timestamped mouse input; game loop keeps running during Windows modal resize/drag |
| 2 | Job system: worker pool sized to the CPU; ECS systems without conflicting data access run in parallel | `spawn-jobs` (pool, [ADR-0012](0012-job-system.md)), `spawn-ecs` (scheduler) | Use all cores for large battles (physics, animation, AI) |
| 3 | Render thread: simulation prepares frame N while the render thread submits frame N-1, with late latching of camera/input | `spawn-render` | GPU stalls no longer delay simulation ticks |
| — | Real-time audio thread (already required by `spawn-audio`) | `spawn-audio` | Glitch-free audio |

Platform constraints that make this possible: Win32 windows may be created and pumped on a non-main thread (the thread that creates a window must pump its messages); Wayland event queues can be dispatched from a dedicated thread. macOS, which requires the main thread, is not a target ([ADR-0001](0001-in-house-platform-layer.md)).

**Risks of the target model:**

- **Added input latency.** Pipelining simulation and rendering delays presentation by up to one frame, the most damaging defect for an FPS. Mitigation: late latching (the render thread applies the newest input to the camera immediately before submission) and keeping queued frames minimal. Latency is measured before and after step 3; the step is rejected if latency regresses without mitigation.
- **Loss of determinism.** Client prediction and reconciliation need identical results for identical inputs. With jobs, completion order varies. Mitigation: combine parallel results in a fixed order (e.g. by entity order, never by completion order); determinism tests cover parallel systems where determinism is claimed.
- **Deadlocks and ordering bugs.** Rust prevents data races at compile time, but not deadlocks, lock-order inversions or logic that depends on timing. Mitigation: message queues instead of shared locks between dedicated threads, no locks held across sync points, no global locks in hot paths (root [Engineering Standards](../../AGENTS.md#engineering-standards)).
- **Harder debugging and profiling.** Failures become timing-dependent and hard to reproduce. Mitigation: per-thread timing in `spawn-profiler` before step 2, and a configuration that forces single-threaded execution for reproduction.
- **Event timestamp skew.** Input timestamped on one thread and consumed on another requires a single monotonic clock for all threads; mixing clock sources misassigns input to ticks.
- **Bounded queues under load.** If the game thread stalls, the input queue fills. The overflow policy (block, drop oldest, or coalesce motion) must be defined before step 1 so button edges are never lost.
- **Complexity cost.** Each step adds synchronization code that must be maintained. A step is only taken when measurements show the single-threaded path is the bottleneck or blocks a required feature.

## Alternatives

- The event loop lives in `spawn-core`: couples the foundational contract to OS backends and blocks headless uses.
- Rendering callbacks drive variable simulation delta: simple, but violates fixed-step needs and creates prediction/replay problems.
- ECS owns the entire application lifecycle: premature coupling of a generic runtime to one system model.
- Backlog policy "catch up fully": preserves real-time, but after a long stall runs many ticks at once, freezes the window and can fall into a spiral where each catch-up frame creates more backlog.
- Render thread from the start: uses more cores, but adds synchronization and double-buffered state before any workload needs it.

## Consequences if accepted

- Simulation is testable without a window or GPU.
- A dedicated server can reuse timing contracts without linking rendering.
- Lifecycle ownership remains explicit in the composition root.
- Platform redraw timing must not induce unbounded catch-up or duplicate input transitions.
- After a stall (window drag, debugger pause, suspend), simulation time falls behind real time by the dropped backlog; this is visible as a brief slowdown and is logged.
- Single-threaded foundation: one frame's CPU cost is the sum of event handling, ticks, and render submission; a blocking present (vsync) also delays the next ticks. Acceptable for the foundation; the target threading model above addresses it step by step.
- Server-side backlog behavior under sustained overload (dropping ticks affects clients) needs its own decision in the networking milestone.

## Acceptance conditions

Confirm thread/event-loop restrictions with the selected backend and require tests for pause, catch-up, tick-index monotonicity, and rendering cadence independence.
