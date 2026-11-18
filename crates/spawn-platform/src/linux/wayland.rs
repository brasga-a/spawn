//! Wayland backend: protocol glue between `wayland-client` and [`Translator`].
//!
//! Every handler decodes one protocol message, forwards it to the translator
//! and applies the requests the translator returns. Decisions live in the
//! translator, which is tested without a display server.
//!
//! Protocols: `wl_compositor` (required, up to v6), `xdg_wm_base` (required,
//! up to v6), `wl_seat` (optional, up to v8; first seat only),
//! `wp_viewporter` + `wp_fractional_scale_v1` (optional; scale stays 1.0
//! without the viewporter), `zxdg_decoration_manager_v1` (optional;
//! server-side decorations are requested when present),
//! `zwp_relative_pointer_manager_v1` (optional, v1; unaccelerated relative
//! pointer motion) and `zwp_pointer_constraints_v1` (optional, v1; oneshot
//! cursor lock and confinement).
//!
//! `libwayland-client` is loaded at runtime; its absence is reported as
//! `PLAT-0001`.

use std::cell::Cell;
use std::marker::PhantomData;
use std::os::fd::AsFd;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use raw_window_handle::{
    DisplayHandle, HandleError, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle,
    WaylandWindowHandle, WindowHandle,
};
use rustix::event::{PollFd, PollFlags, Timespec};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{BindError, GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{
    Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::error::{PlatformError, PlatformErrorKind};
use crate::event::{CursorGrabMode, ElementState, Event, WindowId};
use crate::linux::translate::{Axis, ConstraintId, SurfaceMetrics, SurfaceUpdate, Translator};
use crate::window::WindowDescriptor;

/// How long window creation waits for the compositor's first configure.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(5);

/// Marker that keeps platform objects on the thread that created them.
type NotSend = PhantomData<*const ()>;

/// Connection to the Wayland compositor and the event queue Spawn pumps.
pub(crate) struct WaylandPlatform {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    _not_send: NotSend,
}

enum WaylandConstraint {
    Locked(zwp_locked_pointer_v1::ZwpLockedPointerV1),
    Confined(zwp_confined_pointer_v1::ZwpConfinedPointerV1),
}

impl WaylandConstraint {
    fn destroy(self) {
        match self {
            Self::Locked(locked) => locked.destroy(),
            Self::Confined(confined) => confined.destroy(),
        }
    }
}

/// Shared single-threaded slot so either [`WaylandWindow::drop`] or
/// [`WaylandPlatform`] destroys a window's constraint before the surface or
/// pointer it extends.
#[derive(Clone, Default)]
struct ConstraintSlot(Rc<Cell<Option<WaylandConstraint>>>);

impl ConstraintSlot {
    fn set(&self, constraint: WaylandConstraint) {
        if let Some(previous) = self.0.replace(Some(constraint)) {
            previous.destroy();
        }
    }

    fn destroy(&self) {
        if let Some(existing) = self.0.take() {
            existing.destroy();
        }
    }
}

/// Protocol objects of one window. Destroyed in protocol order on drop.
pub(crate) struct WaylandWindow {
    id: WindowId,
    connection: Connection,
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    viewport: Option<wp_viewport::WpViewport>,
    fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    constraint: ConstraintSlot,
    _not_send: NotSend,
}

/// Bound globals.
struct Globals {
    compositor: wl_compositor::WlCompositor,
    wm_base: xdg_wm_base::XdgWmBase,
    viewporter: Option<wp_viewporter::WpViewporter>,
    fractional_scale: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    decorations: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    relative_pointer_manager: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,
    pointer_constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
    seat: Option<wl_seat::WlSeat>,
}

/// The objects the dispatch handlers need to apply a [`SurfaceUpdate`].
struct SurfaceRecord {
    id: WindowId,
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    viewport: Option<wp_viewport::WpViewport>,
    has_fractional_scale: bool,
    constraint: ConstraintSlot,
}

/// Dispatch state.
struct State {
    globals: Globals,
    translator: Translator,
    surfaces: Vec<SurfaceRecord>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    relative_pointer: Option<zwp_relative_pointer_v1::ZwpRelativePointerV1>,
}

impl WaylandPlatform {
    /// Connects to the compositor named by the environment
    /// (`WAYLAND_DISPLAY` / `WAYLAND_SOCKET`) and binds the globals.
    pub(crate) fn connect() -> Result<Self, PlatformError> {
        let connection = Connection::connect_to_env().map_err(|error| {
            PlatformError::new(PlatformErrorKind::DisplayUnavailable)
                .with_detail("Wayland")
                .with_cause(&error)
        })?;
        let (globals, queue) = registry_queue_init::<State>(&connection).map_err(|error| {
            PlatformError::new(PlatformErrorKind::ConnectionLost)
                .with_detail("reading the Wayland registry")
                .with_cause(&error)
        })?;
        let globals = Globals::bind(&globals, &queue.handle())?;
        let mut platform = Self {
            connection,
            queue,
            state: State {
                globals,
                translator: Translator::default(),
                surfaces: Vec::new(),
                keyboard: None,
                pointer: None,
                relative_pointer: None,
            },
            _not_send: PhantomData,
        };
        // Receive the seat capabilities before the first window exists.
        platform.roundtrip()?;
        Ok(platform)
    }

    /// Creates a toplevel window and waits for its first configure, so its
    /// size and scale are known when this returns.
    pub(crate) fn create_window(
        &mut self,
        descriptor: &WindowDescriptor,
    ) -> Result<WaylandWindow, PlatformError> {
        let qh = self.queue.handle();
        let globals = &self.state.globals;
        let id = self.state.translator.add_window(descriptor.size());

        let surface = globals.compositor.create_surface(&qh, id);
        let xdg_surface = globals.wm_base.get_xdg_surface(&surface, &qh, id);
        let toplevel = xdg_surface.get_toplevel(&qh, id);
        toplevel.set_title(descriptor.title().to_owned());
        if let Some(app_id) = descriptor.app_id() {
            toplevel.set_app_id(app_id.to_owned());
        }
        let viewport = globals
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, &qh, ()));
        // Fractional scale is only usable through a viewport.
        let fractional_scale = viewport
            .as_ref()
            .and(globals.fractional_scale.as_ref())
            .map(|manager| manager.get_fractional_scale(&surface, &qh, id));
        let decoration = globals.decorations.as_ref().map(|manager| {
            let decoration = manager.get_toplevel_decoration(&toplevel, &qh, id);
            decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            decoration
        });
        surface.commit();

        let constraint = ConstraintSlot::default();
        self.state.surfaces.push(SurfaceRecord {
            id,
            surface: surface.clone(),
            xdg_surface: xdg_surface.clone(),
            viewport: viewport.clone(),
            has_fractional_scale: fractional_scale.is_some(),
            constraint: constraint.clone(),
        });
        let window = WaylandWindow {
            id,
            connection: self.connection.clone(),
            surface,
            xdg_surface,
            toplevel,
            viewport,
            fractional_scale,
            decoration,
            constraint,
            _not_send: PhantomData,
        };

        // Dropping `window` on error destroys the protocol objects again.
        let deadline = Instant::now() + CONFIGURE_TIMEOUT;
        while !self.state.translator.is_configured(id) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(
                    PlatformError::new(PlatformErrorKind::Timeout).with_detail(format!(
                        "no configure for a new window within {CONFIGURE_TIMEOUT:?}"
                    )),
                );
            }
            self.dispatch(Some(remaining))?;
        }
        Ok(window)
    }

    /// Dispatches pending messages, waiting at most `timeout` (`None`: no
    /// limit) when none are queued, and appends the resulting events to `out`.
    pub(crate) fn pump(
        &mut self,
        timeout: Option<Duration>,
        out: &mut Vec<Event>,
    ) -> Result<(), PlatformError> {
        self.forget_destroyed_windows();
        self.dispatch(timeout)?;
        self.state.translator.drain_into(out);
        Ok(())
    }

    /// Size and scale of a live window.
    pub(crate) fn metrics(&self, id: WindowId) -> Option<SurfaceMetrics> {
        self.state
            .surfaces
            .iter()
            .any(|record| record.id == id && record.surface.is_alive())
            .then(|| self.state.translator.metrics(id))
            .flatten()
    }

    /// Currently active cursor constraint of a live window.
    pub(crate) fn cursor_grab(&self, id: WindowId) -> Option<CursorGrabMode> {
        self.state
            .surfaces
            .iter()
            .any(|record| record.id == id && record.surface.is_alive())
            .then(|| self.state.translator.cursor_grab(id))
            .flatten()
    }

    /// Requests a cursor constraint (`None`, `Confined` or `Locked`) on `id`.
    pub(crate) fn set_cursor_grab(
        &mut self,
        id: WindowId,
        mode: CursorGrabMode,
    ) -> Result<(), PlatformError> {
        self.forget_destroyed_windows();
        let Some(surface_index) = self
            .state
            .surfaces
            .iter()
            .position(|record| record.id == id)
        else {
            return Err(PlatformError::new(PlatformErrorKind::CursorGrabFailed)
                .with_detail(format!("window {id:?} does not exist or was destroyed")));
        };
        if mode != CursorGrabMode::None {
            if self.state.globals.pointer_constraints.is_none() {
                return Err(PlatformError::new(PlatformErrorKind::CursorGrabFailed)
                    .with_detail("Wayland global `zwp_pointer_constraints_v1` is not available"));
            }
            if mode == CursorGrabMode::Locked
                && self.state.globals.relative_pointer_manager.is_none()
            {
                return Err(
                    PlatformError::new(PlatformErrorKind::CursorGrabFailed).with_detail(
                        "Wayland global `zwp_relative_pointer_manager_v1` is not available",
                    ),
                );
            }
            if self.state.pointer.is_none() {
                return Err(PlatformError::new(PlatformErrorKind::CursorGrabFailed)
                    .with_detail("no Wayland pointer device is available on the seat"));
            }
        }

        let Some(update) = self.state.translator.request_cursor_grab(id, mode) else {
            return Err(PlatformError::new(PlatformErrorKind::CursorGrabFailed)
                .with_detail(format!("window {id:?} does not exist or was destroyed")));
        };
        let record = &self.state.surfaces[surface_index];
        if update.destroy_previous {
            record.constraint.destroy();
        }
        if let Some((cid, requested)) = update.create
            && let (Some(constraints), Some(pointer)) = (
                self.state.globals.pointer_constraints.as_ref(),
                self.state.pointer.as_ref(),
            )
        {
            let qh = self.queue.handle();
            let lifetime = zwp_pointer_constraints_v1::Lifetime::Oneshot;
            let created = match requested {
                CursorGrabMode::Locked => WaylandConstraint::Locked(constraints.lock_pointer(
                    &record.surface,
                    pointer,
                    None,
                    lifetime,
                    &qh,
                    (id, cid),
                )),
                CursorGrabMode::Confined => {
                    WaylandConstraint::Confined(constraints.confine_pointer(
                        &record.surface,
                        pointer,
                        None,
                        lifetime,
                        &qh,
                        (id, cid),
                    ))
                }
                CursorGrabMode::None => return Ok(()),
            };
            record.constraint.set(created);
        }
        self.flush()
    }

    /// The `wl_display` pointer for surface creation.
    pub(crate) fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        display_handle(&self.connection)
    }

    /// Drops the records of windows whose owner destroyed them.
    fn forget_destroyed_windows(&mut self) {
        let State {
            surfaces,
            translator,
            ..
        } = &mut self.state;
        surfaces.retain(|record| {
            let alive = record.surface.is_alive();
            if !alive {
                record.constraint.destroy();
                translator.remove_window(record.id);
            }
            alive
        });
    }

    fn roundtrip(&mut self) -> Result<(), PlatformError> {
        self.queue
            .roundtrip(&mut self.state)
            .map(drop)
            .map_err(|error| self.dispatch_error(&error))
    }

    #[track_caller]
    fn flush(&self) -> Result<(), PlatformError> {
        match self.connection.flush() {
            // The socket is full; the requests stay buffered for the next pump.
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(())
            }
            Err(error) => Err(self.connection_error(&error)),
            Ok(()) => Ok(()),
        }
    }

    /// One pump step: dispatch queued messages, flush requests, wait for the
    /// socket when nothing is queued, read and dispatch what arrived.
    fn dispatch(&mut self, timeout: Option<Duration>) -> Result<(), PlatformError> {
        let dispatched = self
            .queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| self.dispatch_error(&error))?;
        self.flush()?;
        let timeout = if dispatched > 0 {
            Some(Duration::ZERO)
        } else {
            timeout
        };
        // `prepare_read` is `None` while messages are queued; a guard dropped
        // without reading cancels the read.
        if let Some(guard) = self.queue.prepare_read()
            && wait_readable(guard.connection_fd(), timeout)?
        {
            match guard.read() {
                Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(self.connection_error(&error)),
                Ok(_) => {}
            }
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .map(drop)
            .map_err(|error| self.dispatch_error(&error))?;
        self.flush()
    }

    #[track_caller]
    fn dispatch_error(&self, error: &DispatchError) -> PlatformError {
        match error {
            DispatchError::Backend(error) => self.connection_error(error),
            DispatchError::BadMessage { .. } => {
                PlatformError::new(PlatformErrorKind::ConnectionLost).with_cause(error)
            }
        }
    }

    #[track_caller]
    fn connection_error(&self, error: &WaylandError) -> PlatformError {
        let error_out = PlatformError::new(PlatformErrorKind::ConnectionLost).with_cause(error);
        match self.connection.protocol_error() {
            Some(protocol) => error_out.with_detail(format!(
                "protocol error {} on {}@{}: {}",
                protocol.code, protocol.object_interface, protocol.object_id, protocol.message
            )),
            None => error_out,
        }
    }
}

