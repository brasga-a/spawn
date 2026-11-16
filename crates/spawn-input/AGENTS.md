# spawn-input

Local rules for `spawn-input` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Turns raw device events into semantic input: device abstraction, per-frame/tick input state, named actions, and configurable mappings. Produces the intents the simulation and network layer consume.

## Responsibilities

- Device abstraction for keyboard, mouse, gamepad, touch (PLANNED) and device connect/disconnect.
- Input state (pressed/held/released, axes) and action layer with bindings/rebinding, contexts, dead zones, sensitivity.
- Sampling input into timestamped, serializable input commands suitable for fixed-timestep simulation, client prediction, and input replay ([root: Multiplayer](../../AGENTS.md#multiplayer-foundational)).
- Input recording/playback support if a feature requires it.

## Non-Responsibilities

- OS windows and raw system events (`spawn-platform`) — this crate consumes them, it does not own them.
- File access for mapping persistence (`spawn-io`, data format per [root: Data-Driven Design and Scripting Security](../../AGENTS.md#data-driven-design-and-scripting-security)), UI focus/navigation (`spawn-ui`), network transport (`spawn-network`).
- Gameplay reaction to actions (frameworks, Luau): input never decides outcomes.

## Architectural Boundaries

- Receives raw events from `spawn-platform` through types owned by `spawn-platform` ([ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md), accepted), depending on it without its native backend feature; must not depend on `spawn-render`, `spawn-physics`, or gameplay crates.
- Action definitions are data; no genre-specific actions (`Fire`, `Reload`) in this crate: games/frameworks define them.

## Public API Guidelines

- Distinguish raw events from semantic actions in the type system.
- Input commands are small, `Copy`-friendly, and serialization-friendly; they are intents, never state.

## Internal Design Principles

- Deterministic sampling: the same event sequence yields the same command sequence.
- Backend-agnostic: testable with synthetic devices.

## Performance and Safety

- Low-latency path; no allocation per event in steady state. Network-received commands are untrusted and must be validated (ranges, NaN) before use.

## Testing Requirements

- Headless tests with synthetic event streams; mapping/rebinding and dead-zone tests; replay-determinism tests.

## Governing Decisions

- **Accepted (binding):** [ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md) (raw event ownership), [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (fixed-step loop, tick index, threading). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0018](../../docs/adr/0018-gamepad-input.md) (gamepad input).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): none beyond workspace-wide test tools. Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No device, input state, or mapping exists.

## Agent Instructions

- Do not add windowing or OS event code.
- Do not hard-code genre actions or key bindings.
