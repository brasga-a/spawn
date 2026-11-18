//! Deterministic `f32` and `f64` math subset (`SPN-INV-044`, ADR-0010 decision 4,
//! ADR-0005).
//!
//! Functions in this module execute identical pure-Rust software algorithms (`libm` with
//! `default-features = false`, disabling architecture-specific overrides) on Linux and
//! Windows for identical bit-pattern inputs.
//!
//! ## Fused multiply-add (FMA) rule
//!
//! - Composite operations in `det` use unfused IEEE-754 `+`, `-`, `*`, and `/` so results
//!   never depend on whether the target CPU enables hardware FMA contraction.
//! - Callers that explicitly want a deterministic IEEE-754 correctly rounded fused
//!   multiply-add use [`mul_add_f32`] or [`mul_add_f64`] (backed by `libm::fmaf` / `libm::fma`,
//!   never OS C runtime `fmaf`/`fma`).
//!
//! Only simulation, prediction, reconciliation, and replay paths need this subset;
//! presentation-only code (projection matrices, frame interpolation, rendering) may use
//! standard `f32` methods.

use crate::{DVec3, Quat, Vec3};

/// Default pole safety margin (`1e-3` rad, ~`0.057°`) away from `±π/2` for pitch clamping (`f32`).
pub const DEFAULT_PITCH_LIMIT_EPSILON_F32: f32 = 1.0e-3;

/// Default pole safety margin (`1e-3` rad, ~`0.057°`) away from `±π/2` for pitch clamping (`f64`).
pub const DEFAULT_PITCH_LIMIT_EPSILON_F64: f64 = 1.0e-3;

/// Deterministic IEEE-754 correctly rounded square root (`f32`).
#[inline]
pub fn sqrt_f32(x: f32) -> f32 {
    libm::sqrtf(x)
}

/// Deterministic IEEE-754 correctly rounded square root (`f64`).
#[inline]
pub fn sqrt_f64(x: f64) -> f64 {
    libm::sqrt(x)
}

/// Deterministic sine of `radians` (`f32`).
#[inline]
pub fn sin_f32(radians: f32) -> f32 {
    libm::sinf(radians)
}

/// Deterministic sine of `radians` (`f64`).
#[inline]
pub fn sin_f64(radians: f64) -> f64 {
    libm::sin(radians)
}

/// Deterministic cosine of `radians` (`f32`).
#[inline]
pub fn cos_f32(radians: f32) -> f32 {
    libm::cosf(radians)
}

/// Deterministic cosine of `radians` (`f64`).
#[inline]
pub fn cos_f64(radians: f64) -> f64 {
    libm::cos(radians)
}

/// Deterministic simultaneous `(sin(radians), cos(radians))` (`f32`).
#[inline]
pub fn sin_cos_f32(radians: f32) -> (f32, f32) {
    libm::sincosf(radians)
}

/// Deterministic simultaneous `(sin(radians), cos(radians))` (`f64`).
#[inline]
pub fn sin_cos_f64(radians: f64) -> (f64, f64) {
    libm::sincos(radians)
}

/// Deterministic tangent of `radians` (`f32`).
#[inline]
pub fn tan_f32(radians: f32) -> f32 {
    libm::tanf(radians)
}

/// Deterministic tangent of `radians` (`f64`).
#[inline]
pub fn tan_f64(radians: f64) -> f64 {
    libm::tan(radians)
}

