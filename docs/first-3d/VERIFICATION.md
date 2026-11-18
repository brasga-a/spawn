# Verification and Acceptance — First 3D and Free Camera

**Status:** `PLANNED`. None of `F3D-T01..T13` exist yet (`SPN-INV-100`). All Foundation verification cases (`FND-T01..T10` in [foundation/VERIFICATION.md](../foundation/VERIFICATION.md)) must remain green throughout every phase.

## Required automated checks

Every commit passes Gate G0 (`./scripts/check.sh`) without `slangc`, a display server, or a GPU installed. In addition, the milestone adds headless automated tests (`F3D-T01..T10`, `F3D-T13`), a parser fuzz target (Gate G2), and opt-in Vulkan 1.3 headless-surface GPU readback tests (`F3D-T11`, `F3D-T12`):

| Test ID | Requirement / invariant | Scenario | Pass condition |
| --- | --- | --- | --- |
| F3D-T01 | `F3D-FR-01` / `F3D-INV-05,15` | Headless `spawn-math` unit tests, `proptest` algebraic laws, `glam` test-oracle comparisons (`Vec2/3/4`, `Mat3/4`, `Quat`, `Transform`, `Aabb`, `Ray`, `Plane`, `Sphere`), and degenerate inputs (zero-vector normalize, singular inverse, NaN). | Identities and oracle comparisons hold within documented `f32` tolerances; zero panics on degenerate inputs. |
| F3D-T02 | `F3D-FR-02` / `F3D-INV-05,07` | Headless right-handed perspective projection tests (finite and infinite-far reversed-Z) and clip/framebuffer winding sign check. | Eye `(0, 0, -near)` → NDC `z = 1.0`; `(0, 0, -far)` → `z = 0.0`; strict depth ordering `z(d1) > z(d2)` for `d1 < d2`; `+Y` → Vulkan NDC `-Y` in one site only; right-handed CCW front triangle yields Vulkan `COUNTER_CLOCKWISE` (`a < 0`). |
| F3D-T03 | `F3D-FR-01,10` / `F3D-INV-06` | Headless `f64` world-position separation and large-coordinate camera-relative precision test at `±50_000.0 m` (`50 km`) from world origin. | `WorldPos` and local `Vec3` cannot mix implicitly; `(mesh_f64 - cam_f64)` converted explicitly to `f32` for a `1.0 m` offset at `50 km` retains `< 1e-5 m` relative precision (no ~4 mm `f32` world quantization). |
| F3D-T04 | `F3D-FR-03` / `F3D-INV-08` | Headless deterministic math golden-vector table (`sin`, `cos`, `sin_cos`, `atan2`, clamped `asin`/`acos`, `exp`, `ln`, IEEE `sqrt`) in `f32` and `f64` across quadrant boundaries, tiny/large magnitudes, and subnormals. | Exact `to_bits()` match against recorded reference vectors on both Linux and Windows CI. |
| F3D-T05 | `F3D-FR-09,10` / `F3D-INV-08,09,10` | Headless 60 Hz `f64` free-fly camera simulation and `alpha` presentation interpolation across 240 fps (0-tick sub-frames), 60 fps, and 15 fps (catch-up-clamped) schedules, pitch `±π/2` pole clamping, `±π` quaternion wrap, and focus-loss input reset. | Identical `Tick`-indexed `f64` poses across frame rates; `alpha ∈ [0.0, 1.0)`; shortest-arc slerp; simulation pose bitwise unchanged before and after presentation extraction. |
| F3D-T06 | `F3D-FR-04` / `F3D-INV-11,12,15` | **Proposal (per [ADR-0014](../adr/0014-shader-toolchain.md)):** Headless `spawn-cli` shader-cook tests using an injected mock `slangc` runner in `std::env::temp_dir()` (always run in CI), plus an opt-in real-`slangc` test when the pinned compiler is installed. | Missing or wrong-version `slangc` fails with located `CLI-0001`+ error and nonzero exit; valid run emits uncommitted SPIR-V + reflection artifact; malformed shader forwards file/line diagnostics. |
| F3D-T07 | `F3D-FR-05,12` / `F3D-INV-11,12,15` | Headless cooked-shader & reflection validator tests in `spawn-render` (`default-features = false`, synthesized SPIR-V word buffers in Rust) plus a `cargo-fuzz` target (Gate G2). | Valid fixture loads; bad magic/version, unaligned bytes, oversized payloads, missing entry points, and vertex/push-constant/descriptor mismatches return located `RenderError`s (`REND-0010`+) with `insta` report snapshots and zero panics. |
| F3D-T08 | `F3D-FR-06,12` / `F3D-INV-13,15` | Headless unit/property tests of `spawn-render`'s pure GPU block suballocator and timeline-retirement tracker (no GPU required). | Honors alignment and buffer-image granularity; coalesces freed ranges; retires allocations only when completed timeline ≥ retirement value; returns typed OOM error on budget exhaustion; detects unretired leaks. |
| F3D-T09 | `F3D-FR-07` / `F3D-INV-09,14` | Headless presentation & depth-attachment lifecycle tests across `Ready` → `0x0` `Suspended` → `Ready` (`w1×h1` → `w2×h2`) → `Recovering` → `Stopped`. | Depth attachment recreates idempotently on extent change, never allocates `0x0` while suspended, and never resets simulation `Tick`. |
| F3D-T10 | `F3D-FR-11,12` / `F3D-INV-09,10,15` | Headless `spawn-engine` 3D session & failure-injection tests over `RuntimeBackend` (shader load, mesh upload, pipeline init, draw failure, cursor-lock toggle, resize/suspend, repeated close). | Partial startup or fatal frame failure unwinds scene handles → renderer → window → platform in reverse order once each; `ENGN-*` error snapshots match [ADR-0009](../adr/0009-error-handling-and-diagnostics.md). |
| F3D-T11 | `F3D-FR-06,08` / `F3D-NFR-06`, `F3D-INV-13,14` | Opt-in GPU test (`cargo test -p spawn-render --features vulkan -- --ignored`, `VK_EXT_headless_surface` + `VK_LAYER_KHRONOS_validation` + sync validation): dynamic-rendering colored triangle + host-visible pixel readback. | Center pixel matches expected triangle color; corner pixels match clear color; `validation_error_count() == 0`; zero leaked GPU memory blocks on drop. |
| F3D-T12 | `F3D-FR-06,07,08,10` / `F3D-NFR-06`, `F3D-INV-06,07,13,14` | Opt-in GPU test (`--features vulkan -- --ignored` + validation + sync validation): two overlapping 3D meshes at `-2.0 m` and `-5.0 m` submitted **far-after-near**, a `±50 km` camera-relative case, and a resize + `0x0` suspend/resume sequence with pixel readback. | Overlap pixel equals the `-2.0 m` mesh color (proving reversed-Z `GREATER_OR_EQUAL` + `D32_SFLOAT` clear `0.0` over submission order); backfaces culled; resize/suspend clean; `validation_error_count() == 0`. |
| F3D-T13 | `F3D-NFR-01..05,07` / `F3D-INV-01..04,11,16` | Automated workspace manifest, `cargo tree`, `build.rs`, `.spv`-tracking, ADR/status consistency, and public-API audit test. | Acyclic graph; allowed crate edges only; zero window/GPU crates in headless `spawn-math`, `spawn-cli`, and default `spawn-render`; zero `build.rs` compiler calls or tracked `.spv` blobs; all crates in [DEPENDENCIES.md](../DEPENDENCIES.md); `AGENTS.md` status tags match code; zero genre terms or leaked 3rd-party types in public APIs. |

