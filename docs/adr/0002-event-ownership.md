# ADR-0002 — Platform Events and Input Ownership

**Status:** Proposed  
**Scope:** Separation of `spawn-platform`, `spawn-input`, and the composition root.

## Context

The platform backend receives native device/window messages. Gameplay needs semantic, tick-sampled input. Mixing these responsibilities would entangle windowing, simulation and networking.

## Proposed decision

`spawn-platform` owns translation from OS/backend events into small **raw event types**, with window/device identity, ordering and focus/resize lifecycle information where needed. `spawn-input` consumes these events to build input state, edge transitions and caller-defined semantic actions. The executable routes events and decides whether a close request shuts down the process.

The raw input event representation may be defined by `spawn-platform` or by a narrowly shared lower-level contract if and only if a real dependency problem requires it; do not make `spawn-platform` depend on `spawn-input`. Do not define per-game actions in either crate.

## Alternatives

- Raw types owned by `spawn-input`, forcing platform to reference input: simpler event reuse but violates the intended one-way dependency unless adapters invert control.
- Backend types passed directly to all consumers: smaller initial code but leaks `winit` (or equivalent) into independent crate APIs.
- Shared event crate from day one: extra abstraction without evidence of multiple consumers.

## Consequences if accepted

- Input can be tested with synthetic raw event streams in headless CI.
- Focus loss and reconnect semantics are documented; lost releases cannot cause permanently held actions.
- Cross-event ordering and tick sampling must be defined before implementation.

## Acceptance conditions

Demonstrate the planned dependency direction with a minimal interface sketch, define the input transition sampling policy and focus-reset behavior, and review with `spawn-input`/`spawn-platform` owners.
