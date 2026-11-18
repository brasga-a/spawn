//! Column-major `f32` matrix primitives (`Mat3`, `Mat4`) and right-handed reversed-Z
//! perspective projections (`SPN-INV-050`, `SPN-INV-053`, ADR-0010).
//!
//! ## Storage and multiplication convention
//!
//! - Matrices are stored in **column-major** layout (`[col][row]`): `x_axis` is column `0`,
//!   `y_axis` is column `1`, `z_axis` is column `2`, and (for [`Mat4`]) `w_axis` is column `3`.
//! - Vectors are treated as **column vectors** multiplied on the right (`M * v`), matching
//!   SPIR-V and Slang default column-major matrices.
//! - Singular or non-finite `inverse()` calls return `None` without panicking.
//!
//! ## Reversed-Z and single-site Vulkan NDC Y-flip (`SPN-INV-053`)
//!
//! [`Mat4::perspective_reversed_z_rh`] and [`Mat4::perspective_infinite_reversed_z_rh`] map
//! right-handed eye space (`+X` right, `+Y` up, `-Z` into the scene) directly into Vulkan
//! `[0, 1]` clip space with **reversed-Z** (`z_e = -near -> 1.0`, `z_e = -far -> 0.0`) and
//! negate clip `Y` (`y_clip = -g * y_e`) in this single site so `VkViewport` keeps positive
//! height (`height > 0`) and glTF right-handed counter-clockwise front faces yield positive
//! Vulkan polygon signed area (`a > 0` -> `VK_FRONT_FACE_COUNTER_CLOCKWISE`).

use std::ops::{Add, AddAssign, Mul, MulAssign, Sub, SubAssign};

use crate::{MathError, MathErrorKind, Quat, Vec3, Vec4};

/// 3x3 column-major single-precision matrix (`f32`).
///
/// [`Default`] returns [`Mat3::IDENTITY`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    /// Column 0 (`[c0r0, c0r1, c0r2]`).
    pub x_axis: Vec3,
    /// Column 1 (`[c1r0, c1r1, c1r2]`).
    pub y_axis: Vec3,
    /// Column 2 (`[c2r0, c2r1, c2r2]`).
    pub z_axis: Vec3,
}

impl Default for Mat3 {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    /// Zero matrix.
    pub const ZERO: Self = Self::from_cols(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    /// Identity matrix.
    pub const IDENTITY: Self = Self::from_cols(Vec3::X, Vec3::Y, Vec3::Z);
    /// `(NaN)` matrix.
    pub const NAN: Self = Self::from_cols(Vec3::NAN, Vec3::NAN, Vec3::NAN);

    /// Constructs a 3x3 matrix from its three column vectors.
    #[inline]
    pub const fn from_cols(x_axis: Vec3, y_axis: Vec3, z_axis: Vec3) -> Self {
        Self {
            x_axis,
            y_axis,
            z_axis,
        }
    }

    /// Constructs a 3x3 matrix from flat column-major `[c0r0, c0r1, c0r2, c1r0, ...]`.
    #[inline]
    pub const fn from_cols_array(m: &[f32; 9]) -> Self {
        Self::from_cols(
            Vec3::new(m[0], m[1], m[2]),
            Vec3::new(m[3], m[4], m[5]),
            Vec3::new(m[6], m[7], m[8]),
        )
    }

    /// Converts this matrix to flat column-major `[f32; 9]`.
    #[inline]
    pub const fn to_cols_array(&self) -> [f32; 9] {
        [
            self.x_axis.x,
            self.x_axis.y,
            self.x_axis.z,
            self.y_axis.x,
            self.y_axis.y,
            self.y_axis.z,
            self.z_axis.x,
            self.z_axis.y,
            self.z_axis.z,
        ]
    }

    /// Constructs a 3x3 matrix from nested column-major `[[f32; 3]; 3]` (`cols[col][row]`).
    #[inline]
    pub const fn from_cols_array_2d(cols: &[[f32; 3]; 3]) -> Self {
        Self::from_cols(
            Vec3::from_array(cols[0]),
            Vec3::from_array(cols[1]),
            Vec3::from_array(cols[2]),
        )
    }

    /// Converts this matrix to nested column-major `[[f32; 3]; 3]` (`cols[col][row]`).
    #[inline]
    pub const fn to_cols_array_2d(&self) -> [[f32; 3]; 3] {
        [
            self.x_axis.to_array(),
            self.y_axis.to_array(),
            self.z_axis.to_array(),
        ]
    }

    /// Diagonal 3x3 matrix from `diagonal`.
    #[inline]
    pub const fn from_diagonal(diagonal: Vec3) -> Self {
        Self::from_cols(
            Vec3::new(diagonal.x, 0.0, 0.0),
            Vec3::new(0.0, diagonal.y, 0.0),
            Vec3::new(0.0, 0.0, diagonal.z),
        )
    }

    /// Non-uniform scale matrix.
    #[inline]
    pub const fn from_scale(scale: Vec3) -> Self {
        Self::from_diagonal(scale)
    }

    /// Right-handed rotation matrix around `+X` by `angle_radians` (or [`Self::IDENTITY`] if non-finite).
    #[inline]
    pub fn from_rotation_x(angle_radians: f32) -> Self {
        if !angle_radians.is_finite() {
            return Self::IDENTITY;
        }
        let (s, c) = angle_radians.sin_cos();
        Self::from_cols(Vec3::X, Vec3::new(0.0, c, s), Vec3::new(0.0, -s, c))
    }

    /// Right-handed rotation matrix around `+Y` by `angle_radians` (or [`Self::IDENTITY`] if non-finite).
    #[inline]
    pub fn from_rotation_y(angle_radians: f32) -> Self {
        if !angle_radians.is_finite() {
            return Self::IDENTITY;
        }
        let (s, c) = angle_radians.sin_cos();
        Self::from_cols(Vec3::new(c, 0.0, -s), Vec3::Y, Vec3::new(s, 0.0, c))
    }

    /// Right-handed rotation matrix around `+Z` by `angle_radians` (or [`Self::IDENTITY`] if non-finite).
    #[inline]
    pub fn from_rotation_z(angle_radians: f32) -> Self {
        if !angle_radians.is_finite() {
            return Self::IDENTITY;
        }
        let (s, c) = angle_radians.sin_cos();
        Self::from_cols(Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0), Vec3::Z)
    }

