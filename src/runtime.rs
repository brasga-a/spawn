//! Single-threaded composition loop (`ADR-0003`, `ADR-0010`, `ADR-0014`,
//! `docs/foundation/DESIGN.md`, `docs/first-3d/DESIGN.md` §5).
//!
//! Coordinates `spawn-core` (`FixedStepClock`), `spawn-math` (`f64` [`WorldPos`],
//! deterministic `det` camera math, reversed-Z projections), `spawn-platform`
//! (window, raw events, cursor constraints), `spawn-input` (`InputMap`,
//! tick-sampled [`InputCommand`]), and `spawn-render` (cooked shader validation,
//! `dynamicRendering` 3D mesh pipeline, staged `DEVICE_LOCAL` meshes, and
//! swapchain presentation) behind [`RuntimeBackend`] and [`RenderSurface`].

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::path::Path;
use std::time::Duration;

use spawn_core::{FixedStepClock, Tick};
use spawn_input::{InputCommand, InputState};
use spawn_platform::{
    CursorGrabMode, Event, PhysicalSize, WindowDescriptor, WindowEvent, WindowId,
};
use spawn_render::{
    CameraRelativeView, ClearColor, CookedShader, FrameOutcome, GpuAllocatorStats,
    GraphicsPipelineDescriptor, MESH_PUSH_CONSTANT_RANGE, MESH_VERTEX_ATTRIBUTES, MeshDescriptor,
    MeshDraw, MeshHandle, PipelineHandle, PipelineLayoutDesc, PresentationState, PushConstantRange,
    RendererDescriptor, SHADER_FORMAT_VERSION, SPIRV_MAGIC, ShaderHandle, ShaderStage, SurfaceSize,
    ValidationMessage, validate_graphics_pipeline_shaders,
};

use crate::camera::{CameraInputBindings, FreeCamera, FreeCameraPose, interpolation_alpha};
use crate::error::{EngineError, EngineErrorKind, SHADER_COOK_HINT};
use crate::options::{RuntimeOptions, validate_shader_dir_path};
use crate::scene::{MESH_3D_FS_ARTIFACT, MESH_3D_VS_ARTIFACT, ProceduralScene};

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
    /// Final cursor constraint state when the session loop exited.
    pub final_cursor_grab: CursorGrabMode,
    /// GPU block allocator snapshot captured before tearing down scene handles.
    pub active_allocator_stats: GpuAllocatorStats,
    /// Post-shutdown GPU block allocator snapshot returned by [`RenderSurface::close`].
    pub shutdown_allocator_stats: GpuAllocatorStats,
}

/// Minimal window handle capability required by the composition loop.
pub trait WindowSurface {
    /// Stable identifier of this window for event matching and queries.
    fn id(&self) -> WindowId;
}

