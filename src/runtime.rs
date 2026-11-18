//! Generic single-threaded foundation composition loop (ADR-0003,
//! `docs/foundation/DESIGN.md`).
//!
//! Coordinates `spawn-core`, `spawn-platform`, `spawn-input`, and
//! `spawn-render` through a thin backend seam so startup failure unwinding
//! (`FND-T08`), idempotent close handling (`FND-T10`), and simulation-clock
//! continuity across surface suspension/recovery (`FND-INV-06`) are verified
//! headlessly on every CI target while executing identically over Wayland and
//! Vulkan on Linux.

use std::time::Duration;

use spawn_core::{FixedStepClock, Tick};
use spawn_input::{ActionId, ButtonTrigger, InputCommand, InputMap, InputState};
use spawn_platform::{
    CursorGrabMode, Event, KeyCode, PhysicalSize, WindowDescriptor, WindowEvent, WindowId,
};
use spawn_render::{
    ClearColor, FrameOutcome, PresentationState, RendererDescriptor, SurfaceSize, ValidationMessage,
};

use crate::error::{EngineError, EngineErrorKind};
use crate::options::RuntimeOptions;

/// Bounded event-wait timeout while presentation is suspended or occluded so
/// the loop never busy-spins while remaining responsive to wakeup and close.
const IDLE_WAIT_TIMEOUT: Duration = Duration::from_millis(50);

/// Why the composition loop exited cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExitReason {
    /// The user or window system requested window close (`FND-FR-03`).
    CloseRequested,
    /// The development-only exit action (`Escape` bound in the composition
    /// root) fired on a simulation tick.
    ExitAction,
    /// `RuntimeOptions::auto_close_after` (`SPAWN_SMOKE_SECONDS`) elapsed.
    AutoClose,
}

/// Observable counters from a completed [`run_session`] execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSummary {
    /// Clean exit trigger.
    pub exit_reason: ExitReason,
    /// Total fixed simulation ticks executed (`FND-FR-07`).
    pub ticks_executed: u64,
    /// Index of the last executed simulation tick, if at least one ran.
    pub last_tick: Option<Tick>,
    /// Total excess simulation backlog dropped under catch-up clamping
    /// (`FND-FR-08`).
    pub dropped_backlog: Duration,
    /// Number of wakeups that dropped excess simulation backlog.
    pub backlog_drop_count: u64,
    /// Frames queued for presentation (`FrameOutcome::Presented`).
    pub frames_presented: u64,
    /// Frames skipped while the surface was zero-sized (`FrameOutcome::Suspended`).
    pub frames_suspended: u64,
    /// Frames skipped during transient out-of-date/surface-loss/timeout recovery
    /// (`FrameOutcome::Skipped`).
    pub frames_skipped: u64,
    /// Raw `WindowEvent::CloseRequested` events observed before shutdown.
    pub close_requests: u64,
}

/// Minimal window handle capability required by the composition loop.
pub trait WindowSurface {
    /// Stable identifier of this window for event matching and queries.
    fn id(&self) -> WindowId;
}

/// Minimal renderer capability required by the composition loop.
pub trait RenderSurface {
    /// Reports the latest physical pixel size of the drawable surface.
    fn resize(&mut self, size: SurfaceSize);

    /// Clears and presents the next frame, or suspends/recovers according to
    /// presentation policy.
    ///
    /// # Errors
    ///
    /// Returns a fatal [`EngineError`] (`ENGN-0005`) on unrecoverable device,
    /// timeout, or exhausted surface-recovery failures.
    fn render_clear(&mut self, color: ClearColor) -> Result<FrameOutcome, EngineError>;

    /// Current coarse presentation state.
    fn state(&self) -> PresentationState;

    /// Moves queued validation-layer messages into `out`, returning dropped
    /// overflow count.
    fn drain_validation_messages(&mut self, out: &mut Vec<ValidationMessage>) -> u64;

    /// Total error-severity validation messages observed since creation.
    fn validation_error_count(&self) -> u64;
}

/// Backend seam supplying monotonic elapsed time, window/event pumping, and
/// surface rendering to [`run_session`].
pub trait RuntimeBackend {
    /// Owned platform event-loop connection (dropped last).
    type Platform;
    /// Owned native top-level window (dropped before [`Self::Platform`]).
    type Window: WindowSurface;
    /// Owned renderer borrowing `&'w Self::Window` (dropped before
    /// [`Self::Window`]).
    type Renderer<'w>: RenderSurface
    where
        Self::Window: 'w;

    /// Returns monotonic time elapsed since the previous call (returning
    /// [`Duration::ZERO`] on the initial prime call at startup).
    fn read_elapsed(&mut self) -> Duration;

    /// Connects to the platform display/event loop (`FND-FR-01`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0002` or `ENGN-0004`) if unavailable.
    fn create_platform(&mut self) -> Result<Self::Platform, EngineError>;

    /// Creates one top-level window and returns it with its initial physical
    /// pixel extent (`FND-FR-01`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0004`) on failure.
    fn create_window(
        &mut self,
        platform: &mut Self::Platform,
        descriptor: &WindowDescriptor,
    ) -> Result<(Self::Window, PhysicalSize), EngineError>;

