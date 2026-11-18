//! Public platform and window types, backed by the compiled native backend.

use std::time::Duration;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::error::PlatformError;
use crate::event::{CursorGrabMode, Event, PhysicalSize, WindowId};
use crate::linux::wayland::{WaylandPlatform, WaylandWindow};
use crate::window::WindowDescriptor;

/// How long [`Platform::pump_events`] may wait for the window system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpMode {
    /// Process what has arrived and return immediately (game loops).
    Poll,
    /// Wait until something arrives or the duration passes.
    WaitTimeout(Duration),
    /// Wait until something arrives (tools that only react to input).
    Wait,
}

/// Connection to the native window system and owner of its event loop.
///
/// Spawn owns the loop (ADR-0001): the caller pumps events explicitly with
/// [`Platform::pump_events`]; nothing runs in callbacks or background threads.
///
/// **Thread affinity:** neither `Platform` nor [`Window`] is `Send` or `Sync`;
/// use them on the thread that created them.
///
/// Implements [`HasDisplayHandle`] so the renderer can create its instance
/// before any window exists.
pub struct Platform {
    backend: WaylandPlatform,
}

impl Platform {
    /// Connects to the window system named by the environment. On Linux this
    /// is the Wayland compositor (`WAYLAND_DISPLAY`).
    ///
    /// # Errors
    ///
    /// - `PLAT-0001` [`DisplayUnavailable`](crate::PlatformErrorKind::DisplayUnavailable):
    ///   no compositor is reachable or `libwayland-client` is not installed.
    /// - `PLAT-0002` [`UnsupportedDisplay`](crate::PlatformErrorKind::UnsupportedDisplay):
    ///   a required protocol (`wl_compositor`, `xdg_wm_base`) is missing.
    /// - `PLAT-0003` [`ConnectionLost`](crate::PlatformErrorKind::ConnectionLost):
    ///   the connection failed during setup.
    pub fn new() -> Result<Self, PlatformError> {
        Ok(Self {
            backend: WaylandPlatform::connect()?,
        })
    }

    /// Creates a top-level window. Returns once the window system has
    /// assigned its initial size, available through [`Platform::window_size`].
    ///
    /// The window is shown once the renderer presents its first frame (on
    /// Wayland, a surface without content is not mapped).
    ///
    /// # Errors
    ///
    /// - `PLAT-0003` [`ConnectionLost`](crate::PlatformErrorKind::ConnectionLost).
    /// - `PLAT-0005` [`Timeout`](crate::PlatformErrorKind::Timeout): the
    ///   compositor did not configure the window within 5 seconds.
    pub fn create_window(
        &mut self,
        descriptor: &WindowDescriptor,
    ) -> Result<Window, PlatformError> {
        Ok(Window {
            backend: self.backend.create_window(descriptor)?,
        })
    }

    /// Processes window-system messages and appends the resulting raw events
    /// to `events`, in the order the window system reported them. Existing
    /// contents of `events` are kept; reuse one buffer to avoid allocation.
    ///
    /// # Errors
    ///
    /// `PLAT-0003` [`ConnectionLost`](crate::PlatformErrorKind::ConnectionLost)
    /// when the connection failed or the compositor reported a protocol error.
    /// The platform cannot recover; drop it and its windows.
    pub fn pump_events(
        &mut self,
        mode: PumpMode,
        events: &mut Vec<Event>,
    ) -> Result<(), PlatformError> {
        let timeout = match mode {
            PumpMode::Poll => Some(Duration::ZERO),
            PumpMode::WaitTimeout(timeout) => Some(timeout),
            PumpMode::Wait => None,
        };
        self.backend.pump(timeout, events)
    }

    /// Current drawable size of a live window in physical pixels, or `None`
    /// for a destroyed or unknown window.
    pub fn window_size(&self, window: WindowId) -> Option<PhysicalSize> {
        self.backend
            .metrics(window)
            .map(|metrics| metrics.physical())
    }

    /// Current scale factor (physical pixels per logical unit) of a live
    /// window, or `None` for a destroyed or unknown window.
    pub fn scale_factor(&self, window: WindowId) -> Option<f64> {
        self.backend
            .metrics(window)
            .map(|metrics| metrics.scale_factor())
    }

    /// Requests a cursor constraint (`None`, `Confined` or `Locked`) for a live
    /// window.
    ///
    /// Constraints may take effect asynchronously once the window system
    /// activates them; [`WindowEvent::CursorGrabChanged`](crate::WindowEvent::CursorGrabChanged)
    /// reports every change to the active mode, and [`Platform::cursor_grab`]
    /// returns the currently active mode. Focus loss or compositor intervention
    /// releases an active or pending constraint back to
    /// [`CursorGrabMode::None`]. Repeating the current request is a no-op.
    ///
    /// # Errors
    ///
    /// - `PLAT-0006` [`CursorGrabFailed`](crate::PlatformErrorKind::CursorGrabFailed):
    ///   `window` is unknown or destroyed, or a non-`None` constraint was
    ///   requested when the display server lacks pointer-constraint or
    ///   relative-pointer support or has no pointer device.
    /// - `PLAT-0003` [`ConnectionLost`](crate::PlatformErrorKind::ConnectionLost):
    ///   sending the request to the display server failed.
    pub fn set_cursor_grab(
        &mut self,
        window: WindowId,
        mode: CursorGrabMode,
    ) -> Result<(), PlatformError> {
        self.backend.set_cursor_grab(window, mode)
    }

    /// Currently active cursor constraint of a live window, or `None` for a
    /// destroyed or unknown window.
    pub fn cursor_grab(&self, window: WindowId) -> Option<CursorGrabMode> {
        self.backend.cursor_grab(window)
    }
}

impl HasDisplayHandle for Platform {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}

/// A native top-level window. Dropping it destroys the window.
///
/// Implements the `raw-window-handle` traits so the renderer can create a
/// surface (the single approved third-party exception in the public API,
/// SPN-INV-011). A renderer surface must be destroyed before its window.
pub struct Window {
    backend: WaylandWindow,
}

impl Window {
    /// Identity of this window in events and queries.
    pub fn id(&self) -> WindowId {
        self.backend.id()
    }
}

impl std::fmt::Debug for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Window")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Platform").finish_non_exhaustive()
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.backend.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}
