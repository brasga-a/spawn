# spawn-core

Local rules for `spawn-core` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Defines the smallest set of lifecycle and orchestration contracts that subsystems need to be driven uniformly (initialize, update, shutdown) and the fixed-timestep loop contract (PLANNED). It is a contract crate, not a utility library.

## Responsibilities

- Subsystem lifecycle contracts and engine-level orchestration vocabulary (PLANNED).
- Fixed-timestep loop contract separating simulation ticks from presentation frames (see [root: ECS and System Design](../../AGENTS.md#ecs-and-system-design)). Scheduling of ECS systems stays in `spawn-ecs` (accepted in [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md); driving the loop belongs to the root composition binary).
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

- Headless unit tests for loop timing (accumulator behavior, catch-up limits, no spiral of death) once implemented.
- Compile-time/doc tests showing a minimal implementor of each contract.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No lifecycle contract, loop, or shared type exists.

## Agent Instructions

- Before adding anything here, check whether `spawn-math`, `spawn-io`, `spawn-profiler`, or `spawn-ecs` owns it.
- Do not add dependencies on any `spawn-*` crate.
- Proposed decisions affecting this crate (follow them once accepted; until then, do not implement contrary designs): [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (errors).
- The job system is **not** part of this crate; it lives in the planned `spawn-jobs` crate ([ADR-0012](../../docs/adr/0012-job-system.md), accepted).
