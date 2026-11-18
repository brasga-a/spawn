# Technical Design — First 3D Rendering and Free Camera

**Status:** `PLANNED` DESIGN PROPOSAL. Type, module, and command names in this document illustrate intended responsibilities and contracts; none exist in source yet (`SPN-INV-100`). Elements governed by *Proposed* [ADR-0014](../adr/0014-shader-toolchain.md), [ADR-0020](../adr/0020-memory-strategy.md), or [ADR-0022](../adr/0022-repository-layout.md) are explicitly marked **proposal (pending Phase 0 owner acceptance)**.

## 1. Architecture and crate boundaries

```text
Offline tool path (proposal, pending Phase 0 acceptance of ADR-0014):
  shaders/*.slang ──► spawn-cli (invokes pinned slangc) ──► build output (.spv + reflection)
                            │ (optional headless format reuse: spawn-render default-features=false)
                            ▼
Runtime composition path (single-threaded, ADR-0003):
  spawn-engine (root binary: 60 Hz f64 free-fly camera + alpha extraction + procedural 3D scene)
     ├── spawn-core       FixedStepClock, Tick, pending(), tick_duration()
     ├── spawn-math       Vec/Mat/Quat/Transform, f64 world pos, reversed-Z, libm det subset
     ├── spawn-platform   Wayland window, relative pointer motion, cursor lock/confinement
     ├── spawn-input      DeviceState, InputMap, tick-sampled InputCommand
     └── spawn-render     cooked shader validation + private Vulkan 1.3 RHI (allocator, buffers,
            │             D32_SFLOAT reversed-Z depth, dynamic-rendering pipelines)
            └──► spawn-math (default-features = false)

Forbidden edges:
  spawn-math, spawn-core !→ any spawn-* crate or tool (SPN-INV-004)
  spawn-render !→ spawn-platform, spawn-input, spawn-core, spawn-cli (SPN-INV-001, 006)
  all crates/spawn-* and spawn-engine !→ tools/spawn-cli or game frameworks (SPN-INV-001)
```

### Boundary and dependency matrix

| Package | Allowed `spawn-*` deps | Approved 3rd-party deps ([DEPENDENCIES.md](../DEPENDENCIES.md)) | Restrictions |
| --- | --- | --- | --- |
| `spawn-math` | none | `libm`, `bytemuck` (if POD casts needed); dev-only: `glam` (oracle), `proptest`, `insta` | Zero OS/GPU deps; never logs; no third-party types in public API; error prefix `MATH-0001`+ if needed. |
| `spawn-core` | none | dev-only: `proptest`, `insta` | Existing `FixedStepClock` unchanged unless ≥ 2 crates share an `alpha` helper ([spawn-core AGENTS.md](../../crates/spawn-core/AGENTS.md)). |
| `spawn-cli` (*proposal per ADR-0014*) | `spawn-render` (`default-features = false`, headless format/validation only) if shared binary header lives there | `clap`, `rspirv`, external `slangc` (pending Phase 0 license sign-off); dev-only: `proptest`, `insta` | Headless (`SPN-INV-009`); never links `vulkan` or `wayland` (`tools/spawn-cli/AGENTS.md` refined in Phase 2 — see below); error prefix `CLI-0001`+. `serde_json`, `tempfile`, and `blake3` (in `spawn-cli`) are **not** approved without a Gate G2 `DEPENDENCIES.md` update. |
| `spawn-render` | `spawn-math` (`default-features = false`) | `bytemuck` (always or `vulkan`), `ash` + `raw-window-handle` (`vulkan` feature only); dev-only: `proptest`, `insta`, `cargo-fuzz` + `arbitrary` | `default = []` stays GPU-free; all `ash`/`unsafe` stays in private `src/vulkan/`; new error codes start at `REND-0010` (`REND-0001..0009` already assigned). |
| `spawn-engine` | `spawn-core`, `spawn-math`, `spawn-input`, `spawn-platform`, `spawn-render` | none required (`smallvec`/`arrayvec`/`bitflags` available if needed) | `#![forbid(unsafe_code)]`; new error codes start at `ENGN-0008` (`ENGN-0001..0007` already assigned); keeps layout in root `src/` / `examples/` unless *Proposed* [ADR-0022](../adr/0022-repository-layout.md) is accepted. |

