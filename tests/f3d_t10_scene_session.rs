//! `F3D-T10` (`docs/first-3d/VERIFICATION.md` line 20; `F3D-FR-09, 10, 11, 12`,
//! `F3D-INV-09, 10, 15`): headless `spawn-engine` 3D session and failure-injection
//! verification over [`RuntimeBackend`] and [`RenderSurface`].

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use spawn_core::{FixedStepConfig, Tick};
use spawn_engine::{
    EngineError, EngineErrorKind, ExitReason, FreeCameraConfig, FreeCameraPose,
    MESH_3D_FS_ARTIFACT, MESH_3D_VS_ARTIFACT, RenderSurface, RuntimeBackend, RuntimeOptions,
    SHADER_COOK_HINT, WindowSurface, run_session,
};
use spawn_platform::{
    CursorGrabMode, ElementState, Event, KeyCode, KeyEvent, MouseButton, PhysicalSize,
    WindowDescriptor, WindowEvent, WindowId,
};
use spawn_render::{
    CameraRelativeView, ClearColor, CookedShader, FrameOutcome, GpuAllocatorStats,
    GraphicsPipelineDescriptor, MeshDescriptor, MeshDraw, MeshHandle, PipelineHandle,
    PresentationState, RendererDescriptor, ShaderHandle, SurfaceSize, ValidationMessage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum LifecycleEvent {
    CreatePlatform,
    DropPlatform,
    CreateWindow(WindowId),
    DropWindow(WindowId),
    CreateRenderer(SurfaceSize),
    CloseRenderer,
    DropRenderer,
    LoadShader {
        label: String,
        handle: ShaderHandle,
    },
    DestroyShader(ShaderHandle),
    CreatePipeline {
        label: String,
        handle: PipelineHandle,
    },
    DestroyPipeline(PipelineHandle),
    UploadMesh {
        label: String,
        handle: MeshHandle,
    },
    DestroyMesh(MeshHandle),
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

#[derive(Debug, Clone)]
struct RecordedDrawFrame {
    pipeline: PipelineHandle,
    clear_color: ClearColor,
    view: CameraRelativeView,
    draws: Vec<MeshDraw>,
}

#[derive(Default)]
struct FailurePlan {
    fail_platform: bool,
    fail_window: bool,
    fail_renderer: bool,
    fail_read_shader_at: Option<usize>,
    corrupt_shader_at: Option<usize>,
    fail_pipeline_kind: Option<EngineErrorKind>,
    fail_upload_mesh_at: Option<usize>,
    fail_cursor_grab_fatal: bool,
    fail_cursor_grab_recoverable: bool,
    fail_destroy_mesh_at: Option<usize>,
    fail_destroy_pipeline: bool,
    fail_destroy_shader: bool,
    fail_close: bool,
    validation_errors: u64,
}

struct Mock3dRenderer<'w> {
    _window: &'w MockWindow,
    size: SurfaceSize,
    state: PresentationState,
    log: Rc<RefCell<Vec<LifecycleEvent>>>,
    next_shader_idx: u32,
    next_mesh_idx: u32,
    live_shaders: Vec<ShaderHandle>,
    live_pipeline: Option<PipelineHandle>,
    live_meshes: Vec<MeshHandle>,
    peak_live_allocations: u64,
    scripted_outcomes: VecDeque<Result<FrameOutcome, EngineError>>,
    recorded_frames: Rc<RefCell<Vec<RecordedDrawFrame>>>,
    fail_pipeline_kind: Option<EngineErrorKind>,
    fail_upload_mesh_at: Option<usize>,
    upload_attempts: usize,
    fail_destroy_mesh_at: Option<usize>,
    destroy_mesh_attempts: usize,
    fail_destroy_pipeline: bool,
    fail_destroy_shader: bool,
    fail_close: bool,
    validation_errors: u64,
}

impl Mock3dRenderer<'_> {
    fn current_stats(&self) -> GpuAllocatorStats {
        let live_allocs = (self.live_meshes.len() as u32) * 2 + u32::from(!self.size.is_empty());
        let live_bytes = (self.live_meshes.len() as u64) * 4096;
        GpuAllocatorStats {
            block_count: u32::from(live_allocs > 0),
            dedicated_block_count: 0,
            device_local_block_bytes: if live_allocs > 0 { 16 * 1024 * 1024 } else { 0 },
            host_visible_block_bytes: 0,
            live_allocations: live_allocs,
            retired_allocations: 0,
            live_allocated_bytes: live_bytes,
            retired_bytes: 0,
            peak_block_bytes: 16 * 1024 * 1024,
            peak_allocated_bytes: self.peak_live_allocations * 4096,
        }
    }
}

