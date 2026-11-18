//! Native runtime backend wiring (`Wayland` + `Vulkan 1.3` on Linux; structured
//! `ENGN-0002` diagnostic on targets without a native backend yet).

#![forbid(unsafe_code)]

use crate::error::{EngineError, EngineErrorKind};
use crate::options::RuntimeOptions;
use crate::runtime::SessionSummary;

/// Runs the native runtime with `options`, writing startup, runtime, and
/// shutdown diagnostics to `stderr` (`std` only, ADR-0009).
///
/// # Errors
///
/// Returns [`EngineError`] if the platform lacks a compiled native backend or
/// if startup, shader loading, event pumping, simulation timing, rendering, or
/// validation fails.
#[cfg(target_os = "linux")]
pub fn run_native(options: &RuntimeOptions) -> Result<SessionSummary, EngineError> {
    linux::run(options)
}

/// Runs the native runtime on targets where no window/graphics backend is
/// implemented yet, returning `ENGN-0002` cleanly (`FND-INV-12`).
///
/// # Errors
///
/// Always returns [`EngineErrorKind::UnsupportedPlatform`] (`ENGN-0002`).
#[cfg(not(target_os = "linux"))]
pub fn run_native(_options: &RuntimeOptions) -> Result<SessionSummary, EngineError> {
    Err(unsupported_platform_error(std::env::consts::OS))
}