    /// Right-handed rotation matrix around `axis` by `angle_radians`.
    #[inline]
    pub fn try_from_axis_angle(axis: Vec3, angle_radians: f32) -> Option<Self> {
        Quat::try_from_axis_angle(axis, angle_radians).map(Self::from_quat)
    }

    /// Converts a rotation [`Quat`] (normalized internally, or [`Self::IDENTITY`] if degenerate) to `Mat3`.
    #[inline]
    pub fn from_quat(rotation: Quat) -> Self {
        let q = rotation.normalize_or_identity();
        let x2 = q.x + q.x;
        let y2 = q.y + q.y;
        let z2 = q.z + q.z;
        let xx = q.x * x2;
        let xy = q.x * y2;
        let xz = q.x * z2;
        let yy = q.y * y2;
        let yz = q.y * z2;
        let zz = q.z * z2;
        let wx = q.w * x2;
        let wy = q.w * y2;
        let wz = q.w * z2;

        Self::from_cols(
            Vec3::new(1.0 - (yy + zz), xy + wz, xz - wy),
            Vec3::new(xy - wz, 1.0 - (xx + zz), yz + wx),
            Vec3::new(xz + wy, yz - wx, 1.0 - (xx + yy)),
        )
    }

    /// Computes the normal-transform basis `(R * S)^{-T}` for shading normals under
    /// non-uniform `scale` and `rotation`, returning `None` if any scale axis is zero or non-finite.
    #[inline]
    pub fn normal_matrix(scale: Vec3, rotation: Quat) -> Option<Self> {
        if !scale.is_finite() || !rotation.is_finite() {
            return None;
        }
        if scale.x == 0.0 || scale.y == 0.0 || scale.z == 0.0 {
            return None;
        }
        let inv_scale = Vec3::new(1.0 / scale.x, 1.0 / scale.y, 1.0 / scale.z);
        if !inv_scale.is_finite() {
            return None;
        }
        let rot_mat = Self::from_quat(rotation);
        let result = Self::from_cols(
            rot_mat.x_axis * inv_scale.x,
            rot_mat.y_axis * inv_scale.y,
            rot_mat.z_axis * inv_scale.z,
        );
        result.is_finite().then_some(result)
    }

    /// Non-panicking column lookup (`0..3`).
    #[inline]
    pub const fn col(&self, index: usize) -> Option<Vec3> {
        match index {
            0 => Some(self.x_axis),
            1 => Some(self.y_axis),
            2 => Some(self.z_axis),
            _ => None,
        }
    }

    /// Non-panicking row lookup (`0..3`).
    #[inline]
    pub const fn row(&self, index: usize) -> Option<Vec3> {
        match index {
            0 => Some(Vec3::new(self.x_axis.x, self.y_axis.x, self.z_axis.x)),
            1 => Some(Vec3::new(self.x_axis.y, self.y_axis.y, self.z_axis.y)),
            2 => Some(Vec3::new(self.x_axis.z, self.y_axis.z, self.z_axis.z)),
            _ => None,
        }
    }

    /// Matrix transpose `M^T`.
    #[inline]
    pub const fn transpose(&self) -> Self {
        Self::from_cols(
            Vec3::new(self.x_axis.x, self.y_axis.x, self.z_axis.x),
            Vec3::new(self.x_axis.y, self.y_axis.y, self.z_axis.y),
            Vec3::new(self.x_axis.z, self.y_axis.z, self.z_axis.z),
        )
    }

    /// Matrix determinant `det(M)`.
    #[inline]
    pub fn determinant(&self) -> f32 {
        self.z_axis.dot(self.x_axis.cross(self.y_axis))
    }

