//! Genre-free 60 Hz `f64` free-fly camera simulation and read-only `alpha`
//! presentation interpolation (`ADR-0003`, `ADR-0010`, `SPN-INV-003`,
//! `SPN-INV-040`..`SPN-INV-044`, `SPN-INV-050`..`SPN-INV-054`, `F3D-FR-09`,
//! `F3D-FR-10`).
//!
//! ## Simulation vs. presentation boundary (`SPN-INV-043`)
//!
//! - [`FreeCamera::step_tick`] mutates simulation poses **only** on discrete
//!   [`FixedStepClock`] ticks (`prev_pose = curr_pose`, then deterministic yaw,
//!   clamped pitch, intrinsic `YXZ` unit [`Quat`], and `f64` [`WorldPos`]
//!   displacement using `spawn_math::det` exclusively).
//! - [`FreeCamera::sample_presentation`] takes `&self` (read-only), blends
//!   `prev_pose` and `curr_pose` by `alpha ∈ [0.0, 1.0)` (`f64` world-position
//!   `lerp` + shortest-arc `Quat::slerp`), constructs a [`CameraRelativeView`]
//!   with reversed-Z projection and single-site Vulkan NDC Y-flip, and extracts
//!   camera-relative `f32` [`MeshDraw`] items (`SPN-INV-052`).

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::time::Duration;

use spawn_core::{FixedStepClock, Tick};
use spawn_input::{
    ActionId, AxisConfig, AxisId, AxisSource, ButtonTrigger, InputCommand, InputMap, InputState,
    MotionAxis,
};
use spawn_math::{DVec3, Mat4, Quat, WorldPos, det, radians_from_degrees_f32};
use spawn_platform::{KeyCode, MouseButton};
use spawn_render::{CameraRelativeView, MeshDraw, MeshHandle};

use crate::error::{EngineError, EngineErrorKind};

/// Upper sanity cap on free-fly camera linear speed (`m/s`) so per-tick `f64`
/// integration remains bounded and finite.
pub const MAX_CAMERA_MOVE_SPEED_MPS: f64 = 1_000_000.0;

/// Upper sanity cap on look sensitivity (`radians` per raw pointer count).
pub const MAX_LOOK_RADIANS_PER_COUNT: f64 = std::f64::consts::PI;

/// Largest IEEE-754 `f64` strictly less than `1.0` (`1.0 - 2^-53`), used to
/// clamp `pending / tick_duration` when `tick_duration` exceeds `2^53` ns.
const MAX_SUB_UNIT_ALPHA_F64: f64 = f64::from_bits(1.0_f64.to_bits() - 1);

/// Configuration for a genre-free [`FreeCamera`] (`SPN-INV-003`, `SPN-INV-050`,
/// `SPN-INV-054`).
///
/// All internal angle fields are in **radians** (`fov_y_radians`,
/// `look_radians_per_count`, `pitch_limit_radians`, `initial_yaw_radians`,
/// `initial_pitch_radians`). Degree-accepting constructors
/// ([`Self::from_fov_degrees`], [`Self::with_fov_degrees`]) name the unit
/// explicitly and convert once at the boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeCameraConfig {
    /// Vertical perspective field of view in radians (`(0, π)`).
    pub fov_y_radians: f32,
    /// Positive near clip distance in meters (`near -> 1.0` in reversed-Z).
    pub near: f32,
    /// Finite far clip distance in meters (`far > near`, `far -> 0.0` in reversed-Z).
    pub far: f32,
    /// Maximum translation speed in meters per second (`(0, MAX_CAMERA_MOVE_SPEED_MPS]`).
    pub move_speed_mps: f64,
    /// Radians rotated per raw unaccelerated pointer motion count (`(0, π]`).
    pub look_radians_per_count: f64,
    /// Maximum absolute pitch angle in radians, strictly `< π/2` to prevent pole
    /// singularities (`(0, π/2)`).
    pub pitch_limit_radians: f64,
    /// Initial `f64` world-space eye position in meters.
    pub initial_world_pos: WorldPos,
    /// Initial yaw angle around world `+Y` in radians (`0.0` looks down `-Z`).
    pub initial_yaw_radians: f64,
    /// Initial pitch angle around local `+X` in radians (`0.0` is level).
    pub initial_pitch_radians: f64,
}

