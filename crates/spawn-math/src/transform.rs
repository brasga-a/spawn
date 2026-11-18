//! Local `f32` `Transform` (`translation`, `rotation`, `scale`; ADR-0010).
//!
//! `Transform` represents a local or camera-relative affine transform `T(translation) * R(rotation) * S(scale)`.
//! Absolute world positions belong in [`crate::WorldPos`] (`f64`, `SPN-INV-051`).

use crate::{Mat3, Mat4, Quat, Vec3};

const UNIFORM_SCALE_TOLERANCE: f32 = 1.0e-5;
const IDENTITY_ROTATION_TOLERANCE: f32 = 1.0e-5;

/// Local 3D translation-rotation-scale transform (`f32`).
///
/// Applies `scale` first, `rotation` second, and `translation` last:
/// `transform_point3(p) = rotation * (scale * p) + translation`.
///
/// [`Default`] returns [`Transform::IDENTITY`] (`translation = 0`, `rotation = IDENTITY`, `scale = 1`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Local or camera-relative translation (`f32`, meters).
    pub translation: Vec3,
    /// Unit quaternion orientation.
    pub rotation: Quat,
    /// Local scale along `(X, Y, Z)`.
    pub scale: Vec3,
}

impl Default for Transform {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// Identity transform (`translation = ZERO`, `rotation = IDENTITY`, `scale = ONE`).
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Creates a `Transform` from explicit `translation`, `rotation`, and `scale`.
    #[inline]
    pub const fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Pure translation transform (`rotation = IDENTITY`, `scale = ONE`).
    #[inline]
    pub const fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Pure rotation transform (`translation = ZERO`, `scale = ONE`).
    #[inline]
    pub const fn from_rotation(rotation: Quat) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Pure non-uniform scale transform (`translation = ZERO`, `rotation = IDENTITY`).
    #[inline]
    pub const fn from_scale(scale: Vec3) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale,
        }
    }

    /// Pure uniform scale transform (`scale = Vec3::splat(scale)`).
    #[inline]
    pub const fn from_uniform_scale(scale: f32) -> Self {
        Self::from_scale(Vec3::splat(scale))
    }

    /// Converts this transform to its homogeneous 4x4 matrix `T * R * S`.
    #[inline]
    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Attempts to decompose a shear-free affine [`Mat4`] into a `Transform`.
    ///
    /// Returns `None` if `mat` is non-finite, projective, singular, or sheared.
    #[inline]
    pub fn try_from_mat4(mat: Mat4) -> Option<Self> {
        let (scale, rotation, translation) = mat.try_to_scale_rotation_translation()?;
        Some(Self {
            translation,
            rotation,
            scale,
        })
    }

    /// Computes the normal-transform matrix `(R * S)^{-T}`, or `None` if `scale` is zero or non-finite.
    #[inline]
    pub fn normal_matrix(self) -> Option<Mat3> {
        Mat3::normal_matrix(self.scale, self.rotation)
    }

    /// Transforms a 3D point: `rotation * (scale * point) + translation`.
    #[inline]
    pub fn transform_point3(self, point: Vec3) -> Vec3 {
        self.rotation.mul_vec3(self.scale * point) + self.translation
    }

    /// Transforms a 3D vector (ignoring translation): `rotation * (scale * vector)`.
    #[inline]
    pub fn transform_vector3(self, vector: Vec3) -> Vec3 {
        self.rotation.mul_vec3(self.scale * vector)
    }

    /// Exact 4x4 composition `self.to_mat4() * child.to_mat4()`, supporting arbitrary
    /// non-uniform parent scales and rotated children (which may introduce shear).
    #[inline]
    pub fn mul_mat4(self, child: Self) -> Mat4 {
        self.to_mat4() * child.to_mat4()
    }

    /// Shear-free `Transform` composition (`self` parent followed by `child`) when both transforms
    /// are finite with valid rotations and either `self.scale` is uniform (`|sx - sy| <= 1e-5` and
    /// `|sy - sz| <= 1e-5`) or `child.rotation` is identity.
    ///
    /// Returns `None` if either transform is non-finite, has a zero/degenerate quaternion,
    /// overflows `f32`, or would introduce shear (`S_parent * R_child` cannot be represented as
    /// `R' * S'` when `self.scale` is non-uniform and `child.rotation` is non-identity; use
    /// [`Self::mul_mat4`] for general composition).
    #[inline]
    pub fn try_mul_transform(self, child: Self) -> Option<Self> {
        if !self.is_finite() || !child.is_finite() {
            return None;
        }
        let parent_rot = self.rotation.try_normalize()?;
        let child_rot = child.rotation.try_normalize()?;
        let is_uniform_parent_scale = (self.scale.x - self.scale.y).abs()
            <= UNIFORM_SCALE_TOLERANCE
            && (self.scale.y - self.scale.z).abs() <= UNIFORM_SCALE_TOLERANCE;
        let is_identity_child_rot =
            child_rot.angle_between(Quat::IDENTITY) <= IDENTITY_ROTATION_TOLERANCE;
        if !is_uniform_parent_scale && !is_identity_child_rot {
            return None;
        }
        let translation = parent_rot.mul_vec3(self.scale * child.translation) + self.translation;
        let rotation = (parent_rot * child_rot).try_normalize()?;
        let scale = self.scale * child.scale;
        let composed = Self {
            translation,
            rotation,
            scale,
        };
        composed.is_finite().then_some(composed)
    }

    /// Exact 4x4 inverse of `self.to_mat4()`, supporting arbitrary invertible non-uniform scales
    /// (which may introduce shear in `S^-1 * R^-1`).
    #[inline]
    pub fn inverse_mat4(self) -> Option<Mat4> {
        self.to_mat4().inverse()
    }

    /// Shear-free `Transform` inverse when `self` has non-zero finite scale and either
    /// uniform scale (`|sx - sy| <= 1e-5` and `|sy - sz| <= 1e-5`) or identity rotation.
    ///
    /// Returns `None` if `self` is singular, non-finite, or would introduce shear (`S^-1 * R^-1`
    /// cannot be represented as `R' * S'` when scale is non-uniform and rotation is non-trivial;
    /// use [`Self::inverse_mat4`] for that case).
    #[inline]
    pub fn try_inverse(self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        if self.scale.x == 0.0 || self.scale.y == 0.0 || self.scale.z == 0.0 {
            return None;
        }
        let inv_scale = Vec3::new(1.0 / self.scale.x, 1.0 / self.scale.y, 1.0 / self.scale.z);
        if !inv_scale.is_finite() {
            return None;
        }
        let rot = self.rotation.try_normalize()?;
        let is_uniform = (self.scale.x - self.scale.y).abs() <= UNIFORM_SCALE_TOLERANCE
            && (self.scale.y - self.scale.z).abs() <= UNIFORM_SCALE_TOLERANCE;
        let is_identity_rot = rot.angle_between(Quat::IDENTITY) <= IDENTITY_ROTATION_TOLERANCE;
        if !is_uniform && !is_identity_rot {
            return None;
        }
        let inv_rot = rot.conjugate();
        let inv_translation = -(inv_scale * inv_rot.mul_vec3(self.translation));
        inv_translation.is_finite().then_some(Self {
            translation: inv_translation,
            rotation: inv_rot,
            scale: inv_scale,
        })
    }

    /// Interpolates translation (`lerp`), rotation (`slerp` shortest arc), and scale (`lerp`).
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }

    /// Local `+X` right direction (`rotation * Vec3::X`).
    #[inline]
    pub fn right(self) -> Vec3 {
        self.rotation.right()
    }

    /// Local `+Y` up direction (`rotation * Vec3::Y`).
    #[inline]
    pub fn up(self) -> Vec3 {
        self.rotation.up()
    }

    /// Local `-Z` forward direction (`rotation * Vec3::NEG_Z`).
    #[inline]
    pub fn forward(self) -> Vec3 {
        self.rotation.forward()
    }

    /// Returns `true` if `translation`, `rotation`, and `scale` are all finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.translation.is_finite() && self.rotation.is_finite() && self.scale.is_finite()
    }
}
