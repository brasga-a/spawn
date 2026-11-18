//! `VkSurfaceKHR` creation from native window handles.

use ash::{khr, vk};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::error::{RenderError, RenderErrorKind};
use crate::vulkan::instance::{InstanceOwner, SurfaceExtension};
use crate::vulkan::vk_error;

/// Where the renderer's surface comes from. The raw handles are copied so the
/// surface can be recreated after `VK_ERROR_SURFACE_LOST_KHR`; the public
/// `Renderer` borrows the window for its whole lifetime, so they stay valid.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SurfaceSource {
    Window {
        display: RawDisplayHandle,
        window: RawWindowHandle,
    },
    /// A `VK_EXT_headless_surface` surface for headless rendering and GPU tests without a window.
    Headless,
}

impl SurfaceSource {
    /// The instance extension needed, or why the handles are unsupported.
    pub(crate) fn extension(&self) -> Result<SurfaceExtension, RenderError> {
        match self {
            Self::Window {
                display: RawDisplayHandle::Wayland(_),
                window: RawWindowHandle::Wayland(_),
            } => Ok(SurfaceExtension::Wayland),
            Self::Window { display, window } => Err(RenderError::new(
                RenderErrorKind::UnsupportedWindowSystem,
            )
            .with_detail(format!(
                "only Wayland windows are supported (display: {}, window: {})",
                display_kind(display),
                window_kind(window)
            ))),
            Self::Headless => Ok(SurfaceExtension::Headless),
        }
    }
}

fn display_kind(handle: &RawDisplayHandle) -> &'static str {
    match handle {
        RawDisplayHandle::Wayland(_) => "Wayland",
        RawDisplayHandle::Xlib(_) => "Xlib",
        RawDisplayHandle::Xcb(_) => "XCB",
        RawDisplayHandle::Windows(_) => "Windows",
        _ => "other",
    }
}

fn window_kind(handle: &RawWindowHandle) -> &'static str {
    match handle {
        RawWindowHandle::Wayland(_) => "Wayland",
        RawWindowHandle::Xlib(_) => "Xlib",
        RawWindowHandle::Xcb(_) => "XCB",
        RawWindowHandle::Win32(_) => "Win32",
        _ => "other",
    }
}

/// Owns one `VkSurfaceKHR`. Dropped after every swapchain created from it
/// and before the instance.
pub(crate) struct SurfaceOwner {
    pub loader: khr::surface::Instance,
    pub handle: vk::SurfaceKHR,
}

impl SurfaceOwner {
    pub(crate) fn new(
        instance: &InstanceOwner,
        source: &SurfaceSource,
    ) -> Result<Self, RenderError> {
        let loader = khr::surface::Instance::new(&instance.entry, &instance.instance);
        let handle = match source {
            SurfaceSource::Window {
                display: RawDisplayHandle::Wayland(display),
                window: RawWindowHandle::Wayland(window),
            } => {
                let info = vk::WaylandSurfaceCreateInfoKHR::default()
                    .display(display.display.as_ptr())
                    .surface(window.surface.as_ptr());
                let wayland =
                    khr::wayland_surface::Instance::new(&instance.entry, &instance.instance);
                // SAFETY: the instance enables `VK_KHR_wayland_surface`
                // (`SurfaceSource::extension`); the display and surface
                // pointers come from a window that the public `Renderer`
                // borrows for its whole lifetime.
                unsafe { wayland.create_wayland_surface(&info, None) }
                    .map_err(|result| vk_error("vkCreateWaylandSurfaceKHR", result))?
            }
            SurfaceSource::Window { .. } => {
                return Err(source.extension().err().unwrap_or_else(unsupported));
            }
            SurfaceSource::Headless => {
                let headless =
                    ash::ext::headless_surface::Instance::new(&instance.entry, &instance.instance);
                // SAFETY: the instance enables `VK_EXT_headless_surface`.
                unsafe {
                    headless
                        .create_headless_surface(&vk::HeadlessSurfaceCreateInfoEXT::default(), None)
                }
                .map_err(|result| vk_error("vkCreateHeadlessSurfaceEXT", result))?
            }
        };
        Ok(Self { loader, handle })
    }

    /// Destroys the surface now (before recreating it); idempotent.
    pub(crate) fn destroy(&mut self) {
        if self.handle != vk::SurfaceKHR::null() {
            // SAFETY: no swapchain uses the surface any more (callers destroy
            // the swapchain first); the handle is nulled so this runs once.
            unsafe { self.loader.destroy_surface(self.handle, None) };
            self.handle = vk::SurfaceKHR::null();
        }
    }
}

fn unsupported() -> RenderError {
    RenderError::new(RenderErrorKind::UnsupportedWindowSystem)
}

impl Drop for SurfaceOwner {
    fn drop(&mut self) {
        self.destroy();
    }
}
