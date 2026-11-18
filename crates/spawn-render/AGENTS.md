# spawn-render

Local rules for `spawn-render` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Rendering infrastructure: GPU abstraction, resources, shaders, materials, pipelines, and render-frame execution. Generic; contains no gameplay rules.

## Responsibilities

- GPU device/queue/surface management and resource lifetime (in-house graphics API over Vulkan as a private module, accepted in [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md); `wgpu` is a design reference, not a dependency). The clear-frame path `EXISTS` behind the `vulkan` feature; everything else is `PLANNED`.
- Shaders, materials, pipelines, render graph/passes, camera/view abstraction, mesh/texture GPU resources.
- Extraction of renderable state from simulation into presentation state (simulation never touches GPU).
- Debug draw primitives if a feature requires them.

## Non-Responsibilities

- Windows/surfaces creation (`spawn-platform`), in-game UI layout/widgets (`spawn-ui` supplies draw data), editor viewport UI (`spawn-editor`).
- Asset identity/loading (`spawn-assets`), animation evaluation (`spawn-animation`), culling by physics queries (`spawn-physics`), gameplay effects (frameworks/Luau).
- Anything weapon-, team-, or match-aware.

## Architectural Boundaries

- May depend on `spawn-math`, `spawn-assets`, and a window handle (per [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md), accepted: through `raw-window-handle` traits, without depending on `spawn-platform`); must not depend on `spawn-physics`, `spawn-network`, `spawn-scripting`, or tools.
- `spawn-ui` and `spawn-editor` integrate through render-facing contracts, not by reaching into GPU internals.
- Server/headless builds must not require this crate.

## Public API Guidelines

- Hide the graphics backend; no Vulkan, `ash` or RHI-internal types in public APIs.
- The only third-party types accepted at the public boundary are the `raw-window-handle` traits/handles needed to create a surface from a window (SPN-INV-011); keep that surface minimal.
- Resources are referenced by handles; frame data is plain data.

## Internal Design Principles

- Separate render world/state from simulation state; keep GPU code behind thin layers so logic (sorting, culling, batching) is headless-testable.
- `unsafe` confined and justified.

## Performance and Safety

- Frame-time budgets, minimized state changes and allocations per frame, explicit GPU memory accounting; shader/asset inputs are untrusted — validate, never panic on device loss or bad shaders.

## Testing Requirements

- Headless unit tests for CPU-side logic; golden-image tests only if a headless adapter is available; benchmarks for frame extraction and culling.

## Governing Decisions