    /// Non-panicking matrix inverse `M^-1`. Returns `None` if `self` is singular, near-singular
    /// (overflowing `f32`), or non-finite.
    #[inline]
    pub fn inverse(&self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let tmp0 = self.y_axis.cross(self.z_axis);
        let tmp1 = self.z_axis.cross(self.x_axis);
        let tmp2 = self.x_axis.cross(self.y_axis);
        let det = self.z_axis.dot(tmp2);
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        if !inv_det.is_finite() {
            return None;
        }
        let inv = Self::from_cols(tmp0 * inv_det, tmp1 * inv_det, tmp2 * inv_det).transpose();
        inv.is_finite().then_some(inv)
    }

    /// Right-multiplies column vector `rhs` (`M * rhs`).
    #[inline]
    pub fn mul_vec3(&self, rhs: Vec3) -> Vec3 {
        self.x_axis * rhs.x + self.y_axis * rhs.y + self.z_axis * rhs.z
    }

    /// Matrix product `self * rhs`.
    #[inline]
    pub fn mul_mat3(&self, rhs: &Self) -> Self {
        Self::from_cols(
            self.mul_vec3(rhs.x_axis),
            self.mul_vec3(rhs.y_axis),
            self.mul_vec3(rhs.z_axis),
        )
    }

    /// Returns `true` if all 9 elements are finite.
    #[inline]
    pub fn is_finite(&self) -> bool {
        self.x_axis.is_finite() && self.y_axis.is_finite() && self.z_axis.is_finite()
    }

    /// Returns `true` if any element is `NaN`.
    #[inline]
    pub fn is_nan(&self) -> bool {
        self.x_axis.is_nan() || self.y_axis.is_nan() || self.z_axis.is_nan()
    }

    /// Returns `true` if every element differs by at most `max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(&self, rhs: &Self, max_abs_diff: f32) -> bool {
        self.x_axis.abs_diff_eq(rhs.x_axis, max_abs_diff)
            && self.y_axis.abs_diff_eq(rhs.y_axis, max_abs_diff)
            && self.z_axis.abs_diff_eq(rhs.z_axis, max_abs_diff)
    }
}

/// 4x4 column-major single-precision matrix (`f32`).
///
/// [`Default`] returns [`Mat4::IDENTITY`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    /// Column 0 (`[c0r0, c0r1, c0r2, c0r3]`).
    pub x_axis: Vec4,
    /// Column 1 (`[c1r0, c1r1, c1r2, c1r3]`).
    pub y_axis: Vec4,
    /// Column 2 (`[c2r0, c2r1, c2r2, c2r3]`).
    pub z_axis: Vec4,
    /// Column 3 (`[c3r0, c3r1, c3r2, c3r3]`; affine translation in `.xyz()` with `w = 1.0`).
    pub w_axis: Vec4,
}