Adding `spawn-render -> spawn-math`, `spawn-engine -> spawn-math`, and (if shared cooked-shader types live in `spawn-render`) `spawn-cli -> spawn-render (default-features = false)` requires updating root `AGENTS.md`, `crates/spawn-render/AGENTS.md`, `tools/spawn-cli/AGENTS.md`, and `docs/DEPENDENCIES.md` (`In use` column) in the same change (`SPN-INV-101`, Gate G0/G2). Note: `tools/spawn-cli/AGENTS.md` currently states *"Headless: must not require `spawn-platform` windowing or `spawn-render`, except for explicitly graphical commands"*; Phase 2 refines that sentence (per root `AGENTS.md` doc-conflict rules) to forbid `spawn-render` GPU backends (`vulkan` feature) while allowing headless `spawn-render` (`default-features = false`) so engine validation rules are called rather than duplicated.

## 2. Spatial math, coordinates, and determinism (`spawn-math`, ADR-0010)

### 2.1 Types and coordinate conventions (`SPN-INV-050..054`)

- **Local `f32` types:** `Vec2`, `Vec3`, `Vec4`, `Mat3`, `Mat4` (column-major `[col][row]`, multiplying column vectors `M * v` to match Slang/SPIR-V defaults), `Quat` (`(x, y, z, w)` unit quaternion as canonical rotation), `Transform` (`Vec3` translation, `Quat` rotation, `Vec3` scale), and geometric primitives `Aabb`, `Ray`, `Plane`, `Sphere`. All types are `Copy`, lifetime-free, and heap-free.
- **World `f64` position type:** A distinct `f64` world-space point type (e.g. `WorldPos` backed by `DVec3`, not interchangeable with `Vec3`). Difference between two world positions `(a - b)` yields an `f64` displacement vector (`DVec3`), converted to local `Vec3` (`f32`) only through an explicit camera-relative conversion method (`to_vec3_lossy` / `camera_relative_f32(world_pos, camera_pos)`), never via `From` or implicit casts (`SPN-INV-051`, `SPN-INV-052`).
- **Axes and units:** Right-handed, `+X` right, `+Y` up, `−Z` forward; meters, seconds, kilograms; **radians** across all Rust functions. Configuration or CLI options accepting human-authored angles name the unit explicitly (e.g. `fov_degrees: f32`) and convert once at the boundary (`SPN-INV-054`).
- **Euler convention (documented once):** Intrinsic yaw around world `+Y` followed by pitch around local `+X` (roll `0` for free-fly look), producing a normalized `Quat`. Zero yaw and zero pitch look down `−Z` with `+X` right and `+Y` up.
- **Degenerate-input safety (`SPN-INV-030`, `031`):** Zero-length normalization (`try_normalize -> Option<Self>`, `normalize_or_zero`), zero-determinant `Mat3`/`Mat4::inverse -> Option<Self>`, and invalid projection parameters (non-positive `near`, `fov_y_radians <= 0` or `>= π`, `aspect <= 0`) never panic.

### 2.2 Reversed-Z projection and single-site Vulkan Y-flip (`SPN-INV-053`)

Vulkan clip space uses `x_ndc, y_ndc ∈ [-1, 1]` with **`+Y` pointing down** in framebuffer space and depth **`z_ndc ∈ [0, 1]`**. Spawn applies the Y-flip **exclusively in `spawn-math`'s perspective projection constructor** (`VkViewport` keeps positive height `height > 0`, `minDepth = 0.0`, `maxDepth = 1.0`, and shaders perform no sign hacks):

For vertical field of view `fov_y` (radians), `aspect = width / height`, `g = 1 / tan(fov_y / 2)`, right-handed eye-space forward `z_e < 0` (`w_clip = -z_e > 0`):

- **Finite reversed-Z (`0 < near < far`):**
  - `x_clip = (g / aspect) * x_e`
  - `y_clip = -g * y_e` *(single-site Vulkan Y-flip: world/eye `+Y` maps to top of screen `y_ndc < 0`)*
  - `z_clip = (near / (far - near)) * z_e + (far * near / (far - near))`
  - `w_clip = -z_e`
  - At `z_e = -near`: `z_ndc = 1.0`. At `z_e = -far`: `z_ndc = 0.0`.
- **Infinite-far reversed-Z (`0 < near`, `far → ∞`):**
  - `z_clip = near` (`0 * z_e + near * 1`), `w_clip = -z_e`, so `z_ndc = near / (-z_e)` (`1.0` at `near`, approaching `0.0` monotonically at infinity).
