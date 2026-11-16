# ADR-0010 — Math Library and Spatial Conventions

**Status:** Accepted  
**Scope:** `spawn-math`; conventions binding for every crate.  
**Related:** [ADR-0004](0004-in-house-graphics-api.md) (Vulkan clip space), [ADR-0005](0005-physics-strategy.md) (deterministic movement math).

## Context

Every simulation, rendering, animation and audio crate shares vector, matrix and transform types. Conventions (handedness, up axis, units, depth range) are expensive to change later because they leak into assets, shaders, scripts and network data. ADR-0005 requires deterministic transcendental functions for movement code across Windows and Linux.

## Decision

1. **In-house types in `spawn-math`:** `Vec2/3/4`, `Mat3/4`, `Quat`, `Transform` (translation, rotation, uniform or non-uniform scale), `Aabb`, `Ray`, `Plane`, `Sphere`, in `f32`, plus `f64` variants for world positions (point 3). No third-party math type appears in any public API.
2. **Conventions (fixed engine-wide):**
   - **Right-handed, Y-up, −Z forward** (matches glTF, simplifying asset import).
   - **Units:** meters, seconds, kilograms, radians.
   - **Matrices:** column-major storage, column vectors (`M * v`), matching SPIR-V/Slang defaults.
   - **Depth:** Vulkan `[0, 1]` clip depth with **reversed-Z** (near = 1, far = 0) for precision over long view distances.
   - **Vulkan Y-flip** handled once in the projection matrix or viewport, never scattered.
   - **Rotation order** for Euler-angle APIs (e.g. FPS yaw/pitch): documented once; quaternions are the canonical representation.
3. **Large-world coordinates: `f64` world positions from the start** (owner decision), following the model used by Unreal Engine 5 and Jolt's double-precision mode:
   - **`f64`:** absolute world positions only (entity translations in world space, world-space bounds, spawn points, network-decoded positions before quantization). Identical on client and server, so prediction and reconciliation compute the same values.
   - **`f32`:** everything relative or local: rotations, scales, velocities, directions, local-space offsets, animation, and all GPU data.
   - **Rendering is camera-relative:** `f64` world positions minus the `f64` camera position, converted to `f32` on the CPU before upload. The GPU never sees absolute world positions.
   - **Physics:** world positions of bodies and query origins in `f64`; shape-local computation in `f32` relative to a nearby origin. Detailed in the physics implementation under [ADR-0005](0005-physics-strategy.md).
   - **Network:** positions are quantized for transport ([ADR-0016](0016-networking-and-replication.md)); quantization range and precision are defined per map size.
   - Conversion between `f64` and `f32` spaces happens through explicit functions, never implicit casts.
4. **Deterministic subset**, for `f32` and `f64`: functions whose results are identical on Windows and Linux for the same inputs: `sqrt` (IEEE correctly rounded) and `sin`, `cos`, `atan2`, `exp`, `ln`, etc. from the pure-Rust `libm` crate (same code on every platform, approved in [DEPENDENCIES](../DEPENDENCIES.md)), wrapped by `spawn-math`; in-house implementations only where `libm` is insufficient **Only** code that needs determinism (predicted movement, reconciliation, other replayed simulation) uses this subset; rendering, audio, particles and other presentation code use fast platform functions. FMA usage in the subset is explicit, never left to compiler contraction.
5. **Angles at the boundaries:** code and internal APIs use **radians**. Declarative data ([ADR-0013](0013-declarative-data-format.md)) and the Luau API ([ADR-0017](0017-luau-scripting-runtime.md)) use **degrees** (e.g. field of view, recoil, turn limits); conversion happens once when data is loaded or when a value crosses the script boundary. Field names state the unit (e.g. `fov_degrees`).
6. **SIMD:** scalar implementations first; `std::arch` intrinsics only where benchmarks on the reference hardware show a gain. `std::simd` waits for stabilization.

## Alternatives

- **`glam`:** fast, widely used, SIMD-ready; uses platform math functions for transcendentals (not deterministic across platforms) and would be a public-API dependency. Kept as primary design reference.
- **`nalgebra`:** generic and complete; heavy compile times and complex types for game use.
- **Left-handed / Z-up** (Unreal, Source): no inherent benefit; would require conversion of every glTF asset.
- **`f64` everywhere:** precision, but halves SIMD throughput and doubles memory and bandwidth for data that does not need it.
- **`f32` positions with origin rebasing or camera-relative rendering only:** cheaper, sufficient for maps of a few kilometers; rejected by the owner in favor of `f64` world positions from the start, avoiding a costly migration if maps grow.

## References

`glam` (MIT/Apache-2.0), DirectXMath (MIT), *Real-Time Collision Detection*, "Reversed-Z" depth precision articles (e.g. NVIDIA developer blog), CORE-MATH and fdlibm/musl libm (algorithms for correctly rounded or portable transcendentals; check each license before porting).

## Consequences

- Asset importers, shaders, scripts and network serialization all follow these conventions.
- Deterministic functions are slower than hardware/libm ones; non-prediction code uses fast versions.
- World-space transforms are larger (24 bytes for a position instead of 12) and `f64` arithmetic is slower for SIMD; this cost is limited to world positions.
- Every crate handling world positions (ECS transforms, physics, networking, rendering extraction, audio listener/emitter placement) uses the `f64` position type and explicit conversions.

## Risks

- **Convention mistakes** propagate everywhere. Mitigation: a small set of conversion tests (glTF import axes, projection, reversed-Z) written with the first render code.
- **Precision bugs at `f64`/`f32` boundaries** (e.g. converting absolute positions to `f32` too early). Mitigation: distinct types for world and local positions so the compiler rejects mixing them; tests with entities far from the origin (e.g. 50 km).
- **Deterministic function accuracy.** Mitigation: tests against reference values and cross-platform comparison tests (ADR-0005).

## Revisit triggers

SIMD or memory bandwidth for world positions becomes a measured bottleneck; a target platform with different floating-point behavior.

## Acceptance conditions

Accepted by the owner. Applying it is an implementation task.
