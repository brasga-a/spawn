//! Local `f32` vector primitives (`Vec2`, `Vec3`, `Vec4`).
//!
//! All types are `#[repr(C)]`, `Copy`, lifetime-free, and allocation-free.
//! Degenerate normalization (`0.0`, `NaN`, `±Inf`) never panics: use
//! `try_normalize` or `normalize_or_zero`.

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Relative tolerance around `1.0` used by `is_normalized` checks on `f32` vectors.
const NORMALIZED_LEN_SQ_TOLERANCE: f32 = 2.0e-4;

#[inline]
fn lerp_scalar_f32(a: f32, b: f32, t: f32) -> f32 {
    if t == 0.0 {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    let delta = b - a;
    if delta.is_finite() {
        a + delta * t
    } else {
        a * (1.0 - t) + b * t
    }
}

/// 2D single-precision vector (`f32`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
}

impl Vec2 {
    /// `(0.0, 0.0)`.
    pub const ZERO: Self = Self::new(0.0, 0.0);
    /// `(1.0, 1.0)`.
    pub const ONE: Self = Self::new(1.0, 1.0);
    /// `(-1.0, -1.0)`.
    pub const NEG_ONE: Self = Self::new(-1.0, -1.0);
    /// Unit `+X` `(1.0, 0.0)`.
    pub const X: Self = Self::new(1.0, 0.0);
    /// Unit `+Y` `(0.0, 1.0)`.
    pub const Y: Self = Self::new(0.0, 1.0);
    /// Unit `-X` `(-1.0, 0.0)`.
    pub const NEG_X: Self = Self::new(-1.0, 0.0);
    /// Unit `-Y` `(0.0, -1.0)`.
    pub const NEG_Y: Self = Self::new(0.0, -1.0);
    /// `(∞, ∞)`.
    pub const INFINITY: Self = Self::new(f32::INFINITY, f32::INFINITY);
    /// `(-∞, -∞)`.
    pub const NEG_INFINITY: Self = Self::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    /// `(NaN, NaN)`.
    pub const NAN: Self = Self::new(f32::NAN, f32::NAN);

