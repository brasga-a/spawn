//! End-to-end headless verification tests for Foundation Phases 5 & 6
//! (`docs/foundation/VERIFICATION.md`: `FND-T08`, `FND-T09`, `FND-T10`,
//! `FND-INV-05`, `FND-INV-06`, `FND-INV-08`).

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

#[path = "common/graph.rs"]
mod graph;

use spawn_core::{FixedStepConfig, Tick};
use spawn_engine::{
    EngineError, EngineErrorKind, ExitReason, RenderSurface, RuntimeBackend, RuntimeOptions,
    WindowSurface, run_session,
};
use spawn_input::InputCommand;
use spawn_platform::{
    CursorGrabMode, ElementState, Event, KeyCode, KeyEvent, PhysicalSize, WindowDescriptor,
    WindowEvent, WindowId,
};
use spawn_render::{
    CameraRelativeView, ClearColor, CookedShader, FrameOutcome, GpuAllocatorStats,
    GraphicsPipelineDescriptor, MeshDescriptor, MeshDraw, MeshHandle, PipelineHandle,
    PresentationState, RendererDescriptor, ShaderHandle, ShaderStage, SurfaceSize,
    ValidationMessage, ValidationSeverity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum LifecycleEvent {
    CreatePlatform,
    DropPlatform,
    CreateWindow(WindowId),
    DropWindow(WindowId),
    CreateRenderer(SurfaceSize),
    DropRenderer,
    SetCursorGrab(CursorGrabMode),
}

struct MockPlatform {
    log: Rc<RefCell<Vec<LifecycleEvent>>>,
}

impl Drop for MockPlatform {
    fn drop(&mut self) {
        self.log.borrow_mut().push(LifecycleEvent::DropPlatform);
    }
}

struct MockWindow {
    id: WindowId,
    log: Rc<RefCell<Vec<LifecycleEvent>>>,
}

impl WindowSurface for MockWindow {
    fn id(&self) -> WindowId {
        self.id
    }
}

impl Drop for MockWindow {
    fn drop(&mut self) {
        self.log
            .borrow_mut()
            .push(LifecycleEvent::DropWindow(self.id));
    }
}

struct MockRenderer<'w> {
    _window: &'w MockWindow,
    size: SurfaceSize,
    state: PresentationState,
    scripted_outcomes: VecDeque<Result<FrameOutcome, EngineError>>,
    validation_messages: Vec<ValidationMessage>,
    validation_errors: u64,
    acquire_attempts_while_empty: u64,
    log: Rc<RefCell<Vec<LifecycleEvent>>>,
}

impl RenderSurface for MockRenderer<'_> {
    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
        self.state = if size.is_empty() {
            PresentationState::Suspended
        } else {
            PresentationState::Ready
        };
    }

    fn render_clear(&mut self, _color: ClearColor) -> Result<FrameOutcome, EngineError> {
        if self.size.is_empty() {
            self.state = PresentationState::Suspended;
            return Ok(FrameOutcome::Suspended);
        }
        if let Some(next) = self.scripted_outcomes.pop_front() {
            match &next {
                Ok(FrameOutcome::Presented) => self.state = PresentationState::Ready,
                Ok(FrameOutcome::Suspended) => {
                    self.acquire_attempts_while_empty += 1;
                    self.state = PresentationState::Suspended;
                }
                Ok(FrameOutcome::Skipped) => self.state = PresentationState::Recovering,
                Err(_) => self.state = PresentationState::Fatal,
            }
            return next;
        }
        self.state = PresentationState::Ready;
        Ok(FrameOutcome::Presented)
    }

    fn state(&self) -> PresentationState {
        self.state
    }

    fn drain_validation_messages(&mut self, out: &mut Vec<ValidationMessage>) -> u64 {
        out.append(&mut self.validation_messages);
        0
    }

    fn validation_error_count(&self) -> u64 {
        self.validation_errors
    }

    fn load_shader(
        &mut self,
        label: &str,
        cooked_bytes: &[u8],
    ) -> Result<ShaderHandle, EngineError> {
        let cooked = CookedShader::decode_named(label, cooked_bytes).map_err(|err| {
            EngineError::new(EngineErrorKind::Shader)
                .with_detail(format!("loading cooked shader `{label}`"))
                .with_cause(&err)
        })?;
        let slot = match cooked.stage {
            ShaderStage::Vertex => 0,
            ShaderStage::Fragment => 1,
            ShaderStage::Compute => 2,
        };
        Ok(ShaderHandle::from_raw(slot, 1))
    }

    fn destroy_shader(&mut self, _handle: ShaderHandle) -> Result<(), EngineError> {
        Ok(())
    }

    fn create_graphics_pipeline(
        &mut self,
        descriptor: &GraphicsPipelineDescriptor<'_>,
    ) -> Result<PipelineHandle, EngineError> {
        if descriptor.vertex_shader == descriptor.fragment_shader {
            return Err(EngineError::new(EngineErrorKind::Shader).with_detail(
                "vertex_shader and fragment_shader cannot reference the same ShaderHandle",
            ));
        }
        Ok(PipelineHandle::from_raw(0, 1))
    }

    fn destroy_pipeline(&mut self, _handle: PipelineHandle) -> Result<(), EngineError> {
        Ok(())
    }

    fn upload_mesh(&mut self, descriptor: &MeshDescriptor<'_>) -> Result<MeshHandle, EngineError> {
        descriptor.validate().map_err(|err| {
            EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("uploading mesh `{}`", descriptor.label))
                .with_cause(&err)
        })?;
        Ok(MeshHandle::from_raw(0, 1))
    }

    fn destroy_mesh(&mut self, _handle: MeshHandle) -> Result<(), EngineError> {
        Ok(())
    }

    fn render_meshes(
        &mut self,
        _pipeline: PipelineHandle,
        clear_color: ClearColor,
        view: &CameraRelativeView,
        draws: &[MeshDraw],
    ) -> Result<FrameOutcome, EngineError> {
        for draw in draws {
            draw.pack_push_constants(view).map_err(|err| {
                EngineError::new(EngineErrorKind::Render)
                    .with_detail("packing camera-relative 3D mesh push constants")
                    .with_cause(&err)
            })?;
        }
        self.render_clear(clear_color)
    }

    fn allocator_stats(&self) -> GpuAllocatorStats {
        GpuAllocatorStats::default()
    }

    fn close(self) -> Result<GpuAllocatorStats, EngineError> {
        Ok(self.allocator_stats())
    }
}

