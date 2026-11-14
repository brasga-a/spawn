# ADR-0003 — Main Loop and Fixed-Step Ownership

**Status:** Proposed  
**Scope:** Runtime timing contracts, event-loop control and future ECS scheduling.

## Context

Spawn needs fixed-step simulation for future server-authoritative multiplayer, but presenting a window is driven by a native event loop with platform-specific wakeup rules. A generic engine contract must not own OS objects or decide ECS scheduling policy.

## Proposed decision

`spawn-core` defines a minimal, headless-testable fixed-step timing/lifecycle **contract**. The root `spawn-engine` composition layer drives that contract using monotonic time while responding to native event-loop callbacks/events from `spawn-platform`. `spawn-render` presents independently of the number of simulation ticks executed; `spawn-input` samples at documented tick boundaries.

Future ECS system scheduling remains owned by `spawn-ecs`, not by the generic loop contract. Do not implement ECS scheduling in the foundation milestone.

Use a finite per-wakeup simulation-step limit and explicit accumulator/backlog policy; proposed test defaults are 60 Hz fixed simulation and at most four catch-up steps, to be measured rather than permanently mandated.

## Alternatives

- The event loop lives in `spawn-core`: couples the foundational contract to OS backends and blocks headless uses.
- Rendering callbacks drive variable simulation delta: simple, but violates fixed-step needs and creates prediction/replay problems.
- ECS owns the entire application lifecycle: premature coupling of a generic runtime to one system model.

## Consequences if accepted

- Simulation is testable without a window or GPU.
- A dedicated server can reuse timing contracts without linking rendering.
- Lifecycle ownership remains explicit in the composition root.
- Platform redraw timing must not induce unbounded catch-up or duplicate input transitions.

## Acceptance conditions

Confirm thread/event-loop restrictions with the selected backend, define bounded backlog behavior (drop/clamp/etc.) and input sampling policy, and require tests for pause, catch-up, and rendering cadence independence.
