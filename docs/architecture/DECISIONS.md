# Decision Register

Consolidated record of every architectural decision: what was decided, which crates it governs, and when it must be revisited. The [ADR](../adr/README.md) is authoritative; this register is the overview and must stay in sync with it (SPN-INV-101 in [INVARIANTS](INVARIANTS.md)).

**Statuses:** `Accepted` (binding), `Proposed` (authorizes nothing yet), `Pending` (no ADR yet).

## ADR decisions

| ADR | Status | Decision | Governs | Revisit when |
| --- | --- | --- | --- | --- |
| [0001](../adr/0001-in-house-platform-layer.md) | Accepted | In-house platform layer; Windows (Win32) and Linux (Wayland, X11); no macOS; Spawn owns the event loop; thin binding crates | `spawn-platform` | Gamepads or a second platform are added |
| [0002](../adr/0002-platform-owns-raw-input-events.md) | Accepted | Raw events owned by `spawn-platform`, compiled without backends; semantic input in `spawn-input` | `spawn-platform`, `spawn-input` | Feature split proves insufficient |
| [0003](../adr/0003-fixed-step-loop-and-threading.md) | Accepted | Fixed step (60 Hz test default, 4 catch-up ticks, drop backlog); `u64` tick in `spawn-core`; root binary drives the loop; single thread first, staged threading target with recorded risks | `spawn-core`, root binary | Measurements show the main thread is the bottleneck |
| [0004](../adr/0004-in-house-graphics-api.md) | Accepted | In-house RHI over Vulkan 1.3 (`ash`, in-house allocator, Slang → SPIR-V); `wgpu` and others as references | `spawn-render` | Mesh/camera milestone or measured limitation |
| [0005](../adr/0005-physics-strategy.md) | Accepted | In-house collision, queries, character controller, lag-compensation history; dynamics deferred to benchmark | `spawn-physics` | Dynamics milestone (props, ragdolls, vehicles) |
| [0006](../adr/0006-in-house-audio.md) | Accepted | In-house output (WASAPI, PipeWire only), mixer, voices, HDR mixing, HRTF; Steam Audio only for expensive features; `opus` + `lewton` decoding | `spawn-audio` | Propagation/reflections needed |
| [0007](../adr/0007-toolchain-lints-and-ci.md) | Accepted | Pinned Rust 1.99.1; strict lints; `Result` for fallible functions; CI on pull requests; local check command | workspace | Nightly needed; GPU tests needed |
| [0008](../adr/0008-third-party-license-policy.md) | Accepted | No royalties; permissive OSS anywhere; royalty-free proprietary SDKs isolated; no strong copyleft; reference-code rules | all | Paid/review-required license needed; consoles |
| [0009](../adr/0009-error-handling-and-diagnostics.md) | Accepted | Hand-written errors with location, code, cause chain; `unwind`; crash reports and minidumps; own logging facade; Tracy in development | all | First shipped native code (minidump implementation) |
| [0010](../adr/0010-math-library.md) | Accepted | In-house math; right-handed Y-up −Z forward; reversed-Z; `f64` world positions; deterministic subset via `libm`; degrees in data/Luau | `spawn-math`, all | SIMD/bandwidth bottleneck |
| [0011](../adr/0011-ecs-architecture.md) | Accepted | In-house ECS; archetypes + sparse sets; tick change detection; command buffers; deterministic scheduling; `bevy_ecs` as reference | `spawn-ecs` | Measured iteration/structural-change bottleneck |
| [0012](../adr/0012-job-system.md) | Accepted | In-house work-stealing jobs in new `spawn-jobs` crate; `crossbeam-deque`; tasks, no fibers; configurable workers | `spawn-jobs` | Contention or idle workers measured |
| [0013](../adr/0013-declarative-data-format.md) | Accepted | RON + `serde` authoring; cooked binary runtime; `version` + migrations; unknown fields are errors | data-defining crates | Modders find the format a barrier |
| [0014](../adr/0014-shader-toolchain.md) | Proposed | Slang compiled by `spawn-cli`, never `build.rs`; pinned compiler; SPIR-V not committed; hot reload | `spawn-cli`, `spawn-render` | — |
| [0015](../adr/0015-asset-pipeline.md) | Accepted | Offline cook in `spawn-cli`; glTF primary, FBX via ufbx; PNG/JPEG/EXR by role; BCn; UUID `.meta`; content-hash cache | `spawn-assets`, `spawn-cli` | Load times or memory exceed budgets |
| [0016](../adr/0016-networking-and-replication.md) | Proposed | In-house UDP protocol; audited crypto; snapshot deltas; prediction/reconciliation; interpolation; lag compensation; interest management | `spawn-network` | — |
| [0017](../adr/0017-luau-scripting-runtime.md) | Proposed | `mlua` + Luau; separate client/server VMs; sandbox; limits; versioned API; type checking | `spawn-scripting` | — |
| [0018](../adr/0018-gamepad-input.md) | Proposed | Native gamepad backends (XInput, HID, evdev, hidraw); SDL GameControllerDB mappings | `spawn-platform`, `spawn-input` | — |
| [0019](../adr/0019-anti-cheat-and-client-trust.md) | Accepted | No kernel anti-cheat; server validation; information minimization; humans decide penalties; ToS disclosure; evaluation tracks | `spawn-network`, games | Cheating measurably harms the game |
| [0020](../adr/0020-memory-strategy.md) | Proposed | System allocator; frame arenas; pools; subsystem budgets; ~2.5 GB VRAM target | all | — |
| [0021](../adr/0021-scene-ecs-relationship.md) | Proposed | Scenes as data via reflection registry; no scene → ECS dependency; `spawn-world` instantiates | `spawn-scene`, `spawn-reflect`, `spawn-world` | — |
| [0022](../adr/0022-repository-layout.md) | Proposed | `frameworks/`, `examples/`, root `tests/`; games in separate repositories | workspace | — |
| [0023](../adr/0023-platform-services-integration.md) | Proposed | `spawn-services` contract + Steam backend + null backend; SDK isolated | integrations | — |
| [0024](../adr/0024-packages-and-game-extensions.md) | Proposed | Compile-time Rust extension only; one package format (base, official, mod); trust levels; `spawn-packages` | packages, scripting, assets | — |
| [0025](../adr/0025-mods-and-workshop.md) | Proposed | Sandboxed mods, no native code; Workshop, local and server downloads; competitive integrity | packages, scripting | — |

