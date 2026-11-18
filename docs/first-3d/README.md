# Milestone 2 — First 3D Rendering and Free Camera

**Milestone:** Spawn First 3D / initial 3D graphics, shader cook, math, and camera slice  
**Status:** `PLANNED` (builds on the [Foundation milestone](../foundation/README.md), which `EXISTS` on Linux/Wayland + Vulkan 1.3; no Milestone 2 code exists yet). Items governed by *Proposed* [ADR-0014](../adr/0014-shader-toolchain.md), [ADR-0020](../adr/0020-memory-strategy.md), or [ADR-0022](../adr/0022-repository-layout.md) are marked **proposal (pending Phase 0 owner acceptance)** throughout this suite.  
**Target:** Extend the genre-free native runtime from a clear frame to a minimal, interactive 3D scene: in-house spatial math (`spawn-math`), offline Slang-to-SPIR-V shader cooking (`spawn-cli`, *proposal per ADR-0014*), an in-house Vulkan 1.3 GPU block allocator, vertex/index/staging buffers, a `D32_SFLOAT` reversed-Z depth attachment, dynamic-rendering graphics pipelines (`spawn-render`), and a 60 Hz fixed-step `f64` free-fly camera with sub-tick `alpha` presentation interpolation and camera-relative `f32` GPU submission (`spawn-engine`).

The milestone proves spatial conventions, large-world `f64` precision, offline shader cooking, GPU resource lifetimes, reversed-Z occlusion, and simulation/presentation separation before ECS worlds, glTF/texture asset pipelines, physics, or genre frameworks are introduced.

## Ownership

| Package | Scope for this milestone |
| --- | --- |
| `spawn-math` | In-house `f32` linear-algebra/geometry primitives, distinct `f64` world-position types, explicit `f64` ↔ `f32` camera-relative conversions, reversed-Z perspective projection with single-site Vulkan Y-flip, and `libm`-backed deterministic math subset |
| `spawn-core` | Existing fixed-step clock (`FixedStepClock`, `Tick`, `pending()`, `tick_duration()`); no new subsystem coupling |
| `spawn-platform` | Existing window, event pump, unaccelerated relative pointer motion, and cursor lock/confinement |
| `spawn-input` | Existing keyboard/mouse device tracking, digital/motion axes, and tick-indexed `InputCommand` sampling |
| `spawn-cli` | Offline Slang → SPIR-V + reflection build command invoking pinned `slangc` (**proposal**, pending Phase 0 acceptance of [ADR-0014](../adr/0014-shader-toolchain.md) and Slang license sign-off) |
| `spawn-render` | Cooked SPIR-V + reflection validation, in-house Vulkan 1.3 block allocator, staging/vertex/index buffers, swapchain-sized `D32_SFLOAT` reversed-Z depth attachment across resize/suspend, and dynamic-rendering graphics pipelines |
| `spawn-engine` (root binary) | Composition root: 60 Hz fixed-step `f64` free-fly camera simulation, `alpha` presentation interpolation, camera-relative `f32` draw extraction, procedural 3D test scene, and reverse-order teardown |

Dependency direction remains strictly downward and acyclic: `spawn-math` and `spawn-core` depend on zero `spawn-*` crates; `spawn-render` adds `spawn-math` only (never `spawn-platform`, `spawn-input`, `spawn-core`, or `spawn-cli`); engine crates never depend on `spawn-cli`.

## Read in order

1. [Specification](SPEC.md)
2. [Design](DESIGN.md)
3. [Invariants](INVARIANTS.md)
4. [Verification](VERIFICATION.md)
5. [Implementation plan](PLAN.md)
6. [ADRs](../adr/README.md)

Details marked **proposal** are not authorized for implementation until the governing ADR is accepted by the owner in [Phase 0](PLAN.md#ordered-increments). Keep unrelated `SCAFFOLD` crates (`spawn-ecs`, `spawn-world`, `spawn-scene`, `spawn-assets`, `spawn-io`, `spawn-physics`, `spawn-scripting`, etc.) untouched.