impl Default for Mat4 {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    /// Zero matrix.
    pub const ZERO: Self = Self::from_cols(Vec4::ZERO, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO);
    /// Identity matrix.
    pub const IDENTITY: Self = Self::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::W);
    /// `(NaN)` matrix.
    pub const NAN: Self = Self::from_cols(Vec4::NAN, Vec4::NAN, Vec4::NAN, Vec4::NAN);

    /// Constructs a 4x4 matrix from its four column vectors.
    #[inline]
    pub const fn from_cols(x_axis: Vec4, y_axis: Vec4, z_axis: Vec4, w_axis: Vec4) -> Self {
        Self {
            x_axis,
            y_axis,
            z_axis,
            w_axis,
        }
    }

    /// Constructs a 4x4 matrix from flat column-major `[f32; 16]`.
    #[inline]
    pub const fn from_cols_array(m: &[f32; 16]) -> Self {
        Self::from_cols(
            Vec4::new(m[0], m[1], m[2], m[3]),
            Vec4::new(m[4], m[5], m[6], m[7]),
            Vec4::new(m[8], m[9], m[10], m[11]),
            Vec4::new(m[12], m[13], m[14], m[15]),
        )
    }

    /// Converts this matrix to flat column-major `[f32; 16]`.
    #[inline]
    pub const fn to_cols_array(&self) -> [f32; 16] {
        [
            self.x_axis.x,
            self.x_axis.y,
            self.x_axis.z,
            self.x_axis.w,
            self.y_axis.x,
            self.y_axis.y,
            self.y_axis.z,
            self.y_axis.w,
            self.z_axis.x,
            self.z_axis.y,
            self.z_axis.z,
            self.z_axis.w,
            self.w_axis.x,
            self.w_axis.y,
            self.w_axis.z,
            self.w_axis.w,
        ]
    }

    /// Constructs a 4x4 matrix from nested column-major `[[f32; 4]; 4]` (`cols[col][row]`).
    #[inline]
    pub const fn from_cols_array_2d(cols: &[[f32; 4]; 4]) -> Self {
        Self::from_cols(
            Vec4::from_array(cols[0]),
            Vec4::from_array(cols[1]),
            Vec4::from_array(cols[2]),
            Vec4::from_array(cols[3]),
        )
    }

    /// Converts this matrix to nested column-major `[[f32; 4]; 4]` (`cols[col][row]`).
    #[inline]
    pub const fn to_cols_array_2d(&self) -> [[f32; 4]; 4] {
        [
            self.x_axis.to_array(),
            self.y_axis.to_array(),
            self.z_axis.to_array(),
            self.w_axis.to_array(),
        ]
    }

    /// Encodes all 16 column-major `f32` values in little-endian byte order (`64` bytes).
    #[inline]
    pub fn to_le_bytes(&self) -> [u8; 64] {
        let mut out = [0_u8; 64];
        let cols = self.to_cols_array();
        for (idx, val) in cols.into_iter().enumerate() {
            let b = val.to_le_bytes();
            let base = idx * 4;
            out[base] = b[0];
            out[base + 1] = b[1];
            out[base + 2] = b[2];
            out[base + 3] = b[3];
        }
        out
    }

    /// Decodes a column-major `Mat4` from 64 little-endian IEEE-754 bytes.
    #[inline]
    pub fn from_le_bytes(bytes: [u8; 64]) -> Self {
        let mut cols = [0.0_f32; 16];
        for (idx, slot) in cols.iter_mut().enumerate() {
            let base = idx * 4;
            *slot = f32::from_le_bytes([
                bytes[base],
                bytes[base + 1],
                bytes[base + 2],
                bytes[base + 3],
            ]);
        }
        Self::from_cols_array(&cols)
    }

    /// Diagonal 4x4 matrix.
    #[inline]
    pub const fn from_diagonal(diagonal: Vec4) -> Self {
        Self::from_cols(
            Vec4::new(diagonal.x, 0.0, 0.0, 0.0),
            Vec4::new(0.0, diagonal.y, 0.0, 0.0),
            Vec4::new(0.0, 0.0, diagonal.z, 0.0),
            Vec4::new(0.0, 0.0, 0.0, diagonal.w),
        )
    }

    /// Affine 3D non-uniform scale matrix.
    #[inline]
    pub const fn from_scale(scale: Vec3) -> Self {
        Self::from_diagonal(Vec4::new(scale.x, scale.y, scale.z, 1.0))
    }

    /// Affine 3D translation matrix.
    #[inline]
    pub const fn from_translation(translation: Vec3) -> Self {
        Self::from_cols(Vec4::X, Vec4::Y, Vec4::Z, translation.extend(1.0))
    }

    /// Embeds a 3x3 linear matrix into homogeneous 4x4 (`w_axis = (0, 0, 0, 1)`).
    #[inline]
    pub const fn from_mat3(m: Mat3) -> Self {
        Self::from_cols(
            m.x_axis.extend(0.0),
            m.y_axis.extend(0.0),
            m.z_axis.extend(0.0),
            Vec4::W,
        )
    }

    /// Extracts the upper-left 3x3 linear submatrix.
    #[inline]
    pub const fn to_mat3(&self) -> Mat3 {
        Mat3::from_cols(self.x_axis.xyz(), self.y_axis.xyz(), self.z_axis.xyz())
    }

    /// Homogeneous rotation matrix from `rotation`.
    #[inline]
    pub fn from_quat(rotation: Quat) -> Self {
        Self::from_mat3(Mat3::from_quat(rotation))
    }

    /// Right-handed rotation matrix around `+X` by `angle_radians`.
    #[inline]
    pub fn from_rotation_x(angle_radians: f32) -> Self {
        Self::from_mat3(Mat3::from_rotation_x(angle_radians))
    }

    /// Right-handed rotation matrix around `+Y` by `angle_radians`.
    #[inline]
    pub fn from_rotation_y(angle_radians: f32) -> Self {
        Self::from_mat3(Mat3::from_rotation_y(angle_radians))
    }

    /// Right-handed rotation matrix around `+Z` by `angle_radians`.
    #[inline]
    pub fn from_rotation_z(angle_radians: f32) -> Self {
        Self::from_mat3(Mat3::from_rotation_z(angle_radians))
    }

    /// Right-handed rotation matrix around `axis` by `angle_radians`.
    #[inline]
    pub fn try_from_axis_angle(axis: Vec3, angle_radians: f32) -> Option<Self> {
        Mat3::try_from_axis_angle(axis, angle_radians).map(Self::from_mat3)
    }

    /// Affine rigid transform `T(translation) * R(rotation)`.
    #[inline]
    pub fn from_rotation_translation(rotation: Quat, translation: Vec3) -> Self {
        let m = Mat3::from_quat(rotation);
        Self::from_cols(
            m.x_axis.extend(0.0),
            m.y_axis.extend(0.0),
            m.z_axis.extend(0.0),
            translation.extend(1.0),
        )
    }

    /// Affine `T(translation) * R(rotation) * S(scale)` transform (`M * p` scales first,
    /// rotates second, and translates last).
    ///
    /// For camera-relative rendering (`SPN-INV-052`), pass [`crate::camera_relative_f32`]
    /// as `translation` so the GPU never receives absolute world coordinates.
    #[inline]
    pub fn from_scale_rotation_translation(scale: Vec3, rotation: Quat, translation: Vec3) -> Self {
        let m = Mat3::from_quat(rotation);
        Self::from_cols(
            (m.x_axis * scale.x).extend(0.0),
            (m.y_axis * scale.y).extend(0.0),
            (m.z_axis * scale.z).extend(0.0),
            translation.extend(1.0),
        )
    }

    /// Decomposes a shear-free affine `T * R * S` matrix into `(scale, rotation, translation)`.
    ///
    /// Negative-determinant reflections absorb `det.signum()` into `scale.x` so `rotation`
    /// remains a right-handed `SO(3)` unit quaternion.
    ///
    /// Returns `None` if `self` is non-finite, projective (`row(3) != (0, 0, 0, 1)`),
    /// singular (`det == 0`), or sheared (columns are not mutually orthogonal).
    #[inline]
    pub fn try_to_scale_rotation_translation(&self) -> Option<(Vec3, Quat, Vec3)> {
        if !self.is_finite() {
            return None;
        }
        if (self.x_axis.w).abs() > 1.0e-5
            || (self.y_axis.w).abs() > 1.0e-5
            || (self.z_axis.w).abs() > 1.0e-5
            || (self.w_axis.w - 1.0).abs() > 1.0e-5
        {
            return None;
        }
        let c0 = self.x_axis.xyz();
        let c1 = self.y_axis.xyz();
        let c2 = self.z_axis.xyz();
        let lx = c0.length();
        let ly = c1.length();
        let lz = c2.length();
        if lx == 0.0
            || ly == 0.0
            || lz == 0.0
            || !lx.is_finite()
            || !ly.is_finite()
            || !lz.is_finite()
        {
            return None;
        }
        let u0 = c0.try_normalize()?;
        let u1 = c1.try_normalize()?;
        let u2 = c2.try_normalize()?;
        let unit_det = Mat3::from_cols(u0, u1, u2).determinant();
        if unit_det == 0.0 || !unit_det.is_finite() {
            return None;
        }
        let sign = unit_det.signum();
        let scale = Vec3::new(lx * sign, ly, lz);
        let rot_mat = Mat3::from_cols(u0 * sign, u1, u2);
        let rotation = Quat::try_from_mat3(rot_mat)?;
        let translation = self.w_axis.xyz();
        Some((scale, rotation, translation))
    }

    /// Right-handed world-to-view matrix from `eye` looking along `dir` (`-Z_eye = normalize(dir)`)
    /// with reference `up`.
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0004` [`MathErrorKind::DegenerateDirection`]) if `eye`, `dir`,
    /// or `up` is non-finite, `dir` or `up` is zero-length, or `dir` is collinear with `up`.
    #[track_caller]
    pub fn try_look_to_rh(eye: Vec3, dir: Vec3, up: Vec3) -> Result<Self, MathError> {
        if !eye.is_finite() || !dir.is_finite() || !up.is_finite() {
            return Err(
                MathError::new(MathErrorKind::DegenerateDirection).with_detail(format!(
                    "non-finite look_to_rh input: eye={eye:?}, dir={dir:?}, up={up:?}"
                )),
            );
        }
        let Some(f) = dir.try_normalize() else {
            return Err(
                MathError::new(MathErrorKind::DegenerateDirection).with_detail("dir is zero")
            );
        };
        let Some(s) = f.cross(up).try_normalize() else {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail("dir and up are zero or collinear"));
        };
        let u = s.cross(f);
        Ok(Self::from_cols(
            Vec4::new(s.x, u.x, -f.x, 0.0),
            Vec4::new(s.y, u.y, -f.y, 0.0),
            Vec4::new(s.z, u.z, -f.z, 0.0),
            Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
        ))
    }

    /// Right-handed world-to-view matrix from `eye` looking toward `target` with reference `up`.
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0004` [`MathErrorKind::DegenerateDirection`]) if `eye == target`,
    /// `target - eye` is collinear with `up`, or any input is non-finite.
    #[track_caller]
    pub fn try_look_at_rh(eye: Vec3, target: Vec3, up: Vec3) -> Result<Self, MathError> {
        Self::try_look_to_rh(eye, target - eye, up)
    }

    /// Finite right-handed perspective projection for Vulkan `[0, 1]` clip depth with
    /// **reversed-Z** (`near -> 1.0`, `far -> 0.0`) and **single-site Vulkan NDC Y-flip**
    /// (`y_clip = -g * y_e`; `SPN-INV-053`).
    ///
    /// Eye space uses `+X` right, `+Y` up, and `-Z` forward (`z_e < 0` in front of camera,
    /// `w_clip = -z_e > 0`).
    ///
    /// # Errors
    ///
    /// - `MATH-0001` ([`MathErrorKind::InvalidFov`]) if `fov_y_radians` is non-finite, `<= 0`, or `>= π`.
    /// - `MATH-0002` ([`MathErrorKind::InvalidAspectRatio`]) if `aspect_ratio` is non-finite or `<= 0`.
    /// - `MATH-0003` ([`MathErrorKind::InvalidClipPlanes`]) if `near`/`far` are non-finite, `near <= 0`, or `far <= near`.
    #[track_caller]
    pub fn perspective_reversed_z_rh(
        fov_y_radians: f32,
        aspect_ratio: f32,
        near: f32,
        far: f32,
    ) -> Result<Self, MathError> {
        let g = validate_perspective_fov_and_aspect(fov_y_radians, aspect_ratio)?;
        if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
            return Err(MathError::new(MathErrorKind::InvalidClipPlanes)
                .with_detail(format!("near = {near}, far = {far}")));
        }
        let x_scale = g / aspect_ratio;
        let k = near / (far - near);
        let z_bias = far * k;
        if !x_scale.is_finite() || !k.is_finite() || !z_bias.is_finite() {
            return Err(MathError::new(MathErrorKind::InvalidClipPlanes).with_detail(format!(
                "projection coefficients overflowed: near = {near}, far = {far}, aspect = {aspect_ratio}"
            )));
        }
        Ok(Self::from_cols(
            Vec4::new(x_scale, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -g, 0.0, 0.0),
            Vec4::new(0.0, 0.0, k, -1.0),
            Vec4::new(0.0, 0.0, z_bias, 0.0),
        ))
    }

    /// Infinite-far right-handed perspective projection (`far -> ∞`) for Vulkan `[0, 1]` clip
    /// depth with **reversed-Z** (`z_ndc = near / (-z_e)`, so `near -> 1.0` and `∞ -> 0.0`)
    /// and **single-site Vulkan NDC Y-flip** (`y_clip = -g * y_e`; `SPN-INV-053`).
    ///
    /// # Errors
    ///
    /// - `MATH-0001` ([`MathErrorKind::InvalidFov`]) if `fov_y_radians` is non-finite, `<= 0`, or `>= π`.
    /// - `MATH-0002` ([`MathErrorKind::InvalidAspectRatio`]) if `aspect_ratio` is non-finite or `<= 0`.
    /// - `MATH-0003` ([`MathErrorKind::InvalidClipPlanes`]) if `near` is non-finite or `<= 0`.
    #[track_caller]
    pub fn perspective_infinite_reversed_z_rh(
        fov_y_radians: f32,
        aspect_ratio: f32,
        near: f32,
    ) -> Result<Self, MathError> {
        let g = validate_perspective_fov_and_aspect(fov_y_radians, aspect_ratio)?;
        if !near.is_finite() || near <= 0.0 {
            return Err(MathError::new(MathErrorKind::InvalidClipPlanes)
                .with_detail(format!("near = {near}")));
        }
        let x_scale = g / aspect_ratio;
        if !x_scale.is_finite() {
            return Err(MathError::new(MathErrorKind::InvalidAspectRatio).with_detail(format!(
                "x_scale overflowed: fov_y_radians = {fov_y_radians}, aspect_ratio = {aspect_ratio}"
            )));
        }
        Ok(Self::from_cols(
            Vec4::new(x_scale, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -g, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, near, 0.0),
        ))
    }

    /// Non-panicking column lookup (`0..4`).
    #[inline]
    pub const fn col(&self, index: usize) -> Option<Vec4> {
        match index {
            0 => Some(self.x_axis),
            1 => Some(self.y_axis),
            2 => Some(self.z_axis),
            3 => Some(self.w_axis),
            _ => None,
        }
    }

    /// Non-panicking row lookup (`0..4`).
    #[inline]
    pub const fn row(&self, index: usize) -> Option<Vec4> {
        match index {
            0 => Some(Vec4::new(
                self.x_axis.x,
                self.y_axis.x,
                self.z_axis.x,
                self.w_axis.x,
            )),
            1 => Some(Vec4::new(
                self.x_axis.y,
                self.y_axis.y,
                self.z_axis.y,
                self.w_axis.y,
            )),
            2 => Some(Vec4::new(
                self.x_axis.z,
                self.y_axis.z,
                self.z_axis.z,
                self.w_axis.z,
            )),
            3 => Some(Vec4::new(
                self.x_axis.w,
                self.y_axis.w,
                self.z_axis.w,
                self.w_axis.w,
            )),
            _ => None,
        }
    }

    /// Matrix transpose `M^T`.
    #[inline]
    pub const fn transpose(&self) -> Self {
        Self::from_cols(
            Vec4::new(self.x_axis.x, self.y_axis.x, self.z_axis.x, self.w_axis.x),
            Vec4::new(self.x_axis.y, self.y_axis.y, self.z_axis.y, self.w_axis.y),
            Vec4::new(self.x_axis.z, self.y_axis.z, self.z_axis.z, self.w_axis.z),
            Vec4::new(self.x_axis.w, self.y_axis.w, self.z_axis.w, self.w_axis.w),
        )
    }

    /// Matrix determinant `det(M)`.
    #[inline]
    pub fn determinant(&self) -> f32 {
        let c0 = self.x_axis;
        let c1 = self.y_axis;
        let c2 = self.z_axis;
        let c3 = self.w_axis;

        let s0 = c0.x * c1.y - c1.x * c0.y;
        let s1 = c0.x * c1.z - c1.x * c0.z;
        let s2 = c0.x * c1.w - c1.x * c0.w;
        let s3 = c0.y * c1.z - c1.y * c0.z;
        let s4 = c0.y * c1.w - c1.y * c0.w;
        let s5 = c0.z * c1.w - c1.z * c0.w;

        let k0 = c2.x * c3.y - c3.x * c2.y;
        let k1 = c2.x * c3.z - c3.x * c2.z;
        let k2 = c2.x * c3.w - c3.x * c2.w;
        let k3 = c2.y * c3.z - c3.y * c2.z;
        let k4 = c2.y * c3.w - c3.y * c2.w;
        let k5 = c2.z * c3.w - c3.z * c2.w;

        s0 * k5 - s1 * k4 + s2 * k3 + s3 * k2 - s4 * k1 + s5 * k0
    }

    /// Non-panicking 4x4 matrix inverse `M^-1`. Returns `None` if `self` is singular,
    /// near-singular (overflowing `f32`), or non-finite.
    #[inline]
    pub fn inverse(&self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let c0 = self.x_axis;
        let c1 = self.y_axis;
        let c2 = self.z_axis;
        let c3 = self.w_axis;

        let s0 = c0.x * c1.y - c1.x * c0.y;
        let s1 = c0.x * c1.z - c1.x * c0.z;
        let s2 = c0.x * c1.w - c1.x * c0.w;
        let s3 = c0.y * c1.z - c1.y * c0.z;
        let s4 = c0.y * c1.w - c1.y * c0.w;
        let s5 = c0.z * c1.w - c1.z * c0.w;

        let k0 = c2.x * c3.y - c3.x * c2.y;
        let k1 = c2.x * c3.z - c3.x * c2.z;
        let k2 = c2.x * c3.w - c3.x * c2.w;
        let k3 = c2.y * c3.z - c3.y * c2.z;
        let k4 = c2.y * c3.w - c3.y * c2.w;
        let k5 = c2.z * c3.w - c3.z * c2.w;

        let det = s0 * k5 - s1 * k4 + s2 * k3 + s3 * k2 - s4 * k1 + s5 * k0;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        if !inv_det.is_finite() {
            return None;
        }

        let r0 = Vec4::new(
            c1.y * k5 - c1.z * k4 + c1.w * k3,
            -(c1.x * k5 - c1.z * k2 + c1.w * k1),
            c1.x * k4 - c1.y * k2 + c1.w * k0,
            -(c1.x * k3 - c1.y * k1 + c1.z * k0),
        );
        let r1 = Vec4::new(
            -(c0.y * k5 - c0.z * k4 + c0.w * k3),
            c0.x * k5 - c0.z * k2 + c0.w * k1,
            -(c0.x * k4 - c0.y * k2 + c0.w * k0),
            c0.x * k3 - c0.y * k1 + c0.z * k0,
        );
        let r2 = Vec4::new(
            c3.y * s5 - c3.z * s4 + c3.w * s3,
            -(c3.x * s5 - c3.z * s2 + c3.w * s1),
            c3.x * s4 - c3.y * s2 + c3.w * s0,
            -(c3.x * s3 - c3.y * s1 + c3.z * s0),
        );
        let r3 = Vec4::new(
            -(c2.y * s5 - c2.z * s4 + c2.w * s3),
            c2.x * s5 - c2.z * s2 + c2.w * s1,
            -(c2.x * s4 - c2.y * s2 + c2.w * s0),
            c2.x * s3 - c2.y * s1 + c2.z * s0,
        );

        let inv = Self::from_cols(
            Vec4::new(r0.x, r1.x, r2.x, r3.x) * inv_det,
            Vec4::new(r0.y, r1.y, r2.y, r3.y) * inv_det,
            Vec4::new(r0.z, r1.z, r2.z, r3.z) * inv_det,
            Vec4::new(r0.w, r1.w, r2.w, r3.w) * inv_det,
        );
        inv.is_finite().then_some(inv)
    }

    /// Right-multiplies homogeneous column vector `rhs` (`M * rhs`).
    #[inline]
    pub fn mul_vec4(&self, rhs: Vec4) -> Vec4 {
        self.x_axis * rhs.x + self.y_axis * rhs.y + self.z_axis * rhs.z + self.w_axis * rhs.w
    }

    /// Transforms an affine 3D point `(point.x, point.y, point.z, 1.0)` without perspective divide.
    #[inline]
    pub fn transform_point3(&self, point: Vec3) -> Vec3 {
        self.x_axis.xyz() * point.x
            + self.y_axis.xyz() * point.y
            + self.z_axis.xyz() * point.z
            + self.w_axis.xyz()
    }

    /// Transforms a 3D direction/offset vector `(vector.x, vector.y, vector.z, 0.0)`.
    #[inline]
    pub fn transform_vector3(&self, vector: Vec3) -> Vec3 {
        self.x_axis.xyz() * vector.x + self.y_axis.xyz() * vector.y + self.z_axis.xyz() * vector.z
    }

    /// Projects a 3D point into normalized device coordinates `(clip.xyz / clip.w)`,
    /// returning `None` if `clip.w == 0` or the result is non-finite.
    #[inline]
    pub fn project_point3(&self, point: Vec3) -> Option<Vec3> {
        self.mul_vec4(point.extend(1.0)).perspective_divide()
    }

    /// Matrix product `self * rhs`.
    #[inline]
    pub fn mul_mat4(&self, rhs: &Self) -> Self {
        Self::from_cols(
            self.mul_vec4(rhs.x_axis),
            self.mul_vec4(rhs.y_axis),
            self.mul_vec4(rhs.z_axis),
            self.mul_vec4(rhs.w_axis),
        )
    }

    /// Returns `true` if all 16 elements are finite.
    #[inline]
    pub fn is_finite(&self) -> bool {
        self.x_axis.is_finite()
            && self.y_axis.is_finite()
            && self.z_axis.is_finite()
            && self.w_axis.is_finite()
    }

    /// Returns `true` if any element is `NaN`.
    #[inline]
    pub fn is_nan(&self) -> bool {
        self.x_axis.is_nan() || self.y_axis.is_nan() || self.z_axis.is_nan() || self.w_axis.is_nan()
    }

    /// Returns `true` if every element differs by at most `max_abs_diff`.
    #[inline]
    pub fn abs_diff_eq(&self, rhs: &Self, max_abs_diff: f32) -> bool {
        self.x_axis.abs_diff_eq(rhs.x_axis, max_abs_diff)
            && self.y_axis.abs_diff_eq(rhs.y_axis, max_abs_diff)
            && self.z_axis.abs_diff_eq(rhs.z_axis, max_abs_diff)
            && self.w_axis.abs_diff_eq(rhs.w_axis, max_abs_diff)
    }
}

