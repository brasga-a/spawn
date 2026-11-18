//! `F3D-T01` (`F3D-FR-01`, `F3D-INV-05, 15`): headless `spawn-math` unit tests,
//! `proptest` algebraic laws, `glam` test-oracle comparisons (`Vec2/3/4`, `Mat3/4`,
//! `Quat`, `Transform`, `Aabb`, `Ray`, `Plane`, `Sphere`), and adversarial degenerate
//! torture cases (`NaN`, `±Inf`, subnormals, zero-length vectors, singular matrices,
//! `180°` antipodal quaternions).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use proptest::prelude::*;
use spawn_math::{
    Aabb, Mat3, Mat4, MathErrorKind, Plane, Quat, Ray, Sphere, Transform, Vec2, Vec3, Vec4,
    degrees_from_radians_f32, degrees_from_radians_f64, radians_from_degrees_f32,
    radians_from_degrees_f64,
};

fn to_glam_vec2(v: Vec2) -> glam::Vec2 {
    glam::Vec2::new(v.x, v.y)
}

fn to_glam_vec3(v: Vec3) -> glam::Vec3 {
    glam::Vec3::new(v.x, v.y, v.z)
}

fn to_glam_vec4(v: Vec4) -> glam::Vec4 {
    glam::Vec4::new(v.x, v.y, v.z, v.w)
}