impl RenderSurface for Mock3dRenderer<'_> {
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
                Ok(FrameOutcome::Suspended) => self.state = PresentationState::Suspended,
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

    fn drain_validation_messages(&mut self, _out: &mut Vec<ValidationMessage>) -> u64 {
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
        CookedShader::decode_named(label, cooked_bytes).map_err(|err| {
            EngineError::new(EngineErrorKind::Shader)
                .with_detail(format!("loading cooked shader `{label}`"))
                .with_cause(&err)
        })?;
        let handle = ShaderHandle::from_raw(self.next_shader_idx, 1);
        self.next_shader_idx += 1;
        self.live_shaders.push(handle);
        self.log.borrow_mut().push(LifecycleEvent::LoadShader {
            label: label.to_owned(),
            handle,
        });
        Ok(handle)
    }

    fn destroy_shader(&mut self, handle: ShaderHandle) -> Result<(), EngineError> {
        self.log
            .borrow_mut()
            .push(LifecycleEvent::DestroyShader(handle));
        let prev_len = self.live_shaders.len();
        self.live_shaders.retain(|&h| h != handle);
        if self.live_shaders.len() == prev_len {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("unknown or stale shader handle {handle:?}")));
        }
        if self.fail_destroy_shader {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail("injected destroy_shader failure"));
        }
        Ok(())
    }

    fn create_graphics_pipeline(
        &mut self,
        descriptor: &GraphicsPipelineDescriptor<'_>,
    ) -> Result<PipelineHandle, EngineError> {
        if let Some(kind) = self.fail_pipeline_kind {
            return Err(EngineError::new(kind).with_detail(format!(
                "injected pipeline creation failure for `{}`",
                descriptor.label
            )));
        }
        if descriptor.vertex_shader == descriptor.fragment_shader
            || !self.live_shaders.contains(&descriptor.vertex_shader)
            || !self.live_shaders.contains(&descriptor.fragment_shader)
        {
            return Err(EngineError::new(EngineErrorKind::Shader)
                .with_detail("invalid or unknown shader handle in pipeline descriptor"));
        }
        let handle = PipelineHandle::from_raw(0, 1);
        self.live_pipeline = Some(handle);
        self.log.borrow_mut().push(LifecycleEvent::CreatePipeline {
            label: descriptor.label.to_owned(),
            handle,
        });
        Ok(handle)
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) -> Result<(), EngineError> {
        self.log
            .borrow_mut()
            .push(LifecycleEvent::DestroyPipeline(handle));
        if self.live_pipeline != Some(handle) {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("unknown or stale pipeline handle {handle:?}")));
        }
        self.live_pipeline = None;
        if self.fail_destroy_pipeline {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail("injected destroy_pipeline failure"));
        }
        Ok(())
    }

    fn upload_mesh(&mut self, descriptor: &MeshDescriptor<'_>) -> Result<MeshHandle, EngineError> {
        let attempt = self.upload_attempts;
        self.upload_attempts += 1;
        if self.fail_upload_mesh_at == Some(attempt) {
            return Err(
                EngineError::new(EngineErrorKind::Render).with_detail(format!(
                    "injected upload_mesh failure at index {attempt} (`{}`)",
                    descriptor.label
                )),
            );
        }
        descriptor.validate().map_err(|err| {
            EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("uploading mesh `{}`", descriptor.label))
                .with_cause(&err)
        })?;
        let handle = MeshHandle::from_raw(self.next_mesh_idx, 1);
        self.next_mesh_idx += 1;
        self.live_meshes.push(handle);
        self.peak_live_allocations = self
            .peak_live_allocations
            .max(self.live_meshes.len() as u64);
        self.log.borrow_mut().push(LifecycleEvent::UploadMesh {
            label: descriptor.label.to_owned(),
            handle,
        });
        Ok(handle)
    }

    fn destroy_mesh(&mut self, handle: MeshHandle) -> Result<(), EngineError> {
        let attempt = self.destroy_mesh_attempts;
        self.destroy_mesh_attempts += 1;
        self.log
            .borrow_mut()
            .push(LifecycleEvent::DestroyMesh(handle));
        let prev_len = self.live_meshes.len();
        self.live_meshes.retain(|&h| h != handle);
        if self.live_meshes.len() == prev_len {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("unknown or stale mesh handle {handle:?}")));
        }
        if self.fail_destroy_mesh_at == Some(attempt) {
            return Err(
                EngineError::new(EngineErrorKind::Render).with_detail(format!(
                    "injected destroy_mesh failure at teardown index {attempt}"
                )),
            );
        }
        Ok(())
    }

    fn render_meshes(
        &mut self,
        pipeline: PipelineHandle,
        clear_color: ClearColor,
        view: &CameraRelativeView,
        draws: &[MeshDraw],
    ) -> Result<FrameOutcome, EngineError> {
        if self.live_pipeline != Some(pipeline) {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("unknown or stale pipeline handle {pipeline:?}")));
        }
        for draw in draws {
            if !self.live_meshes.contains(&draw.mesh) {
                return Err(EngineError::new(EngineErrorKind::Render)
                    .with_detail(format!("unknown or stale mesh handle {:?}", draw.mesh)));
            }
            draw.pack_push_constants(view).map_err(|err| {
                EngineError::new(EngineErrorKind::Render)
                    .with_detail("packing 3D mesh push constants")
                    .with_cause(&err)
            })?;
        }
        let outcome = self.render_clear(clear_color)?;
        if outcome == FrameOutcome::Presented {
            self.recorded_frames.borrow_mut().push(RecordedDrawFrame {
                pipeline,
                clear_color,
                view: *view,
                draws: draws.to_vec(),
            });
        }
        Ok(outcome)
    }

    fn allocator_stats(&self) -> GpuAllocatorStats {
        self.current_stats()
    }

    fn close(mut self) -> Result<GpuAllocatorStats, EngineError> {
        self.log.borrow_mut().push(LifecycleEvent::CloseRenderer);
        self.size = SurfaceSize::new(0, 0);
        if self.fail_close {
            return Err(EngineError::new(EngineErrorKind::Render)
                .with_detail("injected renderer close failure"));
        }
        Ok(self.current_stats())
    }
}