impl Drop for MockRenderer<'_> {
    fn drop(&mut self) {
        self.log.borrow_mut().push(LifecycleEvent::DropRenderer);
    }
}

#[derive(Default)]
struct ScriptStep {
    elapsed: Duration,
    events: Vec<Event>,
    pump_error: Option<EngineErrorKind>,
}

struct MockBackend {
    log: Rc<RefCell<Vec<LifecycleEvent>>>,
    window_id: WindowId,
    initial_size: PhysicalSize,
    fail_platform: bool,
    fail_window: bool,
    fail_renderer: bool,
    fail_cursor_grab: bool,
    primed: bool,
    steps: VecDeque<ScriptStep>,
    wait_timeouts: Vec<Option<Duration>>,
    renderer_outcomes: VecDeque<Result<FrameOutcome, EngineError>>,
    renderer_validation_messages: Vec<ValidationMessage>,
    renderer_validation_errors: u64,
    sampled_ticks: Vec<u64>,
    observed_commands: Vec<InputCommand>,
    drained_messages: Vec<ValidationMessage>,
    backlog_drops: Vec<Duration>,
}

impl MockBackend {
    fn new(log: Rc<RefCell<Vec<LifecycleEvent>>>) -> Self {
        Self {
            log,
            window_id: WindowId::from_raw(1),
            initial_size: PhysicalSize::new(960, 540),
            fail_platform: false,
            fail_window: false,
            fail_renderer: false,
            fail_cursor_grab: false,
            primed: false,
            steps: VecDeque::new(),
            wait_timeouts: Vec::new(),
            renderer_outcomes: VecDeque::new(),
            renderer_validation_messages: Vec::new(),
            renderer_validation_errors: 0,
            sampled_ticks: Vec::new(),
            observed_commands: Vec::new(),
            drained_messages: Vec::new(),
            backlog_drops: Vec::new(),
        }
    }
}

impl RuntimeBackend for MockBackend {
    type Platform = MockPlatform;
    type Window = MockWindow;
    type Renderer<'w> = MockRenderer<'w>;

    fn read_elapsed(&mut self) -> Duration {
        if !self.primed {
            self.primed = true;
            return Duration::ZERO;
        }
        self.steps
            .front()
            .map_or(Duration::from_millis(20), |step| step.elapsed)
    }

    fn create_platform(&mut self) -> Result<Self::Platform, EngineError> {
        if self.fail_platform {
            return Err(EngineError::new(EngineErrorKind::Platform)
                .with_detail("injected platform failure"));
        }
        self.log.borrow_mut().push(LifecycleEvent::CreatePlatform);
        Ok(MockPlatform {
            log: Rc::clone(&self.log),
        })
    }

    fn create_window(
        &mut self,
        _platform: &mut Self::Platform,
        _descriptor: &WindowDescriptor,
    ) -> Result<(Self::Window, PhysicalSize), EngineError> {
        if self.fail_window {
            return Err(
                EngineError::new(EngineErrorKind::Platform).with_detail("injected window failure")
            );
        }
        self.log
            .borrow_mut()
            .push(LifecycleEvent::CreateWindow(self.window_id));
        Ok((
            MockWindow {
                id: self.window_id,
                log: Rc::clone(&self.log),
            },
            self.initial_size,
        ))
    }

    fn set_cursor_grab(
        &mut self,
        _platform: &mut Self::Platform,
        _window: WindowId,
        mode: CursorGrabMode,
    ) -> Result<(), EngineError> {
        if self.fail_cursor_grab {
            return Err(EngineError::new(EngineErrorKind::Platform)
                .with_detail("injected cursor grab failure"));
        }
        self.log
            .borrow_mut()
            .push(LifecycleEvent::SetCursorGrab(mode));
        Ok(())
    }