/// Renderer capability seam required by [`run_session`].
///
/// Minimal clear-only mocks (such as Foundation verification harnesses) need
/// only implement the five baseline surface methods; the 3D shader, pipeline,
/// mesh, and allocator methods provide pure-Rust validating defaults and are
/// overridden by `spawn_render::Renderer` on native targets and by 3D session
/// verification mocks (`F3D-T10`).
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

    /// Validates untrusted `.spnshdr` bytes with a diagnostic `label` and
    /// returns a generational [`ShaderHandle`] (`ENGN-0008` on invalid shader).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0008` or `ENGN-0005`) on validation or
    /// resource-pool failure.
    fn load_shader(
        &mut self,
        label: &str,
        cooked_bytes: &[u8],
    ) -> Result<ShaderHandle, EngineError> {
        let cooked = CookedShader::decode_named(label, cooked_bytes).map_err(|err| {
            EngineError::new(EngineErrorKind::Shader)
                .with_detail(format!("validating cooked shader `{label}`"))
                .with_cause(&err)
        })?;
        let slot = match cooked.stage {
            ShaderStage::Vertex => 0,
            ShaderStage::Fragment => 1,
            ShaderStage::Compute => 2,
        };
        Ok(ShaderHandle::from_raw(slot, 1))
    }

    /// Releases `handle`, invalidating its generation.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) if `handle` is invalid or stale.
    fn destroy_shader(&mut self, _handle: ShaderHandle) -> Result<(), EngineError> {
        Ok(())
    }

    /// Validates shader compatibility and compiles a `dynamicRendering`
    /// graphics pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0008` on reflection mismatch, `ENGN-0005`
    /// on GPU compilation failure).
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

    /// Releases `handle`, retiring the underlying GPU pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) if `handle` is invalid or stale.
    fn destroy_pipeline(&mut self, _handle: PipelineHandle) -> Result<(), EngineError> {
        Ok(())
    }

    /// Validates `descriptor` and uploads vertices and `u32` indices into
    /// `DEVICE_LOCAL` buffers.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) on invalid descriptor or GPU memory
    /// exhaustion.
    fn upload_mesh(&mut self, descriptor: &MeshDescriptor<'_>) -> Result<MeshHandle, EngineError> {
        descriptor.validate().map_err(|err| {
            EngineError::new(EngineErrorKind::Render)
                .with_detail(format!("uploading mesh `{}`", descriptor.label))
                .with_cause(&err)
        })?;
        Ok(MeshHandle::from_raw(0, 1))
    }

    /// Releases an uploaded 3D mesh handle.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) if `handle` is invalid or stale.
    fn destroy_mesh(&mut self, _handle: MeshHandle) -> Result<(), EngineError> {
        Ok(())
    }

    /// Clears the next swapchain image to `clear_color` and `D32_SFLOAT`
    /// reversed-Z depth to `0.0`, draws `draws` as camera-relative indexed 3D
    /// meshes (`SPN-INV-052`), and queues presentation.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) on invalid draw state or
    /// unrecoverable presentation failure.
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

    /// Current GPU block suballocator statistics.
    fn allocator_stats(&self) -> GpuAllocatorStats {
        GpuAllocatorStats::default()
    }

    /// Waits for GPU idle, releases remaining renderer-internal resources and
    /// memory blocks, and returns post-shutdown [`GpuAllocatorStats`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] (`ENGN-0005`) if GPU shutdown fails.
    fn close(self) -> Result<GpuAllocatorStats, EngineError>
    where
        Self: Sized,
    {
        Ok(self.allocator_stats())
    }
}

/// Backend seam supplying monotonic elapsed time, cooked-shader artifact I/O,
/// window/event pumping, and surface rendering to [`run_session`].
pub trait RuntimeBackend {
    /// Owned platform event-loop connection (dropped last).
    type Platform;
    /// Owned native top-level window (dropped before [`Self::Platform`]).
    type Window: WindowSurface;
    /// Owned renderer borrowing `&'w Self::Window` (closed/dropped before
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

    /// Loads raw cooked `.spnshdr` bytes for `artifact_name` inside
    /// `shader_dir` (`ADR-0014`, `SPN-INV-083`).
    ///
    /// Native backends read `shader_dir.join(artifact_name)` from disk and
    /// include [`SHADER_COOK_HINT`] on missing files; headless mocks inherit
    /// minimal self-contained `mesh_3d` artifacts so Foundation tests run in
    /// `slangc`-free CI without external files.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Shader`] (`ENGN-0008`) if the artifact cannot
    /// be read.
    fn read_cooked_shader(
        &mut self,
        shader_dir: &Path,
        artifact_name: &str,
    ) -> Result<Vec<u8>, EngineError> {
        synthesize_minimal_mesh_3d_shader(shader_dir, artifact_name)
    }

    /// Observer hook invoked once after shaders, pipeline, and procedural 3D
    /// meshes are uploaded.
    fn on_scene_initialized(
        &mut self,
        _mesh_count: usize,
        _instance_count: usize,
        _stats: GpuAllocatorStats,
    ) {
    }

    /// Observer hook invoked once per executed simulation tick with the sampled
    /// [`InputCommand`].
    fn on_tick(&mut self, _command: &InputCommand) {}

    /// Observer hook invoked once per executed simulation tick after stepping
    /// [`FreeCamera`].
    fn on_camera_tick(
        &mut self,
        _tick: Tick,
        _pose: &FreeCameraPose,
        _cursor_grab: CursorGrabMode,
    ) {
    }

