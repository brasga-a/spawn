# AGENTS.md — Spawn Engine

Authoritative architectural reference for coding agents. Nested `AGENTS.md` files (one per crate/tool, linked in [Subsystem Responsibility Map](#subsystem-responsibility-map)) define local rules and must not contradict this file. If code and documentation disagree, say so explicitly and propose which should change; never diverge silently.

**Status vocabulary** (used throughout all `AGENTS.md` files):

| Tag | Meaning |
|---|---|
| `EXISTS` | Verified implementation in source. |
| `SCAFFOLD` | Directory/package exists without meaningful implementation. |
| `PLANNED` | Architectural intention, not implemented. Never describe as existing. |

Never invent crates, modules, dependencies, or APIs. Verify against the tree and `Cargo.toml` files before asserting anything exists.

---

## Vision

Spawn is a modular, high-performance, **general-purpose** game engine: Rust core, embedded Luau for gameplay. The validation project is a multiplayer FPS (inspired by Black Ops II, Battlefield 4, Hardline): large-scale combat, responsive movement, weapons, vehicles, classes, advanced animation and physics, extensible gameplay. FPS is the first use case, not a limitation; RPG, racing, simulation, strategy, and platformers must remain feasible.

Priorities: performance, modularity, maintainability, extensibility. Generality comes from **proven requirements**, not speculation: extract to infrastructure only when a genre-independent need is demonstrated.

## Layers and Boundaries

```
Game Projects        game-specific rules, content, configs, scripts
Gameplay Frameworks  optional, genre-specific (FPS first)
Engine Infrastructure generic systems; no genre knowledge
```

Dependencies point **down only**, are explicit, and acyclic.

- **Engine infrastructure** (`crates/spawn-*`): no genre concepts (`Weapon`, `Health`, `Team`, `Vehicle`…). Never depends on frameworks, games, or tools.
- **Gameplay frameworks**: independent, genre-specific, depend on infrastructure only; one framework must not depend on another without explicit agreement. Intended home: `frameworks/` (`SCAFFOLD`: empty directory, not a workspace member). Only the FPS framework is in scope; do not build others.
- **Game projects / examples**: `examples/` (`SCAFFOLD`: empty). Root package `spawn-engine` (`src/main.rs`) is a placeholder sandbox binary.
- **Tools** (`tools/*`): development tooling. Engine crates must never depend on tools. Tools depend on engine crates.
- Subsystems are optional: a game must not be forced to link or initialize systems it does not use.

## Rust and Luau

**Rust owns** infrastructure and orchestration, rendering/GPU, ECS and simulation, physics, animation, audio, networking/replication, input and platform integration, asset management, and anything performance-critical or that scales with entity count per tick.

**Luau owns** gameplay behavior: character/weapon behavior and rules, abilities, game modes, gameplay events, AI behavior orchestration, modding.

Luau is invoked on **events and decisions**, never as the simulation loop: no per-entity per-tick hot loops, physics math, hit registration, bulk projectile processing, or pathfinding inner loops in Luau.

Integration layer: `mlua` with the `luau` feature (`PLANNED`; not a dependency). Before adding it, verify current mlua docs for supported Luau features; pin it in `[workspace.dependencies]`. All `mlua` usage is confined to `spawn-scripting`; no other crate may import `mlua` types. Prefer a command/event queue across the boundary, applied by Rust at defined sync points.

## Workspace

Root `Cargo.toml` (verified): `resolver = "3"`, members `.`, `crates/*`, `tools/*`; `[workspace.package]` provides `version` and `edition` (2024), inherited by every member via `version.workspace = true` / `edition.workspace = true`. No `[workspace.dependencies]` exist yet. **Every package currently has zero dependencies** (including inter-crate), so no actual dependency edges exist; all edges in nested documents are `PLANNED`.

Conventions:
- One crate per subsystem with a narrow public API; names `spawn-<subsystem>`.
- Add a dependency edge only when real code needs it; lower layers must not depend on higher ones.
- Do not assume every crate depends on `spawn-core`. Prefer small shared contracts and dependency inversion (the higher layer implements a trait/contract owned by the lower one) over upward references.
- Third-party deps: state what, why not std/hand-written, maintenance health, compile-time cost, licence; declare in `[workspace.dependencies]` with `default-features = false` where possible. Candidates (unadopted; verify first): `winit`, `wgpu`, `mlua`, `glam`, a physics library (build vs. integrate: decide deliberately and record why), `tracing`, `egui` (editor only).
- Workspace membership (`.`, `crates/*`, `tools/*`) matches the directories on disk: the root package, 18 packages under `crates/`, and 3 under `tools/`. Each has an `AGENTS.md` (map below); packages outside those globs are not part of the workspace.

## Subsystem Responsibility Map

Intended layering (`PLANNED`, non-binding until code exists): foundations (math, core, profiler, reflect, io) → data/runtime (ecs, assets, scene, world) → domain systems (platform, input, render, physics, animation, audio, network, ui, scripting) → frameworks → games. Tools sit beside the top.

| Crate | Owns (one line) | Status |
|---|---|---|
| [spawn-core](crates/spawn-core/AGENTS.md) | Lifecycle/orchestration contracts shared by subsystems; fixed-timestep loop contract | SCAFFOLD |
| [spawn-math](crates/spawn-math/AGENTS.md) | Vector/matrix/quaternion/geometry primitives | SCAFFOLD |
| [spawn-ecs](crates/spawn-ecs/AGENTS.md) | Entity identity, component storage, queries, system scheduling | SCAFFOLD |
| [spawn-world](crates/spawn-world/AGENTS.md) | Runtime worlds: lifecycle, activation, load/unload, streaming | SCAFFOLD |
| [spawn-scene](crates/spawn-scene/AGENTS.md) | Scene data model, hierarchy, (de)serialization, asset/script references | SCAFFOLD |
| [spawn-assets](crates/spawn-assets/AGENTS.md) | Asset identity, loading, caching, dependencies, hot reload | SCAFFOLD |
| [spawn-io](crates/spawn-io/AGENTS.md) | Filesystem/stream/byte-level resource access | SCAFFOLD |
| [spawn-reflect](crates/spawn-reflect/AGENTS.md) | Runtime type metadata for serialization, editor, script exposure | SCAFFOLD |
| [spawn-platform](crates/spawn-platform/AGENTS.md) | OS integration: windows, system events, surfaces | SCAFFOLD |
| [spawn-input](crates/spawn-input/AGENTS.md) | Input devices, state, actions, mappings | SCAFFOLD |
| [spawn-render](crates/spawn-render/AGENTS.md) | GPU abstraction, resources, shaders, materials, frame execution | SCAFFOLD |
| [spawn-physics](crates/spawn-physics/AGENTS.md) | Rigid-body simulation, collision, spatial queries | SCAFFOLD |
| [spawn-animation](crates/spawn-animation/AGENTS.md) | Skeletal/property animation runtime, blending, state machines | SCAFFOLD |
| [spawn-audio](crates/spawn-audio/AGENTS.md) | Audio playback, mixing, spatialization | SCAFFOLD |
| [spawn-network](crates/spawn-network/AGENTS.md) | Transport, connections, replication, prediction support | SCAFFOLD |
| [spawn-ui](crates/spawn-ui/AGENTS.md) | In-game UI runtime (layout, widgets, HUD) | SCAFFOLD |
| [spawn-scripting](crates/spawn-scripting/AGENTS.md) | Luau VM, sandbox, script lifecycle, exposed API | SCAFFOLD |
| [spawn-profiler](crates/spawn-profiler/AGENTS.md) | Timing, counters, tracing/diagnostic instrumentation | SCAFFOLD |
| [spawn-cli](tools/spawn-cli/AGENTS.md) | Command-line project/build/validate/asset tooling | SCAFFOLD |
| [spawn-editor](tools/spawn-editor/AGENTS.md) | Graphical authoring tools | SCAFFOLD |
| [spawn-mcp](tools/spawn-mcp/AGENTS.md) | MCP server for external AI agents/tools | SCAFFOLD |

Verified scaffold state: every `lib.rs` is the unmodified `cargo new --lib` stub (`add` function and `it_works` test). That stub carries no engine meaning and is not API.

Ownership rule: each piece of state has exactly one owning system; others use its API. If a responsibility seems to fit two crates, stop and report rather than duplicate it.

## ECS and System Design

- Composition over inheritance; components are plain data, systems are behavior. Genre-free in infrastructure.
- ECS storage (`spawn-ecs`) is distinct from world management (`spawn-world`) and scene data (`spawn-scene`).
- Data-oriented layouts; no per-entity heap allocation or global locks in hot paths; design critical systems (ECS storage, replication, physics broadphase, scheduling) for scale, but do not optimize prematurely.
- External references to entities, assets, and script-visible objects are **generational handles/IDs**, not references or pointers.
- Simulation runs on a **fixed timestep**, decoupled from render rate; simulation state is separate from presentation state.
- No infrastructure or abstraction without a named concrete feature that needs it.

## Multiplayer (foundational)

- **Server-authoritative.** Rust owns authoritative state, physics, hit registration, movement validation, replication, prediction/reconciliation.
- Clients send inputs/intents, never state, and never decide outcomes (hits, damage, kills, pickups).
- Gameplay systems must run as listen-server, dedicated server, and client-predicted paths. For every gameplay feature state: server vs. client ownership, what is replicated, what is predicted, and what happens on misprediction.
- Determinism where it pays off (movement prediction, input replay); do not assume full physics determinism.
- Scale targets (interest management, delta snapshots, bandwidth budgets) inform data layout now; implement after basic connectivity works.

## Data-Driven Design and Scripting Security

**Data:** static configuration (characters, classes, movement profiles, weapon stats, ballistics, attachments, abilities, animation references) is declarative data, not Luau and not Rust constants. Luau implements behavior only. Format is undecided (`PLANNED`; decide once, record here). Definitions are versioned and validated at load with errors naming file, field, and reason. Hot reload is a goal for data and scripts. Do not build a pipeline before a concrete feature needs it.

**Scripting** (all `PLANNED`):
- Allowlist-only API exposure, default deny; the boundary is law. Never expose raw Rust structures, pointers, ECS internals, or filesystem/OS/network access; never bypass the boundary for convenience. Missing API is added deliberately: documented, versioned, tested.
- Sandboxed environments (no `os`, `io`, `debug`, dynamic code loading, FFI in untrusted contexts); per-VM instruction/time and memory limits, stack limits, per-frame time budget.
- Error isolation: a failing script is logged with script/line and disabled or restarted; it never crashes or stalls the engine or other scripts.
- Separate client and server VMs with separate API surfaces. Client Luau is presentation/cosmetic only; server Luau defines rules through controlled APIs.
- Versioned engine→script API with a changelog; scripts declare their target version. Define hot-reload state semantics before implementing.
- Scripts are untrusted. Validate every value crossing into Rust (type, range, NaN/inf, lengths, handle validity). A stale or forged handle yields an error, never UB or a panic.

## Engineering Standards

- Stable toolchain, edition 2024. `cargo fmt`; `cargo clippy --all-targets -- -D warnings` clean. No rustfmt/clippy config or CI exists yet (`PLANNED`).
- `unsafe` only when needed (GPU/FFI/perf); each block has a `// SAFETY:` comment and, where feasible, a test.
- No `unwrap()`/`expect()` on fallible runtime paths (assets, network, scripts); use typed errors; panic only on true invariant violations.
- Untrusted input (packets, mod scripts/data, asset files) must never cause panic, unbounded allocation, or UB.
- Public APIs are small; default to `pub(crate)`. Document public items (`///`) and every script-exposed function (signature, semantics, errors, server/client availability, API version).
- Build in observability when the first real system exists (structured logging, timing, counters, debug draw), owned by `spawn-profiler`; no speculative framework.
- Small focused changes; no drive-by refactors or renames.

**Testing:** add tests for critical behavior (ECS invariants, serialization round-trips, replication/reconciliation, sandbox escapes and limit enforcement, data validation, determinism where claimed). Keep logic headless-testable (no window/GPU) and isolate GPU/window code behind thin layers. Run `cargo test --workspace` before declaring work done; state explicitly anything not run or verified.

## Agent Rules

- Check what exists; use the status tags correctly.
- Keep infrastructure genre-agnostic; gameplay goes to frameworks/scripts.
- Rust simulates, Luau decides, server is authoritative.
- Justify every dependency and abstraction. Confirm undecided choices (windowing backend, data format, physics build-vs-integrate) with the user instead of assuming.
- No speculative frameworks (RPG/racing/etc.), no invented modules, no client authority over outcomes, no bypassing the Rust/Luau boundary.
- When responsibilities are ambiguous or a nested document conflicts with this one, report it.

## Documentation Maintenance

- A change that adds a dependency edge, moves a responsibility, or changes a status tag updates the affected `AGENTS.md` files in the same change (root map, the crate's own file, and any crate whose boundary shifts).
- New crate/tool: add it to the [Subsystem Responsibility Map](#subsystem-responsibility-map) and create its `AGENTS.md` with the standard sections (Purpose, Responsibilities, Non-Responsibilities, Architectural Boundaries, Public API Guidelines, Internal Design Principles, Performance and Safety, Testing Requirements, Current Implementation Status, Agent Instructions).
- Keep status text factual and non-temporal: describe what the source contains, not when or how long.
- Nested files contain local rules only; global rules live here.

## Open Architectural Questions

1. Windowing/event backend (`winit` presumed, unconfirmed) and whether raw event types are owned by `spawn-platform` or `spawn-input` (see both documents).
2. Declarative data format (RON/TOML/JSON) and versioning scheme.
3. Physics: build vs. integrate an existing library.
4. Dependency relationship between `spawn-scene` and `spawn-ecs` (does scene data reference ECS component types directly, or go through `spawn-reflect`?).
5. Where fixed-timestep loop and system scheduling live: `spawn-core` (loop contract) vs. `spawn-ecs` (scheduler). Current documents split them; confirm.
6. Whether `frameworks/`, `examples/`, and `tests/` (empty directories) become workspace members and their layout.
