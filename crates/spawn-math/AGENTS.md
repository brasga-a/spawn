# spawn-math

Local rules for `spawn-math` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Provides the numeric and geometric primitives (vectors, matrices, quaternions, transforms, bounds, rays) shared by simulation, rendering, animation, audio, and networking.

## Responsibilities

- Linear algebra and geometry primitives and their operations, including `f64` world-position types distinct from `f32` local types (PLANNED; [ADR-0010](../../docs/adr/0010-math-library.md), accepted).
- Deterministic, documented numeric behavior (floating-point conventions, handedness, units, angle conventions).
- Compact serialization-friendly layouts for types sent over the network or stored in assets.

## Non-Responsibilities

- Collision detection, broadphase, or spatial queries (`spawn-physics`).
- GPU uniform layout or shader types (`spawn-render`).
- Randomness, time, or ECS components wrapping these types.

## Architectural Boundaries

- Foundation crate: depends on no other `spawn-*` crate. Any crate may depend on it.
- In-house types only; no third-party math type in any public API. Transcendental functions of the deterministic subset wrap `libm`; `glam` is used only as a test oracle ([ADR-0010](../../docs/adr/0010-math-library.md)).

## Public API Guidelines

- Conventions are fixed by ADR-0010 and stated once in crate docs: right-handed, Y-up, −Z forward; meters, seconds, kilograms, radians; column-major; reversed-Z. Absolute world positions use distinct `f64` types; local data is `f32`; conversion is explicit.
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
- Determinism tests for any operation used in prediction/reconciliation, comparing Windows and Linux results ([ADR-0005](../../docs/adr/0005-physics-strategy.md): movement code must not rely on platform math libraries for transcendental functions).

## Governing Decisions

- **Accepted (binding):** [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions), [ADR-0005](../../docs/adr/0005-physics-strategy.md) (physics strategy). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `libm`, `bytemuck`, `glam` (test oracle only). Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No math type exists.

## Agent Instructions

- Do not add physics, rendering, or gameplay helpers.
- Document convention changes as breaking and update all dependents' docs.
- Only `libm` and `bytemuck` are approved runtime dependencies here; anything else needs owner approval.
