# AGENTS.md — Spawn Engine: Architecture & Engineering Guide

**Spawn Engine** is the name of this engine. This is the primary architectural and engineering reference for AI coding agents working in this repository. Read it before making changes. If code and this document disagree, say so explicitly and propose which should change — do not silently diverge.

> **Convention used below:** **[EXISTS]** = present in the repo today. **[PLANNED]** = target architecture, not yet implemented. Never describe a planned item as existing, and never invent modules, crates, dependencies, or APIs.

---

## 1. Current Repository State (as of this writing)

The repository is a **bare scaffold**. Nothing engine-related is implemented.

| Item | State |
|---|---|
| Root package `spawn-engine` (`Cargo.toml`, edition 2024, no dependencies) | [EXISTS] — `src/main.rs` is `println!("Hello, world!")` |
| Crates in `crates/` | [EXISTS] — all placeholders (`cargo new --lib` stubs): spawn-animation, assets, audio, cli, core, ecs, io, math, mcp, network, physics, platform, profiler, reflect, render, scene, scripting, ui, world. Names only indicate intended scope; none implement it |
| `tools/spawn-editor` | [EXISTS] — placeholder only |
| Cargo workspace | [EXISTS] — root `Cargo.toml`: `resolver = "3"`, members `.`, `crates/*`, `tools/*`, shared `[workspace.package]` (version, edition). No `[workspace.dependencies]` yet; no inter-crate dependencies declared yet |
| Dependencies (wgpu, mlua, winit, etc.) | **None** |
| Rendering, ECS, physics, audio, networking, input, assets, animation, Luau | **None** |
| CI, rustfmt/clippy config, docs, examples | **None** |
| Git history | No commits yet; all files untracked |

### Known issues with the scaffold (fix as part of the first real task, with justification in the commit)