    fn pump_events(
        &mut self,
        _platform: &mut Self::Platform,
        wait_timeout: Option<Duration>,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineError> {
        self.wait_timeouts.push(wait_timeout);
        if let Some(step) = self.steps.front() {
            if let Some(kind) = step.pump_error {
                let _ = self.steps.pop_front();
                return Err(EngineError::new(kind).with_detail("injected pump failure"));
            }
            events.extend_from_slice(&step.events);
        }
        Ok(())
    }

    fn create_renderer<'w>(
        &mut self,
        window: &'w Self::Window,
        size: SurfaceSize,
        _descriptor: &RendererDescriptor,
    ) -> Result<Self::Renderer<'w>, EngineError> {
        if self.fail_renderer {
            return Err(
                EngineError::new(EngineErrorKind::Render).with_detail("injected renderer failure")
            );
        }
        self.log
            .borrow_mut()
            .push(LifecycleEvent::CreateRenderer(size));
        Ok(MockRenderer {
            _window: window,
            size,
            state: if size.is_empty() {
                PresentationState::Suspended
            } else {
                PresentationState::Ready
            },
            scripted_outcomes: std::mem::take(&mut self.renderer_outcomes),
            validation_messages: std::mem::take(&mut self.renderer_validation_messages),
            validation_errors: self.renderer_validation_errors,
            acquire_attempts_while_empty: 0,
            log: Rc::clone(&self.log),
        })
    }

    fn read_cooked_shader(
        &mut self,
        shader_dir: &Path,
        artifact_name: &str,
    ) -> Result<Vec<u8>, EngineError> {
        spawn_engine::synthesize_minimal_mesh_3d_shader(shader_dir, artifact_name)
    }

    fn on_tick(&mut self, command: &InputCommand) {
        self.sampled_ticks.push(command.tick().get());
        self.observed_commands.push(*command);
    }

    fn on_backlog_dropped(&mut self, dropped: Duration) {
        self.backlog_drops.push(dropped);
    }

    fn on_validation_messages(&mut self, messages: &[ValidationMessage], _dropped: u64) {
        self.drained_messages.extend_from_slice(messages);
    }

    fn on_frame_step(&mut self, _state: PresentationState, _elapsed: Duration) {
        let _ = self.steps.pop_front();
    }
}

/// Adjusts `MockBackend::read_elapsed` / `pump_events` step consumption cleanly
/// when a step exits before `on_frame_step` (e.g. `CloseRequested` or `Escape`).
struct StepQueueBackend {
    inner: MockBackend,
    current_elapsed: Duration,
}

impl StepQueueBackend {
    fn new(inner: MockBackend) -> Self {
        Self {
            inner,
            current_elapsed: Duration::ZERO,
        }
    }
}

impl RuntimeBackend for StepQueueBackend {
    type Platform = MockPlatform;
    type Window = MockWindow;
    type Renderer<'w> = MockRenderer<'w>;

    fn read_elapsed(&mut self) -> Duration {
        if !self.inner.primed {
            self.inner.primed = true;
            return Duration::ZERO;
        }
        self.current_elapsed
    }

    fn create_platform(&mut self) -> Result<Self::Platform, EngineError> {
        self.inner.create_platform()
    }

    fn create_window(
        &mut self,
        platform: &mut Self::Platform,
        descriptor: &WindowDescriptor,
    ) -> Result<(Self::Window, PhysicalSize), EngineError> {
        self.inner.create_window(platform, descriptor)
    }

    fn set_cursor_grab(
        &mut self,
        platform: &mut Self::Platform,
        window: WindowId,
        mode: CursorGrabMode,
    ) -> Result<(), EngineError> {
        self.inner.set_cursor_grab(platform, window, mode)
    }

    fn pump_events(
        &mut self,
        _platform: &mut Self::Platform,
        wait_timeout: Option<Duration>,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineError> {
        self.inner.wait_timeouts.push(wait_timeout);
        if let Some(step) = self.inner.steps.pop_front() {
            self.current_elapsed = step.elapsed;
            if let Some(kind) = step.pump_error {
                return Err(EngineError::new(kind).with_detail("injected pump failure"));
            }
            events.extend(step.events);
        } else {
            self.current_elapsed = Duration::from_millis(20);
            events.push(Event::Window {
                window: self.inner.window_id,
                event: WindowEvent::CloseRequested,
            });
        }
        Ok(())
    }

    fn create_renderer<'w>(
        &mut self,
        window: &'w Self::Window,
        size: SurfaceSize,
        descriptor: &RendererDescriptor,
    ) -> Result<Self::Renderer<'w>, EngineError> {
        self.inner.create_renderer(window, size, descriptor)
    }

    fn read_cooked_shader(
        &mut self,
        shader_dir: &Path,
        artifact_name: &str,
    ) -> Result<Vec<u8>, EngineError> {
        self.inner.read_cooked_shader(shader_dir, artifact_name)
    }

    fn on_tick(&mut self, command: &InputCommand) {
        self.inner.on_tick(command);
    }

    fn on_backlog_dropped(&mut self, dropped: Duration) {
        self.inner.on_backlog_dropped(dropped);
    }

    fn on_validation_messages(&mut self, messages: &[ValidationMessage], dropped: u64) {
        self.inner.on_validation_messages(messages, dropped);
    }
}

