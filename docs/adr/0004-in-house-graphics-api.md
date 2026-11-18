# ADR-0004 — In-House Graphics API over Vulkan

**Status:** Accepted (foundation scope; revisited when a later milestone or a measured limitation requires it).  
**Scope:** The GPU abstraction inside `spawn-render`: device, queues, memory, resources, synchronization, shaders, swapchain/presentation. Foundation needs only a clear frame.  
**Related:** [ADR-0001](0001-in-house-platform-layer.md) (platform layer), [ADR-0003](0003-fixed-step-loop-and-threading.md) (presentation cadence, render thread).

## Context

The validation project (large-scale competitive FPS) will need control that general-purpose graphics abstractions limit:

- multiple queues (async compute, dedicated transfer);
- explicit synchronization and barrier placement;
- explicit memory management, including aliasing of transient render-graph resources;
- full bindless resource access and GPU-driven rendering;
- direct access to vendor extensions (e.g. low-latency modes) and, later, ray tracing.

Supported targets are Windows and Linux only ([ADR-0001](0001-in-house-platform-layer.md)). **Vulkan covers both**, so a single explicit backend serves every target; Metal is not needed. The owner prefers full control and accepts the extra implementation time.

## Decision (owner direction)

Spawn builds its own graphics API (a rendering hardware interface, RHI) inside `spawn-render`:

