# spawn-ecs

Local rules for `spawn-ecs` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Owns the entity-component-system data model: entity identity, component storage, queries, and the infrastructure that runs systems over that data.

## Responsibilities

- Generational entity identifiers and entity lifecycle (spawn/despawn, liveness checks).
- Component storage and archetype/table layout (PLANNED; layout decision is internal and not yet made).
- Queries, filters, change detection, and deferred command buffers.
- System definition, access declaration (read/write sets), and scheduling/parallel execution (see [root: Open Architectural Questions](../../AGENTS.md#open-architectural-questions) for the split with `spawn-core`).
- Events/resources scoped to ECS data if a concrete feature requires them.

## Non-Responsibilities

- World lifecycle, active-world management, loading/streaming (`spawn-world`).
- Scene serialization and hierarchy authoring data (`spawn-scene`).
- Runtime type metadata (`spawn-reflect`); script-facing handles and API (`spawn-scripting`).
- Any concrete component such as `Transform`, `Health`, `Weapon`: components belong to the crate that owns the concept; genre components to frameworks.

## Architectural Boundaries

- Depends on no higher-level crate (render, physics, scripting, world, scene). May plausibly depend on `spawn-math` and `spawn-reflect` later; not required.
- `spawn-world`, `spawn-scene`, and domain systems depend on ECS; ECS never depends on them.
- No knowledge of Luau: scripts see generational handles exposed by `spawn-scripting`, never ECS internals.

## Public API Guidelines

- Public surface: entity IDs, component trait/registration, queries, commands, system scheduling. Storage internals stay `pub(crate)`.
- Stale entity IDs are detectable and yield errors/`None`, never UB.
- Access conflicts are expressed in types or checked at scheduling time, not by convention.

## Internal Design Principles

- Data-oriented storage; no per-entity heap allocation in iteration paths; no global locks in hot paths.
- Structural changes are deferred and applied at defined sync points.
- Parallelism must be safe by construction; `unsafe` is permitted for storage internals only with `// SAFETY:` comments and tests (e.g. under Miri).

## Performance and Safety

- Iteration, spawn, and despawn are the benchmarked critical paths; add benches with the first storage implementation.
- Memory growth from untrusted sources (network/scripts creating entities) must be bounded by callers' limits; expose capacity hooks.

## Testing Requirements

- Invariant tests: generation reuse, despawn during iteration, query aliasing rejection.
- Property/model-based tests comparing storage against a naive reference model.
- Concurrency tests for the scheduler; benchmarks for iteration and structural change.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No entity, component, query, or scheduler exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not define gameplay components or world/scene types here.
- Do not choose storage layout or scheduler design without recording the rationale in this file.
- Keep ECS usable without any renderer, physics, or scripting crate (headless).