/// FND-T08 (FR-10 / INV-04, 12): fail successive startup and runtime steps;
/// verify every previously initialized resource is released once in exact
/// reverse ownership order (`Renderer` -> `Window` -> `Platform`), with no
/// panic.
#[test]
fn fnd_t08_partial_startup_and_fatal_runtime_unwind_in_reverse_order() {
    let win = WindowId::from_raw(1);
    let size = SurfaceSize::new(960, 540);

    // 1. Invalid window option (zero logical width) fails before any native subsystem starts.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut backend = StepQueueBackend::new(MockBackend::new(Rc::clone(&log)));
        let options = RuntimeOptions {
            window_size: spawn_platform::LogicalSize::new(0, 540),
            ..RuntimeOptions::default()
        };
        let err = run_session(&options, &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::InvalidOption);
        assert!(log.borrow().is_empty());
    }

    // 2. Platform connection failure leaves zero leaked resources.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.fail_platform = true;
        let mut backend = StepQueueBackend::new(mock);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Platform);
        assert!(log.borrow().is_empty());
    }

    // 3. Window creation failure drops Platform once.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.fail_window = true;
        let mut backend = StepQueueBackend::new(mock);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Platform);
        assert_eq!(
            *log.borrow(),
            vec![LifecycleEvent::CreatePlatform, LifecycleEvent::DropPlatform]
        );
    }

    // 4. Renderer creation failure drops Window then Platform.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.fail_renderer = true;
        let mut backend = StepQueueBackend::new(mock);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Render);
        assert_eq!(
            *log.borrow(),
            vec![
                LifecycleEvent::CreatePlatform,
                LifecycleEvent::CreateWindow(win),
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }

    // 5. Post-renderer startup failure (cursor grab) drops Renderer -> Window -> Platform.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.fail_cursor_grab = true;
        let mut backend = StepQueueBackend::new(mock);
        let options = RuntimeOptions {
            smoke_cursor_grab: Some(CursorGrabMode::Locked),
            ..RuntimeOptions::default()
        };
        let err = run_session(&options, &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Platform);
        assert_eq!(
            *log.borrow(),
            vec![
                LifecycleEvent::CreatePlatform,
                LifecycleEvent::CreateWindow(win),
                LifecycleEvent::CreateRenderer(size),
                LifecycleEvent::DropRenderer,
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }

    // 6. Mid-loop fatal render error (e.g. device lost) drains validation
    //    diagnostics and drops Renderer -> Window -> Platform.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(20),
            events: Vec::new(),
            pump_error: None,
        });
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(20),
            events: Vec::new(),
            pump_error: None,
        });
        mock.renderer_outcomes
            .push_back(Ok(FrameOutcome::Presented));
        mock.renderer_outcomes.push_back(Err(
            EngineError::new(EngineErrorKind::Render).with_detail("simulated device loss")
        ));
        mock.renderer_validation_messages.push(ValidationMessage {
            severity: ValidationSeverity::Warning,
            text: "diagnostic before device loss".to_owned(),
        });
        let mut backend = StepQueueBackend::new(mock);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Render);
        assert_eq!(backend.inner.drained_messages.len(), 1);
        assert_eq!(
            *log.borrow(),
            vec![
                LifecycleEvent::CreatePlatform,
                LifecycleEvent::CreateWindow(win),
                LifecycleEvent::CreateRenderer(size),
                LifecycleEvent::DropRenderer,
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }

    // 7. Validation-layer error fails the session (`SPN-INV-092`) and unwinds cleanly.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(20),
            events: Vec::new(),
            pump_error: None,
        });
        mock.renderer_validation_errors = 1;
        let mut backend = StepQueueBackend::new(mock);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Validation);
        assert_eq!(err.code(), "ENGN-0006");
        assert_eq!(
            *log.borrow(),
            vec![
                LifecycleEvent::CreatePlatform,
                LifecycleEvent::CreateWindow(win),
                LifecycleEvent::CreateRenderer(size),
                LifecycleEvent::DropRenderer,
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }
}

