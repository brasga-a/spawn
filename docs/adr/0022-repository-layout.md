# ADR-0022 — Repository Layout for Frameworks, Examples, Tests and Games

**Status:** Proposed  
**Scope:** `frameworks/`, `examples/`, `tests/`, and where game projects live. Resolves root open question on these directories.  
**Related:** root [Layers and Boundaries](../../AGENTS.md#layers-and-boundaries) and [Workspace](../../AGENTS.md#workspace).

## Context

`frameworks/`, `examples/` and `tests/` exist as empty directories. The workspace members are `.`, `crates/*` and `tools/*`. In Cargo, a `tests/` directory next to a package manifest holds that package's integration tests, so root `tests/` already belongs to the root `spawn-engine` package.

## Decision

1. **`frameworks/`:** gameplay framework crates, one directory per framework, added to the workspace with a `frameworks/*` member glob when the first framework crate exists. Naming: `spawn-framework-<genre>` (first: `spawn-framework-fps`). Frameworks depend on engine crates only; engine crates never depend on frameworks.
2. **`examples/`:** small runnable programs demonstrating engine features, each its own package (`examples/<name>`, added through an `examples/*` member glob when the first one exists), named `spawn-example-<name>`. Examples are not games and contain no framework code unless demonstrating a framework.
3. **`tests/`:** integration tests of the root `spawn-engine` package (Cargo convention), e.g. end-to-end startup/shutdown tests that are headless. Crate-specific integration tests live in each crate's own `tests/` directory.
4. **Game projects** (including the FPS validation game) live in **separate repositories** that depend on Spawn (path or git dependency). This keeps game content, assets and licenses out of the engine repository and proves the engine is usable as an external dependency.
5. **Assets for tests and examples** live next to the example or test that uses them, kept small; large content belongs to game repositories.

## Alternatives

- **Game inside the engine repository:** convenient early on, but blurs the engine/game boundary and mixes licenses and large assets into the engine history.
- **Cargo `[[example]]` targets in the root package:** simple, but examples share the root package's dependencies, violating the "link only what you use" rule.

## Consequences

- Adding the first framework or example changes the workspace member list (an implementation change).
- The FPS validation game needs its own repository when gameplay work starts.

## Risks

- **Cross-repository changes** (engine + game) need coordination. Mitigation: path dependencies during development; versioned engine releases later.

## Revisit triggers

Coordinating separate repositories becomes a measurable burden.

## Acceptance conditions

Owner review before the first framework or example crate.
