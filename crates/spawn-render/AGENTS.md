# spawn-render

Local rules for `spawn-render` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Rendering infrastructure: GPU abstraction, resources, shaders, materials, pipelines, and render-frame execution. Generic; contains no gameplay rules.

## Responsibilities

- GPU device/queue/surface management, in-house block memory suballocation, and timeline-gated resource lifetime (in-house graphics API over Vulkan as a private module, accepted in [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md); `wgpu` is a design reference, not a dependency). Clear-frame presentation, GPU block suballocation, swapchain-matched `D32_SFLOAT` reversed-Z depth attachments, `dynamicRendering` graphics pipelines, unindexed triangle draws, static 3D vertex/index staging uploads, camera-relative indexed mesh draws, and offscreen RGBA8 readback `EXISTS` behind the `vulkan` feature; render graphs, bindless descriptor tables, and materials are `PLANNED`.
- Shaders, materials, pipelines, render graph/passes, camera/view abstraction, mesh/texture GPU resources.
- Extraction of renderable state from simulation into presentation state (simulation never touches GPU).
- Debug draw primitives if a feature requires them.

## Non-Responsibilities

- Windows/surfaces creation (`spawn-platform`), in-game UI layout/widgets (`spawn-ui` supplies draw data), editor viewport UI (`spawn-editor`).
- Asset identity/loading (`spawn-assets`), animation evaluation (`spawn-animation`), culling by physics queries (`spawn-physics`), gameplay effects (frameworks/Luau).
- Anything weapon-, team-, or match-aware.

## Architectural Boundaries

- Depends on `spawn-math` (`default-features = false`, `EXISTS`); may depend on `spawn-assets` (`PLANNED`) and a window handle (per [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md), accepted: through `raw-window-handle` traits, without depending on `spawn-platform`); must not depend on `spawn-physics`, `spawn-network`, `spawn-scripting`, or tools.
- `spawn-ui` and `spawn-editor` integrate through render-facing contracts, not by reaching into GPU internals.
- Server/headless builds must not require this crate.

## Public API Guidelines

- Hide the graphics backend and POD byte-casting internals; no Vulkan, `ash`, `bytemuck`, or RHI-internal types in public APIs (`SPN-INV-011`).
- The only third-party types accepted at the public boundary are the `raw-window-handle` traits/handles needed to create a surface from a window (`SPN-INV-011`); keep that surface minimal.
- Resources are referenced by generational handles (`ShaderHandle`, `PipelineHandle`, `OffscreenTargetHandle`, `MeshHandle`); frame data is plain data (`MeshVertex`, `MeshDescriptor`, `CameraRelativeView`, `MeshDraw`).

## Internal Design Principles

- Separate render world/state from simulation state; keep GPU code behind thin layers so logic (allocator bookkeeping, depth/presentation lifecycles, device selection, shader validation) is headless-testable.
- `#![forbid(unsafe_code)]` in every module outside `src/vulkan/`; `unsafe` confined strictly to `src/vulkan/` with `// SAFETY:` comments.

## Performance and Safety

- Frame-time budgets, minimized state changes and allocations per frame, explicit GPU memory accounting; shader/asset inputs and resource handles are untrusted — validate before Vulkan object creation, never panic on device loss, bad shaders, or stale handles.

## Testing Requirements

- Headless unit/property tests for CPU-side logic; opt-in GPU offscreen-readback and headless-surface tests when a Vulkan 1.3 adapter is available; benchmarks for frame extraction and culling.

## Governing Decisions