impl Drop for WaylandPlatform {
    fn drop(&mut self) {
        // Extension objects and input devices are released in dependency order;
        // `xdg_wm_base` and `wl_compositor` go away with the connection so they
        // outlive any window still owned by the caller.
        self.state.destroy_all_constraints();
        if let Some(relative) = self.state.relative_pointer.take() {
            relative.destroy();
        }
        if let Some(keyboard) = self
            .state
            .keyboard
            .take()
            .filter(|keyboard| keyboard.version() >= 3)
        {
            keyboard.release();
        }
        if let Some(pointer) = self
            .state
            .pointer
            .take()
            .filter(|pointer| pointer.version() >= 3)
        {
            pointer.release();
        }
        if let Some(seat) = self
            .state
            .globals
            .seat
            .take()
            .filter(|seat| seat.version() >= 5)
        {
            seat.release();
        }
        if let Some(constraints) = self.state.globals.pointer_constraints.take() {
            constraints.destroy();
        }
        if let Some(relative_manager) = self.state.globals.relative_pointer_manager.take() {
            relative_manager.destroy();
        }
        // A failed flush means the connection is already gone; `Drop` cannot
        // report it.
        let _ = self.connection.flush();
    }
}

impl WaylandWindow {
    /// Identity of this window.
    pub(crate) fn id(&self) -> WindowId {
        self.id
    }