    /// Creates a new 2D vector.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Creates a vector with both components set to `value`.
    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self { x: value, y: value }
    }

    /// Creates a `Vec2` from `[x, y]`.
    #[inline]
    pub const fn from_array(arr: [f32; 2]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
        }
    }

    /// Converts this vector to `[x, y]`.
    #[inline]
    pub const fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }

    /// Encodes components in little-endian IEEE-754 byte order (`8` bytes).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 8] {
        let x = self.x.to_le_bytes();
        let y = self.y.to_le_bytes();
        [x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3]]
    }

    /// Decodes components from little-endian IEEE-754 bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 8]) -> Self {
        let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        Self { x, y }
    }

    /// Non-panicking component lookup (`0 => x`, `1 => y`).
    #[inline]
    pub const fn get(self, index: usize) -> Option<f32> {
        match index {
            0 => Some(self.x),
            1 => Some(self.y),
            _ => None,
        }
    }

    /// Extends to a [`Vec3`] by appending `z`.
    #[inline]
    pub const fn extend(self, z: f32) -> Vec3 {
        Vec3::new(self.x, self.y, z)
    }

    /// Dot product `self.x * rhs.x + self.y * rhs.y`.
    #[inline]
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y
    }

    /// 2D perpendicular `(-y, x)` (90° counter-clockwise in `+X` right, `+Y` up).
    #[inline]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// 2D wedge / perpendicular dot product `self.x * rhs.y - self.y * rhs.x`.
    #[inline]
    pub fn perp_dot(self, rhs: Self) -> f32 {
        self.x * rhs.y - self.y * rhs.x
    }

    /// Squared Euclidean length.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length (overflow- and underflow-safe across finite inputs).
    #[inline]
    pub fn length(self) -> f32 {
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            return len_sq.sqrt();
        }
        if !self.is_finite() {
            return if self.is_nan() {
                f32::NAN
            } else {
                f32::INFINITY
            };
        }
        let max_abs = self.x.abs().max(self.y.abs());
        if max_abs == 0.0 {
            0.0
        } else {
            let scaled = Self::new(self.x / max_abs, self.y / max_abs);
            max_abs * scaled.length_squared().sqrt()
        }
    }

    /// Squared Euclidean distance between `self` and `rhs`.
    #[inline]
    pub fn distance_squared(self, rhs: Self) -> f32 {
        (self - rhs).length_squared()
    }

    /// Euclidean distance between `self` and `rhs`.
    #[inline]
    pub fn distance(self, rhs: Self) -> f32 {
        (self - rhs).length()
    }

    /// Returns a unit-length vector in the direction of `self`, or `None` if `self`
    /// is zero-length or non-finite (`NaN` / `±Inf`).
    ///
    /// Subnormal and near-`f32::MAX` finite non-zero inputs are rescaled safely
    /// before normalization instead of underflowing or overflowing `length_squared`.
    #[inline]
    pub fn try_normalize(self) -> Option<Self> {
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            let inv_len = 1.0 / len_sq.sqrt();
            return Some(self * inv_len);
        }
        if !self.is_finite() {
            return None;
        }
        let max_abs = self.x.abs().max(self.y.abs());
        if max_abs == 0.0 {
            return None;
        }
        let scaled = Self::new(self.x / max_abs, self.y / max_abs);
        let inv_len = 1.0 / scaled.length_squared().sqrt();
        Some(scaled * inv_len)
    }

    /// Returns a normalized vector, or [`Self::ZERO`] if `self` is zero-length or non-finite.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        self.try_normalize().unwrap_or(Self::ZERO)
    }

    /// Returns a normalized vector, or `fallback` if `self` is zero-length or non-finite.
    #[inline]
    pub fn normalize_or(self, fallback: Self) -> Self {
        self.try_normalize().unwrap_or(fallback)
    }

    /// Returns `true` if `self` is finite and within `1e-4` of unit length.
    #[inline]
    pub fn is_normalized(self) -> bool {
        self.is_finite() && (self.length_squared() - 1.0).abs() <= NORMALIZED_LEN_SQ_TOLERANCE
    }

    /// Returns `true` if all components are finite (neither `NaN` nor infinite).
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Returns `true` if any component is `NaN`.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.x.is_nan() || self.y.is_nan()
    }

    /// Componentwise absolute value.
    #[inline]
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs())
    }

    /// Componentwise signum.
    #[inline]
    pub fn signum(self) -> Self {
        Self::new(self.x.signum(), self.y.signum())
    }

    /// Componentwise minimum (`NaN` propagated if either input is `NaN` via IEEE min? Note:
    /// uses `f32::min`, matching IEEE-754 `minNum` / Rust `std`).
    #[inline]
    pub fn min(self, rhs: Self) -> Self {
        Self::new(self.x.min(rhs.x), self.y.min(rhs.y))
    }

    /// Componentwise maximum.
    #[inline]
    pub fn max(self, rhs: Self) -> Self {
        Self::new(self.x.max(rhs.x), self.y.max(rhs.y))
    }

    /// Componentwise clamp to `[min, max]` without panicking if `min > max`
    /// (returns `min.max(self.min(max))` when `min <= max`, or `NAN` on inverted bounds).
    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self::new(
            clamp_non_panicking_f32(self.x, min.x, max.x),
            clamp_non_panicking_f32(self.y, min.y, max.y),
        )
    }

    /// Smallest component.
    #[inline]
    pub fn min_element(self) -> f32 {
        self.x.min(self.y)
    }

    /// Largest component.
    #[inline]
    pub fn max_element(self) -> f32 {
        self.x.max(self.y)
    }

    /// Componentwise floor.
    #[inline]
    pub fn floor(self) -> Self {
        Self::new(self.x.floor(), self.y.floor())
    }

    /// Componentwise ceil.
    #[inline]
    pub fn ceil(self) -> Self {
        Self::new(self.x.ceil(), self.y.ceil())
    }

    /// Componentwise round.
    #[inline]
    pub fn round(self) -> Self {
        Self::new(self.x.round(), self.y.round())
    }

    /// Componentwise fractional part `self - self.floor()`.
    #[inline]
    pub fn fract_gl(self) -> Self {
        self - self.floor()
    }

    /// Linearly interpolates between `self` (`t = 0.0`) and `rhs` (`t = 1.0`).
    #[inline]
    pub fn lerp(self, rhs: Self, t: f32) -> Self {
        Self::new(
            lerp_scalar_f32(self.x, rhs.x, t),
            lerp_scalar_f32(self.y, rhs.y, t),
        )
    }

    /// Projects `self` onto `onto`, returning [`Self::ZERO`] if `onto` is zero or non-finite.
    #[inline]
    pub fn project_onto_or_zero(self, onto: Self) -> Self {
        let Some(dir) = onto.try_normalize() else {
            return Self::ZERO;
        };
        dir * self.dot(dir)
    }

    /// Rejects `self` from `from` (`self - project_onto(from)`), returning `self` if `from` is zero.
    #[inline]
    pub fn reject_from_or_zero(self, from: Self) -> Self {
        self - self.project_onto_or_zero(from)
    }

    /// Reflects incident vector `self` across unit `normal` (`self - 2 * dot(self, normal) * normal`).
    #[inline]
    pub fn reflect(self, normal: Self) -> Self {
        self - normal * (2.0 * self.dot(normal))
    }

    /// Returns `true` if `|self - rhs|` is componentwise `<= max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(self, rhs: Self, max_abs_diff: f32) -> bool {
        (self.x - rhs.x).abs() <= max_abs_diff && (self.y - rhs.y).abs() <= max_abs_diff
    }
}