/// FND-T10 (FR-03 / INV-04, 12): repeated close requests in a single batch and
/// across frames are handled idempotently and shut down in reverse order; the
/// composition root's development `Escape` binding also exits cleanly on tick.
#[test]
fn fnd_t10_repeated_close_requests_and_escape_action_exit_idempotently() {
    let win = WindowId::from_raw(1);
    let other_win = WindowId::from_raw(99);
    let size = SurfaceSize::new(960, 540);

    // Case A: Frame 0 renders normally (ignoring both Escape and CloseRequested
    // for a foreign `other_win` window); Frame 1 delivers three duplicate
    // CloseRequested events on `win` in one batch.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(20),
            events: vec![
                Event::Window {
                    window: other_win,
                    event: WindowEvent::Key(KeyEvent {
                        key: KeyCode::Escape,
                        native_code: 1,
                        state: ElementState::Pressed,
                    }),
                },
                Event::Window {
                    window: other_win,
                    event: WindowEvent::CloseRequested,
                },
            ],
            pump_error: None,
        });
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(20),
            events: vec![
                Event::Window {
                    window: win,
                    event: WindowEvent::CloseRequested,
                },
                Event::Window {
                    window: win,
                    event: WindowEvent::CloseRequested,
                },
                Event::Window {
                    window: win,
                    event: WindowEvent::CloseRequested,
                },
            ],
            pump_error: None,
        });
        let mut backend = StepQueueBackend::new(mock);
        let summary = run_session(&RuntimeOptions::default(), &mut backend).unwrap();
        assert_eq!(summary.exit_reason, ExitReason::CloseRequested);
        assert_eq!(summary.close_requests, 3);
        assert_eq!(summary.frames_presented, 1);
        assert_eq!(summary.ticks_executed, 2);
        assert_eq!(
            *log.borrow(),
            vec![
                LifecycleEvent::CreatePlatform,
                LifecycleEvent::CreateWindow(win),
                LifecycleEvent::CreateRenderer(size),
                LifecycleEvent::DropRenderer,
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }

    // Case B: Sub-tick Escape press on a 0-tick frame (5 ms) is preserved by
    // `spawn-input` until the next tick (15 ms) and exits cleanly via ExitAction.
    {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut mock = MockBackend::new(Rc::clone(&log));
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(5),
            events: vec![Event::Window {
                window: win,
                event: WindowEvent::Key(KeyEvent {
                    key: KeyCode::Escape,
                    native_code: 1,
                    state: ElementState::Pressed,
                }),
            }],
            pump_error: None,
        });
        mock.steps.push_back(ScriptStep {
            elapsed: Duration::from_millis(15),
            events: Vec::new(),
            pump_error: None,
        });
        let mut backend = StepQueueBackend::new(mock);
        let summary = run_session(&RuntimeOptions::default(), &mut backend).unwrap();
        assert_eq!(summary.exit_reason, ExitReason::ExitAction);
        assert_eq!(summary.frames_presented, 1);
        assert_eq!(summary.ticks_executed, 1);
        assert_eq!(summary.last_tick, Some(Tick::ZERO));
    }
}

/// FND-INV-05, FND-INV-06, FND-INV-08: zero-size suspension, occlusion,
/// recoverable surface skips, resize, and clamped catch-up backlog preserve
/// strictly monotonic simulation `Tick` indices without resetting simulation
/// time or busy-spinning while suspended.
#[test]
fn fnd_inv_06_resize_suspend_and_surface_recovery_preserve_simulation_clock() {
    let win = WindowId::from_raw(1);
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut mock = MockBackend::new(Rc::clone(&log));

    // Exact 10 ms fixed step (100 Hz), max 4 catch-up ticks per wakeup.
    let options = RuntimeOptions {
        fixed_step: FixedStepConfig::new(Duration::from_millis(10), 4).unwrap(),
        ..RuntimeOptions::default()
    };

    // Step 0: drawable 960x540, 10 ms -> Tick 0, Presented (Poll).
    mock.steps.push_back(ScriptStep {
        elapsed: Duration::from_millis(10),
        events: Vec::new(),
        pump_error: None,
    });
    // Step 1: resize to 0x0 (minimized), 10 ms -> Tick 1, Suspended (no GPU acquire).
    mock.steps.push_back(ScriptStep {
        elapsed: Duration::from_millis(10),
        events: vec![Event::Window {
            window: win,
            event: WindowEvent::Resized(PhysicalSize::new(0, 0)),
        }],
        pump_error: None,
    });
    // Step 2: still 0x0 -> loop uses bounded WaitTimeout (not Poll!), 10 ms -> Tick 2, Suspended.
    mock.steps.push_back(ScriptStep {
        elapsed: Duration::from_millis(10),
        events: Vec::new(),
        pump_error: None,
    });
    // Step 3: resize to 1280x720 + transient surface out-of-date (`Skipped`), 20 ms -> Ticks 3, 4.
    mock.steps.push_back(ScriptStep {
        elapsed: Duration::from_millis(20),
        events: vec![Event::Window {
            window: win,
            event: WindowEvent::Resized(PhysicalSize::new(1280, 720)),
        }],
        pump_error: None,
    });
    // Step 4: recovered `Presented` + 100 ms stall -> clamped to 4 ticks (Ticks 5..=8), 60 ms dropped.
    mock.steps.push_back(ScriptStep {
        elapsed: Duration::from_millis(100),
        events: Vec::new(),
        pump_error: None,
    });
    // Step 5: CloseRequested + 10 ms -> Tick 9, orderly exit.
    mock.steps.push_back(ScriptStep {
        elapsed: Duration::from_millis(10),
        events: vec![Event::Window {
            window: win,
            event: WindowEvent::CloseRequested,
        }],
        pump_error: None,
    });

    // Scripted non-empty render outcomes (Step 0 = Presented, Step 3 = Skipped, Step 4 = Presented).
    mock.renderer_outcomes
        .push_back(Ok(FrameOutcome::Presented));
    mock.renderer_outcomes.push_back(Ok(FrameOutcome::Skipped));
    mock.renderer_outcomes
        .push_back(Ok(FrameOutcome::Presented));

    let mut backend = StepQueueBackend::new(mock);
    let summary = run_session(&options, &mut backend).unwrap();

    assert_eq!(summary.exit_reason, ExitReason::CloseRequested);
    assert_eq!(summary.frames_presented, 2);
    assert_eq!(summary.frames_suspended, 2);
    assert_eq!(summary.frames_skipped, 1);
    assert_eq!(summary.ticks_executed, 10);
    assert_eq!(summary.last_tick, Some(Tick::new(9)));
    assert_eq!(summary.dropped_backlog, Duration::from_millis(60));
    assert_eq!(summary.backlog_drop_count, 1);

    let expected_ticks: Vec<u64> = (0..10).collect();
    assert_eq!(backend.inner.sampled_ticks, expected_ticks);

    // Step 2 (after Step 1 entered Suspended) waited with a bounded timeout instead of spinning.
    assert_eq!(backend.inner.wait_timeouts[0], None);
    assert_eq!(backend.inner.wait_timeouts[1], None);
    assert!(backend.inner.wait_timeouts[2].is_some());
}

