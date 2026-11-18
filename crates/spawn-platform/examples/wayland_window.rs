//! Manual smoke test of the Wayland backend (foundation Phase 2).
//!
//! Opens one window, prints every raw event in delivery order and exits on the
//! first close request (window-manager close) or Escape. Set
//! `SPAWN_SMOKE_SECONDS=<n>` to close automatically after `n` seconds.
//!
//! ```text
//! cargo run -p spawn-platform --features wayland --example wayland_window
//! ```
//!
//! A Wayland surface is shown only once it has content, and presenting content
//! is the renderer's job (Phase 3). The `Painter` below stands in for the
//! renderer: like `spawn-render` will with Vulkan, it reaches the window only
//! through the `raw-window-handle` traits, then attaches a solid-color
//! shared-memory buffer of the window's physical size.

use std::error::Error;
use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use spawn_platform::{
    ElementState, Event, KeyCode, LogicalSize, PhysicalSize, Platform, PumpMode, WindowDescriptor,
    WindowEvent,
};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let auto_close = std::env::var("SPAWN_SMOKE_SECONDS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map(Duration::from_secs);

    let mut platform = Platform::new()?;
    let descriptor =
        WindowDescriptor::new("Spawn platform smoke test", LogicalSize::new(960, 540))?
            .with_app_id("dev.spawn.smoke");
    let window = platform.create_window(&descriptor)?;
    let size = platform.window_size(window.id()).ok_or("window vanished")?;
    let scale = platform
        .scale_factor(window.id())
        .ok_or("window vanished")?;
    eprintln!(
        "created {:?}: {}x{} physical pixels, scale {scale}",
        window.id(),
        size.width,
        size.height
    );

    let mut painter = Painter::new(&window)?;
    painter.paint(size)?;

    let started = Instant::now();
    let mut events = Vec::new();
    let mut close_requests = 0_u32;
    let mut escape = false;
    while close_requests == 0 && !escape {
        events.clear();
        platform.pump_events(
            PumpMode::WaitTimeout(Duration::from_millis(100)),
            &mut events,
        )?;
        for event in &events {
            eprintln!("{event:?}");
            let Event::Window { event, .. } = event else {
                continue;
            };
            match event {
                WindowEvent::CloseRequested => close_requests += 1,
                WindowEvent::Resized(size) => painter.paint(*size)?,
                WindowEvent::Key(key)
                    if key.key == KeyCode::Escape && key.state == ElementState::Pressed =>
                {
                    escape = true;
                }
                _ => {}
            }
        }
        if auto_close.is_some_and(|limit| started.elapsed() >= limit) {
            eprintln!("auto-close after {:?}", started.elapsed());
            break;
        }
    }

    // Teardown in ownership order: content, window, connection.
    drop(painter);
    drop(window);
    drop(platform);
    eprintln!(
        "closed cleanly ({close_requests} close request(s) in the last batch, escape: {escape})"
    );
    Ok(())
}

/// Stand-in renderer: presents a solid color through `wl_shm`.
struct Painter {
    connection: Connection,
    queue: EventQueue<PainterState>,
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    buffer: Option<wl_buffer::WlBuffer>,
}

struct PainterState;

impl Painter {
    fn new(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Result<Self, Box<dyn Error>> {
        let RawDisplayHandle::Wayland(display) = window.display_handle()?.as_raw() else {
            return Err("not a Wayland display".into());
        };
        let RawWindowHandle::Wayland(surface) = window.window_handle()?.as_raw() else {
            return Err("not a Wayland window".into());
        };
        // SAFETY: the display pointer comes from a live window that outlives
        // the painter (dropped first in `run`).
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<PainterState>(&connection)?;
        let shm = globals.bind(&queue.handle(), 1..=1, ())?;
        // SAFETY: the surface pointer is a live `wl_surface` owned by the
        // window, which outlives the painter.
        let surface_id = unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface.surface.as_ptr().cast(),
            )
        }?;
        let surface = wl_surface::WlSurface::from_id(&connection, surface_id)?;
        Ok(Self {
            connection,
            queue,
            shm,
            surface,
            buffer: None,
        })
    }

    fn paint(&mut self, size: PhysicalSize) -> Result<(), Box<dyn Error>> {
        self.queue.dispatch_pending(&mut PainterState)?;
        if size.is_empty() {
            return Ok(());
        }
        let width = i32::try_from(size.width)?;
        let height = i32::try_from(size.height)?;
        let stride = width.checked_mul(4).ok_or("row too large")?;
        let length = stride.checked_mul(height).ok_or("buffer too large")?;

        // XRGB8888 in little-endian byte order: B, G, R, X.
        let pixel = [0x60, 0x40, 0x18, 0xff];
        let row: Vec<u8> = pixel.repeat(size.width as usize);
        let fd = rustix::fs::memfd_create("spawn-smoke", rustix::fs::MemfdFlags::CLOEXEC)?;
        let mut file = std::fs::File::from(fd);
        for _ in 0..size.height {
            file.write_all(&row)?;
        }

        let qh = self.queue.handle();
        let pool = self.shm.create_pool(file.as_fd(), length, &qh, ());
        let buffer =
            pool.create_buffer(0, width, height, stride, wl_shm::Format::Xrgb8888, &qh, ());
        pool.destroy();
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
        if let Some(previous) = self.buffer.replace(buffer) {
            previous.destroy();
        }
        self.connection.flush()?;
        Ok(())
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.destroy();
        }
        let _ = self.connection.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PainterState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(PainterState: ignore wl_shm::WlShm);
delegate_noop!(PainterState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(PainterState: ignore wl_buffer::WlBuffer);
