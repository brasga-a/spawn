//! Public renderer, backed by the private Vulkan RHI.

#![forbid(unsafe_code)]

use std::marker::PhantomData;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::allocator::{GpuAllocatorStats, GpuMemoryBudget};
use crate::error::{RenderError, RenderErrorKind};
use crate::handle::{MeshHandle, OffscreenTargetHandle, PipelineHandle, ShaderHandle};
use crate::mesh::{CameraRelativeView, MeshDescriptor, MeshDraw};
use crate::types::{
    ClearColor, FrameOutcome, GraphicsPipelineDescriptor, OffscreenReadback, PresentationState,
    RendererDescriptor, RendererInfo, SurfaceSize, ValidationMessage,
};
use crate::vulkan::{SurfaceSource, VulkanRenderer};

/// Presents frames to one window (or headless surface) through Vulkan 1.3.
///
/// The renderer **borrows the window** (`'window`) for its whole lifetime, so
/// the native window and display stay valid while the Vulkan surface exists;
/// dropping the renderer waits for the GPU and releases every GPU resource
/// before the borrow ends.
///
/// Rendering is driven explicitly by the caller (ADR-0003): it reports the
/// drawable size with [`Renderer::resize`] and calls
/// [`Renderer::render_clear`], [`Renderer::render_triangle`], or
/// [`Renderer::render_meshes`] once per presented frame. The renderer never
/// touches simulation state or time.
///
/// **Thread affinity:** not `Send`/`Sync`; use it on the thread that created
/// it.
///
/// **Supported window systems:** Wayland only (plus `VK_EXT_headless_surface`
/// through [`Renderer::new_headless`]). Other `raw-window-handle` kinds return
/// `REND-0002`.
pub struct Renderer<'window> {
    backend: VulkanRenderer,
    _window: PhantomData<&'window ()>,
}

impl<'window> Renderer<'window> {
    /// Initializes Vulkan for `window`, whose drawable surface currently has
    /// `size` physical pixels, using [`GpuMemoryBudget::default`] and creating
    /// the first swapchain + `D32_SFLOAT` reversed-Z depth attachment (or
    /// starting suspended when `size` is zero).
    ///
    /// # Errors
    ///
    /// - `REND-0001` [`VulkanUnavailable`](RenderErrorKind::VulkanUnavailable):
    ///   no Vulkan loader, or it lacks Vulkan 1.3.
    /// - `REND-0002` [`UnsupportedWindowSystem`](RenderErrorKind::UnsupportedWindowSystem):
    ///   the window is not a Wayland window, its handles are unavailable, or
    ///   the loader lacks the surface extension.
    /// - `REND-0003` [`NoSuitableDevice`](RenderErrorKind::NoSuitableDevice):
    ///   no GPU meets ADR-0004 and Milestone 2 limits; the detail lists each
    ///   GPU and why.
    /// - `REND-0008` [`ValidationUnavailable`](RenderErrorKind::ValidationUnavailable):
    ///   validation is [`Required`](crate::Validation::Required) but the layer
    ///   is not installed.
    /// - `REND-0012` [`GpuMemoryExhausted`](RenderErrorKind::GpuMemoryExhausted):
    ///   depth allocation exceeded budget or device memory.
    /// - `REND-0004`/`REND-0005`: a Vulkan call failed or the device was lost.
    ///
    /// Every resource created before a failure is released before returning.
    pub fn new<W>(
        window: &'window W,
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
    ) -> Result<Self, RenderError>
    where
        W: HasWindowHandle + HasDisplayHandle + ?Sized,
    {
        Self::new_with_budget(window, size, descriptor, GpuMemoryBudget::default())
    }

    /// Initializes Vulkan for `window` with an explicit [`GpuMemoryBudget`].
    pub fn new_with_budget<W>(
        window: &'window W,
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
        budget: GpuMemoryBudget,
    ) -> Result<Self, RenderError>
    where
        W: HasWindowHandle + HasDisplayHandle + ?Sized,
    {
        let handle_error = |error: raw_window_handle::HandleError| {
            RenderError::new(RenderErrorKind::UnsupportedWindowSystem)
                .with_detail(format!("window handle unavailable: {error:?}"))
        };
        let display = window.display_handle().map_err(handle_error)?.as_raw();
        let native = window.window_handle().map_err(handle_error)?.as_raw();
        let backend = VulkanRenderer::new(
            SurfaceSource::Window {
                display,
                window: native,
            },
            size,
            descriptor,
            budget,
        )?;
        Ok(Self {
            backend,
            _window: PhantomData,
        })
    }

    /// Reports the drawable size in physical pixels. Zero suspends
    /// presentation; a new non-zero size recreates the swapchain and its
    /// `D32_SFLOAT` depth attachment at the next frame. Cheap and idempotent:
    /// call it for every resize event.
    pub fn resize(&mut self, size: SurfaceSize) {
        self.backend.resize(size);
    }