- **Winding consistency:** A right-handed counter-clockwise triangle facing the camera (`v0 = (0, 0, -near)`, `v1 = (1, 0, -near)`, `v2 = (0, 1, -near)`) maps to `(0, 0) → (+x, 0) → (0, -y)` (`x, y > 0`) in Vulkan framebuffer coordinates (`+X` right, `+Y` down). Its raw 2D framebuffer determinant `(x1 - x0)(y2 - y0) - (x2 - x0)(y1 - y0) = -x * y` is negative, so Vulkan's polygon-orientation formula `a = -1/2 * sum(x_i * y_{i+1} - x_{i+1} * y_i) = +1/2 * x * y > 0` yields **positive** Vulkan signed area `a > 0`, which Vulkan's rasterizer classifies as **`VK_FRONT_FACE_COUNTER_CLOCKWISE`**. Meshes therefore keep standard glTF right-handed CCW winding and pipelines use `VK_FRONT_FACE_COUNTER_CLOCKWISE` with `VK_CULL_MODE_BACK_BIT`.

### 2.3 Deterministic subset vs. fast math (`SPN-INV-044`)

- `spawn-math::det` (module name illustrative) wraps IEEE-754 `f32::sqrt`/`f64::sqrt` and pure-Rust `libm` (`sinf`/`sin`, `cosf`/`cos`, `atan2f`/`atan2`, clamped `asinf`/`acosf`, `expf`/`exp`, `logf`/` log`) with no architecture-dependent FMA contraction. Simulation-tick camera orientation and movement integration call only this subset.
- Presentation-only code (projection setup from `fov_y`, frame interpolation helpers not shared with prediction) may use standard platform float operations.

## 3. Offline shader cooking and validation (**proposal**, pending Phase 0 acceptance of ADR-0014)

> **Proposal status:** Everything in §3 beyond [ADR-0004](../adr/0004-in-house-graphics-api.md)'s accepted rule (*"Slang, compiled offline to SPIR-V; runtime loads SPIR-V only"*) is a **proposal** contingent on Phase 0 owner acceptance of [ADR-0014](../adr/0014-shader-toolchain.md) and Slang (`Apache-2.0 WITH LLVM-exception`) license sign-off in [DEPENDENCIES.md](../DEPENDENCIES.md).

### 3.1 `spawn-cli` shader build workflow (`SPN-INV-083`, *proposal* `SPN-INV-084`)

- `tools/spawn-cli` adds a binary entrypoint (`spawn` / `spawn-cli`) using `clap` with a non-interactive subcommand (e.g. `shaders build --manifest <dir> --out <dir>`).
- **Pinned compiler check:** Repository configuration records the exact required `slangc` version string. Before compiling, `spawn-cli` invokes `slangc -v` (or `--version`), parses the version, and fails with a located `CLI-0001`+ error if `slangc` is missing or mismatched.
- **Compilation:** Invokes `slangc` targeting Vulkan 1.3 (`-target spirv -profile spirv_1_6` with explicit column-major matrix layout matching `spawn-math`) per shader entry point. No `build.rs` in the workspace invokes `slangc`, and generated `.spv` / cooked shader files live under git-ignored build output (never committed).
- **Reflection & cooked shader bundle:** `spawn-cli` inspects the emitted SPIR-V via approved `rspirv` (or a minimal in-house Slang reflection parser; using `serde_json` is **not** assumed unless approved and added to `DEPENDENCIES.md` in Phase 0) and writes a compact, versioned Spawn shader artifact containing:
  - Magic bytes, format `version`, stage mask, entry-point name, push-constant byte size/stages, descriptor bindings `(set, binding, kind, count, stages)`, vertex attribute `(location, format)` declarations, and 4-byte-aligned SPIR-V words.
- **CI and missing-`slangc` rule:** Because `slangc` is an external developer tool not required for headless Rust builds, `spawn-cli` abstracts process execution behind an injectable runner trait so unit/integration tests (`F3D-T06`) verify version checking, flag construction, artifact packaging, and error reporting in `./scripts/check.sh` without `slangc` installed.

### 3.2 Untrusted cooked-shader validation in `spawn-render` (`SPN-INV-034`)

