# Architecture Decision Records

ADRs capture **why** a technical choice was made. They complement, but do not override, [root AGENTS.md](../../AGENTS.md) or the [foundation specification](../foundation/SPEC.md).

## Status definitions

- **Proposed:** candidate evaluated; implementation is **not authorized** by the ADR alone.
- **Accepted:** project owner/team has chosen this approach and related documentation has been synchronized.
- **Superseded:** replaced by a linked newer ADR.
- **Rejected:** considered and declined with rationale.

Update an ADR's status, alternatives, consequences, and supersession links when a decision changes. Never retroactively label an unreviewed proposal Accepted.

## Index

| ADR | Status | Topic |
| --- | --- | --- |
| [ADR-0001](0001-in-house-platform-layer.md) | Accepted | In-house platform layer for Windows and Linux |
| [ADR-0002](0002-platform-owns-raw-input-events.md) | Accepted | Raw input events owned by `spawn-platform`; semantic input in `spawn-input` |
| [ADR-0003](0003-fixed-step-loop-and-threading.md) | Accepted | Fixed-step main loop, tick index and threading model |
| [ADR-0004](0004-in-house-graphics-api.md) | Accepted | In-house graphics API (RHI) over Vulkan; `wgpu` and others as design references |
| [ADR-0005](0005-physics-strategy.md) | Accepted | In-house collision, queries and character controller; rigid-body dynamics deferred to a benchmark |
| [ADR-0006](0006-in-house-audio.md) | Accepted | In-house audio output (WASAPI, PipeWire), mixer, voice management and HRTF; Steam Audio only where in-house is expensive |
| [ADR-0007](0007-toolchain-lints-and-ci.md) | Accepted | Pinned toolchain, strict workspace lints, `Result` for fallible functions, CI on pull requests |
| [ADR-0008](0008-third-party-license-policy.md) | Accepted | No royalties; royalty-free proprietary SDKs isolated in optional crates; copyleft forbidden; reference-code rules; `cargo-deny` |
| [ADR-0009](0009-error-handling-and-diagnostics.md) | Accepted | Hand-written errors with compiler-like traceability, crash reports, own logging facade |
| [ADR-0010](0010-math-library.md) | Accepted | In-house math; right-handed Y-up, reversed-Z, `f64` world positions, deterministic subset, degrees in data/scripts |
| [ADR-0011](0011-ecs-architecture.md) | Accepted | In-house ECS (archetypes + sparse sets, deterministic scheduling); `bevy_ecs` as primary reference |
| [ADR-0012](0012-job-system.md) | Accepted | In-house work-stealing job system in a new `spawn-jobs` crate (tasks, no fibers) |
| [ADR-0013](0013-declarative-data-format.md) | Accepted | RON + `serde` for authoring, cooked binary at runtime |
| [ADR-0014](0014-shader-toolchain.md) | Accepted | Slang compiled by `spawn-cli`, never in `build.rs` |
| [ADR-0015](0015-asset-pipeline.md) | Accepted | glTF primary + FBX via ufbx; PNG/JPEG/EXR by role; UUID `.meta` identity; cooked formats |
| [ADR-0016](0016-networking-and-replication.md) | Proposed | In-house UDP protocol, snapshot deltas, prediction, lag compensation |
| [ADR-0017](0017-luau-scripting-runtime.md) | Proposed | `mlua` + Luau: VMs, sandbox, limits, API versioning |
| [ADR-0018](0018-gamepad-input.md) | Proposed | Native gamepad backends (XInput, HID, evdev, hidraw) |
| [ADR-0019](0019-anti-cheat-and-client-trust.md) | Accepted | No kernel anti-cheat; server validation; evaluation of user-mode AC, AI detection and integrity checks |
| [ADR-0020](0020-memory-strategy.md) | Proposed | System allocator, frame arenas, pools, memory budgets |
| [ADR-0021](0021-scene-ecs-relationship.md) | Proposed | Scenes as data via reflection registry; no scene→ECS dependency |
| [ADR-0022](0022-repository-layout.md) | Proposed | `frameworks/`, `examples/`, `tests/`; games in separate repositories |
| [ADR-0023](0023-platform-services-integration.md) | Proposed | Steamworks and other platform services behind `spawn-services`, isolated per ADR-0008 |
| [ADR-0024](0024-packages-and-game-extensions.md) | Proposed | Compile-time Rust extension by registration; one package format for base game, DLC and mods; trust levels |
| [ADR-0025](0025-mods-and-workshop.md) | Proposed | Sandboxed mods (no native code), Steam Workshop, server-required mods, competitive integrity |

ADRs 0001–0004 close Phase 0 of the [implementation plan](../foundation/PLAN.md). The root project's open architectural questions take precedence for any topic without an accepted ADR.