/// 3D single-precision vector (`f32`) for local positions, offsets, directions, scales, and velocities.
///
/// Engine-wide spatial convention (ADR-0010): right-handed, `+X` right, `+Y` up, `-Z` forward.
/// Absolute world coordinates must use [`crate::WorldPos`] (`f64`), never `Vec3` (`SPN-INV-051`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// X component (`+X` right).
    pub x: f32,
    /// Y component (`+Y` up).
    pub y: f32,
    /// Z component (`-Z` forward, `+Z` back).
    pub z: f32,
}

impl Vec3 {
    /// `(0.0, 0.0, 0.0)`.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// `(1.0, 1.0, 1.0)`.
    pub const ONE: Self = Self::new(1.0, 1.0, 1.0);
    /// `(-1.0, -1.0, -1.0)`.
    pub const NEG_ONE: Self = Self::new(-1.0, -1.0, -1.0);
    /// Right axis `+X` `(1.0, 0.0, 0.0)`.
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    /// Up axis `+Y` `(0.0, 1.0, 0.0)`.
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    /// Back axis `+Z` `(0.0, 0.0, 1.0)`.
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);
    /// Left axis `-X` `(-1.0, 0.0, 0.0)`.
    pub const NEG_X: Self = Self::new(-1.0, 0.0, 0.0);
    /// Down axis `-Y` `(0.0, -1.0, 0.0)`.
    pub const NEG_Y: Self = Self::new(0.0, -1.0, 0.0);
    /// Forward axis `-Z` `(0.0, 0.0, -1.0)`.
    pub const NEG_Z: Self = Self::new(0.0, 0.0, -1.0);
    /// Canonical right direction (`+X`).
    pub const RIGHT: Self = Self::X;
    /// Canonical up direction (`+Y`).
    pub const UP: Self = Self::Y;
    /// Canonical forward direction (`-Z`).
    pub const FORWARD: Self = Self::NEG_Z;
    /// `(∞, ∞, ∞)`.
    pub const INFINITY: Self = Self::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    /// `(-∞, -∞, -∞)`.
    pub const NEG_INFINITY: Self =
        Self::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    /// `(NaN, NaN, NaN)`.
    pub const NAN: Self = Self::new(f32::NAN, f32::NAN, f32::NAN);

    /// Creates a new 3D vector.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Creates a vector with all components set to `value`.
    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self {
            x: value,
            y: value,
            z: value,
        }
    }

    /// Creates a `Vec3` from `[x, y, z]`.
    #[inline]
    pub const fn from_array(arr: [f32; 3]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
        }
    }

    /// Converts this vector to `[x, y, z]`.
    #[inline]
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// Encodes components in little-endian IEEE-754 byte order (`12` bytes).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 12] {
        let x = self.x.to_le_bytes();
        let y = self.y.to_le_bytes();
        let z = self.z.to_le_bytes();
        [
            x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3], z[0], z[1], z[2], z[3],
        ]
    }

    /// Decodes components from little-endian IEEE-754 bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 12]) -> Self {
        let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        Self { x, y, z }
    }

    /// Non-panicking component lookup (`0 => x`, `1 => y`, `2 => z`).
    #[inline]
    pub const fn get(self, index: usize) -> Option<f32> {
        match index {
            0 => Some(self.x),
            1 => Some(self.y),
            2 => Some(self.z),
            _ => None,
        }
    }

    /// Truncates to `(x, y)`.
    #[inline]
    pub const fn truncate(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Returns `(x, y)`.
    #[inline]
    pub const fn xy(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Extends to a [`Vec4`] by appending `w`.
    #[inline]
    pub const fn extend(self, w: f32) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }

    /// Dot product `x1*x2 + y1*y2 + z1*z2`.
    #[inline]
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Right-handed cross product (`Vec3::X.cross(Vec3::Y) == Vec3::Z`).
    #[inline]
    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    /// Squared Euclidean length.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length (overflow- and underflow-safe across finite inputs).
    #[inline]
    pub fn length(self) -> f32 {
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            return len_sq.sqrt();
        }
        if !self.is_finite() {
            return if self.is_nan() {
                f32::NAN
            } else {
                f32::INFINITY
            };
        }
        let max_abs = self.x.abs().max(self.y.abs()).max(self.z.abs());
        if max_abs == 0.0 {
            0.0
        } else {
            let scaled = Self::new(self.x / max_abs, self.y / max_abs, self.z / max_abs);
            max_abs * scaled.length_squared().sqrt()
        }
    }

    /// Squared Euclidean distance between `self` and `rhs`.
    #[inline]
    pub fn distance_squared(self, rhs: Self) -> f32 {
        (self - rhs).length_squared()
    }

    /// Euclidean distance between `self` and `rhs`.
    #[inline]
    pub fn distance(self, rhs: Self) -> f32 {
        (self - rhs).length()
    }

    /// Returns a unit-length vector in the direction of `self`, or `None` if `self`
    /// is zero-length or non-finite (`NaN` / `±Inf`).
    #[inline]
    pub fn try_normalize(self) -> Option<Self> {
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            let inv_len = 1.0 / len_sq.sqrt();
            return Some(self * inv_len);
        }
        if !self.is_finite() {
            return None;
        }
        let max_abs = self.x.abs().max(self.y.abs()).max(self.z.abs());
        if max_abs == 0.0 {
            return None;
        }
        let scaled = Self::new(self.x / max_abs, self.y / max_abs, self.z / max_abs);
        let inv_len = 1.0 / scaled.length_squared().sqrt();
        Some(scaled * inv_len)
    }

    /// Returns a normalized vector, or [`Self::ZERO`] if `self` is zero-length or non-finite.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        self.try_normalize().unwrap_or(Self::ZERO)
    }

    /// Returns a normalized vector, or `fallback` if `self` is zero-length or non-finite.
    #[inline]
    pub fn normalize_or(self, fallback: Self) -> Self {
        self.try_normalize().unwrap_or(fallback)
    }

    /// Returns `true` if `self` is finite and within `1e-4` of unit length.
    #[inline]
    pub fn is_normalized(self) -> bool {
        self.is_finite() && (self.length_squared() - 1.0).abs() <= NORMALIZED_LEN_SQ_TOLERANCE
    }

    /// Returns `true` if all components are finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Returns `true` if any component is `NaN`.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.x.is_nan() || self.y.is_nan() || self.z.is_nan()
    }

    /// Componentwise absolute value.
    #[inline]
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    /// Componentwise signum.
    #[inline]
    pub fn signum(self) -> Self {
        Self::new(self.x.signum(), self.y.signum(), self.z.signum())
    }

    /// Componentwise minimum.
    #[inline]
    pub fn min(self, rhs: Self) -> Self {
        Self::new(self.x.min(rhs.x), self.y.min(rhs.y), self.z.min(rhs.z))
    }

    /// Componentwise maximum.
    #[inline]
    pub fn max(self, rhs: Self) -> Self {
        Self::new(self.x.max(rhs.x), self.y.max(rhs.y), self.z.max(rhs.z))
    }

    /// Componentwise non-panicking clamp to `[min, max]`.
    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self::new(
            clamp_non_panicking_f32(self.x, min.x, max.x),
            clamp_non_panicking_f32(self.y, min.y, max.y),
            clamp_non_panicking_f32(self.z, min.z, max.z),
        )
    }

    /// Smallest component.
    #[inline]
    pub fn min_element(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }

    /// Largest component.
    #[inline]
    pub fn max_element(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }

    /// Componentwise floor.
    #[inline]
    pub fn floor(self) -> Self {
        Self::new(self.x.floor(), self.y.floor(), self.z.floor())
    }

    /// Componentwise ceil.
    #[inline]
    pub fn ceil(self) -> Self {
        Self::new(self.x.ceil(), self.y.ceil(), self.z.ceil())
    }

    /// Componentwise round.
    #[inline]
    pub fn round(self) -> Self {
        Self::new(self.x.round(), self.y.round(), self.z.round())
    }

    /// Componentwise fractional part `self - self.floor()`.
    #[inline]
    pub fn fract_gl(self) -> Self {
        self - self.floor()
    }

    /// Linearly interpolates between `self` (`t = 0.0`) and `rhs` (`t = 1.0`).
    #[inline]
    pub fn lerp(self, rhs: Self, t: f32) -> Self {
        Self::new(
            lerp_scalar_f32(self.x, rhs.x, t),
            lerp_scalar_f32(self.y, rhs.y, t),
            lerp_scalar_f32(self.z, rhs.z, t),
        )
    }

    /// Projects `self` onto `onto`, returning [`Self::ZERO`] if `onto` is zero or non-finite.
    #[inline]
    pub fn project_onto_or_zero(self, onto: Self) -> Self {
        let Some(dir) = onto.try_normalize() else {
            return Self::ZERO;
        };
        dir * self.dot(dir)
    }

    /// Rejects `self` from `from`, returning `self` if `from` is zero or non-finite.
    #[inline]
    pub fn reject_from_or_zero(self, from: Self) -> Self {
        self - self.project_onto_or_zero(from)
    }

    /// Reflects incident vector `self` across unit `normal`.
    #[inline]
    pub fn reflect(self, normal: Self) -> Self {
        self - normal * (2.0 * self.dot(normal))
    }

    /// Returns an arbitrary unit vector orthogonal to unit vector `self`.
    #[inline]
    pub fn any_orthonormal_vector(self) -> Option<Self> {
        let candidate = if self.x.abs() < 0.9 {
            Self::X.cross(self)
        } else {
            Self::Y.cross(self)
        };
        candidate.try_normalize()
    }

    /// Returns `true` if `|self - rhs|` is componentwise `<= max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(self, rhs: Self, max_abs_diff: f32) -> bool {
        (self.x - rhs.x).abs() <= max_abs_diff
            && (self.y - rhs.y).abs() <= max_abs_diff
            && (self.z - rhs.z).abs() <= max_abs_diff
    }
}