    /// The `wl_surface` pointer for surface creation.
    pub(crate) fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let pointer =
            NonNull::new(self.surface.id().as_ptr().cast()).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer));
        // SAFETY: the `wl_surface` is destroyed only in `Drop`, which cannot run
        // while the returned handle borrows `self`.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }

    /// The `wl_display` pointer for surface creation.
    pub(crate) fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        display_handle(&self.connection)
    }
}

impl Drop for WaylandWindow {
    fn drop(&mut self) {
        // Role and extension objects go before the objects they extend.
        self.constraint.destroy();
        if let Some(decoration) = &self.decoration {
            decoration.destroy();
        }
        if let Some(fractional_scale) = &self.fractional_scale {
            fractional_scale.destroy();
        }
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        self.toplevel.destroy();
        self.xdg_surface.destroy();
        self.surface.destroy();
        // A failed flush means the connection is already gone, which also
        // releases the objects; `Drop` cannot report it.
        let _ = self.connection.flush();
    }
}

fn display_handle(connection: &Connection) -> Result<DisplayHandle<'_>, HandleError> {
    let pointer =
        NonNull::new(connection.backend().display_ptr().cast()).ok_or(HandleError::Unavailable)?;
    let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(pointer));
    // SAFETY: the `wl_display` stays connected while any `Connection` clone
    // exists, and the returned handle borrows one.
    Ok(unsafe { DisplayHandle::borrow_raw(raw) })
}