    /// Requests a cursor constraint on `window`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0004`) if unsupported or disconnected.
    fn set_cursor_grab(
        &mut self,
        platform: &mut Self::Platform,
        window: WindowId,
        mode: CursorGrabMode,
    ) -> Result<(), EngineError>;

    /// Pumps raw platform events into `events` (`None` = non-blocking poll;
    /// `Some(timeout)` = bounded wait while idle/occluded).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0004`) on connection loss.
    fn pump_events(
        &mut self,
        platform: &mut Self::Platform,
        wait_timeout: Option<Duration>,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineError>;

    /// Initializes the renderer for `window` at `size` physical pixels
    /// (`FND-FR-04`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) on failure.
    fn create_renderer<'w>(
        &mut self,
        window: &'w Self::Window,
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
    ) -> Result<Self::Renderer<'w>, EngineError>;

    /// Observer hook invoked once per executed simulation tick with the sampled
    /// [`InputCommand`].
    fn on_tick(&mut self, _command: &InputCommand) {}

    /// Diagnostic hook invoked when catch-up clamping drops `dropped` backlog.
    fn on_backlog_dropped(&mut self, _dropped: Duration) {}

    /// Diagnostic hook invoked when validation messages are drained.
    fn on_validation_messages(&mut self, _messages: &[ValidationMessage], _dropped: u64) {}

    /// Diagnostic hook invoked on notable window lifecycle events (`Resized`,
    /// `Occluded`, `ScaleFactorChanged`, `CursorGrabChanged`).
    fn on_window_event(&mut self, _window: WindowId, _event: &WindowEvent) {}

    /// Diagnostic hook invoked once per second or on shutdown with current
    /// presentation state and counters.
    fn on_frame_step(&mut self, _state: PresentationState, _elapsed: Duration) {}
}

/// Runs the foundation initialization, event/tick/render loop, and reverse-order
/// teardown (`Renderer` -> `Window` -> `Platform`) over `backend`.
///
/// # Errors
///
/// Returns a typed [`EngineError`] if option validation, subsystem startup,
/// event pumping, clock advancement, rendering, or graphics validation fails.
/// Every resource initialized prior to a failure is dropped in reverse
/// initialization order before returning (`FND-FR-10`, `FND-INV-04`).
pub fn run_session<B: RuntimeBackend>(
    options: &RuntimeOptions,
    backend: &mut B,
) -> Result<SessionSummary, EngineError> {
    let window_descriptor = WindowDescriptor::new(&options.title, options.window_size)
        .map_err(|err| {
            EngineError::new(EngineErrorKind::InvalidOption)
                .with_detail("invalid window descriptor")
                .with_cause(&err)
        })?
        .with_app_id(options.app_id.clone());

    let (mut input, exit_action) = build_development_input()?;
    let mut clock = FixedStepClock::new(options.fixed_step);

    let mut platform = backend.create_platform()?;
    let (window, initial_size) = backend.create_window(&mut platform, &window_descriptor)?;
    let window_id = window.id();

    let render_descriptor = RendererDescriptor {
        present_mode: options.present_mode,
        validation: options.validation,
    };

    let result = {
        let mut renderer = backend.create_renderer(
            &window,
            SurfaceSize::new(initial_size.width, initial_size.height),
            &render_descriptor,
        )?;
        // Anchor monotonic clock after platform, window, and renderer startup
        // so initialization latency is not treated as a simulation stall.
        let _ = backend.read_elapsed();

        let loop_result = drive_loop(
            options,
            backend,
            &mut platform,
            window_id,
            &mut renderer,
            &mut clock,
            &mut input,
            exit_action,
        );

        let mut trailing_messages = Vec::new();
        let dropped_messages = renderer.drain_validation_messages(&mut trailing_messages);
        if !trailing_messages.is_empty() || dropped_messages > 0 {
            backend.on_validation_messages(&trailing_messages, dropped_messages);
        }
        let validation_errors = renderer.validation_error_count();

        // Explicit reverse-order teardown Step 1: GPU/surface resources before
        // the borrowed window.
        drop(renderer);

        match loop_result {
            Err(error) => Err(error),
            Ok(_) if validation_errors > 0 => Err(EngineError::new(EngineErrorKind::Validation)
                .with_detail(format!(
                    "graphics validation layer reported {validation_errors} error(s)"
                ))),
            Ok(summary) => Ok(summary),
        }
    };

    // Explicit reverse-order teardown Steps 2 & 3: native window, then platform
    // display connection.
    drop(input);
    drop(window);
    drop(platform);

    result
}

