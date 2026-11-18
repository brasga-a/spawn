//! Manual smoke test of the Milestone 2 Phase 3 Vulkan 1.3 `dynamicRendering`
//! colored triangle (`D32_SFLOAT` reversed-Z depth + in-house GPU block allocator).
//!
//! Cooks `shaders/triangle.slang` offline first with `spawn-cli`:
//!
//! ```text
//! cargo run -p spawn-cli -- shaders build
//! cargo run --example vulkan_triangle
//! SPAWN_SMOKE_SECONDS=5 cargo run --example vulkan_triangle   # bounded smoke
//! ```
//!
//! Keeps `examples/vulkan_clear.rs` (Foundation Phase 3) intact.

#![forbid(unsafe_code)]

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
    eprintln!("vulkan_triangle: only Linux/Wayland is supported");
}

#[cfg(target_os = "linux")]
mod smoke {
    use std::error::Error;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use spawn_platform::{
        ElementState, Event, KeyCode, LogicalSize, Platform, PumpMode, WindowDescriptor,
        WindowEvent,
    };
    use spawn_render::{
        ClearColor, FrameOutcome, GraphicsPipelineDescriptor, Renderer, RendererDescriptor,
        SurfaceSize, ValidationMessage,
    };

    /// Dark slate background so the RGB-interpolated triangle stands out clearly.
    const CLEAR: ClearColor = ClearColor::new(0.02, 0.03, 0.06, 1.0);

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
            .and_then(|seconds| seconds.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(Duration::from_secs_f64);

        let shader_dir = std::env::var_os("SPAWN_SHADER_DIR")
            .map_or_else(|| PathBuf::from("target/shaders"), PathBuf::from);
        let vs_path = shader_dir.join("triangle.vs_main.spnshdr");
        let fs_path = shader_dir.join("triangle.fs_main.spnshdr");

        let vs_bytes = fs::read(&vs_path).map_err(|err| {
            format!(
                "missing cooked vertex shader `{}` ({err}); run `cargo run -p spawn-cli -- shaders build` first",
                vs_path.display()
            )
        })?;
        let fs_bytes = fs::read(&fs_path).map_err(|err| {
            format!(
                "missing cooked fragment shader `{}` ({err}); run `cargo run -p spawn-cli -- shaders build` first",
                fs_path.display()
            )
        })?;

        let mut platform = Platform::new()?;
        let descriptor =
            WindowDescriptor::new("Spawn Vulkan triangle", LogicalSize::new(960, 540))?
                .with_app_id("dev.spawn.vulkan-triangle");
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

        let vs = renderer.load_shader_named(&vs_path.to_string_lossy(), &vs_bytes)?;
        let fs = renderer.load_shader_named(&fs_path.to_string_lossy(), &fs_bytes)?;
        let pipeline = renderer
            .create_graphics_pipeline(&GraphicsPipelineDescriptor::new("triangle", vs, fs))?;

        let init_stats = renderer.allocator_stats();
        eprintln!(
            "allocator (startup): blocks={} dedicated={} device_local_block_bytes={} live_allocs={} live_bytes={}",
            init_stats.block_count,
            init_stats.dedicated_block_count,
            init_stats.device_local_block_bytes,
            init_stats.live_allocations,
            init_stats.live_allocated_bytes,
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
                        eprintln!("scale factor {scale_factor}");
                    }
                    _ => {}
                }
            }
            if close.is_some() {
                break;
            }

            if !occluded {
                match renderer.render_triangle(pipeline, CLEAR, 3)? {
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
                let stats = renderer.allocator_stats();
                eprintln!(
                    "t={:>5.1}s presented={} suspended={} skipped={} idle_waits={} state={:?} blocks={} live_allocs={} live_bytes={}",
                    started.elapsed().as_secs_f64(),
                    counts.presented,
                    counts.suspended,
                    counts.skipped,
                    counts.idle_waits,
                    renderer.state(),
                    stats.block_count,
                    stats.live_allocations,
                    stats.live_allocated_bytes,
                );
                last_report = Instant::now();
            }
            if auto_close.is_some_and(|limit| started.elapsed() >= limit) {
                close = Some("auto-close");
            }
        }

        let active_stats = renderer.allocator_stats();
        renderer.destroy_pipeline(pipeline)?;
        renderer.destroy_shader(vs)?;
        renderer.destroy_shader(fs)?;
        report_validation(&mut renderer, &mut messages);
        let validation_errors = renderer.validation_error_count();

        // Explicit shutdown + reverse-order drop: GPU resources -> window -> Wayland connection.
        let closed_stats = renderer.close()?;
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
            "closed cleanly ({reason}): presented={} suspended={} skipped={} validation_errors=0 \
             active_blocks={} active_live_bytes={} peak_block_bytes={} peak_allocated_bytes={} \
             shutdown_blocks={} shutdown_live_allocs={} shutdown_retired_allocs={}",
            counts.presented,
            counts.suspended,
            counts.skipped,
            active_stats.block_count,
            active_stats.live_allocated_bytes,
            closed_stats.peak_block_bytes,
            closed_stats.peak_allocated_bytes,
            closed_stats.block_count,
            closed_stats.live_allocations,
            closed_stats.retired_allocations,
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