impl Default for FreeCameraConfig {
    fn default() -> Self {
        Self {
            fov_y_radians: radians_from_degrees_f32(70.0),
            near: 0.1,
            far: 1_000.0,
            move_speed_mps: 6.0,
            look_radians_per_count: 0.0022,
            pitch_limit_radians: std::f64::consts::FRAC_PI_2 - det::DEFAULT_PITCH_LIMIT_EPSILON_F64,
            initial_world_pos: WorldPos::new(0.0, 1.5, 0.0),
            initial_yaw_radians: 0.0,
            initial_pitch_radians: 0.0,
        }
    }
}

impl FreeCameraConfig {
    /// Constructs a configuration converting `fov_y_degrees` to radians once at
    /// the boundary (`SPN-INV-054`).
    #[must_use]
    pub fn from_fov_degrees(fov_y_degrees: f32, near: f32, far: f32) -> Self {
        Self {
            fov_y_radians: radians_from_degrees_f32(fov_y_degrees),
            near,
            far,
            ..Self::default()
        }
    }

    /// Updates `fov_y_radians` from an explicit degree value (`SPN-INV-054`).
    #[must_use]
    pub const fn with_fov_degrees(mut self, fov_y_degrees: f32) -> Self {
        self.fov_y_radians = radians_from_degrees_f32(fov_y_degrees);
        self
    }

    /// Sets the initial world-space camera position (`f64` meters).
    #[must_use]
    pub const fn with_initial_world_pos(mut self, world_pos: WorldPos) -> Self {
        self.initial_world_pos = world_pos;
        self
    }

    /// Sets initial `(yaw_radians, pitch_radians)` angles.
    #[must_use]
    pub const fn with_initial_yaw_pitch_radians(
        mut self,
        yaw_radians: f64,
        pitch_radians: f64,
    ) -> Self {
        self.initial_yaw_radians = yaw_radians;
        self.initial_pitch_radians = pitch_radians;
        self
    }

    /// Validates projection parameters, speed/sensitivity ranges, strict `< π/2`
    /// pole margin, and initial pose finiteness without panicking.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Camera`] (`ENGN-0009`) on the first invalid
    /// field.
    #[track_caller]
    pub fn validate(&self) -> Result<(), EngineError> {
        Mat4::perspective_reversed_z_rh(self.fov_y_radians, 1.0, self.near, self.far).map_err(
            |err| {
                EngineError::new(EngineErrorKind::Camera)
                    .with_detail(format!(
                        "invalid camera projection config (fov_y_radians={}, near={}, far={})",
                        self.fov_y_radians, self.near, self.far
                    ))
                    .with_cause(&err)
            },
        )?;

        if !self.move_speed_mps.is_finite()
            || self.move_speed_mps <= 0.0
            || self.move_speed_mps > MAX_CAMERA_MOVE_SPEED_MPS
        {
            return Err(EngineError::new(EngineErrorKind::Camera).with_detail(format!(
                "field `move_speed_mps` must be finite and in (0.0, {MAX_CAMERA_MOVE_SPEED_MPS}], got {}",
                self.move_speed_mps
            )));
        }

        if !self.look_radians_per_count.is_finite()
            || self.look_radians_per_count <= 0.0
            || self.look_radians_per_count > MAX_LOOK_RADIANS_PER_COUNT
        {
            return Err(EngineError::new(EngineErrorKind::Camera).with_detail(format!(
                "field `look_radians_per_count` must be finite and in (0.0, {MAX_LOOK_RADIANS_PER_COUNT}], got {}",
                self.look_radians_per_count
            )));
        }

        if !self.pitch_limit_radians.is_finite()
            || self.pitch_limit_radians <= 0.0
            || self.pitch_limit_radians >= std::f64::consts::FRAC_PI_2
            || det::cos_f64(self.pitch_limit_radians) <= 0.0
        {
            return Err(EngineError::new(EngineErrorKind::Camera).with_detail(format!(
                "field `pitch_limit_radians` must be finite and strictly in (0.0, pi/2), got {}",
                self.pitch_limit_radians
            )));
        }

        if !self.initial_world_pos.is_finite() {
            return Err(
                EngineError::new(EngineErrorKind::Camera).with_detail(format!(
                    "field `initial_world_pos` must contain finite coordinates, got {:?}",
                    self.initial_world_pos
                )),
            );
        }

        if !self.initial_yaw_radians.is_finite() || !self.initial_pitch_radians.is_finite() {
            return Err(
                EngineError::new(EngineErrorKind::Camera).with_detail(format!(
                    "initial yaw/pitch must be finite radians, got yaw={}, pitch={}",
                    self.initial_yaw_radians, self.initial_pitch_radians
                )),
            );
        }

        Ok(())
    }
}