- **Accepted (binding):** [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (caller-driven presentation cadence, simulation/presentation separation), [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md) (graphics API), [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (errors and diagnostics), [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions), [ADR-0014](../../docs/adr/0014-shader-toolchain.md) (shader toolchain). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0020](../../docs/adr/0020-memory-strategy.md) (memory strategy).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `ash`, `raw-window-handle`, `bytemuck`, `renderdoc` (behind a feature), `gpu-allocator` (fallback only). Any other crate needs owner approval first.

## Current Implementation Status

`EXISTS` (foundation Phase 3: clear frame through Vulkan, Wayland surfaces only); shaders, pipelines, resources, memory allocator and everything else are `PLANNED`.

**Cargo features:** `default = []` (no dependencies; presentation policy, selection logic and public types compile and test without a GPU). `vulkan` adds `ash` 0.38 (`loaded`, `std`: `libvulkan.so.1` is loaded at runtime, so building needs no system packages) and `raw-window-handle`. Never depends on `spawn-platform` (`cargo tree -p spawn-render --features vulkan -e normal`).

| Item | Module | Availability | Notes |
| --- | --- | --- | --- |
| `SurfaceSize`, `ClearColor`, `RendererDescriptor`, `PresentModePreference`, `Validation` | `types` | always | Physical pixels; linear clear color, sanitized (non-finite → 0, clamped) and encoded per swapchain format |
| `FrameOutcome`, `PresentationState`, `RendererInfo`, `GpuKind`, `ValidationMessage`, `ValidationSeverity` | `types` | always | Explicit presentation outcomes and diagnostics |
| `RenderError`, `RenderErrorKind` | `error` | always | Located, coded, detail, flattened cause, optional backtrace (ADR-0009) |
| Presentation controller | `presentation` | always (crate-private) | Pure state machine: suspend, recreate, surface recovery, bounded retries, fatal stop |
| Device and swapchain choices | `selection` | always (crate-private) | Pure: ADR-0004 requirement check with per-GPU rejection reasons, ranking, format/present-mode/image-count/extent choices |
| `Renderer<'window>` | `renderer` | `vulkan` | Public API: `new(&window, size, &descriptor)`, `resize`, `render_clear`, `state`, `info`, `drain_validation_messages`, `validation_error_count`. Borrows the window for its lifetime; not `Send`/`Sync` |
| Vulkan RHI | `vulkan/` (`instance`, `surface`, `device`, `swapchain`, `frame`, `mod`) | `vulkan` | Only place with `ash` types and `unsafe`; every block has a `// SAFETY:` comment |

**Error codes:**

| Code | Kind | Meaning |
| --- | --- | --- |
| `REND-0001` | `VulkanUnavailable` | No loader, loader below 1.3, or no driver (ICD) providing `VK_KHR_surface` |
| `REND-0002` | `UnsupportedWindowSystem` | Window/display handle not Wayland, unavailable, or no driver provides its surface extension |
| `REND-0003` | `NoSuitableDevice` | No GPU meets ADR-0004; the detail lists every GPU with all its missing requirements |
| `REND-0004` | `VulkanCallFailed` | A Vulkan call failed (detail: call; cause: `VkResult`). Fatal after initialization |
| `REND-0005` | `DeviceLost` | `VK_ERROR_DEVICE_LOST`. Fatal; no device recovery |
| `REND-0006` | `SurfaceRecoveryFailed` | Bounded swapchain/surface recovery exhausted. Fatal |
| `REND-0007` | `GpuTimeout` | A frame slot did not finish within 5 s. Fatal |
| `REND-0008` | `ValidationUnavailable` | `Validation::Required` without the Khronos layer |
| `REND-0009` | `RendererFailed` | Call after a fatal error (detail names the original code) |

**Vulkan initialization and device selection:**

- Instance: Vulkan 1.3; extensions `VK_KHR_surface` + `VK_KHR_wayland_surface` only, plus `VK_EXT_debug_utils` and `VK_EXT_validation_features` (synchronization validation) when the validation layer is active. `Validation` default: `IfAvailable` with debug assertions, `Disabled` otherwise. Loader/instance setup and teardown (`Entry::load`, pre-instance enumeration, `vkCreateInstance`, `vkDestroyInstance`, `dlclose`) are serialized across threads (`LOADER_LOCK`, cold path only) so concurrent `Renderer` creation/drop cannot race inside non-thread-safe ICD global initialization.
- Device requirements (ADR-0004, checked per GPU from the actual feature bits): API ≥ 1.3; `VK_KHR_swapchain`; `dynamicRendering`, `synchronization2`, `timelineSemaphore`, `descriptorIndexing`, `runtimeDescriptorArray`, `descriptorBindingPartiallyBound`, `descriptorBindingVariableDescriptorCount`, `shaderSampledImageArrayNonUniformIndexing`, `descriptorBindingSampledImageUpdateAfterBind`, `descriptorBindingStorageBufferUpdateAfterBind`; one queue family with graphics and presentation; at least one surface format and present mode. All listed features are enabled at device creation. Device limits are not checked beyond the surface's image extent limits (no consumer yet).
- Ranking: discrete > integrated > virtual > CPU, then enumeration order; no vendor preference.
- One queue (graphics + present). No transfer or compute queues.

**Swapchain and synchronization:**

- Format: `B8G8R8A8_SRGB`/`R8G8B8A8_SRGB` in `SRGB_NONLINEAR` (GPU encodes), else 8-bit UNORM (CPU encodes the clear color), else the first format. Present mode: preference (`Fifo` default, `Mailbox`, `Immediate`) with FIFO fallback. Images: min + 1 within max. Usage: color attachment only. Composite alpha: opaque when supported.
- Two frames in flight. One timeline semaphore counts submissions; a frame slot is reused only after the timeline reaches its last value (5 s limit → `REND-0007`). Per slot: transient command pool, one command buffer, one binary acquire semaphore. Per swapchain image: one binary present semaphore.
- Commands: synchronization2 barrier `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL`, dynamic rendering with `LOAD_OP_CLEAR`, barrier to `PRESENT_SRC_KHR` (`dstStageMask = NONE`); `vkQueueSubmit2` waits the acquire semaphore at color-attachment output and signals both the present semaphore and the timeline at `ALL_COMMANDS` (ordering presentation after the layout transition).
- No memory allocation: swapchain images are the only images (the in-house allocator of ADR-0004 waits for its first consumer).

**Resize, suspension and recovery** (presentation controller):

- `resize(size)` only records the size (idempotent for unchanged sizes, even when the surface clamps or dictates a different swapchain extent). Zero → `Suspended`: no acquire, no present, `render_clear` returns `FrameOutcome::Suspended` (the caller should wait for events). A different non-zero size → swapchain recreated at the next frame, once, for the last size.
- A surface reporting a zero current extent also suspends until the next non-zero `resize`.
- `VK_ERROR_OUT_OF_DATE_KHR` (acquire or present) → frame `Skipped`, recreation next frame. `VK_SUBOPTIMAL_KHR` → frame presented, recreation next frame. `VK_TIMEOUT`/`VK_NOT_READY` (100 ms acquire timeout) → `Skipped`, no recovery. `VK_ERROR_SURFACE_LOST_KHR` → surface recreated from the stored handles, then the swapchain.
- Bounds: 3 consecutive failed recreations, or 16 consecutive recoveries without a presented frame → `REND-0006` (fatal). At most one recreation and one acquire per `render_clear` call, so no call spins.
- Device lost and other unexpected failures → fatal; later calls return `REND-0009`.
- Recreation waits for the device to be idle, then creates the new swapchain with `oldSwapchain` and destroys the old one (retired even on failure).
- The renderer has no clock and no simulation access: resize and recovery cannot reset simulation time (FND-INV-06).

**Ownership and shutdown order:** `Renderer` borrows the window, so the window (and its display connection) outlives every Vulkan object. Drop: wait for device idle → frames → swapchain → device → surface → validation messenger → instance. During construction each step owns what it created (RAII owners), so a failure releases exactly what exists, in reverse order.

**Tests:**

- Headless, always run (CI): 35 unit tests without the feature: errors (2), types (4), presentation controller (18, including FND-T06 and FND-T07), selection (11). With `vulkan`: +2 (validation sink bounds, `VkFailure` classification).
- GPU, opt-in (`cargo test -p spawn-render --features vulkan -- --ignored`, needs a Vulkan 1.3 GPU with `VK_EXT_headless_surface`): 120 presented frames; present-mode preferences (`Fifo`, `Mailbox`, `Immediate`); resize to zero (suspended, no acquire) and back to four sizes with matching swapchain extents; start suspended and create/drop three times; each asserts zero validation errors. All 4 pass on the reference hardware with `VK_LAYER_KHRONOS_validation` 1.4.363 and `VK_EXT_validation_features` synchronization validation active.

**Manual smoke test:** `examples/vulkan_clear.rs` in the root package (the composition root may depend on both `spawn-platform` and `spawn-render`; this crate must not); drains trailing validation messages on exit and returns a non-zero exit code on validation errors. Records: [VERIFICATION](../../docs/foundation/VERIFICATION.md#recorded-smoke-runs).

**Not implemented / deferred:**

- X11, Win32 and other surfaces (`REND-0002`); Windows is not claimed.
- Separate graphics/present queue families (rejected with a reason), async compute, transfer queues.
- Device-loss recovery (fatal by design for the foundation), HDR/wide-gamut color spaces, `VK_EXT_swapchain_maintenance1`.
- GPU memory allocator, shaders, pipelines, render graph, debug names beyond swapchain/frame objects.

## Agent Instructions

- Never add gameplay logic.
- The backend is decided ([ADR-0004](../../docs/adr/0004-in-house-graphics-api.md)): Vulkan 1.3 through `ash`, in-house RHI as a private module, in-house GPU memory allocator; `wgpu` is a design reference only. Do not reopen this choice without a new ADR.
- New dependencies beyond the approved list need justification under the root dependency rules and owner approval.
- Start from the minimal vertical slice ([foundation PLAN](../../docs/foundation/PLAN.md): clear frame first; triangle and mesh/camera are later milestones); no premature render graph.
- Keep `ash` types and `unsafe` inside `vulkan/`; put decisions (presentation, selection) in the pure modules and test them without a GPU.
- Never add a dependency on `spawn-platform`; smoke examples that need a window live in the composition root package.
- New error kinds get the next `REND-` code; codes are never reused or renumbered.
