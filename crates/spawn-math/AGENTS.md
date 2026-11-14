# spawn-math

Local rules for `spawn-math` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Provides the numeric and geometric primitives (vectors, matrices, quaternions, transforms, bounds, rays) shared by simulation, rendering, animation, audio, and networking.

## Responsibilities

- Linear algebra and geometry primitives and their operations (PLANNED).
- Deterministic, documented numeric behavior (floating-point conventions, handedness, units, angle conventions).
- Compact serialization-friendly layouts for types sent over the network or stored in assets.

## Non-Responsibilities

- Collision detection, broadphase, or spatial queries (`spawn-physics`).
- GPU uniform layout or shader types (`spawn-render`).
- Randomness, time, or ECS components wrapping these types.

## Architectural Boundaries

- Foundation crate: depends on no other `spawn-*` crate. Any crate may depend on it.
- Using an external math crate (e.g. `glam`) is an unadopted option: either wrap or re-export decisions must be recorded in [root: Workspace](../../AGENTS.md#workspace) per the dependency rules. Do not leak a third-party type into public APIs unintentionally.

## Public API Guidelines

- Conventions (coordinate system, handedness, rotation order, units) are stated once in crate docs and never vary.
- Small, `Copy` value types; free of lifetimes and heap use.
- Do not expose script-facing or network-facing behavior here; those layers adapt these types.

## Internal Design Principles

- Data layouts are explicit (`repr`) only where a consumer needs layout guarantees; SIMD only with measurements.
- No `unsafe` unless benchmarked and justified with `// SAFETY:`.

## Performance and Safety

- Hot-path friendly: inlinable, branch-light, allocation-free.
- NaN/inf behavior is documented per operation; normalization of zero-length input must not panic.

## Testing Requirements

- Unit tests for identities and edge cases (zero vectors, gimbal cases, denormals).
- Property-based tests for algebraic laws (e.g. rotation composition, inverse round-trips) with tolerances.
- Determinism tests for any operation used in prediction/reconciliation.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No math type exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not add physics, rendering, or gameplay helpers.
- Document convention changes as breaking and update all dependents' docs.
- Confirm with the user before adopting an external math dependency.