## Decisions recorded outside ADRs

| Decision | Status | Where |
| --- | --- | --- |
| Rust is a first-class gameplay language; Luau is optional (games may be Rust-only; engine and frameworks never require `spawn-scripting`) | Accepted | Root [Rust and Luau](../../AGENTS.md#rust-and-luau), SPN-INV-069 |
| Document precedence: root `AGENTS.md` → crate `AGENTS.md` → milestone specs → ADRs (rationale) | Accepted | Root [AGENTS.md](../../AGENTS.md) |
| Reference hardware: Ryzen 5 1600, RX 460 (RADV, Vulkan 1.4), Linux with PipeWire; minimum target | Accepted | Root [Accepted Decisions](../../AGENTS.md#accepted-decisions) |
| First smoke-test platform: Linux, Wayland (Hyprland) | Accepted | [foundation SPEC](../foundation/SPEC.md) |
| Approved, rejected and deferred third-party crates | Accepted | [DEPENDENCIES](../DEPENDENCIES.md) |
| `symphonia` rejected (MPL-2.0) | Accepted | [DEPENDENCIES](../DEPENDENCIES.md#rejected) |
| Online backend services (`tokio`, `axum`, `tower`, `sqlx`) deferred; never in engine crates or the game UDP path | Accepted | [DEPENDENCIES](../DEPENDENCIES.md#deferred) |
| Stray `tools./` duplicate package removed | Done | Repository history |

## Pending decisions

### Open questions on proposed ADRs

| Topic | Question | ADR |
| --- | --- | --- |
| Extensions | Is "how a game grows after release (DLC, expansions, mods)" the intended meaning? No dynamic Rust plugins or native mods? | [0024](../adr/0024-packages-and-game-extensions.md) |
| Competitive integrity | Ranked modes only with official content and approved cosmetic mods? | [0025](../adr/0025-mods-and-workshop.md) |
| Non-Steam mods | Support mods for players outside Steam (e.g. mod.io) now or later? | [0025](../adr/0025-mods-and-workshop.md) |
| Shader toolchain | Verify the Slang compiler license (blocks the first shader) | [0014](../adr/0014-shader-toolchain.md) |

### Choices made at implementation time

| Choice | Options | Source |
| --- | --- | --- |
| Benchmark framework | `criterion` or `divan` | [DEPENDENCIES](../DEPENDENCIES.md) |
| Archive compression | `zstd` or `ruzstd` | [DEPENDENCIES](../DEPENDENCIES.md) |
| Anti-cheat model inference | `tract` or `candle` | [DEPENDENCIES](../DEPENDENCIES.md) |
| Minidump writer | in-house or `minidump-writer` | [ADR-0009](../adr/0009-error-handling-and-diagnostics.md) |
| Physics dynamics | in-house (Jolt as reference), Jolt, or Rapier, by benchmark | [ADR-0005](../adr/0005-physics-strategy.md) |
| Default worker count | per physical core or per logical thread, by benchmark | [ADR-0012](../adr/0012-job-system.md) |

### Areas without an ADR

| Area | Needed before |
| --- | --- |
| Animation runtime (formats, compression, retargeting) | Animation milestone |
| UI and text architecture (approved crates exist: `cosmic-text`, `rustybuzz`, `swash`) | First in-game UI |
| Editor architecture | First editor work |
| MCP integration (`rmcp` approved, maturity to confirm) | First `spawn-mcp` work |
| Save and replay formats | Replays for anti-cheat review / save games |
| Engine versioning and releases | First external use of the engine |
| Online backend services | Public multiplayer testing |