/// Deterministic four-quadrant arctangent `atan2(y, x)` in `[-π, π]` (`f32`).
#[inline]
pub fn atan2_f32(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// Deterministic four-quadrant arctangent `atan2(y, x)` in `[-π, π]` (`f64`).
#[inline]
pub fn atan2_f64(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// Deterministic unclamped arcsine (`f32`); returns `NaN` if `|x| > 1.0` or `x` is `NaN`.
#[inline]
pub fn asin_f32(x: f32) -> f32 {
    libm::asinf(x)
}

/// Deterministic unclamped arcsine (`f64`); returns `NaN` if `|x| > 1.0` or `x` is `NaN`.
#[inline]
pub fn asin_f64(x: f64) -> f64 {
    libm::asin(x)
}

/// Deterministic unclamped arccosine (`f32`); returns `NaN` if `|x| > 1.0` or `x` is `NaN`.
#[inline]
pub fn acos_f32(x: f32) -> f32 {
    libm::acosf(x)
}

/// Deterministic unclamped arccosine (`f64`); returns `NaN` if `|x| > 1.0` or `x` is `NaN`.
#[inline]
pub fn acos_f64(x: f64) -> f64 {
    libm::acos(x)
}

/// Deterministic pole-clamped arcsine (`f32`): clamps non-`NaN` `x` to `[-1.0, 1.0]` before
/// `asinf` so slight unit-vector dot-product drift never yields `NaN`. `NaN` inputs propagate `NaN`.
#[inline]
pub fn asin_clamped_f32(x: f32) -> f32 {
    libm::asinf(x.clamp(-1.0, 1.0))
}

/// Deterministic pole-clamped arcsine (`f64`): clamps non-`NaN` `x` to `[-1.0, 1.0]` before
/// `asin`. `NaN` inputs propagate `NaN`.
#[inline]
pub fn asin_clamped_f64(x: f64) -> f64 {
    libm::asin(x.clamp(-1.0, 1.0))
}

/// Deterministic pole-clamped arccosine (`f32`): clamps non-`NaN` `x` to `[-1.0, 1.0]` before
/// `acosf`. `NaN` inputs propagate `NaN`.
#[inline]
pub fn acos_clamped_f32(x: f32) -> f32 {
    libm::acosf(x.clamp(-1.0, 1.0))
}

/// Deterministic pole-clamped arccosine (`f64`): clamps non-`NaN` `x` to `[-1.0, 1.0]` before
/// `acos`. `NaN` inputs propagate `NaN`.
#[inline]
pub fn acos_clamped_f64(x: f64) -> f64 {
    libm::acos(x.clamp(-1.0, 1.0))
}

/// Deterministic natural exponential `e^x` (`f32`).
#[inline]
pub fn exp_f32(x: f32) -> f32 {
    libm::expf(x)
}

/// Deterministic natural exponential `e^x` (`f64`).
#[inline]
pub fn exp_f64(x: f64) -> f64 {
    libm::exp(x)
}

/// Deterministic natural logarithm `ln(x)` (`f32`).
#[inline]
pub fn ln_f32(x: f32) -> f32 {
    libm::logf(x)
}

/// Deterministic natural logarithm `ln(x)` (`f64`).
#[inline]
pub fn ln_f64(x: f64) -> f64 {
    libm::log(x)
}

/// Explicit deterministic IEEE-754 fused multiply-add `(a * b) + c` (`f32`), backed by pure-Rust `libm::fmaf`.
#[inline]
pub fn mul_add_f32(a: f32, b: f32, c: f32) -> f32 {
    libm::fmaf(a, b, c)
}

/// Explicit deterministic IEEE-754 fused multiply-add `(a * b) + c` (`f64`), backed by pure-Rust `libm::fma`.
#[inline]
pub fn mul_add_f64(a: f64, b: f64, c: f64) -> f64 {
    libm::fma(a, b, c)
}

/// Deterministically wraps a finite radian angle into `(-π, π]` (`f32`).
///
/// Non-finite inputs return `f32::NAN`.
#[inline]
pub fn wrap_angle_pi_f32(radians: f32) -> f32 {
    if !radians.is_finite() {
        return f32::NAN;
    }
    let tau = std::f32::consts::TAU;
    let pi = std::f32::consts::PI;
    let mut wrapped = libm::fmodf(radians, tau);
    if wrapped <= -pi {
        wrapped += tau;
    } else if wrapped > pi {
        wrapped -= tau;
    }
    wrapped
}

/// Deterministically wraps a finite radian angle into `(-π, π]` (`f64`).
///
/// Non-finite inputs return `f64::NAN`.
#[inline]
pub fn wrap_angle_pi_f64(radians: f64) -> f64 {
    if !radians.is_finite() {
        return f64::NAN;
    }
    let tau = std::f64::consts::TAU;
    let pi = std::f64::consts::PI;
    let mut wrapped = libm::fmod(radians, tau);
    if wrapped <= -pi {
        wrapped += tau;
    } else if wrapped > pi {
        wrapped -= tau;
    }
    wrapped
}

/// Deterministically clamps pitch (`f32`, radians) to `[-(π/2 - epsilon), +(π/2 - epsilon)]`
/// so free-fly / look orientation stays strictly away from the vertical `±π/2` poles.
///
/// Negative or excessive `epsilon_radians` values are bounded into `[0, π/2]`.
/// `NaN` pitch propagates `NaN`.
#[inline]
pub fn clamp_pitch_f32(pitch_radians: f32, epsilon_radians: f32) -> f32 {
    if pitch_radians.is_nan() || epsilon_radians.is_nan() {
        return f32::NAN;
    }
    let half_pi = std::f32::consts::FRAC_PI_2;
    let eps = epsilon_radians.clamp(0.0, half_pi);
    let limit = half_pi - eps;
    pitch_radians.clamp(-limit, limit)
}

/// Deterministically clamps pitch (`f64`, radians) to `[-(π/2 - epsilon), +(π/2 - epsilon)]`.
///
/// Negative or excessive `epsilon_radians` values are bounded into `[0, π/2]`.
/// `NaN` pitch propagates `NaN`.
#[inline]
pub fn clamp_pitch_f64(pitch_radians: f64, epsilon_radians: f64) -> f64 {
    if pitch_radians.is_nan() || epsilon_radians.is_nan() {
        return f64::NAN;
    }
    let half_pi = std::f64::consts::FRAC_PI_2;
    let eps = epsilon_radians.clamp(0.0, half_pi);
    let limit = half_pi - eps;
    pitch_radians.clamp(-limit, limit)
}

/// Deterministic unit [`Quat`] from intrinsic yaw around world `+Y` followed by intrinsic
/// pitch around local `+X` (`f32` radians; zero yaw and zero pitch look down `-Z`).
///
/// Returns [`Quat::IDENTITY`] if either angle is non-finite.
#[inline]
pub fn quat_from_yaw_pitch(yaw_radians: f32, pitch_radians: f32) -> Quat {
    if !yaw_radians.is_finite() || !pitch_radians.is_finite() {
        return Quat::IDENTITY;
    }
    let (sy, cy) = sin_cos_f32(yaw_radians * 0.5);
    let (sp, cp) = sin_cos_f32(pitch_radians * 0.5);
    let raw = Quat::from_xyzw(cy * sp, sy * cp, -sy * sp, cy * cp);
    normalize_quat_det(raw)
}

/// Deterministic unit [`Quat`] from `f64` radian yaw (world `+Y`) and pitch (local `+X`),
/// evaluated in `f64` via `libm` before narrowing to normalized `f32` [`Quat`].
///
/// Returns [`Quat::IDENTITY`] if either angle is non-finite.
#[inline]
pub fn quat_from_yaw_pitch_f64(yaw_radians: f64, pitch_radians: f64) -> Quat {
    if !yaw_radians.is_finite() || !pitch_radians.is_finite() {
        return Quat::IDENTITY;
    }
    let (sy, cy) = sin_cos_f64(yaw_radians * 0.5);
    let (sp, cp) = sin_cos_f64(pitch_radians * 0.5);
    let x = cy * sp;
    let y = sy * cp;
    let z = -sy * sp;
    let w = cy * cp;
    let len = sqrt_f64(x * x + y * y + z * z + w * w);
    if len <= 0.0 || !len.is_finite() {
        return Quat::IDENTITY;
    }
    let inv = 1.0 / len;
    Quat::from_xyzw(
        (x * inv) as f32,
        (y * inv) as f32,
        (z * inv) as f32,
        (w * inv) as f32,
    )
}

/// Deterministic unit [`Quat`] from an arbitrary axis and radian angle (`f32`),
/// or `None` if `axis` is zero/non-finite or `angle_radians` is non-finite.
#[inline]
pub fn quat_from_axis_angle(axis: Vec3, angle_radians: f32) -> Option<Quat> {
    if !axis.is_finite() || !angle_radians.is_finite() {
        return None;
    }
    let unit = normalize_vec3_det(axis)?;
    let (s, c) = sin_cos_f32(angle_radians * 0.5);
    Some(normalize_quat_det(Quat::from_xyzw(
        unit.x * s,
        unit.y * s,
        unit.z * s,
        c,
    )))
}

/// Deterministic shortest-arc spherical linear interpolation between unit quaternions.
#[inline]
pub fn slerp_shortest(a: Quat, b: Quat, t: f32) -> Quat {
    if !a.is_finite() || !b.is_finite() || !t.is_finite() {
        return Quat::IDENTITY;
    }
    let qa = normalize_quat_det(a);
    let mut qb = normalize_quat_det(b);
    let mut dot = qa.dot(qb);
    if dot < 0.0 {
        qb = -qb;
        dot = -dot;
    }
    if dot > 0.9995 {
        let blended = Quat::from_xyzw(
            qa.x + (qb.x - qa.x) * t,
            qa.y + (qb.y - qa.y) * t,
            qa.z + (qb.z - qa.z) * t,
            qa.w + (qb.w - qa.w) * t,
        );
        return normalize_quat_det(blended);
    }
    let theta_0 = acos_clamped_f32(dot);
    let sin_theta_0 = sin_f32(theta_0);
    if sin_theta_0 <= f32::MIN_POSITIVE {
        return qa;
    }
    let theta = theta_0 * t;
    let s0 = sin_f32(theta_0 - theta) / sin_theta_0;
    let s1 = sin_f32(theta) / sin_theta_0;
    normalize_quat_det(Quat::from_xyzw(
        qa.x * s0 + qb.x * s1,
        qa.y * s0 + qb.y * s1,
        qa.z * s0 + qb.z * s1,
        qa.w * s0 + qb.w * s1,
    ))
}

/// Orthonormal `f32` right/up/forward basis (`+X` right, `+Y` up, `-Z` forward at identity).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Basis3 {
    /// Local right direction (`+X` at zero yaw/pitch; horizontal with `y == 0.0`).
    pub right: Vec3,
    /// Local up direction (`+Y` at zero yaw/pitch).
    pub up: Vec3,
    /// Local forward direction (`-Z` at zero yaw/pitch).
    pub forward: Vec3,
}

impl Basis3 {
    /// Combines local `(right_left, up_down, forward_back)` scalars into a world-space `Vec3`.
    #[inline]
    pub fn local_vector(self, right_left: f32, up_down: f32, forward_back: f32) -> Vec3 {
        self.right * right_left + self.up * up_down + self.forward * forward_back
    }
}

/// Orthonormal `f64` right/up/forward basis for deterministic `f64` world-space movement
/// integration (`+X` right, `+Y` up, `-Z` forward at identity).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DBasis3 {
    /// Local right direction (`+X` at zero yaw/pitch; horizontal with `y == 0.0`).
    pub right: DVec3,
    /// Local up direction (`+Y` at zero yaw/pitch).
    pub up: DVec3,
    /// Local forward direction (`-Z` at zero yaw/pitch).
    pub forward: DVec3,
}

