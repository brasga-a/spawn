# ADR-0002 — Raw Input Events Owned by `spawn-platform`

**Status:** Accepted (foundation scope; revisited when a later milestone or a measured limitation requires it).  
**Scope:** Separation of `spawn-platform`, `spawn-input`, and the composition root.  
**Related:** [ADR-0001](0001-in-house-platform-layer.md) (windowing backend).

## Context

The platform backend receives native device/window messages. Gameplay needs semantic, tick-sampled input. Mixing these responsibilities would entangle windowing, simulation and networking.

Each module must work with the fewest dependencies possible. If `spawn-input` depended on `spawn-platform` with its native backend always compiled, every consumer of input types would also link the window backend: headless tests, and a dedicated server that only deserializes input commands received from clients. A backend problem would then affect crates that never open a window, and errors would be harder to attribute.

## Decision

`spawn-platform` owns translation from OS/backend events into small **raw event types**, with window/device identity, ordering and focus/resize lifecycle information where needed. `spawn-input` consumes these events to build input state, edge transitions and caller-defined semantic actions. The executable routes events and decides whether a close request shuts down the process.

`spawn-platform` exposes its raw event types without any native dependency. The native backend ([ADR-0001](0001-in-house-platform-layer.md)) is compiled only behind a Cargo feature of `spawn-platform`, enabled by the executable that opens a window. `spawn-input` depends on `spawn-platform` **without** that feature, so it compiles and tests with no window backend.

Do not make `spawn-platform` depend on `spawn-input`. Do not define per-game actions in either crate. Do not create a separate event crate unless this feature split proves insufficient.

## Alternatives

- Raw types owned by `spawn-input`, forcing platform to reference input: simpler event reuse but violates the intended one-way dependency unless adapters invert control.
- Backend types passed directly to all consumers: smaller initial code but leaks OS or binding-crate types into independent crate APIs.
- `spawn-input` depends on `spawn-platform` with the backend always enabled: simplest manifests, but every input consumer links the native backend.
- Shared event crate from day one: extra crate without evidence that the feature split is insufficient.

## Consequences if accepted

- Input can be tested with synthetic raw event streams in headless CI, without compiling the window backend.
- A dedicated server can use input command types without linking windowing code.
- `cargo tree` on `spawn-input` must show no native backend; this is checked as part of FND-T09.
- Focus loss and reconnect semantics are documented; lost releases cannot cause permanently held actions.
- Cross-event ordering and tick sampling must be defined before implementation.

## Limits of this isolation

Minimal dependencies isolate **compilation, testing and error attribution**. They do not isolate **runtime failure**: all crates run in one process, so a panic in any crate can still stop the engine. Runtime containment comes from each subsystem returning typed errors instead of panicking (root [Engineering Standards](../../AGENTS.md#engineering-standards)).

## Acceptance conditions

Demonstrate the planned dependency direction with a minimal interface sketch, define the input transition sampling policy and focus-reset behavior, and review with `spawn-input`/`spawn-platform` owners.
