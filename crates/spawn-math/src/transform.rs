//! Local `f32` `Transform` (`translation`, `rotation`, `scale`; ADR-0010).
//!
//! `Transform` represents a local or camera-relative affine transform `T(translation) * R(rotation) * S(scale)`.
//! Absolute world positions belong in [`crate::WorldPos`] (`f64`, `SPN-INV-051`).

use crate::{Mat3, Mat4, Quat, Vec3};

const ORTHOGONAL_AXIS_TOLERANCE: f32 = 1.0e-4;
const RECONSTRUCTION_TOLERANCE: f32 = 2.5e-4;
const TRIG_NOISE_COLUMN_REL_TOLERANCE: f32 = 1.0e-6;

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

    /// Shear-free `Transform` composition (`self` parent followed by `child`) whenever both
    /// transforms are finite with valid rotations and the exact product `self.to_mat4() * child.to_mat4()`
    /// can be represented as `T * R * S` without shear.
    ///
    /// Succeeds whenever `S_parent * R_child * S_child` preserves mutually orthogonal axes (including
    /// uniform-magnitude parent scales with reflections, identity child rotation, rotations around
    /// an axis whose perpendicular parent scale magnitudes match such as `(2, 1, 2)` with a Y-axis
    /// rotation, signed 90°/180° axis-permutation rotations, and zero scales that collapse sheared
    /// axes).
    ///
    /// Returns `None` if either transform is non-finite, has a zero/degenerate quaternion,
    /// overflows `f32`, or introduces shear (use [`Self::mul_mat4`] for general composition).
    #[inline]
    pub fn try_mul_transform(self, child: Self) -> Option<Self> {
        if !self.is_finite() || !child.is_finite() {
            return None;
        }
        let parent_rot = self.rotation.try_normalize()?;
        let child_rot = child.rotation.try_normalize()?;
        let translation = parent_rot.mul_vec3(self.scale * child.translation) + self.translation;
        if !translation.is_finite() {
            return None;
        }
        let r_c = Mat3::from_quat(child_rot);
        let mid = Mat3::from_cols(
            (self.scale * r_c.x_axis) * child.scale.x,
            (self.scale * r_c.y_axis) * child.scale.y,
            (self.scale * r_c.z_axis) * child.scale.z,
        );
        let noise_eligible = (
            col_is_parent_masked_trig_noise(self.scale, r_c.x_axis),
            col_is_parent_masked_trig_noise(self.scale, r_c.y_axis),
            col_is_parent_masked_trig_noise(self.scale, r_c.z_axis),
        );
        let (mid_rot, scale) = decompose_linear_trs(mid, child_rot, noise_eligible)?;
        let rotation = (parent_rot * mid_rot).try_normalize()?;
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

    /// Shear-free `Transform` inverse whenever `self` is finite with invertible non-zero scale,
    /// a valid rotation, and `S^-1 * R^-1` can be represented as `R' * S'` without shear.
    ///
    /// Succeeds for uniform-magnitude scales (including reflections), identity rotation, rotations
    /// around an axis whose perpendicular scale magnitudes match (such as `(2, 1, 2)` with a
    /// Y-axis rotation), and signed 90°/180° axis-permutation rotations.
    ///
    /// Returns `None` if `self` is singular, non-finite, overflows `f32`, or would introduce shear
    /// (use [`Self::inverse_mat4`] for general inversion).
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
        let inv_rot = rot.conjugate();
        let inv_translation = -(inv_scale * inv_rot.mul_vec3(self.translation));
        if !inv_translation.is_finite() {
            return None;
        }
        let r_inv = Mat3::from_quat(inv_rot);
        let linear_inv = Mat3::from_cols(
            inv_scale * r_inv.x_axis,
            inv_scale * r_inv.y_axis,
            inv_scale * r_inv.z_axis,
        );
        let (rotation, scale) = decompose_linear_trs(linear_inv, inv_rot, (false, false, false))?;
        let inv = Self {
            translation: inv_translation,
            rotation,
            scale,
        };
        inv.is_finite().then_some(inv)
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

/// Returns `true` if every surviving row of `rot_col` (where `parent_scale != 0`)
/// is pure `f32` trig rounding noise (`<= TRIG_NOISE_COLUMN_REL_TOLERANCE`).
#[inline]
fn col_is_parent_masked_trig_noise(parent_scale: Vec3, rot_col: Vec3) -> bool {
    (parent_scale.x == 0.0 || rot_col.x.abs() <= TRIG_NOISE_COLUMN_REL_TOLERANCE)
        && (parent_scale.y == 0.0 || rot_col.y.abs() <= TRIG_NOISE_COLUMN_REL_TOLERANCE)
        && (parent_scale.z == 0.0 || rot_col.z.abs() <= TRIG_NOISE_COLUMN_REL_TOLERANCE)
}

/// Decomposes a 3x3 linear matrix (`S * R` or `S_p * R_c * S_c`) into `(rotation, scale)`
/// iff its non-zero columns are mutually orthogonal and reconstruct the linear matrix
/// without shear.
#[inline]
fn decompose_linear_trs(
    linear: Mat3,
    zero_fallback_rot: Quat,
    noise_eligible: (bool, bool, bool),
) -> Option<(Quat, Vec3)> {
    if !linear.is_finite() {
        return None;
    }
    let c0 = linear.x_axis;
    let c1 = linear.y_axis;
    let c2 = linear.z_axis;
    let l0 = c0.length();
    let l1 = c1.length();
    let l2 = c2.length();
    if !l0.is_finite() || !l1.is_finite() || !l2.is_finite() {
        return None;
    }

    let u0 = if l0 > 0.0 {
        Some(c0.try_normalize()?)
    } else {
        None
    };
    let u1 = if l1 > 0.0 {
        Some(c1.try_normalize()?)
    } else {
        None
    };
    let u2 = if l2 > 0.0 {
        Some(c2.try_normalize()?)
    } else {
        None
    };

    if let Some(result) = decompose_normalized_cols(l0, l1, l2, u0, u1, u2, zero_fallback_rot) {
        return Some(result);
    }

    if !noise_eligible.0 && !noise_eligible.1 && !noise_eligible.2 {
        return None;
    }

    // `f32` trig rounding in 90° rotations (e.g. `cos(FRAC_PI_2) ≈ -4.37e-8`) can leave
    // residual ~1e-7 noise on an axis-collapsed column when an input scale has a zero.
    // Only trim columns whose surviving rotation entries are themselves `<= 1e-6` AND whose
    // scaled norm is `<= 1e-6 * max_len` so true sheared/oblique columns are never dropped.
    let max_len = l0.max(l1).max(l2);
    let noise_floor = max_len * TRIG_NOISE_COLUMN_REL_TOLERANCE;
    if noise_floor == 0.0 {
        return None;
    }
    let trim0 = noise_eligible.0 && l0 > 0.0 && l0 <= noise_floor;
    let trim1 = noise_eligible.1 && l1 > 0.0 && l1 <= noise_floor;
    let trim2 = noise_eligible.2 && l2 > 0.0 && l2 <= noise_floor;
    if !trim0 && !trim1 && !trim2 {
        return None;
    }
    let (l0_t, u0_t) = if trim0 { (0.0, None) } else { (l0, u0) };
    let (l1_t, u1_t) = if trim1 { (0.0, None) } else { (l1, u1) };
    let (l2_t, u2_t) = if trim2 { (0.0, None) } else { (l2, u2) };
    decompose_normalized_cols(l0_t, l1_t, l2_t, u0_t, u1_t, u2_t, zero_fallback_rot)
}

#[inline]
fn decompose_normalized_cols(
    l0: f32,
    l1: f32,
    l2: f32,
    u0: Option<Vec3>,
    u1: Option<Vec3>,
    u2: Option<Vec3>,
    zero_fallback_rot: Quat,
) -> Option<(Quat, Vec3)> {
    match (u0, u1, u2) {
        (None, None, None) => Some((zero_fallback_rot, Vec3::ZERO)),
        (Some(u), None, None) => {
            verify_single_axis_trs(Vec3::X, u, Vec3::new(l0, 0.0, 0.0), |r| r.x_axis)
        }
        (None, Some(u), None) => {
            verify_single_axis_trs(Vec3::Y, u, Vec3::new(0.0, l1, 0.0), |r| r.y_axis)
        }
        (None, None, Some(u)) => {
            verify_single_axis_trs(Vec3::Z, u, Vec3::new(0.0, 0.0, l2), |r| r.z_axis)
        }
        (Some(a0), Some(a1), None) => {
            let a2 = complete_orthogonal_axis(a0, a1)?;
            let rotation = Quat::try_from_mat3(Mat3::from_cols(a0, a1, a2))?;
            Some((rotation, Vec3::new(l0, l1, 0.0)))
        }
        (None, Some(a1), Some(a2)) => {
            let a0 = complete_orthogonal_axis(a1, a2)?;
            let rotation = Quat::try_from_mat3(Mat3::from_cols(a0, a1, a2))?;
            Some((rotation, Vec3::new(0.0, l1, l2)))
        }
        (Some(a0), None, Some(a2)) => {
            let a1 = complete_orthogonal_axis(a2, a0)?;
            let rotation = Quat::try_from_mat3(Mat3::from_cols(a0, a1, a2))?;
            Some((rotation, Vec3::new(l0, 0.0, l2)))
        }
        (Some(a0), Some(a1), Some(a2)) => {
            let unit_det = Mat3::from_cols(a0, a1, a2).determinant();
            if unit_det == 0.0 || !unit_det.is_finite() {
                return None;
            }
            let sign = unit_det.signum();
            let scale = Vec3::new(l0 * sign, l1, l2);
            let rot_mat = Mat3::from_cols(a0 * sign, a1, a2);
            let rotation = Quat::try_from_mat3(rot_mat)?;
            Some((rotation, scale))
        }
    }
}

#[inline]
fn verify_single_axis_trs(
    basis_axis: Vec3,
    target_unit: Vec3,
    scale: Vec3,
    extract_col: impl FnOnce(Mat3) -> Vec3,
) -> Option<(Quat, Vec3)> {
    let rotation = Quat::try_from_rotation_arc(basis_axis, target_unit)?;
    let col = extract_col(Mat3::from_quat(rotation));
    col.abs_diff_eq(target_unit, RECONSTRUCTION_TOLERANCE)
        .then_some((rotation, scale))
}

#[inline]
fn complete_orthogonal_axis(a: Vec3, b: Vec3) -> Option<Vec3> {
    if a.dot(b).abs() > ORTHOGONAL_AXIS_TOLERANCE {
        return None;
    }
    a.cross(b).try_normalize()
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    use super::*;

    fn assert_mul_matches_mat4(parent: Transform, child: Transform) {
        let composed = parent
            .try_mul_transform(child)
            .unwrap_or_else(|| panic!("expected valid TRS composition for {parent:?} * {child:?}"));
        assert!(composed.rotation.is_normalized());
        let expected = parent.mul_mat4(child);
        assert!(
            composed.to_mat4().abs_diff_eq(&expected, 1.0e-4),
            "composed.to_mat4() mismatch:\n  got:      {:?}\n  expected: {:?}",
            composed.to_mat4(),
            expected
        );
        let probe = Vec3::new(1.5, -2.25, 3.75);
        assert!(composed.transform_point3(probe).abs_diff_eq(
            parent.transform_point3(child.transform_point3(probe)),
            1.0e-3
        ));
    }

    fn assert_inverse_roundtrip(t: Transform) {
        let inv = t
            .try_inverse()
            .unwrap_or_else(|| panic!("expected valid TRS inverse for {t:?}"));
        assert!(inv.rotation.is_normalized());
        assert!(
            (t.to_mat4() * inv.to_mat4()).abs_diff_eq(&Mat4::IDENTITY, 1.0e-4),
            "t * inv != I: {:?}",
            t.to_mat4() * inv.to_mat4()
        );
        assert!(
            (inv.to_mat4() * t.to_mat4()).abs_diff_eq(&Mat4::IDENTITY, 1.0e-4),
            "inv * t != I: {:?}",
            inv.to_mat4() * t.to_mat4()
        );
        let prod_right = t
            .try_mul_transform(inv)
            .expect("t * inv must compose back to IDENTITY");
        assert!(
            prod_right.to_mat4().abs_diff_eq(&Mat4::IDENTITY, 1.0e-4),
            "t.try_mul_transform(inv) != I: {prod_right:?}"
        );
        let prod_left = inv
            .try_mul_transform(t)
            .expect("inv * t must compose back to IDENTITY");
        assert!(
            prod_left.to_mat4().abs_diff_eq(&Mat4::IDENTITY, 1.0e-4),
            "inv.try_mul_transform(t) != I: {prod_left:?}"
        );
    }

    #[test]
    fn compatible_non_uniform_scale_and_axis_rotations_compose_and_invert() {
        // (2, 1, 2) with Y-axis rotation commutes and stays shear-free.
        for angle in [0.15_f32, 0.75, -1.2, FRAC_PI_4, FRAC_PI_2, PI] {
            let parent = Transform::new(
                Vec3::new(3.0, -4.0, 5.0),
                Quat::from_yaw_pitch_roll(0.4, -0.3, 0.2),
                Vec3::new(2.0, 1.0, 2.0),
            );
            let child = Transform::new(
                Vec3::new(-1.5, 2.0, 0.5),
                Quat::from_rotation_y(angle),
                Vec3::new(1.5, 0.75, 3.0),
            );
            assert_mul_matches_mat4(parent, child);

            let single = Transform::new(
                Vec3::new(2.0, -1.0, 4.0),
                Quat::from_rotation_y(angle),
                Vec3::new(2.0, 1.0, 2.0),
            );
            assert_inverse_roundtrip(single);
        }

        // (1, 3, 3) with X-axis rotation and (2.5, 2.5, 0.5) with Z-axis rotation.
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(1.0, 3.0, 3.0)),
            Transform::from_rotation(Quat::from_rotation_x(0.85)),
        );
        assert_inverse_roundtrip(Transform::new(
            Vec3::new(-2.0, 5.0, 1.0),
            Quat::from_rotation_x(-0.65),
            Vec3::new(1.0, 3.0, 3.0),
        ));
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(2.5, 2.5, 0.5)),
            Transform::from_rotation(Quat::from_rotation_z(-1.1)),
        );
        assert_inverse_roundtrip(Transform::new(
            Vec3::new(1.0, -3.0, 2.0),
            Quat::from_rotation_z(0.55),
            Vec3::new(2.5, 2.5, 0.5),
        ));
    }

    #[test]
    fn ninety_degree_axis_permutations_with_distinct_non_uniform_scales_succeed() {
        let permutation_rotations = [
            Quat::IDENTITY,
            Quat::from_rotation_x(FRAC_PI_2),
            Quat::from_rotation_x(-FRAC_PI_2),
            Quat::from_rotation_x(PI),
            Quat::from_rotation_y(FRAC_PI_2),
            Quat::from_rotation_y(-FRAC_PI_2),
            Quat::from_rotation_y(PI),
            Quat::from_rotation_z(FRAC_PI_2),
            Quat::from_rotation_z(-FRAC_PI_2),
            Quat::from_rotation_z(PI),
            Quat::from_rotation_y(FRAC_PI_2) * Quat::from_rotation_x(FRAC_PI_2),
            Quat::from_rotation_z(FRAC_PI_2) * Quat::from_rotation_y(FRAC_PI_2),
            Quat::from_rotation_x(FRAC_PI_2) * Quat::from_rotation_z(FRAC_PI_2),
        ];

        for scale in [
            Vec3::new(2.0, 1.0, 0.5),
            Vec3::new(-2.0, 1.0, 0.5),
            Vec3::new(-2.0, -1.0, 0.5),
            Vec3::new(-2.0, -1.0, -0.5),
        ] {
            let parent = Transform::new(
                Vec3::new(1.0, 2.0, -3.0),
                Quat::from_yaw_pitch(0.5, -0.2),
                scale,
            );
            for rot in permutation_rotations {
                let child =
                    Transform::new(Vec3::new(-2.0, 0.5, 1.25), rot, Vec3::new(1.5, 0.75, 2.25));
                assert_mul_matches_mat4(parent, child);

                let single = Transform::new(Vec3::new(4.0, -1.0, 2.5), rot, scale);
                assert_inverse_roundtrip(single);
            }
        }
    }

    #[test]
    fn negative_uniform_and_compatible_reflection_scales_succeed() {
        let child = Transform::new(
            Vec3::new(1.0, -2.0, 3.0),
            Quat::from_yaw_pitch_roll(0.6, -0.45, 0.3),
            Vec3::new(1.5, 0.5, 2.0),
        );
        for neg_uniform in [
            Vec3::new(-2.0, 2.0, 2.0),
            Vec3::new(2.0, -2.0, 2.0),
            Vec3::new(2.0, 2.0, -2.0),
            Vec3::new(-2.0, -2.0, 2.0),
            Vec3::new(-2.0, -2.0, -2.0),
        ] {
            let parent = Transform::new(
                Vec3::new(-3.0, 4.0, 1.0),
                Quat::from_yaw_pitch(0.3, 0.2),
                neg_uniform,
            );
            assert_mul_matches_mat4(parent, child);

            let single = Transform::new(
                Vec3::new(2.0, -1.5, 0.5),
                Quat::from_yaw_pitch_roll(0.6, -0.45, 0.3),
                neg_uniform,
            );
            assert_inverse_roundtrip(single);
        }

        // Non-uniform reflection (-2, 1, 2) + Y rotation.
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(-2.0, 1.0, 2.0)),
            Transform::from_rotation(Quat::from_rotation_y(0.7)),
        );
        assert_inverse_roundtrip(Transform::new(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(0.7),
            Vec3::new(-2.0, 1.0, 2.0),
        ));
    }

    #[test]
    fn true_shear_and_small_non_uniform_scales_are_rejected() {
        // (2, 1, 2) with X or Z rotation introduces shear.
        let p_212 = Transform::from_scale(Vec3::new(2.0, 1.0, 2.0));
        assert_eq!(
            p_212.try_mul_transform(Transform::from_rotation(Quat::from_rotation_x(0.4))),
            None
        );
        assert_eq!(
            p_212.try_mul_transform(Transform::from_rotation(Quat::from_rotation_z(0.4))),
            None
        );
        assert_eq!(
            Transform::new(
                Vec3::ZERO,
                Quat::from_rotation_x(0.4),
                Vec3::new(2.0, 1.0, 2.0)
            )
            .try_inverse(),
            None
        );

        // Small-magnitude 10x non-uniform scale (1e-6, 1e-6, 1e-5) with 45° X rotation
        // must NOT be misclassified as uniform scale.
        let tiny_non_uniform = Vec3::new(1.0e-6, 1.0e-6, 1.0e-5);
        assert_eq!(
            Transform::from_scale(tiny_non_uniform)
                .try_mul_transform(Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_4))),
            None
        );
        assert_eq!(
            Transform::new(
                Vec3::ZERO,
                Quat::from_rotation_x(FRAC_PI_4),
                tiny_non_uniform
            )
            .try_inverse(),
            None
        );

        // Subtle 0.1% non-uniformity with 45° rotation exceeds orthogonality tolerance.
        assert_eq!(
            Transform::from_scale(Vec3::new(1.0, 1.0, 1.001))
                .try_mul_transform(Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_4))),
            None
        );
    }

    #[test]
    fn zero_scale_extreme_scale_and_degenerate_inputs_never_panic() {
        let min_subnormal = f32::from_bits(1);

        // Zero scales in try_inverse are always singular -> None.
        for bad_scale in [
            Vec3::ZERO,
            Vec3::new(-0.0, 1.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(min_subnormal, 1.0, 1.0),
            Vec3::NAN,
            Vec3::INFINITY,
        ] {
            assert_eq!(Transform::from_scale(bad_scale).try_inverse(), None);
        }

        // Large and small scales where Mat4::det would overflow/underflow still invert cleanly!
        for extreme_scale in [Vec3::splat(1.0e20), Vec3::splat(1.0e-20)] {
            let t = Transform::new(
                Vec3::new(1.0, -2.0, 3.0),
                Quat::from_yaw_pitch(0.5, -0.3),
                extreme_scale,
            );
            let inv = t
                .try_inverse()
                .expect("extreme scale inverse should not overflow det");
            let round = t.try_mul_transform(inv).expect("t * inv must compose");
            assert!(round.to_mat4().abs_diff_eq(&Mat4::IDENTITY, 1.0e-4));
        }

        // Zero-scale compositions: shear-free ones succeed, sheared ones return None.
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::ZERO),
            Transform::from_rotation(Quat::from_yaw_pitch(0.5, 0.4)),
        );
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(2.0, 0.0, 2.0)),
            Transform::from_rotation(Quat::from_rotation_y(0.6)),
        );
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(0.0, 1.0, 1.0)),
            Transform::from_rotation(Quat::from_rotation_z(FRAC_PI_2)),
        );
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(2.0, 1.0, 0.5)),
            Transform::new(
                Vec3::ZERO,
                Quat::from_rotation_z(FRAC_PI_4),
                Vec3::new(0.0, 1.0, 1.0),
            ),
        );
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(1.0, 0.0, 0.0)),
            Transform::new(
                Vec3::ZERO,
                Quat::from_rotation_y(FRAC_PI_4),
                Vec3::new(1.0, 0.0, 0.0),
            ),
        );
        assert_eq!(
            Transform::from_scale(Vec3::new(0.0, 1.0, 1.0))
                .try_mul_transform(Transform::from_rotation(Quat::from_rotation_z(FRAC_PI_4))),
            None
        );

        // Zero-scale with large orthogonal axis: oblique rotation is sheared -> None,
        // while 90° rotation with trig-noise collapsed column still succeeds cleanly.
        assert_eq!(
            Transform::from_scale(Vec3::new(1.0, 1.0e7, 0.0))
                .try_mul_transform(Transform::from_rotation(Quat::from_rotation_y(0.3))),
            None
        );
        assert_eq!(
            Transform::from_scale(Vec3::new(1.0, 1.0, 0.0)).try_mul_transform(Transform::new(
                Vec3::ZERO,
                Quat::from_rotation_y(0.3),
                Vec3::new(1.0e-7, 1.0, 1.0),
            )),
            None
        );
        assert_mul_matches_mat4(
            Transform::from_scale(Vec3::new(1.0, 1.0e7, 0.0)),
            Transform::from_rotation(Quat::from_rotation_y(FRAC_PI_2)),
        );
        assert_eq!(
            Transform::from_scale(Vec3::new(1.0e7, 1.0, 0.0)).try_mul_transform(Transform::new(
                Vec3::ZERO,
                Quat::from_rotation_y(FRAC_PI_2),
                Vec3::new(1.0, 1.0, 1.0e-7),
            )),
            None
        );

        // Overflow and non-finite inputs return None without panicking.
        assert_eq!(
            Transform::from_scale(Vec3::splat(1.0e25))
                .try_mul_transform(Transform::from_scale(Vec3::splat(1.0e25))),
            None
        );
        assert_eq!(
            Transform::from_translation(Vec3::NAN).try_mul_transform(Transform::IDENTITY),
            None
        );
        assert_eq!(
            Transform::IDENTITY.try_mul_transform(Transform::from_translation(Vec3::INFINITY)),
            None
        );
        assert_eq!(Transform::from_translation(Vec3::NAN).try_inverse(), None);
    }
}