#[track_caller]
#[inline]
fn validate_perspective_fov_and_aspect(
    fov_y_radians: f32,
    aspect_ratio: f32,
) -> Result<f32, MathError> {
    if !fov_y_radians.is_finite() || fov_y_radians <= 0.0 || fov_y_radians >= std::f32::consts::PI {
        return Err(MathError::new(MathErrorKind::InvalidFov)
            .with_detail(format!("fov_y_radians = {fov_y_radians}")));
    }
    if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
        return Err(MathError::new(MathErrorKind::InvalidAspectRatio)
            .with_detail(format!("aspect_ratio = {aspect_ratio}")));
    }
    let half_tan = (fov_y_radians * 0.5).tan();
    if !half_tan.is_finite() || half_tan <= 0.0 {
        return Err(MathError::new(MathErrorKind::InvalidFov)
            .with_detail(format!("fov_y_radians = {fov_y_radians}")));
    }
    let g = 1.0 / half_tan;
    if !g.is_finite() || g <= 0.0 {
        return Err(MathError::new(MathErrorKind::InvalidFov)
            .with_detail(format!("fov_y_radians = {fov_y_radians}")));
    }
    Ok(g)
}

impl Mul<Vec3> for Mat3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, rhs: Vec3) -> Self::Output {
        self.mul_vec3(rhs)
    }
}

