# spawn-platform

Local rules for `spawn-platform` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Operating-system integration: windows, surfaces, system/window events, event loop, clipboard/cursor/display queries, and other platform-specific abstractions.

## Responsibilities

- Window creation and lifecycle, display/monitor information, surface handles for rendering (PLANNED: backends are decided, not implemented).
- Event loop and delivery of raw OS events (window, keyboard, mouse, gamepad connection) to consumers.
- Platform-specific differences hidden behind one abstraction (Windows and Linux; see [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md)).
- In-house OS backends for Windows (Win32) and Linux (Wayland and X11); macOS is not supported. Spawn owns the event loop. Accepted in [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md). Per [ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md) (accepted), the native backend is compiled only behind a Cargo feature; raw event types are available without it.

## Non-Responsibilities

- Semantic input: actions, mappings, input state, device abstraction for gameplay (`spawn-input`). This crate emits *raw* platform events only.
- Rendering/GPU work (`spawn-render`) beyond handing over a surface/window handle.
- Filesystem and stream access (`spawn-io`), audio devices (`spawn-audio`), sockets (`spawn-network`).

## Architectural Boundaries

- Must not depend on `spawn-render`, `spawn-input`, or `spawn-ui`; they consume platform events/handles. Raw event types live here ([ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md), accepted); never depend on `spawn-input`.
- Headless operation must be possible: the engine (server, tests, CLI) runs without linking a window backend.

## Public API Guidelines

- Expose engine-defined types, not backend types (no OS handles, binding-crate types, or protocol objects in the public API).
- Exception: the window type implements the `raw-window-handle` traits so `spawn-render` can create a surface (SPN-INV-011, [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md)). Expose only those traits/handle contracts, nothing else from backend crates.
- Event delivery is explicit about thread and ordering guarantees.

## Internal Design Principles

- OS calls are confined to per-OS backend modules behind Cargo features; all `unsafe`/FFI lives there with `// SAFETY:` comments.
- Window/GPU code sits behind thin layers so logic stays headless-testable.

## Performance and Safety

- Event pump must not stall the fixed-timestep simulation; handle lost-surface/minimize/resize without panicking.

## Testing Requirements

- Headless tests of event translation and queues with synthetic events; window creation tests are manual or opt-in (not required in CI).

## Governing Decisions

- **Accepted (binding):** [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md) (platform layer), [ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md) (raw event ownership), [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (fixed-step loop, tick index, threading). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0018](../../docs/adr/0018-gamepad-input.md) (gamepad input).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `windows-sys`, `wayland-client`, `wayland-protocols`, `x11rb`, `xkbcommon`, `raw-window-handle`, `rustix`, `evdev`, `udev`, `hidapi`. Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No window, event loop, or OS abstraction exists.

## Agent Instructions

- Do not add input mapping or action logic.
- The backends are decided ([ADR-0001](../../docs/adr/0001-in-house-platform-layer.md)): Win32 on Windows, Wayland and X11 on Linux, an in-house abstraction, and a Spawn-owned event loop; macOS is out of the target matrix. Do not reopen this choice without a new ADR.
- Decided is not implemented: no backend exists yet (`SCAFFOLD`); implement each backend behind its Cargo feature and record its smoke test before claiming support.
