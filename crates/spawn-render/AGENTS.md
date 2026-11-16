# spawn-render

Local rules for `spawn-render` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Rendering infrastructure: GPU abstraction, resources, shaders, materials, pipelines, and render-frame execution. Generic; contains no gameplay rules.

## Responsibilities

- GPU device/queue/surface management and resource lifetime (PLANNED; in-house graphics API over Vulkan as a private module, accepted in [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md); `wgpu` is a design reference, not a dependency).
- Shaders, materials, pipelines, render graph/passes, camera/view abstraction, mesh/texture GPU resources.
- Extraction of renderable state from simulation into presentation state (simulation never touches GPU).
- Debug draw primitives if a feature requires them.

## Non-Responsibilities

- Windows/surfaces creation (`spawn-platform`), in-game UI layout/widgets (`spawn-ui` supplies draw data), editor viewport UI (`spawn-editor`).
- Asset identity/loading (`spawn-assets`), animation evaluation (`spawn-animation`), culling by physics queries (`spawn-physics`), gameplay effects (frameworks/Luau).
- Anything weapon-, team-, or match-aware.

## Architectural Boundaries

- May depend on `spawn-math`, `spawn-assets`, and a window handle (per [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md), accepted: through `raw-window-handle` traits, without depending on `spawn-platform`); must not depend on `spawn-physics`, `spawn-network`, `spawn-scripting`, or tools.
- `spawn-ui` and `spawn-editor` integrate through render-facing contracts, not by reaching into GPU internals.
- Server/headless builds must not require this crate.

## Public API Guidelines

- Hide the graphics backend; no Vulkan or RHI-internal types in public APIs.
- Resources are referenced by handles; frame data is plain data.

## Internal Design Principles

- Separate render world/state from simulation state; keep GPU code behind thin layers so logic (sorting, culling, batching) is headless-testable.
- `unsafe` confined and justified.

## Performance and Safety

- Frame-time budgets, minimized state changes and allocations per frame, explicit GPU memory accounting; shader/asset inputs are untrusted — validate, never panic on device loss or bad shaders.

## Testing Requirements

- Headless unit tests for CPU-side logic; golden-image tests only if a headless adapter is available; benchmarks for frame extraction and culling.

## Governing Decisions

- **Accepted (binding):** [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md) (graphics API), [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0014](../../docs/adr/0014-shader-toolchain.md) (shader toolchain), [ADR-0020](../../docs/adr/0020-memory-strategy.md) (memory strategy).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `ash`, `raw-window-handle`, `bytemuck`, `renderdoc` (behind a feature), `gpu-allocator` (fallback only). Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No GPU, shader, material, or pipeline code exists.

## Agent Instructions

- Never add gameplay logic. Do not pick the GPU backend or add dependencies without confirmation and a recorded rationale.
- Start from the minimal vertical slice ([foundation PLAN](../../docs/foundation/PLAN.md): clear frame first; triangle and mesh/camera are later milestones); no premature render graph.
