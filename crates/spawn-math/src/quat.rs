//! Unit quaternion rotation primitive (`Quat`).
//!
//! Quaternions (`x, y, z` imaginary vector part, `w` real scalar part) are the canonical
//! 3D rotation representation in Spawn (ADR-0010).
//!
//! ## Fixed Euler convention (ADR-0010 decision 2)
//!
//! [`Quat::from_yaw_pitch`] and [`Quat::from_yaw_pitch_roll`] use **intrinsic `YXZ`** order:
//! 1. **Yaw** around world `+Y` (up).
//! 2. **Pitch** around the rotated local `+X` (right).
//! 3. **Roll** (`0` for [`Quat::from_yaw_pitch`]) around the rotated local `+Z` (back).
//!
//! At `yaw = 0`, `pitch = 0`, `roll = 0` ([`Quat::IDENTITY`]):
//! - local right (`+X`) is `(1, 0, 0)`,
//! - local up (`+Y`) is `(0, 1, 0)`,
//! - local forward (`-Z`) is `(0, 0, -1)`.

use std::ops::{Mul, MulAssign, Neg};

use crate::{Mat3, Vec3, Vec4};

const NORMALIZED_QUAT_LEN_SQ_TOLERANCE: f32 = 2.0e-4;

/// Single-precision quaternion `(x, y, z, w)`.
///
/// Representing 3D rotations requires unit-length quaternions (`is_normalized() == true`).
/// [`Default`] returns [`Quat::IDENTITY`] `(0, 0, 0, 1)`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    /// Imaginary `i` component.
    pub x: f32,
    /// Imaginary `j` component.
    pub y: f32,
    /// Imaginary `k` component.
    pub z: f32,
    /// Real scalar component (`1.0` for [`Self::IDENTITY`]).
    pub w: f32,
}

impl Default for Quat {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    /// Identity rotation `(0.0, 0.0, 0.0, 1.0)`.
    pub const IDENTITY: Self = Self::from_xyzw(0.0, 0.0, 0.0, 1.0);
    /// `(NaN, NaN, NaN, NaN)`.
    pub const NAN: Self = Self::from_xyzw(f32::NAN, f32::NAN, f32::NAN, f32::NAN);

    /// Constructs a quaternion directly from raw `(x, y, z, w)` components without normalizing.
    #[inline]
    pub const fn from_xyzw(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Constructs a quaternion from `[x, y, z, w]` without normalizing.
    #[inline]
    pub const fn from_array(arr: [f32; 4]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
            w: arr[3],
        }
    }

