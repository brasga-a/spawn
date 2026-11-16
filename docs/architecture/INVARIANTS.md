# Engine Invariants

Rules that must never be violated anywhere in Spawn. Each invariant has a stable ID, its source decision, and how it is verified. Milestone-specific invariants live in milestone documents (e.g. [foundation](../foundation/INVARIANTS.md)) and must not contradict these.

**Normative status:** invariants from **Accepted** ADRs are binding. Invariants from **Proposed** ADRs (marked *proposed*) are binding only once their ADR is accepted; until then, agents must not implement designs that contradict them.

**Changing an invariant** requires updating its source ADR (or a new ADR superseding it), this file, [GATES](GATES.md) where a gate checks it, and [DECISIONS](DECISIONS.md), in the same change.

**Verification kinds:** `auto` = checked by tooling/CI; `test` = covered by automated tests; `review` = checked in code review; `manual` = checked on the reference hardware.

## Layering and dependencies

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-001 | Dependencies point down only: engine crates never depend on frameworks, games or tools; frameworks never depend on games or tools. | Root [Layers](../../AGENTS.md#layers-and-boundaries) | auto (`cargo metadata`), review |
| SPN-INV-002 | The crate dependency graph is acyclic. | Root [Workspace](../../AGENTS.md#workspace) | auto |
| SPN-INV-003 | Engine infrastructure contains no genre concepts (weapon, health, team, player, vehicle, match). | Root [Layers](../../AGENTS.md#layers-and-boundaries) | review |
| SPN-INV-004 | Foundation crates `spawn-core`, `spawn-math`, `spawn-jobs` depend on no other `spawn-*` crate. | [ADR-0012](../adr/0012-job-system.md), [ADR-0009](../adr/0009-error-handling-and-diagnostics.md) | auto |
| SPN-INV-005 | `spawn-platform` never depends on `spawn-input` or `spawn-render`; its raw event types compile without any native backend feature. | [ADR-0002](../adr/0002-platform-owns-raw-input-events.md) | auto (`cargo tree`) |
| SPN-INV-006 | `spawn-render` does not depend on `spawn-platform`; the window handle crosses through `raw-window-handle`. | [ADR-0004](../adr/0004-in-house-graphics-api.md) | auto |
| SPN-INV-007 | `spawn-audio` never depends on `spawn-physics`; occlusion values are supplied by higher layers. | [ADR-0006](../adr/0006-in-house-audio.md) | auto |
| SPN-INV-008 | `spawn-scene` does not depend on `spawn-ecs` (*proposed*). | [ADR-0021](../adr/0021-scene-ecs-relationship.md) | auto |
| SPN-INV-009 | Headless consumers (tests, dedicated servers, CLI validation) build without window, GPU or audio backends. | [ADR-0002](../adr/0002-platform-owns-raw-input-events.md), root [Workspace](../../AGENTS.md#workspace) | auto (CI is headless) |
| SPN-INV-010 | `mlua` types appear only inside `spawn-scripting`. | Root [Rust and Luau](../../AGENTS.md#rust-and-luau) | auto (grep), review |
| SPN-INV-011 | No backend or third-party type (Vulkan/`ash`, OS handles, binding-crate types, `egui`, Steam) appears in an engine crate's public API; backend implementation details stay private. **Single approved exception:** the `raw-window-handle` interoperability boundary between `spawn-platform` and `spawn-render` (ADR-0004): the platform window may implement, and the renderer may accept, the `raw-window-handle` traits and handle types, exposing only the minimum needed to create a surface. This exception does not authorize any other third-party type in public APIs. | [ADR-0001](../adr/0001-in-house-platform-layer.md), [ADR-0004](../adr/0004-in-house-graphics-api.md), [ADR-0010](../adr/0010-math-library.md) | review |
| SPN-INV-012 | `egui` and editor code never enter the game runtime; `ufbx` and asset decoders exist only in tools. | [ADR-0015](../adr/0015-asset-pipeline.md), [DEPENDENCIES](../DEPENDENCIES.md) | auto (`cargo tree` of runtime crates) |

## Licensing and dependencies

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-020 | No dependency requires royalties, revenue share or fees tied to revenue, budget or units. | [ADR-0008](../adr/0008-third-party-license-policy.md) | auto (`cargo-deny`), review |
| SPN-INV-021 | No strong-copyleft (GPL/LGPL/AGPL/SSPL) or non-commercial code in anything shipped. | [ADR-0008](../adr/0008-third-party-license-policy.md) | auto (`cargo-deny`) |
| SPN-INV-022 | Engine crates, frameworks and tools build without any proprietary component; proprietary SDKs live only in optional integration crates or games. | [ADR-0008](../adr/0008-third-party-license-policy.md) | auto (default-feature build), review |
| SPN-INV-023 | Every third-party crate is listed in [DEPENDENCIES](../DEPENDENCIES.md) and used only in the crates listed there. | [DEPENDENCIES](../DEPENDENCIES.md) | review, auto where scripted |
| SPN-INV-024 | Code ported or closely adapted from external sources keeps its notice and is listed in `THIRD_PARTY_NOTICES.md`; code from forbidden sources (GPL, Unreal EULA, Source SDK) is never copied, including by AI agents. | [ADR-0008](../adr/0008-third-party-license-policy.md) | review |

## Errors, safety and robustness

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-030 | No `unwrap`, `expect`, `panic!`, `todo!` or `unimplemented!` outside tests. | [ADR-0007](../adr/0007-toolchain-lints-and-ci.md) | auto (clippy) |
| SPN-INV-031 | Every fallible function returns `Result` with a typed error; foundation crates never panic on behalf of callers. | [ADR-0007](../adr/0007-toolchain-lints-and-ci.md), [ADR-0009](../adr/0009-error-handling-and-diagnostics.md) | review, test |
| SPN-INV-032 | Every error carries its origin location, a stable error code and its cause chain. | [ADR-0009](../adr/0009-error-handling-and-diagnostics.md) | test (snapshot), review |
| SPN-INV-033 | Every `unsafe` block has a `// SAFETY:` comment; `unsafe` is confined to modules that need it (FFI, GPU, storage internals). | [ADR-0007](../adr/0007-toolchain-lints-and-ci.md) | auto (clippy), review |
| SPN-INV-034 | Untrusted input (packets, files, assets, mods, scripts) never causes a panic, undefined behavior or unbounded allocation; sizes are bounded before allocating. | Root [Engineering Standards](../../AGENTS.md#engineering-standards), [ADR-0020](../adr/0020-memory-strategy.md) | test (fuzzing), review |
| SPN-INV-035 | Stale or forged handles (entities, assets, voices, scripts) produce errors, never undefined behavior. | Root [ECS](../../AGENTS.md#ecs-and-system-design), [ADR-0011](../adr/0011-ecs-architecture.md) | test |
| SPN-INV-036 | The real-time audio thread never allocates, locks or logs directly. | [ADR-0006](../adr/0006-in-house-audio.md), [ADR-0009](../adr/0009-error-handling-and-diagnostics.md) | review, test |
| SPN-INV-037 | Crash reports are written locally; nothing is uploaded without player consent. | [ADR-0009](../adr/0009-error-handling-and-diagnostics.md) | review |
| SPN-INV-038 | No in-house cryptographic primitives; encryption uses approved audited crates (*proposed*). | [ADR-0016](../adr/0016-networking-and-replication.md) | review |

## Simulation, time and determinism

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-040 | Simulation advances only in fixed steps from monotonic time, never by render-frame delta. | [ADR-0003](../adr/0003-fixed-step-loop-and-threading.md) | test (`spawn-core`: FND-T01/T02) |
| SPN-INV-041 | A single wakeup runs at most the configured catch-up ticks; excess backlog is dropped and reported. | [ADR-0003](../adr/0003-fixed-step-loop-and-threading.md) | test (`spawn-core`: FND-T01/T02) |
| SPN-INV-042 | The tick index is a `u64` owned by `spawn-core`, incremented once per executed tick, never reset by resize, recovery or dropped backlog. | [ADR-0003](../adr/0003-fixed-step-loop-and-threading.md) | test (`spawn-core`: FND-T01/T02) |
| SPN-INV-043 | Simulation state is separate from presentation state; rendering never mutates simulation state. | Root [ECS](../../AGENTS.md#ecs-and-system-design), [ADR-0003](../adr/0003-fixed-step-loop-and-threading.md) | review |
| SPN-INV-044 | Code used in prediction/reconciliation produces identical results on Windows and Linux for the same inputs, using only the deterministic subset of `spawn-math`. | [ADR-0005](../adr/0005-physics-strategy.md), [ADR-0010](../adr/0010-math-library.md) | test (cross-platform comparison) |
| SPN-INV-045 | Parallel execution never changes the result of simulation systems used in prediction: results are combined in a fixed order, never completion order. | [ADR-0003](../adr/0003-fixed-step-loop-and-threading.md), [ADR-0011](../adr/0011-ecs-architecture.md), [ADR-0012](../adr/0012-job-system.md) | test |
| SPN-INV-046 | ECS system order is explicit and stable; structural changes apply at sync points in system order. | [ADR-0011](../adr/0011-ecs-architecture.md) | test |
| SPN-INV-047 | Every build supports a single-threaded execution mode for reproduction. | [ADR-0012](../adr/0012-job-system.md) | test |

## Spatial conventions

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-050 | Right-handed, Y-up, −Z forward; meters, seconds, kilograms; radians in code. | [ADR-0010](../adr/0010-math-library.md) | test (import/projection) |
| SPN-INV-051 | Absolute world positions are `f64` (distinct type from `f32` local data); conversion is explicit, never implicit. | [ADR-0010](../adr/0010-math-library.md) | auto (types), review |
| SPN-INV-052 | The GPU never receives absolute world positions; rendering is camera-relative. | [ADR-0010](../adr/0010-math-library.md) | review, test |
| SPN-INV-053 | Reversed-Z depth; the Vulkan Y-flip is applied in exactly one place. | [ADR-0010](../adr/0010-math-library.md) | review |
| SPN-INV-054 | Declarative data and Luau use degrees; field names state units; conversion happens once at the boundary. | [ADR-0010](../adr/0010-math-library.md) | review |

## Multiplayer and trust

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-060 | The server is authoritative: clients send inputs/intents only and never decide outcomes (hits, damage, kills, pickups). | Root [Multiplayer](../../AGENTS.md#multiplayer-foundational), [ADR-0019](../adr/0019-anti-cheat-and-client-trust.md) | review, test |
| SPN-INV-061 | Every client input is validated on the server (rate, ranges, timestamp plausibility). | [ADR-0019](../adr/0019-anti-cheat-and-client-trust.md) | test |
| SPN-INV-062 | The server does not replicate information a client cannot plausibly perceive. | [ADR-0019](../adr/0019-anti-cheat-and-client-trust.md) | review, test |
| SPN-INV-063 | No kernel-level anti-cheat. | [ADR-0019](../adr/0019-anti-cheat-and-client-trust.md) | review |
| SPN-INV-064 | No automated system applies penalties; AI and integrity checks only flag, and every penalty is a recorded human decision. | [ADR-0019](../adr/0019-anti-cheat-and-client-trust.md) | review, process |
| SPN-INV-065 | Every gameplay feature declares server/client ownership, replication, prediction and misprediction behavior. | Root [Multiplayer](../../AGENTS.md#multiplayer-foundational) | review |

## Scripting, packages and mods

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-069 | Luau is optional: games can be implemented entirely in Rust; no engine crate or framework depends on `spawn-scripting`, and component registration, system execution and game initialization never require Luau. | Root [Rust and Luau](../../AGENTS.md#rust-and-luau) | auto (`cargo tree`), review |
| SPN-INV-070 | Luau runs on events and decisions, never as a per-entity per-tick loop. | Root [Rust and Luau](../../AGENTS.md#rust-and-luau) | review, profiler budget |
| SPN-INV-071 | Script APIs are allowlisted (default deny); scripts get no filesystem, OS or network access; every value crossing into Rust is validated. | Root [Scripting](../../AGENTS.md#data-driven-design-and-scripting-security) | test (sandbox escapes) |
| SPN-INV-072 | Client and server scripts run in separate VMs with separate APIs. | Root [Scripting](../../AGENTS.md#data-driven-design-and-scripting-security) | test |
| SPN-INV-073 | A failing script or mod is disabled with a clear error and never crashes or stalls the engine. | Root [Scripting](../../AGENTS.md#data-driven-design-and-scripting-security), [ADR-0025](../adr/0025-mods-and-workshop.md) | test |
| SPN-INV-074 | No dynamically loaded Rust plugins and no native-code mods (*proposed*). | [ADR-0024](../adr/0024-packages-and-game-extensions.md) | review |
| SPN-INV-075 | Multiplayer clients match the server's `server`/`shared` package set exactly, verified by content hash (*proposed*). | [ADR-0024](../adr/0024-packages-and-game-extensions.md) | test |

## Data and assets

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-080 | Static configuration is declarative data, never Luau or Rust constants; every file has a `version`; unknown fields are errors. | [ADR-0013](../adr/0013-declarative-data-format.md) | test |
| SPN-INV-081 | Data and asset errors name file, field (or node) and reason. | [ADR-0013](../adr/0013-declarative-data-format.md), [ADR-0015](../adr/0015-asset-pipeline.md) | test |
| SPN-INV-082 | Game data references assets by UUID, never by path. | [ADR-0015](../adr/0015-asset-pipeline.md) | review, validation command |
| SPN-INV-083 | Shipping builds load only cooked assets; the runtime never decodes PNG/JPEG/EXR or compiles shaders. | [ADR-0015](../adr/0015-asset-pipeline.md), [ADR-0014](../adr/0014-shader-toolchain.md) | auto (`cargo tree`), review |
| SPN-INV-084 | `build.rs` scripts never invoke external compilers such as `slangc` (*proposed*). | [ADR-0014](../adr/0014-shader-toolchain.md) | review |

## Platforms and hardware

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-090 | Supported platforms are Windows and Linux only; no platform is claimed as supported without a recorded smoke test. | [ADR-0001](../adr/0001-in-house-platform-layer.md) | manual |
| SPN-INV-091 | The minimum target is the reference hardware (Ryzen 5 1600, RX 460, Linux/PipeWire); performance budgets are measured on it. | Root [Accepted Decisions](../../AGENTS.md#accepted-decisions) | manual |
| SPN-INV-092 | Vulkan 1.3 with dynamic rendering, synchronization2, timeline semaphores and descriptor indexing is the graphics baseline; development builds run with validation layers and report no errors. | [ADR-0004](../adr/0004-in-house-graphics-api.md) | manual |

## Documentation

| ID | Invariant | Source | Verify |
| --- | --- | --- | --- |
| SPN-INV-100 | Nothing is documented as `EXISTS` without inspected source and corresponding verification. | Root status vocabulary | review |
| SPN-INV-101 | Accepting, superseding or rejecting a decision updates the ADR index, [DECISIONS](DECISIONS.md), the root `AGENTS.md` and affected crate `AGENTS.md` files in the same change. | Root [Documentation Maintenance](../../AGENTS.md#documentation-maintenance) | review |