fn to_glam_quat(q: Quat) -> glam::Quat {
    glam::Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

fn to_glam_mat3(m: Mat3) -> glam::Mat3 {
    glam::Mat3::from_cols_array(&m.to_cols_array())
}

fn from_glam_quat(q: glam::Quat) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

fn from_glam_mat3(m: glam::Mat3) -> Mat3 {
    Mat3::from_cols_array(&m.to_cols_array())
}

fn from_glam_mat4(m: glam::Mat4) -> Mat4 {
    Mat4::from_cols_array(&m.to_cols_array())
}

fn proptest_cfg() -> ProptestConfig {
    ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

#[test]
fn f3d_t01_right_handed_axes_and_euler_identity_conventions() {
    // Right-handed cross products: X × Y = +Z, Y × Z = +X, Z × X = +Y.
    assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
    assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
    assert_eq!(Vec3::Z.cross(Vec3::X), Vec3::Y);
    assert_eq!(Vec3::RIGHT.cross(Vec3::UP), -Vec3::FORWARD);

    // Canonical identity orientation looks down -Z with +X right and +Y up.
    let id = Quat::from_yaw_pitch(0.0, 0.0);
    assert!(id.abs_diff_eq(Quat::IDENTITY, 1.0e-6));
    assert!(id.right().abs_diff_eq(Vec3::X, 1.0e-6));
    assert!(id.up().abs_diff_eq(Vec3::Y, 1.0e-6));
    assert!(id.forward().abs_diff_eq(Vec3::NEG_Z, 1.0e-6));

    // Positive pitch (+45° around +X) pitches camera look UP (+Y, -Z).
    let pitch_up = Quat::from_yaw_pitch(0.0, FRAC_PI_4);
    let fwd_up = pitch_up.forward();
    assert!(fwd_up.y > 0.7 && fwd_up.z < -0.7, "fwd_up = {fwd_up:?}");

    // Positive yaw (+90° around +Y) yaws camera look LEFT (-X) with horizon level (right.y == 0).
    let yaw_left = Quat::from_yaw_pitch(FRAC_PI_2, 0.0);
    assert!(yaw_left.forward().abs_diff_eq(Vec3::NEG_X, 1.0e-6));
    assert!(yaw_left.right().abs_diff_eq(Vec3::NEG_Z, 1.0e-6));
    assert!(yaw_left.up().abs_diff_eq(Vec3::Y, 1.0e-6));

    // Boundary degree <-> radian conversion helpers (SPN-INV-054).
    assert!((radians_from_degrees_f32(90.0) - FRAC_PI_2).abs() <= 1.0e-6);
    assert!((degrees_from_radians_f32(PI) - 180.0).abs() <= 1.0e-5);
    assert!((radians_from_degrees_f64(180.0) - std::f64::consts::PI).abs() <= 1.0e-15);
    assert!((degrees_from_radians_f64(std::f64::consts::FRAC_PI_2) - 90.0).abs() <= 1.0e-13);
}

#[test]
fn f3d_t01_column_major_layout_and_byte_serialization_roundtrip() {
    assert_eq!(std::mem::size_of::<Vec2>(), 8);
    assert_eq!(std::mem::size_of::<Vec3>(), 12);
    assert_eq!(std::mem::size_of::<Vec4>(), 16);
    assert_eq!(std::mem::size_of::<Quat>(), 16);
    assert_eq!(std::mem::size_of::<Mat3>(), 36);
    assert_eq!(std::mem::size_of::<Mat4>(), 64);

    // Translation lives in column 3 (`w_axis`), flat indices 12..15 (`cols[3][0..3]`).
    let t = Vec3::new(3.0, -7.5, 12.25);
    let m = Mat4::from_translation(t);
    let cols_2d = m.to_cols_array_2d();
    assert_eq!(cols_2d[3], [3.0, -7.5, 12.25, 1.0]);
    let flat = m.to_cols_array();
    assert_eq!(&flat[12..16], &[3.0, -7.5, 12.25, 1.0]);
    assert_eq!(
        m * Vec4::new(1.0, 2.0, 3.0, 1.0),
        Vec4::new(4.0, -5.5, 15.25, 1.0)
    );
    assert_eq!(Mat4::from_le_bytes(m.to_le_bytes()), m);

    let v2 = Vec2::new(-1.5, 4.25);
    assert_eq!(Vec2::from_le_bytes(v2.to_le_bytes()), v2);
    let v3 = Vec3::new(1.0, -2.0, 3.5);
    assert_eq!(Vec3::from_le_bytes(v3.to_le_bytes()), v3);
    let v4 = Vec4::new(0.25, -0.5, 0.75, 1.0);
    assert_eq!(Vec4::from_le_bytes(v4.to_le_bytes()), v4);
    let q = Quat::from_yaw_pitch_roll(0.4, -0.3, 0.2);
    assert_eq!(Quat::from_le_bytes(q.to_le_bytes()), q);
}

#[test]
fn f3d_t01_adversarial_nan_inf_subnormal_zero_singular_and_180_quat_torture() {
    let min_subnormal = f32::from_bits(1); // ~1.4e-45, positive subnormal
    assert!(min_subnormal > 0.0 && min_subnormal < f32::MIN_POSITIVE);

    // 1. Zero, negative-zero, NaN, and ±Inf vector normalization never panics.
    for bad in [
        Vec3::ZERO,
        Vec3::splat(-0.0),
        Vec3::NAN,
        Vec3::INFINITY,
        Vec3::NEG_INFINITY,
        Vec3::new(f32::NAN, 1.0, 0.0),
        Vec3::new(f32::INFINITY, 1.0, 0.0),
    ] {
        assert_eq!(bad.try_normalize(), None, "expected None for {bad:?}");
        assert_eq!(bad.normalize_or_zero(), Vec3::ZERO);
        assert!(!bad.is_normalized());
    }
    assert_eq!(Vec2::ZERO.try_normalize(), None);
    assert_eq!(Vec4::ZERO.try_normalize(), None);
    assert_eq!(Vec2::NAN.normalize_or_zero(), Vec2::ZERO);
    assert_eq!(Vec4::INFINITY.normalize_or_zero(), Vec4::ZERO);

    // Out-of-bounds component indexing returns None instead of panicking.
    assert_eq!(Vec2::ONE.get(2), None);
    assert_eq!(Vec3::ONE.get(3), None);
    assert_eq!(Vec4::ONE.get(4), None);
    assert_eq!(Mat3::IDENTITY.col(3), None);
    assert_eq!(Mat4::IDENTITY.row(4), None);

    // Inverted clamp bounds (`min > max`) return NaN instead of panicking!
    let inverted = Vec3::ONE.clamp(Vec3::splat(5.0), Vec3::splat(-5.0));
    assert!(inverted.is_nan());

    // 2. Subnormal (`1.4e-45`, `1e-25`) and near-f32::MAX (`1e38`) vectors normalize cleanly!
    for extreme in [
        Vec3::new(min_subnormal, 0.0, 0.0),
        Vec3::new(0.0, -1.0e-25, 0.0),
        Vec3::new(1.0e-30, 1.0e-30, 0.0),
        Vec3::new(1.0e38, -1.0e38, 0.0),
    ] {
        let unit = extreme
            .try_normalize()
            .unwrap_or_else(|| panic!("expected extreme finite vector {extreme:?} to normalize"));
        assert!(unit.is_normalized(), "unit = {unit:?} for {extreme:?}");
    }
    assert!(
        Vec2::new(min_subnormal, 0.0)
            .try_normalize()
            .unwrap()
            .is_normalized()
    );
    assert!(
        Vec4::new(0.0, 0.0, 0.0, 1.0e38)
            .try_normalize()
            .unwrap()
            .is_normalized()
    );

    // 3. Singular, near-singular, NaN, and ±Inf matrices return None on inverse().
    let singular_m3 = [
        Mat3::ZERO,
        Mat3::NAN,
        Mat3::from_cols(Vec3::X, Vec3::X, Vec3::Z),
        Mat3::from_scale(Vec3::new(1.0, 0.0, 1.0)),
        Mat3::from_scale(Vec3::splat(min_subnormal)),
        Mat3::from_scale(Vec3::splat(f32::INFINITY)),
    ];
    for m in singular_m3 {
        assert_eq!(m.inverse(), None, "Mat3::inverse must be None for {m:?}");
    }

    let singular_m4 = [
        Mat4::ZERO,
        Mat4::NAN,
        Mat4::from_cols(Vec4::X, Vec4::X, Vec4::Z, Vec4::W),
        Mat4::from_scale(Vec3::new(0.0, 2.0, 3.0)),
        Mat4::from_scale(Vec3::splat(min_subnormal)),
        Mat4::from_scale(Vec3::splat(f32::INFINITY)),
    ];
    for m in singular_m4 {
        assert_eq!(m.inverse(), None, "Mat4::inverse must be None for {m:?}");
    }

    // 4. 180° (`π` rad) rotation & antipodal double-cover quaternion torture cases!
    for axis in [
        Vec3::X,
        Vec3::Y,
        Vec3::Z,
        Vec3::new(1.0, 1.0, 1.0).try_normalize().unwrap(),
        Vec3::new(-2.0, 0.5, 3.0).try_normalize().unwrap(),
    ] {
        let q180 = Quat::try_from_axis_angle(axis, PI).unwrap();
        assert!(q180.is_normalized());
        let m3 = Mat3::from_quat(q180);
        // Shepperd's method recovers the exact 180° rotation even when trace == -1.0!
        let recovered = Quat::try_from_mat3(m3).unwrap();
        assert!(
            q180.angle_between(recovered) <= 1.0e-5,
            "180° Shepperd round-trip failed for axis {axis:?}: {q180:?} vs {recovered:?}"
        );
    }

    // Antiparallel 180° `try_from_rotation_arc(u, -u)` produces a valid 180° unit quaternion!
    for u in [Vec3::X, Vec3::Y, Vec3::NEG_Z, Vec3::new(0.6, -0.8, 0.0)] {
        let arc = Quat::try_from_rotation_arc(u, -u).unwrap();
        assert!(arc.is_normalized());
        let rotated = arc * u.try_normalize().unwrap();
        assert!(
            rotated.abs_diff_eq(-u.try_normalize().unwrap(), 1.0e-5),
            "180° arc failed for {u:?}: got {rotated:?}"
        );
    }

    // Antipodal double-cover `q.slerp(-q, t)` stays stationary on `q` without dividing by sin(π) == 0!
    let q = Quat::from_yaw_pitch(0.7, -0.4);
    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let s = q.slerp(-q, t);
        assert!(s.is_normalized());
        assert!(q.angle_between(s) <= 1.0e-5);
    }

    // Degenerate zero/NaN quaternions never panic.
    let zero_q = Quat::from_xyzw(0.0, 0.0, 0.0, 0.0);
    assert_eq!(zero_q.try_normalize(), None);
    assert_eq!(zero_q.normalize_or_identity(), Quat::IDENTITY);
    assert_eq!(zero_q.try_inverse(), None);
    assert_eq!(zero_q.to_axis_angle(), (Vec3::X, 0.0));
    assert_eq!(Quat::NAN.slerp(Quat::IDENTITY, 0.5), Quat::IDENTITY);
    assert!(
        Quat::from_xyzw(min_subnormal, 0.0, 0.0, 0.0)
            .try_normalize()
            .unwrap()
            .is_normalized()
    );
}

#[test]
fn f3d_t01_geometry_primitives_and_located_errors() {
    // Aabb validation, point containment, union, and affine transform vs 8 corners.
    let err = Aabb::try_new(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)).unwrap_err();
    assert_eq!(err.kind(), MathErrorKind::InvalidBounds);
    assert_eq!(err.code(), "MATH-0005");
    assert!(
        err.location()
            .file()
            .ends_with("f3d_t01_primitives_and_oracle.rs")
    );

    let aabb = Aabb::try_new(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(2.0, 4.0, 1.0)).unwrap();
    assert!(aabb.contains_point(Vec3::ZERO));
    assert!(!aabb.contains_point(Vec3::new(3.0, 0.0, 0.0)));
    assert_eq!(
        aabb.closest_point(Vec3::new(10.0, -10.0, 0.5)),
        Vec3::new(2.0, -2.0, 0.5)
    );

    let trs = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 0.5, 1.5),
        Quat::from_yaw_pitch_roll(0.6, -0.3, 0.25),
        Vec3::new(4.0, -3.0, 8.0),
    );
    let fast_aabb = aabb.transformed_by_affine_mat4(trs);
    let corners = [
        Vec3::new(aabb.min.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.max.z),
    ];
    let transformed_corners: Vec<Vec3> = corners.iter().map(|&c| trs.transform_point3(c)).collect();
    let brute_aabb = Aabb::from_points(&transformed_corners).unwrap();
    assert!(fast_aabb.min.abs_diff_eq(brute_aabb.min, 1.0e-5));
    assert!(fast_aabb.max.abs_diff_eq(brute_aabb.max, 1.0e-5));

    // Sphere validation & containment.
    assert_eq!(
        Sphere::try_new(Vec3::ZERO, -0.1).unwrap_err().kind(),
        MathErrorKind::InvalidBounds
    );
    let sphere = Sphere::try_new(Vec3::new(0.0, 0.0, -5.0), 2.0).unwrap();
    assert!(sphere.contains_point(Vec3::new(0.0, 1.5, -5.0)));
    assert!(!sphere.contains_point(Vec3::ZERO));

    // Plane validation, CCW construction, signed distance, projection.
    assert_eq!(
        Plane::try_from_point_normal(Vec3::ZERO, Vec3::ZERO)
            .unwrap_err()
            .kind(),
        MathErrorKind::DegenerateDirection
    );
    let plane = Plane::try_from_points_ccw(
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::new(0.0, 2.0, 1.0),
        Vec3::new(1.0, 2.0, 0.0),
    )
    .unwrap();
    assert!(plane.normal.abs_diff_eq(Vec3::Y, 1.0e-6));
    assert!((plane.signed_distance(Vec3::new(3.0, 5.0, -4.0)) - 3.0).abs() <= 1.0e-6);
    assert!(
        plane
            .project_point(Vec3::new(3.0, 5.0, -4.0))
            .abs_diff_eq(Vec3::new(3.0, 2.0, -4.0), 1.0e-6)
    );

    // Ray intersections (plane, sphere, AABB, including parallel face-grazing direction.x == 0!).
    assert_eq!(
        Ray::try_new(Vec3::ZERO, Vec3::ZERO).unwrap_err().kind(),
        MathErrorKind::DegenerateDirection
    );
    let ray = Ray::try_new(Vec3::ZERO, Vec3::FORWARD).unwrap();
    let t_sphere = ray.intersect_sphere(sphere).unwrap();
    assert!((t_sphere - 3.0).abs() <= 1.0e-5);

    let grazing_ray = Ray::try_new(Vec3::new(-1.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let t_grazing = grazing_ray.intersect_aabb(aabb).unwrap();
    assert!(
        (t_grazing - 4.0).abs() <= 1.0e-5,
        "ray on AABB x == min.x face with direction.x == 0 must not NaN: got {t_grazing}"
    );

    // Non-uniform scaled + rotated Transform rejects shear-inducing TRS inverse, while inverse_mat4 succeeds!
    let sheared = Transform::new(
        Vec3::new(1.0, 2.0, 3.0),
        Quat::from_yaw_pitch(0.5, 0.3),
        Vec3::new(2.0, 1.0, 0.5),
    );
    assert_eq!(sheared.try_inverse(), None);
    let inv_m4 = sheared.inverse_mat4().unwrap();
    assert!((sheared.to_mat4() * inv_m4).abs_diff_eq(&Mat4::IDENTITY, 1.0e-5));
}

#[test]
fn f3d_t01_trs_shear_rejection_composition_and_negative_scale_roundtrip() {
    // 1. Sheared Mat3 / Mat4 torture cases with det == 1 (upper triangular, lower triangular,
    //    subtle off-diagonal shear, and symmetric hyperbolic squeeze where off-diagonal skew is zero).
    let hyp_a = 0.35_f32;
    let sheared_m3_cases = [
        // Upper triangular shear (`det = 1`): columns `(1, 0, 0)`, `(1, 1, 0)`, `(0, 0, 1)`.
        Mat3::from_cols(Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Z),
        // Upper triangular 3D shear (`det = 1`).
        Mat3::from_cols(Vec3::X, Vec3::new(0.4, 1.0, 0.0), Vec3::new(-0.3, 0.7, 1.0)),
        // Lower triangular shear (`det = 1`).
        Mat3::from_cols(Vec3::new(1.0, 1.0, 0.0), Vec3::Y, Vec3::Z),
        Mat3::from_cols(
            Vec3::new(1.0, -0.5, 0.25),
            Vec3::new(0.0, 1.0, 0.6),
            Vec3::Z,
        ),
        // Subtle shear just above SO(3) orthogonality tolerance (`det = 1`).
        Mat3::from_cols(Vec3::X, Vec3::new(1.0e-3, 1.0, 0.0), Vec3::Z),
        // Symmetric hyperbolic squeeze (`det = cosh^2(a) - sinh^2(a) == 1`).
        Mat3::from_cols(
            Vec3::new(hyp_a.cosh(), hyp_a.sinh(), 0.0),
            Vec3::new(hyp_a.sinh(), hyp_a.cosh(), 0.0),
            Vec3::Z,
        ),
    ];
    for m3 in sheared_m3_cases {
        assert!(
            (m3.determinant() - 1.0).abs() <= 1.0e-5,
            "expected det ≈ 1 for {m3:?}"
        );
        assert_eq!(
            Quat::try_from_mat3(m3),
            None,
            "Quat::try_from_mat3 must reject sheared Mat3 {m3:?}"
        );
        let m4 = Mat4::from_cols(
            m3.x_axis.extend(0.0),
            m3.y_axis.extend(0.0),
            m3.z_axis.extend(0.0),
            Vec4::new(3.0, -2.0, 5.0, 1.0),
        );
        assert_eq!(
            m4.try_to_scale_rotation_translation(),
            None,
            "Mat4::try_to_scale_rotation_translation must reject sheared Mat4 {m4:?}"
        );
        assert_eq!(
            Transform::try_from_mat4(m4),
            None,
            "Transform::try_from_mat4 must reject sheared Mat4 {m4:?}"
        );
    }

    // 2. Non-unit columns & improper reflections (`det < 0`) in `Quat::try_from_mat3`.
    for non_so3 in [
        Mat3::from_scale(Vec3::splat(2.0)),
        Mat3::from_scale(Vec3::new(2.0, 0.5, 1.0)),
        Mat3::from_scale(Vec3::new(1.001, 1.0, 1.0)),
        Mat3::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
        Mat3::from_scale(Vec3::splat(-1.0)),
    ] {
        assert_eq!(
            Quat::try_from_mat3(non_so3),
            None,
            "Quat::try_from_mat3 must reject non-SO(3) Mat3 {non_so3:?}"
        );
    }

    // 3. `Transform::try_mul_transform` & `mul_mat4`:
    //    Non-uniform parent scale + rotated child rejects TRS composition (`None`), while `mul_mat4`
    //    preserves exact sheared composition and `try_from_mat4` rejects the sheared product!
    let parent_non_uniform = Transform::new(
        Vec3::new(4.0, -2.0, 1.5),
        Quat::from_yaw_pitch(0.4, -0.25),
        Vec3::new(2.5, 0.5, 1.25),
    );
    let child_rotated = Transform::new(
        Vec3::new(-1.0, 3.0, 2.0),
        Quat::from_yaw_pitch_roll(-0.6, 0.5, 0.3),
        Vec3::new(0.75, 1.5, 2.0),
    );
    assert_eq!(parent_non_uniform.try_mul_transform(child_rotated), None);
    let composed_sheared_m4 = parent_non_uniform.mul_mat4(child_rotated);
    assert_eq!(
        composed_sheared_m4,
        parent_non_uniform.to_mat4() * child_rotated.to_mat4()
    );
    assert_eq!(
        composed_sheared_m4.try_to_scale_rotation_translation(),
        None
    );
    assert_eq!(Transform::try_from_mat4(composed_sheared_m4), None);

    let test_pt = Vec3::new(1.25, -0.75, 2.5);
    assert!(composed_sheared_m4.transform_point3(test_pt).abs_diff_eq(
        parent_non_uniform.transform_point3(child_rotated.transform_point3(test_pt)),
        1.0e-4
    ));

    // Uniform parent scale (positive and negative) + rotated child succeeds and matches `mul_mat4`.
    for uniform_scale in [2.5_f32, -1.75_f32] {
        let parent_uniform = Transform::new(
            Vec3::new(4.0, -2.0, 1.5),
            Quat::from_yaw_pitch(0.4, -0.25),
            Vec3::splat(uniform_scale),
        );
        let composed = parent_uniform
            .try_mul_transform(child_rotated)
            .expect("uniform parent scale composition must succeed");
        assert!(
            composed
                .to_mat4()
                .abs_diff_eq(&parent_uniform.mul_mat4(child_rotated), 1.0e-5)
        );
    }

    // Non-uniform parent scale + identity child rotation (`IDENTITY` and `-IDENTITY`) succeeds!
    for id_rot in [Quat::IDENTITY, -Quat::IDENTITY] {
        let child_id_rot = Transform::new(
            Vec3::new(-1.0, 3.0, 2.0),
            id_rot,
            Vec3::new(0.75, -1.5, 2.0),
        );
        let composed = parent_non_uniform
            .try_mul_transform(child_id_rot)
            .expect("identity child rotation composition must succeed");
        assert!(
            composed
                .to_mat4()
                .abs_diff_eq(&parent_non_uniform.mul_mat4(child_id_rot), 1.0e-5)
        );
    }

    // Degenerate / non-finite transforms return `None` from `try_mul_transform`.
    let zero_rot_transform =
        Transform::new(Vec3::ZERO, Quat::from_xyzw(0.0, 0.0, 0.0, 0.0), Vec3::ONE);
    let nan_transform = Transform::new(Vec3::NAN, Quat::IDENTITY, Vec3::ONE);
    assert_eq!(
        Transform::IDENTITY.try_mul_transform(zero_rot_transform),
        None
    );
    assert_eq!(
        zero_rot_transform.try_mul_transform(Transform::IDENTITY),
        None
    );
    assert_eq!(Transform::IDENTITY.try_mul_transform(nan_transform), None);

    // Compatible non-uniform scale (2, 1, 2) + Y-axis rotation commutes and stays shear-free!
    let parent_212 = Transform::new(
        Vec3::new(3.0, -4.0, 2.0),
        Quat::from_yaw_pitch(0.3, -0.2),
        Vec3::new(2.0, 1.0, 2.0),
    );
    let child_y = Transform::new(
        Vec3::new(-1.0, 2.5, 0.5),
        Quat::from_rotation_y(0.75),
        Vec3::new(1.25, 0.5, 2.5),
    );
    let composed_212 = parent_212
        .try_mul_transform(child_y)
        .expect("(2, 1, 2) parent + Y-rotation child must compose without shear");
    assert!(
        composed_212
            .to_mat4()
            .abs_diff_eq(&parent_212.mul_mat4(child_y), 1.0e-4)
    );
    let inv_212 = Transform::new(
        Vec3::new(1.0, -2.0, 3.0),
        Quat::from_rotation_y(-0.75),
        Vec3::new(2.0, 1.0, 2.0),
    )
    .try_inverse()
    .expect("(2, 1, 2) + Y-rotation transform must invert without shear");
    assert!(
        (inv_212.to_mat4()
            * Transform::new(
                Vec3::new(1.0, -2.0, 3.0),
                Quat::from_rotation_y(-0.75),
                Vec3::new(2.0, 1.0, 2.0),
            )
            .to_mat4())
        .abs_diff_eq(&Mat4::IDENTITY, 1.0e-4)
    );

    // Distinct non-uniform scale (2, 1, 0.5) + 90° Z-axis permutation succeeds in both compose and inverse!
    let perm_child = Transform::new(
        Vec3::new(0.5, -1.0, 2.0),
        Quat::from_rotation_z(FRAC_PI_2),
        Vec3::new(1.0, 3.0, 0.5),
    );
    let composed_perm = parent_non_uniform
        .try_mul_transform(perm_child)
        .expect("90° axis-permutation child must compose without shear");
    assert!(
        composed_perm
            .to_mat4()
            .abs_diff_eq(&parent_non_uniform.mul_mat4(perm_child), 1.0e-4)
    );

    // Small-scale 10x non-uniformity (1e-6, 1e-6, 1e-5) with 45° X rotation must be rejected.
    let tiny_non_uniform = Transform::new(
        Vec3::ZERO,
        Quat::from_rotation_x(FRAC_PI_4),
        Vec3::new(1.0e-6, 1.0e-6, 1.0e-5),
    );
    assert_eq!(tiny_non_uniform.try_inverse(), None);
    assert_eq!(
        Transform::from_scale(Vec3::new(1.0e-6, 1.0e-6, 1.0e-5))
            .try_mul_transform(Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_4))),
        None
    );

    // 4. Valid negative-scale shear-free TRS `to_mat4()` -> `try_from_mat4()` round-trip across all
    //    7 negative-axis sign combinations (including odd reflections where `sx` absorbs `det.signum()`).
    let base_rot = Quat::from_yaw_pitch_roll(0.65, -0.4, 0.3);
    let base_trans = Vec3::new(-5.5, 8.25, -12.0);
    for neg_scale in [
        Vec3::new(-2.0, 1.5, 0.75),
        Vec3::new(2.0, -1.5, 0.75),
        Vec3::new(2.0, 1.5, -0.75),
        Vec3::new(-2.0, -1.5, 0.75),
        Vec3::new(-2.0, 1.5, -0.75),
        Vec3::new(2.0, -1.5, -0.75),
        Vec3::new(-2.0, -1.5, -0.75),
    ] {
        let original = Transform::new(base_trans, base_rot, neg_scale);
        let original_m4 = original.to_mat4();
        let recovered = Transform::try_from_mat4(original_m4)
            .unwrap_or_else(|| panic!("expected valid negative-scale TRS for {neg_scale:?}"));
        assert!(recovered.rotation.is_normalized());
        assert!(
            recovered.to_mat4().abs_diff_eq(&original_m4, 1.0e-5),
            "reconstructed to_mat4() mismatch for {neg_scale:?}: got {:?}, expected {original_m4:?}",
            recovered.to_mat4()
        );
    }
}

