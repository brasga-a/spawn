# spawn-scripting

Local rules for `spawn-scripting` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

The only crate that integrates Luau (via `mlua`, PLANNED): VM management, script execution, sandboxing, script lifecycle, and the controlled, versioned engine API exposed to scripts.

## Responsibilities

- Luau VM creation, per-VM memory/instruction/time limits, recursion and stack limits.
- Sandbox environments: allowlisted globals only; no `os`, `io`, `debug`, dynamic code loading, FFI.
- Script lifecycle: load, init, event dispatch, shutdown, unload, reload (state semantics defined before hot reload is implemented).
- Separate client and server VMs with separate API surfaces.
- The exposed API: documented, versioned, with a changelog; validation of every value crossing into Rust; generational handles only.
- Command/event queue applying script effects at sync points; error isolation and logging with script/line context.

## Non-Responsibilities

- Scene structure and script *references* (`spawn-scene`); world orchestration (`spawn-world`).
- Simulation loops, physics, hit registration, per-entity per-tick logic (Rust systems).
- Gameplay content and rules (scripts in games/frameworks); config data (declarative data files).
- Runtime type metadata (`spawn-reflect`) — available for implementing the API, but it never defines what is exposed.

## Architectural Boundaries

- Sole owner of `mlua`; no other crate imports `mlua` types ([root: Rust and Luau](../../AGENTS.md#rust-and-luau)). Verify mlua's Luau feature support before adding it and pin in `[workspace.dependencies]`.
- Higher layer: may depend on `spawn-ecs`, `spawn-math`, `spawn-reflect`, `spawn-assets` as needed; engine systems must not depend on it — they define traits/events/commands it implements or consumes.
- Must not expose raw Rust structures, pointers, ECS internals, or filesystem/OS/network access.

## Public API Guidelines

- Allowlist, default deny. Each exposed function documents signature, semantics, errors, server/client availability, and API version introduced.
- Breaking changes bump the API version; scripts declare the version they target.

## Internal Design Principles

- Scripts are untrusted. Errors are values with context; a failing script is disabled or restarted without affecting others.
- Prefer events/decisions over frequent calls; batch crossings.

## Performance and Safety

- Per-frame script time budget enforced via interrupt callbacks; stale/forged handles → error; NaN/inf/length validation on every argument; no panics reachable from script input.

## Testing Requirements

- Sandbox-escape tests, limit-enforcement tests (infinite loop, memory bomb, deep recursion), handle-validity tests, client/server API separation tests, versioning tests, error-isolation tests.

## Governing Decisions

- **Accepted (binding):** [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0017](../../docs/adr/0017-luau-scripting-runtime.md) (Luau runtime), [ADR-0024](../../docs/adr/0024-packages-and-game-extensions.md) (packages and extensions), [ADR-0025](../../docs/adr/0025-mods-and-workshop.md) (mods and Workshop).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `mlua` (feature luau), once ADR-0017 is accepted. Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No Luau or `mlua` integration exists.

## Agent Instructions

- Never add an API without documentation, version, tests, and server/client scoping.
- Never expose a convenience shortcut around the boundary; add the API deliberately instead.
- Do not add `mlua` until the task explicitly asks for it.