- Shared binary header definitions and layout validation live in a pure, always-compiled module of `spawn-render` (`default-features = false`, `#![deny(clippy::indexing_slicing)]`), callable by both `spawn-cli` (when writing/verifying artifacts) and `spawn-render`'s pipeline builder.
- Before calling `vkCreateShaderModule` or `vkCreateGraphicsPipelines`, `spawn-render` validates:
  1. Payload byte cap (rejects multi-megabyte bogus allocations up front), header magic, format version, and SPIR-V magic (`0x07230203`) + 4-byte word alignment.
  2. Exact match (or compatible superset where documented) between reflected vertex inputs, push-constant byte range, and descriptor bindings versus the RHI pipeline descriptor. Mismatches return `RenderError` (`REND-0010`+), never a driver crash or validation-layer fault.

## 4. Vulkan 1.3 RHI extensions (`spawn-render`, ADR-0004)

All `ash` calls and `unsafe` blocks remain strictly inside `crates/spawn-render/src/vulkan/`. Public `spawn-render` types expose only generational Spawn handles (`ShaderHandle`, `MeshHandle`, `PipelineHandle` — conceptual names), plain-data vertex/draw descriptors (`bytemuck::Pod` + `Zeroable`), and camera-relative `f32` frame views.

### 4.1 Device selection updates (`src/selection.rs`)

Extend the pure capability check (`REND-0003` rejection detail) to require:
- Optimal-tiling `VK_FORMAT_D32_SFLOAT` support for `VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT`.
- `maxPushConstantsSize >= 128` bytes (Vulkan 1.3 minimum guarantee, checked explicitly) and required vertex input attribute/binding counts.

### 4.2 In-house GPU memory allocator and buffer uploads (`ADR-0004` decision 3)