/// Authoritative fixed-tick simulation pose of a [`FreeCamera`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeCameraPose {
    /// Absolute `f64` world-space camera position in meters (`SPN-INV-051`).
    pub world_pos: WorldPos,
    /// Wrapped yaw angle around world `+Y` in `(-π, π]` radians (`0.0` looks down `-Z`).
    pub yaw_radians: f64,
    /// Clamped pitch angle around local `+X` in `[-pitch_limit, +pitch_limit]` radians.
    pub pitch_radians: f64,
    /// Canonical unit rotation quaternion (`YXZ` intrinsic order, `SPN-INV-044`).
    pub orientation: Quat,
}

impl FreeCameraPose {
    /// Returns `true` when every `f64` and `f32` scalar in `self` and `other`
    /// has identical IEEE-754 bit patterns (`to_bits()`).
    #[must_use]
    pub fn bitwise_eq(&self, other: &Self) -> bool {
        self.world_pos.x.to_bits() == other.world_pos.x.to_bits()
            && self.world_pos.y.to_bits() == other.world_pos.y.to_bits()
            && self.world_pos.z.to_bits() == other.world_pos.z.to_bits()
            && self.yaw_radians.to_bits() == other.yaw_radians.to_bits()
            && self.pitch_radians.to_bits() == other.pitch_radians.to_bits()
            && self.orientation.x.to_bits() == other.orientation.x.to_bits()
            && self.orientation.y.to_bits() == other.orientation.y.to_bits()
            && self.orientation.z.to_bits() == other.orientation.z.to_bits()
            && self.orientation.w.to_bits() == other.orientation.w.to_bits()
    }

    /// Returns `true` when `world_pos`, `yaw_radians`, `pitch_radians`, and
    /// `orientation` are all finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.world_pos.is_finite()
            && self.yaw_radians.is_finite()
            && self.pitch_radians.is_finite()
            && self.orientation.is_finite()
    }
}

/// Genre-free per-tick intent consumed by [`FreeCamera::step_tick`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CameraTickInput {
    /// Local forward (`+1.0`, `-Z`) / backward (`-1.0`, `+Z`) axis intent.
    pub move_forward_back: f64,
    /// Local right (`+1.0`, `+X`) / left (`-1.0`, `-X`) axis intent.
    pub move_right_left: f64,
    /// Local up (`+1.0`, `+Y`) / down (`-1.0`, `-Y`) axis intent.
    pub move_up_down: f64,
    /// Unaccelerated horizontal pointer delta (`+` = pointer moved right).
    pub look_yaw_delta: f64,
    /// Unaccelerated vertical pointer delta (`+` = pointer moved down; `-` = pointer moved up).
    pub look_pitch_delta: f64,
    /// Whether `look_yaw_delta` and `look_pitch_delta` rotate the camera on this tick
    /// (true while `CursorGrabMode::Locked` is active or explicitly enabled in headless tests).
    pub look_enabled: bool,
}

