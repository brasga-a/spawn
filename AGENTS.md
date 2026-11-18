# AGENTS.md — Spawn Engine

Authoritative architectural reference for coding agents. Nested `AGENTS.md` files (one per crate/tool, linked in [Subsystem Responsibility Map](#subsystem-responsibility-map)) define local rules and must not contradict this file. If code and documentation disagree, say so explicitly and propose which should change; never diverge silently.

**Status vocabulary** (used throughout all `AGENTS.md` files):

| Tag | Meaning |
|---|---|
| `EXISTS` | Verified implementation in source. |
| `SCAFFOLD` | Directory/package exists without meaningful implementation. |
| `PLANNED` | Architectural intention, not implemented. Never describe as existing. |

Never invent crates, modules, dependencies, or APIs. Verify against the tree and `Cargo.toml` files before asserting anything exists.

**Document precedence.** Engineering documentation lives in [`docs/`](docs/README.md): engine-wide [invariants](docs/architecture/INVARIANTS.md), [design](docs/architecture/DESIGN.md), [quality gates](docs/architecture/GATES.md) and [decision register](docs/architecture/DECISIONS.md) in `docs/architecture/`; milestone specifications in [`docs/foundation/`](docs/foundation/README.md); decision records in [`docs/adr/`](docs/adr/README.md); approved crates in [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md). When documents conflict:

1. This file governs global rules, layering, and ownership.
2. [INVARIANTS](docs/architecture/INVARIANTS.md) are binding engine-wide rules derived from accepted ADRs; every change passes the [GATES](docs/architecture/GATES.md).
3. A crate/tool `AGENTS.md` governs local rules within those limits.
4. `docs/foundation/` defines milestone requirements and must stay within the rules above.
5. ADRs record rationale. A *Proposed* ADR authorizes nothing; an *Accepted* ADR is synchronized into the affected `AGENTS.md` files in the same change.

Code and manifests are the only evidence of what is implemented. A conflict between documents is reported and fixed in the change that finds it, never resolved silently.

---

## Vision

Spawn is a modular, high-performance, **general-purpose** game engine written in Rust. Games can be written **entirely in Rust**; embedded Luau is an **optional** layer for scripting, rapid iteration and modding. The validation project is a multiplayer FPS (inspired by Black Ops II, Battlefield 4, Hardline): large-scale combat, responsive movement, weapons, vehicles, classes, advanced animation and physics, extensible gameplay. FPS is the first use case, not a limitation; RPG, racing, simulation, strategy, and platformers must remain feasible.

Priorities: performance, modularity, maintainability, extensibility. Generality comes from **proven requirements**, not speculation: extract to infrastructure only when a genre-independent need is demonstrated.

## Layers and Boundaries

```
Game Projects        game-specific rules (Rust and/or Luau), content, configs
Gameplay Frameworks  optional, genre-specific (FPS first)
Engine Infrastructure generic systems; no genre knowledge
```

Dependencies point **down only**, are explicit, and acyclic.

- **Engine infrastructure** (`crates/spawn-*`): no genre concepts (`Weapon`, `Health`, `Team`, `Vehicle`…). Never depends on frameworks, games, or tools.
- **Gameplay frameworks**: independent, genre-specific, depend on infrastructure only; one framework must not depend on another without explicit agreement. Intended home: `frameworks/` (`SCAFFOLD`: empty directory, not a workspace member). Only the FPS framework is in scope; do not build others.
- **Game projects / examples**: `examples/` holds only `vulkan_clear.rs`, a Cargo example target of the root package (manual Phase 3 smoke test); example packages per the *proposed* [ADR-0022](docs/adr/0022-repository-layout.md) do not exist, and if that ADR is accepted this file moves into one. Root package `spawn-engine` (`src/lib.rs`, `src/main.rs`) is the foundation composition root (`EXISTS`: single-threaded Wayland + Vulkan clear-frame runtime loop on Linux; headless seam and `ENGN-0002` diagnostic on targets without a native backend).
- **Tools** (`tools/*`): development tooling. Engine crates must never depend on tools. Tools depend on engine crates.
- Subsystems are optional: a game must not be forced to link or initialize systems it does not use.

## Rust and Luau

**Rust is a first-class gameplay language.** Rust covers engine infrastructure and orchestration, rendering/GPU, ECS and simulation, physics, animation, audio, networking/replication, input and platform integration, asset management, performance-critical code — **and** gameplay systems, mechanics, game-specific logic, frameworks and compile-time extensions. A game may be written entirely in Rust.

**Luau is optional.** When a game uses it, Luau provides gameplay scripting, rapid iteration, dynamic rules and behaviors, AI behavior orchestration, modding and runtime extension through controlled, sandboxed APIs.

Rules:
- Games must be able to use Rust exclusively; no engine crate or framework requires Luau or `spawn-scripting`.
- Component registration, system execution and game initialization are Rust APIs; Luau is never required for them. `spawn-scripting` is an optional consumer of the same engine contracts.
- When both are used, Rust gameplay systems and Luau scripts interact only through defined engine contracts (events, commands, registered APIs); neither reaches into the other's internals.
- Authority and security are language-independent: the server is authoritative whether gameplay is in Rust or Luau, and all Luau security rules below still apply.
- Luau is invoked on **events and decisions**, never as the simulation loop: no per-entity per-tick hot loops, physics math, hit registration, bulk projectile processing, or pathfinding inner loops in Luau. Code with those characteristics is Rust.

Integration layer: `mlua` with the `luau` feature (`PLANNED`; not a dependency). Before adding it, verify current mlua docs for supported Luau features; pin it in `[workspace.dependencies]`. All `mlua` usage is confined to `spawn-scripting`; no other crate may import `mlua` types. Prefer a command/event queue across the boundary, applied by Rust at defined sync points.

## Workspace

Root `Cargo.toml` (verified): `resolver = "3"`, members `.`, `crates/*`, `tools/*`; `[workspace.package]` provides `version`, `edition` (2024), `license` (MIT) and `rust-version` (1.99.1), inherited by every member; `[workspace.lints]` (ADR-0007) is inherited by every member through `[lints] workspace = true`; dev profile uses `opt-level = 1` (dependencies `opt-level = 3`). `[workspace.dependencies]` pins `ash` (used by `spawn-render` behind its optional `vulkan` feature), `raw-window-handle` (both crates' optional backend features), and `rustix`, `wayland-client`, `wayland-protocols` (`spawn-platform`'s optional `wayland` feature); with default features **every package has zero third-party normal dependencies**. The inter-crate edges are `spawn-input` depending on `spawn-core` and `spawn-platform` (both with `default-features = false`, so no native windowing backend is linked; ADR-0002, FND-T09) and the root composition package `spawn-engine` depending on `spawn-core`, `spawn-input`, `spawn-platform` and `spawn-render` (`default-features = false` globally, plus `spawn-platform/wayland` and `spawn-render/vulkan` on `cfg(target_os = "linux")`); no library crate in `crates/` depends on `spawn-engine` or `tools/`, and all other inter-crate edges in nested documents are `PLANNED`.

Conventions:
- One crate per subsystem with a narrow public API; names `spawn-<subsystem>`.
- Add a dependency edge only when real code needs it; lower layers must not depend on higher ones.
- Minimal dependencies per crate: every crate must build and test with the fewest `spawn-*` and external dependencies possible. Native or heavy backends sit behind Cargo features so consumers that only need a crate's types (headless tests, dedicated servers) do not link them. This isolates compilation and error attribution; runtime containment still relies on typed errors instead of panics.
- Do not assume every crate depends on `spawn-core`. Prefer small shared contracts and dependency inversion (the higher layer implements a trait/contract owned by the lower one) over upward references.
- Third-party deps: state what, why not std/hand-written, maintenance health, compile-time cost, licence; declare in `[workspace.dependencies]` with `default-features = false` where possible. **Approved dependencies** (owner-approved, added only when code needs them; `symphonia` rejected) are listed with license, kind and allowed crates in [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md); a crate not listed there needs owner approval first. No physics library until the dynamics evaluation of ADR-0005. No royalty-bearing or revenue-tied dependency (e.g. FMOD, Wwise, Havok); royalty-free proprietary SDKs (e.g. Steamworks) only in optional integration crates ([ADR-0008](docs/adr/0008-third-party-license-policy.md)).
- Workspace membership (`.`, `crates/*`, `tools/*`) matches the directories on disk: the root package, 18 packages under `crates/`, and 3 under `tools/`. Each has an `AGENTS.md` (map below); packages outside those globs are not part of the workspace.

## Subsystem Responsibility Map

Intended layering (`PLANNED`, non-binding until code exists): foundations (math, core, profiler, reflect, io) → data/runtime (ecs, assets, scene, world) → domain systems (platform, input, render, physics, animation, audio, network, ui, scripting) → frameworks → games. Tools sit beside the top.

| Crate | Owns (one line) | Status |
|---|---|---|
| [spawn-core](crates/spawn-core/AGENTS.md) | Lifecycle/orchestration contracts shared by subsystems; fixed-timestep loop contract | EXISTS (fixed-step clock); rest PLANNED |
| [spawn-math](crates/spawn-math/AGENTS.md) | Vector/matrix/quaternion/geometry primitives | SCAFFOLD |
| [spawn-ecs](crates/spawn-ecs/AGENTS.md) | Entity identity, component storage, queries, system scheduling | SCAFFOLD |
| [spawn-world](crates/spawn-world/AGENTS.md) | Runtime worlds: lifecycle, activation, load/unload, streaming | SCAFFOLD |
| [spawn-scene](crates/spawn-scene/AGENTS.md) | Scene data model, hierarchy, (de)serialization, asset/script references | SCAFFOLD |
| [spawn-assets](crates/spawn-assets/AGENTS.md) | Asset identity, loading, caching, dependencies, hot reload | SCAFFOLD |
| [spawn-io](crates/spawn-io/AGENTS.md) | Filesystem/stream/byte-level resource access | SCAFFOLD |
| [spawn-reflect](crates/spawn-reflect/AGENTS.md) | Runtime type metadata for serialization, editor, script exposure | SCAFFOLD |
| [spawn-platform](crates/spawn-platform/AGENTS.md) | OS integration: windows, system events, surfaces | EXISTS (raw events, relative motion, cursor constraints; Wayland backend); X11, Win32 PLANNED |
| [spawn-input](crates/spawn-input/AGENTS.md) | Input devices, state, actions, mappings | EXISTS (keyboard/mouse state, semantic mappings, tick sampling); gamepads, touch, persistence PLANNED |
| [spawn-render](crates/spawn-render/AGENTS.md) | GPU abstraction, resources, shaders, materials, frame execution | EXISTS (Vulkan clear frame, Wayland surfaces); rest PLANNED |
| [spawn-physics](crates/spawn-physics/AGENTS.md) | Rigid-body simulation, collision, spatial queries | SCAFFOLD |
| [spawn-animation](crates/spawn-animation/AGENTS.md) | Skeletal/property animation runtime, blending, state machines | SCAFFOLD |
| [spawn-audio](crates/spawn-audio/AGENTS.md) | Audio playback, mixing, spatialization | SCAFFOLD |
| [spawn-network](crates/spawn-network/AGENTS.md) | Transport, connections, replication, prediction support | SCAFFOLD |
| [spawn-ui](crates/spawn-ui/AGENTS.md) | In-game UI runtime (layout, widgets, HUD) | SCAFFOLD |
| [spawn-scripting](crates/spawn-scripting/AGENTS.md) | Luau VM, sandbox, script lifecycle, exposed API | SCAFFOLD |
| [spawn-profiler](crates/spawn-profiler/AGENTS.md) | Timing, counters, tracing/diagnostic instrumentation | SCAFFOLD |
| `spawn-jobs` (no directory yet) | Work-stealing job system ([ADR-0012](docs/adr/0012-job-system.md)) | PLANNED |
| [spawn-cli](tools/spawn-cli/AGENTS.md) | Command-line project/build/validate/asset tooling | SCAFFOLD |
| [spawn-editor](tools/spawn-editor/AGENTS.md) | Graphical authoring tools | SCAFFOLD |
| [spawn-mcp](tools/spawn-mcp/AGENTS.md) | MCP server for external AI agents/tools | SCAFFOLD |

Verified state: `spawn-core` contains the fixed-step clock (`FixedStepClock`, `FixedStepConfig`, `FixedStepUpdate`, `Tick`, `ClockError`) with unit, verification (FND-T01, FND-T02) and doc tests; `spawn-platform` contains the raw event types (including `PointerRelativeMotion` and `CursorGrabChanged`), `CursorGrabMode`, `WindowDescriptor` and `PlatformError` (always compiled) and, behind the `wayland` feature, the Wayland backend (`Platform`, `Window`, `PumpMode`, `zwp_relative_pointer_v1`, `zwp_pointer_constraints_v1`) with headless translation tests (FND-T05, FND-T10 platform part, Phase 2b relative motion and cursor constraint lifecycle) and a manual smoke example; `spawn-input` contains allocation-free keyboard/mouse `DeviceState`, data-driven `InputMap` (`ActionId`, `AxisId`, `ContextId`, `AxisConfig`), tick-indexed `InputCommand` sampling and untrusted-command validation (`InputState`, `InputError`, `#![forbid(unsafe_code)]`) with headless unit, verification (FND-T03, FND-T04, Phase 2b relative-motion wiring, replay determinism) and doc tests; `spawn-render` contains its public types, `RenderError`, the pure presentation controller and selection logic (always compiled, FND-T06/T07 tests) and, behind the `vulkan` feature, `Renderer` over a private Vulkan 1.3 RHI verified clean under `VK_LAYER_KHRONOS_validation` (including synchronization validation) with opt-in GPU tests and the `vulkan_clear` smoke example; the root `spawn-engine` package (`src/`, `tests/foundation_verification.rs`) composes all four foundation subsystems (`EngineError`, `RuntimeOptions`, `run_session`, `run_native`, `#![forbid(unsafe_code)]`) with headless verification tests (FND-T08, FND-T09, FND-T10, FND-INV-06) and recorded Wayland/Vulkan smoke runs; every other `lib.rs` contains only a crate-level doc comment (no items, no tests).

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

**Data:** static configuration (characters, classes, movement profiles, weapon stats, ballistics, attachments, abilities, animation references) is declarative data, not code (neither Luau nor Rust constants). Behavior is implemented in Rust or, optionally, Luau. Format: RON + `serde` for authored data, cooked binary at runtime ([ADR-0013](docs/adr/0013-declarative-data-format.md); `PLANNED`). Definitions are versioned and validated at load with errors naming file, field, and reason. Hot reload is a goal for data and scripts. Do not build a pipeline before a concrete feature needs it.

**Scripting** (all `PLANNED`):
- Allowlist-only API exposure, default deny; the boundary is law. Never expose raw Rust structures, pointers, ECS internals, or filesystem/OS/network access; never bypass the boundary for convenience. Missing API is added deliberately: documented, versioned, tested.
- Sandboxed environments (no `os`, `io`, `debug`, dynamic code loading, FFI in untrusted contexts); per-VM instruction/time and memory limits, stack limits, per-frame time budget.
- Error isolation: a failing script is logged with script/line and disabled or restarted; it never crashes or stalls the engine or other scripts.
- Separate client and server VMs with separate API surfaces. Client Luau is presentation/cosmetic only; server Luau defines rules through controlled APIs.
- Versioned engine→script API with a changelog; scripts declare their target version. Define hot-reload state semantics before implementing.
- Scripts are untrusted. Validate every value crossing into Rust (type, range, NaN/inf, lengths, handle validity). A stale or forged handle yields an error, never UB or a panic.

## Engineering Standards

- Stable toolchain pinned, edition 2024. `cargo fmt`; `cargo clippy --all-targets -- -D warnings` clean. Applied from [ADR-0007](docs/adr/0007-toolchain-lints-and-ci.md) (`EXISTS`): `rust-toolchain.toml`, `[workspace.lints]` + `clippy.toml`, `.github/workflows/ci.yml` (pull requests), `deny.toml`, and the local check command **`scripts/check.sh`** (run it before every push; it fails when a mandatory check cannot run). Without `rustup` (e.g. distribution-packaged Rust), `rust-toolchain.toml` is ignored locally; use the pinned version.
- `unsafe` only when needed (GPU/FFI/perf); each block has a `// SAFETY:` comment and, where feasible, a test.
- No `unwrap()`/`expect()`/`panic!`/`todo!` outside tests. Fallible functions return `Result` with typed errors carrying origin location, error code and cause chain ([ADR-0009](docs/adr/0009-error-handling-and-diagnostics.md)); foundation crates never panic on behalf of callers; panic only on true invariant violations.
- Untrusted input (packets, mod scripts/data, asset files) must never cause panic, unbounded allocation, or UB.
- Public APIs are small; default to `pub(crate)`. Document public items (`///`) and every script-exposed function (signature, semantics, errors, server/client availability, API version).
- Build in observability when the first real system exists (structured logging, timing, counters, debug draw), owned by `spawn-profiler`; no speculative framework.
- Small focused changes; no drive-by refactors or renames.

**Testing:** add tests for critical behavior (ECS invariants, serialization round-trips, replication/reconciliation, sandbox escapes and limit enforcement, data validation, determinism where claimed). Approved test tools ([DEPENDENCIES](docs/DEPENDENCIES.md#testing-and-quality)): `proptest` (properties), `cargo-fuzz` + `arbitrary` (every untrusted-input parser), `loom` (lock-free and concurrency code), `insta` (snapshots, e.g. error reports), `criterion` or `divan` (benchmarks on the reference hardware), Miri (pure-Rust `unsafe`). Keep logic headless-testable (no window/GPU) and isolate GPU/window code behind thin layers. Run `scripts/check.sh` before declaring work done; state explicitly anything not run or verified.

## Agent Rules

- Check what exists; use the status tags correctly.
- Follow the [INVARIANTS](docs/architecture/INVARIANTS.md); pass the [GATES](docs/architecture/GATES.md) (G0 before every commit: `scripts/check.sh`).
- Read the crate's `AGENTS.md` **Governing Decisions** before changing it; use only the third-party crates approved there and in [DEPENDENCIES](docs/DEPENDENCIES.md).
- Work on a branch and merge through a pull request (CI runs only on pull requests); never commit directly to `master` unless the owner asks.
- Keep infrastructure genre-agnostic; gameplay goes to frameworks and games (Rust, optionally Luau).
- Rust implements engine and gameplay; Luau optionally scripts gameplay through engine contracts; the server is authoritative either way.
- Justify every dependency and abstraction. Confirm undecided choices (rigid-body dynamics, anything covered only by a *Proposed* ADR) with the user instead of assuming.
- No speculative frameworks (RPG/racing/etc.), no invented modules, no client authority over outcomes, no bypassing the Rust/Luau boundary.
- When responsibilities are ambiguous or a nested document conflicts with this one, report it.

## Documentation Maintenance

- A change that adds a dependency edge, moves a responsibility, or changes a status tag updates the affected `AGENTS.md` files in the same change (root map, the crate's own file, and any crate whose boundary shifts).
- New crate/tool: add it to the [Subsystem Responsibility Map](#subsystem-responsibility-map) and create its `AGENTS.md` with the standard sections (Purpose, Responsibilities, Non-Responsibilities, Architectural Boundaries, Public API Guidelines, Internal Design Principles, Performance and Safety, Testing Requirements, Governing Decisions, Current Implementation Status, Agent Instructions). **Governing Decisions** lists the accepted and proposed ADRs for the crate and its approved third-party crates; keep it in sync with the ADR index and [DEPENDENCIES](docs/DEPENDENCIES.md).
- Keep status text factual and non-temporal: describe what the source contains, not when or how long.
- Nested files contain local rules only; global rules live here.
- Accepting, superseding, or rejecting an ADR updates the [ADR index](docs/adr/README.md), the [decision register](docs/architecture/DECISIONS.md), [INVARIANTS](docs/architecture/INVARIANTS.md) and [GATES](docs/architecture/GATES.md) where a rule changes, the related [Open Architectural Questions](#open-architectural-questions) entry, and the affected crate `AGENTS.md` files in the same change.

## Accepted Decisions

Summaries only; the ADR is authoritative.

- [ADR-0001](docs/adr/0001-in-house-platform-layer.md): in-house platform layer for Windows and Linux (Wayland and X11); macOS not supported; Spawn owns the event loop; thin binding crates (`windows-sys`, `wayland-client`, `x11rb`, `xkbcommon`).
- [ADR-0002](docs/adr/0002-platform-owns-raw-input-events.md): `spawn-platform` owns raw event types, available without its native backend feature; `spawn-input` builds semantic input on top.
- [ADR-0003](docs/adr/0003-fixed-step-loop-and-threading.md): `spawn-core` owns the fixed-step timing contract and `u64` tick index; the root binary drives the loop; `spawn-ecs` owns system scheduling; single-threaded foundation with a staged multi-threading target and recorded risks.
- [ADR-0004](docs/adr/0004-in-house-graphics-api.md): in-house graphics API over Vulkan 1.3 inside `spawn-render` (`ash`, in-house allocator, Slang compiled offline to SPIR-V); `wgpu` and others are design references only.
- [ADR-0005](docs/adr/0005-physics-strategy.md): in-house collision, spatial queries, kinematic character controller and lag-compensation query support in `spawn-physics`; deterministic movement math from `spawn-math`; rigid-body dynamics deferred to a benchmark-based evaluation (in-house Rust with Jolt as reference vs. Jolt vs. Rapier).
- [ADR-0006](docs/adr/0006-in-house-audio.md): in-house audio output (WASAPI, PipeWire only), mixer, voice management, HDR-style mixing and HRTF; Steam Audio (Apache-2.0) only for expensive features such as propagation and reflections, each by its own ADR; no FMOD/Wwise.
- [ADR-0007](docs/adr/0007-toolchain-lints-and-ci.md): pinned toolchain (Rust 1.99.1), strict workspace lints, `Result` for every fallible function, GitHub Actions on pull requests only plus a local check command.
- [ADR-0008](docs/adr/0008-third-party-license-policy.md): no royalties or revenue-tied fees, ever; permissive open source anywhere; royalty-free proprietary SDKs (e.g. Steamworks) allowed only in optional integration crates or games, each by its own ADR; strong copyleft forbidden in shipped code; MPL-2.0 by owner approval; reference-code and AI-code rules; `cargo-deny`.
- [ADR-0009](docs/adr/0009-error-handling-and-diagnostics.md): hand-written errors with origin location, error codes, cause chains and backtraces; `unwind` panics; crash reports and minidumps with archived symbols; own logging facade in `spawn-profiler`.
- [ADR-0010](docs/adr/0010-math-library.md): in-house math; right-handed, Y-up, −Z forward; meters/seconds/kilograms; column-major; reversed-Z; `f64` absolute world positions with `f32` local data and camera-relative rendering; deterministic `f32`/`f64` subset only where needed; radians in code, degrees in data and Luau.
- [ADR-0011](docs/adr/0011-ecs-architecture.md): in-house ECS; generational 64-bit entities; archetype tables with opt-in sparse sets; tick-based change detection; command buffers; deterministic, explicitly ordered scheduling; `bevy_ecs` rejected as a dependency, kept as primary reference.
- [ADR-0012](docs/adr/0012-job-system.md): in-house work-stealing job system (tasks, no fibers, two priorities, single-threaded debug mode) in a planned foundation-level crate `spawn-jobs`; worker count configurable, default by benchmark.
- [ADR-0013](docs/adr/0013-declarative-data-format.md): RON + `serde` for authored data, cooked binary at runtime; mandatory `version` field and migrations; unknown fields are errors; validation errors name file, field and reason.
- [ADR-0014](docs/adr/0014-shader-toolchain.md): offline Slang → SPIR-V + reflection compilation in `spawn-cli` (never `build.rs` or shipping runtime); pinned `slangc` version (`Apache-2.0 WITH LLVM-exception`); generated SPIR-V uncommitted; RHI layout validation against reflection; development hot reload and explicit variant budgets.
- [ADR-0015](docs/adr/0015-asset-pipeline.md): offline import/cook in `spawn-cli`; glTF 2.0 primary, FBX through ufbx (tooling only); PNG default, JPEG for photo albedo only, EXR/HDR for HDR; BCn cooked textures; UUID `.meta` identity; content-hash cooking cache.
- [ADR-0019](docs/adr/0019-anti-cheat-and-client-trust.md): no kernel anti-cheat; untrusted client; server-side validation and information minimization; humans decide every penalty (AI only flags); disclosure in Terms of Service and Privacy Policy; evaluation tracks for user-mode anti-cheat, small AI models and integrity analysis.

**Reference hardware** (minimum target; the owner's machine): AMD Ryzen 5 1600 (6 cores / 12 threads), AMD Radeon RX 460 (Polaris, RADV driver, Vulkan 1.4), Linux with PipeWire. Performance budgets are measured on it. It has no hardware ray tracing or mesh shaders, so features depending on them cannot be developed or verified on it.

## Open Architectural Questions

Each remaining question has a proposed ADR awaiting owner review; see the [ADR index](docs/adr/README.md) and the [decision register](docs/architecture/DECISIONS.md#pending-decisions) for every pending item.

1. Dependency relationship between `spawn-scene` and `spawn-ecs`: proposed in [ADR-0021](docs/adr/0021-scene-ecs-relationship.md).
2. Layout of `frameworks/`, `examples/`, `tests/`: proposed in [ADR-0022](docs/adr/0022-repository-layout.md).