#[allow(clippy::too_many_arguments)]
fn drive_loop<B: RuntimeBackend>(
    options: &RuntimeOptions,
    backend: &mut B,
    platform: &mut B::Platform,
    window_id: WindowId,
    renderer: &mut B::Renderer<'_>,
    clock: &mut FixedStepClock,
    input: &mut InputState,
    exit_action: ActionId,
) -> Result<SessionSummary, EngineError> {
    if let Some(mode) = options.smoke_cursor_grab {
        backend.set_cursor_grab(platform, window_id, mode)?;
    }

    let mut summary = SessionSummary {
        exit_reason: ExitReason::CloseRequested,
        ticks_executed: 0,
        last_tick: None,
        dropped_backlog: Duration::ZERO,
        backlog_drop_count: 0,
        frames_presented: 0,
        frames_suspended: 0,
        frames_skipped: 0,
        close_requests: 0,
    };

    let mut total_elapsed = Duration::ZERO;
    let mut smoke_grab_released = options
        .smoke_cursor_grab
        .is_none_or(|mode| mode == CursorGrabMode::None);
    let mut occluded = false;
    let mut idle = renderer.state() == PresentationState::Suspended;
    let mut exit_reason: Option<ExitReason> = None;
    let mut events = Vec::new();
    let mut validation_messages = Vec::new();

    loop {
        events.clear();
        let wait_timeout = (idle || occluded).then_some(IDLE_WAIT_TIMEOUT);
        backend.pump_events(platform, wait_timeout, &mut events)?;

        for raw_event in &events {
            match raw_event {
                Event::Window { window, event } => {
                    if *window != window_id {
                        continue;
                    }
                    input.handle_event(raw_event);
                    backend.on_window_event(*window, event);
                    match event {
                        WindowEvent::CloseRequested => {
                            summary.close_requests += 1;
                            exit_reason.get_or_insert(ExitReason::CloseRequested);
                        }
                        WindowEvent::Resized(size) => {
                            renderer.resize(SurfaceSize::new(size.width, size.height));
                        }
                        WindowEvent::Occluded(hidden) => {
                            occluded = *hidden;
                        }
                        _ => {}
                    }
                }
                _ => input.handle_event(raw_event),
            }
        }

        let elapsed = backend.read_elapsed();
        total_elapsed = total_elapsed.saturating_add(elapsed);
        let update = clock.advance(elapsed).map_err(|err| {
            EngineError::new(EngineErrorKind::Clock)
                .with_detail("advancing fixed-step clock")
                .with_cause(&err)
        })?;
        if update.dropped() > Duration::ZERO {
            summary.dropped_backlog = summary.dropped_backlog.saturating_add(update.dropped());
            summary.backlog_drop_count += 1;
            backend.on_backlog_dropped(update.dropped());
        }
        for tick in update.ticks() {
            let command = input.sample_tick(tick);
            summary.ticks_executed += 1;
            summary.last_tick = Some(tick);
            backend.on_tick(&command);
            if command.is_pressed(exit_action) {
                exit_reason.get_or_insert(ExitReason::ExitAction);
            }
        }

        if let Some(reason) = exit_reason {
            summary.exit_reason = reason;
            break;
        }

        if !occluded {
            match renderer.render_clear(options.clear_color)? {
                FrameOutcome::Presented => {
                    summary.frames_presented += 1;
                    idle = false;
                }
                FrameOutcome::Suspended => {
                    summary.frames_suspended += 1;
                    idle = true;
                }
                FrameOutcome::Skipped => {
                    summary.frames_skipped += 1;
                    idle = false;
                }
            }
        }

        validation_messages.clear();
        let dropped = renderer.drain_validation_messages(&mut validation_messages);
        if !validation_messages.is_empty() || dropped > 0 {
            backend.on_validation_messages(&validation_messages, dropped);
        }
        let validation_errors = renderer.validation_error_count();
        if validation_errors > 0 {
            return Err(
                EngineError::new(EngineErrorKind::Validation).with_detail(format!(
                    "graphics validation layer reported {validation_errors} error(s)"
                )),
            );
        }

        backend.on_frame_step(renderer.state(), total_elapsed);

        if !smoke_grab_released
            && options
                .auto_close_after
                .is_some_and(|limit| total_elapsed >= limit / 2)
        {
            backend.set_cursor_grab(platform, window_id, CursorGrabMode::None)?;
            smoke_grab_released = true;
        }

        if options
            .auto_close_after
            .is_some_and(|limit| total_elapsed >= limit)
        {
            summary.exit_reason = ExitReason::AutoClose;
            break;
        }
    }

    Ok(summary)
}

/// Binds the development-only `Escape` exit intent owned by the composition
/// root (`docs/foundation/DESIGN.md` Event and focus policy). `spawn-input`
/// itself stays completely free of default actions.
fn build_development_input() -> Result<(InputState, ActionId), EngineError> {
    let exit_action = ActionId::new(0).map_err(|err| {
        EngineError::new(EngineErrorKind::Input)
            .with_detail("allocating composition exit action")
            .with_cause(&err)
    })?;
    let mut map = InputMap::new();
    map.bind_action(exit_action, ButtonTrigger::Key(KeyCode::Escape))
        .map_err(|err| {
            EngineError::new(EngineErrorKind::Input)
                .with_detail("binding Escape to composition exit action")
                .with_cause(&err)
        })?;
    Ok((InputState::with_map(map), exit_action))
}