impl CameraTickInput {
    /// Idle tick input with zero translation and zero rotation.
    pub const IDLE: Self = Self {
        move_forward_back: 0.0,
        move_right_left: 0.0,
        move_up_down: 0.0,
        look_yaw_delta: 0.0,
        look_pitch_delta: 0.0,
        look_enabled: false,
    };
}

/// Read-only interpolated presentation snapshot produced by
/// [`FreeCamera::sample_presentation`] (`SPN-INV-043`, `SPN-INV-052`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPresentation {
    /// Sub-tick interpolation factor in `[0.0, 1.0)`.
    pub alpha: f64,
    /// Interpolated `f64` world-space eye position (`lerp(prev, curr, alpha)`).
    pub world_pos: WorldPos,
    /// Interpolated unit orientation (`Quat::slerp(prev, curr, alpha as f32)`).
    pub orientation: Quat,
    /// Camera-relative view + reversed-Z projection ready for `spawn-render`.
    pub view: CameraRelativeView,
}

impl CameraPresentation {
    /// Builds a camera-relative [`MeshDraw`] by subtracting `self.world_pos`
    /// from `object_world_pos` in `f64` on the CPU before narrowing to `f32`
    /// (`SPN-INV-052`).
    #[must_use]
    #[inline]
    pub const fn extract_draw(&self, mesh: MeshHandle, object_world_pos: WorldPos) -> MeshDraw {
        MeshDraw::from_world(mesh, object_world_pos, self.world_pos)
    }
}

/// Genre-free 60 Hz `f64` free-fly simulation camera (`SPN-INV-003`,
/// `SPN-INV-040`..`SPN-INV-044`).
#[derive(Debug, Clone)]
pub struct FreeCamera {
    config: FreeCameraConfig,
    prev_pose: FreeCameraPose,
    curr_pose: FreeCameraPose,
    last_tick: Option<Tick>,
}

impl FreeCamera {
    /// Validates `config` and constructs a stationary [`FreeCamera`] with
    /// `prev_pose == curr_pose` at the configured initial pose.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Camera`] (`ENGN-0009`) if `config` fails
    /// [`FreeCameraConfig::validate`].
    #[track_caller]
    pub fn new(config: FreeCameraConfig) -> Result<Self, EngineError> {
        config.validate()?;
        let yaw = canonical_zero_f64(det::wrap_angle_pi_f64(config.initial_yaw_radians));
        let pitch = canonical_zero_f64(
            config
                .initial_pitch_radians
                .clamp(-config.pitch_limit_radians, config.pitch_limit_radians),
        );
        let orientation = det::quat_from_yaw_pitch_f64(yaw, pitch);
        let initial_pose = FreeCameraPose {
            world_pos: canonical_world_pos(config.initial_world_pos),
            yaw_radians: yaw,
            pitch_radians: pitch,
            orientation,
        };
        Ok(Self {
            config,
            prev_pose: initial_pose,
            curr_pose: initial_pose,
            last_tick: None,
        })
    }

    /// Validated configuration of this camera.
    pub const fn config(&self) -> &FreeCameraConfig {
        &self.config
    }

    /// Simulation pose prior to the most recent executed [`Tick`] (equals
    /// [`Self::curr_pose`] before the first tick runs).
    pub const fn prev_pose(&self) -> &FreeCameraPose {
        &self.prev_pose
    }

    /// Latest authoritative simulation pose as of [`Self::last_tick`].
    pub const fn curr_pose(&self) -> &FreeCameraPose {
        &self.curr_pose
    }

    /// Most recent simulation [`Tick`] processed by [`Self::step_tick`], or
    /// `None` before the first tick executes.
    pub const fn last_tick(&self) -> Option<Tick> {
        self.last_tick
    }