impl Drop for Mock3dRenderer<'_> {
    fn drop(&mut self) {
        self.log.borrow_mut().push(LifecycleEvent::DropRenderer);
    }
}

struct Step {
    elapsed: Duration,
    events: Vec<Event>,
}

struct Mock3dBackend {
    log: Rc<RefCell<Vec<LifecycleEvent>>>,
    window_id: WindowId,
    initial_size: PhysicalSize,
    failures: FailurePlan,
    primed: bool,
    steps: VecDeque<Step>,
    current_elapsed: Duration,
    read_shader_calls: usize,
    scripted_outcomes: VecDeque<Result<FrameOutcome, EngineError>>,
    recorded_frames: Rc<RefCell<Vec<RecordedDrawFrame>>>,
    camera_poses: Vec<(Tick, FreeCameraPose, CursorGrabMode)>,
    unavailable_grab_errors: Vec<String>,
    wait_timeouts: Vec<Option<Duration>>,
}

impl Mock3dBackend {
    fn new() -> Self {
        Self {
            log: Rc::new(RefCell::new(Vec::new())),
            window_id: WindowId::from_raw(7),
            initial_size: PhysicalSize::new(960, 540),
            failures: FailurePlan::default(),
            primed: false,
            steps: VecDeque::new(),
            current_elapsed: Duration::ZERO,
            read_shader_calls: 0,
            scripted_outcomes: VecDeque::new(),
            recorded_frames: Rc::new(RefCell::new(Vec::new())),
            camera_poses: Vec::new(),
            unavailable_grab_errors: Vec::new(),
            wait_timeouts: Vec::new(),
        }
    }
}

impl RuntimeBackend for Mock3dBackend {
    type Platform = MockPlatform;
    type Window = MockWindow;
    type Renderer<'w> = Mock3dRenderer<'w>;

    fn read_elapsed(&mut self) -> Duration {
        if !self.primed {
            self.primed = true;
            return Duration::ZERO;
        }
        self.current_elapsed
    }

