//! Manual smoke test of the Vulkan clear frame (foundation Phase 3).
//!
//! Opens one Wayland window through `spawn-platform`, initializes
//! `spawn-render` through the window's `raw-window-handle` traits and presents
//! a solid clear color every frame. Resizes reconfigure the swapchain; while
//! the window is hidden (`Occluded`) or has no drawable size, rendering stops
//! and the loop only waits for window events. Exits on the first close request
//! or Escape. Once per second it prints frame counts and validation messages.
//!
//! ```text
//! cargo run --example vulkan_clear
//! SPAWN_SMOKE_SECONDS=10 cargo run --example vulkan_clear   # auto-close
//! ```
//!
//! This is a development harness, not the Phase 5 composition root. For
//! platform problems without Vulkan, use
//! `cargo run -p spawn-platform --features wayland --example wayland_window`.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    match smoke::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("vulkan_clear: only Linux/Wayland is supported");
}

#[cfg(target_os = "linux")]
mod smoke {
    use std::error::Error;
    use std::time::{Duration, Instant};

    use spawn_platform::{
        ElementState, Event, KeyCode, LogicalSize, Platform, PumpMode, WindowDescriptor,
        WindowEvent,
    };
    use spawn_render::{
        ClearColor, FrameOutcome, Renderer, RendererDescriptor, SurfaceSize, ValidationMessage,
    };

    /// A deep teal, distinct from compositor backgrounds and from the
    /// `wl_shm` smoke example's color.
    const COLOR: ClearColor = ClearColor::new(0.02, 0.25, 0.30, 1.0);

    #[derive(Default)]
    struct Counts {
        presented: u64,
        suspended: u64,
        skipped: u64,
        idle_waits: u64,
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let auto_close = std::env::var("SPAWN_SMOKE_SECONDS")
            .ok()
            .and_then(|seconds| seconds.parse().ok())
            .map(Duration::from_secs);

        let mut platform = Platform::new()?;
        let descriptor = WindowDescriptor::new("Spawn Vulkan clear", LogicalSize::new(960, 540))?
            .with_app_id("dev.spawn.vulkan-clear");
        let window = platform.create_window(&descriptor)?;
        let size = platform.window_size(window.id()).ok_or("window vanished")?;

        let mut renderer = Renderer::new(
            &window,
            SurfaceSize::new(size.width, size.height),
            &RendererDescriptor::default(),
        )?;
        let info = renderer.info();
        eprintln!(
            "renderer: {} ({:?}), Vulkan {}.{}.{}, driver {}, validation {}",
            info.gpu_name,
            info.gpu_kind,
            info.api_version.0,
            info.api_version.1,
            info.api_version.2,
            info.driver,
            if info.validation_enabled {
                "on"
            } else {
                "OFF (layer not installed or disabled)"
            }
        );
        eprintln!(
            "window {:?}: {}x{} physical pixels",
            window.id(),
            size.width,
            size.height
        );

        let started = Instant::now();
        let mut last_report = Instant::now();
        let mut counts = Counts::default();
        let mut events = Vec::new();
        let mut messages = Vec::new();
        let mut occluded = false;
        let mut idle = false;
        let mut close = None;

        while close.is_none() {
            events.clear();
            // Wait for events instead of spinning when nothing is drawn.
            let mode = if idle || occluded {
                counts.idle_waits += 1;
                PumpMode::WaitTimeout(Duration::from_millis(250))
            } else {
                PumpMode::Poll
            };
            platform.pump_events(mode, &mut events)?;
            for event in &events {
                let Event::Window { event, .. } = event else {
                    continue;
                };
                match event {
                    WindowEvent::CloseRequested => close = Some("close request"),
                    WindowEvent::Key(key)
                        if key.key == KeyCode::Escape && key.state == ElementState::Pressed =>
                    {
                        close = Some("Escape");
                    }
                    WindowEvent::Resized(size) => {
                        eprintln!("resized to {}x{}", size.width, size.height);
                        renderer.resize(SurfaceSize::new(size.width, size.height));
                    }
                    WindowEvent::Occluded(hidden) => {
                        eprintln!("occluded: {hidden}");
                        occluded = *hidden;
                    }
                    WindowEvent::ScaleFactorChanged { scale_factor } => {
                        eprintln!("scale factor {scale_factor}")
                    }
                    _ => {}
                }
            }
            if close.is_some() {
                break;
            }

            if !occluded {
                match renderer.render_clear(COLOR)? {
                    FrameOutcome::Presented => {
                        counts.presented += 1;
                        idle = false;
                    }
                    FrameOutcome::Suspended => {
                        counts.suspended += 1;
                        idle = true;
                    }
                    FrameOutcome::Skipped => {
                        counts.skipped += 1;
                        idle = false;
                    }
                }
            }

            if last_report.elapsed() >= Duration::from_secs(1) {
                report_validation(&mut renderer, &mut messages);
                eprintln!(
                    "t={:>5.1}s presented={} suspended={} skipped={} idle_waits={} state={:?}",
                    started.elapsed().as_secs_f64(),
                    counts.presented,
                    counts.suspended,
                    counts.skipped,
                    counts.idle_waits,
                    renderer.state()
                );
                last_report = Instant::now();
            }
            if auto_close.is_some_and(|limit| started.elapsed() >= limit) {
                close = Some("auto-close");
            }
        }

        report_validation(&mut renderer, &mut messages);
        let validation_errors = renderer.validation_error_count();
        // Teardown in ownership order: GPU resources, window, connection.
        drop(renderer);
        drop(window);
        drop(platform);
        let reason = close.unwrap_or("loop ended");
        if validation_errors > 0 {
            return Err(format!(
                "closed ({reason}) with {validation_errors} validation error(s): \
                 presented={} suspended={} skipped={}",
                counts.presented, counts.suspended, counts.skipped
            )
            .into());
        }
        eprintln!(
            "closed cleanly ({reason}): presented={} suspended={} skipped={} validation_errors=0",
            counts.presented, counts.suspended, counts.skipped
        );
        Ok(())
    }

    fn report_validation(renderer: &mut Renderer<'_>, messages: &mut Vec<ValidationMessage>) {
        let dropped = renderer.drain_validation_messages(messages);
        for message in messages.drain(..) {
            eprintln!("validation {:?}: {}", message.severity, message.text);
        }
        if dropped > 0 {
            eprintln!("validation: {dropped} message(s) dropped");
        }
    }
}
