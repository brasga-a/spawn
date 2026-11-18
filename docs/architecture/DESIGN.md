# Engine Technical Design

How Spawn's subsystems fit together, derived from the [ADRs](../adr/README.md). **Most of the architecture described here is `PLANNED`**: only the foundation slices of `spawn-core`, `spawn-platform`, `spawn-input`, `spawn-render`, and the single-threaded root `spawn-engine` composition loop exist (`EXISTS`; see the root [Subsystem Responsibility Map](../../AGENTS.md#subsystem-responsibility-map) for exact scope, while all other crates remain `SCAFFOLD` or `PLANNED`). Rationale lives in the ADRs; global rules in the root [AGENTS.md](../../AGENTS.md); non-negotiable rules in [INVARIANTS](INVARIANTS.md). Parts based on *Proposed* ADRs are marked *proposed*.

## 1. Layers

```text
┌────────────────────────────────────────────────────────────────────┐
│ Game projects (separate repositories, proposed ADR-0022)           │
│   game rules in Rust (optionally Luau), content packages, services │
├────────────────────────────────────────────────────────────────────┤
│ Gameplay frameworks (frameworks/, e.g. spawn-framework-fps)        │
├────────────────────────────────────────────────────────────────────┤
│ Optional integrations: spawn-services, spawn-services-steam        │
│   (proposed ADR-0023; proprietary SDKs isolated here)              │
├────────────────────────────────────────────────────────────────────┤
│ Engine infrastructure (crates/spawn-*)                             │
└────────────────────────────────────────────────────────────────────┘
  Tools (tools/: spawn-cli, spawn-editor, spawn-mcp) depend on engine crates;
  nothing in the engine depends on tools.
```

## 2. Crate dependency plan

Only `spawn-input -> spawn-core, spawn-platform` (without backend features) and root `spawn-engine -> spawn-core, spawn-input, spawn-platform, spawn-render` (`wayland` + `vulkan` enabled on Linux) exist so far. The intended edges below follow the ADRs and [INVARIANTS](INVARIANTS.md); each is added only when code needs it.

| Level | Crate | Intended `spawn-*` dependencies |
| --- | --- | --- |
| Foundation | `spawn-math` | none |
| | `spawn-core` | none |
| | `spawn-jobs` (planned crate) | none |
| | `spawn-profiler` | none |
| | `spawn-reflect` | none |
| | `spawn-io` | profiler |
| Data/runtime | `spawn-ecs` | math, reflect, jobs, profiler |
| | `spawn-assets` | io, jobs, profiler |
| | `spawn-scene` | reflect, math (asset IDs from assets) — not ecs (*proposed* ADR-0021) |
| | `spawn-world` | ecs, scene, reflect, assets, core |
| Domain | `spawn-platform` | none required; backends behind features |
| | `spawn-input` | platform (no backend feature), core |
| | `spawn-render` | math, assets, profiler (window handle via `raw-window-handle`, not platform) |
| | `spawn-physics` | math, jobs, profiler |
| | `spawn-animation` | math, assets |
| | `spawn-audio` | math, assets (never physics) |
| | `spawn-network` | math, ecs, reflect, core |
| | `spawn-ui` | math, input; render through render-facing contracts |
| | `spawn-scripting` | ecs, math, reflect, assets |
| Packages | `spawn-packages` (planned, *proposed* ADR-0024) | io, assets |
| Composition | `spawn-engine` (root binary) | the subsystems a given build uses |

`spawn-core`, `spawn-math` and `spawn-jobs` never log; they expose hooks that the root binary connects to `spawn-profiler`.

## 3. Runtime architecture

### 3.1 Main loop (ADR-0003)

The root binary owns a poll-driven loop. The single-threaded foundation loop exists in `spawn-engine` (`run_session` over `FixedStepClock`, `spawn-platform`, `spawn-input` and `spawn-render`); ECS system execution and multi-threaded stages remain `PLANNED`:

```text
loop {
    pump OS events (spawn-platform)  ──►  raw events  ──►  spawn-input
    accumulate monotonic time
    while accumulator >= dt and ticks_this_wakeup < max_catch_up:
        sample input for tick N  ──►  simulation tick N (ECS systems)
        tick N += 1
    drop excess backlog (diagnostic)
    extract presentation state  ──►  spawn-render presents a frame
    wait (timeout) when idle or suspended
}
```

Defaults for tests: 60 Hz fixed step, at most 4 catch-up ticks per wakeup.

### 3.2 Threads

| Stage | Threads |
| --- | --- |
| Foundation | One thread: events, ticks, presentation |
| Target step 1 | Window/input thread → timestamped raw events → bounded queue → game thread |
| Target step 2 | `spawn-jobs` worker pool; ECS runs non-conflicting systems in parallel |
| Target step 3 | Render thread with late latching of camera/input |
| Always | Real-time audio thread (lock-free, allocation-free) |