    fn create_platform(&mut self) -> Result<Self::Platform, EngineError> {
        if self.failures.fail_platform {
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
        if self.failures.fail_window {
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
        if self.failures.fail_cursor_grab_fatal {
            return Err(EngineError::new(EngineErrorKind::Platform)
                .with_detail("injected fatal cursor grab failure"));
        }
        if self.failures.fail_cursor_grab_recoverable && mode != CursorGrabMode::None {
            return Err(EngineError::new(EngineErrorKind::Platform)
                .with_detail("compositor does not support pointer constraints")
                .with_recoverable_cursor_grab());
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
        if let Some(step) = self.steps.pop_front() {
            self.current_elapsed = step.elapsed;
            events.extend(step.events);
        } else {
            self.current_elapsed = Duration::from_nanos(16_666_666);
            events.push(Event::Window {
                window: self.window_id,
                event: WindowEvent::CloseRequested,
            });
        }
        Ok(())
    }

    fn create_renderer<'w>(
        &mut self,
        window: &'w Self::Window,
        size: SurfaceSize,
        _descriptor: &RendererDescriptor,
    ) -> Result<Self::Renderer<'w>, EngineError> {
        if self.failures.fail_renderer {
            return Err(
                EngineError::new(EngineErrorKind::Render).with_detail("injected renderer failure")
            );
        }
        self.log
            .borrow_mut()
            .push(LifecycleEvent::CreateRenderer(size));
        Ok(Mock3dRenderer {
            _window: window,
            size,
            state: if size.is_empty() {
                PresentationState::Suspended
            } else {
                PresentationState::Ready
            },
            log: Rc::clone(&self.log),
            next_shader_idx: 0,
            next_mesh_idx: 0,
            live_shaders: Vec::new(),
            live_pipeline: None,
            live_meshes: Vec::new(),
            peak_live_allocations: 0,
            scripted_outcomes: std::mem::take(&mut self.scripted_outcomes),
            recorded_frames: Rc::clone(&self.recorded_frames),
            fail_pipeline_kind: self.failures.fail_pipeline_kind,
            fail_upload_mesh_at: self.failures.fail_upload_mesh_at,
            upload_attempts: 0,
            fail_destroy_mesh_at: self.failures.fail_destroy_mesh_at,
            destroy_mesh_attempts: 0,
            fail_destroy_pipeline: self.failures.fail_destroy_pipeline,
            fail_destroy_shader: self.failures.fail_destroy_shader,
            fail_close: self.failures.fail_close,
            validation_errors: self.failures.validation_errors,
        })
    }

    fn read_cooked_shader(
        &mut self,
        shader_dir: &Path,
        artifact_name: &str,
    ) -> Result<Vec<u8>, EngineError> {
        let idx = self.read_shader_calls;
        self.read_shader_calls += 1;
        if self.failures.fail_read_shader_at == Some(idx) {
            let full = shader_dir.join(artifact_name);
            return Err(
                EngineError::new(EngineErrorKind::Shader).with_detail(format!(
                    "file `{}`: injected missing cooked shader; {SHADER_COOK_HINT}",
                    full.display()
                )),
            );
        }
        if self.failures.corrupt_shader_at == Some(idx) {
            return Ok(b"BADSHDR\0corrupt_payload".to_vec());
        }
        spawn_engine::synthesize_minimal_mesh_3d_shader(shader_dir, artifact_name)
    }

    fn on_camera_tick(&mut self, tick: Tick, pose: &FreeCameraPose, cursor_grab: CursorGrabMode) {
        self.camera_poses.push((tick, *pose, cursor_grab));
    }

    fn on_cursor_grab_unavailable(&mut self, error: &EngineError) {
        self.unavailable_grab_errors.push(error.to_string());
    }
}

fn expected_shader_label(artifact: &str) -> String {
    Path::new("target/shaders")
        .join(artifact)
        .to_string_lossy()
        .into_owned()
}

fn vs_handle() -> ShaderHandle {
    ShaderHandle::from_raw(0, 1)
}
fn fs_handle() -> ShaderHandle {
    ShaderHandle::from_raw(1, 1)
}
fn pipe_handle() -> PipelineHandle {
    PipelineHandle::from_raw(0, 1)
}
fn mesh_handle(i: u32) -> MeshHandle {
    MeshHandle::from_raw(i, 1)
}

fn prefix_events(win: WindowId) -> Vec<LifecycleEvent> {
    vec![
        LifecycleEvent::CreatePlatform,
        LifecycleEvent::CreateWindow(win),
        LifecycleEvent::CreateRenderer(SurfaceSize::new(960, 540)),
    ]
}

fn suffix_events(win: WindowId) -> Vec<LifecycleEvent> {
    vec![
        LifecycleEvent::CloseRenderer,
        LifecycleEvent::DropRenderer,
        LifecycleEvent::DropWindow(win),
        LifecycleEvent::DropPlatform,
    ]
}

fn key_event(win: WindowId, key: KeyCode, state: ElementState) -> Event {
    Event::Window {
        window: win,
        event: WindowEvent::Key(KeyEvent {
            key,
            native_code: 0,
            state,
        }),
    }
}

fn mouse_btn_event(win: WindowId, button: MouseButton, state: ElementState) -> Event {
    Event::Window {
        window: win,
        event: WindowEvent::PointerButton { button, state },
    }
}

#[test]
fn f3d_t10_partial_startup_and_runtime_failures_unwind_scene_handles_in_exact_reverse_order() {
    let win = WindowId::from_raw(7);

    // 1. Pre-startup validation errors (shader_dir and camera config) leak zero resources.
    {
        let mut backend = Mock3dBackend::new();
        let bad_dir_opts = RuntimeOptions {
            shader_dir: "".into(),
            ..RuntimeOptions::default()
        };
        let err = run_session(&bad_dir_opts, &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::InvalidOption);
        assert_eq!(err.code(), "ENGN-0001");
        assert!(backend.log.borrow().is_empty());

        let mut backend2 = Mock3dBackend::new();
        let bad_cam_opts = RuntimeOptions {
            camera: FreeCameraConfig {
                pitch_limit_radians: std::f64::consts::FRAC_PI_2,
                ..FreeCameraConfig::default()
            },
            ..RuntimeOptions::default()
        };
        let err2 = run_session(&bad_cam_opts, &mut backend2).unwrap_err();
        assert_eq!(err2.kind(), EngineErrorKind::Camera);
        assert_eq!(err2.code(), "ENGN-0009");
        assert!(backend2.log.borrow().is_empty());
    }

    // 2. Vertex shader read failure & corrupt payload failure -> closes/drops Renderer -> Window -> Platform.
    for (label, mutate) in [
        (
            "read_vs",
            Box::new(|f: &mut FailurePlan| f.fail_read_shader_at = Some(0))
                as Box<dyn Fn(&mut FailurePlan)>,
        ),
        (
            "corrupt_vs",
            Box::new(|f: &mut FailurePlan| f.corrupt_shader_at = Some(0)),
        ),
    ] {
        let mut backend = Mock3dBackend::new();
        mutate(&mut backend.failures);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Shader, "case {label}");
        assert_eq!(err.code(), "ENGN-0008", "case {label}");
        let mut expected = prefix_events(win);
        expected.extend(suffix_events(win));
        assert_eq!(*backend.log.borrow(), expected, "case {label}");
    }

