# spawn-math

Local rules for `spawn-math` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Provides the numeric and geometric primitives (vectors, matrices, quaternions, transforms, bounds, rays) shared by simulation, rendering, animation, audio, and networking.

## Responsibilities

- Linear algebra and geometry primitives and their operations, including `f64` world-position types distinct from `f32` local types (`EXISTS`; [ADR-0010](../../docs/adr/0010-math-library.md), accepted).
- Deterministic, documented numeric behavior (floating-point conventions, handedness, units, angle conventions, and pure-Rust `libm`-backed deterministic subset `det`; `EXISTS`).
- Compact serialization-friendly layouts (`#[repr(C)]`, little-endian byte helpers) for types sent over the network or stored in assets (`EXISTS`).

## Non-Responsibilities

- Collision detection, broadphase, or spatial queries (`spawn-physics`).
- GPU uniform layout or shader types (`spawn-render`).
- Randomness, time, or ECS components wrapping these types.

## Architectural Boundaries

- Foundation crate: depends on no other `spawn-*` crate. Any crate may depend on it.
- In-house types only; no third-party math type in any public API. Transcendental functions of the deterministic subset wrap `libm` (`default-features = false`); `glam` is used only as a test oracle ([ADR-0010](../../docs/adr/0010-math-library.md)).

## Public API Guidelines

- Conventions are fixed by ADR-0010 and stated once in crate docs: right-handed, Y-up, −Z forward; meters, seconds, kilograms, radians; column-major (`M * v`); reversed-Z with single-site Vulkan NDC Y-flip. Absolute world positions use distinct `f64` types (`WorldPos`, `DVec3`); local data is `f32`; conversion is explicit (`camera_relative_f32` / `DVec3::to_vec3_lossy`).
- Small, `Copy` value types; free of lifetimes and heap use on math operations. Validated constructors return located `MathError` (`MATH-0001`..`MATH-0005`, [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md)).
- Do not expose script-facing or network-facing behavior here; those layers adapt these types.

## Internal Design Principles

- Data layouts are explicit (`#[repr(C)]`); SIMD only with measurements (`PLANNED`).
- `#![forbid(unsafe_code)]`; no `unsafe` unless benchmarked and justified with `// SAFETY:`.

## Performance and Safety

- Hot-path friendly: inlinable, branch-light, allocation-free.
- NaN/inf behavior is documented per operation; normalization of zero-length input and inversion of singular matrices return `Option` and never panic.

## Testing Requirements

- Unit tests for identities and edge cases (zero vectors, subnormals, 180° rotations, singular matrices, NaN/Inf).
- Property-based tests (`proptest`) for algebraic laws and `glam` test-oracle comparisons with tolerances.
- Golden `to_bits()` determinism tests for operations in `det` used in prediction/reconciliation, comparing Linux and Windows results ([ADR-0005](../../docs/adr/0005-physics-strategy.md): movement code must not rely on platform math libraries for transcendental functions).

## Governing Decisions

- **Accepted (binding):** [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions), [ADR-0005](../../docs/adr/0005-physics-strategy.md) (physics strategy), [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (located coded errors). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `libm` (in use, `default-features = false`), `bytemuck` (approved, not currently linked), `glam` (in use, test oracle only), `proptest` and `insta` (in use, dev-dependencies). Any other crate needs owner approval first.

## Current Implementation Status

`EXISTS`. `src/` implements `#![forbid(unsafe_code)]` local `f32` primitives (`Vec2`, `Vec3`, `Vec4`, column-major `Mat3`, `Mat4`, `Quat` with intrinsic `YXZ` yaw/pitch helpers and Kahan angular distance, `Transform`, `Aabb`, `Sphere`, `Plane`, `Ray`), distinct `f64` world coordinates (`WorldPos`, `DVec3`, `camera_relative_f32`) enforced by `compile_fail` doctests, finite and infinite-far right-handed reversed-Z Vulkan projections with single-site NDC Y-flip, pure-Rust `libm`-backed deterministic subset `det` (`f32`/`f64` transcendentals, IEEE `sqrt`, explicit `mul_add`, `Basis3`/`DBasis3`, `quat_from_yaw_pitch`), boundary degree/radian helpers, and located `MathError` (`MATH-0001`..`MATH-0005`). Verified by headless unit, `insta` snapshot, `proptest`, `glam` oracle, golden-bit, and hygiene tests (`F3D-T01..T04`, Phase 1 `F3D-T13`). SIMD intrinsics remain `PLANNED`.

## Agent Instructions

- Do not add physics, rendering, or gameplay helpers.
- Document convention changes as breaking and update all dependents' docs.
- Only `libm` and `bytemuck` are approved runtime dependencies here; anything else needs owner approval.
