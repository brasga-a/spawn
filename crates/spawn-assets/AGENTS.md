# spawn-assets

Local rules for `spawn-assets` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Asset management: stable asset identity, loading, caching, dependency tracking, and hot reload of imported/runtime asset data.

## Responsibilities

- Asset IDs/handles with generational validity and reference counting or equivalent lifetime policy (PLANNED).
- Loader registration and asynchronous loading, caching, dependency graphs.
- Hot-reload notification of changed assets (development).
- Asset metadata and versioning; validated load errors naming file, field, reason.
- Import/build pipeline contracts used by `spawn-cli` and `spawn-editor` (pipeline built only when a feature needs it).

## Non-Responsibilities

- Raw file/stream access (`spawn-io`); assets call into it.
- GPU resources, shaders, materials as runtime objects (`spawn-render`); audio devices/mixing (`spawn-audio`); skeleton evaluation (`spawn-animation`). Those crates define their asset *formats' runtime use*; this crate manages identity and lifecycle.
- Scene structure (`spawn-scene`) and world loading coordination (`spawn-world`).

## Architectural Boundaries

- Plausibly depends on `spawn-io`; must not depend on render, physics, audio, world, scene, or scripting. Domain crates register loaders with it (dependency inversion), not the other way round.
- No editor-only logic in the runtime crate.

## Public API Guidelines

- Typed handles; loading state (pending/ready/failed) is observable.
- Callers never see partially initialized assets.

## Internal Design Principles

- Loading is off the simulation thread; completion applied at sync points.
- Content-addressed or ID-based identity decision must be recorded here when made.

## Performance and Safety

- Asset files are untrusted: size limits, bounds-checked parsing, no panics.
- Memory budgets and eviction policy are explicit.

## Testing Requirements

- Headless tests with in-memory sources: dedup, dependency order, failure propagation, reload.
- Concurrency tests for async loads; fuzzing of loader inputs.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No asset type or loader exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not implement format-specific decoding for other subsystems here.
- Reuse `spawn-io` for access; do not call `std::fs` directly in this crate once `spawn-io` exists.
- Proposed decisions affecting this crate (follow them once accepted; until then, do not implement contrary designs): [ADR-0020](../../docs/adr/0020-memory-strategy.md) (budgets).
- Accepted decisions governing this crate: [ADR-0015](../../docs/adr/0015-asset-pipeline.md) (identity, cooking, loading).