proptest! {
    #![proptest_config(proptest_cfg())]

    #[test]
    fn f3d_t01_prop_vec2_vec3_vec4_algebra_and_glam_oracle(
        ax in -1000.0_f32..1000.0,
        ay in -1000.0_f32..1000.0,
        az in -1000.0_f32..1000.0,
        aw in -1000.0_f32..1000.0,
        bx in -1000.0_f32..1000.0,
        by in -1000.0_f32..1000.0,
        bz in -1000.0_f32..1000.0,
        bw in -1000.0_f32..1000.0,
        t in 0.0_f32..=1.0,
    ) {
        let a2 = Vec2::new(ax, ay);
        let b2 = Vec2::new(bx, by);
        let ga2 = to_glam_vec2(a2);
        let gb2 = to_glam_vec2(b2);
        prop_assert!((a2.perp_dot(b2) - ga2.perp_dot(gb2)).abs() <= 1.0e-1);
        prop_assert_eq!(a2.perp().dot(a2), 0.0);

        let a4 = Vec4::new(ax, ay, az, aw);
        let b4 = Vec4::new(bx, by, bz, bw);
        let ga4 = to_glam_vec4(a4);
        let gb4 = to_glam_vec4(b4);
        let g4_sum = ga4 + gb4;
        prop_assert!((a4 + b4).abs_diff_eq(Vec4::new(g4_sum.x, g4_sum.y, g4_sum.z, g4_sum.w), 1.0e-3));

        let a = Vec3::new(ax, ay, az);
        let b = Vec3::new(bx, by, bz);
        let ga = to_glam_vec3(a);
        let gb = to_glam_vec3(b);

        prop_assert!((a.dot(b) - ga.dot(gb)).abs() <= 1.0e-1 * (1.0 + ga.dot(gb).abs()) * 1.0e-5 + 1.0e-4);
        let c = a.cross(b);
        let gc = ga.cross(gb);
        prop_assert!(Vec3::new(gc.x, gc.y, gc.z).abs_diff_eq(c, 1.0e-2));
        // Anti-commutativity: a × b == -(b × a).
        prop_assert!(c.abs_diff_eq(-b.cross(a), 1.0e-3));

        // Endpoints of lerp are exact.
        prop_assert_eq!(a.lerp(b, 0.0), a);
        prop_assert_eq!(a.lerp(b, 1.0), b);
        let lerped = a.lerp(b, t);
        let glerped = ga.lerp(gb, t);
        prop_assert!(lerped.abs_diff_eq(Vec3::new(glerped.x, glerped.y, glerped.z), 1.0e-3));

        if let Some(unit) = a.try_normalize() {
            prop_assert!(unit.is_normalized());
            if let Some(gunit) = ga.try_normalize() {
                prop_assert!(unit.abs_diff_eq(Vec3::new(gunit.x, gunit.y, gunit.z), 1.0e-5));
            }
            // Orthogonal decomposition: proj + rej == b.
            let proj = b.project_onto_or_zero(unit);
            let rej = b.reject_from_or_zero(unit);
            prop_assert!((proj + rej).abs_diff_eq(b, 1.0e-3));
            prop_assert!(rej.dot(unit).abs() <= 1.0e-2);
        }
    }

    #[test]
    fn f3d_t01_prop_quat_laws_and_glam_oracle(
        yaw1 in -PI..PI,
        pitch1 in -1.5_f32..1.5,
        roll1 in -PI..PI,
        yaw2 in -PI..PI,
        pitch2 in -1.5_f32..1.5,
        roll2 in -PI..PI,
        vx in -100.0_f32..100.0,
        vy in -100.0_f32..100.0,
        vz in -100.0_f32..100.0,
        t in 0.0_f32..=1.0,
    ) {
        let q1 = Quat::from_yaw_pitch_roll(yaw1, pitch1, roll1);
        let q2 = Quat::from_yaw_pitch_roll(yaw2, pitch2, roll2);
        let gq1 = glam::Quat::from_euler(glam::EulerRot::YXZ, yaw1, pitch1, roll1);
        let gq2 = glam::Quat::from_euler(glam::EulerRot::YXZ, yaw2, pitch2, roll2);

        prop_assert!(q1.is_normalized());
        prop_assert!(q2.is_normalized());
        prop_assert!(q1.angle_between(from_glam_quat(gq1)) <= 1.0e-5);
        prop_assert!(q2.angle_between(from_glam_quat(gq2)) <= 1.0e-5);

        // Composition homomorphism: (q1 * q2) * v == q1 * (q2 * v) == Mat3::from_quat(q1 * q2) * v.
        let v = Vec3::new(vx, vy, vz);
        let q12 = q1 * q2;
        let rot_comp = q12 * v;
        let rot_seq = q1 * (q2 * v);
        let rot_mat = Mat3::from_quat(q12) * v;
        let g_rot = (gq1 * gq2) * to_glam_vec3(v);
        prop_assert!(rot_comp.abs_diff_eq(rot_seq, 1.0e-3));
        prop_assert!(rot_comp.abs_diff_eq(rot_mat, 1.0e-3));
        prop_assert!(rot_comp.abs_diff_eq(Vec3::new(g_rot.x, g_rot.y, g_rot.z), 1.0e-3));

        // Inverse round-trip: q1.conjugate() * (q1 * v) == v.
        let roundtrip_v = q1.conjugate() * (q1 * v);
        prop_assert!(roundtrip_v.abs_diff_eq(v, 1.0e-3));

        // Mat3 -> Quat Shepperd round-trip.
        let recovered = Quat::try_from_mat3(Mat3::from_quat(q1)).unwrap();
        prop_assert!(q1.angle_between(recovered) <= 2.0e-4);

        // Shortest-arc slerp matches glam and is invariant under `q2 -> -q2`.
        let s_pos = q1.slerp(q2, t);
        let s_neg = q1.slerp(-q2, t);
        let gs = gq1.slerp(gq2, t);
        prop_assert!(s_pos.is_normalized());
        prop_assert!(s_pos.angle_between(s_neg) <= 1.0e-5);
        prop_assert!(s_pos.angle_between(from_glam_quat(gs)) <= 5.0e-4);
    }

    #[test]
    fn f3d_t01_prop_mat4_transform_inverse_and_glam_oracle(
        tx in -200.0_f32..200.0,
        ty in -200.0_f32..200.0,
        tz in -200.0_f32..200.0,
        yaw in -PI..PI,
        pitch in -1.4_f32..1.4,
        sx in 0.25_f32..4.0,
        sy in 0.25_f32..4.0,
        sz in 0.25_f32..4.0,
        uniform_s in 0.25_f32..4.0,
        px in -50.0_f32..50.0,
        py in -50.0_f32..50.0,
        pz in -50.0_f32..50.0,
    ) {
        let translation = Vec3::new(tx, ty, tz);
        let rotation = Quat::from_yaw_pitch(yaw, pitch);
        let scale = Vec3::new(sx, sy, sz);
        let p = Vec3::new(px, py, pz);

        let m = Mat4::from_scale_rotation_translation(scale, rotation, translation);
        let gm = glam::Mat4::from_scale_rotation_translation(
            to_glam_vec3(scale),
            to_glam_quat(rotation),
            to_glam_vec3(translation),
        );
        prop_assert!(m.abs_diff_eq(&from_glam_mat4(gm), 1.0e-4));

        // Mat4::inverse matches glam and satisfies M * M^-1 == I.
        let inv = m.inverse().unwrap();
        let ginv = from_glam_mat4(gm.inverse());
        prop_assert!(inv.abs_diff_eq(&ginv, 5.0e-4));
        prop_assert!((m * inv).abs_diff_eq(&Mat4::IDENTITY, 5.0e-4));

        // TRS decomposition round-trip.
        let (dec_s, dec_r, dec_t) = m.try_to_scale_rotation_translation().unwrap();
        prop_assert!(dec_s.abs_diff_eq(scale, 5.0e-4));
        prop_assert!(dec_r.angle_between(rotation) <= 5.0e-4);
        prop_assert!(dec_t.abs_diff_eq(translation, 1.0e-4));

        // Normal matrix preserves surface-normal orthogonality under non-uniform scale!
        let n_local = Vec3::new(0.6, 0.8, 0.0);
        let tangent_local = Vec3::new(-0.8, 0.6, 0.5); //Wait: dot = -0.48 + 0.48 + 0 = 0!
        let n_world = (Mat3::normal_matrix(scale, rotation).unwrap() * n_local)
            .try_normalize()
            .unwrap();
        let t_world = m.transform_vector3(tangent_local);
        prop_assert!(n_world.dot(t_world).abs() <= 1.0e-4);

        // Similarity Transform (uniform scale) inverse & composition match Mat4!
        let sim = Transform::new(translation, rotation, Vec3::splat(uniform_s));
        prop_assert!(sim.transform_point3(p).abs_diff_eq(sim.to_mat4().transform_point3(p), 1.0e-3));
        let sim_inv = sim.try_inverse().unwrap();
        let recovered_p = sim_inv.transform_point3(sim.transform_point3(p));
        prop_assert!(recovered_p.abs_diff_eq(p, 2.0e-3));

        // look_to_rh matches glam::camera::rh::view::look_to_mat4!
        let look = Mat4::try_look_to_rh(translation, rotation.forward(), Vec3::Y).unwrap();
        let glook = from_glam_mat4(glam::camera::rh::view::look_to_mat4(
            to_glam_vec3(translation),
            to_glam_vec3(rotation.forward()),
            glam::Vec3::Y,
        ));
        prop_assert!(look.abs_diff_eq(&glook, 5.0e-4));
        // Mat3 determinant & inverse match glam.
        let m3 = m.to_mat3();
        let gm3 = to_glam_mat3(m3);
        prop_assert!((m3.determinant() - gm3.determinant()).abs() <= 1.0e-3);
        prop_assert!(m3.inverse().unwrap().abs_diff_eq(&from_glam_mat3(gm3.inverse()), 5.0e-4));
    }

    #[test]
    fn f3d_t01_prop_transform_composition_shear_and_negative_scale_roundtrip(
        p_tx in -50.0_f32..50.0,
        p_ty in -50.0_f32..50.0,
        p_tz in -50.0_f32..50.0,
        c_tx in -50.0_f32..50.0,
        c_ty in -50.0_f32..50.0,
        c_tz in -50.0_f32..50.0,
        p_yaw in -PI..PI,
        p_pitch in -1.4_f32..1.4,
        c_yaw in 0.15_f32..1.35,
        c_pitch in 0.15_f32..1.25,
        uniform_mag in 0.25_f32..4.0,
        uniform_neg in any::<bool>(),
        sx in 1.5_f32..3.5,
        sy in 0.25_f32..0.9,
        sz in 0.4_f32..2.5,
        csx in 0.25_f32..3.0,
        csy in 0.25_f32..3.0,
        csz in 0.25_f32..3.0,
        neg_x in any::<bool>(),
        neg_y in any::<bool>(),
        neg_z in any::<bool>(),
    ) {
        let p_trans = Vec3::new(p_tx, p_ty, p_tz);
        let c_trans = Vec3::new(c_tx, c_ty, c_tz);
        let p_rot = Quat::from_yaw_pitch(p_yaw, p_pitch);
        let c_rot = Quat::from_yaw_pitch(c_yaw, c_pitch);
        let c_scale = Vec3::new(csx, csy, csz);

        // 1. Uniform parent scale (positive or negative) + rotated child -> exact `Some(t)`.
        let signed_uniform = if uniform_neg { -uniform_mag } else { uniform_mag };
        let parent_uniform = Transform::new(p_trans, p_rot, Vec3::splat(signed_uniform));
        let child_rot_t = Transform::new(c_trans, c_rot, c_scale);
        let composed_u = parent_uniform.try_mul_transform(child_rot_t).unwrap();
        let expected_u_m4 = parent_uniform.mul_mat4(child_rot_t);
        prop_assert!(composed_u.to_mat4().abs_diff_eq(&expected_u_m4, 1.0e-3));

        // 2. Non-uniform parent scale + identity child rotation -> exact `Some(t)`.
        let non_uniform_s = Vec3::new(sx, sy, sz);
        let parent_non_uniform = Transform::new(p_trans, p_rot, non_uniform_s);
        let child_id_t = Transform::new(c_trans, Quat::IDENTITY, c_scale);
        let composed_id = parent_non_uniform.try_mul_transform(child_id_t).unwrap();
        let expected_id_m4 = parent_non_uniform.mul_mat4(child_id_t);
        prop_assert!(composed_id.to_mat4().abs_diff_eq(&expected_id_m4, 1.0e-3));

        // 3. Non-uniform parent scale (`sx >= 1.5 > 0.9 >= sy`) + oblique child rotation
        //    (`c_yaw, c_pitch` strictly away from 90° axis permutations) -> `None` everywhere!
        prop_assert_eq!(parent_non_uniform.try_mul_transform(child_rot_t), None);
        let sheared_m4 = parent_non_uniform.mul_mat4(child_rot_t);
        prop_assert_eq!(sheared_m4.try_to_scale_rotation_translation(), None);
        prop_assert_eq!(Transform::try_from_mat4(sheared_m4), None);

        // 4. Signed / negative-scale shear-free TRS round-trip (`to_mat4` -> `try_from_mat4` -> `to_mat4`).
        let signed_scale = Vec3::new(
            if neg_x { -sx } else { sx },
            if neg_y { -sy } else { sy },
            if neg_z { -sz } else { sz },
        );
        let signed_trs = Transform::new(p_trans, p_rot, signed_scale);
        let signed_m4 = signed_trs.to_mat4();
        let recovered = Transform::try_from_mat4(signed_m4).unwrap();
        prop_assert!(recovered.rotation.is_normalized());
        prop_assert!(recovered.to_mat4().abs_diff_eq(&signed_m4, 1.0e-3));
    }

    #[test]
    fn f3d_t01_prop_compatible_trs_composition_and_inverse(
        p_tx in -25.0_f32..25.0,
        p_ty in -25.0_f32..25.0,
        p_tz in -25.0_f32..25.0,
        c_tx in -25.0_f32..25.0,
        c_ty in -25.0_f32..25.0,
        c_tz in -25.0_f32..25.0,
        p_yaw in -PI..PI,
        p_pitch in -1.3_f32..1.3,
        c_yaw in -PI..PI,
        c_pitch in -1.3_f32..1.3,
        c_roll in -PI..PI,
        xz_mag in 0.25_f32..4.0,
        y_mag in 0.25_f32..4.0,
        u_mag in 0.25_f32..4.0,
        neg_x in any::<bool>(),
        neg_y in any::<bool>(),
        neg_z in any::<bool>(),
        csx in 0.25_f32..3.0,
        csy in 0.25_f32..3.0,
        csz in 0.25_f32..3.0,
    ) {
        let p_trans = Vec3::new(p_tx, p_ty, p_tz);
        let c_trans = Vec3::new(c_tx, c_ty, c_tz);
        let p_rot = Quat::from_yaw_pitch(p_yaw, p_pitch);
        let c_scale = Vec3::new(csx, csy, csz);

        // 1. Compatible planar-uniform non-uniform scale `(±xz, ±y, ±xz)` + Y-axis rotation:
        //    both composition and inverse succeed and match matrix semantics.
        let planar_scale = Vec3::new(
            if neg_x { -xz_mag } else { xz_mag },
            if neg_y { -y_mag } else { y_mag },
            if neg_z { -xz_mag } else { xz_mag },
        );
        let parent_planar = Transform::new(p_trans, p_rot, planar_scale);
        let child_y = Transform::new(c_trans, Quat::from_rotation_y(c_yaw), c_scale);
        let composed_planar = parent_planar.try_mul_transform(child_y).unwrap();
        prop_assert!(composed_planar.rotation.is_normalized());
        prop_assert!(composed_planar.to_mat4().abs_diff_eq(&parent_planar.mul_mat4(child_y), 5.0e-3));

        let y_rot_trs = Transform::new(p_trans, Quat::from_rotation_y(c_yaw), planar_scale);
        let y_inv = y_rot_trs.try_inverse().unwrap();
        prop_assert!((y_rot_trs.to_mat4() * y_inv.to_mat4()).abs_diff_eq(&Mat4::IDENTITY, 1.0e-3));
        let y_round = y_rot_trs.try_mul_transform(y_inv).unwrap();
        prop_assert!(y_round.to_mat4().abs_diff_eq(&Mat4::IDENTITY, 1.0e-3));

        // 2. Signed uniform-magnitude scale `(±u, ±u, ±u)` + arbitrary 3-axis rotation:
        //    both composition and inverse succeed regardless of reflection signs.
        let signed_uniform_scale = Vec3::new(
            if neg_x { -u_mag } else { u_mag },
            if neg_y { -u_mag } else { u_mag },
            if neg_z { -u_mag } else { u_mag },
        );
        let parent_signed_u = Transform::new(p_trans, p_rot, signed_uniform_scale);
        let child_3d = Transform::new(
            c_trans,
            Quat::from_yaw_pitch_roll(c_yaw, c_pitch, c_roll),
            c_scale,
        );
        let composed_su = parent_signed_u.try_mul_transform(child_3d).unwrap();
        prop_assert!(composed_su.to_mat4().abs_diff_eq(&parent_signed_u.mul_mat4(child_3d), 5.0e-3));

        let signed_u_trs = Transform::new(
            p_trans,
            Quat::from_yaw_pitch_roll(c_yaw, c_pitch, c_roll),
            signed_uniform_scale,
        );
        let su_inv = signed_u_trs.try_inverse().unwrap();
        prop_assert!((signed_u_trs.to_mat4() * su_inv.to_mat4()).abs_diff_eq(&Mat4::IDENTITY, 1.0e-3));
        let su_round = signed_u_trs.try_mul_transform(su_inv).unwrap();
        prop_assert!(su_round.to_mat4().abs_diff_eq(&Mat4::IDENTITY, 1.0e-3));

        // 3. Zero-false-positive soundness invariant on arbitrary transforms:
        //    if `try_mul_transform` or `try_inverse` returns `Some`, it MUST match `Mat4`!
        let arbitrary_parent = Transform::new(p_trans, p_rot, Vec3::new(xz_mag, y_mag, u_mag));
        if let Some(composed) = arbitrary_parent.try_mul_transform(child_3d) {
            prop_assert!(composed.rotation.is_normalized());
            prop_assert!(composed.to_mat4().abs_diff_eq(&arbitrary_parent.mul_mat4(child_3d), 5.0e-3));
        }
        if let Some(inv) = arbitrary_parent.try_inverse() {
            prop_assert!(inv.rotation.is_normalized());
            prop_assert!((arbitrary_parent.to_mat4() * inv.to_mat4()).abs_diff_eq(&Mat4::IDENTITY, 1.0e-3));
        }
    }
}
