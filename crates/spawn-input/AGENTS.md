# spawn-input

Local rules for `spawn-input` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Turns raw device events into semantic input: device abstraction, per-frame/tick input state, named actions, and configurable mappings. Produces the intents the simulation and network layer consume.

## Responsibilities

- Device abstraction for keyboard and mouse (`EXISTS`), explicit device disconnect (`EXISTS`), gamepad ([ADR-0018](../../docs/adr/0018-gamepad-input.md), proposed: `PLANNED`), and touch (`PLANNED`).
- Input state (pressed/held/released, axes) and action layer with bindings/rebinding, contexts, dead zones, and sensitivity (`EXISTS`).
- Sampling input into tick-indexed, `Copy` input commands suitable for fixed-timestep simulation, client prediction, and input replay (`EXISTS`; wire serialization format `PLANNED`, see [root: Multiplayer](../../AGENTS.md#multiplayer-foundational)).
- Input recording/playback persistence (`PLANNED`, added if a concrete feature requires it).

## Non-Responsibilities

- OS windows and raw system events (`spawn-platform`) — this crate consumes them, it does not own them.
- File access for mapping persistence (`spawn-io`, data format per [root: Data-Driven Design and Scripting Security](../../AGENTS.md#data-driven-design-and-scripting-security)), UI focus/navigation (`spawn-ui`), network transport (`spawn-network`).
- Gameplay reaction to actions (frameworks, Luau): input never decides outcomes.

## Architectural Boundaries

- Receives raw events from `spawn-platform` through types owned by `spawn-platform` ([ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md), accepted), depending on it with `default-features = false` (no native windowing backend) and on `spawn-core` with `default-features = false` (for `Tick`); must not depend on `spawn-render`, `spawn-physics`, or gameplay crates.
- Action definitions are data; no genre-specific actions in this crate: games/frameworks define them.

## Public API Guidelines

- Distinguish raw events (`DeviceState`) from semantic actions (`ActionId`, `AxisId`, `ContextId`, `InputMap`, `InputCommand`) in the type system.
- Input commands are small (96 bytes), `Copy`, and serialization-friendly; they are intents, never authoritative state.

## Internal Design Principles

- Deterministic sampling: the same event sequence and tick schedule yield the bit-identical command sequence.
- Backend-agnostic: testable with synthetic events and zero native libraries.

## Performance and Safety

- `#![forbid(unsafe_code)]`; `#![deny(clippy::indexing_slicing)]`.
- Low-latency path; zero heap allocation per event or per tick in steady state (fixed bitsets and inline slots in `DeviceState` and `InputCommand`; `InputMap` allocates only when bindings are added).
- Network-received or replay-loaded commands are untrusted and must be validated (`InputCommand::validate` / `InputMap::validate_command`: ranges, `NaN`, infinities, unregistered bits, impossible transition flags) before use.

## Testing Requirements

- Headless tests with synthetic event streams; tick-boundary sampling (`FND-T03`); focus loss and device disconnect (`FND-T04`); mapping/rebinding, context switching, dead-zone, sensitivity, and untrusted-command validation tests; replay-determinism tests.

## Governing Decisions

- **Accepted (binding):** [ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md) (raw event ownership), [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (fixed-step loop, tick index, threading), [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (errors and diagnostics). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0018](../../docs/adr/0018-gamepad-input.md) (gamepad input).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): none beyond workspace-wide test tools. Any other crate needs owner approval first.

## Current Implementation Status

`EXISTS` (foundation Phase 4: keyboard and mouse device state, edge transitions, semantic action/axis/context mappings, live rebinding, tick-boundary command sampling, focus/close/disconnect reset, and untrusted-command validation); gamepads, touch, mapping file persistence, and wire codecs are `PLANNED`.

**Dependencies:** `spawn-core` (`default-features = false`) and `spawn-platform` (`default-features = false`); zero third-party normal dependencies (`cargo tree -p spawn-input -e normal`, `FND-T09`).

| Item | Module | Notes |
| --- | --- | --- |
| `InputState` | `state` | Coordinates `DeviceState`, `InputMap`, active `ContextId` bitmask, and `sample_tick(Tick) -> InputCommand` |
| `DeviceState`, `ButtonState`, `PointerDelta`, `ScrollVector`, `DeviceKind` | `device` | Allocation-free tracking of named `KeyCode`s, `native_code` keys, `MouseButton`s, cursor position, surface cursor delta, unaccelerated relative motion delta, scroll lines/pixels, and focus |
| `ActionId`, `AxisId`, `ContextId` | `action` | Validated caller-defined identifiers (`MAX_ACTIONS = 64`, `MAX_AXES = 16`, `MAX_CONTEXTS = 16`; `ContextId::GLOBAL = 0`) |
| `ButtonTrigger`, `AxisSource`, `MotionAxis`, `AxisConfig`, `InputMap` | `action` | Data-driven bindings, atomic rebinding, axial dead zone (`[0.0, 1.0)`), sensitivity (`(0.0, 10_000.0]`), inversion, optional zero-inclusive `[min, max]` clamp |
| `InputCommand`, `CommandValidation` | `command` | 96-byte `Copy` tick-indexed intent (`pressed`/`held`/`released` `u64` masks + `[f32; 16]` axes) and standalone or map-derived validation |
| `InputError`, `InputErrorKind` | `error` | Hand-written, `#[track_caller]` located, coded (`INPT-0001`..`INPT-0006`), detail, flattened cause, optional backtrace (ADR-0009) |

**Error codes:**

| Code | Kind | Meaning |
| --- | --- | --- |
| `INPT-0001` | `InvalidDeadZone` | Axis dead zone is non-finite, `< 0.0`, or `>= 1.0` |
| `INPT-0002` | `InvalidSensitivity` | Axis sensitivity is non-finite, `<= 0.0`, or above `AxisConfig::MAX_SENSITIVITY` |
| `INPT-0003` | `InvalidAxisRange` | Axis clamp range or magnitude cap has non-finite bounds, excludes neutral `0.0` (`min > 0.0` or `max < 0.0`), or `<= 0.0` |
| `INPT-0004` | `InvalidBinding` | Invalid trigger/source (e.g. `KeyCode::Unidentified` by `KeyCode`, non-finite/zero button scale, identical digital pair triggers, or binding capacity exceeded) |
| `INPT-0005` | `IdOutOfRange` | `ActionId`, `AxisId`, or `ContextId` exceeds its bounded slot count |
| `INPT-0006` | `InvalidCommand` | Untrusted `InputCommand` has non-finite/out-of-range axes, unregistered bits set, or impossible transition flags |

**Sampling and reset contracts:**

- **Tick-boundary sampling (`FND-FR-09`, `FND-INV-10`):** transitions (`pressed`, `released`) and transient pointer/scroll deltas accumulate across zero-tick frames and are consumed on the first `sample_tick` of a wakeup. Subsequent catch-up ticks in the same wakeup keep `held` and continuous button-driven axes active without duplicating press/release edges or motion/scroll deltas. Rapid sub-tick press+release sequences latch both `pressed` and `released` on that tick (`was_active() == true`, `held == false`).
- **Pointer motion:** `DeviceState::pointer_delta` prefers unaccelerated relative motion (`InputState::handle_relative_motion(dx, dy)`, aligned with Phase 2b's `WindowEvent::PointerRelativeMotion { dx, dy }`) when received during the tick interval, falling back to surface cursor displacement (`PointerMoved`) so unlocked platforms delivering both never double-count motion. Multi-window pointer focus (`pointer_window`) re-anchors across windows and ignores stale `PointerLeft` from unfocused windows. Non-input `WindowEvent` variants are ignored without panicking.
- **Focus loss, close, disconnect, context, and rebind (`FND-INV-11`):** `WindowEvent::Focused(false)`, `WindowEvent::CloseRequested`, multi-window focus/pointer transfer, `WindowEvent::PointerLeft` (releases held pointer buttons on the active pointer window), `InputState::release_held`, `InputState::disconnect_device`, disabling an active `ContextId`, `InputState::set_map` (clearing bits absent from the new map's schema while releasing registered ones), or live-rebinding away from a held trigger transitions affected held states to `false` and latches `released = true` for the next tick. `InputState::reset` clears all live/pending/sampled state immediately while keeping `InputMap` and active contexts.

**Tests (all headless, no window or GPU):** 12 unit tests (`error`: 3, `device`: 3, `action`: 4, `command`: 2), 14 verification/integration tests in `tests/input_verification.rs` (`FND-T03`: 5, `FND-T04`: 6, deterministic replay + command validation: 1, `set_map` + unilateral axis range validation: 1, non-input events & extreme floats: 1), and 1 doc test.

**Not implemented / deferred:**

- Wiring `WindowEvent::PointerRelativeMotion { dx, dy }` directly inside `DeviceState::handle_window_event_for` once Phase 2b (`swarm/phase-2b-pointer-capture`) merges with this branch (`handle_relative_motion(dx, dy)` already matches its signature).
- Gamepad devices, standard controller layout mappings, sensors, and haptics ([ADR-0018](../../docs/adr/0018-gamepad-input.md), proposed).
- Touch input, chords/modifiers, hold-duration/double-tap gesture recognizers, sub-tick high-precision event timestamps, RON mapping persistence ([ADR-0013](../../docs/adr/0013-declarative-data-format.md)), and network bitstream packing ([ADR-0016](../../docs/adr/0016-networking-and-replication.md)).

## Agent Instructions

- Do not add windowing or OS event code.
- Do not hard-code genre actions or key bindings.
- New error kinds get the next `INPT-` code; codes are never reused or renumbered.