    // 3. Fragment shader read/load failure -> destroys VS handle once -> Renderer -> Window -> Platform.
    for (label, mutate) in [
        (
            "read_fs",
            Box::new(|f: &mut FailurePlan| f.fail_read_shader_at = Some(1))
                as Box<dyn Fn(&mut FailurePlan)>,
        ),
        (
            "corrupt_fs",
            Box::new(|f: &mut FailurePlan| f.corrupt_shader_at = Some(1)),
        ),
    ] {
        let mut backend = Mock3dBackend::new();
        mutate(&mut backend.failures);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Shader, "case {label}");
        assert_eq!(err.code(), "ENGN-0008", "case {label}");
        let mut expected = prefix_events(win);
        expected.push(LifecycleEvent::LoadShader {
            label: expected_shader_label(MESH_3D_VS_ARTIFACT),
            handle: vs_handle(),
        });
        expected.push(LifecycleEvent::DestroyShader(vs_handle()));
        expected.extend(suffix_events(win));
        assert_eq!(*backend.log.borrow(), expected, "case {label}");
    }

    // 4. Graphics pipeline failure (shader mismatch ENGN-0008 or GPU ENGN-0005)
    //    -> destroys FS then VS -> Renderer -> Window -> Platform.
    for kind in [EngineErrorKind::Shader, EngineErrorKind::Render] {
        let mut backend = Mock3dBackend::new();
        backend.failures.fail_pipeline_kind = Some(kind);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), kind);
        let mut expected = prefix_events(win);
        expected.push(LifecycleEvent::LoadShader {
            label: expected_shader_label(MESH_3D_VS_ARTIFACT),
            handle: vs_handle(),
        });
        expected.push(LifecycleEvent::LoadShader {
            label: expected_shader_label(MESH_3D_FS_ARTIFACT),
            handle: fs_handle(),
        });
        expected.push(LifecycleEvent::DestroyShader(fs_handle()));
        expected.push(LifecycleEvent::DestroyShader(vs_handle()));
        expected.extend(suffix_events(win));
        assert_eq!(*backend.log.borrow(), expected);
    }

    // 5. Partial mesh upload failure (mesh #2 after meshes #0 and #1 uploaded)
    //    -> destroys mesh #1 -> mesh #0 -> pipeline -> FS -> VS -> Renderer -> Window -> Platform.
    {
        let mut backend = Mock3dBackend::new();
        backend.failures.fail_upload_mesh_at = Some(2);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Render);
        assert_eq!(err.code(), "ENGN-0005");
        let mut expected = prefix_events(win);
        expected.push(LifecycleEvent::LoadShader {
            label: expected_shader_label(MESH_3D_VS_ARTIFACT),
            handle: vs_handle(),
        });
        expected.push(LifecycleEvent::LoadShader {
            label: expected_shader_label(MESH_3D_FS_ARTIFACT),
            handle: fs_handle(),
        });
        expected.push(LifecycleEvent::CreatePipeline {
            label: "mesh_3d".to_owned(),
            handle: pipe_handle(),
        });
        expected.push(LifecycleEvent::UploadMesh {
            label: "unit_cube".to_owned(),
            handle: mesh_handle(0),
        });
        expected.push(LifecycleEvent::UploadMesh {
            label: "unit_octahedron".to_owned(),
            handle: mesh_handle(1),
        });
        expected.push(LifecycleEvent::DestroyMesh(mesh_handle(1)));
        expected.push(LifecycleEvent::DestroyMesh(mesh_handle(0)));
        expected.push(LifecycleEvent::DestroyPipeline(pipe_handle()));
        expected.push(LifecycleEvent::DestroyShader(fs_handle()));
        expected.push(LifecycleEvent::DestroyShader(vs_handle()));
        expected.extend(suffix_events(win));
        assert_eq!(*backend.log.borrow(), expected);
    }

    // 6. Mid-loop fatal `render_meshes` failure & validation-layer error (`ENGN-0006`)
    //    -> unwinds all 3 meshes (rev) -> pipeline -> 2 shaders (rev) -> Renderer -> Window -> Platform.
    for (is_validation, expected_kind) in [
        (false, EngineErrorKind::Render),
        (true, EngineErrorKind::Validation),
    ] {
        let mut backend = Mock3dBackend::new();
        backend.steps.push_back(Step {
            elapsed: Duration::from_nanos(16_666_666),
            events: Vec::new(),
        });
        if is_validation {
            backend.failures.validation_errors = 2;
        } else {
            backend.scripted_outcomes.push_back(Err(
                EngineError::new(EngineErrorKind::Render).with_detail("simulated draw failure")
            ));
        }
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), expected_kind);

        let log = backend.log.borrow();
        let teardown_start = log
            .iter()
            .position(|ev| matches!(ev, LifecycleEvent::DestroyMesh(_)))
            .expect("scene teardown must run");
        assert_eq!(
            &log[teardown_start..],
            &[
                LifecycleEvent::DestroyMesh(mesh_handle(2)),
                LifecycleEvent::DestroyMesh(mesh_handle(1)),
                LifecycleEvent::DestroyMesh(mesh_handle(0)),
                LifecycleEvent::DestroyPipeline(pipe_handle()),
                LifecycleEvent::DestroyShader(fs_handle()),
                LifecycleEvent::DestroyShader(vs_handle()),
                LifecycleEvent::CloseRenderer,
                LifecycleEvent::DropRenderer,
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }

    // 7. Error during `destroy_mesh` on an otherwise clean exit still finishes
    //    unwinding all remaining handles + renderer + window + platform once each.
    {
        let mut backend = Mock3dBackend::new();
        backend.failures.fail_destroy_mesh_at = Some(0);
        let err = run_session(&RuntimeOptions::default(), &mut backend).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Render);
        assert!(
            err.detail()
                .unwrap_or("")
                .contains("injected destroy_mesh failure")
        );
        let log = backend.log.borrow();
        let teardown_start = log
            .iter()
            .position(|ev| matches!(ev, LifecycleEvent::DestroyMesh(_)))
            .expect("teardown must run");
        assert_eq!(
            &log[teardown_start..],
            &[
                LifecycleEvent::DestroyMesh(mesh_handle(2)),
                LifecycleEvent::DestroyMesh(mesh_handle(1)),
                LifecycleEvent::DestroyMesh(mesh_handle(0)),
                LifecycleEvent::DestroyPipeline(pipe_handle()),
                LifecycleEvent::DestroyShader(fs_handle()),
                LifecycleEvent::DestroyShader(vs_handle()),
                LifecycleEvent::CloseRenderer,
                LifecycleEvent::DropRenderer,
                LifecycleEvent::DropWindow(win),
                LifecycleEvent::DropPlatform,
            ]
        );
    }
}