## Manual smoke tests (record environment)

On **at least one explicitly named tested platform** (first target: the reference hardware in root [Accepted Decisions](../../AGENTS.md#accepted-decisions) — AMD Ryzen 5 1600, AMD Radeon RX 460 Polaris / RADV, Linux/Wayland/Hyprland, with `VK_LAYER_KHRONOS_validation` + `VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true`):

1. **Offline shader cook (*after Phase 0 ADR-0014 acceptance*):** Record `slangc` version, run `cargo run -p spawn-cli -- shaders build`, verify exit `0` and uncommitted SPIR-V + reflection artifacts in build output; verify wrong/missing `slangc` (`PATH` probe) prints a `CLI-0001`+ report and exits nonzero.
2. **Interactive 3D scene & camera:** Launch `spawn-engine` with Khronos validation + sync validation enabled; confirm procedural 3D meshes appear with correct perspective, right-side-up orientation (`+Y` up), backface culling, and front-to-back reversed-Z occlusion.
3. **Controls & cursor capture:** Toggle cursor lock (`CursorGrabMode::Locked`), rotate yaw/pitch smoothly via unaccelerated relative mouse motion (confirm pitch clamps cleanly near straight up/down without flipping), translate along forward/back, right/left, and up/down axes, and confirm releasing focus or toggling unlock restores normal cursor behavior without stuck movement.
4. **Resize, suspend & close:** Exercise tiling/floating/pixel resizes (verifying swapchain + `D32_SFLOAT` depth recreation without visual corruption or clock reset), zero-size/minimized suspension where supported by the compositor, and clean close via `Escape` and compositor `xdg_toplevel.close`.
5. **Failure diagnostics & resource hygiene:** Probe missing cooked shader directory (`REND-0010`+ / `ENGN-*`), unavailable display (`ENGN-0004`), and unavailable Vulkan driver (`ENGN-0005`); record steady-state frame rate, CPU time, GPU allocator block usage, and `validation_errors=0`.

A Linux/Wayland smoke run does **not** claim Windows or X11 support (`SPN-INV-090`).

## Recorded smoke runs

*None yet — Milestone 2 is `PLANNED`. Record each phase's environment, exact command, observed metrics/pixels/diagnostics, and unverified boundaries here as implementation lands, matching the table format in [foundation/VERIFICATION.md](../foundation/VERIFICATION.md#recorded-smoke-runs).*

## Traceability

| Requirement | Invariants | Automated tests | Delivery phase |
| --- | --- | --- | --- |
| `F3D-FR-01` (`spawn-math` primitives & `f64` world pos) | `F3D-INV-01, 04, 05, 06, 15` | `F3D-T01`, `F3D-T03`, `F3D-T13` | Phase 1 |
| `F3D-FR-02` (reversed-Z & single-site Vulkan Y-flip) | `F3D-INV-05, 07` | `F3D-T02`, `F3D-T12` | Phase 1, Phase 4 |
| `F3D-FR-03` (`libm` deterministic subset) | `F3D-INV-04, 08` | `F3D-T04`, `F3D-T05` | Phase 1, Phase 5 |
| `F3D-FR-04` (`spawn-cli` offline Slang cook — *proposal*) | `F3D-INV-01, 02, 11, 12, 15, 16` | `F3D-T06`, `F3D-T13` | Phase 0, Phase 2 |
| `F3D-FR-05` (`spawn-render` shader & reflection validation) | `F3D-INV-02, 11, 12, 15` | `F3D-T07` (+ fuzz) | Phase 2, Phase 3 |
| `F3D-FR-06` (in-house GPU block allocator & staging) | `F3D-INV-04, 13, 15` | `F3D-T08`, `F3D-T11`, `F3D-T12` | Phase 3, Phase 4 |
| `F3D-FR-07` (`D32_SFLOAT` reversed-Z depth & resize/suspend) | `F3D-INV-07, 09, 14` | `F3D-T09`, `F3D-T12` | Phase 3, Phase 4 |
| `F3D-FR-08` (Vulkan 1.3 dynamic-rendering pipelines & draws) | `F3D-INV-07, 13, 14` | `F3D-T11`, `F3D-T12` | Phase 3, Phase 4 |
| `F3D-FR-09` (60 Hz `f64` simulation free-fly camera) | `F3D-INV-03, 05, 08, 09, 10` | `F3D-T05`, `F3D-T10`, `F3D-T13` | Phase 5 |
| `F3D-FR-10` (`alpha` interpolation & camera-relative `f32`) | `F3D-INV-06, 09, 10` | `F3D-T03`, `F3D-T05`, `F3D-T10`, `F3D-T12` | Phase 4, Phase 5 |
| `F3D-FR-11` (interactive procedural 3D scene in `spawn-engine`) | `F3D-INV-03, 09, 10, 14, 16` | `F3D-T10` + manual smoke | Phase 5, Phase 6 |
| `F3D-FR-12` (located errors & reverse-order unwind) | `F3D-INV-12, 13, 15` | `F3D-T06`, `F3D-T07`, `F3D-T08`, `F3D-T10` + smoke | Phases 1–6 |
| `F3D-NFR-01..07` (genre-free, boundaries, deps, headless, safety, validation, evidence) | `F3D-INV-01..04, 11..16` | `F3D-T06..T13`, `./scripts/check.sh`, Gate G3 | Phases 0–6 |

## Exit gate

Milestone 2 satisfies [Gate G3](../architecture/GATES.md#g3--milestone-exit) **only when**:
1. Phase 0 owner decisions (`ADR-0014` acceptance, Slang license verification in `DEPENDENCIES.md`, pinned `slangc` version, and any optional `ADR-0020`/`0022` decisions) are recorded and synchronized across all governance docs (`SPN-INV-101`).
2. `./scripts/check.sh` passes cleanly (including `FND-T01..T10`, `F3D-T01..T10`, `F3D-T13`, rustdoc `-D warnings`, and `cargo deny check`) without `slangc`, a display server, or a GPU.
3. The cooked-shader binary/reflection parser fuzz target builds and runs cleanly over a bounded seed corpus without panics or OOM (`Gate G2`).
4. Opt-in Vulkan 1.3 GPU readback tests (`F3D-T11`, `F3D-T12`) pass on the reference RX 460 with `VK_LAYER_KHRONOS_validation` + synchronization validation reporting `0` errors (`SPN-INV-092`).
5. Manual smoke runs of `spawn-cli` shader cooking and `spawn-engine` interactive 3D navigation, resize/recovery, cursor capture, and failure diagnostics are recorded on the reference Linux/Wayland/Hyprland hardware with frame-time and GPU-memory observations (`SPN-INV-090`, `091`).
6. Affected `AGENTS.md` status tags (`spawn-math`, `spawn-cli`, `spawn-render`, root `AGENTS.md`) and `DEPENDENCIES.md` (`In use` column) match the verified implementation (`SPN-INV-100`).
7. Unsupported platforms (Windows, X11), untested GPUs/compositors, and any unexercised recovery edges are listed explicitly.
