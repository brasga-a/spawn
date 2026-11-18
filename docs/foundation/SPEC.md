# Specification — First Native Runtime Slice

**Status:** `EXISTS` on Linux/Wayland + Vulkan 1.3 (X11 and Windows backends remain `PLANNED`; see [VERIFICATION.md](VERIFICATION.md) for automated test and smoke-run evidence).  
**Audience:** contributors implementing `spawn-core`, `spawn-platform`, `spawn-input`, `spawn-render`, and the root `spawn-engine` executable.

## Goal

Provide a minimal, genre-independent Spawn runtime that can be started, can run a fixed simulation cadence independently of presentation cadence, can display a GPU-backed clear frame in a native window, handles normal window/input events and exits predictably.

This is **not** a full game loop with ECS gameplay, physics, networking, scripts, or content pipelines.

## Functional requirements

| ID | Requirement | Observable acceptance |
| --- | --- | --- |
| FND-FR-01 | The executable starts and creates one native top-level window when its target environment supports it. | Window becomes visible; failed startup returns a diagnostic error. |
| FND-FR-02 | The platform layer delivers raw window, keyboard and pointer events in a documented order without directly mutating gameplay state. | A synthetic event test confirms translation and ordering; escape/close policy is handled by the composition root. |
| FND-FR-03 | The application handles close requests and orderly termination. | OS close control terminates without a panic or lingering event loop. |
| FND-FR-04 | The render layer initializes a graphics device/surface and presents a single clear color. | Window visibly presents repeated frames; no shader, mesh or editor is needed. |
| FND-FR-05 | Resize and minimize/zero-size states do not crash the application. | Valid resize reconfigures the surface; zero size suspends presentation until valid. |
| FND-FR-06 | Surface out-of-date/lost conditions are handled according to a documented recovery policy; unrecoverable failures result in controlled termination. | Tests of renderer state transitions and a manual minimize/resize scenario. |
| FND-FR-07 | A monotonic clock drives a fixed-step simulation schedule that is independent of render frequency. | Headless timing tests prove consistent tick counts under equivalent elapsed-time sequences within the defined catch-up policy. |
| FND-FR-08 | Long elapsed-time gaps do not create an unbounded catch-up loop. | Unit test verifies configured maximum simulation steps per update and documented backlog policy. |
| FND-FR-09 | Input state captures press/release and pointer motion and can be sampled per simulation tick. | Synthetic input tests confirm press/hold/release semantics and that actions are not processed twice accidentally. |
| FND-FR-10 | Initialization and shutdown respect resource ownership; partially initialized startup can unwind safely. | Failure-injection or state-machine tests cover each supported initialization failure edge. |
| FND-FR-11 | The application exposes understandable diagnostics for startup, device/surface failures and shutdown. | The [manual smoke test](VERIFICATION.md#manual-smoke-tests-record-environment) records the diagnostics observed for each failure case; errors include context without leaking backend internals as public API. |

## Non-functional requirements

| ID | Requirement | Verification |
| --- | --- | --- |
| FND-NFR-01 | Generic infrastructure contains no genre-specific concepts such as player, weapon, team or health. | Code review / dependency audit. |
| FND-NFR-02 | Engine subsystem dependencies are acyclic; no renderer dependency in `spawn-platform`, `spawn-input`, or `spawn-core`. | `cargo metadata` and import review. |
| FND-NFR-03 | The timing and input state-transition logic can be tested without a window or GPU. | Headless unit tests in CI. |
| FND-NFR-04 | Errors in expected runtime paths (window/surface creation, resize, device loss) are handled, not unchecked panics. | Tests, review, and manual smoke test. |
| FND-NFR-05 | The runtime can be exited without leaked native resources or background activity owned by Spawn. | Repeated start/stop and failure-injection tests; platform-specific manual check. |
| FND-NFR-06 | No hard-coded promises of FPS, frame time, multi-platform support or deterministic GPU output without measurements. | Performance evidence, tested platform matrix and scope review. |

## Out of scope

ECS system scheduling/implementation, physics, Luau, networking, resource/asset pipelines, editor widgets, rendering meshes or 3D scenes, UI framework selection, runtime reflection, multiplayer, and a general plugin system. The generic loop must not assume those subsystems exist.

## Assumptions and blockers

- Platform layer: in-house, Windows and Linux, accepted in [ADR-0001](../adr/0001-in-house-platform-layer.md). macOS is out of scope.
- Graphics: in-house graphics API over Vulkan 1.3, accepted in [ADR-0004](../adr/0004-in-house-graphics-api.md).
- Raw event ownership and the platform→input boundary: accepted in [ADR-0002](../adr/0002-platform-owns-raw-input-events.md).
- Composition root, clock ownership and scheduling boundary: accepted in [ADR-0003](../adr/0003-fixed-step-loop-and-threading.md).
- First smoke-test platform: Linux on Wayland (Hyprland).
- Platform support is only claimed for environments actually smoke-tested. Headless CI must not require graphics hardware.

## Definition of done

All applicable **automated** cases in [VERIFICATION.md](VERIFICATION.md) pass; a manual smoke run on at least one named platform demonstrates a visible clear frame, resize/minimize recovery and graceful close; invariants remain satisfied; accepted ADRs match the implementation. Passing `cargo check` or the preexisting scaffold tests alone is insufficient.
