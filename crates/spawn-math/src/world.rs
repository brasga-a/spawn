//! Double-precision (`f64`) world-space coordinate and displacement primitives
//! (`SPN-INV-051`, `SPN-INV-052`, ADR-0010).
//!
//! - [`WorldPos`] represents an absolute position in world space (meters, `f64`).
//!   It is intentionally a distinct type from [`crate::Vec3`] (`f32`) and [`DVec3`]
//!   so the compiler rejects mixing world positions with local `f32` vectors or
//!   implicitly adding two absolute points.
//! - [`DVec3`] represents a double-precision displacement or direction vector (`f64`),
//!   such as `world_a - world_b`.
//! - Rendering and local computations convert `f64` world positions to camera-relative
//!   `f32` offsets explicitly via [`camera_relative_f32`] or [`DVec3::to_vec3_lossy`].

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::Vec3;

const NORMALIZED_LEN_SQ_TOLERANCE_F64: f64 = 1.0e-10;

#[inline]
fn lerp_scalar_f64(a: f64, b: f64, t: f64) -> f64 {
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

#[inline]
fn clamp_non_panicking_f64(v: f64, min: f64, max: f64) -> f64 {
    if v.is_nan() || min.is_nan() || max.is_nan() || min > max {
        f64::NAN
    } else {
        v.clamp(min, max)
    }
}

/// 3D double-precision displacement or direction vector (`f64`).
///
/// Returned by subtracting two [`WorldPos`] points (`a - b`), and used for
/// high-precision displacement integration in simulation before explicit
/// conversion to local [`Vec3`] (`f32`) via [`DVec3::to_vec3_lossy`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DVec3 {
    /// X component (`+X` right, meters).
    pub x: f64,
    /// Y component (`+Y` up, meters).
    pub y: f64,
    /// Z component (`-Z` forward, `+Z` back, meters).
    pub z: f64,
}

impl DVec3 {
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
    pub const INFINITY: Self = Self::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    /// `(-∞, -∞, -∞)`.
    pub const NEG_INFINITY: Self =
        Self::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    /// `(NaN, NaN, NaN)`.
    pub const NAN: Self = Self::new(f64::NAN, f64::NAN, f64::NAN);

    /// Creates a new double-precision 3D vector.
    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Creates a vector with all components set to `value`.
    #[inline]
    pub const fn splat(value: f64) -> Self {
        Self {
            x: value,
            y: value,
            z: value,
        }
    }

    /// Creates a `DVec3` from `[x, y, z]`.
    #[inline]
    pub const fn from_array(arr: [f64; 3]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
        }
    }

    /// Converts this vector to `[x, y, z]`.
    #[inline]
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// Encodes components in little-endian IEEE-754 byte order (`24` bytes).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 24] {
        let x = self.x.to_le_bytes();
        let y = self.y.to_le_bytes();
        let z = self.z.to_le_bytes();
        [
            x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7], y[0], y[1], y[2], y[3], y[4], y[5],
            y[6], y[7], z[0], z[1], z[2], z[3], z[4], z[5], z[6], z[7],
        ]
    }

    /// Decodes components from little-endian IEEE-754 bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 24]) -> Self {
        let x = f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        let y = f64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        let z = f64::from_le_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
        ]);
        Self { x, y, z }
    }

    /// Non-panicking component lookup (`0 => x`, `1 => y`, `2 => z`).
    #[inline]
    pub const fn get(self, index: usize) -> Option<f64> {
        match index {
            0 => Some(self.x),
            1 => Some(self.y),
            2 => Some(self.z),
            _ => None,
        }
    }

    /// Explicitly widens a local `f32` [`Vec3`] displacement into `f64` (lossless).
    #[inline]
    pub const fn from_vec3_exact(v: Vec3) -> Self {
        Self {
            x: v.x as f64,
            y: v.y as f64,
            z: v.z as f64,
        }
    }

    /// Explicitly narrows a relative `f64` displacement to `f32` [`Vec3`] (`SPN-INV-051`).
    ///
    /// Callers must subtract the camera or reference [`WorldPos`] in `f64` **before**
    /// narrowing to `f32` so large-world precision is preserved (`SPN-INV-052`).
    #[inline]
    pub const fn to_vec3_lossy(self) -> Vec3 {
        Vec3::new(self.x as f32, self.y as f32, self.z as f32)
    }

    /// Dot product `x1*x2 + y1*y2 + z1*z2`.
    #[inline]
    pub fn dot(self, rhs: Self) -> f64 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Right-handed cross product (`DVec3::X.cross(DVec3::Y) == DVec3::Z`).
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
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    /// Euclidean length using deterministic IEEE-754 `sqrt` (overflow- and subnormal-safe).
    #[inline]
    pub fn length(self) -> f64 {
        let len_sq = self.length_squared();
        if len_sq >= f64::MIN_POSITIVE && len_sq.is_finite() {
            return crate::det::sqrt_f64(len_sq);
        }
        if !self.is_finite() {
            return if self.is_nan() {
                f64::NAN
            } else {
                f64::INFINITY
            };
        }
        let max_abs = self.x.abs().max(self.y.abs()).max(self.z.abs());
        if max_abs == 0.0 {
            0.0
        } else {
            let scaled = Self::new(self.x / max_abs, self.y / max_abs, self.z / max_abs);
            max_abs * crate::det::sqrt_f64(scaled.length_squared())
        }
    }

    /// Returns a unit-length vector in the direction of `self`, or `None` if zero or non-finite.
    #[inline]
    pub fn try_normalize(self) -> Option<Self> {
        let len_sq = self.length_squared();
        if len_sq >= f64::MIN_POSITIVE && len_sq.is_finite() {
            let inv_len = 1.0 / crate::det::sqrt_f64(len_sq);
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
        let inv_len = 1.0 / crate::det::sqrt_f64(scaled.length_squared());
        Some(scaled * inv_len)
    }

    /// Returns a normalized vector, or [`Self::ZERO`] if zero or non-finite.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        self.try_normalize().unwrap_or(Self::ZERO)
    }

    /// Returns `true` if `self` is finite and within `1e-10` of unit length.
    #[inline]
    pub fn is_normalized(self) -> bool {
        self.is_finite() && (self.length_squared() - 1.0).abs() <= NORMALIZED_LEN_SQ_TOLERANCE_F64
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
            clamp_non_panicking_f64(self.x, min.x, max.x),
            clamp_non_panicking_f64(self.y, min.y, max.y),
            clamp_non_panicking_f64(self.z, min.z, max.z),
        )
    }

    /// Linearly interpolates between `self` (`t = 0.0`) and `rhs` (`t = 1.0`).
    #[inline]
    pub fn lerp(self, rhs: Self, t: f64) -> Self {
        Self::new(
            lerp_scalar_f64(self.x, rhs.x, t),
            lerp_scalar_f64(self.y, rhs.y, t),
            lerp_scalar_f64(self.z, rhs.z, t),
        )
    }

    /// Returns `true` if `|self - rhs|` is componentwise `<= max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(self, rhs: Self, max_abs_diff: f64) -> bool {
        (self.x - rhs.x).abs() <= max_abs_diff
            && (self.y - rhs.y).abs() <= max_abs_diff
            && (self.z - rhs.z).abs() <= max_abs_diff
    }
}

