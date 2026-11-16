# spawn-scene

Local rules for `spawn-scene` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Defines the scene data model and its persistence: scene structures, hierarchy, serialization/deserialization, references to assets, and references or bindings to scripts.

## Responsibilities

- Scene/prefab data structures and parent-child hierarchy (PLANNED).
- Serialization and deserialization with versioned, validated formats (format undecided, [root: Open Architectural Questions](../../AGENTS.md#open-architectural-questions)); errors name file, field, reason.
- Asset references (by asset ID, resolved by `spawn-assets`) and script references/bindings (by script identifier and declared API version; never script execution).
- Migration between scene format versions.

## Non-Responsibilities

- Running scripts, VM state, sandboxing (`spawn-scripting`).
- Runtime world orchestration, instantiation lifecycle, streaming (`spawn-world`).
- Component storage (`spawn-ecs`), asset loading/caching (`spawn-assets`), byte-level I/O (`spawn-io`).
- Authoring UI (`spawn-editor`).

## Architectural Boundaries

- Plausible dependencies: `spawn-reflect`, `spawn-math`, asset ID types. Whether it references ECS types directly is an open question ([root: Open Architectural Questions](../../AGENTS.md#open-architectural-questions)).
- Must not depend on `spawn-world`, `spawn-scripting`, render, or physics. `spawn-world`, `spawn-editor`, and `spawn-cli` consume it.
- Scene data never contains genre-specific schema in this crate; game/framework data extends it through reflection or opaque, validated component payloads.

## Public API Guidelines

- Format version is explicit; unknown versions/fields produce errors, not silent defaults.
- Scene types are plain data usable without a world, renderer, or VM so the CLI and editor can validate scenes headlessly.

## Internal Design Principles

- Hierarchy is acyclic by construction or validated on load; reject cycles.
- Stable IDs for nodes so references survive edits and hot reload.

## Performance and Safety

- Scene files are untrusted: bound sizes, depth, and node counts; no panics or unbounded allocation on malformed input.
- Deserialization avoids per-node allocation amplification.

## Testing Requirements

- Round-trip tests, golden-file tests per format version, migration tests.
- Fuzz/property tests on malformed and adversarial inputs (deep nesting, cycles, huge counts).

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No scene structure or serializer exists.

## Agent Instructions

- Do not add script execution or world instantiation here.
- Do not choose the serialization format without confirming and recording it in [root: Data-Driven Design and Scripting Security](../../AGENTS.md#data-driven-design-and-scripting-security).
- Proposed decisions affecting this crate (follow them once accepted; until then, do not implement contrary designs): [ADR-0021](../../docs/adr/0021-scene-ecs-relationship.md) (no dependency on `spawn-ecs`), [ADR-0013](../../docs/adr/0013-declarative-data-format.md) (RON).
