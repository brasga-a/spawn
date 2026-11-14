# ADR-0001 — Windowing and Rendering Backends

**Status:** Proposed  
**Scope:** Foundation milestone, native window and first GPU-backed clear frame.

## Context

Spawn is Rust-first and must expose genre-neutral engine contracts. The foundation requires a native window/event loop and a presentable GPU surface. Backend choices must not leak into unrelated public APIs or force dedicated servers to depend on graphics.

## Proposed decision

Evaluate **`winit`** for native event loop and windows, isolated behind `spawn-platform`; evaluate **`wgpu`** for GPU resources, surface management and presentation behind `spawn-render`. The root executable composes them. No specific crate version, OS coverage, or GPU capability is accepted yet.

The design must account for ownership/lifetimes of native window handles during surface creation, backend-specific event-loop restrictions, physical vs logical pixels, swapchain/surface recovery, and backend requirements on the selected target OS.

## Alternatives

- Platform-specific windowing layer written in-house: maximizes control but adds OS maintenance and reduces time to first frame.
- Other Rust windowing and graphics libraries: could fit particular platforms better, but must be compared against the foundation requirements.
- A full existing game engine as backend: conflicts with building Spawn's own reusable infrastructure.

## Consequences if accepted

- `spawn-platform` alone translates window-system events.
- `spawn-render` owns GPU configuration, surface acquire/present and recovery.
- Headless tests remain independent of both backends.
- The chosen crate versions, features, tested OS/compositor/GPU and known limitations are recorded at implementation time.

## Acceptance conditions

Verify current upstream APIs and platform support; show a minimal viable path to window + clear frame on the target platform; check compatible handle ownership; review maintenance/licensing and dependency footprint; record a tested baseline and failure handling. Accept only after explicit owner review.