    /// Diagnostic hook invoked when an optional interactive cursor-grab request
    /// is unsupported by the window system and degrades to [`CursorGrabMode::None`].
    fn on_cursor_grab_unavailable(&mut self, _error: &EngineError) {}

    /// Diagnostic hook invoked when catch-up clamping drops `dropped` backlog.
    fn on_backlog_dropped(&mut self, _dropped: Duration) {}

    /// Diagnostic hook invoked when validation messages are drained.
    fn on_validation_messages(&mut self, _messages: &[ValidationMessage], _dropped: u64) {}

    /// Diagnostic hook invoked on notable window lifecycle events (`Resized`,
    /// `Occluded`, `ScaleFactorChanged`, `CursorGrabChanged`, `Focused`).
    fn on_window_event(&mut self, _window: WindowId, _event: &WindowEvent) {}

    /// Diagnostic hook invoked once per frame step with current presentation
    /// state and cumulative elapsed time.
    fn on_frame_step(&mut self, _state: PresentationState, _elapsed: Duration) {}

    /// Diagnostic hook invoked on clean shutdown with pre-teardown and
    /// post-close [`GpuAllocatorStats`].
    fn on_shutdown_stats(
        &mut self,
        _active_stats: GpuAllocatorStats,
        _shutdown_stats: GpuAllocatorStats,
    ) {
    }
}

/// Synthesizes a minimal, structurally valid `.spnshdr` artifact matching the
/// `shaders/mesh_3d.slang` reflection layout (`MESH_3D_VS_ARTIFACT` or
/// `MESH_3D_FS_ARTIFACT`) for headless tests without `slangc`.
///
/// # Errors
///
/// Returns [`EngineErrorKind::Shader`] (`ENGN-0008`) if `artifact_name` is
/// unrecognized or encoding fails.
pub fn synthesize_minimal_mesh_3d_shader(
    shader_dir: &Path,
    artifact_name: &str,
) -> Result<Vec<u8>, EngineError> {
    let (vs, fs) = match artifact_name {
        MESH_3D_VS_ARTIFACT => {
            let shader = minimal_mesh_3d_vertex_cooked_shader();
            return shader.encode().map_err(|err| {
                EngineError::new(EngineErrorKind::Shader)
                    .with_detail(format!("encoding synthetic `{artifact_name}`"))
                    .with_cause(&err)
            });
        }
        MESH_3D_FS_ARTIFACT => (
            minimal_mesh_3d_vertex_cooked_shader(),
            minimal_mesh_3d_fragment_cooked_shader(),
        ),
        other => {
            let full = shader_dir.join(other);
            return Err(
                EngineError::new(EngineErrorKind::Shader).with_detail(format!(
                    "file `{}`: unknown cooked shader artifact `{other}`; {SHADER_COOK_HINT}",
                    full.display()
                )),
            );
        }
    };

    validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &MESH_VERTEX_ATTRIBUTES,
            push_constants: MESH_PUSH_CONSTANT_RANGE,
            descriptor_bindings: &[],
        },
    )
    .map_err(|err| {
        EngineError::new(EngineErrorKind::Shader)
            .with_detail("validating synthetic mesh_3d shader layout")
            .with_cause(&err)
    })?;

    fs.encode().map_err(|err| {
        EngineError::new(EngineErrorKind::Shader)
            .with_detail(format!("encoding synthetic `{artifact_name}`"))
            .with_cause(&err)
    })
}

fn minimal_mesh_3d_vertex_cooked_shader() -> CookedShader {
    let spirv_words = vec![
        SPIRV_MAGIC,
        0x0001_0600,
        0,
        8,
        0,
        (2 << 16) | 17, // OpCapability Shader
        1,
        (3 << 16) | 14, // OpMemoryModel Logical GLSL450
        0,
        1,
        (5 << 16) | 15, // OpEntryPoint Vertex %4 "vs_main"
        0,
        4,
        u32::from_le_bytes(*b"vs_m"),
        u32::from_le_bytes(*b"ain\0"),
    ];
    CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Vertex,
        entry_point: "vs_main".to_owned(),
        push_constants: MESH_PUSH_CONSTANT_RANGE,
        descriptor_bindings: Vec::new(),
        vertex_inputs: MESH_VERTEX_ATTRIBUTES.to_vec(),
        spirv_words,
    }
}

