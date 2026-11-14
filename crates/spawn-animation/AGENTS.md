# spawn-animation

Local rules for `spawn-animation` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Animation runtime: skeletal poses, clip sampling, blending, state machines/graphs, and animation events, producing poses for presentation and (when required) for simulation.

## Responsibilities

- Skeleton/pose representation and clip sampling (PLANNED).
- Blending, layering, masks, IK as features require; animation graphs/state machines driven by parameters supplied by callers.
- Animation events (notifies) emitted as data.
- Root motion extraction where movement requires it.

## Non-Responsibilities

- Rendering/skinning on the GPU (`spawn-render`), asset loading (`spawn-assets`), physics ragdolls' simulation (`spawn-physics`), gameplay decisions on which state to play (frameworks/Luau supply parameters).
- Animation authoring tools (`spawn-editor`).

## Architectural Boundaries

- May depend on `spawn-math` and `spawn-assets`; must not depend on `spawn-render`, `spawn-physics`, `spawn-scripting`, or gameplay crates.
- Graph definitions are data (root §8).

## Public API Guidelines

- Parameters in, poses/events out; no hidden global state.
- Hit-relevant poses used on the server must be available in a headless configuration.

## Internal Design Principles

- Evaluation is allocation-free per tick and parallelizable across entities.
- Separate presentation animation from simulation-relevant animation.

## Performance and Safety

- Scales with entity count: budget per-character cost, support LOD; clip/graph data is untrusted and validated.

## Testing Requirements

- Headless tests for sampling, blending weights, state-machine transitions; benchmarks per skeleton size.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No skeleton, clip, or blend exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- No gameplay logic (weapon states belong to frameworks).
- Do not add Luau-facing APIs here; expose through `spawn-scripting` only.