    /// Advances simulation state by exactly one fixed `tick` of duration
    /// `tick_duration` (`SPN-INV-040`, `SPN-INV-044`).
    ///
    /// Uses **only** `spawn_math::det` (`libm` transcendentals + IEEE-754 `sqrt`)
    /// and unfused IEEE-754 arithmetic:
    /// 1. Latches `prev_pose = curr_pose`.
    /// 2. Integrates right-handed yaw/pitch when `input.look_enabled` (`dx > 0`
    ///    pointer-right decreases `+Y` yaw to turn right toward `+X`; `dy < 0`
    ///    pointer-up increases `+X` pitch to look up toward `+Y`), wrapping yaw
    ///    into `(-π, π]` and clamping pitch to `[-pitch_limit, +pitch_limit]`.
    /// 3. Normalizes 3D translation intent whenever its magnitude exceeds `1.0`
    ///    so diagonal movement never exceeds `move_speed_mps`, and integrates
    ///    displacement into `f64` [`WorldPos`].
    pub fn step_tick(&mut self, tick: Tick, tick_duration: Duration, input: CameraTickInput) {
        self.prev_pose = self.curr_pose;

        let (yaw, pitch) = if input.look_enabled {
            let dyaw = sanitize_f64(input.look_yaw_delta) * self.config.look_radians_per_count;
            let dpitch = sanitize_f64(input.look_pitch_delta) * self.config.look_radians_per_count;
            let wrapped_yaw =
                canonical_zero_f64(det::wrap_angle_pi_f64(self.curr_pose.yaw_radians - dyaw));
            let clamped_pitch = canonical_zero_f64((self.curr_pose.pitch_radians - dpitch).clamp(
                -self.config.pitch_limit_radians,
                self.config.pitch_limit_radians,
            ));
            (wrapped_yaw, clamped_pitch)
        } else {
            (self.curr_pose.yaw_radians, self.curr_pose.pitch_radians)
        };

        let orientation = det::quat_from_yaw_pitch_f64(yaw, pitch);

        let right_left = sanitize_f64(input.move_right_left).clamp(-1.0, 1.0);
        let up_down = sanitize_f64(input.move_up_down).clamp(-1.0, 1.0);
        let forward_back = sanitize_f64(input.move_forward_back).clamp(-1.0, 1.0);

        let len_sq = right_left * right_left + up_down * up_down + forward_back * forward_back;
        let world_pos = if len_sq > 0.0 {
            let (nx, ny, nz) = if len_sq > 1.0 {
                let inv_len = 1.0 / det::sqrt_f64(len_sq);
                (
                    right_left * inv_len,
                    up_down * inv_len,
                    forward_back * inv_len,
                )
            } else {
                (right_left, up_down, forward_back)
            };
            let basis = det::yaw_pitch_basis_f64(yaw, pitch);
            let world_dir: DVec3 = basis.local_vector(nx, ny, nz);
            let dt_seconds = duration_seconds_f64(tick_duration);
            let distance = self.config.move_speed_mps * dt_seconds;
            let displacement = world_dir * distance;
            if displacement.is_finite() {
                canonical_world_pos(self.curr_pose.world_pos + displacement)
            } else {
                self.curr_pose.world_pos
            }
        } else {
            self.curr_pose.world_pos
        };

        self.curr_pose = FreeCameraPose {
            world_pos,
            yaw_radians: yaw,
            pitch_radians: pitch,
            orientation,
        };
        self.last_tick = Some(tick);
    }