fn minimal_mesh_3d_fragment_cooked_shader() -> CookedShader {
    let spirv_words = vec![
        SPIRV_MAGIC,
        0x0001_0600,
        0,
        8,
        0,
        (2 << 16) | 17, // OpCapability Shader
        1,
        (3 << 16) | 14, // OpMemoryModel Logical GLSL450
        0,
        1,
        (5 << 16) | 15, // OpEntryPoint Fragment %4 "fs_main"
        4,
        4,
        u32::from_le_bytes(*b"fs_m"),
        u32::from_le_bytes(*b"ain\0"),
    ];
    CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Fragment,
        entry_point: "fs_main".to_owned(),
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: Vec::new(),
        vertex_inputs: Vec::new(),
        spirv_words,
    }
}

#[derive(Default)]
struct SceneGpuHandles {
    shaders: Vec<ShaderHandle>,
    pipeline: Option<PipelineHandle>,
    meshes: Vec<MeshHandle>,
}

/// Runs initialization, the 60 Hz `FreeCamera` + `alpha`-interpolated 3D scene
/// loop, and strict reverse-order teardown
/// (`MeshHandles (rev) -> PipelineHandle -> ShaderHandles (rev) -> Renderer -> Window -> Platform`)
/// over `backend`.
///
/// # Errors
///
/// Returns a typed [`EngineError`] if option validation, subsystem startup,
/// cooked-shader loading/validation, pipeline/mesh initialization, event
/// pumping, clock advancement, rendering, or graphics validation fails. Every
/// resource initialized prior to a failure is released once in exact reverse
/// ownership order before returning (`FND-FR-10`, `F3D-FR-12`).
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
    validate_shader_dir_path(&options.shader_dir)?;

    let mut camera = FreeCamera::new(options.camera)?;
    let (mut input, bindings) = CameraInputBindings::build_default_input()?;
    let mut clock = FixedStepClock::new(options.fixed_step);
    let scene = ProceduralScene::default_scene()?;

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

        let mut gpu_handles = SceneGpuHandles::default();
        let session_result = init_and_drive_scene(
            options,
            backend,
            &mut platform,
            window_id,
            initial_size,
            &mut renderer,
            &mut gpu_handles,
            &scene,
            &mut clock,
            &mut input,
            &bindings,
            &mut camera,
        );

        let active_stats = renderer.allocator_stats();

        // Explicit reverse-order teardown Steps 1..3:
        // uploaded MeshHandles (reverse) -> PipelineHandle -> ShaderHandles (reverse).
        let handle_teardown_err = teardown_scene_handles(&mut renderer, &mut gpu_handles);

        let mut trailing_messages = Vec::new();
        let dropped_messages = renderer.drain_validation_messages(&mut trailing_messages);
        if !trailing_messages.is_empty() || dropped_messages > 0 {
            backend.on_validation_messages(&trailing_messages, dropped_messages);
        }
        let validation_errors = renderer.validation_error_count();

        // Explicit reverse-order teardown Step 4: close/drop Renderer before
        // the borrowed Window.
        let close_result = renderer.close();

        match session_result {
            Err(primary_error) => Err(primary_error),
            Ok(_) if handle_teardown_err.is_some() => {
                if let Some(err) = handle_teardown_err {
                    Err(err)
                } else {
                    Err(EngineError::new(EngineErrorKind::Render))
                }
            }
            Ok(_) if close_result.is_err() => match close_result {
                Err(err) => Err(err),
                Ok(_) => Err(EngineError::new(EngineErrorKind::Render)),
            },
            Ok(_) if validation_errors > 0 => Err(EngineError::new(EngineErrorKind::Validation)
                .with_detail(format!(
                    "graphics validation layer reported {validation_errors} error(s)"
                ))),
            Ok(mut summary) => {
                let shutdown_stats = close_result.unwrap_or_default();
                summary.active_allocator_stats = active_stats;
                summary.shutdown_allocator_stats = shutdown_stats;
                backend.on_shutdown_stats(active_stats, shutdown_stats);
                Ok(summary)
            }
        }
    };

    // Explicit reverse-order teardown Steps 5 & 6: native Window, then Platform
    // display connection.
    drop(input);
    drop(window);
    drop(platform);

    result
}