    /// Converts this quaternion to `[x, y, z, w]`.
    #[inline]
    pub const fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// Converts this quaternion to a [`Vec4`] `(x, y, z, w)`.
    #[inline]
    pub const fn to_vec4(self) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, self.w)
    }

    /// Encodes `(x, y, z, w)` in little-endian IEEE-754 byte order (`16` bytes).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 16] {
        self.to_vec4().to_le_bytes()
    }

    /// Decodes `(x, y, z, w)` from little-endian IEEE-754 bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 16]) -> Self {
        let v = Vec4::from_le_bytes(bytes);
        Self::from_xyzw(v.x, v.y, v.z, v.w)
    }

    /// Right-handed rotation around `+X` by `angle_radians` (`None` if non-finite).
    #[inline]
    pub fn try_from_rotation_x(angle_radians: f32) -> Option<Self> {
        if !angle_radians.is_finite() {
            return None;
        }
        let (s, c) = (angle_radians * 0.5).sin_cos();
        Some(Self::from_xyzw(s, 0.0, 0.0, c))
    }

    /// Right-handed rotation around `+X` by `angle_radians` (or [`Self::IDENTITY`] if non-finite).
    #[inline]
    pub fn from_rotation_x(angle_radians: f32) -> Self {
        Self::try_from_rotation_x(angle_radians).unwrap_or(Self::IDENTITY)
    }

    /// Right-handed rotation around `+Y` by `angle_radians` (`None` if non-finite).
    #[inline]
    pub fn try_from_rotation_y(angle_radians: f32) -> Option<Self> {
        if !angle_radians.is_finite() {
            return None;
        }
        let (s, c) = (angle_radians * 0.5).sin_cos();
        Some(Self::from_xyzw(0.0, s, 0.0, c))
    }

    /// Right-handed rotation around `+Y` by `angle_radians` (or [`Self::IDENTITY`] if non-finite).
    #[inline]
    pub fn from_rotation_y(angle_radians: f32) -> Self {
        Self::try_from_rotation_y(angle_radians).unwrap_or(Self::IDENTITY)
    }

    /// Right-handed rotation around `+Z` by `angle_radians` (`None` if non-finite).
    #[inline]
    pub fn try_from_rotation_z(angle_radians: f32) -> Option<Self> {
        if !angle_radians.is_finite() {
            return None;
        }
        let (s, c) = (angle_radians * 0.5).sin_cos();
        Some(Self::from_xyzw(0.0, 0.0, s, c))
    }

    /// Right-handed rotation around `+Z` by `angle_radians` (or [`Self::IDENTITY`] if non-finite).
    #[inline]
    pub fn from_rotation_z(angle_radians: f32) -> Self {
        Self::try_from_rotation_z(angle_radians).unwrap_or(Self::IDENTITY)
    }

    /// Right-handed rotation around `axis` (normalized internally) by `angle_radians`.
    ///
    /// Returns `None` if `axis` is zero-length or non-finite, or `angle_radians` is non-finite.
    #[inline]
    pub fn try_from_axis_angle(axis: Vec3, angle_radians: f32) -> Option<Self> {
        if !angle_radians.is_finite() {
            return None;
        }
        let unit = axis.try_normalize()?;
        let (s, c) = (angle_radians * 0.5).sin_cos();
        Self::from_xyzw(unit.x * s, unit.y * s, unit.z * s, c).try_normalize()
    }

    /// Right-handed rotation around `axis` by `angle_radians`, or [`Self::IDENTITY`] on degenerate input.
    #[inline]
    pub fn from_axis_angle_or_identity(axis: Vec3, angle_radians: f32) -> Self {
        Self::try_from_axis_angle(axis, angle_radians).unwrap_or(Self::IDENTITY)
    }

    /// Intrinsic yaw around world `+Y` followed by intrinsic pitch around local `+X` (radians).
    ///
    /// Zero yaw and zero pitch look down `-Z` with `+X` right and `+Y` up.
    /// Non-finite inputs return [`Self::IDENTITY`].
    ///
    /// Note: simulation/prediction paths should call [`crate::det::quat_from_yaw_pitch`] so
    /// cross-platform `libm` transcendentals are used (`SPN-INV-044`).
    #[inline]
    pub fn from_yaw_pitch(yaw_radians: f32, pitch_radians: f32) -> Self {
        if !yaw_radians.is_finite() || !pitch_radians.is_finite() {
            return Self::IDENTITY;
        }
        let (sy, cy) = (yaw_radians * 0.5).sin_cos();
        let (sp, cp) = (pitch_radians * 0.5).sin_cos();
        Self::from_xyzw(cy * sp, sy * cp, -sy * sp, cy * cp).normalize_or_identity()
    }

    /// Intrinsic `YXZ` Euler rotation: yaw (`+Y`), pitch (local `+X`), roll (local `+Z`), in radians.
    ///
    /// Non-finite inputs return [`Self::IDENTITY`].
    #[inline]
    pub fn from_yaw_pitch_roll(yaw_radians: f32, pitch_radians: f32, roll_radians: f32) -> Self {
        if !yaw_radians.is_finite() || !pitch_radians.is_finite() || !roll_radians.is_finite() {
            return Self::IDENTITY;
        }
        let (sy, cy) = (yaw_radians * 0.5).sin_cos();
        let (sp, cp) = (pitch_radians * 0.5).sin_cos();
        let (sr, cr) = (roll_radians * 0.5).sin_cos();
        Self::from_xyzw(
            cy * sp * cr + sy * cp * sr,
            sy * cp * cr - cy * sp * sr,
            cy * cp * sr - sy * sp * cr,
            cy * cp * cr + sy * sp * sr,
        )
        .normalize_or_identity()
    }

    /// Minimal-arc rotation mapping direction `from` onto direction `to`.
    ///
    /// Handles parallel (`0°`) and antiparallel (`180°`) vectors cleanly without division by zero;
    /// returns `None` if either vector is zero-length or non-finite.
    #[inline]
    pub fn try_from_rotation_arc(from: Vec3, to: Vec3) -> Option<Self> {
        let u = from.try_normalize()?;
        let v = to.try_normalize()?;
        let dot = u.dot(v);
        if dot >= 0.999_999 {
            return Some(Self::IDENTITY);
        }
        if dot <= -0.999_999 {
            let ortho = u.any_orthonormal_vector()?;
            return Some(Self::from_xyzw(ortho.x, ortho.y, ortho.z, 0.0));
        }
        let s = ((1.0 + dot) * 2.0).sqrt();
        let inv_s = 1.0 / s;
        let cross = u.cross(v);
        Self::from_xyzw(cross.x * inv_s, cross.y * inv_s, cross.z * inv_s, s * 0.5).try_normalize()
    }

    /// Minimal-arc rotation mapping `from` onto `to`, or [`Self::IDENTITY`] on degenerate input.
    #[inline]
    pub fn from_rotation_arc_or_identity(from: Vec3, to: Vec3) -> Self {
        Self::try_from_rotation_arc(from, to).unwrap_or(Self::IDENTITY)
    }

    /// Extracts a unit quaternion from a pure rotation [`Mat3`] using Shepperd's 4-branch method
    /// (numerically stable across all angles including `180°` rotations where `trace == -1`).
    ///
    /// Returns `None` if `mat` is non-finite or has non-positive determinant.
    #[inline]
    pub fn try_from_mat3(mat: Mat3) -> Option<Self> {
        if !mat.is_finite() || mat.determinant() <= 0.0 {
            return None;
        }
        let c0r0 = mat.x_axis.x;
        let c0r1 = mat.x_axis.y;
        let c0r2 = mat.x_axis.z;
        let c1r0 = mat.y_axis.x;
        let c1r1 = mat.y_axis.y;
        let c1r2 = mat.y_axis.z;
        let c2r0 = mat.z_axis.x;
        let c2r1 = mat.z_axis.y;
        let c2r2 = mat.z_axis.z;

        let trace = c0r0 + c1r1 + c2r2;
        let raw = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            let inv_s = 1.0 / s;
            Self::from_xyzw(
                (c1r2 - c2r1) * inv_s,
                (c2r0 - c0r2) * inv_s,
                (c0r1 - c1r0) * inv_s,
                0.25 * s,
            )
        } else if c0r0 > c1r1 && c0r0 > c2r2 {
            let s = (1.0 + c0r0 - c1r1 - c2r2).max(0.0).sqrt() * 2.0;
            if s <= f32::MIN_POSITIVE {
                return None;
            }
            let inv_s = 1.0 / s;
            Self::from_xyzw(
                0.25 * s,
                (c0r1 + c1r0) * inv_s,
                (c2r0 + c0r2) * inv_s,
                (c1r2 - c2r1) * inv_s,
            )
        } else if c1r1 > c2r2 {
            let s = (1.0 + c1r1 - c0r0 - c2r2).max(0.0).sqrt() * 2.0;
            if s <= f32::MIN_POSITIVE {
                return None;
            }
            let inv_s = 1.0 / s;
            Self::from_xyzw(
                (c0r1 + c1r0) * inv_s,
                0.25 * s,
                (c1r2 + c2r1) * inv_s,
                (c2r0 - c0r2) * inv_s,
            )
        } else {
            let s = (1.0 + c2r2 - c0r0 - c1r1).max(0.0).sqrt() * 2.0;
            if s <= f32::MIN_POSITIVE {
                return None;
            }
            let inv_s = 1.0 / s;
            Self::from_xyzw(
                (c2r0 + c0r2) * inv_s,
                (c1r2 + c2r1) * inv_s,
                0.25 * s,
                (c0r1 - c1r0) * inv_s,
            )
        };
        raw.try_normalize()
    }

    /// 4D quaternion dot product `x1*x2 + y1*y2 + z1*z2 + w1*w2`.
    #[inline]
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z + self.w * rhs.w
    }

    /// Squared norm `x^2 + y^2 + z^2 + w^2`.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean norm (overflow- and subnormal-safe).
    #[inline]
    pub fn length(self) -> f32 {
        self.to_vec4().length()
    }

    /// Normalizes `self` to unit length, returning `None` if zero-length or non-finite.
    #[inline]
    pub fn try_normalize(self) -> Option<Self> {
        let v = self.to_vec4().try_normalize()?;
        Some(Self::from_xyzw(v.x, v.y, v.z, v.w))
    }

    /// Normalizes `self` to unit length, or returns [`Self::IDENTITY`] if zero-length or non-finite.
    #[inline]
    pub fn normalize_or_identity(self) -> Self {
        self.try_normalize().unwrap_or(Self::IDENTITY)
    }

    /// Returns `true` if `self` is finite and within `1e-4` of unit length.
    #[inline]
    pub fn is_normalized(self) -> bool {
        self.is_finite() && (self.length_squared() - 1.0).abs() <= NORMALIZED_QUAT_LEN_SQ_TOLERANCE
    }

    /// Returns `true` if all components are finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }

    /// Returns `true` if any component is `NaN`.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.x.is_nan() || self.y.is_nan() || self.z.is_nan() || self.w.is_nan()
    }

    /// Quaternion conjugate `(-x, -y, -z, w)`. For unit quaternions, `conjugate()` equals the inverse rotation.
    #[inline]
    pub const fn conjugate(self) -> Self {
        Self::from_xyzw(-self.x, -self.y, -self.z, self.w)
    }

    /// Multiplicative inverse `conjugate() / length_squared()`, or `None` if zero or non-finite.
    #[inline]
    pub fn try_inverse(self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            let inv_len_sq = 1.0 / len_sq;
            let q = Self::from_xyzw(
                -self.x * inv_len_sq,
                -self.y * inv_len_sq,
                -self.z * inv_len_sq,
                self.w * inv_len_sq,
            );
            return q.is_finite().then_some(q);
        }
        None
    }

    /// Decomposes into `(unit_axis, angle_radians)` with `angle_radians ∈ [0, π]`.
    ///
    /// Near zero angle or on non-finite input, returns `(Vec3::X, 0.0)`.
    #[inline]
    pub fn to_axis_angle(self) -> (Vec3, f32) {
        let Some(mut q) = self.try_normalize() else {
            return (Vec3::X, 0.0);
        };
        if q.w < 0.0 {
            q = -q;
        }
        let imag = Vec3::new(q.x, q.y, q.z);
        let sin_half = imag.length();
        if sin_half <= 1.0e-6 {
            return (Vec3::X, 0.0);
        }
        let angle = 2.0 * sin_half.atan2(q.w);
        let axis = (imag / sin_half).normalize_or(Vec3::X);
        (axis, angle)
    }

    /// Shortest 3D angular distance in `[0, π]` radians between two rotations
    /// (invariant under quaternion sign flip `q` vs `-q`).
    ///
    /// Uses Kahan's `4 * atan2(||qa - qb||, ||qa + qb||)` chord/sum formula on the
    /// aligned hemisphere (`qa.dot(qb) >= 0`) to avoid the `sqrt(2 * ulp)` (~`6.9e-4` rad)
    /// catastrophic cancellation of `2 * acos(dot)` near `dot = 1.0`.
    #[inline]
    pub fn angle_between(self, other: Self) -> f32 {
        let Some(qa) = self.try_normalize() else {
            return f32::NAN;
        };
        let Some(mut qb) = other.try_normalize() else {
            return f32::NAN;
        };
        if qa.dot(qb) < 0.0 {
            qb = -qb;
        }
        let diff = (qa.to_vec4() - qb.to_vec4()).length();
        let sum = (qa.to_vec4() + qb.to_vec4()).length();
        4.0 * diff.atan2(sum)
    }

    /// Shortest-arc normalized linear interpolation between `self` (`t = 0.0`) and `other` (`t = 1.0`).
    #[inline]
    pub fn nlerp(self, other: Self, t: f32) -> Self {
        if !self.is_finite() || !other.is_finite() || !t.is_finite() {
            return Self::IDENTITY;
        }
        let qa = self.normalize_or_identity();
        let mut qb = other.normalize_or_identity();
        if qa.dot(qb) < 0.0 {
            qb = -qb;
        }
        Self::from_xyzw(
            qa.x + (qb.x - qa.x) * t,
            qa.y + (qb.y - qa.y) * t,
            qa.z + (qb.z - qa.z) * t,
            qa.w + (qb.w - qa.w) * t,
        )
        .normalize_or_identity()
    }

    /// Shortest-arc spherical linear interpolation (`slerp_shortest`) between `self` (`t = 0.0`)
    /// and `other` (`t = 1.0`).
    ///
    /// Automatically flips hemisphere when `dot < 0.0` (so `q` and `-q` never take the `360°` long
    /// path or divide by zero at `dot == -1.0`), and smoothly blends to normalized linear
    /// interpolation when `dot > 0.9995`. Non-finite inputs return [`Self::IDENTITY`].
    #[inline]
    pub fn slerp(self, other: Self, t: f32) -> Self {
        if !self.is_finite() || !other.is_finite() || !t.is_finite() {
            return Self::IDENTITY;
        }
        let qa = self.normalize_or_identity();
        let mut qb = other.normalize_or_identity();
        let mut dot = qa.dot(qb);
        if dot < 0.0 {
            qb = -qb;
            dot = -dot;
        }
        if dot > 0.9995 {
            return Self::from_xyzw(
                qa.x + (qb.x - qa.x) * t,
                qa.y + (qb.y - qa.y) * t,
                qa.z + (qb.z - qa.z) * t,
                qa.w + (qb.w - qa.w) * t,
            )
            .normalize_or_identity();
        }
        let theta_0 = dot.min(1.0).acos();
        let sin_theta_0 = theta_0.sin();
        if sin_theta_0 <= f32::MIN_POSITIVE {
            return qa;
        }
        let theta = theta_0 * t;
        let s0 = (theta_0 - theta).sin() / sin_theta_0;
        let s1 = theta.sin() / sin_theta_0;
        Self::from_xyzw(
            qa.x * s0 + qb.x * s1,
            qa.y * s0 + qb.y * s1,
            qa.z * s0 + qb.z * s1,
            qa.w * s0 + qb.w * s1,
        )
        .normalize_or_identity()
    }

    /// Rotates a 3D vector `rhs` by unit quaternion `self`.
    #[inline]
    pub fn mul_vec3(self, rhs: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(rhs) * 2.0;
        rhs + t * self.w + u.cross(t)
    }

    /// Rotated local right axis (`self * Vec3::X`).
    #[inline]
    pub fn right(self) -> Vec3 {
        self.mul_vec3(Vec3::X)
    }

    /// Rotated local up axis (`self * Vec3::Y`).
    #[inline]
    pub fn up(self) -> Vec3 {
        self.mul_vec3(Vec3::Y)
    }

    /// Rotated local forward axis (`self * Vec3::NEG_Z`).
    #[inline]
    pub fn forward(self) -> Vec3 {
        self.mul_vec3(Vec3::NEG_Z)
    }

    /// Componentwise difference check (`|self - rhs| <= max_abs_diff`).
    /// For 3D rotation equivalence up to sign (`q` vs `-q`), prefer [`Self::angle_between`].
    #[inline]
    pub fn abs_diff_eq(self, rhs: Self, max_abs_diff: f32) -> bool {
        self.to_vec4().abs_diff_eq(rhs.to_vec4(), max_abs_diff)
    }
}

impl Mul for Quat {
    type Output = Self;
    /// Hamilton product `self * rhs` (applies rotation `rhs` first, then `self`,
    /// matching `(Mat3::from_quat(self) * Mat3::from_quat(rhs)) * v`).
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        }
    }
}

impl MulAssign for Quat {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl Mul<Vec3> for Quat {
    type Output = Vec3;
    #[inline]
    fn mul(self, rhs: Vec3) -> Self::Output {
        self.mul_vec3(rhs)
    }
}

impl Neg for Quat {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self::Output {
        Self::from_xyzw(-self.x, -self.y, -self.z, -self.w)
    }
}
