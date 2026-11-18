//! Native runtime backend wiring (`Wayland` + `Vulkan 1.3` on Linux; structured
//! `ENGN-0002` diagnostic on targets without a native backend yet).

use crate::error::{EngineError, EngineErrorKind};
use crate::options::RuntimeOptions;
use crate::runtime::SessionSummary;

/// Runs the native foundation runtime with `options`, writing startup, runtime,
/// and shutdown diagnostics to `stderr` (`std` only, ADR-0009).
///
/// # Errors
///
/// Returns [`EngineError`] if the platform lacks a compiled native backend or
/// if startup, event pumping, simulation timing, rendering, or validation fails.
#[cfg(target_os = "linux")]
pub fn run_native(options: &RuntimeOptions) -> Result<SessionSummary, EngineError> {
    linux::run(options)
}

/// Runs the native foundation runtime on targets where no window/graphics
/// backend is implemented yet, returning `ENGN-0002` cleanly (`FND-INV-12`).
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
    use std::time::{Duration, Instant};

    use spawn_platform::{
        CursorGrabMode, Event, PhysicalSize, Platform, PumpMode, Window, WindowDescriptor,
        WindowEvent, WindowId,
    };
    use spawn_render::{
        ClearColor, FrameOutcome, PresentationState, Renderer, RendererDescriptor, SurfaceSize,
        ValidationMessage,
    };

    use crate::error::{EngineError, EngineErrorKind};
    use crate::options::RuntimeOptions;
    use crate::runtime::{
        RenderSurface, RuntimeBackend, SessionSummary, WindowSurface, run_session,
    };

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
            Renderer::render_clear(self, color).map_err(|err| {
                EngineError::new(EngineErrorKind::Render)
                    .with_detail("presenting clear frame")
                    .with_cause(&err)
            })
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
    }

    struct LinuxBackend {
        last_instant: Option<Instant>,
        last_report: Duration,
    }

    impl LinuxBackend {
        fn new() -> Self {
            Self {
                last_instant: None,
                last_report: Duration::ZERO,
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
                EngineError::new(EngineErrorKind::Platform)
                    .with_detail(format!("setting cursor grab to {mode:?}"))
                    .with_cause(&err)
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
                eprintln!(
                    "spawn-engine: t={:>5.1}s state={state:?}",
                    elapsed.as_secs_f64()
                );
                self.last_report = elapsed;
            }
        }
    }

    pub(super) fn run(options: &RuntimeOptions) -> Result<SessionSummary, EngineError> {
        let mut backend = LinuxBackend::new();
        let summary = run_session(options, &mut backend)?;
        eprintln!(
            "spawn-engine: exited cleanly ({:?}): ticks={} presented={} suspended={} skipped={} dropped_backlog={:?}",
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
