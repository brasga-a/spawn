# ADR-0013 — Declarative Data Format and Serialization

**Status:** Accepted  
**Scope:** Human-edited definitions (weapons, movement profiles, classes, sound events, input bindings, scenes in text form) and the serialization framework used across crates.  
**Related:** [ADR-0015](0015-asset-pipeline.md) (cooked binary assets), [ADR-0021](0021-scene-ecs-relationship.md) (scene data), root [Data-Driven Design](../../AGENTS.md#data-driven-design-and-scripting-security).

## Context

Root rules require static configuration as declarative data, versioned, validated at load with errors naming file, field and reason, and hot-reloadable. Content will be edited by developers, designers and, later, modders. Shipping builds need fast loading.

## Decision

1. **Two representations:**
   - **Authoring (text): RON.** Human-editable, supports comments, structs, enums and tuples, maps directly to Rust types.
   - **Runtime (binary): in-house cooked binary formats** produced by `spawn-cli` ([ADR-0015](0015-asset-pipeline.md)). Development builds may load text directly for hot reload; shipping builds load cooked data.
2. **Serialization framework: `serde`** with the `ron` crate for text. `serde` is the de facto standard; its derive model keeps definitions typed. Reflection ([`spawn-reflect`](../../crates/spawn-reflect/AGENTS.md)) is used for editor inspection and scene components, not as a replacement for `serde`.
3. **Versioning:** every top-level file declares a `version` field. Loaders support the current version and explicit migrations from older versions; unknown versions and unknown fields are errors (`deny_unknown_fields`).
4. **Validation:** after deserialization, each definition type runs validation (ranges, references to other definitions/assets, NaN/inf). Errors report file, field path, and reason.
5. **One definition per concept per file** where practical, so hot reload and version control diffs stay small.
6. **Modding:** mods use the same text format and the same validation; mod data is untrusted (size limits, depth limits, no panics).

## Alternatives

| Format | Pros | Cons |
| --- | --- | --- |
| **RON** (chosen) | Rust-native types, enums, comments | Less known to non-Rust modders; smaller tool ecosystem |
| TOML | Familiar, simple | Awkward for nested data and enums |
| JSON | Universal tooling | No comments, verbose, no enums |
| YAML | Readable | Ambiguous parsing rules, security pitfalls |
| In-house format and parser | Full control | Cost without a clear benefit over RON |

- **No `serde` (in-house serialization via `spawn-reflect`):** full control; large effort duplicating a mature, standard framework. Rejected for now.

## Consequences

- `serde` and `ron` become early dependencies (justified per root rules) once the first definition exists.
- Error messages from `ron` are wrapped to include Spawn context.

## Risks

- **`serde` derive compile-time cost.** Mitigation: derives only on definition types; measure build times.
- **Breaking changes in definitions** invalidate content. Mitigation: versioning and migrations are mandatory, with tests per migration.
- **Untrusted mod data** causing excessive allocation. Mitigation: file size limits and nesting limits before parsing.

## Revisit triggers

Modders report the format as a barrier; `serde` compile time becomes a measured problem; an editor workflow needs a format RON cannot support.

## Acceptance conditions

Accepted by the owner. Applying it is an implementation task.