    /// Validates untrusted `.spnshdr` bytes via [`crate::CookedShader::from_bytes`]
    /// (`SPN-INV-034`) and returns a generational [`ShaderHandle`].
    #[track_caller]
    pub fn load_shader(&mut self, cooked_bytes: &[u8]) -> Result<ShaderHandle, RenderError> {
        self.backend.load_shader(None, cooked_bytes)
    }

    /// Validates untrusted `.spnshdr` bytes with a diagnostic `label`
    /// (`SPN-INV-081`) and returns a generational [`ShaderHandle`].
    #[track_caller]
    pub fn load_shader_named(
        &mut self,
        label: &str,
        cooked_bytes: &[u8],
    ) -> Result<ShaderHandle, RenderError> {
        self.backend.load_shader(Some(label), cooked_bytes)
    }

    /// Releases `handle`, invalidating its generation (`REND-0013` on stale or
    /// forged handles).
    #[track_caller]
    pub fn destroy_shader(&mut self, handle: ShaderHandle) -> Result<(), RenderError> {
        self.backend.destroy_shader(handle)
    }

    /// Validates vertex + fragment shader compatibility against `descriptor`
    /// before creating `VkShaderModule`s (`SPN-INV-034`) and compiles a Vulkan
    /// 1.3 `dynamicRendering` graphics pipeline.
    #[track_caller]
    pub fn create_graphics_pipeline(
        &mut self,
        descriptor: &GraphicsPipelineDescriptor<'_>,
    ) -> Result<PipelineHandle, RenderError> {
        self.backend.create_graphics_pipeline(descriptor)
    }

    /// Releases `handle` (`REND-0013` on stale or forged handles), retiring the
    /// underlying GPU pipeline once in-flight frames complete.
    #[track_caller]
    pub fn destroy_pipeline(&mut self, handle: PipelineHandle) -> Result<(), RenderError> {
        self.backend.destroy_pipeline(handle)
    }

    /// Allocates an offscreen `R8G8B8A8_UNORM` + `D32_SFLOAT` reversed-Z depth
    /// render target of `size` physical pixels for deterministic offscreen
    /// rendering and host readback.
    #[track_caller]
    pub fn create_offscreen_target(
        &mut self,
        size: SurfaceSize,
    ) -> Result<OffscreenTargetHandle, RenderError> {
        self.backend.create_offscreen_target(size)
    }

    /// Releases an offscreen render target (`REND-0013` on stale or forged
    /// handles), retiring its color and depth suballocations on the timeline
    /// semaphore.
    #[track_caller]
    pub fn destroy_offscreen_target(
        &mut self,
        handle: OffscreenTargetHandle,
    ) -> Result<(), RenderError> {
        self.backend.destroy_offscreen_target(handle)
    }

    /// Validates `descriptor` (`REND-0014` on empty/oversized slices, non-finite
    /// vertex attributes, or out-of-bounds triangle indices; `SPN-INV-034`),
    /// stages vertices and `u32` indices into `DEVICE_LOCAL` buffers via
    /// `vkCmdCopyBuffer2` + `synchronization2`, retires the staging buffer on
    /// the timeline semaphore, and returns a generational [`MeshHandle`].
    #[track_caller]
    pub fn upload_mesh(
        &mut self,
        descriptor: &MeshDescriptor<'_>,
    ) -> Result<MeshHandle, RenderError> {
        self.backend.upload_mesh(descriptor)
    }

    /// Releases an uploaded 3D mesh (`REND-0013` on stale or forged handles),
    /// retiring its `DEVICE_LOCAL` vertex and index suballocations on the
    /// timeline semaphore.
    #[track_caller]
    pub fn destroy_mesh(&mut self, handle: MeshHandle) -> Result<(), RenderError> {
        self.backend.destroy_mesh(handle)
    }

    /// Clears the next swapchain image to `color` (and its `D32_SFLOAT` depth
    /// attachment to `0.0`) and queues it for presentation.
    ///
    /// Blocks at most until a swapchain image is available (bounded by the
    /// present mode and a 100 ms acquire timeout) and until the frame slot
    /// from two frames ago finished on the GPU.
    ///
    /// # Errors
    ///
    /// Fatal errors stop the renderer; later calls return `REND-0009`:
    ///
    /// - `REND-0005` [`DeviceLost`](RenderErrorKind::DeviceLost): device
    ///   recovery is not supported; drop the renderer.
    /// - `REND-0006` [`SurfaceRecoveryFailed`](RenderErrorKind::SurfaceRecoveryFailed):
    ///   bounded swapchain/surface recovery was exhausted.
    /// - `REND-0007` [`GpuTimeout`](RenderErrorKind::GpuTimeout): a frame did
    ///   not finish within 5 seconds.
    /// - `REND-0004` [`VulkanCallFailed`](RenderErrorKind::VulkanCallFailed).
    ///
    /// Outdated swapchains, lost surfaces, suboptimal images and zero sizes
    /// are **not** errors; see [`FrameOutcome`].
    pub fn render_clear(&mut self, color: ClearColor) -> Result<FrameOutcome, RenderError> {
        self.backend.render_clear(color)
    }