/// Waits until `fd` is readable or `timeout` expires (`None`: no limit).
/// Returns whether it is readable (or has an error the read will report).
#[track_caller]
fn wait_readable(fd: impl AsFd, timeout: Option<Duration>) -> Result<bool, PlatformError> {
    let timespec = timeout.map(|timeout| Timespec {
        tv_sec: i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(timeout.subsec_nanos()),
    });
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    match rustix::event::poll(&mut fds, timespec.as_ref()) {
        Ok(ready) => Ok(ready > 0),
        // A signal interrupted the wait: return to the caller's loop.
        Err(rustix::io::Errno::INTR) => Ok(false),
        Err(error) => Err(PlatformError::new(PlatformErrorKind::ConnectionLost)
            .with_detail("waiting for the Wayland socket")
            .with_cause(&std::io::Error::from(error))),
    }
}

impl Globals {
    #[track_caller]
    fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Result<Self, PlatformError> {
        Ok(Self {
            compositor: required(globals.bind(qh, 1..=6, ()), "wl_compositor")?,
            wm_base: required(globals.bind(qh, 1..=6, ()), "xdg_wm_base")?,
            viewporter: globals.bind(qh, 1..=1, ()).ok(),
            fractional_scale: globals.bind(qh, 1..=1, ()).ok(),
            decorations: globals.bind(qh, 1..=1, ()).ok(),
            relative_pointer_manager: globals.bind(qh, 1..=1, ()).ok(),
            pointer_constraints: globals.bind(qh, 1..=1, ()).ok(),
            seat: globals.bind(qh, 1..=8, ()).ok(),
        })
    }
}

