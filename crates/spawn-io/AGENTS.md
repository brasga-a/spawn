# spawn-io

Local rules for `spawn-io` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Low-level data input/output: filesystem operations, byte streams, virtual paths/mounts, and resource access primitives that other crates build on. Not about user input devices.

## Responsibilities

- Filesystem access, path handling, and stream/reader/writer abstractions (PLANNED).
- Virtual file systems/mounts (e.g. packaged archives vs. loose files) if a feature requires them.
- Sync and async read primitives with explicit error types.
- File watching primitives for hot reload, if needed.

## Non-Responsibilities

- Keyboard/mouse/gamepad input (`spawn-input`) and windowing/OS events (`spawn-platform`).
- Asset identity, parsing, caching (`spawn-assets`); scene/data formats (`spawn-scene`); network sockets (`spawn-network`).
- Exposing any I/O to Luau: scripts get no filesystem access ([root: Data-Driven Design and Scripting Security](../../AGENTS.md#data-driven-design-and-scripting-security)).

## Architectural Boundaries

- Foundation-level: depends on no higher `spawn-*` crate. Consumers: `spawn-assets`, `spawn-scene`, tools.
- Platform-specific behavior stays behind this crate's API; coordinate with `spawn-platform` only where OS integration is unavoidable, without duplicating it.

## Public API Guidelines

- Typed errors (path, operation, cause); no panics on I/O failure.
- Paths are validated; traversal outside permitted roots is rejected where sandboxing applies.

## Internal Design Principles

- Backends are swappable behind a narrow interface to allow in-memory test sources.
- No global working-directory assumptions.

## Performance and Safety

- Bounded reads for untrusted sizes; avoid copying large buffers; async paths must not block the simulation tick.

## Testing Requirements

- Tests against temp directories and in-memory backends; path-traversal and error-path tests.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No filesystem, stream, or path abstraction exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not add input-device or windowing code.
- Do not add asset parsing; stop at bytes and streams.
- Proposed decisions affecting this crate (follow them once accepted; until then, do not implement contrary designs): [ADR-0015](../../docs/adr/0015-asset-pipeline.md) (archive mounts for shipping builds).