/// 4D single-precision homogeneous / color / column vector (`f32`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec4 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
    /// W component.
    pub w: f32,
}

impl Vec4 {
    /// `(0.0, 0.0, 0.0, 0.0)`.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);
    /// `(1.0, 1.0, 1.0, 1.0)`.
    pub const ONE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    /// `(-1.0, -1.0, -1.0, -1.0)`.
    pub const NEG_ONE: Self = Self::new(-1.0, -1.0, -1.0, -1.0);
    /// Unit `+X` `(1.0, 0.0, 0.0, 0.0)`.
    pub const X: Self = Self::new(1.0, 0.0, 0.0, 0.0);
    /// Unit `+Y` `(0.0, 1.0, 0.0, 0.0)`.
    pub const Y: Self = Self::new(0.0, 1.0, 0.0, 0.0);
    /// Unit `+Z` `(0.0, 0.0, 1.0, 0.0)`.
    pub const Z: Self = Self::new(0.0, 0.0, 1.0, 0.0);
    /// Unit `+W` `(0.0, 0.0, 0.0, 1.0)`.
    pub const W: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    /// Unit `-X` `(-1.0, 0.0, 0.0, 0.0)`.
    pub const NEG_X: Self = Self::new(-1.0, 0.0, 0.0, 0.0);
    /// Unit `-Y` `(0.0, -1.0, 0.0, 0.0)`.
    pub const NEG_Y: Self = Self::new(0.0, -1.0, 0.0, 0.0);
    /// Unit `-Z` `(0.0, 0.0, -1.0, 0.0)`.
    pub const NEG_Z: Self = Self::new(0.0, 0.0, -1.0, 0.0);
    /// Unit `-W` `(0.0, 0.0, 0.0, -1.0)`.
    pub const NEG_W: Self = Self::new(0.0, 0.0, 0.0, -1.0);
    /// `(∞, ∞, ∞, ∞)`.
    pub const INFINITY: Self =
        Self::new(f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY);
    /// `(-∞, -∞, -∞, -∞)`.
    pub const NEG_INFINITY: Self = Self::new(
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    /// `(NaN, NaN, NaN, NaN)`.
    pub const NAN: Self = Self::new(f32::NAN, f32::NAN, f32::NAN, f32::NAN);

    /// Creates a new 4D vector.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Creates a vector with all components set to `value`.
    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self {
            x: value,
            y: value,
            z: value,
            w: value,
        }
    }