    /// Computes read-only presentation state for `alpha ∈ [0.0, 1.0)` and
    /// drawable `aspect_ratio` (`width / height`) without mutating simulation
    /// state (`SPN-INV-043`, `SPN-INV-052`, `SPN-INV-053`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Camera`] (`ENGN-0009`) if `alpha` is outside
    /// `[0.0, 1.0)` or if `aspect_ratio` is non-finite or `<= 0.0`.
    #[track_caller]
    pub fn sample_presentation(
        &self,
        alpha: f64,
        aspect_ratio: f32,
    ) -> Result<CameraPresentation, EngineError> {
        if !alpha.is_finite() || !(0.0..1.0).contains(&alpha) {
            return Err(
                EngineError::new(EngineErrorKind::Camera).with_detail(format!(
                    "presentation interpolation alpha must be finite and in [0.0, 1.0), got {alpha}"
                )),
            );
        }

        let projection = Mat4::perspective_reversed_z_rh(
            self.config.fov_y_radians,
            aspect_ratio,
            self.config.near,
            self.config.far,
        )
        .map_err(|err| {
            EngineError::new(EngineErrorKind::Camera)
                .with_detail(format!(
                    "building reversed-Z perspective projection (aspect_ratio={aspect_ratio})"
                ))
                .with_cause(&err)
        })?;

        let world_pos = self
            .prev_pose
            .world_pos
            .lerp(self.curr_pose.world_pos, alpha);
        let orientation = self
            .prev_pose
            .orientation
            .slerp(self.curr_pose.orientation, alpha as f32);
        let view = CameraRelativeView::new(projection, orientation);

        Ok(CameraPresentation {
            alpha,
            world_pos,
            orientation,
            view,
        })
    }
}

/// Computes the sub-tick presentation interpolation factor
/// `alpha = pending / tick_duration` in `[0.0, 1.0)` (`F3D-FR-10`).
#[must_use]
#[inline]
pub fn interpolation_alpha(clock: &FixedStepClock) -> f64 {
    let tick_nanos = clock.config().tick_duration().as_nanos();
    if tick_nanos == 0 {
        return 0.0;
    }
    let pending_nanos = clock.pending().as_nanos();
    let raw = (pending_nanos as f64) / (tick_nanos as f64);
    if !raw.is_finite() || raw <= 0.0 {
        0.0
    } else if raw >= 1.0 {
        MAX_SUB_UNIT_ALPHA_F64
    } else {
        raw
    }
}

/// Genre-free semantic [`ActionId`] and [`AxisId`] bindings configured by the
/// `spawn-engine` composition root (`DESIGN.md` §5.1, `SPN-INV-003`).
///
/// `spawn-input` itself defines zero built-in actions or axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraInputBindings {
    /// Clean session exit (`KeyCode::Escape`).
    pub exit: ActionId,
    /// Toggles `CursorGrabMode::Locked` ↔ `CursorGrabMode::None`
    /// (`MouseButton::Right` and `KeyCode::Tab`).
    pub toggle_cursor_grab: ActionId,
    /// Local `-Z` / `+Z` translation axis (`KeyCode::W` `+1`, `KeyCode::S` `-1`).
    pub move_forward_back: AxisId,
    /// Local `+X` / `-X` translation axis (`KeyCode::D` `+1`, `KeyCode::A` `-1`).
    pub move_right_left: AxisId,
    /// Local `+Y` / `-Y` translation axis (`KeyCode::Space` `+1`, `KeyCode::ControlLeft` `-1`).
    pub move_up_down: AxisId,
    /// Unaccelerated relative horizontal pointer motion (`AxisSource::PointerRelative(MotionAxis::X)`).
    pub look_yaw: AxisId,
    /// Unaccelerated relative vertical pointer motion (`AxisSource::PointerRelative(MotionAxis::Y)`).
    pub look_pitch: AxisId,
}

impl CameraInputBindings {
    /// Builds the composition-root [`InputState`] and [`CameraInputBindings`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Input`] (`ENGN-0007`) if action/axis slot
    /// allocation or binding registration fails.
    #[track_caller]
    pub fn build_default_input() -> Result<(InputState, Self), EngineError> {
        let map_err = |detail: &'static str, err: spawn_input::InputError| {
            EngineError::new(EngineErrorKind::Input)
                .with_detail(detail)
                .with_cause(&err)
        };

        let exit = ActionId::new(0).map_err(|e| map_err("allocating Exit action", e))?;
        let toggle_cursor_grab =
            ActionId::new(1).map_err(|e| map_err("allocating ToggleCursorGrab action", e))?;