impl DBasis3 {
    /// Combines local `(right_left, up_down, forward_back)` scalars into a world-space [`DVec3`].
    #[inline]
    pub fn local_vector(self, right_left: f64, up_down: f64, forward_back: f64) -> DVec3 {
        self.right * right_left + self.up * up_down + self.forward * forward_back
    }
}

/// Computes the deterministic orthonormal [`Basis3`] (`f32`) for intrinsic yaw (`+Y`) then
/// pitch (local `+X`). Non-finite inputs return the canonical identity basis.
#[inline]
pub fn yaw_pitch_basis_f32(yaw_radians: f32, pitch_radians: f32) -> Basis3 {
    if !yaw_radians.is_finite() || !pitch_radians.is_finite() {
        return Basis3 {
            right: Vec3::RIGHT,
            up: Vec3::UP,
            forward: Vec3::FORWARD,
        };
    }
    let (sy, cy) = sin_cos_f32(yaw_radians);
    let (sp, cp) = sin_cos_f32(pitch_radians);
    Basis3 {
        right: Vec3::new(cy, 0.0, -sy),
        up: Vec3::new(sy * sp, cp, cy * sp),
        forward: Vec3::new(-sy * cp, sp, -cy * cp),
    }
}

/// Computes the deterministic orthonormal [`DBasis3`] (`f64`) for intrinsic yaw (`+Y`) then
/// pitch (local `+X`). Non-finite inputs return the canonical identity basis.
#[inline]
pub fn yaw_pitch_basis_f64(yaw_radians: f64, pitch_radians: f64) -> DBasis3 {
    if !yaw_radians.is_finite() || !pitch_radians.is_finite() {
        return DBasis3 {
            right: DVec3::RIGHT,
            up: DVec3::UP,
            forward: DVec3::FORWARD,
        };
    }
    let (sy, cy) = sin_cos_f64(yaw_radians);
    let (sp, cp) = sin_cos_f64(pitch_radians);
    DBasis3 {
        right: DVec3::new(cy, 0.0, -sy),
        up: DVec3::new(sy * sp, cp, cy * sp),
        forward: DVec3::new(-sy * cp, sp, -cy * cp),
    }
}