/// FND-T09 (NFR-01, 02 / INV-01, 02, 03, 13, 14; SPN-INV-001..006, 009, 011):
/// inspects workspace manifests, `cargo metadata`, `cargo tree`, and public API
/// declarations across foundation crates.
#[test]
fn fnd_t09_dependency_graph_headless_hygiene_and_genre_free_public_api()
-> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 1. `cargo metadata --no-deps` succeeds cleanly.
    let metadata_output = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()?;
    assert!(
        metadata_output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&metadata_output.stderr)
    );

    // 2. Resolve every workspace package's direct `spawn-*` edges through Cargo's
    //    own resolver (`cargo tree --invert`; normal + build, all targets), so
    //    table-header, renamed, build-only, and target-specific dependencies are
    //    all covered (SPN-INV-001, SPN-INV-002).
    let graph = graph::resolved_workspace_graph(&root)?;
    let edges = &graph.edges;

    // Engine crates never depend on root `spawn-engine` or any `tools/*` crate (SPN-INV-001).
    graph::assert_no_upward_edges(&graph)?;

    // Specific foundation boundary invariants (FND-INV-01, FND-INV-02, SPN-INV-004..006).
    assert!(
        edges["spawn-core"].is_empty(),
        "spawn-core must depend on no spawn-* crate: {:?}",
        edges["spawn-core"]
    );
    assert!(
        edges["spawn-platform"].is_empty(),
        "spawn-platform must depend on no spawn-* crate: {:?}",
        edges["spawn-platform"]
    );
    assert!(
        edges["spawn-math"].is_empty(),
        "spawn-math must depend on no spawn-* crate: {:?}",
        edges["spawn-math"]
    );
    assert_eq!(
        edges["spawn-render"],
        BTreeSet::from(["spawn-math".to_owned()]),
        "spawn-render must depend only on spawn-math (never spawn-platform or spawn-input): {:?}",
        edges["spawn-render"]
    );
    assert_eq!(
        edges["spawn-input"],
        BTreeSet::from(["spawn-core".to_owned(), "spawn-platform".to_owned()]),
        "spawn-input must depend only on spawn-core and spawn-platform"
    );

    // Topological sort (Kahn's algorithm) proves the entire workspace graph is acyclic (SPN-INV-002).
    graph::assert_acyclic(edges);

    // 3. `cargo tree -e normal` verifies headless crates pull zero window/GPU backends (FND-INV-03, SPN-INV-009).
    let core_tree = cargo_tree(&root, &["-p", "spawn-core", "-e", "normal"])?;
    assert_eq!(
        core_tree.lines().count(),
        1,
        "spawn-core must have zero normal dependencies:\n{core_tree}"
    );
    assert_eq!(
        parse_cargo_tree_crates(&core_tree),
        BTreeSet::from(["spawn-core".to_owned()]),
        "spawn-core must have zero normal dependencies:\n{core_tree}"
    );

    let platform_headless_tree = cargo_tree(&root, &["-p", "spawn-platform", "-e", "normal"])?;
    assert_eq!(
        platform_headless_tree.lines().count(),
        1,
        "spawn-platform default features must have zero normal dependencies:\n{platform_headless_tree}"
    );
    assert_eq!(
        parse_cargo_tree_crates(&platform_headless_tree),
        BTreeSet::from(["spawn-platform".to_owned()]),
        "spawn-platform default features must have zero normal dependencies:\n{platform_headless_tree}"
    );

    let render_headless_tree = cargo_tree(&root, &["-p", "spawn-render", "-e", "normal"])?;
    let render_headless_crates = parse_cargo_tree_crates(&render_headless_tree);
    assert_eq!(
        render_headless_crates,
        BTreeSet::from([
            "spawn-render".to_owned(),
            "spawn-math".to_owned(),
            "libm".to_owned(),
        ]),
        "spawn-render default features must pull only pure-Rust spawn-math -> libm (zero window/GPU backends):\n{render_headless_tree}"
    );

    let input_tree = cargo_tree(&root, &["-p", "spawn-input", "-e", "normal"])?;
    check_tree_forbids(
        &input_tree,
        "spawn-input normal tree",
        &[
            "ash",
            "wayland-client",
            "wayland-protocols",
            "rustix",
            "raw-window-handle",
            "spawn-render",
        ],
    )?;

    let render_vulkan_tree = cargo_tree(
        &root,
        &["-p", "spawn-render", "--features", "vulkan", "-e", "normal"],
    )?;
    check_tree_forbids(
        &render_vulkan_tree,
        "spawn-render --features vulkan",
        &["spawn-platform"],
    )?;

    if cfg!(target_os = "linux") {
        let platform_wayland_tree = cargo_tree(
            &root,
            &[
                "-p",
                "spawn-platform",
                "--features",
                "wayland",
                "-e",
                "normal",
            ],
        )?;
        check_tree_forbids(
            &platform_wayland_tree,
            "spawn-platform --features wayland",
            &["spawn-render", "spawn-input", "ash"],
        )?;
    }

    // 4. Audit public API declarations across foundation crates for genre terms
    //    (FND-NFR-01, FND-INV-13, SPN-INV-003) and leaked native/GPU backend
    //    types (FND-INV-14, SPN-INV-011).
    for dir in [
        "crates/spawn-core/src",
        "crates/spawn-platform/src",
        "crates/spawn-input/src",
        "crates/spawn-render/src",
        "src",
    ] {
        audit_public_api_dir(&root.join(dir))?;
    }
    Ok(())
}