- **Pure suballocator core (always compiled, headless-testable):** Manages fixed-size `VkDeviceMemory` blocks (e.g. 16–64 MiB device-local blocks, 4–16 MiB host-visible staging blocks; exact block sizes tunable and well under RX 460's ~2.8 GB usable budget, *budget tracking proposal per [ADR-0020](../adr/0020-memory-strategy.md)*) using power-of-two or linear free-list coalescing that honors `VkMemoryRequirements` alignment and `bufferImageGranularity`. Dedicated block fallback handles oversized allocations; exhaustion returns `RenderError` (`REND-0010`+).
- **Buffer roles:**
  - `DEVICE_LOCAL`: static/procedural vertex buffers, `u32`/`u16` index buffers, and the `D32_SFLOAT` depth image.
  - `HOST_VISIBLE | HOST_COHERENT`: staging ring/slab for CPU → GPU buffer uploads (`vkCmdCopyBuffer2`) and opt-in test image readback (`vkCmdCopyImageToBuffer2`).
- **Timeline-gated retirement:** Destroying a `MeshHandle`, `PipelineHandle`, staging slice, or old depth attachment queues its allocation on a retirement list tagged with the submission timeline value of the frame that last referenced it. Reclaim runs once `vkGetSemaphoreCounterValue` passes that timeline value; `Drop` waits for device idle, drains the retirement queue, destroys Vulkan objects, and asserts zero leaked suballocations.
- **Debug naming:** Buffers, depth images, shader modules, and pipelines receive `VK_EXT_debug_utils` names when validation/debug utils are active.

### 4.3 `D32_SFLOAT` reversed-Z depth attachment across resize and suspend

- Managed alongside swapchain images: created at the current swapchain `VkExtent2D` when `PresentationState::Ready`, never allocated when `0x0` (`Suspended`), and recreated atomically whenever the swapchain recreates on resize or `OUT_OF_DATE_KHR` / `SURFACE_LOST_KHR`.
- Per-frame `synchronization2` barrier transitions the depth image `UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL` (`srcStageMask = EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS`, `dstStageMask = EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS`, `dstAccessMask = DEPTH_STENCIL_ATTACHMENT_READ | DEPTH_STENCIL_ATTACHMENT_WRITE`), and `VkRenderingInfo` binds both color (`LOAD_OP_CLEAR`, `STORE_OP_STORE`) and depth (`LOAD_OP_CLEAR` to `0.0`, `STORE_OP_DONT_CARE` unless read back in tests).

### 4.4 Dynamic-rendering pipelines and draw submission

- Graphics pipelines use `VkPipelineRenderingCreateInfo` (no `VkRenderPass` or `VkFramebuffer`), `VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST`, `VK_FRONT_FACE_COUNTER_CLOCKWISE`, `VK_CULL_MODE_BACK_BIT`, depth test enabled with `VK_COMPARE_OP_GREATER_OR_EQUAL` and depth write enabled (`depthTestEnable = false` optional only for the Phase 3 standalone 2D clip-space triangle step if desired, though reversed-Z depth at `z = 0.5` works uniformly for both).
- **Per-draw transform contract (`SPN-INV-052`):** CPU computes `relative_model = Mat4::from_scale_rotation_translation(scale, rot, camera_relative_pos_f32)` and `view_proj_rot = proj * Mat4::from_quat(interp_cam_rot.conjugate())` (camera sits at `(0, 0, 0)` in camera-relative space!). Push constants (≤ 128 bytes) supply `mvp_camera_relative: Mat4`, `model_rot_scale` / normal basis, and linear base color so zero per-draw descriptor allocations occur on the hot path.

## 5. 60 Hz `f64` free-fly camera and `alpha` interpolation (`spawn-engine`)

### 5.1 Simulation state vs. presentation state (`SPN-INV-040..044`)

```text
Fixed 60 Hz tick (mutates simulation state only):
  InputCommand(tick N) ──► FreeCameraSim::step_tick(dt)
                              prev_pose = curr_pose
                              yaw, pitch (clamped ±(π/2 - ε)) += look_delta * rad_per_count
                              basis = det_quat_from_yaw_pitch(yaw, pitch)   [spawn-math::det]
                              curr_pose.world_pos_f64 += basis * move_axes * speed * dt

Frame render (read-only over FreeCameraSim; SPN-INV-043):
  alpha = clock.pending().as_nanos() as f64 / clock.config().tick_duration().as_nanos() as f64
  cam_pos_f64 = lerp_f64(sim.prev_pose.world_pos_f64, sim.curr_pose.world_pos_f64, alpha)
  cam_rot_f32 = slerp_shortest(sim.prev_pose.rot, sim.curr_pose.rot, alpha as f32)
  for mesh in scene:
      rel_pos_f32 = camera_relative_f32(mesh.world_pos_f64, cam_pos_f64)
      submit DrawItem { mesh: mesh.handle, rel_transform, color }
```

- **Zero-tick frames** (render rate > 60 Hz): `step_tick` runs 0 times; `alpha` advances smoothly across `[0.0, 1.0)` between `prev_pose` and `curr_pose`.
- **Multi-tick catch-up frames** (up to `max_ticks_per_update = 4`): `step_tick` runs for each due `Tick`, updating `prev_pose` on every tick so after the loop `prev_pose` holds tick `N - 1` and `curr_pose` holds tick `N`; excess ticks are dropped and reported as in Foundation.
- **Genre-free bindings in `spawn-engine`:** Digital axes for local translation (`MoveForwardBack`, `MoveRightLeft`, `MoveUpDown`), relative-pointer motion axes (`LookYaw`, `LookPitch` consuming `MotionAxis::PointerX`/`PointerY`), `ToggleCursorGrab` action (`CursorGrabMode::Locked` ↔ `None`), and `Exit` (`Escape`), all configured in `spawn-engine` as composition-root defaults — never hard-coded inside `spawn-input` (`SPN-INV-003`).

### 5.2 Procedural 3D test scene

Generated deterministically in Rust memory at startup (zero glTF/PNG files):
- Indexed unit cube / prism / octahedron meshes with per-vertex position (`[f32; 3]`), normal (`[f32; 3]`), and linear color (`[f32; 4]`) uploaded once via staging to `DEVICE_LOCAL` buffers.
- Placed at multiple depths and scales (e.g. overlapping foreground/midground markers at `-2 m`, `-5 m`, `-25 m`, `-200 m` plus a reference ground plane/grid) to make perspective foreshortening, backface culling, reversed-Z occlusion, and smooth `alpha`-interpolated camera motion immediately visible.

## 6. Headless seams and deferred topics

- **Headless seams:** `spawn-math` (100% pure), `spawn-cli` compiler runner seam, `spawn-render` cooked-shader validator + suballocator bookkeeping + depth/presentation state controller, and `spawn-engine`'s `FreeCameraSim` + `RuntimeBackend` trait all run in `cargo test --workspace` on Linux and Windows without `slangc`, Wayland, or Vulkan.
- **Deferred:** Render graph, bindless material tables, multi-threaded command recording, glTF/BCn cooking, frustum culling, pipeline disk caches, shader hot reload, and ECS integration belong to subsequent milestones.