#[test]
fn f3d_t10_interactive_3d_session_cursor_lock_resize_suspend_and_repeated_close() {
    let win = WindowId::from_raw(7);
    let tick_dt = FixedStepConfig::default().tick_duration();
    let mut backend = Mock3dBackend::new();

    // Tick 0: unlocked `PointerRelativeMotion` does NOT rotate camera; `Tab` pressed -> locks cursor.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![
            Event::Window {
                window: win,
                event: WindowEvent::Focused(true),
            },
            Event::Window {
                window: win,
                event: WindowEvent::PointerRelativeMotion {
                    dx: 300.0,
                    dy: -200.0,
                },
            },
            key_event(win, KeyCode::Tab, ElementState::Pressed),
        ],
    });

    // Tick 1: `Tab` still held (must NOT toggle back off!), `W` held + pointer motion -> translates & rotates.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![
            key_event(win, KeyCode::W, ElementState::Pressed),
            Event::Window {
                window: win,
                event: WindowEvent::PointerRelativeMotion {
                    dx: 50.0,
                    dy: -25.0,
                },
            },
        ],
    });

    // Tick 2: resize to 0x0 (Suspended) while keeping simulation advancing; `Tab` released.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![
            key_event(win, KeyCode::Tab, ElementState::Released),
            Event::Window {
                window: win,
                event: WindowEvent::Resized(PhysicalSize::new(0, 0)),
            },
        ],
    });

    // Tick 3: still 0x0 (bounded wait timeout), simulation continues monotonically.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: Vec::new(),
    });

    // Tick 4: resize to 1280x720 + transient Skipped recovery + `MouseButton::Right` unlocks cursor.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![
            Event::Window {
                window: win,
                event: WindowEvent::Resized(PhysicalSize::new(1280, 720)),
            },
            mouse_btn_event(win, MouseButton::Right, ElementState::Pressed),
        ],
    });

    // Tick 5: recovered Presented frame at 1280x720 + `Tab` locks again.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![
            mouse_btn_event(win, MouseButton::Right, ElementState::Released),
            key_event(win, KeyCode::Tab, ElementState::Pressed),
        ],
    });

    // Tick 6: `Focused(false)` releases cursor lock & held `W`, and duplicate `CloseRequested` exits cleanly.
    backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![
            Event::Window {
                window: win,
                event: WindowEvent::Focused(false),
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
    });

    // Outcomes for non-suspended frames: Tick 0 Presented, Tick 1 Presented, Tick 4 Skipped, Tick 5 Presented.
    backend
        .scripted_outcomes
        .push_back(Ok(FrameOutcome::Presented));
    backend
        .scripted_outcomes
        .push_back(Ok(FrameOutcome::Presented));
    backend
        .scripted_outcomes
        .push_back(Ok(FrameOutcome::Skipped));
    backend
        .scripted_outcomes
        .push_back(Ok(FrameOutcome::Presented));

    let summary = run_session(&RuntimeOptions::default(), &mut backend).unwrap();
    assert_eq!(summary.exit_reason, ExitReason::CloseRequested);
    assert_eq!(summary.ticks_executed, 7);
    assert_eq!(summary.last_tick, Some(Tick::new(6)));
    assert_eq!(summary.frames_presented, 3);
    assert_eq!(summary.frames_suspended, 2);
    assert_eq!(summary.frames_skipped, 1);
    assert_eq!(summary.close_requests, 2);
    assert_eq!(summary.final_cursor_grab, CursorGrabMode::None);
    assert!(summary.active_allocator_stats.live_allocations > 0);
    assert_eq!(summary.shutdown_allocator_stats.live_allocations, 0);

    // Tick 0: unlocked relative motion did NOT rotate camera, cursor locked at end of tick 0.
    assert_eq!(backend.camera_poses[0].1.yaw_radians, 0.0);
    assert_eq!(backend.camera_poses[0].2, CursorGrabMode::Locked);

    // Tick 1: cursor stayed Locked (held Tab did not double-toggle), camera rotated and translated!
    assert_eq!(backend.camera_poses[1].2, CursorGrabMode::Locked);
    assert!(backend.camera_poses[1].1.yaw_radians < 0.0);
    assert!(backend.camera_poses[1].1.pitch_radians > 0.0);
    assert!(backend.camera_poses[1].1.world_pos.z < 0.0);

    // Tick 2 & 3 (0x0 suspended): camera kept moving forward monotonically without resetting pose!
    assert!(backend.camera_poses[3].1.world_pos.z < backend.camera_poses[1].1.world_pos.z);
    assert!(backend.wait_timeouts[3].is_some());

    // Tick 4: MouseButton::Right unlocked cursor.
    assert_eq!(backend.camera_poses[4].2, CursorGrabMode::None);

    // Tick 5: Tab locked cursor again.
    assert_eq!(backend.camera_poses[5].2, CursorGrabMode::Locked);

    // Tick 6: Focused(false) unlocked cursor & stopped held W movement.
    assert_eq!(backend.camera_poses[6].2, CursorGrabMode::None);
    assert!(
        backend.camera_poses[6]
            .1
            .bitwise_eq(&backend.camera_poses[5].1)
    );

    // Verify 3 presented frames: each received all 12 procedural scene draws, and
    // projection aspect ratio updated from 960/540 to 1280/720 on Tick 5!
    let frames = backend.recorded_frames.borrow();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].pipeline, pipe_handle());
    assert_eq!(frames[0].clear_color, RuntimeOptions::default().clear_color);
    assert_eq!(frames[0].draws.len(), 12);
    assert_eq!(frames[2].pipeline, pipe_handle());
    assert_eq!(frames[2].draws.len(), 12);
    let proj_initial_x = frames[0].view.projection.x_axis.x;
    let proj_resized_x = frames[2].view.projection.x_axis.x;
    assert!(
        (proj_initial_x / proj_resized_x - ((1280.0 / 720.0) / (960.0 / 540.0))).abs() < 1.0e-5,
        "projection x scale should reflect updated aspect ratio"
    );

    // Recoverable cursor grab failure degrades cleanly instead of aborting session.
    let mut rec_backend = Mock3dBackend::new();
    rec_backend.failures.fail_cursor_grab_recoverable = true;
    rec_backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![key_event(win, KeyCode::Tab, ElementState::Pressed)],
    });
    rec_backend.steps.push_back(Step {
        elapsed: tick_dt,
        events: vec![key_event(win, KeyCode::Escape, ElementState::Pressed)],
    });
    let rec_summary = run_session(&RuntimeOptions::default(), &mut rec_backend).unwrap();
    assert_eq!(rec_summary.exit_reason, ExitReason::ExitAction);
    assert_eq!(rec_summary.final_cursor_grab, CursorGrabMode::None);
    assert_eq!(rec_backend.unavailable_grab_errors.len(), 1);
    assert!(rec_backend.unavailable_grab_errors[0].contains("ENGN-0004"));
}

