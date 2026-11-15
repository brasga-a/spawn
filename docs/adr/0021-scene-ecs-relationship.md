# ADR-0021 — Relationship Between Scenes, Reflection and the ECS

**Status:** Proposed  
**Scope:** `spawn-scene`, `spawn-reflect`, `spawn-ecs`, `spawn-world`. Resolves root open question on scene/ECS coupling.  
**Related:** [ADR-0011](0011-ecs-architecture.md) (ECS), [ADR-0013](0013-declarative-data-format.md) (data format), [ADR-0015](0015-asset-pipeline.md) (asset identity).

## Context

Scenes must be editable, serializable data usable by the CLI and editor without a running world. The ECS owns runtime storage. If `spawn-scene` referenced ECS component types directly, it would depend on `spawn-ecs` and on every crate that defines components, creating coupling and possible cycles.

## Decision

1. **`spawn-scene` does not depend on `spawn-ecs`.** A scene is plain data: a hierarchy of nodes, each with a stable node ID (UUID) and a list of component records.
2. **Component records** reference component types by a **stable type identifier** (a registered name such as `"spawn.transform"`, versioned) and contain the component's serialized data.
3. **`spawn-reflect`** holds the type registry: each component type registers its stable identifier, version, field metadata and (de)serialization functions. Crates that define components register them; `spawn-reflect` itself knows no concrete types.
4. **`spawn-world`** instantiates scenes: it resolves type identifiers through the registry, deserializes records, and creates entities in its ECS world. Unknown or invalid components are load errors with scene file, node ID and field.
5. **Prefabs:** a scene can instance another scene asset by asset ID with per-instance overrides keyed by node ID.
6. **Script bindings:** component records may reference scripts by asset ID and API version; execution remains in `spawn-scripting`.
7. **Hierarchy at runtime** (parent/child transforms) is represented with ECS components defined by the crate that owns transforms, not by `spawn-scene`.

## Alternatives

- **`spawn-scene` depends on `spawn-ecs` and component crates:** simpler code, but couples data to runtime and forbids headless scene tools that do not link every system.
- **Scenes as serialized ECS worlds:** fast to save/load, but ties the file format to storage internals.

## References

Bevy scenes and `DynamicScene` with type registry (MIT/Apache-2.0), Unity scene/prefab YAML model, Godot `.tscn` format.

## Consequences

- Dependency direction: `spawn-scene` → `spawn-reflect`; `spawn-world` → `spawn-scene`, `spawn-reflect`, `spawn-ecs`. No cycles.
- Component types need stable identifiers and versions from their first use in scenes.

## Risks

- **Registry and migration complexity** for component versions. Mitigation: migrations per component version, tested like data definitions ([ADR-0013](0013-declarative-data-format.md)).
- **Slower instantiation** than direct deserialization into storage. Mitigation: cooked scenes may store pre-resolved type indices ([ADR-0015](0015-asset-pipeline.md)).

## Revisit triggers

Instantiation time becomes a measured problem for large maps.

## Acceptance conditions

Owner review before the first scene file.