#[inline]
fn normalize_vec3_det(v: Vec3) -> Option<Vec3> {
    let len_sq = v.length_squared();
    if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
        let inv = 1.0 / sqrt_f32(len_sq);
        return Some(v * inv);
    }
    if !v.is_finite() {
        return None;
    }
    let max_abs = v.x.abs().max(v.y.abs()).max(v.z.abs());
    if max_abs == 0.0 {
        return None;
    }
    let scaled = Vec3::new(v.x / max_abs, v.y / max_abs, v.z / max_abs);
    let inv = 1.0 / sqrt_f32(scaled.length_squared());
    Some(scaled * inv)
}

#[inline]
fn normalize_quat_det(q: Quat) -> Quat {
    let len_sq = q.length_squared();
    if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
        let inv = 1.0 / sqrt_f32(len_sq);
        return Quat::from_xyzw(q.x * inv, q.y * inv, q.z * inv, q.w * inv);
    }
    if !q.is_finite() {
        return Quat::IDENTITY;
    }
    let max_abs = q.x.abs().max(q.y.abs()).max(q.z.abs()).max(q.w.abs());
    if max_abs == 0.0 {
        return Quat::IDENTITY;
    }
    let sx = q.x / max_abs;
    let sy = q.y / max_abs;
    let sz = q.z / max_abs;
    let sw = q.w / max_abs;
    let inv = 1.0 / sqrt_f32(sx * sx + sy * sy + sz * sz + sw * sw);
    Quat::from_xyzw(sx * inv, sy * inv, sz * inv, sw * inv)
}
