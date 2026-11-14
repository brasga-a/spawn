# spawn-platform

Local rules for `spawn-platform` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Operating-system integration: windows, surfaces, system/window events, event loop, clipboard/cursor/display queries, and other platform-specific abstractions.

## Responsibilities

- Window creation and lifecycle, display/monitor information, surface handles for rendering (PLANNED).
- Event loop and delivery of raw OS events (window, keyboard, mouse, gamepad connection) to consumers.
- Platform-specific differences hidden behind one abstraction (Linux, Windows, macOS targets as decided).
- Backend choice (`winit` presumed) is undecided: confirm before adopting ([root: Open Architectural Questions](../../AGENTS.md#open-architectural-questions)).

## Non-Responsibilities

- Semantic input: actions, mappings, input state, device abstraction for gameplay (`spawn-input`). This crate emits *raw* platform events only.
- Rendering/GPU work (`spawn-render`) beyond handing over a surface/window handle.
- Filesystem and stream access (`spawn-io`), audio devices (`spawn-audio`), sockets (`spawn-network`).

## Architectural Boundaries

- Must not depend on `spawn-render`, `spawn-input`, or `spawn-ui`; they consume platform events/handles. Whether raw event types live here or in `spawn-input` is open ([root: Open Architectural Questions](../../AGENTS.md#open-architectural-questions)); do not create a cycle either way.
- Headless operation must be possible: the engine (server, tests, CLI) runs without linking a window backend.

## Public API Guidelines

- Expose engine-defined types, not backend types (no `winit` types in the public API).
- Event delivery is explicit about thread and ordering guarantees.

## Internal Design Principles

- Backend dependency isolated in this crate; `unsafe`/FFI for OS handles is confined here with `// SAFETY:` comments.
- Window/GPU code sits behind thin layers so logic stays headless-testable.

## Performance and Safety

- Event pump must not stall the fixed-timestep simulation; handle lost-surface/minimize/resize without panicking.

## Testing Requirements

- Headless tests of event translation and queues with synthetic events; window creation tests are manual or opt-in (not required in CI).

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No window, event loop, or OS abstraction exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not add input mapping or action logic.
- Do not select a windowing backend without user confirmation.
