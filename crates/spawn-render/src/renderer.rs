//! Public renderer, backed by the private Vulkan RHI.

use std::marker::PhantomData;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::error::{RenderError, RenderErrorKind};
use crate::types::{
    ClearColor, FrameOutcome, PresentationState, RendererDescriptor, RendererInfo, SurfaceSize,
    ValidationMessage,
};
use crate::vulkan::{SurfaceSource, VulkanRenderer};

/// Presents frames to one window through Vulkan 1.3.
///
/// The renderer **borrows the window** (`'window`) for its whole lifetime, so
/// the native window and display stay valid while the Vulkan surface exists;
/// dropping the renderer waits for the GPU and releases every GPU resource
/// before the borrow ends.
///
/// Rendering is driven explicitly by the caller (ADR-0003): it reports the
/// drawable size with [`Renderer::resize`] and calls
/// [`Renderer::render_clear`] once per presented frame. The renderer never
/// touches simulation state or time.
///
/// **Thread affinity:** not `Send`/`Sync`; use it on the thread that created
/// it.
///
/// **Supported window systems:** Wayland only. Other `raw-window-handle`
/// kinds return `REND-0002`.
pub struct Renderer<'window> {
    backend: VulkanRenderer,
    _window: PhantomData<&'window ()>,
}

impl<'window> Renderer<'window> {
    /// Initializes Vulkan for `window`, whose drawable surface currently has
    /// `size` physical pixels, and creates the first swapchain (or starts
    /// suspended when `size` is zero).
    ///
    /// # Errors
    ///
    /// - `REND-0001` [`VulkanUnavailable`](RenderErrorKind::VulkanUnavailable):
    ///   no Vulkan loader, or it lacks Vulkan 1.3.
    /// - `REND-0002` [`UnsupportedWindowSystem`](RenderErrorKind::UnsupportedWindowSystem):
    ///   the window is not a Wayland window, its handles are unavailable, or
    ///   the loader lacks the surface extension.
    /// - `REND-0003` [`NoSuitableDevice`](RenderErrorKind::NoSuitableDevice):
    ///   no GPU meets ADR-0004; the detail lists each GPU and why.
    /// - `REND-0008` [`ValidationUnavailable`](RenderErrorKind::ValidationUnavailable):
    ///   validation is [`Required`](crate::Validation::Required) but the layer
    ///   is not installed.
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
        )?;
        Ok(Self {
            backend,
            _window: PhantomData,
        })
    }

    /// Reports the drawable size in physical pixels. Zero suspends
    /// presentation; a new non-zero size recreates the swapchain at the next
    /// frame. Cheap and idempotent: call it for every resize event.
    pub fn resize(&mut self, size: SurfaceSize) {
        self.backend.resize(size);
    }

    /// Clears the next swapchain image to `color` and queues it for
    /// presentation.
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

    /// Current presentation state.
    pub fn state(&self) -> PresentationState {
        self.backend.state()
    }

    /// GPU, driver and validation information, for diagnostics.
    pub fn info(&self) -> &RendererInfo {
        self.backend.info()
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

impl std::fmt::Debug for Renderer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("gpu", &self.info().gpu_name)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}
