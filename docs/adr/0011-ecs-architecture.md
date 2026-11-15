# ADR-0011 — ECS Architecture

**Status:** Accepted  
**Scope:** `spawn-ecs`. Needed before roadmap step 3 (ECS and entity lifecycle).  
**Related:** [ADR-0003](0003-fixed-step-loop-and-threading.md) (tick index, threading), [ADR-0012](0012-job-system.md) (parallel execution), [ADR-0021](0021-scene-ecs-relationship.md) (scene instantiation).

## Context

The ECS is the core data model for simulation. Its storage layout determines iteration speed, structural-change cost, parallelism and replication friendliness. Large battles need fast iteration over many entities; gameplay needs frequent component changes (status effects, tags). Prediction requires deterministic system ordering.

## Decision

1. **In-house ECS** in `spawn-ecs`.
2. **Entity identity:** 64-bit `Entity` = 32-bit index + 32-bit generation. Stale IDs are detected by generation mismatch and return `None`/errors.
3. **Storage: archetype tables by default**, with an opt-in **sparse-set** storage per component type for components added/removed very frequently (tags, short-lived effects). Each component type declares its storage at registration.
4. **Change detection** by tick: each component column records the tick of last change; systems query "changed since my last run". Uses the `u64` tick index of [ADR-0003](0003-fixed-step-loop-and-threading.md) or a derived counter.
5. **Structural changes** (spawn, despawn, add/remove component) during system execution go to **command buffers**, applied at defined sync points in a deterministic order.
6. **Systems** declare component and resource access (read/write). A scheduler builds a conflict graph and runs non-conflicting systems in parallel through [ADR-0012](0012-job-system.md).
7. **Deterministic order:** system execution order is explicit and stable (ordered stages plus explicit before/after constraints). Parallelism never changes the result of simulation systems used in prediction.
8. **Queries** are typed, compile-time checked for aliasing where possible, otherwise checked once at registration.
9. **Resources** (singletons such as time, input samples) are stored in the world with the same access rules.
10. **No genre components** in `spawn-ecs`; components belong to the crate that owns the concept.

## Alternatives

- **Adopt `bevy_ecs`** (evaluated and rejected by the owner): the most mature Rust ECS and fastest path. Rejected because its generic system/query types would spread through every crate that writes systems (making a later replacement a rewrite of all simulation code), its 0.x releases break APIs frequently, its parallel executor uses its own thread pool (conflicting with [ADR-0012](0012-job-system.md)), and its ecosystem uses `bevy_reflect` (overlapping `spawn-reflect`, [ADR-0021](0021-scene-ecs-relationship.md)). It remains the **primary reference**; porting parts of its design or code is allowed under MIT/Apache-2.0 with the notice required by [ADR-0008](0008-third-party-license-policy.md).
- **`hecs`:** small and simple; no scheduler or change detection.
- **flecs (C) / EnTT (C++):** excellent designs; C/C++ dependencies. References.
- **Sparse sets only (EnTT style):** cheap structural changes, slower multi-component iteration.
- **Pure archetypes (no sparse sets):** fastest iteration, expensive frequent add/remove.

## References

`bevy_ecs` (MIT/Apache-2.0), flecs (MIT) and Sander Mertens' "Building an ECS" articles, EnTT (MIT) and its blog series, Unity DOTS/Entities documentation, `hecs` (MIT/Apache-2.0).

## Consequences

- Replication ([ADR-0016](0016-networking-and-replication.md)) can use change detection to build snapshot deltas.
- Scene instantiation goes through component registration and reflection ([ADR-0021](0021-scene-ecs-relationship.md)).

## Risks

- **Complexity and `unsafe`**: archetype storage needs type-erased columns. Mitigation: `unsafe` confined to storage internals, Miri tests, model-based tests against a naive reference implementation.
- **Scheduler overhead** for many small systems. Mitigation: benchmarks on the reference hardware; batching small systems.
- **Determinism broken by parallel command application.** Mitigation: commands applied in system order, never completion order.

## Revisit triggers

Benchmarks show iteration or structural changes as bottlenecks; replication requires layouts the design cannot provide.

## Acceptance conditions

Accepted by the owner. Applying it is an implementation task.
