# ADR-0024 — Packages and Game Extension Modules

**Status:** Proposed  
**Scope:** How a game is extended after its base release: official expansions/DLC, content updates, first-party extension modules, and (through [ADR-0025](0025-mods-and-workshop.md)) user mods. Also how game and framework code extends the engine.  
**Related:** [ADR-0013](0013-declarative-data-format.md) (data format), [ADR-0015](0015-asset-pipeline.md) (assets), [ADR-0017](0017-luau-scripting-runtime.md) (Luau), [ADR-0023](0023-platform-services-integration.md) (entitlements).

## Context

A game needs to grow: new maps, weapons, modes, DLC, seasonal content, and community mods. Without a common model, each kind of extension invents its own loading, versioning and trust rules. Rust has no stable ABI, so dynamically loading compiled Rust plugins at runtime is fragile and unsafe.

## Decision

### Two extension levels

1. **Compile-time extension (code, Rust):** the primary way to implement gameplay; a game may be entirely Rust. Games and frameworks extend the engine by **registering** systems, components, resources, asset loaders, data definition types and script APIs through engine registration APIs at startup. Linking is static. **No dynamic loading of Rust libraries** (`dylib`/`cdylib` plugins) and **no native-code mods**: native code from packages would be arbitrary code execution.
2. **Runtime extension (content, packages):** everything added after the build ships is a **package** containing only data, assets and, optionally, Luau scripts. A Rust-only game ships packages without scripts; new Rust gameplay code arrives through a game update.

### Packages

3. **One package format for all runtime extensions:** base game content, official expansions/DLC, content updates and user mods. A package is a directory (development) or archive (shipping) with a manifest (RON, [ADR-0013](0013-declarative-data-format.md)):
   - `id` (stable, namespaced, e.g. `studio.expansion_desert`), `version` (semantic), `name`, `authors`;
   - `game_api_version` (the game's script/data API version it targets, [ADR-0017](0017-luau-scripting-runtime.md));
   - `dependencies` (package IDs with version ranges) and optional `conflicts`;
   - `kind`: `base`, `official`, or `mod`;
   - `scope`: `client` (cosmetic/presentation), `server` (gameplay rules), or `shared`;
   - content: data definitions, cooked assets, Luau scripts.
4. **Trust levels:**

   | Level | Who | Allowed |
   | --- | --- | --- |
   | `base` / `official` | The game's developer, **signed** | Full game API; may require an entitlement (DLC ownership through [ADR-0023](0023-platform-services-integration.md)) |
   | `mod` | Third parties, **untrusted** | Sandboxed Luau and validated data only ([ADR-0025](0025-mods-and-workshop.md)) |

5. **Resolution and loading:** a planned `spawn-packages` crate (`PLANNED`) parses manifests, resolves dependencies and load order deterministically, verifies signatures and content hashes, and mounts package content through `spawn-io` virtual mounts for `spawn-assets` and `spawn-scripting`. Conflicting overrides are reported, never resolved silently.
6. **Multiplayer consistency:** the server announces its active package set (IDs, versions, content hashes). Clients must match `server` and `shared` packages exactly; `client` packages are local only and cannot change gameplay.
7. **Overrides:** a package may add content and override content by asset ID or definition ID; overrides are explicit in the manifest so tools can show what a package changes.

## Alternatives

- **Dynamic Rust plugins (`libloading`):** powerful, but unstable ABI, version coupling, and arbitrary code execution; rejected.
- **WASM modules for mods:** sandboxable compiled code; a possible future option if Luau proves insufficient for heavy mods, by its own ADR.
- **Separate formats for DLC and mods:** duplicated loaders and rules; rejected.

## References

Factorio mod/package model (manifests, dependencies, deterministic multiplayer sync), Bethesda plugin load order (conflict pitfalls to avoid), Valve Source engine content mounting, Bevy plugin registration model (compile-time extension).

## Consequences

- The base game itself is a package; the engine loads all content the same way.
- DLC is content-only; DLC that needs new engine or game code ships as a game update plus a package.

## Risks

- **Load-order and override conflicts** between many packages. Mitigation: explicit overrides, conflict reporting, deterministic ordering, tooling to inspect.
- **API version drift** breaking old packages. Mitigation: `game_api_version`, compatibility checks at load, migration notes in the API changelog.
- **Signature key management** for official packages. Mitigation: keys outside the repository; documented release process.

## Revisit triggers

Mods need capabilities Luau cannot provide; package counts make resolution slow.

## Acceptance conditions

Owner review. The new crates (`spawn-packages`) are created only when implementation starts.
