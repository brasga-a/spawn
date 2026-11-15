# spawn-physics

Local rules for `spawn-physics` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Physics simulation: rigid bodies, collision detection, spatial queries (raycasts, overlaps, sweeps), and physical state. Emits physical events; does not decide gameplay responses.

## Responsibilities

- Collision detection, spatial queries and a kinematic character controller, all in-house ([ADR-0005](../../docs/adr/0005-physics-strategy.md), accepted; PLANNED).
- Rigid-body state, integration, constraints and a fixed-timestep step API (PLANNED; approach decided by the dynamics evaluation in ADR-0005).
- Collision detection (broadphase, narrowphase), shapes, layers/filters, contact/trigger events.
- Spatial queries used by character movement and hit registration, including queries against a bounded, tick-indexed history of poses for lag compensation (the hit-registration *rules* live in the FPS framework/server logic).
- Physics-side state snapshotting needed for rollback/reconciliation where decided.

## Non-Responsibilities

- Gameplay response to collisions (damage, pickups, abilities): frameworks and Luau reacting to events.
- Genre movement rules (acceleration curves, sprint, slide, mantle, tuning) in the FPS framework; this crate owns only the generic controller mechanism (collide-and-slide, step-up, slopes, ground detection, depenetration); ECS storage (`spawn-ecs`), math primitives (`spawn-math`), rendering of debug shapes (`spawn-render`), replication (`spawn-network`).
- Rigid-body dynamics library or solver: none until the dynamics evaluation of [ADR-0005](../../docs/adr/0005-physics-strategy.md) produces its own ADR.

## Architectural Boundaries

- May depend on `spawn-math`; may expose ECS-facing integration without ECS depending on it. Must not depend on render, scripting, ui, or gameplay crates.
- Does not assume the full simulation is deterministic ([root: Multiplayer](../../AGENTS.md#multiplayer-foundational)) and must not be contorted to be. Movement code used in prediction must be deterministic across Windows and Linux and uses `spawn-math` functions for transcendental math.

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
- Follow the references and license notes in ADR-0005; a Rust implementation that closely follows Jolt keeps Jolt's MIT notice. Never copy Quake/Source code.