/// Constructs the `ENGN-0002` diagnostic for an unsupported host OS.
#[track_caller]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn unsupported_platform_error(os: &str) -> EngineError {
    EngineError::new(EngineErrorKind::UnsupportedPlatform).with_detail(format!(
        "host OS `{os}` has no native window/surface backend enabled yet (supported: Linux/Wayland)"
    ))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use spawn_core::Tick;
    use spawn_platform::{
        CursorGrabMode, Event, PhysicalSize, Platform, PlatformErrorKind, PumpMode, Window,
        WindowDescriptor, WindowEvent, WindowId,
    };
    use spawn_render::{
        CameraRelativeView, ClearColor, FrameOutcome, GpuAllocatorStats,
        GraphicsPipelineDescriptor, MeshDescriptor, MeshDraw, MeshHandle, PipelineHandle,
        PresentationState, RenderError, RenderErrorKind, Renderer, RendererDescriptor,
        ShaderHandle, SurfaceSize, ValidationMessage,
    };

    use crate::camera::FreeCameraPose;
    use crate::error::{EngineError, EngineErrorKind};
    use crate::options::{RuntimeOptions, read_cooked_shader_bounded};
    use crate::runtime::{
        RenderSurface, RuntimeBackend, SessionSummary, WindowSurface, run_session,
    };
    use crate::scene::{MESH_3D_FS_ARTIFACT, MESH_3D_VS_ARTIFACT};

    #[track_caller]
    fn map_render_error(detail: impl Into<String>, err: RenderError) -> EngineError {
        let kind = match err.kind() {
            RenderErrorKind::InvalidShaderArtifact | RenderErrorKind::ShaderLayoutMismatch => {
                EngineErrorKind::Shader
            }
            _ => EngineErrorKind::Render,
        };
        EngineError::new(kind).with_detail(detail).with_cause(&err)
    }

    impl WindowSurface for Window {
        fn id(&self) -> WindowId {
            self.id()
        }
    }

    impl RenderSurface for Renderer<'_> {
        fn resize(&mut self, size: SurfaceSize) {
            Renderer::resize(self, size);
        }

        fn render_clear(&mut self, color: ClearColor) -> Result<FrameOutcome, EngineError> {
            Renderer::render_clear(self, color)
                .map_err(|err| map_render_error("presenting clear frame", err))
        }

        fn state(&self) -> PresentationState {
            Renderer::state(self)
        }

        fn drain_validation_messages(&mut self, out: &mut Vec<ValidationMessage>) -> u64 {
            Renderer::drain_validation_messages(self, out)
        }

        fn validation_error_count(&self) -> u64 {
            Renderer::validation_error_count(self)
        }

        fn load_shader(
            &mut self,
            label: &str,
            cooked_bytes: &[u8],
        ) -> Result<ShaderHandle, EngineError> {
            Renderer::load_shader_named(self, label, cooked_bytes)
                .map_err(|err| map_render_error(format!("loading cooked shader `{label}`"), err))
        }

        fn destroy_shader(&mut self, handle: ShaderHandle) -> Result<(), EngineError> {
            Renderer::destroy_shader(self, handle)
                .map_err(|err| map_render_error("destroying shader handle", err))
        }

        fn create_graphics_pipeline(
            &mut self,
            descriptor: &GraphicsPipelineDescriptor<'_>,
        ) -> Result<PipelineHandle, EngineError> {
            Renderer::create_graphics_pipeline(self, descriptor).map_err(|err| {
                map_render_error(
                    format!("creating graphics pipeline `{}`", descriptor.label),
                    err,
                )
            })
        }

        fn destroy_pipeline(&mut self, handle: PipelineHandle) -> Result<(), EngineError> {
            Renderer::destroy_pipeline(self, handle)
                .map_err(|err| map_render_error("destroying graphics pipeline", err))
        }

        fn upload_mesh(
            &mut self,
            descriptor: &MeshDescriptor<'_>,
        ) -> Result<MeshHandle, EngineError> {
            Renderer::upload_mesh(self, descriptor).map_err(|err| {
                map_render_error(
                    format!("uploading procedural mesh `{}`", descriptor.label),
                    err,
                )
            })
        }

        fn destroy_mesh(&mut self, handle: MeshHandle) -> Result<(), EngineError> {
            Renderer::destroy_mesh(self, handle)
                .map_err(|err| map_render_error("destroying procedural mesh", err))
        }

        fn render_meshes(
            &mut self,
            pipeline: PipelineHandle,
            clear_color: ClearColor,
            view: &CameraRelativeView,
            draws: &[MeshDraw],
        ) -> Result<FrameOutcome, EngineError> {
            Renderer::render_meshes(self, pipeline, clear_color, view, draws)
                .map_err(|err| map_render_error("presenting 3D mesh frame", err))
        }

        fn allocator_stats(&self) -> GpuAllocatorStats {
            Renderer::allocator_stats(self)
        }

        fn close(self) -> Result<GpuAllocatorStats, EngineError> {
            Renderer::close(self)
                .map_err(|err| map_render_error("closing Vulkan 1.3 renderer", err))
        }
    }

    struct LinuxBackend {
        last_instant: Option<Instant>,
        last_report: Duration,
        latest_pose: Option<FreeCameraPose>,
        latest_grab: CursorGrabMode,
    }

    impl LinuxBackend {
        fn new() -> Self {
            Self {
                last_instant: None,
                last_report: Duration::ZERO,
                latest_pose: None,
                latest_grab: CursorGrabMode::None,
            }
        }
    }

    impl RuntimeBackend for LinuxBackend {
        type Platform = Platform;
        type Window = Window;
        type Renderer<'w> = Renderer<'w>;

        fn read_elapsed(&mut self) -> Duration {
            let now = Instant::now();
            match self.last_instant.replace(now) {
                Some(previous) => now.saturating_duration_since(previous),
                None => Duration::ZERO,
            }
        }

        fn create_platform(&mut self) -> Result<Self::Platform, EngineError> {
            Platform::new().map_err(|err| {
                EngineError::new(EngineErrorKind::Platform)
                    .with_detail("connecting to Wayland display server")
                    .with_cause(&err)
            })
        }

        fn create_window(
            &mut self,
            platform: &mut Self::Platform,
            descriptor: &WindowDescriptor,
        ) -> Result<(Self::Window, PhysicalSize), EngineError> {
            let window = platform.create_window(descriptor).map_err(|err| {
                EngineError::new(EngineErrorKind::Platform)
                    .with_detail("creating native top-level window")
                    .with_cause(&err)
            })?;
            let id = window.id();
            let size = platform.window_size(id).ok_or_else(|| {
                EngineError::new(EngineErrorKind::Platform)
                    .with_detail(format!("window {id:?} reported no initial physical size"))
            })?;
            let scale = platform.scale_factor(id).unwrap_or(1.0);
            eprintln!(
                "spawn-engine: window {id:?} configured at {}x{} physical pixels (scale {scale})",
                size.width, size.height
            );
            Ok((window, size))
        }

        fn set_cursor_grab(
            &mut self,
            platform: &mut Self::Platform,
            window: WindowId,
            mode: CursorGrabMode,
        ) -> Result<(), EngineError> {
            eprintln!("spawn-engine: setting cursor grab on {window:?} to {mode:?}");
            platform.set_cursor_grab(window, mode).map_err(|err| {
                let recoverable = err.kind() == PlatformErrorKind::CursorGrabFailed;
                let engine_err = EngineError::new(EngineErrorKind::Platform)
                    .with_detail(format!("setting cursor grab to {mode:?}"))
                    .with_cause(&err);
                if recoverable {
                    engine_err.with_recoverable_cursor_grab()
                } else {
                    engine_err
                }
            })
        }

        fn pump_events(
            &mut self,
            platform: &mut Self::Platform,
            wait_timeout: Option<Duration>,
            events: &mut Vec<Event>,
        ) -> Result<(), EngineError> {
            let mode = match wait_timeout {
                Some(timeout) => PumpMode::WaitTimeout(timeout),
                None => PumpMode::Poll,
            };
            platform.pump_events(mode, events).map_err(|err| {
                EngineError::new(EngineErrorKind::Platform)
                    .with_detail("pumping platform event loop")
                    .with_cause(&err)
            })
        }

        fn create_renderer<'w>(
            &mut self,
            window: &'w Self::Window,
            size: SurfaceSize,
            descriptor: &RendererDescriptor,
        ) -> Result<Self::Renderer<'w>, EngineError> {
            let renderer = Renderer::new(window, size, descriptor).map_err(|err| {
                EngineError::new(EngineErrorKind::Render)
                    .with_detail("initializing Vulkan 1.3 renderer and swapchain")
                    .with_cause(&err)
            })?;
            let info = renderer.info();
            eprintln!(
                "spawn-engine: renderer {} ({:?}), Vulkan {}.{}.{}, driver {}, validation {}",
                info.gpu_name,
                info.gpu_kind,
                info.api_version.0,
                info.api_version.1,
                info.api_version.2,
                info.driver,
                if info.validation_enabled { "on" } else { "off" }
            );
            Ok(renderer)
        }

        fn read_cooked_shader(
            &mut self,
            shader_dir: &Path,
            artifact_name: &str,
        ) -> Result<Vec<u8>, EngineError> {
            let path = shader_dir.join(artifact_name);
            let stage = match artifact_name {
                MESH_3D_VS_ARTIFACT => "vertex",
                MESH_3D_FS_ARTIFACT => "fragment",
                _ => "shader",
            };
            read_cooked_shader_bounded(&path, stage)
        }

        fn on_scene_initialized(
            &mut self,
            mesh_count: usize,
            instance_count: usize,
            stats: GpuAllocatorStats,
        ) {
            eprintln!(
                "spawn-engine: scene initialized: meshes={mesh_count} instances={instance_count} \
                 blocks={} dedicated={} live_allocs={} live_bytes={}",
                stats.block_count,
                stats.dedicated_block_count,
                stats.live_allocations,
                stats.live_allocated_bytes,
            );
        }

        fn on_camera_tick(
            &mut self,
            _tick: Tick,
            pose: &FreeCameraPose,
            cursor_grab: CursorGrabMode,
        ) {
            self.latest_pose = Some(*pose);
            self.latest_grab = cursor_grab;
        }

        fn on_cursor_grab_unavailable(&mut self, error: &EngineError) {
            eprintln!(
                "spawn-engine: warning: cursor grab unavailable, continuing unlocked: {error}"
            );
        }

        fn on_backlog_dropped(&mut self, dropped: Duration) {
            eprintln!(
                "spawn-engine: warning: dropped {:?} of simulation backlog (catch-up limit reached)",
                dropped
            );
        }

        fn on_validation_messages(&mut self, messages: &[ValidationMessage], dropped: u64) {
            for message in messages {
                eprintln!(
                    "spawn-engine: validation {:?}: {}",
                    message.severity, message.text
                );
            }
            if dropped > 0 {
                eprintln!("spawn-engine: validation: {dropped} message(s) dropped");
            }
        }

        fn on_window_event(&mut self, window: WindowId, event: &WindowEvent) {
            match event {
                WindowEvent::Resized(size) => {
                    eprintln!(
                        "spawn-engine: {window:?} resized to {}x{} physical pixels",
                        size.width, size.height
                    );
                }
                WindowEvent::Occluded(hidden) => {
                    eprintln!("spawn-engine: {window:?} occluded={hidden}");
                }
                WindowEvent::Focused(focused) => {
                    eprintln!("spawn-engine: {window:?} focused={focused}");
                }
                WindowEvent::ScaleFactorChanged { scale_factor } => {
                    eprintln!("spawn-engine: {window:?} scale_factor={scale_factor}");
                }
                WindowEvent::CursorGrabChanged(mode) => {
                    eprintln!("spawn-engine: {window:?} cursor_grab={mode:?}");
                }
                WindowEvent::CloseRequested => {
                    eprintln!("spawn-engine: {window:?} close requested");
                }
                _ => {}
            }
        }

        fn on_frame_step(&mut self, state: PresentationState, elapsed: Duration) {
            if elapsed.saturating_sub(self.last_report) >= Duration::from_secs(1) {
                if let Some(pose) = self.latest_pose {
                    eprintln!(
                        "spawn-engine: t={:>5.1}s state={state:?} grab={:?} pos=({:.2}, {:.2}, {:.2}) yaw={:.2} pitch={:.2}",
                        elapsed.as_secs_f64(),
                        self.latest_grab,
                        pose.world_pos.x,
                        pose.world_pos.y,
                        pose.world_pos.z,
                        pose.yaw_radians,
                        pose.pitch_radians,
                    );
                } else {
                    eprintln!(
                        "spawn-engine: t={:>5.1}s state={state:?}",
                        elapsed.as_secs_f64()
                    );
                }
                self.last_report = elapsed;
            }
        }

        fn on_shutdown_stats(
            &mut self,
            active_stats: GpuAllocatorStats,
            shutdown_stats: GpuAllocatorStats,
        ) {
            eprintln!(
                "spawn-engine: allocator: active_blocks={} active_live_allocs={} active_live_bytes={} \
                 peak_block_bytes={} peak_allocated_bytes={} shutdown_blocks={} \
                 shutdown_live_allocs={} shutdown_retired_allocs={}",
                active_stats.block_count,
                active_stats.live_allocations,
                active_stats.live_allocated_bytes,
                shutdown_stats.peak_block_bytes,
                shutdown_stats.peak_allocated_bytes,
                shutdown_stats.block_count,
                shutdown_stats.live_allocations,
                shutdown_stats.retired_allocations,
            );
        }
    }

    pub(super) fn run(options: &RuntimeOptions) -> Result<SessionSummary, EngineError> {
        let mut backend = LinuxBackend::new();
        let summary = run_session(options, &mut backend)?;
        eprintln!(
            "spawn-engine: exited cleanly ({:?}): ticks={} presented={} suspended={} skipped={} \
             dropped_backlog={:?} validation_errors=0",
            summary.exit_reason,
            summary.ticks_executed,
            summary.frames_presented,
            summary.frames_suspended,
            summary.frames_skipped,
            summary.dropped_backlog
        );
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_error_has_expected_code_and_os_detail() {
        let err = unsupported_platform_error("windows");
        assert_eq!(err.kind(), EngineErrorKind::UnsupportedPlatform);
        assert_eq!(err.code(), "ENGN-0002");
        assert!(
            err.detail()
                .is_some_and(|detail| detail.contains("windows"))
        );
    }
}