#[allow(clippy::too_many_arguments)]
fn init_and_drive_scene<B: RuntimeBackend>(
    options: &RuntimeOptions,
    backend: &mut B,
    platform: &mut B::Platform,
    window_id: WindowId,
    initial_size: PhysicalSize,
    renderer: &mut B::Renderer<'_>,
    gpu_handles: &mut SceneGpuHandles,
    scene: &ProceduralScene,
    clock: &mut FixedStepClock,
    input: &mut InputState,
    bindings: &CameraInputBindings,
    camera: &mut FreeCamera,
) -> Result<SessionSummary, EngineError> {
    let vs_bytes = backend.read_cooked_shader(&options.shader_dir, MESH_3D_VS_ARTIFACT)?;
    let vs_label = options
        .shader_dir
        .join(MESH_3D_VS_ARTIFACT)
        .to_string_lossy()
        .into_owned();
    let vs_handle = renderer.load_shader(&vs_label, &vs_bytes)?;
    gpu_handles.shaders.push(vs_handle);

    let fs_bytes = backend.read_cooked_shader(&options.shader_dir, MESH_3D_FS_ARTIFACT)?;
    let fs_label = options
        .shader_dir
        .join(MESH_3D_FS_ARTIFACT)
        .to_string_lossy()
        .into_owned();
    let fs_handle = renderer.load_shader(&fs_label, &fs_bytes)?;
    gpu_handles.shaders.push(fs_handle);

    let pipeline_desc = GraphicsPipelineDescriptor::new_mesh_3d("mesh_3d", vs_handle, fs_handle);
    let pipeline_handle = renderer.create_graphics_pipeline(&pipeline_desc)?;
    gpu_handles.pipeline = Some(pipeline_handle);

    gpu_handles.meshes.reserve(scene.meshes().len());
    for mesh_asset in scene.meshes() {
        let handle = renderer.upload_mesh(&mesh_asset.descriptor())?;
        gpu_handles.meshes.push(handle);
    }

    backend.on_scene_initialized(
        gpu_handles.meshes.len(),
        scene.instances().len(),
        renderer.allocator_stats(),
    );

    // Anchor monotonic clock after platform, window, renderer, shader, pipeline,
    // and mesh initialization so startup latency is never treated as a stall.
    let _ = backend.read_elapsed();

    drive_loop(
        options,
        backend,
        platform,
        window_id,
        initial_size,
        renderer,
        pipeline_handle,
        &gpu_handles.meshes,
        scene,
        clock,
        input,
        bindings,
        camera,
    )
}

fn teardown_scene_handles<R: RenderSurface>(
    renderer: &mut R,
    gpu_handles: &mut SceneGpuHandles,
) -> Option<EngineError> {
    let mut first_error: Option<EngineError> = None;

    for mesh in gpu_handles.meshes.drain(..).rev() {
        if let Err(err) = renderer.destroy_mesh(mesh)
            && first_error.is_none()
        {
            first_error = Some(err);
        }
    }

    if let Some(pipeline) = gpu_handles.pipeline.take()
        && let Err(err) = renderer.destroy_pipeline(pipeline)
        && first_error.is_none()
    {
        first_error = Some(err);
    }

    for shader in gpu_handles.shaders.drain(..).rev() {
        if let Err(err) = renderer.destroy_shader(shader)
            && first_error.is_none()
        {
            first_error = Some(err);
        }
    }

    first_error
}