#[track_caller]
fn required<I>(bound: Result<I, BindError>, name: &str) -> Result<I, PlatformError> {
    bound.map_err(|error| {
        PlatformError::new(PlatformErrorKind::UnsupportedDisplay)
            .with_detail(format!("Wayland global `{name}` is not available"))
            .with_cause(&error)
    })
}

impl State {
    fn record(&self, id: WindowId) -> Option<&SurfaceRecord> {
        self.surfaces.iter().find(|record| record.id == id)
    }

    fn destroy_constraint(&self, id: WindowId) {
        if let Some(record) = self.record(id) {
            record.constraint.destroy();
        }
    }

    fn destroy_all_constraints(&self) {
        for record in &self.surfaces {
            record.constraint.destroy();
        }
    }

    fn apply(&self, id: WindowId, update: SurfaceUpdate) {
        let Some(record) = self.record(id) else {
            return;
        };
        record.xdg_surface.ack_configure(update.ack_serial);
        if let (Some(viewport), Some(size)) = (&record.viewport, update.viewport_destination)
            && let (Ok(width), Ok(height)) = (i32::try_from(size.width), i32::try_from(size.height))
        {
            viewport.set_destination(width, height);
        }
        // Do not call `wl_surface.commit()` here: `ack_configure` and
        // `wp_viewport.set_destination` stay pending and apply atomically when
        // the renderer (or Vulkan WSI) commits the matching buffer, avoiding
        // stretching of the previous frame and commits behind WSI's back.
    }
}

fn element_state(pressed: bool) -> ElementState {
    if pressed {
        ElementState::Pressed
    } else {
        ElementState::Released
    }
}