    /// Creates a `Vec4` from `[x, y, z, w]`.
    #[inline]
    pub const fn from_array(arr: [f32; 4]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
            w: arr[3],
        }
    }

    /// Converts this vector to `[x, y, z, w]`.
    #[inline]
    pub const fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// Encodes components in little-endian IEEE-754 byte order (`16` bytes).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 16] {
        let x = self.x.to_le_bytes();
        let y = self.y.to_le_bytes();
        let z = self.z.to_le_bytes();
        let w = self.w.to_le_bytes();
        [
            x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3], z[0], z[1], z[2], z[3], w[0], w[1],
            w[2], w[3],
        ]
    }

    /// Decodes components from little-endian IEEE-754 bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 16]) -> Self {
        let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let w = f32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        Self { x, y, z, w }
    }

    /// Non-panicking component lookup (`0 => x`, `1 => y`, `2 => z`, `3 => w`).
    #[inline]
    pub const fn get(self, index: usize) -> Option<f32> {
        match index {
            0 => Some(self.x),
            1 => Some(self.y),
            2 => Some(self.z),
            3 => Some(self.w),
            _ => None,
        }
    }

    /// Truncates to `(x, y, z)`.
    #[inline]
    pub const fn truncate(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Returns `(x, y, z)`.
    #[inline]
    pub const fn xyz(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Returns `(x, y)`.
    #[inline]
    pub const fn xy(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Dot product `x1*x2 + y1*y2 + z1*z2 + w1*w2`.
    #[inline]
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z + self.w * rhs.w
    }

    /// Squared Euclidean length.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length (overflow- and underflow-safe across finite inputs).
    #[inline]
    pub fn length(self) -> f32 {
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            return len_sq.sqrt();
        }
        if !self.is_finite() {
            return if self.is_nan() {
                f32::NAN
            } else {
                f32::INFINITY
            };
        }
        let max_abs = self
            .x
            .abs()
            .max(self.y.abs())
            .max(self.z.abs())
            .max(self.w.abs());
        if max_abs == 0.0 {
            0.0
        } else {
            let scaled = Self::new(
                self.x / max_abs,
                self.y / max_abs,
                self.z / max_abs,
                self.w / max_abs,
            );
            max_abs * scaled.length_squared().sqrt()
        }
    }

    /// Squared Euclidean distance between `self` and `rhs`.
    #[inline]
    pub fn distance_squared(self, rhs: Self) -> f32 {
        (self - rhs).length_squared()
    }

    /// Euclidean distance between `self` and `rhs`.
    #[inline]
    pub fn distance(self, rhs: Self) -> f32 {
        (self - rhs).length()
    }

    /// Returns a unit-length vector in the direction of `self`, or `None` if `self`
    /// is zero-length or non-finite (`NaN` / `±Inf`).
    #[inline]
    pub fn try_normalize(self) -> Option<Self> {
        let len_sq = self.length_squared();
        if len_sq >= f32::MIN_POSITIVE && len_sq.is_finite() {
            let inv_len = 1.0 / len_sq.sqrt();
            return Some(self * inv_len);
        }
        if !self.is_finite() {
            return None;
        }
        let max_abs = self
            .x
            .abs()
            .max(self.y.abs())
            .max(self.z.abs())
            .max(self.w.abs());
        if max_abs == 0.0 {
            return None;
        }
        let scaled = Self::new(
            self.x / max_abs,
            self.y / max_abs,
            self.z / max_abs,
            self.w / max_abs,
        );
        let inv_len = 1.0 / scaled.length_squared().sqrt();
        Some(scaled * inv_len)
    }

    /// Returns a normalized vector, or [`Self::ZERO`] if `self` is zero-length or non-finite.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        self.try_normalize().unwrap_or(Self::ZERO)
    }

    /// Returns a normalized vector, or `fallback` if `self` is zero-length or non-finite.
    #[inline]
    pub fn normalize_or(self, fallback: Self) -> Self {
        self.try_normalize().unwrap_or(fallback)
    }

    /// Returns `true` if `self` is finite and within `1e-4` of unit length.
    #[inline]
    pub fn is_normalized(self) -> bool {
        self.is_finite() && (self.length_squared() - 1.0).abs() <= NORMALIZED_LEN_SQ_TOLERANCE
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

    /// Componentwise absolute value.
    #[inline]
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs(), self.w.abs())
    }

    /// Componentwise signum.
    #[inline]
    pub fn signum(self) -> Self {
        Self::new(
            self.x.signum(),
            self.y.signum(),
            self.z.signum(),
            self.w.signum(),
        )
    }

    /// Componentwise minimum.
    #[inline]
    pub fn min(self, rhs: Self) -> Self {
        Self::new(
            self.x.min(rhs.x),
            self.y.min(rhs.y),
            self.z.min(rhs.z),
            self.w.min(rhs.w),
        )
    }

    /// Componentwise maximum.
    #[inline]
    pub fn max(self, rhs: Self) -> Self {
        Self::new(
            self.x.max(rhs.x),
            self.y.max(rhs.y),
            self.z.max(rhs.z),
            self.w.max(rhs.w),
        )
    }

    /// Componentwise non-panicking clamp to `[min, max]`.
    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self::new(
            clamp_non_panicking_f32(self.x, min.x, max.x),
            clamp_non_panicking_f32(self.y, min.y, max.y),
            clamp_non_panicking_f32(self.z, min.z, max.z),
            clamp_non_panicking_f32(self.w, min.w, max.w),
        )
    }

    /// Smallest component.
    #[inline]
    pub fn min_element(self) -> f32 {
        self.x.min(self.y).min(self.z).min(self.w)
    }

    /// Largest component.
    #[inline]
    pub fn max_element(self) -> f32 {
        self.x.max(self.y).max(self.z).max(self.w)
    }

    /// Componentwise floor.
    #[inline]
    pub fn floor(self) -> Self {
        Self::new(
            self.x.floor(),
            self.y.floor(),
            self.z.floor(),
            self.w.floor(),
        )
    }

    /// Componentwise ceil.
    #[inline]
    pub fn ceil(self) -> Self {
        Self::new(self.x.ceil(), self.y.ceil(), self.z.ceil(), self.w.ceil())
    }

    /// Componentwise round.
    #[inline]
    pub fn round(self) -> Self {
        Self::new(
            self.x.round(),
            self.y.round(),
            self.z.round(),
            self.w.round(),
        )
    }

    /// Linearly interpolates between `self` (`t = 0.0`) and `rhs` (`t = 1.0`).
    #[inline]
    pub fn lerp(self, rhs: Self, t: f32) -> Self {
        Self::new(
            lerp_scalar_f32(self.x, rhs.x, t),
            lerp_scalar_f32(self.y, rhs.y, t),
            lerp_scalar_f32(self.z, rhs.z, t),
            lerp_scalar_f32(self.w, rhs.w, t),
        )
    }

    /// Perspective divide `(x / w, y / w, z / w)`, returning `None` if `w == 0` or result is non-finite.
    #[inline]
    pub fn perspective_divide(self) -> Option<Vec3> {
        if !self.is_finite() || self.w == 0.0 {
            return None;
        }
        let inv_w = 1.0 / self.w;
        if !inv_w.is_finite() {
            return None;
        }
        let p = Vec3::new(self.x * inv_w, self.y * inv_w, self.z * inv_w);
        p.is_finite().then_some(p)
    }

    /// Returns `true` if `|self - rhs|` is componentwise `<= max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(self, rhs: Self, max_abs_diff: f32) -> bool {
        (self.x - rhs.x).abs() <= max_abs_diff
            && (self.y - rhs.y).abs() <= max_abs_diff
            && (self.z - rhs.z).abs() <= max_abs_diff
            && (self.w - rhs.w).abs() <= max_abs_diff
    }
}

