# spawn-world

Local rules for `spawn-world` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Runtime world management: owns the lifecycle of live worlds (create, activate, deactivate, unload), coordinates loading/unloading, and in the future spatial streaming. An engine runtime subsystem, not a world-authoring tool.

## Responsibilities

- Active-world registry and lifecycle state machine (PLANNED).
- Instantiating scene data into a world and tearing it down, coordinating with `spawn-scene`, `spawn-assets`, and `spawn-ecs`.
- Loading/unloading coordination, including async load completion handling, and future spatial streaming (cells/chunks, LOD of simulation).
- Multiple concurrent worlds (e.g. client and server worlds in a listen-server, editor preview world).

## Non-Responsibilities

- Entity/component storage and queries (`spawn-ecs`); `spawn-world` owns a world's ECS instance, not its implementation.
- Scene format, hierarchy data, and (de)serialization (`spawn-scene`).
- Asset loading mechanics (`spawn-assets`), file access (`spawn-io`), script execution (`spawn-scripting`).
- World creation/editing tools (`spawn-editor`) and level-design content.

## Architectural Boundaries

- Sits above `spawn-ecs`, `spawn-scene`, `spawn-assets`; may be driven by `spawn-core` lifecycle contracts. Must not be depended on by those crates.
- Interacts with physics, render, audio, and network through contracts those crates expose or through ECS data, never through direct coupling to their internals.
- No genre concepts (match, team, spawn points belong to frameworks/game rules).

## Public API Guidelines

- Worlds are referenced by generational handles; operations are explicit about being synchronous vs. asynchronous.
- Server and client world roles are explicit in the API (root §7).
- Keep surface small until at least two real consumers exist.

## Internal Design Principles

- Simulation state separated from presentation state; world teardown must be deterministic and leak-free.
- Streaming decisions are driven by data/policy provided by callers, not hard-coded game logic.

## Performance and Safety

- Load/unload must not block the fixed-timestep tick; budgeted work per tick.
- Failure to load part of a world yields an error state, not a panic.

## Testing Requirements

- Headless lifecycle tests (create/activate/unload ordering, double-unload, load failure).
- Multi-world isolation tests; leak checks across load/unload cycles.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No world type or lifecycle exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not put storage, scene serialization, or tooling logic here; reference the owning crate.
- Do not add streaming before a basic single-world lifecycle exists.