All threads share one monotonic clock. Each step is introduced only after measurement, with the risks recorded in ADR-0003.

### 3.3 Simulation data flow (client and server)

```text
Client                                     Server (authoritative)
──────                                     ──────────────────────
input sampling (tick-indexed commands) ──► validation (rate, range, time)
local prediction (deterministic movement)  simulation tick N (ECS, physics)
                                           lag-compensated queries (pose history)
reconciliation (rewind + replay)  ◄─────── snapshot (delta vs. last ack,
interpolation of remote entities  ◄───────   interest management, quantized)
presentation (render, audio, UI)
```

Networking (*proposed* ADR-0016): UDP with an in-house protocol, encryption from audited crates, a dedicated network thread exchanging data with the simulation through queues (no async runtime in the engine).

### 3.4 Coordinates and precision (ADR-0010)

- World positions `f64`; local data `f32`; GPU data `f32` relative to the camera.
- Deterministic math (wrapping `libm`) only in prediction/reconciliation code.
- Right-handed, Y-up, −Z forward; reversed-Z depth.

### 3.5 Rendering (ADR-0004)

```text
spawn-render public API (Spawn handles and descriptors)
        │
render graph (later: barriers, transient aliasing)
        │
private RHI module (explicit queues, sync, memory)
        │
Vulkan 1.3 through ash  ──►  swapchain on the window handle (raw-window-handle)
```

Shaders: Slang compiled offline to SPIR-V by `spawn-cli` ([ADR-0014](../adr/0014-shader-toolchain.md)).

### 3.6 Audio (ADR-0006)

```text
game/framework: sound events, occlusion values
        │ commands (bounded queues)
spawn-audio mixer thread: voices, buses, HDR mixing, HRTF
        │
output backend: WASAPI (Windows) | PipeWire (Linux) | null (tests)
```

Steam Audio may add propagation/reflections later as DSP inside the mixer, each by its own ADR.

### 3.7 Physics (ADR-0005)

In-house collision, spatial queries, kinematic character controller mechanism and tick-indexed pose history for lag compensation. Genre movement rules live in the FPS framework. Rigid-body dynamics wait for a benchmark-based decision.

## 4. Content architecture

### 4.1 Asset pipeline (ADR-0015)

```text
sources (glTF/FBX, PNG/JPEG/EXR, audio, RON, Slang)  +  .meta (UUID)
        │  spawn-cli import/cook (gltf, ufbx, meshopt, mikktspace, BCn compression)
cooked binary assets (versioned headers, content-hash cache)
        │  packaged into archives (spawn-io mounts)
spawn-assets runtime loading (async jobs, handles, budgets)
```

### 4.2 Scenes (*proposed* ADR-0021)

Scenes are plain data: nodes with UUIDs and component records keyed by stable type IDs. `spawn-reflect` holds the type registry; `spawn-world` instantiates scenes into ECS worlds.

### 4.3 Packages, extensions and mods (*proposed* ADR-0024, ADR-0025)

- Compile-time extension: games and frameworks register systems, components, loaders and script APIs at startup (static linking only).
- Runtime extension: packages (`base`, `official`, `mod`) with RON manifests; official packages signed; mods untrusted (data, assets, sandboxed Luau only).
- Multiplayer: the server announces its package set; clients must match by content hash.

### 4.4 Gameplay code and scripting (*proposed* ADR-0017 for Luau)

Gameplay is Rust by default: games and frameworks register components and systems with the engine at startup. Luau is optional and talks to the same engine contracts:

```text
gameplay event ──► server VM (rules)   ──► commands ──► applied by Rust at sync points
               ──► client VM (cosmetic) ──► commands ──► presentation only
```

Limits per VM (memory, instructions, frame budget); allowlisted, versioned API.

## 5. Cross-cutting

| Concern | Design | Source |
| --- | --- | --- |
| Errors | Hand-written typed errors with location, code, cause chain; compiler-like reports | ADR-0009 |
| Diagnostics | Own logging/timing facade in `spawn-profiler`; Tracy in development | ADR-0009 |
| Crashes | Panic hook and native crash handler write reports/minidumps; symbols archived per build | ADR-0009 |
| Memory | System allocator; frame arenas, pools; ~2.5 GB VRAM budget on the reference GPU | *proposed* ADR-0020 |
| Anti-cheat | Server authority, validation, information minimization, human-decided penalties | ADR-0019 |
| Platform services | `spawn-services` contract, Steam backend, null backend | *proposed* ADR-0023 |
| Dependencies | Only crates in [DEPENDENCIES](../DEPENDENCIES.md) | ADR-0008 |

## 6. Not yet designed

No ADR exists yet for: animation runtime (formats, compression, retargeting), UI and text architecture, editor architecture, MCP integration, save and replay formats, engine versioning and releases, online backend services. These are designed when their milestone approaches; see [DECISIONS](DECISIONS.md#pending-decisions).