/// The window a protocol object was created for, if it was created by Spawn.
fn window_of(proxy: &impl Proxy) -> Option<WindowId> {
    proxy.data::<WindowId>().copied()
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Globals appearing or disappearing after startup are not used yet.
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wp_viewporter::WpViewporter);
delegate_noop!(State: ignore wp_viewport::WpViewport);
delegate_noop!(State: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(State: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(State: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);
delegate_noop!(State: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, WindowId> for State {
    fn event(
        _: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        _: zxdg_toplevel_decoration_v1::Event,
        _: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The compositor's decoration choice needs no action.
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<wl_surface::WlSurface, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        &id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_surface::Event::PreferredBufferScale { factor } = event
            && let Some(record) = state.record(id)
            // Integer scale is only used without fractional scale, and only
            // through a viewport (buffer scale stays 1).
            && !record.has_fractional_scale
            && record.viewport.is_some()
            && let Ok(factor) = u32::try_from(factor)
        {
            state
                .translator
                .preferred_scale(id, factor.saturating_mul(120));
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        &id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.translator.preferred_scale(id, scale);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        &id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(update) = state.translator.surface_configure(id, serial)
        {
            state.apply(id, update);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        &id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                let suspended = states.as_chunks::<4>().0.iter().any(|&bytes| {
                    u32::from_ne_bytes(bytes) == xdg_toplevel::State::Suspended as u32
                });
                state
                    .translator
                    .toplevel_configure(id, width, height, suspended);
            }
            xdg_toplevel::Event::Close => state.translator.close_requested(id),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        let keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
        match (keyboard, state.keyboard.take()) {
            (true, None) => state.keyboard = Some(seat.get_keyboard(qh, ())),
            (true, Some(existing)) => state.keyboard = Some(existing),
            (false, Some(removed)) => {
                if let Some(unfocused) = state.translator.keyboard_capability_lost() {
                    state.destroy_constraint(unfocused);
                }
                if removed.version() >= 3 {
                    removed.release();
                }
            }
            (false, None) => {}
        }
        let pointer = capabilities.contains(wl_seat::Capability::Pointer);
        match (pointer, state.pointer.take()) {
            (true, None) => {
                let wl_pointer = seat.get_pointer(qh, ());
                state.relative_pointer = state
                    .globals
                    .relative_pointer_manager
                    .as_ref()
                    .map(|manager| manager.get_relative_pointer(&wl_pointer, qh, ()));
                state.pointer = Some(wl_pointer);
            }
            (true, Some(existing)) => state.pointer = Some(existing),
            (false, Some(removed)) => {
                state.destroy_all_constraints();
                state.translator.pointer_capability_lost();
                if let Some(relative) = state.relative_pointer.take() {
                    relative.destroy();
                }
                if removed.version() >= 3 {
                    removed.release();
                }
            }
            (false, None) => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // Physical keys need no keymap; dropping the descriptor closes it.
            wl_keyboard::Event::Keymap { fd, .. } => drop(fd),
            wl_keyboard::Event::Enter { surface, .. } => {
                if let Some(id) = window_of(&surface)
                    && let Some(unfocused) = state.translator.keyboard_enter(id)
                {
                    state.destroy_constraint(unfocused);
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                if let Some(id) = window_of(&surface)
                    && state.translator.keyboard_leave(id)
                {
                    state.destroy_constraint(id);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => match key_state {
                wl_keyboard::KeyState::Pressed => state.translator.key(key, element_state(true)),
                wl_keyboard::KeyState::Released => state.translator.key(key, element_state(false)),
                // Compositor-generated repeats (v10) are not raw transitions.
                _ => {}
            },
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                if let Some(id) = window_of(&surface)
                    && let Some(previous) = state.translator.pointer_enter(id, surface_x, surface_y)
                {
                    state.destroy_constraint(previous);
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if let Some(id) = window_of(&surface)
                    && state.translator.pointer_leave(id)
                {
                    state.destroy_constraint(id);
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => state.translator.pointer_motion(surface_x, surface_y),
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(button_state),
                ..
            } => state.translator.pointer_button(
                button,
                element_state(button_state == wl_pointer::ButtonState::Pressed),
            ),
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                if let Some(axis) = scroll_axis(axis) {
                    state.translator.pointer_axis(axis, value);
                    // Before version 5 there are no frame events.
                    if pointer.version() < 5 {
                        state.translator.pointer_frame();
                    }
                }
            }
            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(axis),
                value120,
            } => {
                if let Some(axis) = scroll_axis(axis) {
                    state.translator.pointer_axis_value_120(axis, value120);
                }
            }
            wl_pointer::Event::Frame => state.translator.pointer_frame(),
            _ => {}
        }
    }
}

impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        event: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `wayland-client` already decodes `wl_fixed_t` (`i32 / 256.0`) to `f64`.
        if let zwp_relative_pointer_v1::Event::RelativeMotion {
            dx_unaccel,
            dy_unaccel,
            ..
        } = event
        {
            state
                .translator
                .pointer_relative_motion(dx_unaccel, dy_unaccel);
        }
    }
}

impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, (WindowId, ConstraintId)> for State {
    fn event(
        state: &mut Self,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        event: zwp_locked_pointer_v1::Event,
        &(id, cid): &(WindowId, ConstraintId),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => {
                state
                    .translator
                    .constraint_activated(id, cid, CursorGrabMode::Locked);
            }
            zwp_locked_pointer_v1::Event::Unlocked
                if state.translator.constraint_deactivated(id, cid) =>
            {
                state.destroy_constraint(id);
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_confined_pointer_v1::ZwpConfinedPointerV1, (WindowId, ConstraintId)> for State {
    fn event(
        state: &mut Self,
        _: &zwp_confined_pointer_v1::ZwpConfinedPointerV1,
        event: zwp_confined_pointer_v1::Event,
        &(id, cid): &(WindowId, ConstraintId),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_confined_pointer_v1::Event::Confined => {
                state
                    .translator
                    .constraint_activated(id, cid, CursorGrabMode::Confined);
            }
            zwp_confined_pointer_v1::Event::Unconfined
                if state.translator.constraint_deactivated(id, cid) =>
            {
                state.destroy_constraint(id);
            }
            _ => {}
        }
    }
}

fn scroll_axis(axis: wl_pointer::Axis) -> Option<Axis> {
    match axis {
        wl_pointer::Axis::HorizontalScroll => Some(Axis::Horizontal),
        wl_pointer::Axis::VerticalScroll => Some(Axis::Vertical),
        _ => None,
    }
}