fn cargo_tree(root: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args(["tree", "--color", "never"])
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "cargo tree {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        !stdout.contains('\x1b'),
        "cargo tree {args:?} emitted unexpected ANSI escape sequences:\n{stdout}"
    );
    Ok(stdout)
}

fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for seq_ch in chars.by_ref() {
                    if ('@'..='~').contains(&seq_ch) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_cargo_tree_crates(tree: &str) -> BTreeSet<String> {
    let clean = strip_ansi_escapes(tree);
    clean
        .lines()
        .filter_map(|line| {
            let trimmed =
                line.trim_start_matches([' ', '│', '├', '└', '─', '┬', '|', '+', '`', '\\', '-']);
            if trimmed.is_empty() || trimmed.starts_with('[') {
                return None;
            }
            trimmed.split_whitespace().next().map(str::to_owned)
        })
        .collect()
}

fn check_tree_forbids(tree: &str, context: &str, forbidden: &[&str]) -> Result<(), String> {
    let crates = parse_cargo_tree_crates(tree);
    for &dep in forbidden {
        if crates.contains(dep) {
            return Err(format!("{context} must not contain `{dep}`:\n{tree}"));
        }
    }
    Ok(())
}

#[test]
fn cargo_tree_parser_handles_ansi_color_and_detects_forbidden_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 1. Even if `CARGO_TERM_COLOR=always` is set in the environment, `--color never`
    //    must suppress ANSI escapes from `cargo tree`.
    let forced_color_out = Command::new(env!("CARGO"))
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "always")
        .args([
            "tree",
            "--color",
            "never",
            "-p",
            "spawn-render",
            "-e",
            "normal",
        ])
        .output()?;
    assert!(forced_color_out.status.success());
    let forced_stdout = String::from_utf8(forced_color_out.stdout)?;
    assert!(!forced_stdout.contains('\x1b'));
    assert_eq!(
        parse_cargo_tree_crates(&forced_stdout),
        BTreeSet::from([
            "spawn-render".to_owned(),
            "spawn-math".to_owned(),
            "libm".to_owned(),
        ])
    );

    // 2. If ANSI CSI escapes are present in `cargo tree` output, `parse_cargo_tree_crates`
    //    must strip them cleanly rather than emitting `"2m"` tokens.
    let ansi_tree = "\x1b[0m\x1b[1mspawn-render\x1b[0m v0.1.0 (/repo/crates/spawn-render)\n\
                     \x1b[0m\x1b[2m└──\x1b[0m \x1b[0m\x1b[1mspawn-math\x1b[0m v0.1.0 (/repo/crates/spawn-math)\n\
                     \x1b[0m\x1b[2m    └──\x1b[0m \x1b[0m\x1b[1mlibm\x1b[0m v0.2.16\n";
    assert_eq!(
        parse_cargo_tree_crates(ansi_tree),
        BTreeSet::from([
            "spawn-render".to_owned(),
            "spawn-math".to_owned(),
            "libm".to_owned(),
        ])
    );

    // 3. Forbidden dependencies (`ash`, `wayland-client`, `winit`, or upward crate edges)
    //    must still fail verification in both plain and ANSI-colored outputs.
    for forbidden in [
        "ash",
        "wayland-client",
        "winit",
        "spawn-engine",
        "spawn-cli",
    ] {
        let synthetic_tree = format!(
            "\x1b[0m\x1b[1mspawn-input\x1b[0m v0.1.0\n\
             \x1b[0m\x1b[2m└──\x1b[0m \x1b[0m\x1b[1m{forbidden}\x1b[0m v0.1.0\n"
        );
        let err = check_tree_forbids(
            &synthetic_tree,
            "regression check",
            &[
                "ash",
                "wayland-client",
                "winit",
                "spawn-engine",
                "spawn-cli",
            ],
        )
        .unwrap_err();
        assert!(err.contains(forbidden), "unexpected error text: {err}");
    }

    Ok(())
}

