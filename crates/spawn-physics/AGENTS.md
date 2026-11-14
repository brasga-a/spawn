# spawn-physics

Local rules for `spawn-physics` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Physics simulation: rigid bodies, collision detection, spatial queries (raycasts, overlaps, sweeps), and physical state. Emits physical events; does not decide gameplay responses.

## Responsibilities

- Physical state, integration, constraints, and a fixed-timestep step API (PLANNED).
- Collision detection (broadphase, narrowphase), shapes, layers/filters, contact/trigger events.
- Spatial queries used by character movement and hit registration (queries are owned here; the hit-registration *rules* live in the FPS framework/server logic).
- Physics-side state snapshotting needed for rollback/reconciliation where decided.

## Non-Responsibilities

- Gameplay response to collisions (damage, pickups, abilities): frameworks and Luau reacting to events.
- Character controller *rules* for a genre (framework), ECS storage (`spawn-ecs`), math primitives (`spawn-math`), rendering of debug shapes (`spawn-render`), replication (`spawn-network`).
- Build-vs-integrate decision is open ([root: Open Architectural Questions](../../AGENTS.md#open-architectural-questions)); until decided do not add a physics library.

## Architectural Boundaries

- May depend on `spawn-math`; may expose ECS-facing integration without ECS depending on it. Must not depend on render, scripting, ui, or gameplay crates.
- Does not assume the full simulation is deterministic ([root: Multiplayer](../../AGENTS.md#multiplayer-foundational)) and must not be contorted to be.

## Public API Guidelines

- Bodies/colliders referenced by generational handles; queries return plain data.
- Events are data consumed at defined sync points.

## Internal Design Principles

- Data-oriented, cache-friendly, parallel-friendly broadphase; no per-body heap allocation in the step; fixed timestep only.

## Performance and Safety

- Step cost scales with active bodies; budget and bound untrusted inputs (script/network-driven body creation, query counts); NaN/inf never enters the solver.

## Testing Requirements

- Headless tests of collision correctness, query results, stability scenarios; benchmarks for broadphase and step; determinism tests only where determinism is claimed.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No rigid body, collider, or query exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not add damage, health, weapon, or vehicle-gameplay concepts. Start with what the character controller needs (root roadmap).
- Record the build-vs-integrate decision here before adding code.