/// Absolute position in 3D world space (`f64`, meters; `SPN-INV-051`).
///
/// `WorldPos` forms an affine space over [`DVec3`]:
/// - `WorldPos - WorldPos -> DVec3` yields the `f64` displacement vector between two world positions.
/// - `WorldPos + DVec3 -> WorldPos` and `WorldPos - DVec3 -> WorldPos` translate a world position.
/// - Adding two `WorldPos` points or mixing `WorldPos` directly with local `f32` [`Vec3`] is
///   rejected at compile time. Use [`camera_relative_f32`] to compute camera-relative `f32`
///   coordinates for GPU upload (`SPN-INV-052`).
///
/// ```
/// use spawn_math::{DVec3, Vec3, WorldPos, camera_relative_f32};
///
/// let cam = WorldPos::new(50_000.0, 120.0, -50_000.0);
/// let mesh = cam + DVec3::new(1.0, 2.0, -5.0);
/// assert_eq!(camera_relative_f32(mesh, cam), Vec3::new(1.0, 2.0, -5.0));
/// ```
///
/// Implicit conversion from `WorldPos` to `Vec3` does not compile (`SPN-INV-051`):
///
/// ```compile_fail
/// use spawn_math::{Vec3, WorldPos};
/// let w = WorldPos::new(1.0, 2.0, 3.0);
/// let _local: Vec3 = w.into();
/// ```
///
/// Adding a local `Vec3` directly to a `WorldPos` does not compile (`SPN-INV-051`):
///
/// ```compile_fail
/// use spawn_math::{Vec3, WorldPos};
/// let w = WorldPos::new(1.0, 2.0, 3.0);
/// let _bad = w + Vec3::ONE;
/// ```
///
/// Adding two `WorldPos` points does not compile:
///
/// ```compile_fail
/// use spawn_math::WorldPos;
/// let _bad = WorldPos::ORIGIN + WorldPos::ORIGIN;
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WorldPos {
    /// Absolute world X coordinate (`+X` right, meters).
    pub x: f64,
    /// Absolute world Y coordinate (`+Y` up, meters).
    pub y: f64,
    /// Absolute world Z coordinate (`-Z` forward, `+Z` back, meters).
    pub z: f64,
}

impl WorldPos {
    /// World origin `(0.0, 0.0, 0.0)`.
    pub const ORIGIN: Self = Self::new(0.0, 0.0, 0.0);