1. **Backend: Vulkan.** D3D12 is not implemented now; adding it later requires a new ADR.
2. **No third-party graphics abstraction** (`wgpu`, SDL3 GPU, bgfx, etc.) is a dependency. They are used as **design references** only (see [References](#references)).
3. **Internal, not public.** The RHI is an internal layer of `spawn-render`. Other crates see Spawn-defined handles and descriptors from `spawn-render`'s public API, never Vulkan types and never RHI internals.
4. **Explicit by design.** The RHI exposes queues, synchronization and memory explicitly to the rest of `spawn-render`. Convenience (automatic barriers, transient-resource aliasing) is built on top later by a render graph, not hidden inside the RHI.
5. **Window handle** through the `raw-window-handle` traits implemented by `spawn-platform`'s window ([ADR-0001](0001-in-house-platform-layer.md)), so `spawn-render` does not depend on `spawn-platform`. This is the only third-party type allowed at the public boundary of either crate, limited to the traits/handles needed to create a surface; Vulkan and `ash` types stay private (SPN-INV-011 in [INVARIANTS](../architecture/INVARIANTS.md)).

## Detailed decisions (owner-confirmed)

1. **Vulkan bindings: `ash`** (raw Vulkan declarations and loader, no abstraction logic), consistent with the binding policy of [ADR-0001](0001-in-house-platform-layer.md). Hand-written declarations add risk without adding control.
2. **Vulkan baseline: 1.3**, with dynamic rendering, synchronization2, timeline semaphores and descriptor indexing (bindless) as required features. Older GPUs/drivers without these are not supported; the minimum hardware is recorded once verified. **Verified on the reference hardware:** AMD Radeon RX 460 (Polaris) with RADV reports Vulkan 1.4. Polaris has no hardware ray tracing or mesh shaders; those features need other hardware to develop and test.
3. **GPU memory allocator: in-house**, starting with a simple block allocator and growing with measured needs. Vulkan Memory Allocator and `gpu-allocator` are references, not dependencies.
4. **Shader toolchain: Slang, compiled offline to SPIR-V.** Writing a shader compiler is out of scope. The runtime loads SPIR-V only. Slang can also emit DXIL if a D3D12 backend is ever added. How the compiler is installed and invoked: [ADR-0014](0014-shader-toolchain.md).
5. **Location: private module of `spawn-render`.** A separate crate is created only if a second consumer (e.g. `spawn-editor` tooling) needs the RHI directly, which requires its own decision.

## References

Used to study API design, correctness rules and trade-offs. **No code is copied without checking its license**; code under a non-open-source license is never copied.

| Reference | What to learn from it | License note |
| --- | --- | --- |
| `wgpu` / `wgpu-hal` | Primary reference: safe resource lifetime tracking, validation model, automatic barrier tracking, swapchain/surface recovery, layering between a safe API and an unsafe HAL | MIT / Apache-2.0 |
| WebGPU specification | Validation rules and a portable, safe resource model | W3C specification |
| Vulkan specification, Khronos Vulkan-Samples, Vulkan Guide | Correct usage of synchronization, memory, swapchain, extensions | Specification / Apache-2.0 samples |
| Sascha Willems' Vulkan examples | Focused, minimal examples of individual features | MIT |
| NVRHI (NVIDIA) | Thin RHI over Vulkan/D3D12 with optional automatic state tracking | MIT |
| Granite (Themaister) | Production-quality Vulkan backend design, render graph | MIT |
| The Forge, Diligent Engine | Cross-API RHI layering and resource binding models | Apache-2.0 |
| Vulkan Memory Allocator, `gpu-allocator` | Allocation strategies, heap selection, defragmentation | MIT / MIT-Apache |
| Frostbite FrameGraph (GDC 2017 talk) | Render graph, automatic barriers, transient-resource aliasing | Talk (no code) |
| Unreal Engine RHI and RDG | Layering of RHI, render thread and render dependency graph | **Source under the Unreal EULA: study documentation and talks only; never copy code** |
| bgfx, SDL3 GPU API | Minimal, simple API surfaces | BSD-2 / zlib |

## Alternatives

| Option | Strengths | Weaknesses |
| --- | --- | --- |
| **In-house RHI over Vulkan** (chosen direction) | Full control: multiple queues, async compute, explicit sync and memory, aliasing, bindless, vendor extensions; one backend covers both targets | Highest effort; synchronization and memory bugs are Spawn's responsibility; driver differences must be handled directly |
| **`wgpu`** | Fastest path to a frame; validation, automatic sync and memory; portable | Single queue (no async compute); conservative automatic barriers; no memory aliasing; limited bindless; experimental ray tracing; frequent breaking releases |
| **`wgpu-hal`** directly | Lower level than `wgpu`, no validation overhead | Still shaped by WebGPU; unsafe without the guarantees of `wgpu`; tied to upstream changes |
| **In-house RHI over Vulkan + D3D12** | Native API on Windows | Doubles backend work; not needed while Vulkan covers both targets |
| **SDL3 GPU API**, **bgfx** | Simple, mature | C dependencies; deliberately limited feature sets |

## Consequences if accepted

- The foundation's Phase 3 (renderer) grows: instance, device, queue selection, swapchain, command recording and frame synchronization must be written before the first clear frame.
- `spawn-render` contains a large amount of `unsafe` code; every block follows the root `// SAFETY:` rule and Vulkan calls are confined to the backend module.
- Vulkan validation layers are enabled in development builds and their errors fail tests where a GPU is available.
- Windows relies on Vulkan drivers; this is acceptable for current desktop GPUs and is verified in the Windows smoke test.

## Risks

- **Synchronization bugs** (missing or wrong barriers) cause corruption, flickering or GPU hangs that may appear only on some vendors. Mitigation: validation layers including synchronization validation, conservative barriers first, optimization only with measurements.
- **Device lost / GPU hang.** Driver timeouts (TDR on Windows) terminate the device. Mitigation: a defined device-lost path that reports and shuts down cleanly in the foundation; recovery is a later decision.
- **Driver and vendor differences** (NVIDIA, AMD, Intel; proprietary vs. Mesa on Linux). Mitigation: a tested-GPU matrix recorded per smoke test; no support claims for untested vendors.
- **Memory bugs** (leaks, use-after-free of GPU resources still in flight). Mitigation: deferred destruction tied to frame/timeline completion, leak checks at shutdown.
- **Scope growth.** An RHI attracts speculative features. Mitigation: each RHI feature names the rendering feature that needs it (root rule: no infrastructure without a concrete requirement).
- **Debugging tool dependence.** Development relies on RenderDoc, NVIDIA Nsight and Radeon GPU Profiler; RHI objects carry debug names from the start.
- **Time to first frame.** Significantly longer than with `wgpu`; accepted by the owner.

## Acceptance conditions

Explicit owner acceptance. The first clear frame on Linux, with validation layers reporting no errors, is implementation evidence recorded afterwards, not a precondition.