- **Accepted (binding):** [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (caller-driven presentation cadence, simulation/presentation separation), [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md) (graphics API and in-house GPU memory allocator), [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (errors and diagnostics), [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions, reversed-Z, single-site Y-flip), [ADR-0014](../../docs/adr/0014-shader-toolchain.md) (shader toolchain). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0020](../../docs/adr/0020-memory-strategy.md) (memory strategy).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `ash`, `raw-window-handle`, `bytemuck`, `renderdoc` (behind a feature), `gpu-allocator` (fallback only). Any other crate needs owner approval first.

## Current Implementation Status

`EXISTS` (foundation Phase 3: clear frame through Vulkan, Wayland + headless surfaces; Milestone 2 Phase 2: versioned cooked-shader binary contract and untrusted SPIR-V/reflection layout validator; Milestone 2 Phase 3: in-house GPU block suballocator + timeline retirement, swapchain-matched `D32_SFLOAT` reversed-Z depth attachment, generational handles, Vulkan 1.3 `dynamicRendering` graphics pipelines, colored triangle draws, and offscreen RGBA8 readback; Milestone 2 Phase 4: `spawn-math` integration, static 3D vertex/index staging uploads via `vkCmdCopyBuffer2` + `synchronization2`, `128`-byte camera-relative `MeshDraw` push constants, and reversed-Z `GREATER_OR_EQUAL` indexed mesh occlusion); render graph, bindless descriptor tables, and materials are `PLANNED`.

**Cargo features:** `default = []` (depends only on `spawn-math` with `default-features = false` → pure-Rust `libm`, zero window/GPU or C/FFI dependencies; `#![forbid(unsafe_code)]`; presentation policy, depth lifecycle policy, device selection, GPU block suballocator bookkeeping, generational handle pool, cooked-shader binary codec/validator, 3D mesh descriptor validation, camera-relative `128`-byte push-constant packing, and public types compile and test without a GPU; dev-dependencies `arbitrary`, `insta`, `proptest`). `vulkan` adds `ash` 0.38 (`loaded`, `std`: `libvulkan.so.1` is loaded at runtime, so building needs no system packages), `bytemuck` 1.25 (`derive`, confined strictly inside `src/vulkan/`), and `raw-window-handle`. Never depends on `spawn-platform` (`cargo tree -p spawn-render --features vulkan -e normal`).

| Item | Module | Availability | Notes |
| --- | --- | --- | --- |
| `SurfaceSize`, `ClearColor`, `RendererDescriptor`, `PresentModePreference`, `Validation` | `types` | always | Physical pixels; linear clear color, sanitized (non-finite → 0, clamped) and encoded per swapchain format |
| `FrameOutcome`, `PresentationState`, `RendererInfo`, `GpuKind`, `ValidationMessage`, `ValidationSeverity` | `types` | always | Explicit presentation outcomes and diagnostics |
| `PipelineTarget`, `FrontFace`, `CullMode`, `GraphicsPipelineDescriptor`, `OffscreenReadback` | `types` | always | Reversed-Z, CCW front-face, backface-culled `dynamicRendering` pipeline config (`new`, `new_mesh_3d`) and host RGBA8 readback |
| `GpuMemoryBudget`, `GpuAllocatorStats`, `DEFAULT_*_BYTES` | `allocator` | always | Configurable `DEVICE_LOCAL` / `HOST_VISIBLE` budgets, block sizes, and live/retired/peak accounting |
| `PureGpuAllocator` | `allocator` | always (crate-private) | `#![forbid(unsafe_code)]`, `#![deny(clippy::indexing_slicing)]`; fixed-size block suballocation, dedicated oversized fallback, `bufferImageGranularity` page separation, free-range coalescing, timeline retirement (`F3D-T08`) |
| `REVERSED_Z_CLEAR_DEPTH`, `DepthAttachmentController` | `depth` | `REVERSED_Z_CLEAR_DEPTH` public; controller crate-private (always) | `#![forbid(unsafe_code)]`; swapchain-matched `D32_SFLOAT` lifecycle (`0.0` clear, never allocates `0x0`, idempotent extent sync; `F3D-T09`) |
| `ShaderHandle`, `PipelineHandle`, `OffscreenTargetHandle`, `MeshHandle`, `GenerationalPool` | `handle` | handles public; pool crate-private (always) | `#![forbid(unsafe_code)]`, `#![deny(clippy::indexing_slicing)]`; stale/forged/foreign handles return `REND-0013` (`SPN-INV-035`) |
| `MeshVertex`, `MeshDescriptor`, `CameraRelativeView`, `MeshDraw`, `PackedMeshPushConstants`, `MESH_VERTEX_STRIDE`, `MESH_VERTEX_ATTRIBUTES`, `MESH_PUSH_CONSTANT_BYTES`, `MESH_PUSH_CONSTANT_RANGE`, `MAX_MESH_VERTICES`, `MAX_MESH_INDICES` | `mesh` | always | `#![forbid(unsafe_code)]`, `#![deny(clippy::indexing_slicing)]`; pre-upload descriptor validation (`REND-0014`), CPU `f64` `WorldPos` → `f32` camera-relative conversion (`SPN-INV-052`), and `128`-byte push-constant packing matching `shaders/mesh_3d.slang` |
| `CookedShader`, `ShaderStage`, `ShaderStageFlags`, `VertexFormat`, `VertexInputAttribute`, `DescriptorKind`, `DescriptorBinding`, `PushConstantRange`, `ShaderLayoutExpectation`, `PipelineLayoutDesc`, `validate_graphics_pipeline_shaders` | `shader` | always | `#![forbid(unsafe_code)]`, `#![deny(clippy::indexing_slicing)]`; versioned `SPNSHDR\0` binary format (`from_bytes` / `decode`), pre-allocation bounds/alignment checks, SPIR-V header/`OpEntryPoint` check, and pipeline layout compatibility validator (ADR-0014, `SPN-INV-034`) |
| `RenderError`, `RenderErrorKind` | `error` | always | Located, coded (`REND-0001..0014`), detail, flattened cause, optional backtrace (ADR-0009) |
| Presentation controller | `presentation` | always (crate-private) | Pure state machine: suspend, recreate, surface recovery, bounded retries, fatal stop |
| Device and swapchain choices | `selection` | always (crate-private) | Pure: ADR-0004 + `D32_SFLOAT` depth + push-constant/vertex-input limit check with per-GPU rejection reasons, ranking, format/present-mode/image-count/extent choices |
| `Renderer<'window>` | `renderer` | `vulkan` | Public API: `new`, `new_with_budget`, `new_headless`, `new_headless_with_budget`, `resize`, `load_shader`, `load_shader_named`, `destroy_shader`, `create_graphics_pipeline`, `destroy_pipeline`, `create_offscreen_target`, `destroy_offscreen_target`, `upload_mesh`, `destroy_mesh`, `render_clear`, `render_triangle`, `render_meshes`, `render_offscreen_triangle`, `render_offscreen_meshes`, `state`, `info`, `allocator_stats`, `wait_idle_and_reclaim`, `close`, `drain_validation_messages`, `validation_error_count`. Borrows the window for its lifetime; not `Send`/`Sync` |
| Vulkan RHI | `vulkan/` (`instance`, `surface`, `device`, `swapchain`, `memory`, `mesh`, `pipeline`, `frame`, `mod`) | `vulkan` | Only place with `ash` / `bytemuck` types and `unsafe`; every block has a `// SAFETY:` comment |

**Error codes:**

| Code | Kind | Meaning |
| --- | --- | --- |
| `REND-0001` | `VulkanUnavailable` | No loader, loader below 1.3, or no driver (ICD) providing `VK_KHR_surface` |
| `REND-0002` | `UnsupportedWindowSystem` | Window/display handle not Wayland, unavailable, or no driver provides its surface extension |
| `REND-0003` | `NoSuitableDevice` | No GPU meets ADR-0004 and Milestone 2 limits; the detail lists every GPU with all its missing requirements |
| `REND-0004` | `VulkanCallFailed` | A Vulkan call failed (detail: call; cause: `VkResult`). Fatal after initialization |
| `REND-0005` | `DeviceLost` | `VK_ERROR_DEVICE_LOST`. Fatal; no device recovery |
| `REND-0006` | `SurfaceRecoveryFailed` | Bounded swapchain/surface recovery exhausted. Fatal |
| `REND-0007` | `GpuTimeout` | A frame slot or timeline wait did not finish within 5 s. Fatal |
| `REND-0008` | `ValidationUnavailable` | `Validation::Required` without the Khronos layer |
| `REND-0009` | `RendererFailed` | Call after a fatal error or explicit `close` (detail names the original code) |
| `REND-0010` | `InvalidShaderArtifact` | Cooked shader payload is corrupt, truncated, unaligned, oversized, wrong version, or missing its SPIR-V `OpEntryPoint` |
| `REND-0011` | `ShaderLayoutMismatch` | Cooked shader stage, vertex attributes, push constants, or descriptor bindings mismatch the pipeline layout |
| `REND-0012` | `GpuMemoryExhausted` | GPU block suballocation, memory-type selection, or `vkAllocateMemory` failed or exceeded `GpuMemoryBudget` |
| `REND-0013` | `InvalidResourceHandle` | Generational resource handle (`ShaderHandle`, `PipelineHandle`, `OffscreenTargetHandle`, `MeshHandle`) is stale, destroyed, or forged |
| `REND-0014` | `InvalidResourceDescriptor` | Invalid pipeline, mesh, draw, or render-target descriptor (e.g. `0x0` offscreen extent, empty/oversized/non-finite/out-of-bounds mesh slices, stride/attribute mismatch, or target-format mismatch) |

**Vulkan initialization, memory, depth and pipeline execution:**

- Instance: Vulkan 1.3; extensions `VK_KHR_surface` + `VK_KHR_wayland_surface` (or `VK_EXT_headless_surface` for `Renderer::new_headless`), plus `VK_EXT_debug_utils` and `VK_EXT_validation_features` (synchronization validation) when the validation layer is active. Loader/instance setup and teardown are serialized across threads (`LOADER_LOCK`, cold path only).
- Device requirements (ADR-0004 + Milestone 2): API ≥ 1.3; `VK_KHR_swapchain`; `dynamicRendering`, `synchronization2`, `timelineSemaphore`, `descriptorIndexing`, `runtimeDescriptorArray`, `descriptorBindingPartiallyBound`, `descriptorBindingVariableDescriptorCount`, `shaderSampledImageArrayNonUniformIndexing`, `descriptorBindingSampledImageUpdateAfterBind`, `descriptorBindingStorageBufferUpdateAfterBind`; optimal-tiling `VK_FORMAT_D32_SFLOAT` with `DEPTH_STENCIL_ATTACHMENT`; `maxPushConstantsSize >= 128`, `maxVertexInputAttributes >= 3`, `maxVertexInputBindings >= 1`, `maxVertexInputBindingStride >= 40`; one queue family with graphics and presentation; at least one surface format and present mode.
- GPU memory suballocator (`PureGpuAllocator` + `VulkanMemoryAllocator`): suballocates `16 MiB` `DEVICE_LOCAL` blocks and persistently mapped `4 MiB` `HOST_VISIBLE | HOST_COHERENT` blocks (configurable via `GpuMemoryBudget`) with dedicated-block fallback for oversized requests, exact `bufferImageGranularity` page isolation between linear (`VkBuffer`) and optimal (`VkImage`) suballocations, deterministic adjacent free-range coalescing, and timeline-semaphore-gated retirement of both Vulkan child handles and suballocations before `vkFreeMemory`.
- Static 3D mesh staging & camera-relative draws: `upload_mesh` validates `MeshDescriptor` (`REND-0014`), writes vertices and `u32` indices into a suballocated `HOST_VISIBLE | HOST_COHERENT` staging buffer, submits `vkCmdCopyBuffer2` into `DEVICE_LOCAL` `VERTEX_BUFFER` and `INDEX_BUFFER` allocations followed by `synchronization2` `VkBufferMemoryBarrier2` (`COPY/TRANSFER_WRITE → VERTEX_ATTRIBUTE_INPUT/VERTEX_ATTRIBUTE_READ` and `INDEX_INPUT/INDEX_READ`), and retires the staging buffer on the timeline semaphore without stalling. `render_meshes` and `render_offscreen_meshes` push `128`-byte camera-relative `MeshPushConstants` (`0..64` `mvp_camera_relative`, `64..112` `normal_col0..2` with `normal_col0.w = shade_weight`, `112..128` `base_color`) and issue `vkCmdDrawIndexed`.
- Swapchain and `D32_SFLOAT` reversed-Z depth: swapchain image count is `max(minImageCount, 3)` within the surface maximum (triple buffering; each extra image adds one refresh of FIFO latency, measured in [first-3d VERIFICATION](../../docs/first-3d/VERIFICATION.md#performance-budget-and-measurements-reference-hardware)); swapchain requests `VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT` only (`SPN-INV-092`) paired with a `DEVICE_LOCAL` `VK_FORMAT_D32_SFLOAT` depth image/view matching the active swapchain `VkExtent2D` (never allocated at `0x0` when `Suspended`, retired/recreated idempotently across resize and surface recovery). Per-frame `synchronization2` barriers transition color (`UNDEFINED → COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR`) and depth (`UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL` across `EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS`, cleared to `0.0`).
- Dynamic-rendering pipelines & offscreen readback: `CookedShader::from_bytes` and `validate_graphics_pipeline_shaders` run before `vkCreateShaderModule` (`SPN-INV-034`). Pipelines use `VkPipelineRenderingCreateInfo` (zero `VkRenderPass`/`VkFramebuffer`), `TRIANGLE_LIST`, `VK_FRONT_FACE_COUNTER_CLOCKWISE`, `VK_CULL_MODE_BACK_BIT`, reversed-Z `VK_COMPARE_OP_GREATER_OR_EQUAL`, and positive-height `[0.0, 1.0]` `VkViewport`. Offscreen readback renders into a dedicated `DEVICE_LOCAL` `VK_FORMAT_R8G8B8A8_UNORM` (`COLOR_ATTACHMENT | TRANSFER_SRC`) + `D32_SFLOAT` target and copies via `vkCmdCopyImageToBuffer2` (`COPY → HOST_READ` barrier) into a suballocated `HOST_VISIBLE | HOST_COHERENT` buffer. Memory blocks, buffers, depth/offscreen images, views, shader modules, pipeline layouts, and pipelines carry `VK_EXT_debug_utils` names when active.

**Ownership and shutdown order:** `Renderer` borrows the window, so the window (and its display connection) outlives every Vulkan object. `teardown` / `Drop`: `vkDeviceWaitIdle` → offscreen targets → uploaded meshes → pipelines → shader modules → swapchain depth → frames → swapchain → allocator (`VkDeviceMemory` blocks, verified zero leaks) → device → surface → validation messenger → instance.

**Tests:**

- Headless, always run (CI): 48 unit/property tests (`errors`, `types`, `presentation` FND-T06/T07, `selection` + F3D-T09 rejections, `shader`, `handle`, `mesh` descriptor/50 km push-constant packing, `allocator` F3D-T08 + `proptest` + `insta`, `depth` F3D-T09 lifecycle) + 7 `shader_validation` tests (`F3D-T07`) + 1 `f3d_t13_render_hygiene` audit test without `vulkan`, plus standalone Gate G2 `cargo-fuzz` package `crates/spawn-render/fuzz` (target `decode_cooked_shader`, generated seed corpus `examples/seed_corpus.rs`), run with nightly + ASan by the CI job `fuzz` (owner-approved [ADR-0007](../../docs/adr/0007-toolchain-lints-and-ci.md) exception). With `vulkan`: +3 unit tests (`ValidationSink`, `VkFailure`, `bytemuck` POD byte equivalence).
- GPU, opt-in (`VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true cargo test -p spawn-render --features vulkan -- --ignored`): 4 Foundation headless-surface tests + `tests/f3d_t11_triangle_gpu.rs` (`F3D-T11`: offscreen CCW colored triangle `vkCmdCopyImageToBuffer2` RGBA8 readback, CW backface culling, reversed-Z `z = 0.0` vs clipped `z = -0.25`, negative shader/layout/stale-handle/OOM checks, `VK_EXT_headless_surface` swapchain + `D32_SFLOAT` depth draw/resize/`0x0` suspend) + `tests/f3d_t12_mesh_occlusion_gpu.rs` (`F3D-T12`: 2 tests covering synthesized SPIR-V 1.6 and live `slangc 2026.22`-cooked `shaders/mesh_3d.slang` for far-after-near `-2.0 m` vs `-5.0 m` reversed-Z occlusion across finite & infinite projections, `+Y` world-up orientation, backface culling + directional-shaded 3D cube, byte-exact `±50 km` `2 mm` camera-relative depth separation, swapchain multi-mesh resize/`0x0` suspend, and negative `REND-0012..0014` unwind checks), plus in-crate `upload_mesh` insert-failure rollback, draw-scratch reuse, and rejected-suballocation handle retirement tests. All 10 pass on the reference AMD Radeon RX 460 (`radv Mesa 26.2.5-arch1.1`) with `0` validation/sync errors; `Validation::Required` fails unless debug-utils and synchronization validation are both active (`RendererInfo::sync_validation_enabled`, asserted by `F3D-T11`/`F3D-T12`).

**Manual smoke tests:** `examples/vulkan_clear.rs` (Foundation Phase 3) and `examples/vulkan_triangle.rs` (Milestone 2 Phase 3) in the root package. Records: [Foundation VERIFICATION](../../docs/foundation/VERIFICATION.md#recorded-smoke-runs) and [First 3D VERIFICATION](../../docs/first-3d/VERIFICATION.md#recorded-smoke-runs).

**Not implemented / deferred:**

- X11, Win32 and other native window surfaces (`REND-0002`); Windows is not claimed.
- Separate graphics/present queue families, async compute, transfer queues, descriptor-set binding tables, render graph, HDR color spaces, device-loss recovery.

## Agent Instructions

- Never add gameplay logic.
- The backend is decided ([ADR-0004](../../docs/adr/0004-in-house-graphics-api.md)): Vulkan 1.3 through `ash`, in-house RHI as a private module, in-house GPU memory allocator; `wgpu` is a design reference only. Do not reopen this choice without a new ADR.
- New dependencies beyond the approved list need justification under the root dependency rules and owner approval.
- Keep `ash` types and `unsafe` strictly inside `vulkan/`; keep `#![forbid(unsafe_code)]` in every module outside `vulkan/` and test pure logic headlessly without a GPU.
- Never add a dependency on `spawn-platform`; smoke examples that need a window live in the composition root package.
- New error kinds get the next `REND-` code; codes are never reused or renumbered.