    /// Creates a world position from absolute `f64` meter coordinates.
    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Creates a world position from `[x, y, z]` meters.
    #[inline]
    pub const fn from_array(arr: [f64; 3]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
        }
    }

    /// Converts this world position to `[x, y, z]` meters.
    #[inline]
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// Creates a world position displaced from [`Self::ORIGIN`] by `coords`.
    #[inline]
    pub const fn from_dvec3(coords: DVec3) -> Self {
        Self {
            x: coords.x,
            y: coords.y,
            z: coords.z,
        }
    }

    /// Returns the displacement of this world position from [`Self::ORIGIN`].
    #[inline]
    pub const fn to_dvec3(self) -> DVec3 {
        DVec3::new(self.x, self.y, self.z)
    }

    /// Encodes coordinates in little-endian IEEE-754 byte order (`24` bytes).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 24] {
        self.to_dvec3().to_le_bytes()
    }

    /// Decodes coordinates from little-endian IEEE-754 bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 24]) -> Self {
        Self::from_dvec3(DVec3::from_le_bytes(bytes))
    }

    /// Returns `true` if all coordinates are finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Returns `true` if any coordinate is `NaN`.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.x.is_nan() || self.y.is_nan() || self.z.is_nan()
    }

    /// Squared Euclidean distance (`m^2`) between two world positions.
    #[inline]
    pub fn distance_squared(self, other: Self) -> f64 {
        (self - other).length_squared()
    }

    /// Euclidean distance (`m`) between two world positions.
    #[inline]
    pub fn distance(self, other: Self) -> f64 {
        (self - other).length()
    }

    /// Linearly interpolates between `self` (`alpha = 0.0`) and `other` (`alpha = 1.0`) in `f64`.
    #[inline]
    pub fn lerp(self, other: Self, alpha: f64) -> Self {
        Self::new(
            lerp_scalar_f64(self.x, other.x, alpha),
            lerp_scalar_f64(self.y, other.y, alpha),
            lerp_scalar_f64(self.z, other.z, alpha),
        )
    }

    /// Computes `(self - camera_pos)` in `f64` and narrows the camera-relative offset to `f32`
    /// (`SPN-INV-052`).
    #[inline]
    pub const fn camera_relative_to(self, camera_pos: Self) -> Vec3 {
        camera_relative_f32(self, camera_pos)
    }

    /// Translates this world position by a local `f32` offset widened explicitly to `f64`.
    #[inline]
    pub const fn translate_local_f32(self, local_offset: Vec3) -> Self {
        Self {
            x: self.x + local_offset.x as f64,
            y: self.y + local_offset.y as f64,
            z: self.z + local_offset.z as f64,
        }
    }

    /// Returns `true` if `|self - other|` is coordinatewise `<= max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(self, other: Self, max_abs_diff: f64) -> bool {
        (self - other).abs_diff_eq(DVec3::ZERO, max_abs_diff)
    }
}

/// Subtracts `camera_pos` from `world_pos` in `f64` and converts the relative displacement
/// explicitly to `f32` [`Vec3`] for camera-relative rendering (`SPN-INV-051`, `SPN-INV-052`).
///
/// Performing the subtraction in `f64` before narrowing avoids `f32` world-origin quantization
/// (~`4 mm` ULPs at `±50 km`), preserving sub-micrometer relative accuracy near the camera.
#[inline]
pub const fn camera_relative_f32(world_pos: WorldPos, camera_pos: WorldPos) -> Vec3 {
    DVec3::new(
        world_pos.x - camera_pos.x,
        world_pos.y - camera_pos.y,
        world_pos.z - camera_pos.z,
    )
    .to_vec3_lossy()
}

impl Sub for WorldPos {
    type Output = DVec3;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        DVec3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Add<DVec3> for WorldPos {
    type Output = Self;
    #[inline]
    fn add(self, rhs: DVec3) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign<DVec3> for WorldPos {
    #[inline]
    fn add_assign(&mut self, rhs: DVec3) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl Sub<DVec3> for WorldPos {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: DVec3) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl SubAssign<DVec3> for WorldPos {
    #[inline]
    fn sub_assign(&mut self, rhs: DVec3) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl Add for DVec3 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for DVec3 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl Sub for DVec3 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl SubAssign for DVec3 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl Mul for DVec3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self::new(self.x * rhs.x, self.y * rhs.y, self.z * rhs.z)
    }
}

impl MulAssign for DVec3 {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        self.x *= rhs.x;
        self.y *= rhs.y;
        self.z *= rhs.z;
    }
}

impl Div for DVec3 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        Self::new(self.x / rhs.x, self.y / rhs.y, self.z / rhs.z)
    }
}

impl DivAssign for DVec3 {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        self.x /= rhs.x;
        self.y /= rhs.y;
        self.z /= rhs.z;
    }
}

impl Mul<f64> for DVec3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Mul<DVec3> for f64 {
    type Output = DVec3;
    #[inline]
    fn mul(self, rhs: DVec3) -> Self::Output {
        DVec3::new(self * rhs.x, self * rhs.y, self * rhs.z)
    }
}

impl MulAssign<f64> for DVec3 {
    #[inline]
    fn mul_assign(&mut self, rhs: f64) {
        self.x *= rhs;
        self.y *= rhs;
        self.z *= rhs;
    }
}

impl Div<f64> for DVec3 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f64) -> Self::Output {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl DivAssign<f64> for DVec3 {
    #[inline]
    fn div_assign(&mut self, rhs: f64) {
        self.x /= rhs;
        self.y /= rhs;
        self.z /= rhs;
    }
}

impl Neg for DVec3 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}