    /// Clears the next swapchain image to `clear_color` and its `D32_SFLOAT`
    /// depth attachment to `0.0`, draws `vertex_count` unindexed vertices with
    /// `pipeline`, and queues the image for presentation.
    #[track_caller]
    pub fn render_triangle(
        &mut self,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        vertex_count: u32,
    ) -> Result<FrameOutcome, RenderError> {
        self.backend
            .render_triangle(pipeline, clear_color, vertex_count)
    }

    /// Clears the next swapchain image to `clear_color` and its `D32_SFLOAT`
    /// reversed-Z depth attachment to `0.0`, draws `draws` as camera-relative
    /// indexed 3D meshes (`SPN-INV-052`) with `pipeline`, and queues the image
    /// for presentation.
    #[track_caller]
    pub fn render_meshes(
        &mut self,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        view: &CameraRelativeView,
        draws: &[MeshDraw],
    ) -> Result<FrameOutcome, RenderError> {
        self.backend
            .render_meshes(pipeline, clear_color, view, draws)
    }

    /// Clears `target` (`R8G8B8A8_UNORM` + `D32_SFLOAT` depth `0.0`), draws
    /// `vertex_count` unindexed vertices with `pipeline`, and copies the
    /// resulting linear RGBA8 image back to host memory.
    #[track_caller]
    pub fn render_offscreen_triangle(
        &mut self,
        target: OffscreenTargetHandle,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        vertex_count: u32,
    ) -> Result<OffscreenReadback, RenderError> {
        self.backend
            .render_offscreen_triangle(target, pipeline, clear_color, vertex_count)
    }

    /// Clears `target` (`R8G8B8A8_UNORM` + `D32_SFLOAT` reversed-Z depth `0.0`),
    /// draws `draws` as camera-relative indexed 3D meshes (`SPN-INV-052`) with
    /// `pipeline`, and copies the resulting linear RGBA8 image back to host
    /// memory.
    #[track_caller]
    pub fn render_offscreen_meshes(
        &mut self,
        target: OffscreenTargetHandle,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        view: &CameraRelativeView,
        draws: &[MeshDraw],
    ) -> Result<OffscreenReadback, RenderError> {
        self.backend
            .render_offscreen_meshes(target, pipeline, clear_color, view, draws)
    }

    /// Current presentation state.
    pub fn state(&self) -> PresentationState {
        self.backend.state()
    }

    /// GPU, driver and validation information, for diagnostics.
    pub fn info(&self) -> &RendererInfo {
        self.backend.info()
    }

    /// Current GPU block suballocator and retirement statistics.
    pub fn allocator_stats(&self) -> GpuAllocatorStats {
        self.backend.allocator_stats()
    }

    /// Waits until the GPU is idle and reclaims all timeline-retired GPU
    /// resources, returning the updated [`GpuAllocatorStats`].
    pub fn wait_idle_and_reclaim(&mut self) -> Result<GpuAllocatorStats, RenderError> {
        self.backend.wait_idle_and_reclaim()
    }

    /// Explicitly waits for GPU completion, releases all resources and memory
    /// blocks in reverse ownership order, and returns the post-shutdown
    /// [`GpuAllocatorStats`].
    pub fn close(self) -> Result<GpuAllocatorStats, RenderError> {
        self.backend.close()
    }

    /// Moves validation-layer messages received so far to `out` and returns
    /// how many older messages were dropped because the queue (256 messages)
    /// was full. Always empty without validation.
    pub fn drain_validation_messages(&mut self, out: &mut Vec<ValidationMessage>) -> u64 {
        self.backend.drain_validation_messages(out)
    }

    /// Error-severity validation messages received since creation.
    pub fn validation_error_count(&self) -> u64 {
        self.backend.validation_error_count()
    }
}

impl Renderer<'static> {
    /// Initializes Vulkan over a windowless `VK_EXT_headless_surface` surface
    /// at `size` physical pixels using [`GpuMemoryBudget::default`].
    #[track_caller]
    pub fn new_headless(
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
    ) -> Result<Self, RenderError> {
        Self::new_headless_with_budget(size, descriptor, GpuMemoryBudget::default())
    }

    /// Initializes Vulkan over a windowless `VK_EXT_headless_surface` surface
    /// with an explicit [`GpuMemoryBudget`].
    #[track_caller]
    pub fn new_headless_with_budget(
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
        budget: GpuMemoryBudget,
    ) -> Result<Self, RenderError> {
        let backend = VulkanRenderer::new(SurfaceSource::Headless, size, descriptor, budget)?;
        Ok(Self {
            backend,
            _window: PhantomData,
        })
    }
}

impl std::fmt::Debug for Renderer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("gpu", &self.info().gpu_name)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}