#[allow(clippy::too_many_arguments)]
fn drive_loop<B: RuntimeBackend>(
    options: &RuntimeOptions,
    backend: &mut B,
    platform: &mut B::Platform,
    window_id: WindowId,
    initial_size: PhysicalSize,
    renderer: &mut B::Renderer<'_>,
    pipeline: PipelineHandle,
    mesh_handles: &[MeshHandle],
    scene: &ProceduralScene,
    clock: &mut FixedStepClock,
    input: &mut InputState,
    bindings: &CameraInputBindings,
    camera: &mut FreeCamera,
) -> Result<SessionSummary, EngineError> {
    let mut cursor_grab = CursorGrabMode::None;
    if let Some(mode) = options.smoke_cursor_grab {
        backend.set_cursor_grab(platform, window_id, mode)?;
        cursor_grab = mode;
    }

    let mut last_valid_aspect = if !initial_size.is_empty() {
        (initial_size.width as f32) / (initial_size.height as f32)
    } else {
        (options.window_size.width as f32) / (options.window_size.height as f32)
    };

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
        final_cursor_grab: cursor_grab,
        active_allocator_stats: GpuAllocatorStats::default(),
        shutdown_allocator_stats: GpuAllocatorStats::default(),
    };

    let mut total_elapsed = Duration::ZERO;
    let mut smoke_grab_released = options
        .smoke_cursor_grab
        .is_none_or(|mode| mode == CursorGrabMode::None);
    let mut occluded = false;
    let mut idle = renderer.state() == PresentationState::Suspended;
    let mut exit_reason: Option<ExitReason> = None;
    let mut events = Vec::new();
    let mut draws = Vec::with_capacity(scene.instances().len());
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
                            if !size.is_empty() {
                                last_valid_aspect = (size.width as f32) / (size.height as f32);
                            }
                        }
                        WindowEvent::Occluded(hidden) => {
                            occluded = *hidden;
                        }
                        WindowEvent::Focused(false) => {
                            if cursor_grab != CursorGrabMode::None {
                                cursor_grab = CursorGrabMode::None;
                                if let Err(err) = backend.set_cursor_grab(
                                    platform,
                                    window_id,
                                    CursorGrabMode::None,
                                ) {
                                    if err.is_recoverable_cursor_grab() {
                                        backend.on_cursor_grab_unavailable(&err);
                                    } else {
                                        return Err(err);
                                    }
                                }
                            }
                        }
                        WindowEvent::CursorGrabChanged(mode) => {
                            cursor_grab = *mode;
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

        let tick_duration = clock.config().tick_duration();
        for tick in update.ticks() {
            let command = input.sample_tick(tick);
            summary.ticks_executed += 1;
            summary.last_tick = Some(tick);
            backend.on_tick(&command);

            let was_locked = cursor_grab == CursorGrabMode::Locked;
            if command.is_pressed(bindings.toggle_cursor_grab) {
                let target_mode = if was_locked {
                    CursorGrabMode::None
                } else {
                    CursorGrabMode::Locked
                };
                match backend.set_cursor_grab(platform, window_id, target_mode) {
                    Ok(()) => {
                        cursor_grab = target_mode;
                    }
                    Err(err) if err.is_recoverable_cursor_grab() => {
                        cursor_grab = CursorGrabMode::None;
                        backend.on_cursor_grab_unavailable(&err);
                    }
                    Err(err) => return Err(err),
                }
            }

            let look_enabled = was_locked && cursor_grab == CursorGrabMode::Locked;
            let cam_input = bindings.sample_camera_input(&command, look_enabled);
            camera.step_tick(tick, tick_duration, cam_input);
            backend.on_camera_tick(tick, camera.curr_pose(), cursor_grab);

            if command.is_pressed(bindings.exit) {
                exit_reason.get_or_insert(ExitReason::ExitAction);
            }
        }

        if let Some(reason) = exit_reason {
            summary.exit_reason = reason;
            summary.final_cursor_grab = cursor_grab;
            break;
        }

        if !occluded {
            let alpha = interpolation_alpha(clock);
            let presentation = camera.sample_presentation(alpha, last_valid_aspect)?;
            scene.extract_draws(mesh_handles, &presentation, &mut draws)?;

            match renderer.render_meshes(
                pipeline,
                options.clear_color,
                &presentation.view,
                &draws,
            )? {
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
            cursor_grab = CursorGrabMode::None;
            smoke_grab_released = true;
        }

        if options
            .auto_close_after
            .is_some_and(|limit| total_elapsed >= limit)
        {
            summary.exit_reason = ExitReason::AutoClose;
            summary.final_cursor_grab = cursor_grab;
            break;
        }
    }

    Ok(summary)
}