/// Regression for the audit finding that hand-parsed manifests missed valid
/// Cargo forms. Each form below resolves, in `cargo tree --invert` output, to
/// a plain dependent line, so the resolver-based graph must flag all of them.
#[test]
fn resolver_graph_detects_table_header_renamed_and_build_only_upward_edges() {
    // Synthetic `cargo tree --workspace -i <target> --depth 1 --prefix none` outputs
    // for an engine crate (`spawn-ecs`) declaring an upward edge as:
    //   [dependencies.spawn-engine]                       (table header)
    //   engine = { package = "spawn-engine", path = .. }  (renamed)
    //   [build-dependencies] spawn-cli = { path = .. }     (build-only)
    let cases = [
        (
            "table-header",
            "spawn-engine",
            "spawn-engine v0.1.0 (/repo)\nspawn-ecs v0.1.0 (/repo/crates/spawn-ecs)\n",
        ),
        (
            "renamed",
            "spawn-engine",
            "spawn-engine v0.1.0 (/repo)\nspawn-ecs v0.1.0 (/repo/crates/spawn-ecs)\n",
        ),
        (
            "build-only",
            "spawn-cli",
            "spawn-cli v0.1.0 (/repo/tools/spawn-cli)\nspawn-ecs v0.1.0 (/repo/crates/spawn-ecs)\n",
        ),
    ];
    for (label, target, inverted) in cases {
        let mut graph = graph::WorkspaceGraph {
            edges: ["spawn-ecs", "spawn-engine", "spawn-cli"]
                .into_iter()
                .map(|name| (name.to_owned(), BTreeSet::new()))
                .collect(),
            engine_crates: BTreeSet::from(["spawn-ecs".to_owned()]),
            tool_crates: BTreeSet::from(["spawn-cli".to_owned()]),
        };
        graph::add_inverted_tree_edges(&mut graph.edges, target, inverted);
        assert_eq!(
            graph.edges["spawn-ecs"],
            BTreeSet::from([target.to_owned()]),
            "{label}: dependent edge must be recorded"
        );
        let err = graph::assert_no_upward_edges(&graph).unwrap_err();
        assert!(
            err.contains(target),
            "{label}: unexpected error text: {err}"
        );
    }

    // Allowed downward edges and non-workspace names are not violations.
    let mut graph = graph::WorkspaceGraph {
        edges: ["spawn-render", "spawn-math", "spawn-cli", "spawn-engine"]
            .into_iter()
            .map(|name| (name.to_owned(), BTreeSet::new()))
            .collect(),
        engine_crates: BTreeSet::from(["spawn-render".to_owned(), "spawn-math".to_owned()]),
        tool_crates: BTreeSet::from(["spawn-cli".to_owned()]),
    };
    graph::add_inverted_tree_edges(
        &mut graph.edges,
        "spawn-math",
        "spawn-math v0.1.0 (/repo/crates/spawn-math)\nspawn-render v0.1.0 (/repo/crates/spawn-render)\nexternal-crate v1.0.0\n",
    );
    assert!(graph::assert_no_upward_edges(&graph).is_ok());
    assert_eq!(
        graph.edges["spawn-render"],
        BTreeSet::from(["spawn-math".to_owned()])
    );
}

fn audit_public_api_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    const FORBIDDEN_GENRE_TOKENS: &[&str] = &[
        "weapon", "health", "team", "player", "vehicle", "ammo", "reload", "kill", "score",
        "damage",
    ];
    const FORBIDDEN_BACKEND_PREFIXES: &[&str] = &[
        "ash::",
        "vk::",
        "wayland_client::",
        "wayland_protocols::",
        "wl_",
        "xdg_",
        "zwp_",
        "wp_",
    ];

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Private backend modules (`linux`, `vulkan`, `native`) are verified
            // not to be `pub mod` in their parent `lib.rs` below.
            if matches!(dir_name, "linux" | "vulkan") {
                continue;
            }
            audit_public_api_dir(&path)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        for (line_no, raw_line) in content.lines().enumerate() {
            let code = raw_line.split("//").next().unwrap_or("").trim();
            if code.starts_with("#[cfg(test)]") {
                break;
            }
            assert!(
                !code.starts_with("pub mod linux") && !code.starts_with("pub mod vulkan"),
                "{}:{} must keep backend modules private: `{code}`",
                path.display(),
                line_no + 1
            );
            // Match unrestricted `pub ` items (not `pub(crate)` / `pub(super)`).
            let is_public_item = code.starts_with("pub ") && !code.starts_with("pub mod ");
            if !is_public_item {
                continue;
            }
            for backend in FORBIDDEN_BACKEND_PREFIXES {
                assert!(
                    !code.contains(backend),
                    "{}:{} leaks backend token `{backend}` in public item: `{code}`",
                    path.display(),
                    line_no + 1
                );
            }
            for token in code
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .filter(|tok| !tok.is_empty())
            {
                let lower = token.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN_GENRE_TOKENS.contains(&lower.as_str()),
                    "{}:{} contains genre-specific token `{token}` in public item: `{code}`",
                    path.display(),
                    line_no + 1
                );
            }
        }
    }
    Ok(())
}
