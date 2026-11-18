# spawn-core

Local rules for `spawn-core` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Defines the smallest set of lifecycle and orchestration contracts that subsystems need to be driven uniformly (initialize, update, shutdown; `PLANNED`) and the fixed-timestep loop contract (`EXISTS`: `FixedStepClock`). It is a contract crate, not a utility library.

## Responsibilities

- Subsystem lifecycle contracts and engine-level orchestration vocabulary (PLANNED).
- Fixed-timestep loop contract separating simulation ticks from presentation frames (`EXISTS`: `FixedStepClock`; see [root: ECS and System Design](../../AGENTS.md#ecs-and-system-design)). Scheduling of ECS systems stays in `spawn-ecs` (accepted in [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md); driving the loop belongs to the root composition binary).
- Shared error and identifier conventions only when two or more crates demonstrably need the same definition.

## Non-Responsibilities

- Math types (`spawn-math`), logging/timing (`spawn-profiler`), file access (`spawn-io`), entity/component storage (`spawn-ecs`), world lifecycle (`spawn-world`), windowing (`spawn-platform`).
- General-purpose helpers, string/collection utilities, or any genre concept (`Weapon`, `Health`, `Team`).

## Architectural Boundaries

- Lowest layer alongside `spawn-math`; must not depend on any other `spawn-*` crate or any tool.
- Other crates are not required to depend on it. If a contract is needed by only one consumer, it lives with that consumer; if by many, it may move here.
- Must not link subsystems; it defines contracts that subsystems implement (dependency inversion).

## Public API Guidelines

- Every public item must name the two or more consumers that need it. Prefer small traits and plain data over concrete types.
- No re-exports of subsystem types.
- Breaking changes require updating every dependent crate's documentation in the same change.

## Internal Design Principles

- Zero-cost abstractions; no global mutable state, no singletons.
- No allocation in the per-tick contract path.
- Anti-dumping-ground rule: a new item needs a feature that requires it; reject 'misc' modules.

## Performance and Safety

- Contracts sit on the per-tick path: avoid dynamic dispatch where static dispatch suffices; document any that is unavoidable.
- Errors are typed and non-panicking; panics only for invariant violations.

## Testing Requirements

- Headless unit tests for loop timing (accumulator behavior, catch-up limits, no spiral of death).
- Compile-time/doc tests showing a minimal implementor of each contract.

## Governing Decisions

- **Accepted (binding):** [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (fixed-step loop, tick index, threading), [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (errors and diagnostics), [ADR-0012](../../docs/adr/0012-job-system.md) (job system). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): none beyond workspace-wide test tools. Any other crate needs owner approval first.
- The job system is **not** part of this crate; it lives in the planned `spawn-jobs` crate.

## Current Implementation Status

`EXISTS` (fixed-step clock only; lifecycle contracts are `PLANNED`). No dependencies; inherits workspace metadata and lints.

| Item | Module | Notes |
| --- | --- | --- |
| `Tick` | `fixed_step` | `u64` tick index newtype; monotonic, never reset (ADR-0003) |
| `FixedStepConfig` | `fixed_step` | Tick duration + max ticks per update; `Default` = 60 Hz, 4 ticks; validated constructors (`new`, `from_tick_rate`) |
| `FixedStepClock` | `fixed_step` | `advance(elapsed)` with injected monotonic time; never reads OS time; drops whole ticks beyond the limit, keeps the sub-tick remainder; overflow-safe for any `Duration` |
| `FixedStepUpdate`, `Ticks` | `fixed_step` | Ticks to run (iterator, no allocation) and dropped backlog for the caller's diagnostic |
| `ClockError`, `ClockErrorKind` | `error` | Located (`#[track_caller]`), coded, compiler-like `Display`, optional backtrace (ADR-0009) |

**Error codes:**

| Code | Kind | Meaning |
| --- | --- | --- |
| `CORE-0001` | `ZeroTickDuration` | Tick duration is zero (or tick rate > 1 000 000 000 Hz truncates to 0 ns) |
| `CORE-0002` | `ZeroTickRate` | Tick rate is zero |
| `CORE-0003` | `ZeroMaxTicksPerUpdate` | Max ticks per update is zero |
| `CORE-0004` | `TickIndexExhausted` | Advancing would overflow the `u64` tick index |

**Timing semantics:**

- Time is integer nanoseconds (`Duration`), so there is no floating-point drift in accumulation. `from_tick_rate` truncates the tick to whole nanoseconds (60 Hz → 16 666 666 ns, i.e. ~0.04 ppm faster than exactly 60 Hz).
- **Remainder policy:** after each `advance`, `pending()` is the sub-tick remainder (`< tick_duration`). When more ticks are due than `max_ticks_per_update`, only the limit runs; the whole ticks beyond it are dropped and returned in `dropped()`; the remainder is kept.
- The configuration is fixed for the clock's lifetime (no setters).
- The last executable tick is `u64::MAX - 1`; past that, `advance` returns `CORE-0004`, leaves the clock unchanged and never wraps.

**Tests (all passing):** 15 unit tests in `src/fixed_step.rs` (config validation, zero elapsed, exact tick boundary, partial accumulation, multiple ticks per advance, iterator, last-index boundary, exhaustion, error report format, saturation); 6 verification tests in `tests/fixed_step_verification.rs` (FND-T01: identical sequences, render-cadence independence at 100/40/25 fps, 5 000 irregular frames with exact tick total and consecutive indices; FND-T02: 10 s pause bounded to 4 ticks with 9.96 s dropped and continuous indices, remainder kept after a drop, `Duration::MAX` repeatedly); 1 doc test. No GPU, window or OS clock involved.

**Not implemented / deferred:**

- Interpolation fraction: deferred until rendering interpolation needs it (foundation DESIGN); derivable as `pending() / tick_duration()`.
- Subsystem lifecycle contracts: `PLANNED`, no consumer yet.
- The main loop that reads monotonic time, runs ticks and reports `dropped()`: root binary, foundation Phase 5.
- No claim of cross-platform deterministic gameplay: only the timing contract is deterministic for identical inputs and initial state.

## Agent Instructions

- Before adding anything here, check whether `spawn-math`, `spawn-io`, `spawn-profiler`, or `spawn-ecs` owns it.
- Do not add dependencies on any `spawn-*` crate.
- The clock must stay free of OS time, logging and allocation; the root binary measures time and reports dropped backlog.
- New error kinds get the next `CORE-` code; codes are never reused or renumbered.
