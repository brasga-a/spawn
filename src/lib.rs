//! `spawn-engine`: composition root of the Spawn foundation and Milestone 2 3D
//! free-camera runtime slice.
//!
//! Wires together `spawn-core` (`FixedStepClock`), `spawn-math` (`f64`
//! `WorldPos`, `libm`-backed deterministic `det` camera math, and reversed-Z
//! perspective projections), `spawn-platform` (native window, raw event loop,
//! and cursor constraints), `spawn-input` (device tracking and tick-boundary
//! command sampling), and `spawn-render` (cooked-shader validation, Vulkan 1.3
//! `dynamicRendering` 3D mesh pipelines, staged `DEVICE_LOCAL` buffers, and
//! swapchain recovery) without introducing upward or lateral dependencies
//! between engine crates (`SPN-INV-001`..`SPN-INV-006`).
//!
//! Diagnostics are written to `stderr` via `std` only until `spawn-profiler`
//! provides the structured logging facade (ADR-0009).

#![forbid(unsafe_code)]

mod camera;
mod error;
mod native;
mod options;
mod runtime;
mod scene;

pub use camera::{
    CameraInputBindings, CameraPresentation, CameraTickInput, FreeCamera, FreeCameraConfig,
    FreeCameraPose, MAX_CAMERA_MOVE_SPEED_MPS, MAX_LOOK_RADIANS_PER_COUNT, interpolation_alpha,
};
pub use error::{EngineError, EngineErrorKind, SHADER_COOK_HINT};
pub use native::run_native;
pub use options::{DEFAULT_SHADER_DIR, RuntimeOptions};
pub use runtime::{
    ExitReason, RenderSurface, RuntimeBackend, SessionSummary, WindowSurface, run_session,
    synthesize_minimal_mesh_3d_shader,
};
pub use scene::{
    MESH_3D_FS_ARTIFACT, MESH_3D_VS_ARTIFACT, ProceduralMesh, ProceduralScene, SceneInstance,
    build_ground_grid_mesh, build_unit_cube_mesh, build_unit_octahedron_mesh,
};

/// Validates environment options and executes the native runtime.
///
/// # Errors
///
/// Returns a located, coded [`EngineError`] if options are invalid, no native
/// backend is available on this OS, or initialization/runtime execution fails.
pub fn run() -> Result<SessionSummary, EngineError> {
    let options = RuntimeOptions::from_env()?;
    run_native(&options)
}
