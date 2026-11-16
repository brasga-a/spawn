# spawn-reflect

Local rules for `spawn-reflect` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Runtime type metadata so other systems can inspect and manipulate data generically: serialization, editor inspectors, data validation, and (through `spawn-scripting`) controlled script exposure.

## Responsibilities

- Type/field descriptors, attributes, and registry (PLANNED; mechanism, e.g. derive macro, undecided).
- Generic get/set access to described values with checked types.
- Metadata that supports serialization and validation by other crates.

## Non-Responsibilities

- Serialization formats and files (`spawn-scene`, `spawn-assets`), component storage (`spawn-ecs`), editor UI (`spawn-editor`).
- Deciding what scripts may access: allowlisting is `spawn-scripting`'s job. Reflection availability never implies script exposure.

## Architectural Boundaries

- Foundation-level; depends on no higher `spawn-*` crate. ECS, scene, editor, and scripting may depend on it.
- A proc-macro would require a separate crate; not created here. Confirm before adding one.

## Public API Guidelines

- Metadata is read-only by default; mutation goes through checked accessors.
- Opt-in per type/field; default is not reflected.

## Internal Design Principles

- Prefer static, compile-time-generated metadata over runtime discovery; keep registry lookups off hot paths.

## Performance and Safety

- Dynamic access must be type- and bounds-checked; mistyped access returns errors, not UB or panics.

## Testing Requirements

- Round-trip get/set tests, nested/enum cases, mistyped-access rejection.

## Governing Decisions

- **Accepted (binding):** none specific to this crate. Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0021](../../docs/adr/0021-scene-ecs-relationship.md) (scene/ECS relationship).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): none beyond workspace-wide test tools. Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No type metadata or derive exists.

## Agent Instructions

- Do not add serialization formats or editor UI here.
- Do not treat reflection as the scripting allowlist.
