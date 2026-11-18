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
}