#[inline]
pub(crate) fn clamp_non_panicking_f32(v: f32, min: f32, max: f32) -> f32 {
    if v.is_nan() || min.is_nan() || max.is_nan() || min > max {
        f32::NAN
    } else {
        v.clamp(min, max)
    }
}

macro_rules! impl_vec_ops {
    ($Vec:ident { $($field:ident),+ }) => {
        impl Add for $Vec {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self::Output {
                Self { $($field: self.$field + rhs.$field),+ }
            }
        }

        impl AddAssign for $Vec {
            #[inline]
            fn add_assign(&mut self, rhs: Self) {
                $(self.$field += rhs.$field;)+
            }
        }

        impl Sub for $Vec {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self::Output {
                Self { $($field: self.$field - rhs.$field),+ }
            }
        }

        impl SubAssign for $Vec {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) {
                $(self.$field -= rhs.$field;)+
            }
        }

        impl Mul for $Vec {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: Self) -> Self::Output {
                Self { $($field: self.$field * rhs.$field),+ }
            }
        }

        impl MulAssign for $Vec {
            #[inline]
            fn mul_assign(&mut self, rhs: Self) {
                $(self.$field *= rhs.$field;)+
            }
        }

        impl Div for $Vec {
            type Output = Self;
            #[inline]
            fn div(self, rhs: Self) -> Self::Output {
                Self { $($field: self.$field / rhs.$field),+ }
            }
        }

        impl DivAssign for $Vec {
            #[inline]
            fn div_assign(&mut self, rhs: Self) {
                $(self.$field /= rhs.$field;)+
            }
        }

        impl Mul<f32> for $Vec {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: f32) -> Self::Output {
                Self { $($field: self.$field * rhs),+ }
            }
        }

        impl Mul<$Vec> for f32 {
            type Output = $Vec;
            #[inline]
            fn mul(self, rhs: $Vec) -> Self::Output {
                $Vec { $($field: self * rhs.$field),+ }
            }
        }

        impl MulAssign<f32> for $Vec {
            #[inline]
            fn mul_assign(&mut self, rhs: f32) {
                $(self.$field *= rhs;)+
            }
        }

        impl Div<f32> for $Vec {
            type Output = Self;
            #[inline]
            fn div(self, rhs: f32) -> Self::Output {
                Self { $($field: self.$field / rhs),+ }
            }
        }

        impl DivAssign<f32> for $Vec {
            #[inline]
            fn div_assign(&mut self, rhs: f32) {
                $(self.$field /= rhs;)+
            }
        }

        impl Neg for $Vec {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self::Output {
                Self { $($field: -self.$field),+ }
            }
        }
    };
}

impl_vec_ops!(Vec2 { x, y });
impl_vec_ops!(Vec3 { x, y, z });
impl_vec_ops!(Vec4 { x, y, z, w });