        let move_forward_back =
            AxisId::new(0).map_err(|e| map_err("allocating MoveForwardBack axis", e))?;
        let move_right_left =
            AxisId::new(1).map_err(|e| map_err("allocating MoveRightLeft axis", e))?;
        let move_up_down = AxisId::new(2).map_err(|e| map_err("allocating MoveUpDown axis", e))?;
        let look_yaw = AxisId::new(3).map_err(|e| map_err("allocating LookYaw axis", e))?;
        let look_pitch = AxisId::new(4).map_err(|e| map_err("allocating LookPitch axis", e))?;

        let mut map = InputMap::new();
        map.bind_action(exit, ButtonTrigger::Key(KeyCode::Escape))
            .map_err(|e| map_err("binding Escape to Exit action", e))?;
        map.bind_action(
            toggle_cursor_grab,
            ButtonTrigger::MouseButton(MouseButton::Right),
        )
        .map_err(|e| map_err("binding MouseButton::Right to ToggleCursorGrab action", e))?;
        map.bind_action(toggle_cursor_grab, ButtonTrigger::Key(KeyCode::Tab))
            .map_err(|e| map_err("binding Tab to ToggleCursorGrab action", e))?;

        map.bind_axis_digital_pair(
            move_forward_back,
            ButtonTrigger::Key(KeyCode::S),
            ButtonTrigger::Key(KeyCode::W),
        )
        .map_err(|e| map_err("binding S/W to MoveForwardBack axis", e))?;
        map.set_axis_config(move_forward_back, AxisConfig::unit());

        map.bind_axis_digital_pair(
            move_right_left,
            ButtonTrigger::Key(KeyCode::A),
            ButtonTrigger::Key(KeyCode::D),
        )
        .map_err(|e| map_err("binding A/D to MoveRightLeft axis", e))?;
        map.set_axis_config(move_right_left, AxisConfig::unit());

        map.bind_axis_digital_pair(
            move_up_down,
            ButtonTrigger::Key(KeyCode::ControlLeft),
            ButtonTrigger::Key(KeyCode::Space),
        )
        .map_err(|e| map_err("binding ControlLeft/Space to MoveUpDown axis", e))?;
        map.set_axis_config(move_up_down, AxisConfig::unit());

        map.bind_axis(look_yaw, AxisSource::PointerRelative(MotionAxis::X))
            .map_err(|e| map_err("binding PointerRelative(X) to LookYaw axis", e))?;
        map.set_axis_config(look_yaw, AxisConfig::new());

        map.bind_axis(look_pitch, AxisSource::PointerRelative(MotionAxis::Y))
            .map_err(|e| map_err("binding PointerRelative(Y) to LookPitch axis", e))?;
        map.set_axis_config(look_pitch, AxisConfig::new());

        let bindings = Self {
            exit,
            toggle_cursor_grab,
            move_forward_back,
            move_right_left,
            move_up_down,
            look_yaw,
            look_pitch,
        };
        Ok((InputState::with_map(map), bindings))
    }

    /// Extracts [`CameraTickInput`] from a tick-sampled [`InputCommand`].
    #[must_use]
    pub fn sample_camera_input(
        &self,
        command: &InputCommand,
        look_enabled: bool,
    ) -> CameraTickInput {
        CameraTickInput {
            move_forward_back: f64::from(command.axis(self.move_forward_back)),
            move_right_left: f64::from(command.axis(self.move_right_left)),
            move_up_down: f64::from(command.axis(self.move_up_down)),
            look_yaw_delta: f64::from(command.axis(self.look_yaw)),
            look_pitch_delta: f64::from(command.axis(self.look_pitch)),
            look_enabled,
        }
    }
}

#[inline]
fn duration_seconds_f64(duration: Duration) -> f64 {
    (duration.as_secs() as f64) + (f64::from(duration.subsec_nanos()) / 1_000_000_000.0)
}

#[inline]
fn sanitize_f64(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

#[inline]
fn canonical_zero_f64(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

#[inline]
fn canonical_world_pos(pos: WorldPos) -> WorldPos {
    WorldPos::new(
        canonical_zero_f64(pos.x),
        canonical_zero_f64(pos.y),
        canonical_zero_f64(pos.z),
    )
}