impl Mul for Mat3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        self.mul_mat3(&rhs)
    }
}

impl MulAssign for Mat3 {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = self.mul_mat3(&rhs);
    }
}

impl Mul<f32> for Mat3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self::from_cols(self.x_axis * rhs, self.y_axis * rhs, self.z_axis * rhs)
    }
}

impl Add for Mat3 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self::from_cols(
            self.x_axis + rhs.x_axis,
            self.y_axis + rhs.y_axis,
            self.z_axis + rhs.z_axis,
        )
    }
}

impl AddAssign for Mat3 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x_axis += rhs.x_axis;
        self.y_axis += rhs.y_axis;
        self.z_axis += rhs.z_axis;
    }
}

impl Sub for Mat3 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self::from_cols(
            self.x_axis - rhs.x_axis,
            self.y_axis - rhs.y_axis,
            self.z_axis - rhs.z_axis,
        )
    }
}

impl SubAssign for Mat3 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x_axis -= rhs.x_axis;
        self.y_axis -= rhs.y_axis;
        self.z_axis -= rhs.z_axis;
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    #[inline]
    fn mul(self, rhs: Vec4) -> Self::Output {
        self.mul_vec4(rhs)
    }
}

impl Mul for Mat4 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        self.mul_mat4(&rhs)
    }
}

impl MulAssign for Mat4 {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = self.mul_mat4(&rhs);
    }
}

impl Mul<f32> for Mat4 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self::from_cols(
            self.x_axis * rhs,
            self.y_axis * rhs,
            self.z_axis * rhs,
            self.w_axis * rhs,
        )
    }
}

impl Add for Mat4 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self::from_cols(
            self.x_axis + rhs.x_axis,
            self.y_axis + rhs.y_axis,
            self.z_axis + rhs.z_axis,
            self.w_axis + rhs.w_axis,
        )
    }
}

impl AddAssign for Mat4 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x_axis += rhs.x_axis;
        self.y_axis += rhs.y_axis;
        self.z_axis += rhs.z_axis;
        self.w_axis += rhs.w_axis;
    }
}

impl Sub for Mat4 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self::from_cols(
            self.x_axis - rhs.x_axis,
            self.y_axis - rhs.y_axis,
            self.z_axis - rhs.z_axis,
            self.w_axis - rhs.w_axis,
        )
    }
}

impl SubAssign for Mat4 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x_axis -= rhs.x_axis;
        self.y_axis -= rhs.y_axis;
        self.z_axis -= rhs.z_axis;
        self.w_axis -= rhs.w_axis;
    }
}