1. Workspace exists. When the first external dependency is added, introduce `[workspace.dependencies]` and reference it with `dep.workspace = true`. Keep the root `spawn-engine` package as the thin runnable sandbox binary. A stray `tools./` directory (typo duplicate of `tools/spawn-editor`) is outside the workspace and should be deleted.
2. Crates are already prefixed `spawn-*` (resolved: no clash with Rust's built-in `core`). **Keep this convention for every new crate** (`spawn-<subsystem>`). Placeholder crates may be empty of engine code; don't assume their scope beyond their names.
3. Keep this granularity as the model: **one crate per subsystem with a narrow public API**. Declare inter-crate dependencies only when real code needs them, and keep them acyclic with lower layers (math/core/ecs) below higher ones (render, physics, scripting, ui). `spawn-platform` is the likely home for windowing/input; confirm with the user.

Edition is **2024** (requires a recent toolchain; local toolchain is cargo 1.99). Keep it.

---

## 2. Project Vision

**Spawn Engine** is a modern, high-performance, **general-purpose** game engine. The first use case — and the validation target for the architecture — is an FPS inspired by Call of Duty: Black Ops II, Battlefield 4, and Battlefield Hardline: large-scale multiplayer combat, responsive movement, weapons, vehicles, classes, advanced animation, physics, extensible gameplay.

FPS is the **first use case, not a limitation**. Priorities: performance, modularity, maintainability, extensibility.

---

## 3. Three-Layer Architecture

```
┌──────────────────────────────────────────────────────────────┐
│ Game Projects     game-specific rules, content, configs      │  [PLANNED]
├──────────────────────────────────────────────────────────────┤
│ Gameplay Frameworks   optional, genre-specific (FPS first)   │  [PLANNED]
├──────────────────────────────────────────────────────────────┤
│ Engine Core       generic infrastructure, no genre knowledge │  [PLANNED]
└──────────────────────────────────────────────────────────────┘
        dependencies point DOWN only. Never up.
```

1. **Engine Core** — generic, reusable technical systems. Contains **no** assumptions about genre: no "weapon", "health", "player", "vehicle", "team" concepts.
   Capabilities (target): 2D/3D rendering infrastructure, ECS & entity lifecycle, physics/collision infrastructure, skeletal animation, audio/spatial sound, input abstraction, asset pipeline, scene/world management, networking infrastructure, job scheduling/multithreading, serialization & reflection, embedded Luau runtime, plugin/extension infrastructure.
2. **Gameplay Frameworks** — independent, reusable, genre-specific crates (FPS: weapons, ballistics, character controllers, abilities, classes, vehicles, combat; later possibly RPG, racing, platformer, strategy). They depend on Engine Core, **never the reverse**. A framework must not depend on another framework unless explicitly agreed.
3. **Game Projects** — game-specific rules, content, data, and scripts built on a framework.

**Rules**
- If you are about to write `Weapon`, `Health`, `Team`, etc. in an engine-core crate, stop: it belongs in the FPS framework.
- Subsystems are optional. A game must not be forced to initialize or link systems it doesn't use (e.g. a 2D game must not pull in the vehicle physics or netcode).
- General-purpose design must come from **proven requirements**, not speculative abstraction. The FPS framework is the proving ground; extract to core only when a second need or a clear genre-independent concept exists. Do not build RPG/racing/strategy frameworks now.

---

## 4. Language Split: Rust Core, Luau Gameplay

### Rust owns (performance-critical / infrastructure)
Rendering & GPU abstraction; ECS & entity lifecycle; physics & collision; character movement & controllers; animation runtime; audio; networking & replication; input; asset management & loading; memory management & multithreading; server-authoritative simulation; hit registration; client prediction/reconciliation.

### Luau owns (gameplay behavior) — via `mlua`
Character behaviors; weapon behaviors and rules; abilities & equipment; game modes & match rules; gameplay events/interactions; AI behavior *orchestration*; custom mechanics; modding.

### Division of labor (examples)
| Rust | Luau |
|---|---|
| Computes physics | Defines gameplay response |
| Detects collisions | Reacts to collision events |
| Simulates weapons (fire loop, projectiles, hit reg) | Defines weapon behavior and rules |
| Moves characters | Defines abilities and character-specific behavior |
| Runs networking | Defines gameplay-level server logic |

**Never** implement in Luau: per-frame/per-entity hot loops, physics math, hit registration, large-scale projectile processing, pathfinding inner loops, anything that scales with entity count and runs every tick. Luau is invoked on **events/decisions**, not as the simulation loop.

### The boundary is law
- Luau reaches the engine **only** through explicitly exposed, documented APIs.
- Never expose raw Rust structures, pointers, memory, ECS internals, or filesystem/OS/network access to Luau. Expose handles/IDs (generational), not references.
- **Never bypass the Rust/Luau boundary** to make something "easier". If the API is missing, add it deliberately (documented, versioned, tested).

### `mlua` + Luau [PLANNED — not yet a dependency]
- Use `mlua` with its `luau` feature. **Before adding it, verify against current mlua docs** which features are available for Luau (e.g. `vendored`, `async`, `serialize`, `send`) and which are not; don't assume parity with Lua 5.x/LuaJIT features. Pin the version in `[workspace.dependencies]`.
- Isolate all mlua usage in a single crate (e.g. a `scripting` crate). No other crate imports `mlua` types. Engine systems talk to scripting through engine-defined traits/events/commands.
- Prefer a command/event queue across the boundary (script emits commands applied by Rust at defined sync points) over scripts mutating engine state directly mid-system.

---

## 5. Data-Driven Design

Static configuration is **data**, not code. Luau implements **behavior**, never serves as a config format.

Declarative data (format to be chosen when first needed — e.g. RON/TOML/JSON; decide once, document here, keep it consistent):
- Character attributes, classes, movement profiles
- Weapon statistics, attachments, ballistics tables
- Equipment and ability definitions
- Animation references/graphs
- Reusable gameplay components

Goals: data-driven characters/classes, configurable weapons/attachments, reusable components, **hot-reloadable** scripts and data, future modding. Data definitions are validated at load with clear error messages (file, field, reason), and are versioned.

Don't build a data pipeline until a concrete feature needs it; start with the simplest loader for the first real definition (likely a weapon or movement profile) and generalize from there.

---

## 6. Multiplayer Is Foundational

Multiplayer is an architectural requirement from day one — not a retrofit.

- **Server-authoritative.** Rust owns: authoritative game state, physics & collision, hit registration, movement validation, replication, client prediction & reconciliation, server simulation.
- Clients send **inputs/intents**, never state. Clients never decide outcomes (hits, damage, kills, pickups).
- Design gameplay systems so they run identically as listen-server, dedicated server, and client-predicted code paths. Always ask: *"What does the server do? What does the client predict? What gets replicated?"*
- Simulation uses a **fixed timestep**, decoupled from render rate. Separate simulation state from presentation state.
- Determinism: apply it where it pays off (e.g. character movement prediction/reconciliation, input replay). **Do not assume the full physics simulation is deterministic**, and don't contort the engine to make it so.
- **Client Luau** is restricted to presentation/cosmetic functionality (UI, VFX/SFX triggers, camera flourish). **Server Luau** defines gameplay rules through controlled APIs. Client and server scripts live in separate VMs with separate API surfaces; a client script must never be able to call server-only APIs.
- Scale target (BF-style large battles) implies: interest management, snapshot delta compression, bandwidth budgets, and data layouts friendly to bulk replication. Design for these; don't build them before the basic connectivity slice works.

---

## 7. Scripting Runtime, Sandboxing & Security [PLANNED]

The Luau runtime must provide:
- **Controlled API exposure** — allowlist only; default deny.
- **Sandboxed environments** — use Luau's sandbox facilities; remove/deny `os`, `io`, `debug`, `loadfile`/dynamic code loading from untrusted contexts; no raw FFI.
- **Execution limits** — instruction/time budgets (interrupt callback), memory limits per VM, recursion/stack limits, per-frame script time budget.
- **Lifecycle management** — load, init, update/event dispatch, shutdown, unload, reload.
- **Error isolation** — a failing script is logged with script/line context and disabled or restarted; it must never crash or stall the engine or other scripts.
- **Hot reload** (dev) — preserve or explicitly reset script state; define semantics before implementing.
- **Client/server separation** — distinct VMs and API sets.
- **Versioned engine→script API** — scripts declare the API version they target; breaking changes bump the version; keep a changelog of the scripting API.

**Treat all scripts as untrusted** when third-party mods are supported. Validate every value crossing from Luau into Rust (types, ranges, handle validity, NaN/inf, string lengths). A script passing a stale/forged handle must produce an error, not UB or a panic.

---

## 8. Engineering Principles

1. Performance and correctness over unnecessary abstraction.
2. Modular systems with explicit ownership and boundaries.
3. Minimize dependencies; justify each one (see §9).
4. Keep engine core independent of game mechanics.
5. Composition over deep inheritance/coupling (ECS components, small traits).
6. No premature optimization, but design critical systems (ECS storage, replication, physics broadphase, job system) with scale in mind: data-oriented layouts, no per-entity heap allocation in hot paths, no global locks in hot paths.
7. Keep Rust↔Luau interfaces explicit and stable.
8. Prefer data-driven where appropriate.
9. Build in debuggability, profiling, observability from the start (structured logging via `tracing`-style spans, frame/system timing, debug draw, stats counters) — added when the first real system exists, not as a speculative framework.
10. **No infrastructure without a concrete architectural requirement.** Every new abstraction should name the feature that needs it.

---

## 9. Development Guidelines

**Before coding**
- Inspect the existing code and `Cargo.toml`s first; follow the conventions already in the repo. Where none exist yet (most things today), propose the convention in the PR/commit and document it here.
- Check §1: do not assume something exists because the architecture says it should.

**While coding**
- Small, focused changes. No large unrelated refactors or drive-by renames.
- Define clear ownership: each piece of state has exactly one owning system; others access it through its API.
- **Dependencies:** add only with a stated reason (what it does, why not hand-write/std, maintenance health, compile-time cost, licensing). Declare in `[workspace.dependencies]`, disable default features where possible. Likely candidates when needed (not yet adopted — verify before choosing): `winit`, `wgpu`, `mlua`, a math crate (e.g. `glam`), a physics approach (build vs. integrate such as Rapier — decide deliberately and record why), `tracing`. `spawn-platform` is the presumed home for windowing, but its backend is undecided; confirm with the user rather than assuming.
- Crate layering: lower crates never depend on higher ones; no cycles; engine-core crates never reference framework crates. Keep public APIs small; default to `pub(crate)`.
- Rust: stable toolchain, edition 2024, `cargo fmt`, `cargo clippy --all-targets -- -D warnings` clean. Avoid `unsafe` unless needed (GPU/FFI/perf); every `unsafe` block gets a `// SAFETY:` comment and, where feasible, a test. No `unwrap()`/`expect()` on fallible runtime paths (assets, network, scripts); use typed errors (`thiserror`-style or hand-rolled), reserve panics for true invariant violations.
- Never let untrusted input (network packets, mod scripts/data, asset files) cause a panic, unbounded allocation, or UB.

**Testing**
- Add tests for critical functionality: ECS invariants, movement/physics determinism-where-claimed, serialization round-trips, replication/prediction reconciliation, script sandbox escapes and limit enforcement, data validation.
- Prefer headless-testable logic (no window/GPU) so tests run in CI. Isolate GPU/window code behind thin layers.
- Run `cargo test --workspace` before declaring work done. If something couldn't be run or verified, say so.

**Documentation**
- Document public Rust APIs (`///`) and every scripting-exposed function (signature, semantics, errors, server/client availability, API version introduced).
- Update this file when architecture decisions are made or when an item moves from [PLANNED] to [EXISTS].

**Multiplayer check for every gameplay feature:** state who owns it (server/client), what's replicated, what's predicted, and what happens on misprediction.

---

## 10. Roadmap: Vertical Slice First

Build incrementally; do **not** implement every subsystem at once. Suggested order (adjust with the user; each step should leave the repo building and tested):

0. **Housekeeping:** rustfmt/clippy/CI basics.
1. Window + event loop + input abstraction (likely `spawn-platform` — confirm with the user).
2. Minimal 3D rendering (clear screen → triangle → meshes/camera) in `spawn-render`.
3. ECS + entity lifecycle + fixed-timestep game loop.
4. FPS camera + character controller on basic collision (`spawn-physics`: start with what the controller needs).
5. Simple weapon (hitscan) with server-side hit registration shape, even if single-player first.
6. Basic client/server connectivity: input → authoritative sim → snapshot replication; prediction/reconciliation for movement.
7. Luau integration: sandboxed VM, minimal versioned API, weapon/game-mode behavior in script, hot reload.
8. Animation integration (skeletal playback → blending/state machines).
9. Expand: audio, vehicles, projectiles, classes, asset pipeline, etc.

The FPS framework should be extracted/organized as its own crate(s) once pieces exist, keeping core genre-free. The vertical slice is the architecture's validation — let abstractions emerge from it.

---

## 11. Agent Quick Rules

- ✅ Check what actually exists; distinguish EXISTS vs PLANNED.
- ✅ Keep core genre-agnostic; gameplay goes in frameworks/scripts.
- ✅ Rust simulates; Luau decides. Server is authoritative.
- ✅ Data in declarative files; behavior in Luau.
- ✅ Justify every dependency and abstraction.
- ❌ No hot loops, physics, or hit-reg in Luau.
- ❌ No unrestricted Rust object/memory access from scripts.
- ❌ No client authority over gameplay outcomes.
- ❌ No speculative frameworks (RPG/racing/etc.) or invented modules.
- ❌ No unrelated refactors; no bypassing the Rust/Luau boundary.