fn normalize_location(report: &str) -> String {
    report
        .lines()
        .take_while(|line| *line != "backtrace:")
        .map(|line| {
            if let Some(rest) = line.strip_prefix("  --> ") {
                let file_only = rest
                    .rsplit_once(':')
                    .and_then(|(prefix, _col)| prefix.rsplit_once(':'))
                    .map_or(rest, |(file, _line)| file)
                    .replace('\\', "/");
                format!("  --> {file_only}:<LINE>:<COL>")
            } else {
                line.replace('\\', "/")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn f3d_t10_engine_error_reports_match_adr_0009_snapshots() {
    let mut backend_opt = Mock3dBackend::new();
    let bad_dir = run_session(
        &RuntimeOptions {
            shader_dir: "".into(),
            ..RuntimeOptions::default()
        },
        &mut backend_opt,
    )
    .unwrap_err();
    insta::assert_snapshot!(
        normalize_location(&bad_dir.to_string()),
        @r#"
error[ENGN-0001]: invalid runtime option
  --> src/runtime.rs:<LINE>:<COL>
   = detail: field `shader_dir` cannot be empty
"#
    );

    let mut backend_shader = Mock3dBackend::new();
    backend_shader.failures.fail_read_shader_at = Some(0);
    let missing_shader = run_session(&RuntimeOptions::default(), &mut backend_shader).unwrap_err();
    insta::assert_snapshot!(
        normalize_location(&missing_shader.to_string()),
        @r#"
error[ENGN-0008]: cooked shader artifact load or validation failed
  --> tests/f3d_t10_scene_session.rs:<LINE>:<COL>
   = detail: file `target/shaders/mesh_3d.vs_main.spnshdr`: injected missing cooked shader; run 'cargo run -p spawn-cli -- shaders build' first
"#
    );

    let mut backend_corrupt = Mock3dBackend::new();
    backend_corrupt.failures.corrupt_shader_at = Some(0);
    let corrupt_shader = run_session(&RuntimeOptions::default(), &mut backend_corrupt).unwrap_err();
    insta::assert_snapshot!(
        normalize_location(&corrupt_shader.to_string()),
        @r#"
error[ENGN-0008]: cooked shader artifact load or validation failed
  --> tests/f3d_t10_scene_session.rs:<LINE>:<COL>
   = detail: loading cooked shader `target/shaders/mesh_3d.vs_main.spnshdr`
caused by: error[REND-0010]: invalid cooked shader artifact
  --> tests/f3d_t10_scene_session.rs:<LINE>:<COL>
   = detail: artifact `target/shaders/mesh_3d.vs_main.spnshdr`: field `header`: truncated artifact of 23 bytes (minimum 40 bytes)
"#
    );
}

#[test]
fn f3d_t10_render_surface_rejects_unknown_and_stale_handles() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let window = MockWindow {
        id: WindowId::from_raw(1),
        log: Rc::clone(&log),
    };
    let mut renderer = Mock3dRenderer {
        _window: &window,
        size: SurfaceSize::new(640, 360),
        state: PresentationState::Ready,
        log,
        next_shader_idx: 0,
        next_mesh_idx: 0,
        live_shaders: Vec::new(),
        live_pipeline: None,
        live_meshes: Vec::new(),
        peak_live_allocations: 0,
        scripted_outcomes: VecDeque::new(),
        recorded_frames: Rc::new(RefCell::new(Vec::new())),
        fail_pipeline_kind: None,
        fail_upload_mesh_at: None,
        upload_attempts: 0,
        fail_destroy_mesh_at: None,
        destroy_mesh_attempts: 0,
        fail_destroy_pipeline: false,
        fail_destroy_shader: false,
        fail_close: false,
        validation_errors: 0,
    };

    let stale_shader = ShaderHandle::from_raw(99, 1);
    let stale_pipeline = PipelineHandle::from_raw(99, 1);
    let stale_mesh = MeshHandle::from_raw(99, 1);

    assert_eq!(
        renderer.destroy_shader(stale_shader).unwrap_err().kind(),
        EngineErrorKind::Render
    );
    assert_eq!(
        renderer
            .create_graphics_pipeline(&GraphicsPipelineDescriptor::new_mesh_3d(
                "bad",
                stale_shader,
                stale_shader,
            ))
            .unwrap_err()
            .kind(),
        EngineErrorKind::Shader
    );
    assert_eq!(
        renderer
            .destroy_pipeline(stale_pipeline)
            .unwrap_err()
            .kind(),
        EngineErrorKind::Render
    );
    assert_eq!(
        renderer.destroy_mesh(stale_mesh).unwrap_err().kind(),
        EngineErrorKind::Render
    );
    assert_eq!(
        renderer
            .render_meshes(
                stale_pipeline,
                ClearColor::new(0.0, 0.0, 0.0, 1.0),
                &CameraRelativeView::new(spawn_math::Mat4::IDENTITY, spawn_math::Quat::IDENTITY),
                &[],
            )
            .unwrap_err()
            .kind(),
        EngineErrorKind::Render
    );

    let _ = renderer.close().unwrap();
}
